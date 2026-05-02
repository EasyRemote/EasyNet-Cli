// EasyNet CLI — FFI-side IPC client (library internal)
// ======================================================
//
// File: src/ffi/client.rs
// Description: The library-internal client that dials the daemon's
//              local Control-plane socket, exchanges framed JSON
//              messages, and returns one `OutgoingFrame` per
//              `IncomingFrame`. Owned by `ClientSession`; never
//              directly visible across the C ABI.
//
// Why this module, distinct from services::control::server
// --------------------------------------------------------
// `server.rs` is the daemon side; `client.rs` is the lib side. Both
// speak `frames::IncomingFrame` / `frames::OutgoingFrame` over a
// `LengthDelimitedCodec`-framed UDS, but the code shape is inverted:
// server has an accept loop; client has a single connect + the
// per-call read/write split.
//
// v1 status — real I/O lands in PR-DAEMON Commit 4
// -------------------------------------------------
// `connect()` reads `~/.easynet/control.json`, validates the
// `supported_ipc_versions` overlap with the lib's range, and opens
// a `tokio::net::UnixStream` on the discovered socket. v1 does NOT
// run a wire-level handshake frame yet — the daemon's accept loop
// has no handshake stage either, so adding one client-side would be
// a one-sided contract. PR-INVOCATION-EXEC-UNITY (or a focused
// follow-up) lands the explicit handshake. For v1, "version
// negotiation" = the discovery overlap check.
//
// `round_trip()` writes one length-prefixed JSON frame and reads
// exactly one back. The daemon's `serve_connection` returns one
// `OutgoingFrame` per `IncomingFrame` for both `Invoke` and the
// (skeleton) `Subscribe` / `Cancel` paths, so this 1:1 model is
// correct for the RPC `easynet_ability_invoke` flow. Streaming
// (`easynet_ability_subscribe`) lands in a follow-up commit because
// it needs a long-lived reader task and a frame channel back to the
// FFI callback.
//
// Author: Silan Hu <silan.hu@u.nus.edu>
// Copyright (c) 2026 EasyNet. All rights reserved.

use std::path::Path;

use bytes::Bytes;
use futures::{SinkExt, StreamExt};
use tokio::net::UnixStream;
use tokio_util::codec::{Framed, LengthDelimitedCodec};

use crate::services::control::discovery::{
    self, ControlDiscovery, IpcVersionRange, IPC_VERSION_V1,
};
use crate::services::control::frames::{IncomingFrame, OutgoingFrame};

/// Open IPC connection to the daemon, owned by a `ClientSession`.
///
/// We hold the framed stream behind the type alias for readability;
/// every read and write goes through the same codec the server uses
/// (4-byte little-endian length prefix + JSON payload).
type FramedUds = Framed<UnixStream, LengthDelimitedCodec>;

pub struct IpcClient {
    /// Negotiated protocol version. v1 always equals
    /// `IPC_VERSION_V1`; the field exists so the upcoming wire
    /// handshake can populate it without changing this struct.
    pub ipc_version: u16,
    /// Observed daemon `control.json`, kept for diagnostics so an
    /// operator can see "this handle dialled which daemon" in the
    /// error path.
    pub daemon_discovery: ControlDiscovery,
    /// The framed UDS connection. Held inside the same struct as
    /// the discovery snapshot so a `round_trip` call has both
    /// pieces in scope without re-reading control.json.
    framed: FramedUds,
}

// Manual Debug because `Framed<UnixStream, _>` is not Debug.
impl std::fmt::Debug for IpcClient {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("IpcClient")
            .field("ipc_version", &self.ipc_version)
            .field("daemon_pid", &self.daemon_discovery.pid)
            .finish_non_exhaustive()
    }
}

/// v1 supported version range on the lib side. Must overlap with
/// the daemon's advertised range for `connect()` to succeed.
pub fn supported_versions() -> IpcVersionRange {
    IpcVersionRange::single(IPC_VERSION_V1)
}

