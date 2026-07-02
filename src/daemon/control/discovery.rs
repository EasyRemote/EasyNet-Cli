// EasyNet CLI — control.json discovery file
// ===========================================
//
// File: src/daemon/control/discovery.rs
// Description: The daemon writes a small JSON "discovery" file at a
//              well-known path (`~/.easynet/control.json`) so the
//              Client FFI library can find the IPC listener without
//              hard-coding a socket path. The lib reads this file at
//              `easynet_init()` time, dials the socket/pipe whose
//              address is encoded inside, and completes a version
//              handshake against the declared `supported_ipc_versions`
//              range.
//
// File permissions
// ----------------
// `control.json` is written atomically with mode `0600` on Unix.
// Windows relies on the current user's profile directory ACL; the
// named pipe still owns the actual transport authentication boundary.
// The file contains local routing metadata (`invocation_endpoint`,
// daemon pid, realm, node id), so Unix permissions are fixed rather
// than left to process umask.
//
// Version negotiation
// -------------------
// `supported_ipc_versions` is a *range* `{ min, max }`, not a single
// value. The lib also knows a range; it picks `min(max_both)` if
// the ranges overlap and fails early with `VERSION_INCOMPATIBLE` if
// they don't. This lets the daemon deprecate an old protocol
// version without a flag-day (ship `{ min: 2, max: 3 }` to drop v1;
// old libs fail at init with a clear message).
//
// Author: Silan Hu <silan.hu@u.nus.edu>
// Copyright (c) 2026 EasyNet. All rights reserved.

use std::io::Write;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicU64, Ordering};

use serde::{Deserialize, Serialize};

use crate::persistence::config::state_dir;

/// Filename inside `~/.easynet/`.
pub const CONTROL_JSON_FILENAME: &str = "control.json";

static TEMP_FILE_COUNTER: AtomicU64 = AtomicU64::new(0);

/// v1 IPC protocol version — the one the daemon actually speaks
/// today. The range emitted into `control.json` is `{ min: 1, max:
/// 1 }`. Bumping this requires either (a) maintaining backward
/// compat over the frames the prior version understood or (b)
/// widening the range with a flag-day plan.
pub const IPC_VERSION_V1: u16 = 1;

/// Contents of `~/.easynet/control.json`. The layout is frozen as
/// of PR-DAEMON; adding a field later must use `#[serde(default)]`
/// so old libs ignore it.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ControlDiscovery {
    /// Absolute path to the Unix Domain Socket (Linux/macOS) or
    /// pipe name (Windows). Exactly one is populated per platform.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub socket_path: Option<PathBuf>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub pipe_name: Option<String>,

    /// Absolute local Invocation endpoint the daemon actually bound.
    ///
    /// This is the product-call endpoint used by `libeasynet_cli`
    /// Invocation ABI handles. It is intentionally advertised here
    /// instead of being derived from `control.json`'s directory:
    /// daemon-config.toml supports custom `uds_path` and test
    /// harnesses can override it through environment, so directory
    /// guessing breaks the ABI contract.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub invocation_endpoint: Option<PathBuf>,

    /// Product identity of the daemon that wrote this discovery file.
    ///
    /// Lifecycle attach uses this to refuse reusing a daemon started
    /// for a different mode, realm, or node. A missing identity is
    /// treated as non-attachable by the SDK start path.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub daemon_identity: Option<DaemonIdentity>,

    /// PID of the running daemon. Clients use this to detect stale
    /// `control.json` files from a crashed daemon.
    pub pid: u32,

    /// Human-readable daemon version, from `CARGO_PKG_VERSION`.
    pub daemon_version: String,

    /// Inclusive IPC version range this daemon accepts. Client
    /// libraries negotiate by intersecting their supported range
    /// with this and picking the maximum.
    pub supported_ipc_versions: IpcVersionRange,

    /// Capability flags the daemon declares. Clients use these to
    /// feature-gate without a second RPC round-trip.
    #[serde(default)]
    pub capability_flags: Vec<String>,

    /// Actual local Pages listener port chosen by the daemon.
    ///
    /// Older daemons did not write this field. Readers must treat
    /// `None` as "unknown" and fall back to their historical default
    /// or omit Pages URLs until Ready.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub pages_port: Option<u16>,
}

/// Identity tuple advertised by the daemon in `control.json`.
///
/// This is not an Axon Invocation identity and it is not a security
/// credential. It is a local lifecycle guard: callers that request a
/// device daemon for `(realm, node_id)` must not silently attach to a
/// hub daemon or a different device process just because the sockets
/// are reachable.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct DaemonIdentity {
    /// Deployment mode string: `device`, `hub`, or `both`.
    pub mode: String,
    /// EasyNet realm served by the daemon.
    pub realm: String,
    /// Runtime node id supplied through `EASYNET_NODE_ID`.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub node_id: Option<String>,
}

