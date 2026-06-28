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

use std::collections::HashMap;
use std::sync::Arc;
use std::time::Duration;

use base64::{engine::general_purpose::STANDARD as BASE64_STANDARD, Engine as _};
use easynet_axon::invocation::{
    make_ability, sign_descriptor_bound_invocation, AbilityOptions,
    AgentIdentity as AxiomAgentIdentity, CausalContext, DescriptorBoundEnvelope,
    InvocationEnvelope, LocalRuntime, SubjectIdentity as AxiomSubjectIdentity, UraProfile,
};
use easynet_axon::pb::axon::v1::invocation_client::InvocationClient;
use easynet_axon::pb::axon::v1::invocation_server::InvocationServer;
use easynet_axon::pb::axon::v1::invoke_bidi_down::Payload as DownPayload;
use easynet_axon::pb::axon::v1::invoke_bidi_up::Payload as UpPayload;
use easynet_axon::pb::axon::v1::{
    bidi_control, AgentIdentity, BidiControl, BinaryChunk, CallerSignature, Envelope, EnvelopeOpen,
    InvocationTarget, InvokeBidiDown, InvokeBidiUp, InvokeRequest, InvokeServerStreamRequest,
    StreamDescriptor, SubjectIdentity as PbSubjectIdentity,
};
use easynet_cli::runtime::ability::DEFAULT_ABILITY_DESCRIPTOR_VERSION;
use easynet_cli::services::invocation_transport::admission_facade::AdmissionFacade;
use easynet_cli::services::invocation_transport::daemon_invocation_service::DaemonInvocationService;
use easynet_cli::services::invocation_transport::invocation_wire::ProtoEnvelope;
use easynet_cli::services::invocation_transport::invoke_remote_initiator::{
    InvokeRemoteUp, SessionContentEnvelope, SessionDispatch, ABILITY_INVOKE_REMOTE,
    INVOKE_REMOTE_STREAM_ID,
};
use easynet_cli::services::invocation_transport::local_session_dispatcher::LocalAxonSessionDispatcher;
use easynet_cli::services::invocation_transport::session_initiator::{
    SessionFrameDispatcher, SessionUpSender, ABILITY_SELF_SESSION, SESSION_STREAM_ID,
};
use easynet_cli::services::pending_dispatch::PendingDispatchMap;
use easynet_cli::services::presence_registry::{OfflineReason, PresenceEvent, PresenceRegistry};
use easynet_cli::services::realm_trust_anchor::RealmTrustAnchor;
use ed25519_dalek::SigningKey;
use futures::StreamExt;
use serde_json::json;
use sha2::Digest as _;
use tokio::net::UnixListener;
use tokio::sync::{mpsc, oneshot};
use tokio::task::JoinHandle;
use tokio_stream::wrappers::{ReceiverStream, UnixListenerStream};
use tonic::transport::{Channel, Endpoint, Server, Uri};
use tonic::Request;

const DEVICE_A_URI: &str = "easynet:///r/test-realm/device/device-a";
const DEVICE_B_URI: &str = "easynet:///r/test-realm/device/device-b";
const DEVICE_B_ECHO_ABILITY_URA: &str = "easynet:///r/test-realm/ability/device.device-b.test.echo";
const DEVICE_B_ECHO_PUBLIC_NAME: &str = "test.echo";
const ADVERTISE_ABILITIES: &str = "federation.advertise_abilities";
const DEVICE_A_SIGNING_SEED: [u8; 32] = [0xA1; 32];
const DEVICE_B_SIGNING_SEED: [u8; 32] = [0xB2; 32];
const DEFAULT_URA_PROFILE: &str = "easynet-strict-v2";
const SIGNED_DESCRIPTOR_REF_METADATA_KEY: &str = "x-easynet-signed-descriptor-ref";
const TEST_SCHEMA_HASH: [u8; 32] = [0x33; 32];
const TEST_IMPL_HASH: [u8; 32] = [0x44; 32];

/// 5-second bound on every blocking await in the test. Real
/// transport plane round-trips finish in milliseconds; any test
/// step that takes more than 5 s in this fully-in-process setup
/// is a bug.
const STEP_TIMEOUT: Duration = Duration::from_secs(5);

/// Marker payload device B replies with — easy to assert against
/// the bytes that come out of the originating invoke_remote
/// stream.
const REPLY_MARKER: &[u8] = b"hello-from-device-b";

