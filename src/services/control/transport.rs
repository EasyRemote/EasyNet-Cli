// EasyNet CLI — Control-plane transport abstraction
// ===================================================
//
// File: src/services/control/transport.rs
// Description: OS-specific listener types live behind one trait so
//              the server accept-loop can be written once. Linux and
//              macOS use a Unix Domain Socket; Windows uses a Named
//              Pipe; iOS and Android are out-of-scope for v1 and
//              fall through to an explicit `unsupported` error.
//
// v1 status — skeleton
// --------------------
// This file intentionally ships the *trait + type names* without an
// I/O implementation. The follow-up commit inside PR-DAEMON lands
// real `tokio::net::UnixListener` / `tokio::net::windows::named_pipe`
// wiring. Shipping the trait now keeps the accept-loop in
// `server.rs` compilable and lets the ability-proxy tests use an
// in-memory `MockTransport` without reaching for the real OS APIs.
//
// Why one trait, not two separate impl files
// ------------------------------------------
// The accept-loop shape is the same on both platforms: bind,
// accept-forever, spawn per-connection. The per-connection I/O is
// also the same: framed read/write on a byte stream. Abstracting at
// the byte-stream level means `server.rs` does not `#[cfg(unix)]`
// its own body; only the constructor chooses a variant.
//
// Author: Silan Hu <silan.hu@u.nus.edu>
// Copyright (c) 2026 EasyNet. All rights reserved.

use std::path::PathBuf;

/// Platform-neutral handle to a local IPC listener. The server
/// accept-loop calls `accept()` in a loop and hands each connection
/// to a per-connection task.
///
/// v1 type is concrete (no generic parameters) to keep the server
/// loop monomorphic. The inner variant is chosen at construction
/// time by `ControlListener::bind()`.
///
/// `Debug` is derived manually (not via the macro) because the
/// follow-up variants `Uds(UnixListener)` and `NamedPipe(...)`
/// hold OS handles that do not implement `Debug`. We print the
/// variant name only, which is what test `unwrap_err` assertions
/// and log lines want.
pub enum ControlListener {
    /// Placeholder variant until the OS wiring lands. Exists so
    /// `ControlListener` is non-trivial to match on; ensures we
    /// cannot forget to handle "real" variants when they arrive.
    Unbound,
    // Future variants:
    //   #[cfg(unix)]  Uds(tokio::net::UnixListener),
    //   #[cfg(windows)] NamedPipe(NamedPipeServerBuilder),
}

impl std::fmt::Debug for ControlListener {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Unbound => f.write_str("ControlListener::Unbound"),
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
    #[allow(dead_code)]
    UdsPath(PathBuf),
    /// Windows Named Pipe name, e.g. `\\.\pipe\easynet-<uid>`.
    #[allow(dead_code)]
    NamedPipe(String),
}

/// Bind a listener at the well-known local-control address. v1
/// returns `Unbound` and does not perform I/O; follow-up PR-DAEMON
/// commits land the real UDS / Named Pipe bind.
///
/// Intentional design: the signature returns `Result` so the
/// follow-up implementation (which can fail with `EADDRINUSE`,
/// stale-socket errors, etc.) is a drop-in replacement.
pub fn bind_default() -> anyhow::Result<(ControlListener, ControlAddress)> {
    anyhow::bail!(
        "control-plane transport is a skeleton in v1 of PR-DAEMON; \
         the real UDS/Named-Pipe wiring is a follow-up commit"
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn bind_default_is_explicit_skeleton_error() {
        // The bind should return a clear "not yet implemented" error,
        // not an OS-level error that looks like a production bug.
        let err = bind_default().unwrap_err();
        let msg = format!("{err}");
        assert!(
            msg.contains("skeleton"),
            "expected skeleton message, got: {msg}"
        );
    }

    #[test]
    fn control_address_uds_and_named_pipe_do_not_conflate() {
        // Copy-paste regression guard: the two variants must not
        // accept each other's constructor values. Rust's enum
        // tagging gives us this at compile time, but we pin the
        // semantic here so a future refactor (e.g. "let's store
        // both as `String`") can't erase the distinction.
        let uds = ControlAddress::UdsPath(PathBuf::from("/tmp/x.sock"));
        let np = ControlAddress::NamedPipe(r"\\.\pipe\easynet-0".into());
        // Pin the variant identity via matches!:
        assert!(matches!(uds, ControlAddress::UdsPath(_)));
        assert!(matches!(np, ControlAddress::NamedPipe(_)));
    }
}
