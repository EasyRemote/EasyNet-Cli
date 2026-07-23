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
// Why this module, distinct from daemon::control::server
// --------------------------------------------------------
// `server.rs` is the daemon side; `client.rs` is the lib side. Both
// speak `frames::IncomingFrame` / `frames::OutgoingFrame` over a
// `LengthDelimitedCodec`-framed UDS, but the code shape is inverted:
// server has an accept loop; client has a single connect + the
// per-call read/write split.
//
// Current status — control-plane IPC only
// ---------------------------------------
// `connect()` reads `~/.easynet/control.json`, validates the
// `supported_ipc_versions` overlap with the lib's range, and opens
// a `tokio::net::UnixStream` on the discovered socket. The control
// socket intentionally has no product Invocation traffic; daemon
// Invocation calls use `daemon.sock` through `crate::daemon`.
//
// `round_trip()` writes one length-prefixed JSON frame and reads
// exactly one back. Product ability calls no longer use this client;
// they go through the daemon Invocation transport. This JSON client
// remains for control-plane traffic such as boot/status probes and
// for tests that pin the control socket codec.
//
// Author: Silan Hu <silan.hu@u.nus.edu>
// Copyright (c) 2026 EasyNet. All rights reserved.

use std::path::Path;

use bytes::Bytes;
use futures::{SinkExt, StreamExt};
use tokio_util::codec::{Framed, LengthDelimitedCodec};

#[cfg(windows)]
use tokio::net::windows::named_pipe::NamedPipeClient;
#[cfg(unix)]
use tokio::net::UnixStream;

use crate::daemon::control::discovery::{self, ControlDiscovery, IpcVersionRange, IPC_VERSION_V1};
use crate::daemon::control::frames::{IncomingFrame, OutgoingFrame};

#[derive(Debug, thiserror::Error)]
pub enum IpcConnectError {
    #[error("{0}")]
    DaemonUnavailable(#[from] anyhow::Error),
    #[error(
        "FFI client: IPC version negotiation failed. lib supports {lib_range:?}, daemon supports {daemon_range:?}"
    )]
    VersionIncompatible {
        lib_range: IpcVersionRange,
        daemon_range: IpcVersionRange,
    },
}

/// Open IPC connection to the daemon, owned by a `ClientSession`.
///
/// We hold the framed stream behind the type alias for readability;
/// every read and write goes through the same codec the server uses
/// (4-byte little-endian length prefix + JSON payload).
#[cfg(unix)]
type FramedUds = Framed<UnixStream, LengthDelimitedCodec>;
#[cfg(windows)]
type FramedPipe = Framed<NamedPipeClient, LengthDelimitedCodec>;

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
    #[cfg(unix)]
    framed: FramedUds,
    #[cfg(windows)]
    framed: FramedPipe,
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

/// Supported control IPC version range on the lib side. Must overlap
/// with the daemon's advertised range for `connect()` to succeed.
pub fn supported_versions() -> IpcVersionRange {
    IpcVersionRange::single(IPC_VERSION_V1)
}

/// Read `control.json` at `control_json_path`, dial the discovered
/// UDS, validate the IPC version overlap, and return an open
/// `IpcClient`.
///
/// Errors:
/// - control.json missing or unreadable (`DaemonUnavailable`).
/// - control.json reports no UDS path for a Unix build (`DaemonUnavailable`).
/// - version ranges do not overlap (`VersionIncompatible`).
/// - connect refused (`DaemonUnavailable`).
pub async fn connect(control_json_path: &Path) -> Result<IpcClient, IpcConnectError> {
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
    let chosen = lib_range.overlap(disc.supported_ipc_versions).ok_or(
        IpcConnectError::VersionIncompatible {
            lib_range,
            daemon_range: disc.supported_ipc_versions,
        },
    )?;
    // Pick the highest version both sides support — `IpcVersionRange::overlap`
    // returns the intersection range; `max` is the agreed protocol version.
    let chosen_version = chosen.max;

    #[cfg(unix)]
    let socket_path = disc.socket_path.clone().ok_or_else(|| {
        anyhow::anyhow!(
            "FFI client: control.json reports no UDS socket_path; \
             this Unix build requires Unix Domain Socket control transport"
        )
    })?;

    #[cfg(windows)]
    let pipe_name = disc.pipe_name.clone().ok_or_else(|| {
        anyhow::anyhow!(
            "FFI client: control.json reports no pipe_name; \
             this Windows build requires named-pipe control transport"
        )
    })?;

    #[cfg(unix)]
    let stream = UnixStream::connect(&socket_path).await.map_err(|e| {
        // Wrap the io::Error as a *cause* (anyhow::Error::new) and
        // attach the human-readable message via `.context(...)` so
        // upstream code can downcast through the chain to the
        // original io::Error and pattern-match on its kind. Earlier
        // we built this with `anyhow::anyhow!("…: {e}")` which
        // formats the io::Error into a string and drops its kind —
        // breaking the friendlify_connect_error pre-check in
        // `support/local_invoke.rs`.
        anyhow::Error::new(e).context(format!(
            "FFI client: connect to {} failed",
            socket_path.display()
        ))
    })?;

    // Same codec the daemon uses. Default 8 MiB max frame size is
    // ample for v1 JSON envelopes; v2 may tighten when proto binary
    // payloads come in.
    #[cfg(unix)]
    let codec = LengthDelimitedCodec::builder().little_endian().new_codec();
    #[cfg(unix)]
    let framed = Framed::new(stream, codec);
    #[cfg(windows)]
    let stream = crate::support::platform::named_pipe::connect_with_retry(
        &pipe_name,
        std::time::Duration::from_secs(5),
    )
    .await
    .map_err(|e| anyhow::anyhow!("FFI client: connect to named pipe {pipe_name}: {e}"))?;
    #[cfg(windows)]
    let codec = LengthDelimitedCodec::builder().little_endian().new_codec();
    #[cfg(windows)]
    let framed = Framed::new(stream, codec);

    Ok(IpcClient {
        ipc_version: chosen_version,
        daemon_discovery: disc,
        #[cfg(unix)]
        framed,
        #[cfg(windows)]
        framed,
    })
}