/// Read `control.json` at `control_json_path`, dial the discovered
/// UDS, validate the IPC version overlap, and return an open
/// `IpcClient`.
///
/// Errors:
/// - control.json missing or unreadable (`ERR_DAEMON_DOWN`-mapped
///   message in caller).
/// - control.json reports no UDS path (e.g. v1 saw a Named-Pipe-
///   only daemon — not yet possible in tree, but defended against).
/// - version ranges do not overlap (`ERR_VERSION_INCOMPATIBLE`).
/// - connect refused (`ERR_DAEMON_DOWN`).
pub async fn connect(control_json_path: &Path) -> anyhow::Result<IpcClient> {
    let disc = discovery::read(control_json_path)?.ok_or_else(|| {
        anyhow::anyhow!(
            "FFI client: control.json not found at {} — is `easynet-daemon` running?",
            control_json_path.display()
        )
    })?;

    // Version overlap check. The lib advertises its supported range
    // via `supported_versions()`; intersect with the daemon's
    // `supported_ipc_versions` field. No overlap = hard failure.
    let lib_range = supported_versions();
    let chosen = lib_range
        .overlap(disc.supported_ipc_versions)
        .ok_or_else(|| {
            anyhow::anyhow!(
                "FFI client: IPC version negotiation failed. \
                 lib supports {:?}, daemon supports {:?}",
                lib_range,
                disc.supported_ipc_versions,
            )
        })?;
    // Pick the highest version both sides support — `IpcVersionRange::overlap`
    // returns the intersection range; `max` is the agreed protocol version.
    let chosen_version = chosen.max;

    let socket_path = disc.socket_path.clone().ok_or_else(|| {
        anyhow::anyhow!(
            "FFI client: control.json reports no UDS socket_path; \
             v1 only supports Unix Domain Socket transport"
        )
    })?;

    let stream = UnixStream::connect(&socket_path).await.map_err(|e| {
        anyhow::anyhow!(
            "FFI client: connect to {} failed: {e}",
            socket_path.display()
        )
    })?;

    // Same codec the daemon uses. Default 8 MiB max frame size is
    // ample for v1 JSON envelopes; v2 may tighten when proto binary
    // payloads come in.
    let codec = LengthDelimitedCodec::builder().little_endian().new_codec();
    let framed = Framed::new(stream, codec);

    Ok(IpcClient {
        ipc_version: chosen_version,
        daemon_discovery: disc,
        framed,
    })
}

