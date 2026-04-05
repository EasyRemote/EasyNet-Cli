// EasyNet CLI — Codex Agent Wrapper
// ===================================
//
// File: src/agent/codex.rs
// Description: Invokes Codex in two modes:
//   1. `codex exec` — simple non-interactive mode (like claude -p)
//   2. `codex app-server` — advanced JSON-RPC 2.0 mode with threads and streaming
//
// Author: Silan Hu <silan.hu@u.nus.edu>
// Copyright (c) 2026 EasyNet. All rights reserved.

use std::collections::BTreeMap;
use std::time::Duration;

use super::process_runner::{self, ChildOptions};

pub struct CodexOptions {
    pub model: Option<String>,
    pub timeout: Duration,
    pub max_output_bytes: usize,
    pub env: BTreeMap<String, String>,
    #[allow(dead_code)] // Reserved for future --write / --full-auto mode toggle.
    pub write_mode: bool,
    /// Workspace directory (with .codex/ config). If set, codex runs in this cwd.
    pub cwd: Option<std::path::PathBuf>,
}

impl Default for CodexOptions {
    fn default() -> Self {
        Self {
            model: None,
            timeout: Duration::from_secs(300),
            max_output_bytes: 1_048_576,
            env: BTreeMap::new(),
            write_mode: false,
            cwd: None,
        }
    }
}

/// Invoke Codex in exec mode (simple, non-interactive).
///
/// Spawns: `codex exec [--model <m>]`
/// Prompt is piped via stdin.
pub fn invoke_exec(prompt: &str, opts: CodexOptions) -> anyhow::Result<String> {
    let mut args: Vec<String> = vec!["exec".to_string()];

    if let Some(m) = &opts.model {
        args.push("-c".to_string());
        args.push(format!("model=\"{m}\""));
    }

    let arg_refs: Vec<&str> = args.iter().map(|s| s.as_str()).collect();

    let result = process_runner::run_child("codex", &arg_refs, ChildOptions {
        timeout: opts.timeout,
        max_stdout_bytes: opts.max_output_bytes,
        max_stderr_bytes: 262_144,
        stdin_data: Some(prompt.to_string()),
        env: opts.env,
        cwd: opts.cwd,
    })?;

    if result.exit_code != 0 {
        let err_msg = if result.stderr.is_empty() {
            format!("codex exec exited with code {}", result.exit_code)
        } else {
            format!("codex exec error (exit {}): {}", result.exit_code, result.stderr.trim())
        };
        anyhow::bail!(err_msg);
    }

    Ok(result.stdout)
}