impl IpcClient {
    /// Send one `IncomingFrame` and read exactly one
    /// `OutgoingFrame`. Retained for boot/status control probes;
    /// product ability calls use daemon Invocation over daemon.sock.
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
    use crate::daemon::control::boot_events::BootBus;
    use crate::daemon::control::discovery::{flags, ControlDiscovery, IpcVersionRange};
    use crate::daemon::control::server;
    use crate::daemon::control::transport::{self, ControlListener};
    use std::path::PathBuf;

    /// Short-prefix temp dir; see same comment in daemon/control/{server,transport}.rs.
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
    /// `control.json`, dial via `connect()`, send one boot/status
    /// `Subscribe`, and observe the daemon's boot event frame.
    ///
    /// This test pins the entire FFI-side dial+round-trip path
    /// against the real server harness. A regression in either
    /// half (encode, codec settings, version overlap, decode)
    /// turns up here. It intentionally uses `system.watch_boot`
    /// instead of product `Invoke`; product ability calls belong to
    /// the complete Invocation ABI.
    #[cfg(unix)]
    #[tokio::test]
    async fn connect_and_round_trip_boot_status_against_real_server() {
        let dir = unique_tmp();
        let sock_path = dir.join("c.sock");
        let json_path = dir.join("control.json");

        // Stand up a server on a known socket.
        let (listener, addr) = transport::bind_at(&sock_path).expect("bind");
        let boot = BootBus::new();
        boot.emit_ready();
        let server_task = tokio::spawn(async move {
            #[cfg(unix)]
            if let ControlListener::Uds(uds) = listener {
                let (stream, _) = uds.accept().await.unwrap();
                // serve_connection is private to server.rs; route
                // through the public re-export.
                server::serve_booting_one_for_test(stream, boot)
                    .await
                    .unwrap();
            }
        });

        // Write a control.json the client can read, advertising the
        // socket we just bound and the v1 single-point range.
        let disc = ControlDiscovery {
            socket_path: addr.as_uds_path().map(|p| p.to_path_buf()),
            pipe_name: None,
            invocation_endpoint: Some(dir.join("daemon.sock")),
            daemon_identity: None,
            pid: std::process::id(),
            daemon_version: "test".into(),
            supported_ipc_versions: IpcVersionRange::single(IPC_VERSION_V1),
            capability_flags: vec![flags::BOOT_STATUS.into()],
            pages_port: None,
        };
        discovery::write(&json_path, &disc).expect("write control.json");

        // Dial via the FFI-side client.
        let mut client = connect(&json_path).await.expect("ffi connect");
        assert_eq!(client.ipc_version, IPC_VERSION_V1);

        let frame = client
            .round_trip(IncomingFrame::Subscribe {
                subscription_id: "boot-sub".into(),
                ability: server::WATCH_BOOT_ABILITY.into(),
                args: serde_json::json!({}),
            })
            .await
            .expect("round_trip");

        match frame {
            OutgoingFrame::Frame {
                subscription_id,
                frame,
            } => {
                assert_eq!(subscription_id, "boot-sub");
                assert_eq!(frame["type"], "ready");
            }
            other => panic!("expected Ready boot Frame, got {other:?}"),
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
            invocation_endpoint: Some(dir.join("daemon.sock")),
            daemon_identity: None,
            pid: 0,
            daemon_version: "test".into(),
            // Daemon claims it speaks v99-v100; the lib supports v1.
            supported_ipc_versions: IpcVersionRange { min: 99, max: 100 },
            capability_flags: vec![],
            pages_port: None,
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
