// EasyNet CLI — Control-plane Accept Loop
// ========================================
//
// File: src/daemon/control/server.rs
// Description: Ties transport, discovery, and daemon boot/status
//              events together. `control.sock` is deliberately not a
//              product ability surface; product calls use
//              `daemon.sock` and the Axon Invocation service.
//
// v1 wiring landed in PR-DAEMON Commit 3
// --------------------------------------
// - Bind a UDS at `~/.easynet/control.sock` (mode 0600).
// - Write `~/.easynet/control.json` with the resolved address +
//   `supported_ipc_versions` range and capability flags.
// - Spawn one tokio task per accepted connection.
// - Each task wraps the connection in a `LengthDelimitedCodec`
//   framed reader/writer (4-byte LE length prefix, JSON payload).
// - Read loop: deserialise each frame to `IncomingFrame`; accept
//   only boot/status subscriptions; serialise `OutgoingFrame`
//   events back over the codec.
// - Stay distinct from the RFC-003 gRPC transport socket
//   `~/.easynet/daemon.sock`; `control.sock` is legacy
//   length-delimited JSON IPC, `daemon.sock` is tonic `Invocation`.
//
// What is NOT in this commit
// --------------------------
// - Wire handshake validation. The FFI client already performs
//   discovery-level version overlap against `control.json` before it
//   dials this socket; this server still treats every accepted
//   connection as v1 because there is no first-frame handshake yet.
// - Product ability dispatch. That path is the daemon Invocation
//   transport, not this legacy length-delimited JSON socket.
//
// Author: Silan Hu <silan.hu@u.nus.edu>
// Copyright (c) 2026 EasyNet. All rights reserved.

use std::collections::HashMap;
use std::sync::{Arc, Mutex};

use bytes::Bytes;
use futures::{SinkExt, StreamExt};
use tokio::io::{AsyncRead, AsyncWrite};
use tokio::sync::mpsc;
use tokio_util::codec::{Framed, LengthDelimitedCodec};

#[cfg(all(windows, test))]
use tokio::net::windows::named_pipe::NamedPipeServer;
#[cfg(all(unix, test))]
use tokio::net::UnixStream;

use crate::daemon::control::boot_events::{BootBus, BootEvent};
use crate::daemon::control::discovery::{
    self, flags, ControlDiscovery, DaemonIdentity, IpcVersionRange, IPC_VERSION_V1,
};
use crate::daemon::control::frames::{IncomingFrame, OutgoingFrame};
use crate::daemon::control::transport::{self, ControlAddress, ControlListener};

/// Ability name reserved for daemon boot progress.
pub const WATCH_BOOT_ABILITY: &str = crate::daemon::ability::names::governance::SYSTEM_WATCH_BOOT;

type CancelRegistry = Arc<Mutex<HashMap<String, tokio_util::sync::CancellationToken>>>;

/// Runtime state shared by every accepted control connection.
#[derive(Clone)]
pub struct ControlServerState {
    boot: BootBus,
}

impl ControlServerState {
    /// Create a state handle over the boot event bus.
    pub fn new(boot: BootBus) -> Self {
        Self { boot }
    }
}

/// Handle returned when the daemon starts the control server before
/// the dispatcher is ready.
#[derive(Clone)]
pub struct ControlServerHandle {
    address: ControlAddress,
}

impl ControlServerHandle {
    /// Re-write control.json with the current address and optional
    /// pages port.
    pub fn write_discovery(&self, pages_port: Option<u16>) -> anyhow::Result<()> {
        write_discovery_for(&self.address, pages_port, None)
    }

    /// Re-write control.json with the daemon's ready-state runtime
    /// endpoints and identity.
    pub fn write_ready_discovery(
        &self,
        pages_port: Option<u16>,
        runtime: ControlRuntimeDiscovery,
    ) -> anyhow::Result<()> {
        write_discovery_for(&self.address, pages_port, Some(runtime))
    }
}

/// Runtime metadata advertised once daemon Invocation has bound.
#[derive(Debug, Clone)]
pub struct ControlRuntimeDiscovery {
    /// Actual local Invocation endpoint for product calls.
    pub invocation_endpoint: std::path::PathBuf,
    /// Mode/realm/node tuple this daemon process owns.
    pub daemon_identity: DaemonIdentity,
    /// Runtime readiness capabilities proven before Ready was published.
    pub capability_flags: Vec<String>,
}

/// Bind, advertise, and run the Control-plane accept loop until the
/// listener is dropped.
///
/// The daemon bin calls this from a `#[tokio::main]` runtime. Errors
/// during bind / discovery write are returned synchronously; errors
/// inside per-connection tasks are logged but do not tear down the
/// loop (one bad client must not kill the daemon).
///
pub async fn run() -> anyhow::Result<()> {
    let (listener, addr) = transport::bind_default()?;
    write_discovery_for(&addr, None, None)?;

    let boot = BootBus::new();
    boot.emit_ready();
    let state = ControlServerState::new(boot);
    accept_loop_with_state(listener, state).await
}

