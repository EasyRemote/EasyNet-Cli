//! Cross-device `<self>.invoke_remote` end-to-end integration test
//! ================================================================
//!
//! File: tests/cross_device_invoke_remote_e2e.rs
//! Purpose: PR-3 commit 3/3 acceptance test (mohao, per CTO
//!          ownership handoff letter 26). Proves the entire RFC-003
//!          transport plane is wire-correct end-to-end on a real
//!          tonic gRPC server via UDS, with two simulated devices,
//!          a hub-side `DaemonInvocationService`, and a real
//!          `<self>.invoke_remote` round-trip.
//!
//! What this test exercises (the real bytes, not mocks)
//! ----------------------------------------------------
//! 1. A real `tonic::transport::Server` hosts
//!    `DaemonInvocationService` with `PendingDispatchMap`
//!    injected, on a tempfile UDS.
//! 2. Two simulated devices each open a real `InvokeBidi` RPC
//!    against `<self>.session` and hold the bidi for the test
//!    duration. The hub registers each in the `PresenceRegistry`.
//! 3. One device opens a separate per-call `InvokeBidi` against
//!    `<self>.invoke_remote` targeting the other device.
//! 4. Hub's `dispatch_invoke_remote` registers a pending entry,
//!    pushes a `SessionDispatch::Dispatch` frame down the
//!    target's reverse channel.
//! 5. The target's `<self>.session` test client reads the frame,
//!    replies with a known `SessionDispatch::Result` carrying a
//!    test-marker payload.
//! 6. Hub's `drain_session_up_stream` parses the reply, calls
//!    `pending.complete(call_id, ...)`.
//! 7. The originating `<self>.invoke_remote` stream yields a
//!    terminal `InvokeRemoteDown::Result` carrying the same
//!    test-marker payload.
//! 8. Test asserts the round-trip completes within a bounded
//!    `tokio::time::timeout` so a regression that hangs the
//!    pipeline surfaces as a test failure, not a CI hang.
//!
//! Per `team-work/conventions/cargo-discipline.md`: the test runs
//! in a single tokio runtime, no parallel cargo invocations, and
//! every blocking await is wrapped in a 5-second timeout to
//! preserve "real bug means test failure, never hang".
//!
//! Author: Silan Hu <silan.hu@u.nus.edu>
//! Copyright (c) 2026 EasyNet. All rights reserved.

#![cfg(feature = "axon-pb")]

use std::sync::Arc;
use std::time::Duration;

use easynet_cli::pb::axon::v1::invocation_client::InvocationClient;
use easynet_cli::pb::axon::v1::invocation_server::InvocationServer;
use easynet_cli::pb::axon::v1::invoke_bidi_down::Payload as DownPayload;
use easynet_cli::pb::axon::v1::invoke_bidi_up::Payload as UpPayload;
use easynet_cli::pb::axon::v1::{
    AgentIdentity, BinaryChunk, Envelope, EnvelopeOpen, InvocationTarget, InvokeBidiDown,
    InvokeBidiUp, InvokeServerStreamRequest, StreamDescriptor,
};
use easynet_cli::runtime::ability_dispatch::{AbilityDispatcher, LocalAbilityRegistry};
use easynet_cli::runtime::gateway::NoopGateway;
use easynet_cli::services::axon_serve::admission_facade::AdmissionFacade;
use easynet_cli::services::axon_serve::daemon_invocation_service::DaemonInvocationService;
use easynet_cli::services::axon_serve::invoke_remote_initiator::{
    InvokeRemoteUp, SessionContentEnvelope, SessionDispatch, ABILITY_INVOKE_REMOTE,
    INVOKE_REMOTE_STREAM_ID,
};
use easynet_cli::services::axon_serve::local_ability_dispatcher::LocalAbilityDispatcher;
use easynet_cli::services::axon_serve::session_initiator::{
    SessionFrameDispatcher, SessionUpSender, ABILITY_SELF_SESSION, SESSION_STREAM_ID,
};
use easynet_cli::services::pending_dispatch::PendingDispatchMap;
use easynet_cli::services::presence_registry::{OfflineReason, PresenceEvent, PresenceRegistry};
use easynet_cli::services::realm_trust_anchor::RealmTrustAnchor;
use futures::StreamExt;
use serde_json::json;
use tokio::net::UnixListener;
use tokio::sync::{mpsc, oneshot};
use tokio::task::JoinHandle;
use tokio_stream::wrappers::{ReceiverStream, UnixListenerStream};
use tonic::transport::{Channel, Endpoint, Server, Uri};
use tonic::Request;

