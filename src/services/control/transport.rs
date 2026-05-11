// EasyNet CLI — Control-plane transport (UDS / Named Pipe)
// =========================================================
//
// File: src/services/control/transport.rs
// Description: OS-specific listeners behind one enum so the server
//              accept-loop can be written once. Linux and macOS use
//              a Unix Domain Socket; Windows would use a Named Pipe
//              (the path is sketched but the implementation lands in
//              a follow-up PR-DAEMON commit). iOS / Android are
//              out-of-scope for v1.
//
// v1 status — Unix UDS lands here, Named Pipe follow-up
// ------------------------------------------------------
// PR-DAEMON Commit 3 wires the Unix Domain Socket bind, accept, and
// chmod-0600 atomic-replace. The Windows Named Pipe variant is not
// implemented in this commit; calling `bind_default()` on Windows
// returns an explicit "not yet implemented" error. The plan
// authorises that gap because v10.5 R1 §Platform exceptions lists
// non-Linux/macOS platforms as out-of-scope until a Client binding
// for them surfaces.
//
// Why use plain `tokio::net::UnixListener` instead of the
// `interprocess` crate
// -------------------------------------------------------
// `interprocess` would let us share one bind path across UDS and
// Named Pipe. v1 chose tokio's UnixListener directly because (a) the
// dependency surface is smaller, (b) every CI we run today is
// Linux/macOS, and (c) the Windows port is not in scope for this
// commit. When the Windows variant lands, the call site can be
// re-abstracted; the `ControlListener` enum already has a
// `NamedPipe` variant slot reserved.
//
// Filesystem auth
// ---------------
// Bind also chmod's the socket to mode 0600. With the directory
// (`$HOME/.easynet`) already owned by the user and mode 0700-by-
// convention, this gives the IPC plane single-user isolation
// without a bearer token. See docs/design/daemon-layers-v1.md for
// the threat-model writeup.
//
// Author: Silan Hu <silan.hu@u.nus.edu>
// Copyright (c) 2026 EasyNet. All rights reserved.

use std::path::{Path, PathBuf};

#[cfg(not(windows))]
use crate::persistence::config::state_dir;
#[cfg(windows)]
use crate::support::named_pipe::{scoped_pipe_name, PipeListener};

/// Filename for the Unix Domain Socket inside the user's
/// `~/.easynet/` directory. Pinned so the Client FFI library can
/// fall back to it if `control.json` is missing.
pub const UDS_FILENAME: &str = "control.sock";

/// Platform-neutral listener handle. Concrete OS variants live
/// behind `#[cfg]`. The accept loop in `server.rs` matches on this
/// enum, so adding a new variant (e.g. Named Pipe on Windows)
/// produces a compile error at every match site — preventing a
/// silent platform skew.
pub(crate) enum ControlListener {
    #[cfg(unix)]
    Uds(tokio::net::UnixListener),
    #[cfg(windows)]
    NamedPipe(PipeListener),
    /// Windows / iOS / Android etc. — not yet wired. See module
    /// header for plan reference.
    #[allow(dead_code)]
    Unsupported,
}

impl std::fmt::Debug for ControlListener {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            #[cfg(unix)]
            Self::Uds(_) => f.write_str("ControlListener::Uds(<UnixListener>)"),
            #[cfg(windows)]
            Self::NamedPipe(listener) => {
                write!(f, "ControlListener::NamedPipe({})", listener.name())
            }
            Self::Unsupported => f.write_str("ControlListener::Unsupported"),
        }
    }
}

/// Resolution result for the listener's on-disk address. Linux and
/// macOS return a filesystem path; Windows returns a pipe name. The
/// discovery layer (see `discovery.rs`) serialises whichever is
/// present into `control.json` so the client library can dial.
#[derive(Debug, Clone)]
pub enum ControlAddress {
    /// Unix Domain Socket path, e.g. `~/.easynet/control.sock`.
    UdsPath(PathBuf),
    /// Windows Named Pipe name, e.g. `\\.\pipe\easynet-<uid>`.
    #[allow(dead_code)]
    NamedPipe(String),
}

impl ControlAddress {
    /// Borrow the UDS path if this is the UDS variant, else None.
    /// Used by `discovery::write` to populate `socket_path`.
    pub fn as_uds_path(&self) -> Option<&Path> {
        match self {
            Self::UdsPath(p) => Some(p),
            Self::NamedPipe(_) => None,
        }
    }

    /// Borrow the Named Pipe name if this is the Pipe variant.
    pub fn as_pipe_name(&self) -> Option<&str> {
        match self {
            Self::NamedPipe(n) => Some(n.as_str()),
            Self::UdsPath(_) => None,
        }
    }
}

/// Default UDS path for the local control plane: `~/.easynet/control.sock`.
pub fn default_socket_path() -> PathBuf {
    #[cfg(windows)]
    {
        return PathBuf::from(scoped_pipe_name("control"));
    }

    #[cfg(not(windows))]
    state_dir().join(UDS_FILENAME)
}

