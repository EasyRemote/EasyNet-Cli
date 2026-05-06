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
// - Stay distinct from the RFC-003 gRPC transport socket
//   `~/.easynet/daemon.sock`; `control.sock` is legacy
//   length-delimited JSON IPC, `daemon.sock` is tonic `Invocation`.
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

use std::collections::HashMap;
use std::sync::{Arc, Mutex};

use bytes::Bytes;
use futures::{SinkExt, StreamExt};
use tokio::net::UnixStream;
use tokio::sync::mpsc;
use tokio_util::codec::{Framed, LengthDelimitedCodec};

use crate::runtime::kernel_api::KernelApi;
use crate::services::control::ability_proxy::{AbilityProxy, BidiRegistry, CancelRegistry};
use crate::services::control::discovery::{
    self, flags, ControlDiscovery, IpcVersionRange, IPC_VERSION_V1,
};
use crate::services::control::frames::{IncomingFrame, OutgoingFrame};
use crate::services::control::transport::{self, ControlListener};

/// Bind, advertise, and run the Control-plane accept loop until the
/// listener is dropped.
///
/// The daemon bin calls this from a `#[tokio::main]` runtime. Errors
/// during bind / discovery write are returned synchronously; errors
/// inside per-connection tasks are logged but do not tear down the
/// loop (one bad client must not kill the daemon).
///
/// `proxy` carries the dispatcher + resolver the daemon already
/// built off the Kernel's sub-service handles. Passing it in (rather
/// than constructing a fresh one here) preserves the U1 unity
/// property: every IPC dispatch and every direct KernelApi call
/// observe one set of sub-service state.
pub async fn run(proxy: AbilityProxy) -> anyhow::Result<()> {
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
    accept_loop(listener, proxy).await
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

/// Drive one accepted UnixStream. Splits the framed connection
/// into reader + writer halves so live subscriptions (multiple
/// frames over time) can interleave with new client requests
/// without serialising one against the other.
///
/// Topology:
///
///   reader task     → `IncomingFrame` decode
///                  → `proxy.handle_async(req, out_tx, cancel, bidi)`
///                       │
///                       ├── Invoke / Snapshot subscribe: pushes
///                       │    every frame onto `out_tx` synchronously
///                       │    (small bounded burst).
///                       └── Live / SnapshotThenLive subscribe:
///                            spawns a forwarder task that pushes
///                            frames over time as the broadcast
///                            yields. Forwarder owns its `out_tx`
///                            clone and observes a per-subscription
///                            cancel token.
///
///   writer task     ← `OutgoingFrame` from `out_rx`
///                  → length-prefixed JSON write to the connection
async fn serve_connection(stream: UnixStream, proxy: AbilityProxy) -> anyhow::Result<()> {
    let codec = LengthDelimitedCodec::builder().little_endian().new_codec();
    let framed = Framed::new(stream, codec);
    let (mut sink, mut source) = framed.split();

    // Per-connection writer queue. 256 frames bounded — large
    // enough to absorb a permission/discuss snapshot burst without
    // back-pressuring the forwarder, but bounded so a runaway
    // ability cannot consume unlimited memory.
    let (out_tx, mut out_rx) = mpsc::channel::<OutgoingFrame>(256);

    // Per-connection cancel registry; subscriptions register their
    // CancellationToken here, Cancel frames look up by id.
    let cancel: CancelRegistry = Arc::new(Mutex::new(HashMap::new()));

    // Per-connection bidi-session table; OpenBidi installs rows,
    // SendBidi looks them up to push frames into the handler input,
    // CloseBidi removes them. §D8 keeps this per-connection so a
    // dropped connection deterministically tears every live session
    // down via the cancel-token sweep below.
    let bidi: BidiRegistry = Arc::new(Mutex::new(HashMap::new()));

    // Writer task: drains the queue, serialises each frame to JSON,
    // pushes over the codec.
    let writer = tokio::spawn(async move {
        while let Some(frame) = out_rx.recv().await {
            let bytes = match serde_json::to_vec(&frame) {
                Ok(b) => b,
                Err(_) => continue, // unencodable frame, drop
            };
            if sink.send(Bytes::from(bytes)).await.is_err() {
                break; // connection broken
            }
        }
    });

    // Reader loop: decode each incoming frame, hand to proxy.
    while let Some(frame_res) = source.next().await {
        let bytes = match frame_res {
            Ok(b) => b,
            Err(_) => break,
        };
        let req: IncomingFrame = match serde_json::from_slice(&bytes) {
            Ok(r) => r,
            Err(e) => {
                let err_frame = OutgoingFrame::Error {
                    request_id: None,
                    subscription_id: None,
                    code: crate::services::control::frames::codes::PROTOCOL.into(),
                    message: format!("malformed IncomingFrame JSON: {e}"),
                };
                if out_tx.send(err_frame).await.is_err() {
                    break;
                }
                continue;
            }
        };
        proxy
            .handle_async(req, out_tx.clone(), &cancel, &bidi)
            .await;
    }

    // Reader stopped → connection closing. Cancel every live
    // subscription so forwarder tasks exit promptly, then drain
    // the bidi registry — dropping each `BidiSession` closes its
    // `to_handler` sender (handler observes EOF) and fires its
    // cancel token (the bidi forwarder breaks out of its select
    // and emits its single TerminalBidi per §I2).
    {
        let mut g = cancel.lock().expect("cancel registry lock");
        for (_, tok) in g.drain() {
            tok.cancel();
        }
    }
    {
        let mut g = bidi.lock().expect("bidi registry lock");
        for (_, sess) in g.drain() {
            sess.cancel.cancel();
            // Sender drops at end of scope when `sess` goes out of
            // scope, signalling EOF to the handler in parallel.
        }
    }
    drop(out_tx); // forwarders + writer task observe sender close
    let _ = writer.await;
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
    /// frame from a client UnixStream, observe the dispatcher's
    /// Result response. Validates: bind → accept → codec read →
    /// AbilityProxy.handle → dispatcher → codec write → close.
    #[cfg(unix)]
    #[tokio::test]
    async fn end_to_end_invoke_round_trip_returns_result_for_system_ping() {
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
            ability: "device.observe.health".into(),
            args: serde_json::json!({}),
            subject: None,
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

        // PR-INVOCATION-EXEC-UNITY: Invoke now dispatches through the
        // unified registry, so `observe.health` returns a Result envelope
        // (not the v1 skeleton Error). The exact value shape is owned
        // by the ping handler; here we only pin the request_id round-
        // trip + the envelope variant.
        let resp: OutgoingFrame = serde_json::from_slice(&resp_buf).unwrap();
        match resp {
            OutgoingFrame::Result { request_id, .. } => {
                assert_eq!(request_id, "smoke-1");
            }
            other => panic!("expected Result frame for observe.health, got {other:?}"),
        }

        // Close the client side; the server's read loop sees EOF
        // and serve_connection returns; the server task finishes.
        drop(client);
        let _ = server_task.await;
    }

    /// E2E bidi: bind UDS, register an in-process echo bidi handler,
    /// drive a full OpenBidi → 3× SendBidi → CloseBidi sequence over
    /// the wire, observe 3× RecvBidi + 1× TerminalBidi back. Pins:
    ///   * §I1 — frame ordering (the three RecvBidi arrive in the
    ///           same order the SendBidi went out)
    ///   * §I2 — exactly one TerminalBidi over the actual wire codec
    ///   * §D8 — server-side BidiRegistry plumbed end-to-end
    ///
    /// Uses the same wire-format-by-hand pattern as the existing
    /// observe.health smoke test so the codec is exercised byte for
    /// byte, not stubbed.
    #[cfg(unix)]
    #[tokio::test]
    async fn end_to_end_bidi_round_trip_echoes_three_frames_then_terminal() {
        use crate::runtime::ability_dispatch::{
            AbilityDispatcher, BidiSource, LocalAbilityRegistry, LocalBidiHandler,
            BIDI_CHANNEL_BOUND,
        };
        use crate::runtime::domain::NodeId;
        use crate::runtime::invocation_target::{LocalNodeResolver, TargetResolver};

        let dir = unique_tmp();
        let path = dir.join("bidi.sock");

        // Build a proxy whose registry has one bidi handler. Same
        // pattern as the proxy-level tests in ability_proxy.rs but
        // exercised here through the real serve_connection codec.
        let mut reg = LocalAbilityRegistry::new();
        let handler: LocalBidiHandler = Arc::new(|_args: serde_json::Value| {
            let (xport_to_handler_tx, mut handler_rx) =
                mpsc::channel::<serde_json::Value>(BIDI_CHANNEL_BOUND);
            let (handler_tx, xport_from_handler_rx) =
                mpsc::channel::<serde_json::Value>(BIDI_CHANNEL_BOUND);
            tokio::spawn(async move {
                while let Some(frame) = handler_rx.recv().await {
                    if handler_tx.send(frame).await.is_err() {
                        break;
                    }
                }
            });
            Ok(BidiSource {
                to_client: xport_to_handler_tx,
                from_client: xport_from_handler_rx,
            })
        });
        reg.register_bidi("bidi.echo", handler);
        let registry = Arc::new(reg);
        let gateway: Arc<dyn crate::runtime::gateway_api::GatewayApi> =
            Arc::new(NoopGateway::new());
        let dispatcher = AbilityDispatcher::new(registry, gateway);
        let resolver: Arc<dyn TargetResolver> =
            Arc::new(LocalNodeResolver::new(NodeId::new("self")));
        let proxy = AbilityProxy::new_with_dispatcher(make_kernel(), dispatcher, resolver);

        let (listener, _addr) = transport::bind_at(&path).expect("bind");
        let server_task = tokio::spawn(async move {
            #[cfg(unix)]
            if let ControlListener::Uds(uds) = listener {
                let (stream, _) = uds.accept().await.unwrap();
                serve_connection(stream, proxy).await.unwrap();
            }
        });

        let mut client = UnixStream::connect(&path).await.expect("connect");

        // Helper to send one length-prefixed frame.
        async fn send_frame(c: &mut UnixStream, f: &IncomingFrame) {
            let payload = serde_json::to_vec(f).unwrap();
            let len = u32::try_from(payload.len()).unwrap().to_le_bytes();
            c.write_all(&len).await.unwrap();
            c.write_all(&payload).await.unwrap();
            c.flush().await.unwrap();
        }

        // Helper to receive one length-prefixed frame.
        async fn recv_frame(c: &mut UnixStream) -> OutgoingFrame {
            let mut len_buf = [0u8; 4];
            c.read_exact(&mut len_buf).await.unwrap();
            let n = u32::from_le_bytes(len_buf) as usize;
            let mut buf = vec![0u8; n];
            c.read_exact(&mut buf).await.unwrap();
            serde_json::from_slice(&buf).unwrap()
        }

        // Open + send three frames + close.
        send_frame(
            &mut client,
            &IncomingFrame::OpenBidi {
                session_id: "e2e-1".into(),
                ability: "bidi.echo".into(),
                args: serde_json::json!({}),
            },
        )
        .await;
        for i in 0..3 {
            send_frame(
                &mut client,
                &IncomingFrame::SendBidi {
                    session_id: "e2e-1".into(),
                    frame: serde_json::json!({"i": i}),
                },
            )
            .await;
        }
        send_frame(
            &mut client,
            &IncomingFrame::CloseBidi {
                session_id: "e2e-1".into(),
            },
        )
        .await;

        // Expect 3× RecvBidi in order then exactly 1× TerminalBidi{done}.
        // Loop budget = 4 (the exact frame count); a regression that
        // emits an extra Terminal trips the post-loop tail-check below.
        let mut recv_count = 0;
        // 2 s deadline per frame so a dropped-frame regression fails
        // fast instead of hanging the runner.
        let read_deadline = std::time::Duration::from_secs(2);
        for _ in 0..4 {
            let frame = tokio::time::timeout(read_deadline, recv_frame(&mut client))
                .await
                .unwrap_or_else(|_| {
                    panic!(
                        "timeout reading bidi frame {} of 4 (got {recv_count} RecvBidi so far)",
                        recv_count + 1
                    )
                });
            match frame {
                OutgoingFrame::RecvBidi {
                    session_id,
                    frame: f,
                } => {
                    assert_eq!(session_id, "e2e-1");
                    assert_eq!(
                        f,
                        serde_json::json!({"i": recv_count}),
                        "§I1 violation at index {recv_count}: out-of-order frame"
                    );
                    recv_count += 1;
                }
                OutgoingFrame::TerminalBidi { session_id, reason } => {
                    assert_eq!(session_id, "e2e-1");
                    assert_eq!(reason, "done", "graceful close must report `done`");
                    assert_eq!(recv_count, 3, "Terminal arrived before all RecvBidi");
                    break;
                }
                other => panic!("unexpected frame on bidi e2e: {other:?}"),
            }
        }
        assert_eq!(recv_count, 3, "expected exactly 3 RecvBidi before Terminal");

        // §I2 tail-check: poll briefly for a SECOND Terminal that
        // shouldn't exist. A regression re-firing on the cancel-sweep
        // path during connection drop would surface here. Short
        // window (200 ms) is enough — the forwarder either fires
        // immediately on the racing close path or not at all.
        let stray = tokio::time::timeout(
            std::time::Duration::from_millis(200),
            recv_frame(&mut client),
        )
        .await;
        assert!(
            stray.is_err(),
            "§I2 violation: extra frame after TerminalBidi: {:?}",
            stray.ok()
        );

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
            ability: "device.observe.health".into(),
            args: serde_json::json!({}),
            subject: None,
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

        // PR-INVOCATION-EXEC-UNITY: the recovered second frame is a
        // valid `observe.health` Invoke, so the response is a Result
        // envelope. The point of the test is that the connection
        // survived the bad frame and is still serving real requests.
        let resp: OutgoingFrame = serde_json::from_slice(&resp_buf).unwrap();
        match resp {
            OutgoingFrame::Result { request_id, .. } => {
                assert_eq!(request_id, "after-bad");
            }
            other => panic!("expected Result frame after recovery, got {other:?}"),
        }

        drop(client);
        let _ = server_task.await;
    }
}