const DEVICE_A_URI: &str = "easynet:///r/test-realm/device/device-a";
const DEVICE_B_URI: &str = "easynet:///r/test-realm/device/device-b";

/// 5-second bound on every blocking await in the test. Real
/// transport plane round-trips finish in milliseconds; any test
/// step that takes more than 5 s in this fully-in-process setup
/// is a bug.
const STEP_TIMEOUT: Duration = Duration::from_secs(5);

/// Marker payload device B replies with — easy to assert against
/// the bytes that come out of the originating invoke_remote
/// stream.
const REPLY_MARKER: &[u8] = b"hello-from-device-b";

/// Owns every resource the in-process hub holds open so the test
/// can deterministically tear them down before the tokio runtime
/// drops. Each field is load-bearing for "cargo test never leaves
/// orphan binaries":
///
/// - `tempdir` keeps the UDS path + trust file alive; held here
///   instead of `Box::leak`'d so it is removed on Drop.
/// - `shutdown` triggers `serve_with_incoming_shutdown` to return,
///   which exits the spawned server task on demand instead of "when
///   the runtime drops".
/// - `server` is the JoinHandle for that task; on Drop we send the
///   shutdown signal and abort() the handle as a safety net, so the
///   task is gone before the runtime tears down.
struct TestHub {
    socket_path: std::path::PathBuf,
    presence: Arc<PresenceRegistry>,
    _tempdir: tempfile::TempDir,
    shutdown: Option<oneshot::Sender<()>>,
    server: Option<JoinHandle<()>>,
}

impl TestHub {
    fn socket_path(&self) -> &std::path::Path {
        &self.socket_path
    }

    fn presence(&self) -> Arc<PresenceRegistry> {
        Arc::clone(&self.presence)
    }
}

impl Drop for TestHub {
    fn drop(&mut self) {
        if let Some(tx) = self.shutdown.take() {
            // Receiver lives on the spawned server task. send() may
            // fail if the task already exited (panic, bind error);
            // either way we abort below to guarantee the task is
            // gone before this guard is dropped.
            let _ = tx.send(());
        }
        if let Some(handle) = self.server.take() {
            // We are inside Drop, possibly on a tokio worker thread.
            // block_on inside a worker would panic ("Cannot start a
            // runtime from within a runtime"), so we abort instead.
            // serve_with_incoming_shutdown's future has already been
            // signalled above; abort is the safety net for tasks
            // stuck on a downstream await (e.g. a slow client).
            handle.abort();
        }
    }
}

/// Spawn a real tonic InvocationServer on a tempfile UDS. Returns
/// a `TestHub` guard whose Drop signals the server to shut down
/// and joins the spawned task. The hub admits both DEVICE_A_URI
/// and DEVICE_B_URI via a synthetic realm trust anchor.
async fn start_in_process_hub() -> TestHub {
    use std::io::Write;

    let tempdir = tempfile::tempdir().expect("tempdir");
    let socket_path = tempdir.path().join("hub.sock");

    // Build a trust anchor by writing a real TOML file and
    // loading it via the production loader path. Both devices
    // admitted; empty anchor would reject everyone external.
    let trust_path = tempdir.path().join("realm-trust.toml");
    let trust_toml = format!(
        r#"
[[trusted_agent]]
agent_ura = "{DEVICE_A_URI}"
public_key_b64 = "AAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAA="
role = "device"
added_at_unix_ms = 0

[[trusted_agent]]
agent_ura = "{DEVICE_B_URI}"
public_key_b64 = "AAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAA="
role = "device"
added_at_unix_ms = 0
        "#,
    );
    let mut f = std::fs::File::create(&trust_path).expect("write trust toml");
    f.write_all(trust_toml.as_bytes()).expect("write");
    drop(f);

    let trust_anchor = RealmTrustAnchor::try_load_strict(&trust_path).expect("load trust anchor");

    let presence = Arc::new(PresenceRegistry::new());
    let pending = Arc::new(PendingDispatchMap::new());
    let admission = AdmissionFacade::new(Arc::new(trust_anchor), None);
    let service = DaemonInvocationService::new(Arc::clone(&presence), admission)
        .with_pending(Arc::clone(&pending));

    let listener = UnixListener::bind(&socket_path).expect("bind UDS");
    let incoming = UnixListenerStream::new(listener);

    let (shutdown_tx, shutdown_rx) = oneshot::channel::<()>();
    let server = tokio::spawn(async move {
        let _ = Server::builder()
            .add_service(InvocationServer::new(service))
            .serve_with_incoming_shutdown(incoming, async {
                let _ = shutdown_rx.await;
            })
            .await;
    });

    // Tiny wait so the server is ready to accept before the test
    // dials. tonic's `Server::serve_with_incoming` does not expose
    // a "ready" signal; a millisecond yield is enough on the same
    // tokio runtime.
    tokio::time::sleep(Duration::from_millis(20)).await;

    TestHub {
        socket_path,
        presence,
        _tempdir: tempdir,
        shutdown: Some(shutdown_tx),
        server: Some(server),
    }
}