impl IpcClient {
    /// Send one `IncomingFrame` and read exactly one
    /// `OutgoingFrame`. Used by `easynet_ability_invoke` for the
    /// RPC path. The daemon writes exactly one response per
    /// request (Invoke→Result/Error, Cancel→Error or terminal),
    /// so this 1:1 contract holds for v1.
    ///
    /// Errors:
    /// - serde_json::to_vec fails (impossible for a valid
    ///   `IncomingFrame`; defended against because Rust JSON has
    ///   non-finite-float pitfalls outside the tag fields).
    /// - the framed write fails (peer closed).
    /// - the framed read returns `None` before a frame arrived
    ///   (peer closed mid-handshake).
    /// - the response bytes do not deserialize as `OutgoingFrame`
    ///   (protocol violation by the daemon).
    pub async fn round_trip(&mut self, req: IncomingFrame) -> anyhow::Result<OutgoingFrame> {
        let bytes = serde_json::to_vec(&req)
            .map_err(|e| anyhow::anyhow!("FFI client: encode IncomingFrame failed: {e}"))?;
        self.framed
            .send(Bytes::from(bytes))
            .await
            .map_err(|e| anyhow::anyhow!("FFI client: send frame failed: {e}"))?;

        let resp_bytes = self
            .framed
            .next()
            .await
            .ok_or_else(|| {
                anyhow::anyhow!("FFI client: daemon closed the connection before responding")
            })?
            .map_err(|e| anyhow::anyhow!("FFI client: read frame failed: {e}"))?;

        let resp: OutgoingFrame = serde_json::from_slice(&resp_bytes)
            .map_err(|e| anyhow::anyhow!("FFI client: decode OutgoingFrame failed: {e}"))?;
        Ok(resp)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::runtime::gateway::NoopGateway;
    use crate::runtime::kernel::Kernel;
    use crate::runtime::kernel_api::KernelApi;
    use crate::services::control::ability_proxy::AbilityProxy;
    use crate::services::control::discovery::{flags, ControlDiscovery, IpcVersionRange};
    use crate::services::control::server;
    use crate::services::control::transport::{self, ControlListener};
    use std::path::PathBuf;
    use std::sync::Arc;

    /// Short-prefix temp dir; see same comment in services/control/{server,transport}.rs.
    /// macOS `SUN_LEN` ~= 104 bytes for sockaddr_un.sun_path; the default
    /// `std::env::temp_dir()` already eats ~50 bytes which leaves no
    /// room for a sub-dir + `.sock` filename.
    fn unique_tmp() -> PathBuf {
        let now = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .map(|d| d.as_nanos())
            .unwrap_or(0);
        let p = PathBuf::from(format!("/tmp/eznt-cli-{}-{}", std::process::id(), now));
        std::fs::create_dir_all(&p).unwrap();
        p
    }

    fn make_kernel() -> Arc<dyn KernelApi> {
        Arc::new(Kernel::new(Arc::new(NoopGateway::new())))
    }

    #[test]
    fn supported_versions_is_the_v1_single_point_range() {
        // The lib's declared v1 support is exactly {IPC_VERSION_V1}.
        // A future change that widens the range (to drop support
        // for an older daemon) must touch this test.
        let r = supported_versions();
        assert_eq!(r.min, IPC_VERSION_V1);
        assert_eq!(r.max, IPC_VERSION_V1);
    }

    /// End-to-end: stand up a real UDS server (the same
    /// `serve_connection` path the daemon uses), write a fake
    /// `control.json`, dial via `connect()`, send one `Invoke`,
    /// observe the daemon's v1 skeleton `Error` response.
    ///
    /// This test pins the entire FFI-side dial+round-trip path
    /// against the real server harness. A regression in either
    /// half (encode, codec settings, version overlap, decode)
    /// turns up here.
    #[cfg(unix)]
    #[tokio::test]
    async fn connect_and_round_trip_against_real_server() {
        let dir = unique_tmp();
        let sock_path = dir.join("c.sock");
        let json_path = dir.join("control.json");

        // Stand up a server on a known socket.
        let (listener, addr) = transport::bind_at(&sock_path).expect("bind");
        let proxy = AbilityProxy::new(make_kernel());
        let server_task = tokio::spawn(async move {
            #[cfg(unix)]
            if let ControlListener::Uds(uds) = listener {
                let (stream, _) = uds.accept().await.unwrap();
                // serve_connection is private to server.rs; route
                // through the public re-export.
                server::serve_one_for_test(stream, proxy).await.unwrap();
            }
        });

        // Write a control.json the client can read, advertising the
        // socket we just bound and the v1 single-point range.
        let disc = ControlDiscovery {
            socket_path: addr.as_uds_path().map(|p| p.to_path_buf()),
            pipe_name: None,
            pid: std::process::id(),
            daemon_version: "test".into(),
            supported_ipc_versions: IpcVersionRange::single(IPC_VERSION_V1),
            capability_flags: vec![flags::ABILITY_INVOKE.into()],
        };
        discovery::write(&json_path, &disc).expect("write control.json");

        // Dial via the FFI-side client.
        let mut client = connect(&json_path).await.expect("ffi connect");
        assert_eq!(client.ipc_version, IPC_VERSION_V1);

        let resp = client
            .round_trip(IncomingFrame::Invoke {
                request_id: "ffi-1".into(),
                ability: "observe.health".into(),
                args: serde_json::json!({}),
            })
            .await
            .expect("round_trip");

        // PR-INVOCATION-EXEC-UNITY: Invoke now reaches the real
        // dispatcher; observe.health returns a Result envelope. The
        // request_id round-trip is the load-bearing assertion (Client
        // bindings correlate by it); the value shape is owned by the
        // ping handler.
        match resp {
            OutgoingFrame::Result { request_id, .. } => {
                assert_eq!(request_id, "ffi-1");
            }
            other => panic!("expected Result frame for observe.health, got {other:?}"),
        }

        // Drop the client to close its side; server task exits via EOF.
        drop(client);
        let _ = server_task.await;
    }

    #[tokio::test]
    async fn connect_errors_when_control_json_missing() {
        // Pin the operator-visible message so a Client binding's
        // diagnostic path stays stable. "control.json not found" is
        // the trigger for the FFI layer to map to ERR_DAEMON_DOWN.
        let p = PathBuf::from("/tmp/eznt-nope-no-such-file.json");
        let _ = std::fs::remove_file(&p);
        let err = connect(&p).await.unwrap_err();
        let msg = format!("{err:#}");
        assert!(
            msg.contains("control.json not found"),
            "expected operator message about missing control.json, got: {msg}"
        );
    }

    #[cfg(unix)]
    #[tokio::test]
    async fn connect_errors_when_version_ranges_disjoint() {
        // Build a control.json that claims a daemon range above
        // anything the lib supports. The lib must refuse to dial
        // — this is the only protection against silently talking
        // a wire format the daemon would mis-decode.
        let dir = unique_tmp();
        let sock_path = dir.join("dummy.sock");
        let json_path = dir.join("control.json");

        let disc = ControlDiscovery {
            socket_path: Some(sock_path.clone()),
            pipe_name: None,
            pid: 0,
            daemon_version: "test".into(),
            // Daemon claims it speaks v99-v100; the lib supports v1.
            supported_ipc_versions: IpcVersionRange { min: 99, max: 100 },
            capability_flags: vec![],
        };
        discovery::write(&json_path, &disc).expect("write control.json");

        let err = connect(&json_path).await.unwrap_err();
        let msg = format!("{err:#}");
        assert!(
            msg.contains("version negotiation failed"),
            "expected version-mismatch error, got: {msg}"
        );
    }
}
