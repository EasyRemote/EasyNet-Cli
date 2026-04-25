// EasyNet CLI — Control-plane Accept Loop
// ========================================
//
// File: src/services/control/server.rs
// Description: Ties transport + ability_proxy + discovery together.
//              `run()` is the daemon's main IPC entry: bind the
//              listener, write `~/.easynet/control.json`, accept
//              connections forever, hand each one to a per-
//              connection task that decodes IncomingFrame envelopes
//              and dispatches them to the AbilityProxy.
//
// v1 wiring landed in PR-DAEMON Commit 3
// --------------------------------------
// - Bind a UDS at `~/.easynet/control.sock` (mode 0600).
// - Write `~/.easynet/control.json` with the resolved address +
//   `supported_ipc_versions` range and capability flags.
// - Spawn one tokio task per accepted connection.
// - Each task wraps the connection in a `LengthDelimitedCodec`
//   framed reader/writer (4-byte LE length prefix, JSON payload).
// - Read loop: deserialise each frame to `IncomingFrame`; pass to
//   `AbilityProxy::handle`; serialise the resulting `OutgoingFrame`
//   back over the codec.
//
// What is NOT in this commit
// --------------------------
// - Handshake validation (the negotiator that intersects
//   `IpcVersionRange`s). Today every connection is treated as v1.
//   Follow-up commit lands the explicit handshake before any other
//   frame is accepted.
// - Real ability dispatch — the proxy still returns its v1 skeleton
//   `Error` envelope (see ability_proxy.rs). PR-INVOCATION-EXEC-
//   UNITY swaps that for the real Kernel::invoke path.
// - Cleanup of `control.json` on graceful shutdown. The current
//   implementation drops the daemon's exit cleanup back to the OS
//   removing the temp socket; a follow-up adds a SIGTERM handler.
//
// Author: Silan Hu <silan.hu@u.nus.edu>
// Copyright (c) 2026 EasyNet. All rights reserved.

use std::sync::Arc;

use bytes::Bytes;
use futures::{SinkExt, StreamExt};
use tokio::net::UnixStream;
use tokio_util::codec::{Framed, LengthDelimitedCodec};

use crate::runtime::kernel_api::KernelApi;
use crate::services::control::ability_proxy::AbilityProxy;
use crate::services::control::discovery::{
    self, flags, ControlDiscovery, IpcVersionRange, IPC_VERSION_V1,
};
use crate::services::control::frames::{IncomingFrame, OutgoingFrame};
use crate::services::control::transport::{self, ControlAddress, ControlListener};

/// Bind, advertise, and run the Control-plane accept loop until the
/// listener is dropped.
///
/// The daemon bin calls this from a `#[tokio::main]` runtime. Errors
/// during bind / discovery write are returned synchronously; errors
/// inside per-connection tasks are logged but do not tear down the
/// loop (one bad client must not kill the daemon).
pub async fn run(kernel: Arc<dyn KernelApi>) -> anyhow::Result<()> {
    let (listener, addr) = transport::bind_default()?;

    // Advertise via control.json.
    let pid = std::process::id();
    let disc = ControlDiscovery {
        socket_path: addr.as_uds_path().map(|p| p.to_path_buf()),
        pipe_name: addr.as_pipe_name().map(|s| s.to_string()),
        pid,
        daemon_version: env!("CARGO_PKG_VERSION").to_string(),
        supported_ipc_versions: IpcVersionRange::single(IPC_VERSION_V1),
        capability_flags: vec![
            flags::ABILITY_INVOKE.into(),
            flags::ABILITY_SUBSCRIBE.into(),
            flags::LOOPBACK.into(),
            flags::MISFIRE_POLICY_V1.into(),
        ],
    };
    discovery::write(&discovery::default_path(), &disc)?;

    // Accept loop.
    accept_loop(listener, AbilityProxy::new(kernel)).await
}

/// Accept connections forever, spawn one tokio task per connection.
/// Extracted as a free function so tests can drive it with an
/// arbitrary listener instead of going through `bind_default`.
pub async fn accept_loop(listener: ControlListener, proxy: AbilityProxy) -> anyhow::Result<()> {
    match listener {
        #[cfg(unix)]
        ControlListener::Uds(uds) => {
            loop {
                let (stream, _peer_addr) = uds.accept().await?;
                let proxy = proxy.clone();
                tokio::spawn(async move {
                    if let Err(e) = serve_connection(stream, proxy).await {
                        // Per-connection failure is operator-visible
                        // but never fatal to the listener loop. We
                        // log via stderr; a future PR can route this
                        // through tracing.
                        eprintln!("[control] connection ended with error: {e:#}");
                    }
                });
            }
        }
        ControlListener::Unsupported => anyhow::bail!(
            "control-plane listener is the Unsupported variant; \
             v10.5 R1 does not yet wire this platform"
        ),
    }
}