/// Bind a listener at the default address.
///
/// On Unix:
/// - Ensures the parent directory exists (mode is left as-is; the
///   user is responsible for `~/.easynet/` permissions).
/// - Removes any stale socket file at the path. v1 does not
///   probe-for-liveness against an existing socket; the daemon
///   process supervisor (`easynet self control start`) is expected
///   to have already detected and cleaned a stale daemon. Returning
///   an `EADDRINUSE` here would be a worse UX than a forced unlink.
/// - Binds a `tokio::net::UnixListener`.
/// - Sets the socket file mode to 0600 so other users on the host
///   physically cannot connect.
///
/// On Windows: returns a clear "not yet implemented" error per the
/// v1 platform-scope decision.
pub(crate) fn bind_default() -> anyhow::Result<(ControlListener, ControlAddress)> {
    let path = default_socket_path();
    bind_at(&path)
}

/// Test-friendly variant: bind at an arbitrary path. Same semantics
/// as `bind_default` but skips the `state_dir()` lookup so unit
/// tests can pin the socket inside a temp directory.
pub(crate) fn bind_at(path: &Path) -> anyhow::Result<(ControlListener, ControlAddress)> {
    #[cfg(unix)]
    {
        if let Some(parent) = path.parent() {
            std::fs::create_dir_all(parent)?;
        }
        // Remove stale socket file. See doc comment on bind_default
        // for why we don't try to probe liveness here.
        if path.exists() {
            std::fs::remove_file(path)?;
        }
        let listener = tokio::net::UnixListener::bind(path)?;
        // Tighten file permissions to 0600 so other users can't dial.
        // We do this after bind so the kernel created the file.
        use std::os::unix::fs::PermissionsExt as _;
        let perms = std::fs::Permissions::from_mode(0o600);
        std::fs::set_permissions(path, perms)?;
        Ok((
            ControlListener::Uds(listener),
            ControlAddress::UdsPath(path.to_path_buf()),
        ))
    }

    #[cfg(not(unix))]
    {
        #[cfg(windows)]
        {
            let name = path
                .to_str()
                .ok_or_else(|| anyhow::anyhow!("named-pipe path is not valid UTF-8"))?
                .to_string();
            let listener = PipeListener::bind(name.clone())?;
            return Ok((
                ControlListener::NamedPipe(listener),
                ControlAddress::NamedPipe(name),
            ));
        }

        #[cfg(not(windows))]
        {
            let _ = path;
            anyhow::bail!(
                "control-plane Named Pipe transport not yet implemented in v1 of PR-DAEMON; \
                 v10.5 R1 lists non-Unix platforms as out-of-scope until a Windows \
                 Client binding requests it"
            )
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn unique_tmp() -> PathBuf {
        // SUN_LEN cap: a Unix Domain Socket path on Linux is bounded
        // at ~108 bytes and on macOS at ~104 bytes (the
        // sockaddr_un.sun_path field). The default `std::env::temp_dir()`
        // on macOS is `/var/folders/.../T/` which already eats ~50 of
        // those bytes, leaving no room for a unique-id suffix plus
        // `/test.sock`. Pin the test sandbox to `/tmp/` directly —
        // the kernel guarantees that path stays short.
        let p = PathBuf::from("/tmp").join(format!(
            "ezt-{}-{}",
            std::process::id(),
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .map(|d| d.subsec_nanos())
                .unwrap_or(0),
        ));
        std::fs::create_dir_all(&p).unwrap();
        p
    }

    #[cfg(unix)]
    #[tokio::test]
    async fn bind_at_creates_socket_with_mode_0600() {
        // Bind a UDS at a sandbox path and assert the file exists
        // with mode bits 0600. Regression guard: a future refactor
        // that drops the `set_permissions` call leaves the socket
        // world-readable, which would silently widen the auth
        // model. This test catches that.
        let dir = unique_tmp();
        let p = dir.join("test.sock");
        let (_listener, addr) = bind_at(&p).expect("bind");
        assert!(p.exists(), "socket file did not appear at {}", p.display());
        let meta = std::fs::metadata(&p).unwrap();
        use std::os::unix::fs::PermissionsExt as _;
        // mask off the file-type bits, leave only mode bits
        assert_eq!(meta.permissions().mode() & 0o777, 0o600);
        assert_eq!(addr.as_uds_path(), Some(p.as_path()));
    }

    #[cfg(unix)]
    #[tokio::test]
    async fn bind_at_clears_stale_socket() {
        // Simulate a stale socket left behind by a crashed prior
        // daemon. bind_at must remove and rebind without
        // EADDRINUSE so a restart works without manual cleanup.
        let dir = unique_tmp();
        let p = dir.join("stale.sock");
        std::fs::write(&p, b"stale").unwrap();
        let (_listener, _addr) = bind_at(&p).expect("bind over stale");
        // After bind, the file is a real socket; size is 0 because
        // the prior bytes were truncated by remove+bind.
        let meta = std::fs::metadata(&p).unwrap();
        assert_eq!(meta.len(), 0);
    }

    #[test]
    fn control_address_round_trip_through_accessors() {
        let uds = ControlAddress::UdsPath(PathBuf::from("/tmp/x.sock"));
        let np = ControlAddress::NamedPipe(r"\\.\pipe\easynet-0".into());
        assert_eq!(uds.as_uds_path().unwrap(), Path::new("/tmp/x.sock"));
        assert!(uds.as_pipe_name().is_none());
        assert_eq!(np.as_pipe_name().unwrap(), r"\\.\pipe\easynet-0");
        assert!(np.as_uds_path().is_none());
    }
}
