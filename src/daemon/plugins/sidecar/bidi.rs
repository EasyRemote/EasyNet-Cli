// EasyNet CLI — sidecar bidi pump
// ===============================
//
// File: src/daemon/plugins/sidecar/bidi.rs
// Description: Live bidirectional JSON-frame pump for sidecar processes.

use std::io::{BufRead, BufReader};
use std::path::{Path, PathBuf};
use std::process::{Child, ChildStdin, ExitStatus};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Arc;
use std::time::Duration;

use serde_json::Value;
use tokio::sync::mpsc;

use crate::daemon::plugins::errors::{PluginHostError, Result};
use crate::daemon::plugins::sidecar::io::{
    collect_stderr, duration_millis, spawn_stderr_reader, wait_child_with_timeout,
    write_sidecar_frame,
};
use crate::daemon::plugins::sidecar::{
    SidecarInvocationEnvelope, SidecarRequestFrame, SidecarResponseFrame,
};
use crate::runtime::ability_dispatch::{BidiOutputFrame, BidiSource, BIDI_CHANNEL_BOUND};

/// Open the daemon-owned bidi channels around one spawned sidecar process.
///
/// Invariant 1: the process receives exactly one `bidi_open` frame before any
/// client input can be forwarded.
/// Invariant 2: every output path goes through `SidecarTerminalGuard`, so only
/// one terminal frame can reach the daemon even if the sidecar double-fires.
pub(super) fn open_bidi_session(
    program: &Path,
    mut child: Child,
    call_id: String,
    invocation: SidecarInvocationEnvelope,
    exit_timeout: Duration,
) -> Result<BidiSource> {
    let mut stdin = child
        .stdin
        .take()
        .ok_or_else(|| PluginHostError::SidecarStdinUnavailable {
            program: program.to_path_buf(),
        })?;
    let stdout = child
        .stdout
        .take()
        .ok_or_else(|| PluginHostError::SidecarStdoutUnavailable {
            program: program.to_path_buf(),
        })?;
    let stderr_handle = child.stderr.take().map(spawn_stderr_reader);

    write_sidecar_frame(
        program,
        &mut stdin,
        &SidecarRequestFrame::BidiOpen {
            call_id: call_id.clone(),
            invocation,
        },
    )?;

    let (input_tx, input_rx) = mpsc::channel(BIDI_CHANNEL_BOUND);
    let (output_tx, output_rx) = mpsc::channel(BIDI_CHANNEL_BOUND);
    let terminal = SidecarTerminalGuard::new();
    spawn_bidi_writer(program.to_path_buf(), stdin, call_id.clone(), input_rx);
    spawn_bidi_reader(SidecarBidiReader {
        program: program.to_path_buf(),
        stdout,
        child,
        call_id,
        output_tx,
        terminal,
        stderr_handle,
        exit_timeout,
    });

    Ok(BidiSource {
        to_client: input_tx,
        from_client: output_rx,
    })
}

fn sidecar_process_failed_message(program: &Path, status: ExitStatus, stderr: &str) -> String {
    format!(
        "sidecar process {} exited with {status}; stderr={stderr}",
        program.display()
    )
}

fn sidecar_process_timed_out_message(program: &Path, timeout: Duration, stderr: &str) -> String {
    format!(
        "sidecar process {} timed out after {} ms; stderr={stderr}",
        program.display(),
        duration_millis(timeout),
    )
}

#[derive(Clone, Default)]
struct SidecarTerminalGuard {
    sent: Arc<AtomicBool>,
}

impl SidecarTerminalGuard {
    fn new() -> Self {
        Self::default()
    }

    fn send_closed(
        &self,
        output_tx: &mpsc::Sender<BidiOutputFrame>,
        reason: impl Into<String>,
    ) -> bool {
        if self.sent.swap(true, Ordering::AcqRel) {
            return false;
        }
        let _ = output_tx.blocking_send(BidiOutputFrame::json(serde_json::json!({
            "type": "closed",
            "reason": reason.into(),
        })));
        true
    }