/// Connect to the hub's UDS and return a tonic `Channel` ready
/// for `InvocationClient::new`.
async fn connect_to_hub(socket_path: &std::path::Path) -> Channel {
    let socket_path = socket_path.to_path_buf();
    Endpoint::try_from("http://[::]:50051")
        .expect("dummy endpoint")
        .connect_with_connector(tower::service_fn(move |_: Uri| {
            let path = socket_path.clone();
            async move {
                let stream = tokio::net::UnixStream::connect(path).await?;
                Ok::<_, std::io::Error>(hyper_util::rt::TokioIo::new(stream))
            }
        }))
        .await
        .expect("connect to hub")
}

fn subscribe_directory_request(caller_ura: &str) -> Request<InvokeServerStreamRequest> {
    Request::new(InvokeServerStreamRequest {
        envelope: Some(Envelope {
            caller: Some(AgentIdentity {
                ura: caller_ura.to_string(),
                ..AgentIdentity::default()
            }),
            ..Envelope::default()
        }),
        function_name: "federation.subscribe_directory".to_string(),
        ..InvokeServerStreamRequest::default()
    })
}

fn build_test_echo_dispatcher() -> Arc<AbilityDispatcher> {
    let mut registry = LocalAbilityRegistry::new();
    registry.register_rpc("test.echo", Arc::new(|args| Ok(args)));
    let gateway: Arc<dyn easynet_cli::runtime::gateway_api::GatewayApi> =
        Arc::new(NoopGateway::new());
    Arc::new(AbilityDispatcher::new(Arc::new(registry), gateway))
}

/// A device-side `<self>.session` bidi held open for the duration
/// of the test. Owns the up-stream sender (so the test can push
/// frames) and the JoinHandle of the spawned drain task. On Drop
/// the up-sender is dropped first — tonic closes the request stream
/// which exits the drain loop's `while let Some(_) = down.next()`
/// — and abort() then guarantees the task is collected even if the
/// server side is wedged. Replaces the old "spawn forever, runtime
/// drops at process exit" pattern that produced 5h+ orphan binaries
/// when test parents died abnormally.
struct DeviceSession {
    up_tx: Option<mpsc::Sender<InvokeBidiUp>>,
    drain: Option<JoinHandle<()>>,
}

impl DeviceSession {
    fn up(&self) -> &mpsc::Sender<InvokeBidiUp> {
        self.up_tx.as_ref().expect("up_tx not yet dropped")
    }
}

impl Drop for DeviceSession {
    fn drop(&mut self) {
        // Drop the up-stream sender first: this closes the request
        // stream tonic is reading from, the bidi tears down, and
        // the drain task's `while let Some(_) = down.next().await`
        // exits cleanly. abort() afterwards is the safety net for a
        // drain stuck waiting on the server side; we cannot
        // block_on a JoinHandle from inside Drop because the worker
        // thread is already running a runtime.
        self.up_tx.take();
        if let Some(handle) = self.drain.take() {
            handle.abort();
        }
    }
}

