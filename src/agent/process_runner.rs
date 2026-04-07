// EasyNet CLI — Bounded Child Process Runner
// ============================================
//
// File: src/agent/process_runner.rs
// Description: Spawns child processes with timeout, output size limits,
//              and clean shutdown (SIGTERM → SIGKILL escalation).
//
// Used by claude_code.rs and codex.rs to safely invoke external agent CLIs.
//
// Author: Silan Hu <silan.hu@u.nus.edu>
// Copyright (c) 2026 EasyNet. All rights reserved.

use std::collections::BTreeMap;
use std::io::{BufRead, BufReader, Read};
use std::path::PathBuf;
use std::process::{Command, Stdio};
use std::sync::Arc;
use std::time::{Duration, Instant};

use anyhow::Context;

/// Callback invoked for each complete stdout line while the child is running.
///
/// Used by callers that want to observe progress live (e.g. streaming JSON
/// events from `claude -p --output-format stream-json`). The callback runs on
/// the stdout reader thread and must be cheap and non-blocking.
pub type LineCallback = Arc<dyn Fn(&str) + Send + Sync + 'static>;

pub struct ChildOptions {
    pub timeout: Duration,
    pub max_stdout_bytes: usize,
    pub max_stderr_bytes: usize,
    pub stdin_data: Option<String>,
    pub env: BTreeMap<String, String>,
    pub cwd: Option<PathBuf>,
    /// Optional line callback for streaming stdout consumption.
    pub stdout_line_callback: Option<LineCallback>,
}

impl Default for ChildOptions {
    fn default() -> Self {
        Self {
            timeout: Duration::from_secs(300),
            max_stdout_bytes: 1_048_576,  // 1 MB
            max_stderr_bytes: 262_144,    // 256 KB
            stdin_data: None,
            env: BTreeMap::new(),
            cwd: None,
            stdout_line_callback: None,
        }
    }
}

#[allow(dead_code)] // Fields consumed by callers that inspect the full result.
pub struct ChildResult {
    pub stdout: String,
    pub stderr: String,
    pub exit_code: i32,
    pub duration: Duration,
    pub truncated: bool,
}

/// Run a child process with bounded output collection and timeout.
///
/// - Prompt is piped via stdin if `opts.stdin_data` is set.
/// - Stdout/stderr are read with size caps.
/// - On timeout: SIGTERM, then SIGKILL after 5s grace period.
pub fn run_child(cmd: &str, args: &[&str], opts: ChildOptions) -> anyhow::Result<ChildResult> {
    let start = Instant::now();

    let mut command = Command::new(cmd);
    command.args(args);

    if opts.stdin_data.is_some() {
        command.stdin(Stdio::piped());
    } else {
        command.stdin(Stdio::null());
    }
    command.stdout(Stdio::piped());
    command.stderr(Stdio::piped());

    for (k, v) in &opts.env {
        command.env(k, v);
    }
    if let Some(cwd) = &opts.cwd {
        command.current_dir(cwd);
    }

    let mut child = command.spawn()
        .with_context(|| format!("spawn {cmd}"))?;

    // Write stdin data if provided.
    if let Some(data) = &opts.stdin_data {
        use std::io::Write;
        if let Some(mut stdin) = child.stdin.take() {
            // Write in a best-effort manner; if the child closes stdin early, that's OK.
            let _ = stdin.write_all(data.as_bytes());
            let _ = stdin.flush();
            drop(stdin);
        }
    }

    // Read stdout and stderr with size limits using threads.
    let max_out = opts.max_stdout_bytes;
    let max_err = opts.max_stderr_bytes;

    let stdout_pipe = child.stdout.take().unwrap();
    let mut stderr_pipe = child.stderr.take().unwrap();

    let line_cb = opts.stdout_line_callback.clone();
    let stdout_handle = std::thread::spawn(move || {
        if let Some(cb) = line_cb {
            read_bounded_lines(stdout_pipe, max_out, cb)
        } else {
            let mut pipe = stdout_pipe;
            read_bounded(&mut pipe, max_out)
        }
    });
    let stderr_handle = std::thread::spawn(move || read_bounded(&mut stderr_pipe, max_err));

    // Wait for child with timeout.
    let exit_code = wait_with_timeout(&mut child, opts.timeout)?;

    let (stdout_data, stdout_truncated) = stdout_handle.join().unwrap();
    let (stderr_data, stderr_truncated) = stderr_handle.join().unwrap();

    let stdout = String::from_utf8_lossy(&stdout_data).to_string();
    let stderr = String::from_utf8_lossy(&stderr_data).to_string();

    Ok(ChildResult {
        stdout,
        stderr,
        exit_code,
        duration: start.elapsed(),
        truncated: stdout_truncated || stderr_truncated,
    })
}

