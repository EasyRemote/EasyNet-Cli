// EasyNet CLI — sidecar stream contract
// =====================================
//
// File: src/daemon/plugins/sidecar/stream.rs
// Description: Stream-frame validation and live pump for sidecar streams.

use std::io::{BufRead, BufReader};
use std::path::{Path, PathBuf};
use std::process::{Child, ExitStatus};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Arc;
use std::time::Duration;

use serde_json::Value;
use tokio::sync::broadcast;

use crate::daemon::ability::dispatch::{StreamSource, BIDI_CHANNEL_BOUND};
use crate::daemon::plugins::errors::{PluginHostError, Result};
use crate::daemon::plugins::sidecar::io::{
    collect_stderr, duration_millis, spawn_stderr_reader, wait_child_with_timeout,
    write_sidecar_frame,
};
use crate::daemon::plugins::sidecar::{
    SidecarInvocationEnvelope, SidecarRequestFrame, SidecarResponseFrame,
};

/// Open a live sidecar stream and return immediately with a daemon-owned
/// broadcast receiver.
///
/// Invariant 1: the sidecar receives exactly one `stream_open` frame.
/// Invariant 2: stream items are forwarded as soon as stdout frames arrive.
/// Invariant 3: terminal/error/process-exit paths close the sender exactly
/// once; the Axon stream adapter emits the final terminal event when the
/// broadcast channel closes.
pub(super) fn open_live_stream(
    program: &Path,
    mut child: Child,
    call_id: String,
    invocation: SidecarInvocationEnvelope,
    exit_timeout: Duration,
) -> Result<StreamSource> {
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
        &SidecarRequestFrame::StreamOpen {
            call_id: call_id.clone(),
            invocation,
        },
    )?;
    drop(stdin);

    let (tx, rx) = broadcast::channel::<Value>(BIDI_CHANNEL_BOUND);
    spawn_stream_reader(
        program.to_path_buf(),
        stdout,
        child,
        call_id,
        tx,
        stderr_handle,
        exit_timeout,
    );
    Ok(StreamSource::Live(rx))
}

/// Collect a finite stream and enforce the single-terminal invariant.
///
/// What this is NOT: a live streaming pump. The current sidecar host exposes a
/// bounded snapshot API for stream tests and registration glue; live
/// long-running transport should use the bidi path with an explicit terminal
/// guard.
pub(super) fn collect_stream_snapshot(
    call_id: &str,
    responses: Vec<SidecarResponseFrame>,
) -> Result<Vec<Value>> {
    let mut items = Vec::new();
    let mut terminal_seen = false;
    for response in responses {
        match response {
            SidecarResponseFrame::StreamItem {
                call_id: response_call_id,
                value,
            } if response_call_id == call_id => {
                if terminal_seen {
                    return Err(PluginHostError::SidecarProtocolViolation {
                        message: format!(
                            "sidecar stream {call_id} emitted item after terminal frame"
                        ),
                    });
                }
                items.push(value);
            }
            SidecarResponseFrame::Terminal {
                call_id: response_call_id,
                ..
            } if response_call_id == call_id => {
                if terminal_seen {
                    return Err(PluginHostError::SidecarProtocolViolation {
                        message: format!(
                            "sidecar stream {call_id} emitted multiple terminal frames"
                        ),
                    });
                }
                terminal_seen = true;
            }
            SidecarResponseFrame::Error {
                call_id: response_call_id,
                message,
            } if response_call_id == call_id => {
                if terminal_seen {
                    return Err(PluginHostError::SidecarProtocolViolation {
                        message: format!(
                            "sidecar stream {call_id} emitted error after terminal frame"
                        ),
                    });
                }
                return Err(PluginHostError::SidecarProtocolViolation {
                    message: format!(
                        "sidecar returned stream error for {response_call_id}: {message}"
                    ),
                });
            }
            other => {
                return Err(PluginHostError::SidecarProtocolViolation {
                    message: format!("unexpected stream response frame: {other:?}"),
                });
            }
        }
    }
    if !terminal_seen {
        return Err(PluginHostError::SidecarProtocolViolation {
            message: "sidecar stream ended without terminal frame".to_string(),
        });
    }
    Ok(items)
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

fn send_error_once(tx: &broadcast::Sender<Value>, terminal_sent: &AtomicBool, message: String) {
    if terminal_sent.swap(true, Ordering::AcqRel) {
        return;
    }
    let _ = tx.send(serde_json::json!({
        "type": "error",
        "message": message,
    }));
}

fn spawn_stream_reader(
    program: PathBuf,
    stdout: std::process::ChildStdout,
    mut child: Child,
    call_id: String,
    tx: broadcast::Sender<Value>,
    stderr_handle: Option<std::thread::JoinHandle<String>>,
    exit_timeout: Duration,
) {
    std::thread::spawn(move || {
        let terminal_sent = Arc::new(AtomicBool::new(false));
        for line in BufReader::new(stdout).lines() {
            let line = match line {
                Ok(line) => line,
                Err(err) => {
                    send_error_once(
                        &tx,
                        &terminal_sent,
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
                    send_error_once(
                        &tx,
                        &terminal_sent,
                        format!("sidecar stream frame decode failed: {err}"),
                    );
                    break;
                }
            };
            match frame {
                SidecarResponseFrame::StreamItem {
                    call_id: response_call_id,
                    value,
                } if response_call_id == call_id => {
                    if terminal_sent.load(Ordering::Acquire) {
                        continue;
                    }
                    if tx.send(value).is_err() {
                        break;
                    }
                }
                SidecarResponseFrame::Terminal {
                    call_id: response_call_id,
                    ..
                } if response_call_id == call_id => {
                    terminal_sent.store(true, Ordering::Release);
                    break;
                }
                SidecarResponseFrame::Error {
                    call_id: response_call_id,
                    message,
                } if response_call_id == call_id => {
                    send_error_once(&tx, &terminal_sent, message);
                    break;
                }
                other => {
                    send_error_once(
                        &tx,
                        &terminal_sent,
                        format!("unexpected stream response frame: {other:?}"),
                    );
                    break;
                }
            }
        }
        let status = wait_child_with_timeout(&mut child, exit_timeout);
        let stderr = collect_stderr(stderr_handle);
        match status {
            Ok(Some(status)) if !status.success() => {
                send_error_once(
                    &tx,
                    &terminal_sent,
                    sidecar_process_failed_message(&program, status, &stderr),
                );
            }
            Ok(None) => {
                let _ = child.kill();
                let _ = child.wait();
                send_error_once(
                    &tx,
                    &terminal_sent,
                    sidecar_process_timed_out_message(&program, exit_timeout, &stderr),
                );
            }
            Err(err) => {
                send_error_once(
                    &tx,
                    &terminal_sent,
                    format!("sidecar wait failed for {}: {err}", program.display()),
                );
            }
            _ => {}
        }
    });
}