/// Bind and spawn the control server in booting mode.
///
/// The returned handle can rewrite discovery once optional ports are
/// known.
pub fn spawn_booting(boot: BootBus) -> anyhow::Result<ControlServerHandle> {
    let (listener, address) = transport::bind_default()?;
    write_discovery_for(&address, None, None)?;
    let state = ControlServerState::new(boot);
    let accept_state = state.clone();
    tokio::spawn(async move {
        if let Err(e) = accept_loop_with_state(listener, accept_state).await {
            eprintln!("[control] accept loop exited: {e:#}");
        }
    });
    Ok(ControlServerHandle { address })
}

/// Accept connections forever, spawn one tokio task per connection.
/// Extracted as a free function so tests can drive it with an
/// arbitrary listener instead of going through `bind_default`.
pub(crate) async fn accept_loop_with_state(
    listener: ControlListener,
    state: ControlServerState,
) -> anyhow::Result<()> {
    match listener {
        #[cfg(unix)]
        ControlListener::Uds(uds) => {
            loop {
                let (stream, _peer_addr) = uds.accept().await?;
                let state = state.clone();
                tokio::spawn(async move {
                    if let Err(e) = serve_connection(stream, state).await {
                        // Per-connection failure is operator-visible
                        // but never fatal to the listener loop. We
                        // log via stderr; a future PR can route this
                        // through tracing.
                        let err_msg = format!("{e:#}");
                        crate::op_event!(
                            component = control,
                            kind = connection_ended_with_error,
                            error = err_msg,
                        );
                    }
                });
            }
        }
        #[cfg(windows)]
        ControlListener::NamedPipe(mut listener) => loop {
            let stream = listener.accept().await?;
            let state = state.clone();
            tokio::spawn(async move {
                if let Err(e) = serve_connection(stream, state).await {
                    let err_msg = format!("{e:#}");
                    crate::op_event!(
                        component = control,
                        kind = connection_ended_with_error,
                        error = err_msg,
                    );
                }
            });
        },
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
///                  → `system.watch_boot` subscription handling
///
///   writer task     ← `OutgoingFrame` from `out_rx`
///                  → length-prefixed JSON write to the connection
async fn serve_connection<S>(stream: S, state: ControlServerState) -> anyhow::Result<()>
where
    S: AsyncRead + AsyncWrite + Unpin + Send + 'static,
{
    let codec = LengthDelimitedCodec::builder().little_endian().new_codec();
    let framed = Framed::new(stream, codec);
    let (mut sink, mut source) = framed.split();

    // Per-connection writer queue. 256 frames is enough for a boot
    // event burst while still bounding memory for a stalled client.
    let (out_tx, mut out_rx) = mpsc::channel::<OutgoingFrame>(256);

    // Per-connection cancel registry; subscriptions register their
    // CancellationToken here, Cancel frames look up by id.
    let cancel: CancelRegistry = Arc::new(Mutex::new(HashMap::new()));

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

    // Reader loop: decode each incoming frame, handle control
    // operations only.
    while let Some(frame_res) = source.next().await {
        let bytes = match frame_res {
            Ok(b) => b,
            Err(_) => break,
        };
        let req: IncomingFrame = match serde_json::from_slice(&bytes) {
            Ok(r) => r,
            Err(e) => {
                let err_frame = OutgoingFrame::Error {
                    subscription_id: None,
                    code: crate::daemon::control::frames::codes::PROTOCOL.into(),
                    message: format!("malformed IncomingFrame JSON: {e}"),
                };
                if out_tx.send(err_frame).await.is_err() {
                    break;
                }
                continue;
            }
        };
        handle_request(req, out_tx.clone(), &cancel, &state).await;
    }

    // Reader stopped → connection closing. Cancel every live
    // boot subscription so forwarder tasks exit promptly.
    {
        let mut g = cancel.lock().expect("cancel registry lock");
        for (_, tok) in g.drain() {
            tok.cancel();
        }
    }
    drop(out_tx); // forwarders + writer task observe sender close
    let _ = writer.await;
    Ok(())
}

async fn handle_request(
    req: IncomingFrame,
    out: mpsc::Sender<OutgoingFrame>,
    cancel: &CancelRegistry,
    state: &ControlServerState,
) {
    match req {
        IncomingFrame::Subscribe {
            subscription_id,
            ability,
            ..
        } if ability == WATCH_BOOT_ABILITY => {
            spawn_boot_forwarder(subscription_id, state.boot.clone(), out, cancel.clone());
        }
        IncomingFrame::Cancel { subscription_id } => {
            let token = {
                let mut guard = cancel.lock().expect("cancel registry lock");
                guard.remove(&subscription_id)
            };
            if let Some(token) = token {
                token.cancel();
            } else {
                let _ = out
                    .send(OutgoingFrame::Terminal {
                        subscription_id,
                        reason: "not_found".into(),
                    })
                    .await;
            }
        }
        IncomingFrame::Subscribe {
            subscription_id, ..
        } => {
            let _ = out
                .send(OutgoingFrame::Error {
                    subscription_id: Some(subscription_id),
                    code: crate::daemon::control::frames::codes::NOT_FOUND.into(),
                    message: "unknown control subscription; expected system.watch_boot".into(),
                })
                .await;
        }
    }
}

fn spawn_boot_forwarder(
    subscription_id: String,
    boot: BootBus,
    out: mpsc::Sender<OutgoingFrame>,
    cancel: CancelRegistry,
) {
    let token = tokio_util::sync::CancellationToken::new();
    {
        let mut guard = cancel.lock().expect("cancel registry lock");
        guard.insert(subscription_id.clone(), token.clone());
    }
    tokio::spawn(async move {
        let mut rx = boot.subscribe();
        let reason = loop {
            tokio::select! {
                _ = token.cancelled() => break "cancelled".to_string(),
                event = rx.recv() => match event {
                    Ok(event) => {
                        let terminal = matches!(event, BootEvent::Ready | BootEvent::Failed { .. });
                        let frame = serde_json::to_value(&event).unwrap_or_else(|e| {
                            serde_json::json!({
                                "type": "failed",
                                "stage": "boot-event-serialization",
                                "error": e.to_string(),
                            })
                        });
                        if out
                            .send(OutgoingFrame::Frame {
                                subscription_id: subscription_id.clone(),
                                frame,
                            })
                            .await
                            .is_err()
                        {
                            break "connection_closed".to_string();
                        }
                        if terminal {
                            break "done".to_string();
                        }
                    }
                    Err(tokio::sync::broadcast::error::RecvError::Lagged(n)) => {
                        let frame = serde_json::json!({
                            "type": "lagged",
                            "dropped": n,
                        });
                        if out
                            .send(OutgoingFrame::Frame {
                                subscription_id: subscription_id.clone(),
                                frame,
                            })
                            .await
                            .is_err()
                        {
                            break "connection_closed".to_string();
                        }
                    }
                    Err(tokio::sync::broadcast::error::RecvError::Closed) => {
                        break "boot_bus_closed".to_string();
                    }
                }
            }
        };
        {
            let mut guard = cancel.lock().expect("cancel registry lock");
            guard.remove(&subscription_id);
        }
        let _ = out
            .send(OutgoingFrame::Terminal {
                subscription_id,
                reason,
            })
            .await;
    });
}

fn write_discovery_for(
    addr: &ControlAddress,
    pages_port: Option<u16>,
    runtime: Option<ControlRuntimeDiscovery>,
) -> anyhow::Result<()> {
    let pid = std::process::id();
    let (invocation_endpoint, daemon_identity, runtime_flags) = match runtime {
        Some(runtime) => (
            Some(runtime.invocation_endpoint),
            Some(runtime.daemon_identity),
            runtime.capability_flags,
        ),
        None => (None, None, Vec::new()),
    };
    let capability_flags = discovery_capability_flags(runtime_flags);
    let disc = ControlDiscovery {
        socket_path: addr.as_uds_path().map(|p| p.to_path_buf()),
        pipe_name: addr.as_pipe_name().map(|s| s.to_string()),
        invocation_endpoint,
        daemon_identity,
        pid,
        daemon_version: env!("CARGO_PKG_VERSION").to_string(),
        supported_ipc_versions: IpcVersionRange::single(IPC_VERSION_V1),
        capability_flags,
        pages_port,
    };
    discovery::write(&discovery::default_path(), &disc)
}

fn discovery_capability_flags(runtime_flags: Vec<String>) -> Vec<String> {
    let mut flags = std::collections::BTreeSet::new();
    flags.insert(flags::BOOT_STATUS.to_string());
    flags.insert(flags::CONTROL_DIAGNOSTICS.to_string());
    flags.extend(
        runtime_flags
            .into_iter()
            .map(|flag| flag.trim().to_string())
            .filter(|flag| !flag.is_empty()),
    );
    flags.into_iter().collect()
}

/// Test-only booting-state server harness for control-plane client tests.
///
/// `serve_connection` is private on purpose — production code only
/// reaches it via `accept_loop`. The FFI client tests need to drive
/// exactly one connection without exposing product ability dispatch
/// over JSON control, so this helper intentionally starts in booting
/// state and accepts only boot/status traffic.
#[cfg(test)]
#[cfg(unix)]
pub(crate) async fn serve_booting_one_for_test(
    stream: UnixStream,
    boot: BootBus,
) -> anyhow::Result<()> {
    serve_connection(stream, ControlServerState::new(boot)).await
}

/// Test-only booting-state server harness for control-plane client tests.
#[cfg(test)]
#[cfg(windows)]
pub(crate) async fn serve_booting_one_for_test(
    stream: NamedPipeServer,
    boot: BootBus,
) -> anyhow::Result<()> {
    serve_connection(stream, ControlServerState::new(boot)).await
}

#[cfg(test)]
mod tests {
    use super::*;
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

    #[tokio::test]
    async fn unknown_control_subscription_returns_not_found() {
        let boot = BootBus::new();
        let state = ControlServerState::new(boot);
        let (out_tx, mut out_rx) = mpsc::channel::<OutgoingFrame>(4);
        let cancel: CancelRegistry = Arc::new(Mutex::new(HashMap::new()));

        handle_request(
            IncomingFrame::Subscribe {
                subscription_id: "unknown-sub".into(),
                ability: "observe.health".into(),
                args: serde_json::json!({}),
            },
            out_tx,
            &cancel,
            &state,
        )
        .await;

        match out_rx.recv().await.expect("not-found response") {
            OutgoingFrame::Error {
                subscription_id,
                code,
                ..
            } => {
                assert_eq!(subscription_id.as_deref(), Some("unknown-sub"));
                assert_eq!(code, crate::daemon::control::frames::codes::NOT_FOUND);
            }
            other => panic!("expected not-found Error frame, got {other:?}"),
        }
    }

    #[tokio::test]
    async fn watch_boot_subscription_receives_ready_terminal() {
        let boot = BootBus::new();
        let state = ControlServerState::new(boot.clone());
        let (out_tx, mut out_rx) = mpsc::channel::<OutgoingFrame>(4);
        let cancel: CancelRegistry = Arc::new(Mutex::new(HashMap::new()));

        handle_request(
            IncomingFrame::Subscribe {
                subscription_id: "boot-sub".into(),
                ability: WATCH_BOOT_ABILITY.into(),
                args: serde_json::json!({}),
            },
            out_tx,
            &cancel,
            &state,
        )
        .await;
        boot.emit_ready();

        let first = tokio::time::timeout(std::time::Duration::from_secs(1), out_rx.recv())
            .await
            .expect("ready frame timeout")
            .expect("ready frame");
        match first {
            OutgoingFrame::Frame {
                subscription_id,
                frame,
            } => {
                assert_eq!(subscription_id, "boot-sub");
                let event: BootEvent = serde_json::from_value(frame).unwrap();
                assert_eq!(event, BootEvent::Ready);
            }
            other => panic!("expected Ready boot frame, got {other:?}"),
        }

        let second = tokio::time::timeout(std::time::Duration::from_secs(1), out_rx.recv())
            .await
            .expect("terminal frame timeout")
            .expect("terminal frame");
        match second {
            OutgoingFrame::Terminal {
                subscription_id,
                reason,
            } => {
                assert_eq!(subscription_id, "boot-sub");
                assert_eq!(reason, "done");
            }
            other => panic!("expected boot terminal frame, got {other:?}"),
        }
    }

    /// Malformed JSON in a frame must yield a `PROTOCOL` Error
    /// response without dropping the connection. Pin both halves.
    #[cfg(unix)]
    #[tokio::test]
    async fn malformed_frame_yields_protocol_error_and_keeps_connection_open() {
        let dir = unique_tmp();
        let path = dir.join("bad.sock");

        let (listener, _addr) = transport::bind_at(&path).expect("bind");
        let boot = BootBus::new();
        boot.emit_ready();
        let server_task = tokio::spawn(async move {
            #[cfg(unix)]
            if let ControlListener::Uds(uds) = listener {
                let (stream, _) = uds.accept().await.unwrap();
                serve_connection(stream, ControlServerState::new(boot))
                    .await
                    .unwrap();
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
                assert_eq!(code, crate::daemon::control::frames::codes::PROTOCOL);
            }
            other => panic!("expected PROTOCOL error, got {other:?}"),
        }

        // Now send a valid boot/status frame to confirm the connection survived.
        let req = IncomingFrame::Subscribe {
            subscription_id: "after-bad".into(),
            ability: WATCH_BOOT_ABILITY.into(),
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
        match resp {
            OutgoingFrame::Frame {
                subscription_id,
                frame,
            } => {
                assert_eq!(subscription_id, "after-bad");
                assert_eq!(frame["type"], "ready");
            }
            other => panic!("expected Ready boot frame after recovery, got {other:?}"),
        }

        drop(client);
        let _ = server_task.await;
    }
}