fn signing_key_for(caller_ura: &str) -> SigningKey {
    match caller_ura {
        DEVICE_A_URI => SigningKey::from_bytes(&DEVICE_A_SIGNING_SEED),
        DEVICE_B_URI => SigningKey::from_bytes(&DEVICE_B_SIGNING_SEED),
        other => panic!("no test signing key configured for {other}"),
    }
}

fn public_key_b64_for(caller_ura: &str) -> String {
    BASE64_STANDARD.encode(signing_key_for(caller_ura).verifying_key().to_bytes())
}

fn descriptor_proof_options(options: AbilityOptions) -> AbilityOptions {
    options.with_descriptor_proof(
        DEFAULT_ABILITY_DESCRIPTOR_VERSION,
        TEST_SCHEMA_HASH,
        TEST_IMPL_HASH,
    )
}

struct SignedCallEnvelope {
    envelope: Envelope,
    descriptor_ref: String,
}

impl SignedCallEnvelope {
    fn metadata(&self) -> HashMap<String, String> {
        HashMap::from([(
            SIGNED_DESCRIPTOR_REF_METADATA_KEY.to_string(),
            self.descriptor_ref.clone(),
        )])
    }
}

fn signed_envelope(
    caller_ura: &str,
    callee_ura: &str,
    subject_ura: &str,
    ability: &str,
    args: &[u8],
) -> SignedCallEnvelope {
    let signing_key = signing_key_for(caller_ura);
    let mut envelope = ProtoEnvelope::targeted(caller_ura, callee_ura, subject_ura)
        .expect("valid descriptor-bound test envelope")
        .into_inner();
    let nonce: [u8; 16] = envelope
        .invocation_nonce
        .as_slice()
        .try_into()
        .expect("ProtoEnvelope emits a 16-byte nonce");

    let subject = AxiomSubjectIdentity::new(subject_ura, UraProfile::EasynetStrictV2);
    let ability_ref = format!(
        "{}@{}",
        easynet_cli::ura::owner_ability_ura(callee_ura, ability)
            .expect("callee-owned descriptor ability"),
        DEFAULT_ABILITY_DESCRIPTOR_VERSION
    );
    let axiom_env = InvocationEnvelope {
        caller: AxiomAgentIdentity::new(caller_ura, UraProfile::EasynetStrictV2),
        callee: AxiomAgentIdentity::new(callee_ura, UraProfile::EasynetStrictV2),
        subject: subject.clone(),
        ability: ability_ref.clone(),
        args_digest: sha2::Sha256::digest(args).into(),
        invocation_nonce: nonce,
        causal_context: CausalContext::None,
    };
    let descriptor_bound =
        DescriptorBoundEnvelope::new(axiom_env).expect("descriptor-bound test envelope");
    let key_id_hint = public_key_b64_for(caller_ura);
    let signature =
        sign_descriptor_bound_invocation(&signing_key, &descriptor_bound, key_id_hint.as_str());

    envelope.caller = Some(AgentIdentity {
        ura: caller_ura.to_string(),
        profile: DEFAULT_URA_PROFILE.to_string(),
    });
    envelope.callee = Some(AgentIdentity {
        ura: callee_ura.to_string(),
        profile: DEFAULT_URA_PROFILE.to_string(),
    });
    envelope.subject = Some(PbSubjectIdentity {
        ura: subject_ura.to_string(),
        profile: DEFAULT_URA_PROFILE.to_string(),
    });
    envelope.caller_signature = Some(CallerSignature {
        algorithm: signature.algorithm,
        signature: signature.signature,
        key_id_hint: signature.key_id_hint,
    });
    SignedCallEnvelope {
        envelope,
        descriptor_ref: ability_ref,
    }
}

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
    let device_a_public_key_b64 = public_key_b64_for(DEVICE_A_URI);
    let device_b_public_key_b64 = public_key_b64_for(DEVICE_B_URI);
    let trust_toml = format!(
        r#"
[[trusted_agent]]
agent_ura = "{DEVICE_A_URI}"
public_key_b64 = "{device_a_public_key_b64}"
role = "device"
added_at_unix_ms = 0

[[trusted_agent]]
agent_ura = "{DEVICE_B_URI}"
public_key_b64 = "{device_b_public_key_b64}"
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

async fn start_in_process_local_device() -> TestHub {
    use std::io::Write;

    let tempdir = tempfile::tempdir().expect("tempdir");
    let socket_path = tempdir.path().join("local-device.sock");

    let trust_path = tempdir.path().join("realm-trust.toml");
    let device_a_public_key_b64 = public_key_b64_for(DEVICE_A_URI);
    let trust_toml = format!(
        r#"
[[trusted_agent]]
agent_ura = "{DEVICE_A_URI}"
public_key_b64 = "{device_a_public_key_b64}"
role = "device"
added_at_unix_ms = 0
        "#,
    );
    let mut f = std::fs::File::create(&trust_path).expect("write trust toml");
    f.write_all(trust_toml.as_bytes()).expect("write");
    drop(f);

    let trust_anchor = RealmTrustAnchor::try_load_strict(&trust_path).expect("load trust anchor");
    let presence = Arc::new(PresenceRegistry::new());
    let admission = AdmissionFacade::new(Arc::new(trust_anchor), Some(DEVICE_A_URI.to_string()));
    let runtime = LocalRuntime::new();
    let authority_context =
        easynet_cli::runtime::ability_dispatch::AbilityAuthorityContext::for_device_authority_root(
            DEVICE_A_URI,
        )
        .expect("test device URI is a valid device authority root");
    let mut catalog = easynet_cli::runtime::ability_dispatch::AxonAbilityCatalog::new_with_runtime_and_authority_context(
        Arc::clone(&runtime),
        authority_context,
    );
    easynet_cli::runtime::agents::file_transfer_ability::register(&mut catalog);
    let service =
        DaemonInvocationService::new(Arc::clone(&presence), admission).with_local_runtime(runtime);

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
    let signed = signed_envelope(
        caller_ura,
        caller_ura,
        caller_ura,
        "federation.subscribe_directory",
        b"",
    );
    Request::new(InvokeServerStreamRequest {
        metadata: signed.metadata(),
        envelope: Some(signed.envelope),
        function_name: "federation.subscribe_directory".to_string(),
        ..InvokeServerStreamRequest::default()
    })
}

fn unary_invoke(
    caller_ura: &str,
    callee_ura: &str,
    function_name: &str,
    args: serde_json::Value,
) -> Request<InvokeRequest> {
    let arguments = args.to_string().into_bytes();
    let signed = signed_envelope(
        caller_ura,
        callee_ura,
        callee_ura,
        function_name,
        &arguments,
    );
    Request::new(InvokeRequest {
        metadata: signed.metadata(),
        envelope: Some(signed.envelope),
        function_name: function_name.to_string(),
        arguments,
        ..InvokeRequest::default()
    })
}

/// Publish device B's echo ability through the same
/// `federation.advertise_abilities` wire path a real device uses.
/// The cross-device invoke path is intentionally strict after
/// RFC-005: presence alone proves owner liveness, not callable
/// ability publication.
async fn publish_device_b_echo_projection(socket_path: &std::path::Path) {
    let mut client = InvocationClient::new(connect_to_hub(socket_path).await);
    let request = unary_invoke(
        DEVICE_B_URI,
        DEVICE_B_URI,
        ADVERTISE_ABILITIES,
        json!({
            "owner_ura": DEVICE_B_URI,
            "host_device_ura": DEVICE_B_URI,
            "projection_revision": 1,
            "projection_digest": "sha256:test-device-b-echo",
            "lease_expires_unix_ms": 4_102_444_800_000_i64,
            "ability_summaries": [{
                "ability_ura": DEVICE_B_ECHO_ABILITY_URA,
                "owner_ura": DEVICE_B_URI,
                "namespace": "test",
                "local_name": "echo",
                "descriptor_revision": "sha256:descriptor-device-b-echo",
                "policy_ref": "visibility:PUBLIC",
                "route_summary_ref": format!("route-ref::{DEVICE_B_ECHO_ABILITY_URA}"),
                "tags": ["class:unary"],
                "callable_summary": {
                    "public_name": DEVICE_B_ECHO_PUBLIC_NAME,
                    "description": "echo back the request payload",
                    "ability_class": "unary",
                    "input_fields": [],
                    "flags": {
                        "read_only": true,
                        "destructive": false,
                        "idempotent": true,
                        "streaming_only": false,
                        "bidi_only": false
                    }
                }
            }]
        }),
    );
    let resp = tokio::time::timeout(STEP_TIMEOUT, client.invoke(request))
        .await
        .expect("advertise_abilities did not time out")
        .expect("advertise_abilities returns Ok")
        .into_inner();
    let body: serde_json::Value =
        serde_json::from_slice(&resp.result).expect("advertise response is JSON");
    assert_eq!(
        body["ack"], true,
        "device B projection publication must be acked"
    );
    assert_eq!(body["count"], 1, "exactly one ability published");
}

fn build_test_echo_runtime() -> Arc<easynet_axon::invocation::LocalRuntime> {
    let runtime = LocalRuntime::new();
    futures::executor::block_on(runtime.register_ability_with_options(
        DEVICE_B_ECHO_ABILITY_URA,
        make_ability(|ctx| async move { Ok(ctx.payload.clone()) }),
        descriptor_proof_options(AbilityOptions::default()),
    ))
    .expect("register canonical device-owned echo Ability URA in LocalRuntime");
    runtime
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

    async fn close_gracefully(mut self) {
        self.up_tx.take();
        if let Some(handle) = self.drain.take() {
            match tokio::time::timeout(STEP_TIMEOUT, handle).await {
                Ok(Ok(())) => {}
                Ok(Err(err)) if err.is_cancelled() => {}
                Ok(Err(err)) => panic!("device session drain task failed: {err}"),
                Err(_) => panic!("device session drain task did not finish after graceful close"),
            }
        }
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
    let signed = signed_envelope(
        caller_ura,
        caller_ura,
        caller_ura,
        ABILITY_SELF_SESSION,
        b"",
    );
    let envelope_open = InvokeBidiUp {
        sequence: 0,
        payload: Some(UpPayload::EnvelopeOpen(EnvelopeOpen {
            metadata: signed.metadata(),
            envelope: Some(signed.envelope),
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

    let signed = signed_envelope(
        caller_ura,
        caller_ura,
        caller_ura,
        ABILITY_SELF_SESSION,
        b"",
    );
    let envelope_open = InvokeBidiUp {
        sequence: 0,
        payload: Some(UpPayload::EnvelopeOpen(EnvelopeOpen {
            metadata: signed.metadata(),
            envelope: Some(signed.envelope),
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
async fn cross_device_invoke_remote_round_trip_via_local_session_dispatcher() {
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

        device.close_gracefully().await;

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

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn local_file_transfer_bidi_download_reaches_business_terminal_over_tonic() {
    tokio::time::timeout(Duration::from_secs(60), async {
        let hub = start_in_process_local_device().await;
        let channel = connect_to_hub(hub.socket_path()).await;
        let mut client = InvocationClient::new(channel);

        let path = std::env::temp_dir().join(format!(
            "easynet-tonic-ft-download-{}-{}.bin",
            std::process::id(),
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .map(|d| d.subsec_nanos())
                .unwrap_or(0)
        ));
        let bytes = b"tonic-local-file-transfer-download";
        std::fs::write(&path, bytes).unwrap();

        let args = serde_json::to_vec(&json!({
            "mode": "download",
            "resource_ref": easynet_cli::runtime::agents::fs_ability::resource_ref_for_local_path(
                &path,
                easynet_cli::runtime::agents::fs_ability::FilesystemResourceCapability::Read,
            )
            .expect("local fs ResourceRef"),
        }))
        .unwrap();
        let (up_tx, up_rx) = mpsc::channel::<InvokeBidiUp>(8);
        let signed = signed_envelope(
            DEVICE_A_URI,
            DEVICE_A_URI,
            DEVICE_A_URI,
            easynet_cli::runtime::agents::file_transfer_ability::ABILITY_FILE_TRANSFER,
            &args,
        );
        up_tx
            .send(InvokeBidiUp {
                sequence: 0,
                payload: Some(UpPayload::EnvelopeOpen(EnvelopeOpen {
                    metadata: signed.metadata(),
                    envelope: Some(signed.envelope),
                    target: Some(InvocationTarget {
                        ability_name:
                            easynet_cli::runtime::agents::file_transfer_ability::ABILITY_FILE_TRANSFER
                                .to_string(),
                        ..InvocationTarget::default()
                    }),
                    initial_args: args,
                    args_content_type: "application/json".to_string(),
                    streams: vec![StreamDescriptor {
                        stream_id: 1,
                        content_type: "application/octet-stream".to_string(),
                        ordering: "STRICT".to_string(),
                        ..StreamDescriptor::default()
                    }],
                    ..EnvelopeOpen::default()
                })),
                ..InvokeBidiUp::default()
            })
            .await
            .expect("send frame 0");

        let mut down = client
            .invoke_bidi(Request::new(ReceiverStream::new(up_rx)))
            .await
            .expect("invoke_bidi opens")
            .into_inner();

        up_tx
            .send(InvokeBidiUp {
                sequence: 1,
                payload: Some(UpPayload::Control(BidiControl {
                    control: Some(bidi_control::Control::Eof(true)),
                })),
                ..InvokeBidiUp::default()
            })
            .await
            .expect("send ready/eof");

        let mut downloaded = Vec::new();
        let mut got_complete = false;
        let deadline = tokio::time::Instant::now() + STEP_TIMEOUT;
        while tokio::time::Instant::now() < deadline {
            let remaining = deadline
                .checked_duration_since(tokio::time::Instant::now())
                .unwrap_or_default();
            let Some(frame) = tokio::time::timeout(remaining, down.next())
                .await
                .expect("down frame within bound")
            else {
                break;
            };
            let frame = frame.expect("down frame ok");
            match frame.payload {
                Some(DownPayload::BinaryChunk(chunk)) => downloaded.extend(chunk.data),
                Some(DownPayload::Receipt(receipt))
                    if receipt.state
                        == easynet_axon::invocation::InvocationState::Completed.to_wire_i32() =>
                {
                    assert!(
                        receipt.cleanup_complete,
                        "completed bidi receipt must mark cleanup_complete"
                    );
                    got_complete = true;
                    break;
                }
                Some(DownPayload::Receipt(_)) => {}
                other => panic!("unexpected down payload {other:?}"),
            }
        }

        assert!(
            got_complete,
            "local file_transfer download must reach completed receipt"
        );
        assert_eq!(downloaded, bytes);
        let _ = std::fs::remove_file(&path);
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

    publish_device_b_echo_projection(socket_path).await;

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
            failure: None,
            request_id: None,
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
    // ability_ura=device-B test.echo, args=...). Open a per-call bidi.
    let channel_caller = connect_to_hub(socket_path).await;
    let mut caller_client = InvocationClient::new(channel_caller);

    let invoke_remote_request = InvokeRemoteUp::Request {
        subject_device: DEVICE_B_URI.to_string(),
        subject_ura: DEVICE_B_URI.to_string(),
        ability_ura: DEVICE_B_ECHO_ABILITY_URA.to_string(),
        args: b"args-from-A".to_vec(),
        args_content_envelope: SessionContentEnvelope::plaintext_json(),
        metadata: Default::default(),
        origin_caller: None,
    };
    let initial_args = serde_json::to_vec(&invoke_remote_request).expect("encode request");

    let signed = signed_envelope(
        DEVICE_A_URI,
        DEVICE_A_URI,
        DEVICE_A_URI,
        ABILITY_INVOKE_REMOTE,
        &initial_args,
    );
    let invoke_remote_open = InvokeBidiUp {
        sequence: 0,
        payload: Some(UpPayload::EnvelopeOpen(EnvelopeOpen {
            metadata: signed.metadata(),
            envelope: Some(signed.envelope),
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

    use easynet_cli::services::invocation_transport::invoke_remote_initiator::InvokeRemoteDown;
    let down: InvokeRemoteDown =
        serde_json::from_slice(&chunk.data).expect("decode InvokeRemoteDown");

    let InvokeRemoteDown::Result { payload, error, .. } = down else {
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
        ability, DEVICE_B_ECHO_PUBLIC_NAME,
        "v0 SessionDispatch delivers the owner-local public ability name; LocalAxonSessionDispatcher separately proves canonical runtime dispatch"
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
    publish_device_b_echo_projection(socket_path).await;
    tokio::time::sleep(Duration::from_millis(50)).await;

    let device_b_up_for_reply = SessionUpSender::new(device_b.up().clone());
    let local_dispatcher =
        LocalAxonSessionDispatcher::new().with_local_runtime(build_test_echo_runtime());
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
        subject_ura: DEVICE_B_URI.to_string(),
        ability_ura: DEVICE_B_ECHO_ABILITY_URA.to_string(),
        args: br#"{"echo":"args-from-A"}"#.to_vec(),
        args_content_envelope: SessionContentEnvelope::plaintext_json(),
        metadata: Default::default(),
        origin_caller: None,
    };
    let initial_args = serde_json::to_vec(&invoke_remote_request).expect("encode request");

    let signed = signed_envelope(
        DEVICE_A_URI,
        DEVICE_A_URI,
        DEVICE_A_URI,
        ABILITY_INVOKE_REMOTE,
        &initial_args,
    );
    let invoke_remote_open = InvokeBidiUp {
        sequence: 0,
        payload: Some(UpPayload::EnvelopeOpen(EnvelopeOpen {
            metadata: signed.metadata(),
            envelope: Some(signed.envelope),
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

    use easynet_cli::services::invocation_transport::invoke_remote_initiator::InvokeRemoteDown;
    let down: InvokeRemoteDown =
        serde_json::from_slice(&chunk.data).expect("decode InvokeRemoteDown");

    let InvokeRemoteDown::Result { payload, error, .. } = down else {
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