/// Drive one accepted UnixStream: framed read → proxy → framed write
/// in a loop until the peer closes.
async fn serve_connection(stream: UnixStream, proxy: AbilityProxy) -> anyhow::Result<()> {
    // Codec: 4-byte LE length prefix; payload is JSON bytes per
    // services/control/frames.rs. Default settings are correct for
    // v1 (LE, max frame 8 MiB which is more than sufficient for
    // a JSON envelope; v2 will tighten if needed).
    let codec = LengthDelimitedCodec::builder()
        .little_endian()
        .new_codec();
    let mut framed = Framed::new(stream, codec);

    while let Some(frame) = framed.next().await {
        let bytes = frame?; // BytesMut
        let req: IncomingFrame = match serde_json::from_slice(&bytes) {
            Ok(r) => r,
            Err(e) => {
                // Protocol violation: respond with an Error frame
                // and keep the connection open. The client may have
                // sent one bad frame and recovered.
                let err_frame = OutgoingFrame::Error {
                    request_id: None,
                    subscription_id: None,
                    code: crate::services::control::frames::codes::PROTOCOL.into(),
                    message: format!("malformed IncomingFrame JSON: {e}"),
                };
                write_frame(&mut framed, &err_frame).await?;
                continue;
            }
        };
        let resp = proxy.handle(req);
        write_frame(&mut framed, &resp).await?;
    }
    Ok(())
}

/// JSON-serialise an OutgoingFrame and push it over the codec.
async fn write_frame(
    framed: &mut Framed<UnixStream, LengthDelimitedCodec>,
    frame: &OutgoingFrame,
) -> anyhow::Result<()> {
    let bytes = serde_json::to_vec(frame)?;
    framed.send(Bytes::from(bytes)).await?;
    Ok(())
}

/// Construct an AbilityProxy wrapping the given Kernel. Exists as a
/// named helper so a future test harness can exercise the proxy
/// without going through the full accept loop.
pub fn make_proxy(kernel: Arc<dyn KernelApi>) -> AbilityProxy {
    AbilityProxy::new(kernel)
}