async fn open_device_session(channel: Channel, caller_ura: &str) -> DeviceSession {
    let mut client = InvocationClient::new(channel);

    // Build a frame-0 EnvelopeOpen identifying this device.
    let envelope_open = InvokeBidiUp {
        sequence: 0,
        payload: Some(UpPayload::EnvelopeOpen(EnvelopeOpen {
            envelope: Some(Envelope {
                caller: Some(AgentIdentity {
                    ura: caller_ura.to_string(),
                    ..AgentIdentity::default()
                }),
                ..Envelope::default()
            }),
            target: Some(InvocationTarget {
                ability_name: ABILITY_SELF_SESSION.to_string(),
                ..InvocationTarget::default()
            }),
            streams: vec![StreamDescriptor {
                stream_id: SESSION_STREAM_ID,
                content_type: "application/json".to_string(),
                ..StreamDescriptor::default()
            }],
            ..EnvelopeOpen::default()
        })),
        ..InvokeBidiUp::default()
    };

    let (up_tx, up_rx) = mpsc::channel::<InvokeBidiUp>(8);
    up_tx.send(envelope_open).await.expect("send frame 0");

    let outbound = ReceiverStream::new(up_rx);

    let drain = tokio::spawn(async move {
        let response = client.invoke_bidi(Request::new(outbound)).await;
        if let Ok(response) = response {
            let mut down = response.into_inner();
            while let Some(_frame) = down.next().await {
                // Device A is presence-only; device B's session
                // uses open_device_session_with_drain. Either way
                // the loop exits when the bidi is torn down by
                // up-sender drop in DeviceSession::drop.
            }
        }
    });

    DeviceSession {
        up_tx: Some(up_tx),
        drain: Some(drain),
    }
}

/// Like `open_device_session` but also returns a receiver over
/// the hub's down-stream so the test can react to dispatched
/// frames (e.g. SessionDispatch::Dispatch arriving for device B).
async fn open_device_session_with_drain(
    channel: Channel,
    caller_ura: &str,
) -> (DeviceSession, mpsc::Receiver<InvokeBidiDown>) {
    let mut client = InvocationClient::new(channel);

    let envelope_open = InvokeBidiUp {
        sequence: 0,
        payload: Some(UpPayload::EnvelopeOpen(EnvelopeOpen {
            envelope: Some(Envelope {
                caller: Some(AgentIdentity {
                    ura: caller_ura.to_string(),
                    ..AgentIdentity::default()
                }),
                ..Envelope::default()
            }),
            target: Some(InvocationTarget {
                ability_name: ABILITY_SELF_SESSION.to_string(),
                ..InvocationTarget::default()
            }),
            streams: vec![StreamDescriptor {
                stream_id: SESSION_STREAM_ID,
                content_type: "application/json".to_string(),
                ..StreamDescriptor::default()
            }],
            ..EnvelopeOpen::default()
        })),
        ..InvokeBidiUp::default()
    };

    let (up_tx, up_rx) = mpsc::channel::<InvokeBidiUp>(8);
    up_tx.send(envelope_open).await.expect("send frame 0");
    let outbound = ReceiverStream::new(up_rx);

    let (down_tx, down_rx) = mpsc::channel::<InvokeBidiDown>(8);
    let drain = tokio::spawn(async move {
        let response = client.invoke_bidi(Request::new(outbound)).await;
        if let Ok(response) = response {
            let mut down = response.into_inner();
            while let Some(frame) = down.next().await {
                match frame {
                    Ok(f) => {
                        if down_tx.send(f).await.is_err() {
                            break;
                        }
                    }
                    Err(_) => break,
                }
            }
        }
    });

    (
        DeviceSession {
            up_tx: Some(up_tx),
            drain: Some(drain),
        },
        down_rx,
    )
}