#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq)]
pub struct IpcVersionRange {
    pub min: u16,
    pub max: u16,
}

impl IpcVersionRange {
    pub fn single(v: u16) -> Self {
        Self { min: v, max: v }
    }

    /// Compute the overlap with another range. Returns None when
    /// the ranges don't overlap; otherwise returns the intersection.
    pub fn overlap(self, other: IpcVersionRange) -> Option<IpcVersionRange> {
        let lo = self.min.max(other.min);
        let hi = self.max.min(other.max);
        if lo <= hi {
            Some(IpcVersionRange { min: lo, max: hi })
        } else {
            None
        }
    }
}

/// Capability flags the v1 daemon advertises.
pub mod flags {
    /// The daemon exposes boot progress over control.sock.
    pub const BOOT_STATUS: &str = "boot_status";
    /// The daemon exposes local non-product diagnostics over control.sock.
    pub const CONTROL_DIAGNOSTICS: &str = "control_diagnostics";
}

/// Default discovery path. Callers should prefer this over rolling
/// their own join of `state_dir()`.
pub fn default_path() -> PathBuf {
    state_dir().join(CONTROL_JSON_FILENAME)
}

/// Read and parse `~/.easynet/control.json`. Returns `Ok(None)` if
/// the file doesn't exist; returns `Err` for any parse failure (the
/// file existing but unreadable is an operator-visible problem, not
/// an initial-state).
pub fn read(path: &Path) -> anyhow::Result<Option<ControlDiscovery>> {
    if !path.exists() {
        return Ok(None);
    }
    let bytes = std::fs::read(path)?;
    let parsed: ControlDiscovery = serde_json::from_slice(&bytes)
        .map_err(|e| anyhow::anyhow!("control.json at {} is malformed: {e}", path.display()))?;
    Ok(Some(parsed))
}

/// Write `control.json` atomically.
///
/// On Unix the temporary file is created with `0600` before bytes
/// are written, then renamed into place. That keeps the discovery
/// permission contract independent of umask and avoids readers
/// observing partial JSON during daemon Ready updates.
pub fn write(path: &Path, disc: &ControlDiscovery) -> anyhow::Result<()> {
    if let Some(parent) = path.parent() {
        std::fs::create_dir_all(parent)?;
    }
    let bytes = serde_json::to_vec_pretty(disc)?;
    write_atomic(path, &bytes)?;
    Ok(())
}

#[cfg(unix)]
fn write_atomic(path: &Path, bytes: &[u8]) -> anyhow::Result<()> {
    use std::os::unix::fs::{OpenOptionsExt, PermissionsExt};

    let parent = path.parent().unwrap_or_else(|| Path::new("."));
    let file_name = path
        .file_name()
        .and_then(|name| name.to_str())
        .unwrap_or(CONTROL_JSON_FILENAME);
    let counter = TEMP_FILE_COUNTER.fetch_add(1, Ordering::Relaxed);
    let tmp = parent.join(format!(".{file_name}.{}.{counter}.tmp", std::process::id()));
    let mut file = std::fs::OpenOptions::new()
        .create(true)
        .truncate(true)
        .write(true)
        .mode(0o600)
        .open(&tmp)?;
    file.write_all(bytes)?;
    file.write_all(b"\n")?;
    file.sync_all()?;
    drop(file);
    std::fs::set_permissions(&tmp, std::fs::Permissions::from_mode(0o600))?;
    std::fs::rename(&tmp, path)?;
    Ok(())
}

#[cfg(not(unix))]
fn write_atomic(path: &Path, bytes: &[u8]) -> anyhow::Result<()> {
    let parent = path.parent().unwrap_or_else(|| Path::new("."));
    let file_name = path
        .file_name()
        .and_then(|name| name.to_str())
        .unwrap_or(CONTROL_JSON_FILENAME);
    let counter = TEMP_FILE_COUNTER.fetch_add(1, Ordering::Relaxed);
    let tmp = parent.join(format!(".{file_name}.{}.{counter}.tmp", std::process::id()));
    std::fs::write(&tmp, bytes)?;
    std::fs::rename(&tmp, path)?;
    Ok(())
}