    fn send_error(
        &self,
        output_tx: &mpsc::Sender<BidiOutputFrame>,
        message: impl Into<String>,
    ) -> bool {
        if self.sent.swap(true, Ordering::AcqRel) {
            return false;
        }
        let _ = output_tx.blocking_send(BidiOutputFrame::json(serde_json::json!({
            "type": "error",
            "message": message.into(),
        })));
        true
    }
}

fn spawn_bidi_writer(
    program: PathBuf,
    mut stdin: ChildStdin,
    call_id: String,
    mut input_rx: mpsc::Receiver<Value>,
) {
    tokio::spawn(async move {
        while let Some(frame) = input_rx.recv().await {
            let request = SidecarRequestFrame::BidiInput {
                call_id: call_id.clone(),
                frame,
            };
            if write_sidecar_frame(&program, &mut stdin, &request).is_err() {
                return;
            }
        }
        let _ = write_sidecar_frame(
            &program,
            &mut stdin,
            &SidecarRequestFrame::Close {
                call_id,
                reason: "client_closed".to_string(),
            },
        );
    });
}

struct SidecarBidiReader {
    program: PathBuf,
    stdout: std::process::ChildStdout,
    child: Child,
    call_id: String,
    output_tx: mpsc::Sender<BidiOutputFrame>,
    terminal: SidecarTerminalGuard,
    stderr_handle: Option<std::thread::JoinHandle<String>>,
    exit_timeout: Duration,
}

fn spawn_bidi_reader(reader: SidecarBidiReader) {
    let SidecarBidiReader {
        program,
        stdout,
        mut child,
        call_id,
        output_tx,
        terminal,
        stderr_handle,
        exit_timeout,
    } = reader;
    std::thread::spawn(move || {
        for line in BufReader::new(stdout).lines() {
            let line = match line {
                Ok(line) => line,
                Err(err) => {
                    terminal.send_error(
                        &output_tx,
                        format!(
                            "sidecar stdout read failed for {}: {err}",
                            program.display()
                        ),
                    );
                    break;
                }
            };
            if line.trim().is_empty() {
                continue;
            }
            let frame: SidecarResponseFrame = match serde_json::from_str(&line) {
                Ok(frame) => frame,
                Err(err) => {
                    terminal.send_error(
                        &output_tx,
                        format!("sidecar output frame decode failed: {err}"),
                    );
                    break;
                }
            };
            match frame {
                SidecarResponseFrame::BidiOutput {
                    call_id: response_call_id,
                    frame,
                } if response_call_id == call_id => {
                    if terminal.sent.load(Ordering::Acquire) {
                        continue;
                    }
                    if output_tx
                        .blocking_send(BidiOutputFrame::json(frame))
                        .is_err()
                    {
                        break;
                    }
                }
                SidecarResponseFrame::Terminal {
                    call_id: response_call_id,
                    reason,
                } if response_call_id == call_id => {
                    terminal.send_closed(&output_tx, reason);
                    break;
                }
                SidecarResponseFrame::Error {
                    call_id: response_call_id,
                    message,
                } if response_call_id == call_id => {
                    terminal.send_error(&output_tx, message);
                    break;
                }
                other => {
                    terminal.send_error(
                        &output_tx,
                        format!("unexpected bidi response frame: {other:?}"),
                    );
                    break;
                }
            }
        }
        let status = wait_child_with_timeout(&mut child, exit_timeout);
        let stderr = collect_stderr(stderr_handle);
        match status {
            Ok(Some(status)) if !status.success() => {
                terminal.send_error(
                    &output_tx,
                    sidecar_process_failed_message(&program, status, &stderr),
                );
            }
            Ok(None) => {
                let _ = child.kill();
                let _ = child.wait();
                terminal.send_error(
                    &output_tx,
                    sidecar_process_timed_out_message(&program, exit_timeout, &stderr),
                );
            }
            Err(err) => {
                terminal.send_error(
                    &output_tx,
                    format!("sidecar wait failed for {}: {err}", program.display()),
                );
            }
            _ if !terminal.sent.load(Ordering::Acquire) => {
                terminal.send_closed(&output_tx, "sidecar_stdout_closed");
            }
            _ => {}
        }
    });
}
