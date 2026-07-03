// EasyNet CLI — sidecar process I/O
// =================================
//
// File: src/daemon/plugins/sidecar/io.rs
// Description: Low-level stdin/stdout/stderr and child-process wait helpers.

use std::io::{BufRead, BufReader, Read, Write};
use std::path::{Path, PathBuf};
use std::process::{Child, ChildStderr, ChildStdin, ChildStdout, ExitStatus};
use std::time::{Duration, Instant};

use crate::daemon::plugins::errors::{PluginHostError, Result};
use crate::daemon::plugins::sidecar::{SidecarRequestFrame, SidecarResponseFrame};

const SIDECAR_WAIT_POLL: Duration = Duration::from_millis(10);

/// Write one newline-delimited JSON request frame to sidecar stdin.
pub(super) fn write_sidecar_frame(
    program: &Path,
    stdin: &mut ChildStdin,
    frame: &SidecarRequestFrame,
) -> Result<()> {
    serde_json::to_writer(&mut *stdin, frame)
        .map_err(|source| PluginHostError::SidecarFrameEncodeFailed { source })?;
    stdin
        .write_all(b"\n")
        .map_err(|source| PluginHostError::WriteFailed {
            path: program.to_path_buf(),
            source,
        })?;
    stdin
        .flush()
        .map_err(|source| PluginHostError::WriteFailed {
            path: program.to_path_buf(),
            source,
        })
}

/// Read stderr concurrently so a blocked stderr pipe cannot stall the sidecar.
pub(super) fn spawn_stderr_reader(stderr: ChildStderr) -> std::thread::JoinHandle<String> {
    std::thread::spawn(move || {
        let mut reader = BufReader::new(stderr);
        let mut stderr = String::new();
        let _ = reader.read_to_string(&mut stderr);
        stderr
    })
}

/// Join an optional stderr reader and return best-effort captured diagnostics.
pub(super) fn collect_stderr(handle: Option<std::thread::JoinHandle<String>>) -> String {
    handle
        .and_then(|handle| handle.join().ok())
        .unwrap_or_default()
}

/// Read every stdout line into typed sidecar response frames.
pub(super) fn spawn_stdout_frame_reader(
    program: PathBuf,
    stdout: ChildStdout,
) -> std::thread::JoinHandle<Result<Vec<SidecarResponseFrame>>> {
    std::thread::spawn(move || {
        let mut frames = Vec::new();
        for line in BufReader::new(stdout).lines() {
            let line = line.map_err(|source| PluginHostError::ReadFailed {
                path: program.clone(),
                source,
            })?;
            if line.trim().is_empty() {
                continue;
            }
            frames.push(
                serde_json::from_str(&line)
                    .map_err(|source| PluginHostError::SidecarFrameDecodeFailed { source })?,
            );
        }
        Ok(frames)
    })
}

/// Join the stdout reader and turn thread panic into a protocol violation.
pub(super) fn join_stdout_frame_reader(
    handle: std::thread::JoinHandle<Result<Vec<SidecarResponseFrame>>>,
) -> Result<Vec<SidecarResponseFrame>> {
    handle
        .join()
        .map_err(|_| PluginHostError::SidecarProtocolViolation {
            message: "sidecar stdout reader panicked".to_string(),
        })?
}

/// Wait for child exit with a daemon-owned timeout budget.
pub(super) fn wait_child_with_timeout(
    child: &mut Child,
    timeout: Duration,
) -> std::io::Result<Option<ExitStatus>> {
    let deadline = Instant::now() + timeout;
    loop {
        if let Some(status) = child.try_wait()? {
            return Ok(Some(status));
        }
        if Instant::now() >= deadline {
            return Ok(None);
        }
        std::thread::sleep(SIDECAR_WAIT_POLL);
    }
}

/// Convert duration to a saturated millisecond count for operator diagnostics.
pub(super) fn duration_millis(duration: Duration) -> u64 {
    duration.as_millis().try_into().unwrap_or(u64::MAX)
}
