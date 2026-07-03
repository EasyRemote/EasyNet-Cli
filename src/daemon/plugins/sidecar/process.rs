// EasyNet CLI — sidecar process runtime
// =====================================
//
// File: src/daemon/plugins/sidecar/process.rs
// Description: Process spawning and unary/stream dispatch for sidecar plugins.

use std::process::{Child, Command, Stdio};
use std::time::Duration;

use serde_json::Value;

use crate::daemon::ability::dispatch::{BidiSource, StreamSource};
use crate::daemon::plugins::errors::{PluginHostError, Result};
use crate::daemon::plugins::sidecar::bidi::open_bidi_session;
use crate::daemon::plugins::sidecar::command::SidecarCommand;
use crate::daemon::plugins::sidecar::command::SidecarExecutionModel;
use crate::daemon::plugins::sidecar::io::{
    collect_stderr, duration_millis, join_stdout_frame_reader, spawn_stderr_reader,
    spawn_stdout_frame_reader, wait_child_with_timeout, write_sidecar_frame,
};
use crate::daemon::plugins::sidecar::stream::{collect_stream_snapshot, open_live_stream};
use crate::daemon::plugins::sidecar::{
    SidecarInvocationEnvelope, SidecarRequestFrame, SidecarResponseFrame,
};

const DEFAULT_SIDECAR_RPC_TIMEOUT: Duration = Duration::from_secs(10);
const DEFAULT_SIDECAR_BIDI_EXIT_TIMEOUT: Duration = Duration::from_secs(5);

/// Process-isolated sidecar runtime boundary.
///
/// Invariant 1: every request frame carries a daemon-built invocation envelope.
/// Invariant 2: exactly one terminal outcome is accepted for unary invoke:
/// `Result` or `Error`; stream/bidi frames are rejected on the rpc path.
/// Invariant 3: the sidecar process is a child, not an in-daemon extension; a
/// process failure becomes a typed plugin host error.
pub struct SidecarRuntimeHost {
    command: SidecarCommand,
    limits: SidecarRuntimeLimits,
}

/// Invocation-time limits owned by the daemon sidecar host.
///
/// These limits are deliberately separate from package installation metadata.
/// The package index remains a pure installed-package view; runtime budgets are
/// applied only when a process-backed ability is actually invoked.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct SidecarRuntimeLimits {
    rpc_timeout: Duration,
    bidi_exit_timeout: Duration,
}

impl SidecarRuntimeLimits {
    pub fn new(rpc_timeout: Duration, bidi_exit_timeout: Duration) -> Self {
        Self {
            rpc_timeout,
            bidi_exit_timeout,
        }
    }

    pub fn rpc_timeout(&self) -> Duration {
        self.rpc_timeout
    }

    pub fn bidi_exit_timeout(&self) -> Duration {
        self.bidi_exit_timeout
    }
}

impl Default for SidecarRuntimeLimits {
    fn default() -> Self {
        Self {
            rpc_timeout: DEFAULT_SIDECAR_RPC_TIMEOUT,
            bidi_exit_timeout: DEFAULT_SIDECAR_BIDI_EXIT_TIMEOUT,
        }
    }
}

impl SidecarRuntimeHost {
    /// Construct a runtime host for one executable sidecar command.
    pub fn new(command: SidecarCommand) -> Self {
        Self {
            command,
            limits: SidecarRuntimeLimits::default(),
        }
    }

    /// Construct a runtime host with explicit daemon-owned call limits.
    pub fn with_limits(command: SidecarCommand, limits: SidecarRuntimeLimits) -> Self {
        Self { command, limits }
    }

    /// Process lifecycle model for this host.
    pub fn execution_model(&self) -> SidecarExecutionModel {
        self.command.execution_model()
    }

    /// Invoke a unary sidecar ability with the full daemon invocation envelope.
    pub fn invoke_rpc(
        &self,
        call_id: impl Into<String>,
        invocation: SidecarInvocationEnvelope,
    ) -> Result<Value> {
        let call_id = call_id.into();
        let mut responses = self.exchange(SidecarRequestFrame::Invoke {
            call_id: call_id.clone(),
            invocation,
        })?;
        if responses.len() != 1 {
            return Err(PluginHostError::SidecarProtocolViolation {
                message: format!(
                    "rpc sidecar returned {} frames; expected exactly one",
                    responses.len()
                ),
            });
        }
        let response = responses.remove(0);
        match response {
            SidecarResponseFrame::Result {
                call_id: response_call_id,
                value,
            } if response_call_id == call_id => Ok(value),
            SidecarResponseFrame::Error {
                call_id: response_call_id,
                message,
            } if response_call_id == call_id => Err(PluginHostError::SidecarProtocolViolation {
                message: format!("sidecar returned error for {response_call_id}: {message}"),
            }),
            other => Err(PluginHostError::SidecarProtocolViolation {
                message: format!("unexpected rpc response frame: {other:?}"),
            }),
        }
    }

