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
    InvokeBidiUp, StreamDescriptor,
};
use easynet_cli::services::axon_serve::admission_facade::AdmissionFacade;
use easynet_cli::services::axon_serve::daemon_invocation_service::DaemonInvocationService;
use easynet_cli::services::axon_serve::invoke_remote_initiator::{
    InvokeRemoteUp, SessionDispatch, ABILITY_INVOKE_REMOTE, INVOKE_REMOTE_STREAM_ID,
};
use easynet_cli::services::axon_serve::session_initiator::{
    ABILITY_SELF_SESSION, SESSION_STREAM_ID,
};
use easynet_cli::services::pending_dispatch::PendingDispatchMap;
use easynet_cli::services::presence_registry::PresenceRegistry;
use easynet_cli::services::realm_trust_anchor::RealmTrustAnchor;
use futures::StreamExt;
use tokio::net::UnixListener;
use tokio::sync::mpsc;
use tokio_stream::wrappers::{ReceiverStream, UnixListenerStream};
use tonic::transport::{Channel, Endpoint, Server, Uri};
use tonic::Request;

const DEVICE_A_URI: &str = "easynet:///r/test-realm/agent/device-a";
const DEVICE_B_URI: &str = "easynet:///r/test-realm/agent/device-b";

/// 5-second bound on every blocking await in the test. Real
/// transport plane round-trips finish in milliseconds; any test
/// step that takes more than 5 s in this fully-in-process setup
/// is a bug.
const STEP_TIMEOUT: Duration = Duration::from_secs(5);

/// Marker payload device B replies with — easy to assert against
/// the bytes that come out of the originating invoke_remote
/// stream.
const REPLY_MARKER: &[u8] = b"hello-from-device-b";

/// Spawn a real tonic InvocationServer on a tempfile UDS, return
/// the path. The server runs forever on the tokio runtime; the
/// test does not need to shut it down (the runtime drops at
/// test exit). The hub admits both DEVICE_A_URI and DEVICE_B_URI
/// via a synthetic realm trust anchor.
async fn start_in_process_hub() -> std::path::PathBuf {
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
agent_uri = "{DEVICE_A_URI}"
public_key_b64 = "AAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAA="
role = "device"
added_at_unix_ms = 0

[[trusted_agent]]
agent_uri = "{DEVICE_B_URI}"
public_key_b64 = "AAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAA="
role = "device"
added_at_unix_ms = 0
        "#,
    );
    let mut f = std::fs::File::create(&trust_path).expect("write trust toml");
    f.write_all(trust_toml.as_bytes()).expect("write");
    drop(f);

    let trust_anchor =
        RealmTrustAnchor::try_load_strict(&trust_path).expect("load trust anchor");

    // Leak the tempdir AFTER the trust file is loaded so the UDS
    // path stays valid for the test duration; the process exit
    // cleans up.
    Box::leak(Box::new(tempdir));
    let presence = Arc::new(PresenceRegistry::new());
    let pending = Arc::new(PendingDispatchMap::new());
    let admission = AdmissionFacade::new(Arc::new(trust_anchor), None);
    let service = DaemonInvocationService::new(Arc::clone(&presence), admission)
        .with_pending(Arc::clone(&pending));

    let listener = UnixListener::bind(&socket_path).expect("bind UDS");
    let incoming = UnixListenerStream::new(listener);

    tokio::spawn(async move {
        let _ = Server::builder()
            .add_service(InvocationServer::new(service))
            .serve_with_incoming(incoming)
            .await;
    });

    // Tiny wait so the server is ready to accept before the test
    // dials. tonic's `Server::serve_with_incoming` does not expose
    // a "ready" signal; a millisecond yield is enough on the same
    // tokio runtime.
    tokio::time::sleep(Duration::from_millis(20)).await;

    socket_path
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