/// Invoke Codex via app-server JSON-RPC protocol (advanced mode).
///
/// Protocol flow:
///   1. Spawn `codex app-server` as child process
///   2. Send initialize -> thread/start -> turn/start over stdin
///   3. Read line-delimited JSON-RPC notifications from stdout
///   4. Wait for turn/completed, extract final message
pub fn invoke_app_server(prompt: &str, opts: CodexOptions) -> anyhow::Result<String> {
    use std::collections::HashMap;
    use std::io::{BufRead, BufReader, Write};
    use std::process::{Command, Stdio};
    use std::sync::mpsc;
    use std::time::Instant;

    let started = Instant::now();
    let deadline = started + opts.timeout;

    let mut cmd = Command::new("codex");
    cmd.args(["app-server"])
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::null());

    if let Some(cwd) = &opts.cwd {
        cmd.current_dir(cwd);
    }
    for (k, v) in &opts.env {
        cmd.env(k, v);
    }

    let mut child = cmd.spawn()
        .map_err(|e| anyhow::anyhow!("spawn codex app-server: {e}"))?;

    let mut stdin = child.stdin.take().unwrap();
    let stdout = child.stdout.take().unwrap();

    // Background reader thread.
    let (tx, rx) = mpsc::channel::<serde_json::Value>();
    let _reader_handle = std::thread::spawn(move || {
        let mut reader = BufReader::new(stdout);
        let mut line = String::new();
        while let Ok(n) = reader.read_line(&mut line) {
            if n == 0 { break; }
            let trimmed = line.trim();
            if !trimmed.is_empty() {
                if let Ok(v) = serde_json::from_str::<serde_json::Value>(trimmed) {
                    if tx.send(v).is_err() { break; }
                }
            }
            line.clear();
        }
    });

    // RPC helpers.
    let mut send_rpc = |id: u64, method: &str, params: serde_json::Value| -> anyhow::Result<()> {
        let msg = serde_json::json!({"id": id, "method": method, "params": params});
        let wire = serde_json::to_string(&msg)? + "\n";
        stdin.write_all(wire.as_bytes())?;
        stdin.flush()?;
        Ok(())
    };

    let wait_response = |rx: &mpsc::Receiver<serde_json::Value>,
                         id: u64,
                         deadline: Instant,
                         stash: &mut HashMap<u64, serde_json::Value>|
     -> anyhow::Result<serde_json::Value> {
        if let Some(v) = stash.remove(&id) {
            return Ok(v);
        }
        loop {
            let now = Instant::now();
            if now >= deadline {
                anyhow::bail!("codex app-server: timeout waiting for response id={id}");
            }
            let remaining = deadline - now;
            let msg = rx.recv_timeout(remaining.min(Duration::from_secs(30)))
                .map_err(|_| anyhow::anyhow!("codex app-server: timeout waiting for response id={id}"))?;

            if let Some(msg_id) = msg.get("id").and_then(|v| v.as_u64()) {
                if msg_id == id {
                    return Ok(msg);
                }
                stash.insert(msg_id, msg);
            }
            // Skip notifications.
        }
    };

    let mut stash: HashMap<u64, serde_json::Value> = HashMap::new();

    // 1. Initialize
    send_rpc(1, "initialize", serde_json::json!({
        "clientInfo": { "name": "easynet", "version": env!("CARGO_PKG_VERSION") },
        "capabilities": { "experimentalApi": false }
    }))?;
    let init = wait_response(&rx, 1, deadline, &mut stash)?;
    if init.get("error").is_some() {
        anyhow::bail!("codex app-server initialize error: {init}");
    }

    // 2. Thread start
    let model_str = opts.model.clone().unwrap_or_else(|| "gpt-5.2".to_string());
    send_rpc(2, "thread/start", serde_json::json!({
        "cwd": std::env::current_dir()?.to_string_lossy(),
        "model": model_str,
        "sandbox": "read-only",
        "approvalPolicy": "never",
        "ephemeral": true,
    }))?;
    let thread_resp = wait_response(&rx, 2, deadline, &mut stash)?;
    if thread_resp.get("error").is_some() {
        anyhow::bail!("codex app-server thread/start error: {thread_resp}");
    }
    let thread_id = thread_resp
        .pointer("/result/thread/id")
        .and_then(|v| v.as_str())
        .ok_or_else(|| anyhow::anyhow!("codex app-server: missing thread id"))?
        .to_string();

    // 3. Turn start
    send_rpc(3, "turn/start", serde_json::json!({
        "threadId": thread_id,
        "input": [{ "type": "text", "text": prompt }],
    }))?;
    let turn_resp = wait_response(&rx, 3, deadline, &mut stash)?;
    if turn_resp.get("error").is_some() {
        anyhow::bail!("codex app-server turn/start error: {turn_resp}");
    }

    // 4. Wait for turn/completed notification
    let mut final_message = String::new();
    loop {
        let now = Instant::now();
        if now >= deadline {
            anyhow::bail!("codex app-server: timeout waiting for turn/completed");
        }
        let remaining = deadline - now;
        let msg = rx.recv_timeout(remaining.min(Duration::from_secs(30)))
            .map_err(|_| anyhow::anyhow!("codex app-server: timeout waiting for turn/completed"))?;

        let method = msg.get("method").and_then(|v| v.as_str()).unwrap_or("");
        match method {
            "turn/completed" => break,
            "item/completed" => {
                if msg.pointer("/params/item/type").and_then(|v| v.as_str()) == Some("agentMessage") {
                    if let Some(text) = msg.pointer("/params/item/text").and_then(|v| v.as_str()) {
                        final_message = text.to_string();
                    }
                }
            }
            _ => {}
        }
    }

    // Clean shutdown.
    drop(stdin);
    let _ = child.kill();
    let _ = child.wait();

    if final_message.is_empty() {
        anyhow::bail!("codex app-server: no agent message received");
    }

    Ok(final_message)
}

/// Check if the `codex` CLI is available and return version info.
pub fn doctor() -> anyhow::Result<String> {
    let result = process_runner::run_child("codex", &["--version"], ChildOptions {
        timeout: Duration::from_secs(10),
        max_stdout_bytes: 4096,
        ..Default::default()
    })?;
    if result.exit_code != 0 {
        anyhow::bail!("codex --version failed (exit {})", result.exit_code);
    }
    Ok(result.stdout.trim().to_string())
}