async fn recv_next_binary_chunk_frame(down: &mut mpsc::Receiver<InvokeBidiDown>) -> InvokeBidiDown {
    loop {
        let frame = tokio::time::timeout(STEP_TIMEOUT, down.recv())
            .await
            .expect("device receives session frame within bound")
            .expect("session frame is Some");
        if matches!(frame.payload, Some(DownPayload::BinaryChunk(_))) {
            return frame;
        }
    }
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn cross_device_invoke_remote_round_trip() {
    // Outer 60s timeout: any future spawn-without-cancel regression
    // surfaces as a test failure within bound, never as a multi-hour
    // orphan binary holding target/. The body's own STEP_TIMEOUTs
    // will fire first on a real bug; this is the belt-and-braces
    // catch for unknown unknowns.
    tokio::time::timeout(Duration::from_secs(60), async {
        run_round_trip().await;
    })
    .await
    .expect("test completes within 60 s");
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn cross_device_invoke_remote_round_trip_via_local_ability_dispatcher() {
    tokio::time::timeout(Duration::from_secs(60), async {
        run_round_trip_via_local_dispatcher().await;
    })
    .await
    .expect("test completes within 60 s");
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn session_bidi_rejects_non_envelope_open_first_frame() {
    tokio::time::timeout(Duration::from_secs(60), async {
        let hub = start_in_process_hub().await;
        let channel = connect_to_hub(hub.socket_path()).await;
        let mut client = InvocationClient::new(channel);

        let (up_tx, up_rx) = mpsc::channel::<InvokeBidiUp>(1);
        up_tx
            .send(InvokeBidiUp {
                sequence: 0,
                payload: Some(UpPayload::BinaryChunk(BinaryChunk {
                    data: br#"{}"#.to_vec(),
                    ..BinaryChunk::default()
                })),
                ..InvokeBidiUp::default()
            })
            .await
            .expect("send malformed frame 0");
        drop(up_tx);

        let status = match client
            .invoke_bidi(Request::new(ReceiverStream::new(up_rx)))
            .await
        {
            Err(status) => status,
            Ok(response) => match response.into_inner().next().await {
                Some(Err(status)) => status,
                Some(Ok(_)) => panic!("malformed frame 0 must not yield a down-stream frame"),
                None => panic!("malformed frame 0 must surface an error status"),
            },
        };

        assert_eq!(status.code(), tonic::Code::InvalidArgument);
        assert!(
            status.message().contains("EnvelopeOpen"),
            "reject must name the frame-0 contract, got: {}",
            status.message()
        );
        assert!(
            hub.presence().snapshot().is_empty(),
            "malformed frame 0 must not register a live session"
        );
    })
    .await
    .expect("test completes within 60 s");
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn session_graceful_close_emits_stream_closed_offline() {
    tokio::time::timeout(Duration::from_secs(60), async {
        let hub = start_in_process_hub().await;
        let presence = hub.presence();
        let mut events = presence.subscribe_events();

        let channel = connect_to_hub(hub.socket_path()).await;
        let device = open_device_session(channel, DEVICE_A_URI).await;

        match tokio::time::timeout(STEP_TIMEOUT, events.recv())
            .await
            .expect("online event arrives within bound")
            .expect("online event is delivered")
        {
            PresenceEvent::Online { ura } => assert_eq!(ura, DEVICE_A_URI),
            other => panic!("expected Online event, got {other:?}"),
        }

        drop(device);

        match tokio::time::timeout(STEP_TIMEOUT, events.recv())
            .await
            .expect("offline event arrives within bound")
            .expect("offline event is delivered")
        {
            PresenceEvent::Offline { ura, reason } => {
                assert_eq!(ura, DEVICE_A_URI);
                assert_eq!(reason, OfflineReason::StreamClosed);
            }
            other => panic!("expected Offline(StreamClosed), got {other:?}"),
        }

        assert!(
            presence.lookup(DEVICE_A_URI).is_none(),
            "graceful close must remove the device from PresenceRegistry"
        );
    })
    .await
    .expect("test completes within 60 s");
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn session_duplicate_open_emits_displacement_transition() {
    tokio::time::timeout(Duration::from_secs(60), async {
        let hub = start_in_process_hub().await;
        let presence = hub.presence();

        let first_channel = connect_to_hub(hub.socket_path()).await;
        let first = open_device_session(first_channel, DEVICE_A_URI).await;

        let mut warmup = presence.subscribe_events();
        match tokio::time::timeout(STEP_TIMEOUT, warmup.recv())
            .await
            .expect("initial online event arrives within bound")
            .expect("initial online event is delivered")
        {
            PresenceEvent::Online { ura } => assert_eq!(ura, DEVICE_A_URI),
            other => panic!("expected initial Online event, got {other:?}"),
        }

        let mut events = presence.subscribe_events();
        let second_channel = connect_to_hub(hub.socket_path()).await;
        let second = open_device_session(second_channel, DEVICE_A_URI).await;

        match tokio::time::timeout(STEP_TIMEOUT, events.recv())
            .await
            .expect("displacement offline arrives within bound")
            .expect("displacement offline is delivered")
        {
            PresenceEvent::Offline { ura, reason } => {
                assert_eq!(ura, DEVICE_A_URI);
                assert_eq!(reason, OfflineReason::StreamClosed);
            }
            other => panic!("expected displacement Offline(StreamClosed), got {other:?}"),
        }

        match tokio::time::timeout(STEP_TIMEOUT, events.recv())
            .await
            .expect("replacement online arrives within bound")
            .expect("replacement online is delivered")
        {
            PresenceEvent::Online { ura } => assert_eq!(ura, DEVICE_A_URI),
            other => panic!("expected replacement Online event, got {other:?}"),
        }

        assert!(
            presence.lookup(DEVICE_A_URI).is_some(),
            "replacement session must stay registered after displacement"
        );

        drop(second);
        drop(first);
    })
    .await
    .expect("test completes within 60 s");
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn subscribe_directory_stream_tracks_real_session_online_and_offline() {
    tokio::time::timeout(Duration::from_secs(60), async {
        let hub = start_in_process_hub().await;
        let channel = connect_to_hub(hub.socket_path()).await;
        let mut client = InvocationClient::new(channel);

        let response = tokio::time::timeout(
            STEP_TIMEOUT,
            client.invoke_stream(subscribe_directory_request(DEVICE_A_URI)),
        )
        .await
        .expect("subscribe_directory opens within bound")
        .expect("subscribe_directory request returns Ok");
        let mut stream = response.into_inner();

        let initial = tokio::time::timeout(STEP_TIMEOUT, stream.next())
            .await
            .expect("initial snapshot arrives within bound")
            .expect("initial snapshot exists")
            .expect("initial snapshot frame is Ok");
        let initial_json: serde_json::Value =
            serde_json::from_slice(&initial.payload).expect("initial payload decodes");
        assert_eq!(
            initial_json
                .get("agents")
                .and_then(|v| v.as_array())
                .map(Vec::len),
            Some(0),
            "fresh hub starts with an empty directory snapshot"
        );

        let device_channel = connect_to_hub(hub.socket_path()).await;
        let device = open_device_session(device_channel, DEVICE_B_URI).await;

        let online = tokio::time::timeout(STEP_TIMEOUT, stream.next())
            .await
            .expect("online delta arrives within bound")
            .expect("online delta exists")
            .expect("online delta frame is Ok");
        let online_json: serde_json::Value =
            serde_json::from_slice(&online.payload).expect("online delta decodes");
        assert_eq!(
            online_json.get("kind").and_then(|v| v.as_str()),
            Some("online")
        );
        assert_eq!(
            online_json
                // v1 subscribe_directory stream uses `membership_ura`
                // as the URA field name (per `federation_wrappers`
                // line ~228); v2 introduced `canonical_agent_ura` but
                // this test exercises the v1 surface. An earlier copy
                // of the test asserted the v2 name and silently
                // observed `None`.
                .get("membership_ura")
                .and_then(|v| v.as_str()),
            Some(DEVICE_B_URI)
        );

        drop(device);

        let offline = tokio::time::timeout(STEP_TIMEOUT, stream.next())
            .await
            .expect("offline delta arrives within bound")
            .expect("offline delta exists")
            .expect("offline delta frame is Ok");
        let offline_json: serde_json::Value =
            serde_json::from_slice(&offline.payload).expect("offline delta decodes");
        assert_eq!(
            offline_json.get("kind").and_then(|v| v.as_str()),
            Some("offline")
        );
        assert_eq!(
            offline_json
                // v1 subscribe_directory stream uses `membership_ura`
                // as the URA field name (per `federation_wrappers`
                // line ~228); v2 introduced `canonical_agent_ura` but
                // this test exercises the v1 surface. An earlier copy
                // of the test asserted the v2 name and silently
                // observed `None`.
                .get("membership_ura")
                .and_then(|v| v.as_str()),
            Some(DEVICE_B_URI)
        );
        assert_eq!(
            offline_json.get("reason").and_then(|v| v.as_str()),
            Some("stream_closed")
        );
    })
    .await
    .expect("test completes within 60 s");
}

async fn run_round_trip() {
    let hub = start_in_process_hub().await;
    let socket_path = hub.socket_path();

    // Step 1: open device A's `<self>.session`. Hub registers
    // DEVICE_A_URI in its PresenceRegistry.
    let channel_a = connect_to_hub(socket_path).await;
    let _device_a = open_device_session(channel_a.clone(), DEVICE_A_URI).await;

    // Step 2: open device B's `<self>.session` with a drain so
    // we can see the SessionDispatch::Dispatch frame the hub
    // pushes when device A invokes ability on B.
    let channel_b = connect_to_hub(socket_path).await;
    let (device_b, mut device_b_down) =
        open_device_session_with_drain(channel_b, DEVICE_B_URI).await;

    // Brief settle so PresenceRegistry sees both inserts before
    // step 3's invoke_remote tries to look up B.
    tokio::time::sleep(Duration::from_millis(50)).await;

    // Step 3: spawn device B's reply task. As soon as a
    // BinaryChunk SessionDispatch::Dispatch arrives, device B
    // sends back SessionDispatch::Result with REPLY_MARKER. The
    // up-sender is borrowed via Clone so DeviceSession still owns
    // the original and tears the bidi down on Drop.
    let device_b_up_for_reply = SessionUpSender::new(device_b.up().clone());
    let reply_task_handle = tokio::spawn(async move {
        // Wait for the dispatch frame from the hub.
        let frame = recv_next_binary_chunk_frame(&mut device_b_down).await;

        let DownPayload::BinaryChunk(chunk) = frame.payload.expect("payload") else {
            panic!("device B expected BinaryChunk payload");
        };

        let dispatch: SessionDispatch =
            serde_json::from_slice(&chunk.data).expect("decode SessionDispatch");

        let SessionDispatch::Dispatch {
            call_id,
            ability,
            args: _,
            ..
        } = dispatch
        else {
            panic!("device B expected Dispatch variant");
        };

        // Reply with a typed Result carrying our marker.
        let result = SessionDispatch::Result {
            call_id,
            payload: REPLY_MARKER.to_vec(),
            terminal: true,
            error: None,
        };
        let payload = serde_json::to_vec(&result).expect("encode Result");

        device_b_up_for_reply
            .send_binary_chunk(BinaryChunk {
                data: payload,
                ..BinaryChunk::default()
            })
            .await
            .expect("send reply up");

        ability
    });

    // Step 4: device A invokes <self>.invoke_remote(target=B,
    // ability=test.echo, args=...). Open a per-call bidi.
    let channel_caller = connect_to_hub(socket_path).await;
    let mut caller_client = InvocationClient::new(channel_caller);

    let invoke_remote_request = InvokeRemoteUp::Request {
        subject_device: DEVICE_B_URI.to_string(),
        ability: "test.echo".to_string(),
        args: b"args-from-A".to_vec(),
        args_content_envelope: SessionContentEnvelope::plaintext_json(),
    };
    let initial_args = serde_json::to_vec(&invoke_remote_request).expect("encode request");

    let invoke_remote_open = InvokeBidiUp {
        sequence: 0,
        payload: Some(UpPayload::EnvelopeOpen(EnvelopeOpen {
            envelope: Some(Envelope {
                caller: Some(AgentIdentity {
                    ura: DEVICE_A_URI.to_string(),
                    ..AgentIdentity::default()
                }),
                ..Envelope::default()
            }),
            target: Some(InvocationTarget {
                ability_name: ABILITY_INVOKE_REMOTE.to_string(),
                ..InvocationTarget::default()
            }),
            initial_args,
            streams: vec![StreamDescriptor {
                stream_id: INVOKE_REMOTE_STREAM_ID,
                content_type: "application/json".to_string(),
                ..StreamDescriptor::default()
            }],
            ..EnvelopeOpen::default()
        })),
        ..InvokeBidiUp::default()
    };

    let (caller_up_tx, caller_up_rx) = mpsc::channel::<InvokeBidiUp>(2);
    caller_up_tx
        .send(invoke_remote_open)
        .await
        .expect("send invoke_remote frame 0");
    let caller_outbound = ReceiverStream::new(caller_up_rx);

    let response = tokio::time::timeout(
        STEP_TIMEOUT,
        caller_client.invoke_bidi(Request::new(caller_outbound)),
    )
    .await
    .expect("invoke_bidi opens within bound")
    .expect("invoke_remote bidi returns Ok");

    let mut caller_down = response.into_inner();

    // Step 5: assert the terminal frame arrives within bound,
    // carries a BinaryChunk with InvokeRemoteDown::Result, and
    // that the result payload is REPLY_MARKER.
    let terminal = tokio::time::timeout(STEP_TIMEOUT, caller_down.next())
        .await
        .expect("caller receives reply within bound")
        .expect("at least one frame")
        .expect("frame is Ok");

    let DownPayload::BinaryChunk(chunk) = terminal.payload.expect("payload") else {
        panic!("caller expected BinaryChunk payload");
    };

    use easynet_cli::services::axon_serve::invoke_remote_initiator::InvokeRemoteDown;
    let down: InvokeRemoteDown =
        serde_json::from_slice(&chunk.data).expect("decode InvokeRemoteDown");

    let InvokeRemoteDown::Result { payload, error } = down else {
        panic!("caller expected Result variant");
    };

    assert!(
        error.is_none(),
        "expected no error from device B, got: {error:?}"
    );
    assert_eq!(
        payload, REPLY_MARKER,
        "round-trip payload must match what device B sent"
    );

    // Step 6: confirm device B did receive the dispatched ability
    // name (verifies the hub propagated the request payload, not
    // just the call_id).
    let ability = tokio::time::timeout(STEP_TIMEOUT, reply_task_handle)
        .await
        .expect("reply task completes within bound")
        .expect("reply task did not panic");
    assert_eq!(
        ability, "test.echo",
        "device B must see the ability name device A invoked"
    );
}

async fn run_round_trip_via_local_dispatcher() {
    let hub = start_in_process_hub().await;
    let socket_path = hub.socket_path();

    let channel_a = connect_to_hub(socket_path).await;
    let _device_a = open_device_session(channel_a, DEVICE_A_URI).await;

    let channel_b = connect_to_hub(socket_path).await;
    let (device_b, mut device_b_down) =
        open_device_session_with_drain(channel_b, DEVICE_B_URI).await;
    tokio::time::sleep(Duration::from_millis(50)).await;

    let device_b_up_for_reply = SessionUpSender::new(device_b.up().clone());
    let local_dispatcher = LocalAbilityDispatcher::new(build_test_echo_dispatcher());
    let reply_task_handle = tokio::spawn(async move {
        let frame = recv_next_binary_chunk_frame(&mut device_b_down).await;

        local_dispatcher
            .handle_down(frame, &device_b_up_for_reply)
            .await
            .expect("local ability dispatcher handles frame");
    });

    let channel_caller = connect_to_hub(socket_path).await;
    let mut caller_client = InvocationClient::new(channel_caller);

    let invoke_remote_request = InvokeRemoteUp::Request {
        subject_device: DEVICE_B_URI.to_string(),
        ability: "test.echo".to_string(),
        args: br#"{"echo":"args-from-A"}"#.to_vec(),
        args_content_envelope: SessionContentEnvelope::plaintext_json(),
    };
    let initial_args = serde_json::to_vec(&invoke_remote_request).expect("encode request");

    let invoke_remote_open = InvokeBidiUp {
        sequence: 0,
        payload: Some(UpPayload::EnvelopeOpen(EnvelopeOpen {
            envelope: Some(Envelope {
                caller: Some(AgentIdentity {
                    ura: DEVICE_A_URI.to_string(),
                    ..AgentIdentity::default()
                }),
                ..Envelope::default()
            }),
            target: Some(InvocationTarget {
                ability_name: ABILITY_INVOKE_REMOTE.to_string(),
                ..InvocationTarget::default()
            }),
            initial_args,
            streams: vec![StreamDescriptor {
                stream_id: INVOKE_REMOTE_STREAM_ID,
                content_type: "application/json".to_string(),
                ..StreamDescriptor::default()
            }],
            ..EnvelopeOpen::default()
        })),
        ..InvokeBidiUp::default()
    };

    let (caller_up_tx, caller_up_rx) = mpsc::channel::<InvokeBidiUp>(2);
    caller_up_tx
        .send(invoke_remote_open)
        .await
        .expect("send invoke_remote frame 0");
    let caller_outbound = ReceiverStream::new(caller_up_rx);

    let response = tokio::time::timeout(
        STEP_TIMEOUT,
        caller_client.invoke_bidi(Request::new(caller_outbound)),
    )
    .await
    .expect("invoke_bidi opens within bound")
    .expect("invoke_remote bidi returns Ok");

    let mut caller_down = response.into_inner();
    let terminal = tokio::time::timeout(STEP_TIMEOUT, caller_down.next())
        .await
        .expect("caller receives reply within bound")
        .expect("at least one frame")
        .expect("frame is Ok");

    let DownPayload::BinaryChunk(chunk) = terminal.payload.expect("payload") else {
        panic!("caller expected BinaryChunk payload");
    };

    use easynet_cli::services::axon_serve::invoke_remote_initiator::InvokeRemoteDown;
    let down: InvokeRemoteDown =
        serde_json::from_slice(&chunk.data).expect("decode InvokeRemoteDown");

    let InvokeRemoteDown::Result { payload, error } = down else {
        panic!("caller expected Result variant");
    };

    assert!(
        error.is_none(),
        "expected no error from device B, got: {error:?}"
    );
    let value: serde_json::Value =
        serde_json::from_slice(&payload).expect("device B payload decodes as JSON");
    assert_eq!(value, json!({"echo": "args-from-A"}));

    tokio::time::timeout(STEP_TIMEOUT, reply_task_handle)
        .await
        .expect("reply task completes within bound")
        .expect("reply task did not panic");
}