async fn open_device_session(channel: Channel, caller_uri: &str) -> mpsc::Sender<InvokeBidiUp> {
    let mut client = InvocationClient::new(channel);

    // Build a frame-0 EnvelopeOpen identifying this device.
    let envelope_open = InvokeBidiUp {
        sequence: 0,
        payload: Some(UpPayload::EnvelopeOpen(EnvelopeOpen {
            envelope: Some(Envelope {
                caller: Some(AgentIdentity {
                    uri: caller_uri.to_string(),
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

    // Spawn a task that holds the bidi open: send the request,
    // and forever drain the down-stream into a Vec we never read
    // back. The test does its assertions through the auxiliary
    // hub-side channels (the device-A `<self>.session` is just a
    // presence holder; the device-B session participates in the
    // round-trip by listening on a SHARED down-frame consumer
    // we set up separately for B below).
    tokio::spawn(async move {
        let response = client.invoke_bidi(Request::new(outbound)).await;
        if let Ok(response) = response {
            let mut down = response.into_inner();
            // Drain forever; in this test the bidi stays open
            // until the test exits and the runtime drops it.
            while let Some(_frame) = down.next().await {
                // Nothing to do for device A; device B's session
                // is opened via `open_device_session_with_drain`
                // below if it needs to inspect frames.
            }
        }
    });

    up_tx
}

/// Like `open_device_session` but also returns a receiver over
/// the hub's down-stream so the test can react to dispatched
/// frames (e.g. SessionDispatch::Dispatch arriving for device B).
async fn open_device_session_with_drain(
    channel: Channel,
    caller_uri: &str,
) -> (
    mpsc::Sender<InvokeBidiUp>,
    mpsc::Receiver<InvokeBidiDown>,
) {
    let mut client = InvocationClient::new(channel);

    let envelope_open = InvokeBidiUp {
        sequence: 0,
        payload: Some(UpPayload::EnvelopeOpen(EnvelopeOpen {
            envelope: Some(Envelope {
                caller: Some(AgentIdentity {
                    uri: caller_uri.to_string(),
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
    tokio::spawn(async move {
        let response = client.invoke_bidi(Request::new(outbound)).await;
        if let Ok(response) = response {
            let mut down = response.into_inner();
            while let Some(frame) = down.next().await {
                match frame {
                    Ok(f) => {
                        let _ = down_tx.send(f).await;
                    }
                    Err(_) => break,
                }
            }
        }
    });

    (up_tx, down_rx)
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn cross_device_invoke_remote_round_trip() {
    let socket_path = start_in_process_hub().await;

    // Step 1: open device A's `<self>.session`. Hub registers
    // DEVICE_A_URI in its PresenceRegistry.
    let channel_a = connect_to_hub(&socket_path).await;
    let _device_a_up = open_device_session(channel_a.clone(), DEVICE_A_URI).await;

    // Step 2: open device B's `<self>.session` with a drain so
    // we can see the SessionDispatch::Dispatch frame the hub
    // pushes when device A invokes ability on B.
    let channel_b = connect_to_hub(&socket_path).await;
    let (device_b_up, mut device_b_down) =
        open_device_session_with_drain(channel_b, DEVICE_B_URI).await;

    // Brief settle so PresenceRegistry sees both inserts before
    // step 3's invoke_remote tries to look up B.
    tokio::time::sleep(Duration::from_millis(50)).await;

    // Step 3: spawn device B's reply task. As soon as a
    // BinaryChunk SessionDispatch::Dispatch arrives, device B
    // sends back SessionDispatch::Result with REPLY_MARKER.
    let reply_task_handle = tokio::spawn(async move {
        // Wait for the dispatch frame from the hub.
        let frame = tokio::time::timeout(STEP_TIMEOUT, device_b_down.recv())
            .await
            .expect("device B receives dispatch within bound")
            .expect("dispatch frame is Some");

        let DownPayload::BinaryChunk(chunk) = frame.payload.expect("payload") else {
            panic!("device B expected BinaryChunk payload");
        };

        let dispatch: SessionDispatch =
            serde_json::from_slice(&chunk.data).expect("decode SessionDispatch");

        let SessionDispatch::Dispatch {
            call_id,
            ability,
            args: _,
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

        let reply_frame = InvokeBidiUp {
            sequence: 1,
            payload: Some(UpPayload::BinaryChunk(BinaryChunk {
                data: payload,
                ..BinaryChunk::default()
            })),
            ..InvokeBidiUp::default()
        };
        device_b_up.send(reply_frame).await.expect("send reply up");

        ability
    });

    // Step 4: device A invokes <self>.invoke_remote(target=B,
    // ability=test.echo, args=...). Open a per-call bidi.
    let channel_caller = connect_to_hub(&socket_path).await;
    let mut caller_client = InvocationClient::new(channel_caller);

    let invoke_remote_request = InvokeRemoteUp::Request {
        subject_device: DEVICE_B_URI.to_string(),
        ability: "test.echo".to_string(),
        args: b"args-from-A".to_vec(),
    };
    let initial_args = serde_json::to_vec(&invoke_remote_request).expect("encode request");

    let invoke_remote_open = InvokeBidiUp {
        sequence: 0,
        payload: Some(UpPayload::EnvelopeOpen(EnvelopeOpen {
            envelope: Some(Envelope {
                caller: Some(AgentIdentity {
                    uri: DEVICE_A_URI.to_string(),
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
