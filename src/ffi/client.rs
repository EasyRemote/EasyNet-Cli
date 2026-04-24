// EasyNet CLI — FFI-side IPC client (library internal)
// ======================================================
//
// File: src/ffi/client.rs
// Description: The library-internal client that dials the daemon's
//              local Control-plane socket/pipe, negotiates an IPC
//              version, and exchanges framed JSON messages.
//              Exposed only within the `ffi` module; Client FFI
//              functions construct one on `easynet_init` and hold
//              it inside the `ClientSession` registry.
//
// Why this module, distinct from services::control::server
// --------------------------------------------------------
// `server.rs` is the daemon side of the IPC; `client.rs` is the
// lib side. They speak the same wire format (`frames::IncomingFrame`
// / `frames::OutgoingFrame`) but the code shape is inverted:
// server has an accept loop; client has a connect + read/write
// split. Keeping them in sibling directories makes the grep
// `use crate::services::control` visible in one place each.
//
// v1 status — skeleton
// --------------------
// `connect()` returns an explicit skeleton error. The follow-up
// PR-DAEMON commit lands the real UDS / Named-Pipe connect +
// version handshake. The type signatures are already in place so
// the `easynet_init` / `easynet_ability_invoke` bodies can be
// written against them on the next commit without restructuring.
//
// Author: Silan Hu <silan.hu@u.nus.edu>
// Copyright (c) 2026 EasyNet. All rights reserved.

use std::path::Path;

use crate::services::control::discovery::{ControlDiscovery, IpcVersionRange, IPC_VERSION_V1};
use crate::services::control::frames::{IncomingFrame, OutgoingFrame};

/// Handle to an open IPC connection to the daemon. v1 is a
/// skeleton struct; the follow-up commit will hold either a
/// `tokio::net::UnixStream` (Linux/macOS) or a Named-Pipe handle
/// (Windows) plus a framed codec.
///
/// `Debug` is derived so test `unwrap_err` / `unwrap` on a
/// `Result<IpcClient, _>` works; the skeleton struct has no OS
/// handles yet so the derive is fine today. When the real
/// transport lands and this struct grows a `UnixStream` field,
/// switch to a manual `impl Debug` that prints only the fields
/// that implement Debug.
#[derive(Debug)]
pub struct IpcClient {
    /// Negotiated protocol version after the handshake.
    #[allow(dead_code)]
    pub ipc_version: u16,
    /// Observed daemon `control.json`, kept for diagnostics.
    #[allow(dead_code)]
    pub daemon_discovery: ControlDiscovery,
}

/// v1 supported version range on the lib side. Must overlap with
/// the daemon's advertised range for `connect()` to succeed.
pub fn supported_versions() -> IpcVersionRange {
    IpcVersionRange::single(IPC_VERSION_V1)
}

/// Dial the daemon at `control_json_path` and perform the IPC
/// handshake. Returns `Ok(IpcClient)` on success or a structured
/// error describing what failed (file missing, version mismatch,
/// connect refused, handshake timeout).
///
/// v1 always returns a skeleton error. This lets the Client FFI
/// binding compile against the symbol; real I/O lands in a
/// follow-up PR-DAEMON commit.
pub fn connect(_control_json_path: &Path) -> anyhow::Result<IpcClient> {
    anyhow::bail!(
        "FFI IPC client is a skeleton in v1 of PR-DAEMON; \
         real UDS/Named-Pipe connect + handshake lands in a follow-up commit"
    )
}

impl IpcClient {
    /// Round-trip one `Invoke` frame: write the inbound frame,
    /// read exactly one outbound frame, return it. v1 skeleton
    /// body; real codec comes with the transport work.
    pub fn round_trip(&mut self, _req: IncomingFrame) -> anyhow::Result<OutgoingFrame> {
        anyhow::bail!(
            "IpcClient::round_trip is a skeleton in v1 of PR-DAEMON; \
             the real implementation lands with the transport follow-up"
        )
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::path::PathBuf;

    #[test]
    fn supported_versions_is_the_v1_single_point_range() {
        // The lib's declared v1 support is exactly {IPC_VERSION_V1}.
        // A future change that widens the range (to drop support
        // for an older daemon) must touch this test.
        let r = supported_versions();
        assert_eq!(r.min, IPC_VERSION_V1);
        assert_eq!(r.max, IPC_VERSION_V1);
    }

    #[test]
    fn connect_returns_explicit_skeleton_error() {
        // A Client that attempts to call into the ABI before the
        // transport commit lands gets a loud error mentioning
        // "skeleton" so the failure is not mistaken for a daemon-
        // side production bug.
        let err = connect(&PathBuf::from("/tmp/does-not-matter.json")).unwrap_err();
        assert!(format!("{err}").contains("skeleton"));
    }
}