/// Line-oriented bounded reader. Invokes `callback(line)` for each full line
/// and returns the full collected output (up to `max_bytes`).
fn read_bounded_lines<R: Read>(reader: R, max_bytes: usize, callback: LineCallback) -> (Vec<u8>, bool) {
    let mut br = BufReader::new(reader);
    let mut collected: Vec<u8> = Vec::new();
    let mut truncated = false;
    let mut line = String::new();

    loop {
        line.clear();
        match br.read_line(&mut line) {
            Ok(0) => break,
            Ok(_) => {
                let bytes = line.as_bytes();
                let remaining = max_bytes.saturating_sub(collected.len());
                if remaining == 0 {
                    truncated = true;
                } else if bytes.len() > remaining {
                    collected.extend_from_slice(&bytes[..remaining]);
                    truncated = true;
                } else {
                    collected.extend_from_slice(bytes);
                }
                let trimmed = line.trim_end_matches(['\n', '\r']);
                if !trimmed.is_empty() {
                    callback(trimmed);
                }
            }
            Err(e) if e.kind() == std::io::ErrorKind::Interrupted => continue,
            Err(_) => break,
        }
    }
    (collected, truncated)
}

fn read_bounded(reader: &mut impl Read, max_bytes: usize) -> (Vec<u8>, bool) {
    let mut buf = vec![0u8; 8192];
    let mut collected = Vec::new();
    let mut truncated = false;

    loop {
        match reader.read(&mut buf) {
            Ok(0) => break,
            Ok(n) => {
                let remaining = max_bytes.saturating_sub(collected.len());
                if remaining == 0 {
                    truncated = true;
                    // Keep reading to drain the pipe but discard.
                    continue;
                }
                let take = n.min(remaining);
                collected.extend_from_slice(&buf[..take]);
                if take < n {
                    truncated = true;
                }
            }
            Err(e) if e.kind() == std::io::ErrorKind::Interrupted => continue,
            Err(_) => break,
        }
    }
    (collected, truncated)
}

fn wait_with_timeout(child: &mut std::process::Child, timeout: Duration) -> anyhow::Result<i32> {
    let deadline = Instant::now() + timeout;
    let poll_interval = Duration::from_millis(100);

    loop {
        match child.try_wait()? {
            Some(status) => return Ok(status.code().unwrap_or(-1)),
            None => {
                if Instant::now() >= deadline {
                    // Timeout: escalate kill.
                    kill_child(child);
                    anyhow::bail!(
                        "agent process timed out after {}s",
                        timeout.as_secs()
                    );
                }
                std::thread::sleep(poll_interval);
            }
        }
    }
}

fn kill_child(child: &mut std::process::Child) {
    #[cfg(unix)]
    {
        let pid = child.id() as i32;
        // SIGTERM first.
        unsafe { libc::kill(pid, libc::SIGTERM); }
        // Wait 5s for graceful exit.
        let deadline = Instant::now() + Duration::from_secs(5);
        loop {
            match child.try_wait() {
                Ok(Some(_)) => return,
                _ if Instant::now() >= deadline => break,
                _ => std::thread::sleep(Duration::from_millis(200)),
            }
        }
        // SIGKILL.
        unsafe { libc::kill(pid, libc::SIGKILL); }
        let _ = child.wait();
    }
    #[cfg(not(unix))]
    {
        let _ = child.kill();
        let _ = child.wait();
    }
}