/// Remove `control.json` on graceful daemon shutdown.
///
/// Missing files are harmless: a crashed daemon may already have been
/// cleaned by an operator, and the next start overwrites stale discovery
/// state anyway. Other I/O failures are returned so the daemon can log
/// them without changing its exit status.
pub fn remove(path: &Path) -> anyhow::Result<()> {
    match std::fs::remove_file(path) {
        Ok(()) => Ok(()),
        Err(err) if err.kind() == std::io::ErrorKind::NotFound => Ok(()),
        Err(err) => Err(err.into()),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Produce a unique sandbox directory under the OS temp dir. We
    /// avoid `tempfile` as a dependency — the rest of this crate
    /// uses the same `env::temp_dir` + pid/time pattern (see
    /// `persistence::config` tests) so a test failure doesn't point
    /// at a missing dev-dep.
    fn unique_tmp() -> PathBuf {
        static TEST_DIR_COUNTER: AtomicU64 = AtomicU64::new(0);
        let counter = TEST_DIR_COUNTER.fetch_add(1, Ordering::Relaxed);
        let p = std::env::temp_dir().join(format!(
            "easynet-control-discovery-{}-{}-{}",
            std::process::id(),
            counter,
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .map(|d| d.as_nanos())
                .unwrap_or(0),
        ));
        std::fs::create_dir_all(&p).unwrap();
        p
    }

    fn sample() -> ControlDiscovery {
        ControlDiscovery {
            socket_path: Some(PathBuf::from("/tmp/x.sock")),
            pipe_name: None,
            invocation_endpoint: Some(PathBuf::from("/tmp/custom-daemon.sock")),
            daemon_identity: Some(DaemonIdentity {
                mode: "device".into(),
                realm: "tenant-a".into(),
                node_id: Some("node-a".into()),
            }),
            pid: 12345,
            daemon_version: "1.17.1".into(),
            supported_ipc_versions: IpcVersionRange::single(IPC_VERSION_V1),
            capability_flags: vec![flags::BOOT_STATUS.into()],
            pages_port: Some(8787),
        }
    }

    #[test]
    fn read_missing_file_returns_none_not_error() {
        // A missing control.json is the normal "daemon not running"
        // state; the client library must be able to distinguish it
        // from a corrupt file. `None` vs `Err` is the type-level
        // way we encode that.
        let dir = unique_tmp();
        let p = dir.join(CONTROL_JSON_FILENAME);
        let got = read(&p).unwrap();
        assert!(got.is_none());
    }

    #[test]
    fn round_trip_preserves_version_range_and_flags() {
        // Write then read must yield the same struct. Regression
        // guard for a field-rename or serde-attr change that
        // silently drops a field on one side.
        let dir = unique_tmp();
        let p = dir.join(CONTROL_JSON_FILENAME);
        let disc = sample();
        write(&p, &disc).unwrap();
        let back = read(&p).unwrap().expect("file was written above");
        assert_eq!(back.pid, disc.pid);
        assert_eq!(back.supported_ipc_versions, disc.supported_ipc_versions);
        assert_eq!(back.capability_flags, disc.capability_flags);
        assert_eq!(back.pages_port, Some(8787));
        assert_eq!(
            back.invocation_endpoint,
            Some(PathBuf::from("/tmp/custom-daemon.sock"))
        );
        assert_eq!(back.daemon_identity, disc.daemon_identity);
    }

    #[cfg(unix)]
    #[test]
    fn write_sets_control_json_mode_0600() {
        use std::os::unix::fs::PermissionsExt;

        let dir = unique_tmp();
        let p = dir.join(CONTROL_JSON_FILENAME);
        write(&p, &sample()).unwrap();
        let mode = std::fs::metadata(&p).unwrap().permissions().mode() & 0o777;
        assert_eq!(mode, 0o600);
    }

    #[test]
    fn remove_deletes_file_and_ignores_missing() {
        let dir = unique_tmp();
        let p = dir.join(CONTROL_JSON_FILENAME);
        write(&p, &sample()).unwrap();
        assert!(p.exists());

        remove(&p).unwrap();
        assert!(!p.exists());
        remove(&p).unwrap();
    }

    #[test]
    fn version_overlap_picks_intersection() {
        // A daemon that supports {1,3} and a lib that supports
        // {2,4} must negotiate to {2,3}. The lib then picks the
        // maximum common version (3). Pin both steps.
        let daemon = IpcVersionRange { min: 1, max: 3 };
        let lib = IpcVersionRange { min: 2, max: 4 };
        let overlap = daemon.overlap(lib).unwrap();
        assert_eq!(overlap, IpcVersionRange { min: 2, max: 3 });
        assert_eq!(overlap.max, 3);
    }

    #[test]
    fn disjoint_version_ranges_do_not_overlap() {
        let a = IpcVersionRange { min: 1, max: 1 };
        let b = IpcVersionRange { min: 2, max: 3 };
        assert!(a.overlap(b).is_none());
    }

    #[test]
    fn malformed_control_json_is_a_hard_error_not_silent_none() {
        // If the file exists but the bytes aren't valid JSON, we
        // surface the error — silently falling back to "daemon not
        // running" would mask an operator-visible corruption.
        let dir = unique_tmp();
        let p = dir.join(CONTROL_JSON_FILENAME);
        std::fs::write(&p, b"not json").unwrap();
        let err = read(&p).unwrap_err();
        assert!(format!("{err}").contains("malformed"));
    }
}