    /// Invoke a finite sidecar stream and collect items until a terminal frame.
    pub fn invoke_stream_snapshot(
        &self,
        call_id: impl Into<String>,
        invocation: SidecarInvocationEnvelope,
    ) -> Result<Vec<Value>> {
        let call_id = call_id.into();
        let responses = self.exchange(SidecarRequestFrame::StreamOpen {
            call_id: call_id.clone(),
            invocation,
        })?;
        collect_stream_snapshot(&call_id, responses)
    }

    /// Open a live sidecar stream without waiting for process exit.
    pub fn open_stream(
        &self,
        call_id: impl Into<String>,
        invocation: SidecarInvocationEnvelope,
    ) -> Result<StreamSource> {
        let call_id = call_id.into();
        let child = self.spawn()?;
        open_live_stream(
            self.command.program(),
            child,
            call_id,
            invocation,
            self.limits.bidi_exit_timeout(),
        )
    }

    /// Open a live bidirectional sidecar session.
    ///
    /// The daemon owns the session channels. The child process only sees JSON
    /// frames on stdin/stdout and never receives direct access to daemon
    /// protocol state.
    pub fn open_bidi(
        &self,
        call_id: impl Into<String>,
        invocation: SidecarInvocationEnvelope,
    ) -> Result<BidiSource> {
        let call_id = call_id.into();
        let child = self.spawn()?;
        open_bidi_session(
            self.command.program(),
            child,
            call_id,
            invocation,
            self.limits.bidi_exit_timeout(),
        )
    }

    fn exchange(&self, request: SidecarRequestFrame) -> Result<Vec<SidecarResponseFrame>> {
        let mut child = self.spawn()?;
        {
            let stdin =
                child
                    .stdin
                    .as_mut()
                    .ok_or_else(|| PluginHostError::SidecarStdinUnavailable {
                        program: self.command.program().to_path_buf(),
                    })?;
            write_sidecar_frame(self.command.program(), stdin, &request)?;
        }
        drop(child.stdin.take());

        let stdout =
            child
                .stdout
                .take()
                .ok_or_else(|| PluginHostError::SidecarStdoutUnavailable {
                    program: self.command.program().to_path_buf(),
                })?;
        let stderr_handle = child.stderr.take().map(spawn_stderr_reader);
        let stdout_handle = spawn_stdout_frame_reader(self.command.program().to_path_buf(), stdout);
        let status =
            wait_child_with_timeout(&mut child, self.limits.rpc_timeout()).map_err(|source| {
                PluginHostError::ReadFailed {
                    path: self.command.program().to_path_buf(),
                    source,
                }
            })?;
        let status = match status {
            Some(status) => status,
            None => {
                let _ = child.kill();
                let _ = child.wait();
                let _ = stdout_handle.join();
                let stderr = collect_stderr(stderr_handle);
                return Err(PluginHostError::SidecarProcessTimedOut {
                    program: self.command.program().to_path_buf(),
                    timeout_ms: duration_millis(self.limits.rpc_timeout()),
                    stderr,
                });
            }
        };
        let frames = join_stdout_frame_reader(stdout_handle)?;
        let stderr = collect_stderr(stderr_handle);
        if !status.success() {
            return Err(PluginHostError::SidecarProcessFailed {
                program: self.command.program().to_path_buf(),
                status,
                stderr,
            });
        }
        if frames.is_empty() {
            return Err(PluginHostError::SidecarProtocolViolation {
                message: "sidecar returned no response frame".to_string(),
            });
        }
        Ok(frames)
    }

    fn spawn(&self) -> Result<Child> {
        Command::new(self.command.program())
            .args(self.command.args())
            .stdin(Stdio::piped())
            .stdout(Stdio::piped())
            .stderr(Stdio::piped())
            .spawn()
            .map_err(|source| PluginHostError::SidecarSpawnFailed {
                program: self.command.program().to_path_buf(),
                source,
            })
    }
}