/// Test-only escape hatch for sibling FFI client tests.
///
/// `serve_connection` is private on purpose — production code only
/// reaches it via `accept_loop`. The FFI client tests in
/// `crate::ffi::client` need to drive exactly one connection so the
/// test can dial into the real server harness; route them through
/// this wrapper instead of pub-ing the private function.
#[cfg(test)]
pub(crate) async fn serve_one_for_test(
    stream: UnixStream,
    proxy: AbilityProxy,
) -> anyhow::Result<()> {
    serve_connection(stream, proxy).await
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::runtime::gateway::NoopGateway;
    use crate::runtime::kernel::Kernel;
    use std::path::PathBuf;
    use tokio::io::{AsyncReadExt, AsyncWriteExt};
    use tokio::net::UnixStream;

    /// Build a short, unique temp directory under `/tmp/`.
    ///
    /// Why not `std::env::temp_dir()`? On macOS that returns
    /// `/var/folders/<hash>/T/` which already eats ~50 bytes; once
    /// you append a sub-dir and a `.sock` filename you blow past
    /// `SUN_LEN` (~104 bytes for `sockaddr_un.sun_path`) and bind
    /// fails with EINVAL. `/tmp` keeps the prefix at 5 bytes so the
    /// full UDS path fits comfortably.
    fn unique_tmp() -> PathBuf {
        let now = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .map(|d| d.as_nanos())
            .unwrap_or(0);
        let p = PathBuf::from(format!("/tmp/eznt-srv-{}-{}", std::process::id(), now));
        std::fs::create_dir_all(&p).unwrap();
        p
    }

    fn make_kernel() -> Arc<Kernel> {
        Arc::new(Kernel::new(Arc::new(NoopGateway::new())))
    }

    #[test]
    fn make_proxy_wraps_kernel_handle() {
        let kernel: Arc<dyn KernelApi> = make_kernel();
        let proxy = make_proxy(Arc::clone(&kernel));
        assert!(Arc::ptr_eq(proxy.kernel(), &kernel));
    }

    /// End-to-end smoke: bind UDS at a temp path, send one Invoke
    /// frame from a client UnixStream, observe the v1 skeleton
    /// Error response. Validates: bind → accept → codec read →
    /// AbilityProxy.handle → codec write → close.
    ///
    /// Pinned to the v1 skeleton response (`ABILITY_FAILED`) on
    /// purpose: when PR-INVOCATION-EXEC-UNITY swaps the proxy body
    /// for real dispatch, this test will fail with a clear message
    /// pointing at the stale v1 expectation.
    #[cfg(unix)]
    #[tokio::test]
    async fn end_to_end_invoke_round_trip_returns_v1_skeleton_error() {
        let dir = unique_tmp();
        let path = dir.join("smoke.sock");

        let (listener, _addr) = transport::bind_at(&path).expect("bind");
        let proxy = AbilityProxy::new(make_kernel());
        let server_task = tokio::spawn(async move {
            // Accept exactly one connection then return; this avoids
            // leaking a forever-loop in the test runtime.
            #[cfg(unix)]
            if let ControlListener::Uds(uds) = listener {
                let (stream, _) = uds.accept().await.unwrap();
                serve_connection(stream, proxy).await.unwrap();
            }
        });

        // Client side: connect and send one Invoke frame manually
        // formatted with the 4-byte LE length prefix the codec
        // expects. (Using the codec on the client side too would be
        // more idiomatic, but doing it by hand here pins the wire
        // format byte-for-byte.)
        let mut client = UnixStream::connect(&path).await.expect("connect");
        let req = IncomingFrame::Invoke {
            request_id: "smoke-1".into(),
            ability: "system.ping".into(),
            args: serde_json::json!({}),
        };
        let payload = serde_json::to_vec(&req).unwrap();
        let len = u32::try_from(payload.len()).unwrap().to_le_bytes();
        client.write_all(&len).await.unwrap();
        client.write_all(&payload).await.unwrap();
        client.flush().await.unwrap();

        // Read the response frame (4-byte length + JSON body).
        let mut len_buf = [0u8; 4];
        client.read_exact(&mut len_buf).await.unwrap();
        let resp_len = u32::from_le_bytes(len_buf) as usize;
        let mut resp_buf = vec![0u8; resp_len];
        client.read_exact(&mut resp_buf).await.unwrap();

        let resp: OutgoingFrame = serde_json::from_slice(&resp_buf).unwrap();
        match resp {
            OutgoingFrame::Error {
                request_id,
                code,
                ..
            } => {
                assert_eq!(request_id.as_deref(), Some("smoke-1"));
                assert_eq!(code, crate::services::control::frames::codes::ABILITY_FAILED);
            }
            other => panic!("expected v1 skeleton Error frame, got {other:?}"),
        }

        // Close the client side; the server's read loop sees EOF
        // and serve_connection returns; the server task finishes.
        drop(client);
        let _ = server_task.await;
    }

    /// Malformed JSON in a frame must yield a `PROTOCOL` Error
    /// response without dropping the connection. Pin both halves.
    #[cfg(unix)]
    #[tokio::test]
    async fn malformed_frame_yields_protocol_error_and_keeps_connection_open() {
        let dir = unique_tmp();
        let path = dir.join("bad.sock");

        let (listener, _addr) = transport::bind_at(&path).expect("bind");
        let proxy = AbilityProxy::new(make_kernel());
        let server_task = tokio::spawn(async move {
            #[cfg(unix)]
            if let ControlListener::Uds(uds) = listener {
                let (stream, _) = uds.accept().await.unwrap();
                serve_connection(stream, proxy).await.unwrap();
            }
        });

        let mut client = UnixStream::connect(&path).await.expect("connect");

        // Send a bogus JSON payload.
        let bogus = b"{\"type\":\"invoke\",\"request_id\":\"x\",\"args\":not_json}";
        let len = u32::try_from(bogus.len()).unwrap().to_le_bytes();
        client.write_all(&len).await.unwrap();
        client.write_all(bogus).await.unwrap();
        client.flush().await.unwrap();

        let mut len_buf = [0u8; 4];
        client.read_exact(&mut len_buf).await.unwrap();
        let resp_len = u32::from_le_bytes(len_buf) as usize;
        let mut resp_buf = vec![0u8; resp_len];
        client.read_exact(&mut resp_buf).await.unwrap();

        let resp: OutgoingFrame = serde_json::from_slice(&resp_buf).unwrap();
        match resp {
            OutgoingFrame::Error { code, .. } => {
                assert_eq!(code, crate::services::control::frames::codes::PROTOCOL);
            }
            other => panic!("expected PROTOCOL error, got {other:?}"),
        }

        // Now send a valid frame to confirm the connection survived.
        let req = IncomingFrame::Invoke {
            request_id: "after-bad".into(),
            ability: "system.ping".into(),
            args: serde_json::json!({}),
        };
        let payload = serde_json::to_vec(&req).unwrap();
        let len = u32::try_from(payload.len()).unwrap().to_le_bytes();
        client.write_all(&len).await.unwrap();
        client.write_all(&payload).await.unwrap();
        client.flush().await.unwrap();

        let mut len_buf = [0u8; 4];
        client.read_exact(&mut len_buf).await.unwrap();
        let resp_len = u32::from_le_bytes(len_buf) as usize;
        let mut resp_buf = vec![0u8; resp_len];
        client.read_exact(&mut resp_buf).await.unwrap();

        let resp: OutgoingFrame = serde_json::from_slice(&resp_buf).unwrap();
        // Skeleton error, not a protocol error: confirms the
        // connection delivered the second frame to the proxy.
        match resp {
            OutgoingFrame::Error { code, request_id, .. } => {
                assert_eq!(code, crate::services::control::frames::codes::ABILITY_FAILED);
                assert_eq!(request_id.as_deref(), Some("after-bad"));
            }
            other => panic!("expected skeleton error after recovery, got {other:?}"),
        }

        drop(client);
        let _ = server_task.await;
    }
}
