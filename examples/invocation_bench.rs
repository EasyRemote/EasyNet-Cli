//! Invocation-plane benchmark harness
//! ===================================
//!
//! Measures, against a real in-process tonic hub (production
//! `DaemonInvocationService` + `PresenceRegistry` + pending maps,
//! same wiring as `boot.rs`):
//!
//!   latency  — serial `runtime.invoke_remote` echo RTT distribution
//!   sweep    — concurrency sweep: N parallel invoke_remote calls,
//!              N in {1,2,4,...,512}; p50/p95/p99, throughput, errors
//!   hol      — head-of-line blocking proof: one slow streaming
//!              consumer jams the per-session drain; echo latency on
//!              the SAME device session stalls while a SECOND device
//!              stays healthy (blast-radius control)
//!   all      — run everything
//!
//! Run: cargo run --release --example invocation_bench -- all
//!
//! Diagnostic companion to the §A2/§A3 debt entries in
//! to-be-fix.spec.md; reuses the harness patterns of
//! tests/cross_device_invoke_remote_e2e.rs.
//!
//! Author: Silan Hu <silan.hu@u.nus.edu>
//! Copyright (c) 2026 EasyNet. All rights reserved.

use std::sync::Arc;
use std::time::{Duration, Instant};

use easynet_axon::invocation::{make_ability, LocalRuntime};
use easynet_axon::pb::axon::v1::invocation_client::InvocationClient;
use easynet_axon::pb::axon::v1::invocation_server::InvocationServer;
use easynet_axon::pb::axon::v1::invoke_bidi_down::Payload as DownPayload;
use easynet_axon::pb::axon::v1::invoke_bidi_up::Payload as UpPayload;
use easynet_axon::pb::axon::v1::{
    AgentIdentity, BinaryChunk, Envelope, EnvelopeOpen, InvocationTarget, InvokeBidiDown,
    InvokeBidiUp, InvokeRequest, StreamDescriptor,
};
use easynet_cli::daemon::invocation::admission::admission_facade::AdmissionFacade;
use easynet_cli::daemon::invocation::bidi::invoke_remote_initiator::{
    InvokeRemoteDown, InvokeRemoteUp, SessionContentEnvelope, SessionDispatch,
    ABILITY_INVOKE_REMOTE, INVOKE_REMOTE_STREAM_ID,
};
use easynet_cli::daemon::invocation::bidi::session_initiator::{
    SessionFrameDispatcher, SessionUpSender, ABILITY_SESSION_OPEN, SESSION_STREAM_ID,
};
use easynet_cli::daemon::invocation::bidi::state::pending_dispatch::{
    PendingDispatchMap, PendingStreamDispatchMap,
};
use easynet_cli::daemon::invocation::bidi::state::presence::PresenceRegistry;
use easynet_cli::daemon::invocation::dispatch::daemon_invocation_service::DaemonInvocationService;
use easynet_cli::daemon::invocation::dispatch::local_session_dispatcher::LocalAxonSessionDispatcher;
use easynet_cli::daemon::trust::anchor::RealmTrustAnchor;
use futures::StreamExt;
use rcgen::{generate_simple_self_signed, CertifiedKey};
use serde_json::json;
use tokio::net::UnixListener;
use tokio::sync::{mpsc, oneshot};
use tokio::task::JoinHandle;
use tokio_stream::wrappers::{ReceiverStream, TcpListenerStream, UnixListenerStream};
use tonic::transport::{
    Certificate, Channel, ClientTlsConfig, Endpoint, Identity, Server, ServerTlsConfig, Uri,
};
use tonic::Request;

const DEVICE_A_URI: &str = "easynet:///r/bench-realm/device/device-a";
const DEVICE_B_URI: &str = "easynet:///r/bench-realm/device/device-b";
const DEVICE_C_URI: &str = "easynet:///r/bench-realm/device/device-c";

const ECHO_B_URA: &str = "easynet:///r/bench-realm/ability/device.device-b.test.echo";
const FLOOD_B_URA: &str = "easynet:///r/bench-realm/ability/device.device-b.test.flood";
const SLOW_B_URA: &str = "easynet:///r/bench-realm/ability/device.device-b.test.slow";
const ECHO_C_URA: &str = "easynet:///r/bench-realm/ability/device.device-c.test.echo";

/// test.slow sleeps this long before echoing — the stand-in for a
/// real slow ability (chat hitting an LLM, shell.run, etc).
const SLOW_ABILITY_MS: u64 = 2_000;

/// Flood: non-terminal streaming chunks device B emits for one
/// jammed call. Must exceed every buffer between the device and the
/// non-reading caller: hub pending-stream rx (32) + response channel
/// (16) + h2 stream window (up to ~1MB ≈ 17 frames of this size).
/// 512 x 16KB raw (~60KB each on the JSON number-array wire) jams
/// with a wide margin.
const FLOOD_FRAMES: usize = 512;
const FLOOD_CHUNK_BYTES: usize = 16 * 1024;

const SWEEP_PAYLOAD: &[u8] = br#"{"echo":"bench-payload-0123456789"}"#;

struct Hub {
    socket_path: std::path::PathBuf,
    _tempdir: tempfile::TempDir,
    shutdown: Option<oneshot::Sender<()>>,
    server: Option<JoinHandle<()>>,
}

impl Drop for Hub {
    fn drop(&mut self) {
        if let Some(tx) = self.shutdown.take() {
            let _ = tx.send(());
        }
        if let Some(handle) = self.server.take() {
            handle.abort();
        }
    }
}

async fn start_hub() -> Hub {
    use std::io::Write;

    let tempdir = tempfile::tempdir().expect("tempdir");
    let socket_path = tempdir.path().join("hub.sock");

    let trust_path = tempdir.path().join("realm-trust.toml");
    let mut trust_toml = String::new();
    for ura in [DEVICE_A_URI, DEVICE_B_URI, DEVICE_C_URI] {
        trust_toml.push_str(&format!(
            r#"
[[trusted_agent]]
agent_ura = "{ura}"
public_key_b64 = "AAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAA="
role = "device"
added_at_unix_ms = 0
"#
        ));
    }
    let mut f = std::fs::File::create(&trust_path).expect("write trust toml");
    f.write_all(trust_toml.as_bytes()).expect("write");
    drop(f);

    let trust_anchor = RealmTrustAnchor::try_load_strict(&trust_path).expect("load trust anchor");
    let presence = Arc::new(PresenceRegistry::new());
    let pending = Arc::new(PendingDispatchMap::new());
    let pending_stream = Arc::new(PendingStreamDispatchMap::new());
    let admission = AdmissionFacade::new(Arc::new(trust_anchor), None);
    let service = DaemonInvocationService::new(Arc::clone(&presence), admission)
        .with_pending(Arc::clone(&pending))
        .with_pending_stream(Arc::clone(&pending_stream));

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

    Hub {
        socket_path,
        _tempdir: tempdir,
        shutdown: Some(shutdown_tx),
        server: Some(server),
    }
}

async fn connect(socket_path: &std::path::Path) -> Channel {
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

fn unary_invoke(
    caller_ura: &str,
    callee_ura: &str,
    function_name: &str,
    args: serde_json::Value,
) -> Request<InvokeRequest> {
    Request::new(InvokeRequest {
        envelope: Some(Envelope {
            caller: Some(AgentIdentity {
                ura: caller_ura.to_string(),
                ..AgentIdentity::default()
            }),
            callee: Some(AgentIdentity {
                ura: callee_ura.to_string(),
                ..AgentIdentity::default()
            }),
            invocation_nonce: vec![0x22; 16],
            ..Envelope::default()
        }),
        function_name: function_name.to_string(),
        arguments: args.to_string().into_bytes(),
        ..InvokeRequest::default()
    })
}

fn ability_summary(ability_ura: &str, owner: &str, local: &str, public: &str) -> serde_json::Value {
    json!({
        "ability_ura": ability_ura,
        "owner_ura": owner,
        "namespace": "test",
        "local_name": local,
        "descriptor_revision": format!("sha256:descriptor-{local}"),
        "policy_ref": "visibility:PUBLIC",
        "route_summary_ref": format!("route-ref::{ability_ura}"),
        "tags": ["class:unary"],
        "callable_summary": {
            "public_name": public,
            "description": "bench ability",
            "ability_class": "unary",
            "input_fields": [],
            "flags": {
                "read_only": true, "destructive": false, "idempotent": true,
                "streaming_only": false, "bidi_only": false
            }
        }
    })
}

async fn publish_projection(
    socket_path: &std::path::Path,
    device_ura: &str,
    summaries: Vec<serde_json::Value>,
) {
    publish_projection_on(connect(socket_path).await, device_ura, summaries).await;
}

async fn publish_projection_on(
    channel: Channel,
    device_ura: &str,
    summaries: Vec<serde_json::Value>,
) {
    let count = summaries.len();
    let mut client = InvocationClient::new(channel);
    let request = unary_invoke(
        device_ura,
        device_ura,
        "federation.advertise_abilities",
        json!({
            "owner_ura": device_ura,
            "host_device_ura": device_ura,
            "projection_revision": 1,
            "projection_digest": format!("sha256:bench-{device_ura}"),
            "lease_expires_unix_ms": 4_102_444_800_000_i64,
            "ability_summaries": summaries,
        }),
    );
    let resp = tokio::time::timeout(Duration::from_secs(5), client.invoke(request))
        .await
        .expect("advertise within bound")
        .expect("advertise ok")
        .into_inner();
    let body: serde_json::Value = serde_json::from_slice(&resp.result).expect("json");
    assert_eq!(body["ack"], true, "projection publication acked");
    assert_eq!(body["count"], count, "all abilities published");
}

/// Echo responder device: holds a `session.open` open and answers
/// every Dispatch frame concurrently (spawn per frame, mirroring a
/// well-behaved device). `test.flood` dispatches instead emit
/// FLOOD_FRAMES non-terminal chunks and never terminate — the
/// misbehaving-streaming-call generator for the hol scenario.
struct Responder {
    up_tx: Option<mpsc::Sender<InvokeBidiUp>>,
    tasks: Vec<JoinHandle<()>>,
}

impl Drop for Responder {
    fn drop(&mut self) {
        self.up_tx.take();
        for t in self.tasks.drain(..) {
            t.abort();
        }
    }
}

async fn start_echo_responder(channel: Channel, device_ura: &str) -> Responder {
    let mut client = InvocationClient::new(channel);

    let envelope_open = InvokeBidiUp {
        sequence: 0,
        payload: Some(UpPayload::EnvelopeOpen(EnvelopeOpen {
            envelope: Some(Envelope {
                caller: Some(AgentIdentity {
                    ura: device_ura.to_string(),
                    ..AgentIdentity::default()
                }),
                ..Envelope::default()
            }),
            target: Some(InvocationTarget {
                ability_name: ABILITY_SESSION_OPEN.to_string(),
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

    let (up_tx, up_rx) = mpsc::channel::<InvokeBidiUp>(1024);
    up_tx.send(envelope_open).await.expect("send frame 0");
    let outbound = ReceiverStream::new(up_rx);

    let runtime = LocalRuntime::new();
    runtime
        .register_ability(
            "test.echo",
            make_ability(|ctx| async move { Ok(ctx.payload.clone()) }),
        )
        .await
        .expect("register echo");
    runtime
        .register_ability(
            "test.slow",
            make_ability(|ctx| async move {
                tokio::time::sleep(Duration::from_millis(SLOW_ABILITY_MS)).await;
                Ok(ctx.payload.clone())
            }),
        )
        .await
        .expect("register slow");
    let dispatcher = Arc::new(LocalAxonSessionDispatcher::new().with_local_runtime(runtime));

    // ONE SessionUpSender per session, shared by every reply
    // producer — clones share the sequence counter (the hub enforces
    // a strictly monotonic up-sequence and resets the session on
    // violation, so per-frame fresh senders are lethal).
    let up_sender = SessionUpSender::new(up_tx.clone());
    let drain = tokio::spawn(async move {
        let response = client.invoke_bidi(Request::new(outbound)).await;
        let Ok(response) = response else { return };
        let mut down = response.into_inner();
        while let Some(Ok(frame)) = down.next().await {
            // Peek: flood dispatches get the misbehaving-stream
            // treatment; everything else goes through the production
            // device-side dispatcher INLINE — the same serial pattern
            // as session_initiator/frame_loop.rs:116.
            let flood_call_id = match &frame.payload {
                Some(DownPayload::BinaryChunk(chunk)) => {
                    match serde_json::from_slice::<SessionDispatch>(&chunk.data) {
                        Ok(SessionDispatch::Dispatch {
                            call_id, ability, ..
                        }) if ability == "test.flood" => Some(call_id),
                        _ => None,
                    }
                }
                _ => None,
            };
            if let Some(call_id) = flood_call_id {
                let up = up_sender.clone();
                tokio::spawn(async move {
                    for _ in 0..FLOOD_FRAMES {
                        let result = SessionDispatch::Result {
                            call_id,
                            payload: vec![0x5a; FLOOD_CHUNK_BYTES],
                            terminal: false,
                            error: None,
                            failure: None,
                            request_id: None,
                        };
                        let bytes = serde_json::to_vec(&result).expect("encode flood chunk");
                        if up
                            .send_binary_chunk(BinaryChunk {
                                data: bytes,
                                ..BinaryChunk::default()
                            })
                            .await
                            .is_err()
                        {
                            break;
                        }
                    }
                });
                continue;
            }
            let _ = dispatcher.handle_down(frame, &up_sender).await;
        }
    });

    Responder {
        up_tx: Some(up_tx),
        tasks: vec![drain],
    }
}

/// One `runtime.invoke_remote` echo round trip. Returns latency.
async fn invoke_remote_once(
    channel: Channel,
    ability_ura: &str,
    args: Vec<u8>,
    timeout: Duration,
) -> Result<Duration, String> {
    let started = Instant::now();
    let mut client = InvocationClient::new(channel);

    let request = InvokeRemoteUp::Request {
        subject_device: DEVICE_B_URI.to_string(),
        subject_ura: String::new(),
        ability_ura: ability_ura.to_string(),
        args,
        args_content_envelope: SessionContentEnvelope::plaintext_json(),
        metadata: Default::default(),
        origin_caller: None,
    };
    let initial_args = serde_json::to_vec(&request).map_err(|e| e.to_string())?;

    let open = InvokeBidiUp {
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

    let (up_tx, up_rx) = mpsc::channel::<InvokeBidiUp>(2);
    up_tx.send(open).await.map_err(|e| e.to_string())?;

    let fut = async {
        let response = client
            .invoke_bidi(Request::new(ReceiverStream::new(up_rx)))
            .await
            .map_err(|s| format!("rpc: {s}"))?;
        let mut down = response.into_inner();
        loop {
            let frame = down
                .next()
                .await
                .ok_or_else(|| "stream ended without result".to_string())?
                .map_err(|s| format!("frame: {s}"))?;
            let Some(DownPayload::BinaryChunk(chunk)) = frame.payload else {
                continue;
            };
            let parsed: InvokeRemoteDown =
                serde_json::from_slice(&chunk.data).map_err(|e| e.to_string())?;
            let InvokeRemoteDown::Result { error, .. } = parsed else {
                continue;
            };
            return match error {
                None => Ok(started.elapsed()),
                Some(err) => Err(format!("inband: {err}")),
            };
        }
    };
    match tokio::time::timeout(timeout, fut).await {
        Ok(result) => result,
        Err(_) => Err(format!("timeout after {timeout:?}")),
    }
}

/// Open a flood call and deliberately never poll the down stream.
/// Returned guards must stay alive to keep the call (and the jam)
/// open; dropping them releases the stream and unjams the drain.
async fn open_unread_flood_call(
    channel: Channel,
) -> (mpsc::Sender<InvokeBidiUp>, tonic::Streaming<InvokeBidiDown>) {
    let mut client = InvocationClient::new(channel);
    let request = InvokeRemoteUp::Request {
        subject_device: DEVICE_B_URI.to_string(),
        subject_ura: String::new(),
        ability_ura: FLOOD_B_URA.to_string(),
        args: b"{}".to_vec(),
        args_content_envelope: SessionContentEnvelope::plaintext_json(),
        metadata: Default::default(),
        origin_caller: None,
    };
    let initial_args = serde_json::to_vec(&request).expect("encode flood request");
    let open = InvokeBidiUp {
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
    let (up_tx, up_rx) = mpsc::channel::<InvokeBidiUp>(2);
    up_tx.send(open).await.expect("send flood open");
    let down = client
        .invoke_bidi(Request::new(ReceiverStream::new(up_rx)))
        .await
        .expect("flood bidi opens")
        .into_inner();
    (up_tx, down)
}

fn fmt_dur(d: Duration) -> String {
    if d >= Duration::from_secs(1) {
        format!("{:.2}s", d.as_secs_f64())
    } else {
        format!("{:.2}ms", d.as_secs_f64() * 1000.0)
    }
}

fn percentile(sorted: &[Duration], p: f64) -> Duration {
    if sorted.is_empty() {
        return Duration::ZERO;
    }
    let idx = ((sorted.len() as f64 - 1.0) * p / 100.0).round() as usize;
    sorted[idx.min(sorted.len() - 1)]
}

struct BenchEnv {
    hub: Hub,
    channels: Vec<Channel>,
    responder_b: Option<Responder>,
    _responder_c: Responder,
}

impl BenchEnv {
    fn channel(&self, i: usize) -> Channel {
        self.channels[i % self.channels.len()].clone()
    }

    async fn respawn_device_b(&mut self) {
        self.responder_b.take();
        tokio::time::sleep(Duration::from_millis(100)).await;
        let responder =
            start_echo_responder(connect(&self.hub.socket_path).await, DEVICE_B_URI).await;
        self.responder_b = Some(responder);
        publish_projection(
            &self.hub.socket_path,
            DEVICE_B_URI,
            vec![
                ability_summary(ECHO_B_URA, DEVICE_B_URI, "echo", "test.echo"),
                ability_summary(FLOOD_B_URA, DEVICE_B_URI, "flood", "test.flood"),
                ability_summary(SLOW_B_URA, DEVICE_B_URI, "slow", "test.slow"),
            ],
        )
        .await;
        tokio::time::sleep(Duration::from_millis(100)).await;
    }
}

async fn setup() -> BenchEnv {
    let hub = start_hub().await;

    let responder_b = start_echo_responder(connect(&hub.socket_path).await, DEVICE_B_URI).await;
    let responder_c = start_echo_responder(connect(&hub.socket_path).await, DEVICE_C_URI).await;
    publish_projection(
        &hub.socket_path,
        DEVICE_B_URI,
        vec![
            ability_summary(ECHO_B_URA, DEVICE_B_URI, "echo", "test.echo"),
            ability_summary(FLOOD_B_URA, DEVICE_B_URI, "flood", "test.flood"),
            ability_summary(SLOW_B_URA, DEVICE_B_URI, "slow", "test.slow"),
        ],
    )
    .await;
    publish_projection(
        &hub.socket_path,
        DEVICE_C_URI,
        vec![ability_summary(
            ECHO_C_URA,
            DEVICE_C_URI,
            "echo",
            "test.echo",
        )],
    )
    .await;
    tokio::time::sleep(Duration::from_millis(100)).await;

    let mut channels = Vec::new();
    for _ in 0..16 {
        channels.push(connect(&hub.socket_path).await);
    }

    BenchEnv {
        hub,
        channels,
        responder_b: Some(responder_b),
        _responder_c: responder_c,
    }
}

/// Echo to device C: invoke_remote_once targets DEVICE_B in its
/// Request shape, so the C-targeted control call builds its own.
async fn invoke_remote_echo_c(channel: Channel, timeout: Duration) -> Result<Duration, String> {
    let started = Instant::now();
    let mut client = InvocationClient::new(channel);
    let request = InvokeRemoteUp::Request {
        subject_device: DEVICE_C_URI.to_string(),
        subject_ura: String::new(),
        ability_ura: ECHO_C_URA.to_string(),
        args: SWEEP_PAYLOAD.to_vec(),
        args_content_envelope: SessionContentEnvelope::plaintext_json(),
        metadata: Default::default(),
        origin_caller: None,
    };
    let initial_args = serde_json::to_vec(&request).map_err(|e| e.to_string())?;
    let open = InvokeBidiUp {
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
    let (up_tx, up_rx) = mpsc::channel::<InvokeBidiUp>(2);
    up_tx.send(open).await.map_err(|e| e.to_string())?;
    let fut = async {
        let response = client
            .invoke_bidi(Request::new(ReceiverStream::new(up_rx)))
            .await
            .map_err(|s| format!("rpc: {s}"))?;
        let mut down = response.into_inner();
        loop {
            let frame = down
                .next()
                .await
                .ok_or_else(|| "stream ended without result".to_string())?
                .map_err(|s| format!("frame: {s}"))?;
            let Some(DownPayload::BinaryChunk(chunk)) = frame.payload else {
                continue;
            };
            let parsed: InvokeRemoteDown =
                serde_json::from_slice(&chunk.data).map_err(|e| e.to_string())?;
            let InvokeRemoteDown::Result { error, .. } = parsed else {
                continue;
            };
            return match error {
                None => Ok(started.elapsed()),
                Some(err) => Err(format!("inband: {err}")),
            };
        }
    };
    match tokio::time::timeout(timeout, fut).await {
        Ok(result) => result,
        Err(_) => Err(format!("timeout after {timeout:?}")),
    }
}

async fn scenario_latency(env: &BenchEnv) {
    println!("\n=== latency: serial invoke_remote echo RTT (in-process hub, UDS) ===");
    let iters = 200;
    let mut samples = Vec::with_capacity(iters);
    // warmup
    for _ in 0..20 {
        let _ = invoke_remote_once(
            env.channel(0),
            ECHO_B_URA,
            SWEEP_PAYLOAD.to_vec(),
            Duration::from_secs(10),
        )
        .await;
    }
    for i in 0..iters {
        match invoke_remote_once(
            env.channel(i),
            ECHO_B_URA,
            SWEEP_PAYLOAD.to_vec(),
            Duration::from_secs(10),
        )
        .await
        {
            Ok(d) => samples.push(d),
            Err(e) => println!("  iter {i}: ERROR {e}"),
        }
    }
    samples.sort();
    println!(
        "  n={} p50={} p95={} p99={} max={}",
        samples.len(),
        fmt_dur(percentile(&samples, 50.0)),
        fmt_dur(percentile(&samples, 95.0)),
        fmt_dur(percentile(&samples, 99.0)),
        fmt_dur(*samples.last().unwrap_or(&Duration::ZERO)),
    );
}

async fn scenario_sweep(env: &mut BenchEnv) {
    println!("\n=== sweep: N concurrent invoke_remote echo calls (single target device) ===");
    println!(
        "{:>6} {:>10} {:>10} {:>10} {:>10} {:>12} {:>8}",
        "N", "p50", "p95", "p99", "max", "throughput", "errors"
    );
    for n in [1usize, 2, 4, 8, 16, 32, 64, 128, 256, 512, 1024, 2048, 4096] {
        let started = Instant::now();
        let mut joins = Vec::with_capacity(n);
        for i in 0..n {
            let channel = env.channel(i);
            joins.push(tokio::spawn(invoke_remote_once(
                channel,
                ECHO_B_URA,
                SWEEP_PAYLOAD.to_vec(),
                Duration::from_secs(20),
            )));
        }
        let mut samples = Vec::with_capacity(n);
        let mut errors: Vec<String> = Vec::new();
        for j in joins {
            match j.await {
                Ok(Ok(d)) => samples.push(d),
                Ok(Err(e)) => errors.push(e),
                Err(e) => errors.push(format!("join: {e}")),
            }
        }
        let wall = started.elapsed();
        samples.sort();
        let throughput = if wall.as_secs_f64() > 0.0 {
            samples.len() as f64 / wall.as_secs_f64()
        } else {
            0.0
        };
        println!(
            "{:>6} {:>10} {:>10} {:>10} {:>10} {:>9.0}/s {:>8}",
            n,
            fmt_dur(percentile(&samples, 50.0)),
            fmt_dur(percentile(&samples, 95.0)),
            fmt_dur(percentile(&samples, 99.0)),
            fmt_dur(samples.last().copied().unwrap_or(Duration::ZERO)),
            throughput,
            errors.len(),
        );
        if !errors.is_empty() {
            let mut counts: std::collections::HashMap<&str, usize> = Default::default();
            for e in &errors {
                let key = if e.contains("channel full") {
                    "channel-full/evicted"
                } else if e.contains("not online") || e.contains("unavailable") {
                    "target-offline"
                } else if e.contains("timeout") {
                    "timeout"
                } else {
                    "other"
                };
                *counts.entry(key).or_default() += 1;
            }
            for (k, v) in &counts {
                println!("         error[{k}] x{v}");
            }
            if let Some(sample) = errors.first() {
                println!("         first error: {sample}");
            }
            // Eviction kills device B's session for every later round;
            // respawn to keep measuring.
            env.respawn_device_b().await;
        }
        tokio::time::sleep(Duration::from_millis(200)).await;
    }
}

/// The user-facing symptom reproducer: ONE slow ability call on a
/// device makes every other invocation to that device queue behind
/// it, because the device frame loop executes abilities inline
/// (frame_loop.rs:116 -> local_session_dispatcher.rs:1620).
async fn scenario_slowpoke(env: &mut BenchEnv) {
    println!(
        "\n=== slowpoke: one {SLOW_ABILITY_MS}ms ability call vs everyone else on the device ==="
    );

    let mut baseline = Vec::new();
    for i in 0..10 {
        if let Ok(d) = invoke_remote_once(
            env.channel(i),
            ECHO_B_URA,
            SWEEP_PAYLOAD.to_vec(),
            Duration::from_secs(10),
        )
        .await
        {
            baseline.push(d);
        }
    }
    baseline.sort();
    println!(
        "  baseline echo->B p50: {}",
        fmt_dur(percentile(&baseline, 50.0))
    );

    println!("  firing test.slow ({SLOW_ABILITY_MS}ms) at device B...");
    let slow_channel = env.channel(9);
    let slow_call = tokio::spawn(invoke_remote_once(
        slow_channel,
        SLOW_B_URA,
        b"{}".to_vec(),
        Duration::from_secs(15),
    ));
    tokio::time::sleep(Duration::from_millis(150)).await;

    println!("  echo->B WHILE the slow call runs:");
    for i in 0..3 {
        match invoke_remote_once(
            env.channel(i),
            ECHO_B_URA,
            SWEEP_PAYLOAD.to_vec(),
            Duration::from_secs(10),
        )
        .await
        {
            Ok(d) => println!("    call {i}: {}", fmt_dur(d)),
            Err(e) => println!("    call {i}: ERROR ({e})"),
        }
    }

    println!("  control echo->C WHILE the slow call runs:");
    match invoke_remote_echo_c(env.channel(5), Duration::from_secs(10)).await {
        Ok(d) => println!("    call 0: {}", fmt_dur(d)),
        Err(e) => println!("    call 0: ERROR ({e})"),
    }

    match slow_call.await {
        Ok(Ok(d)) => println!("  slow call itself: {}", fmt_dur(d)),
        Ok(Err(e)) => println!("  slow call itself: ERROR ({e})"),
        Err(e) => println!("  slow call itself: join error ({e})"),
    }
}

async fn scenario_hol(env: &mut BenchEnv) {
    println!("\n=== hol: one unread streaming call vs everyone else on the session ===");

    let baseline_timeout = Duration::from_secs(10);
    let mut baseline = Vec::new();
    for i in 0..10 {
        if let Ok(d) = invoke_remote_once(
            env.channel(i),
            ECHO_B_URA,
            SWEEP_PAYLOAD.to_vec(),
            baseline_timeout,
        )
        .await
        {
            baseline.push(d);
        }
    }
    baseline.sort();
    println!(
        "  baseline echo->B p50: {}",
        fmt_dur(percentile(&baseline, 50.0))
    );

    println!(
        "  opening flood call ({} x {}KB chunks) whose caller never reads its stream...",
        FLOOD_FRAMES,
        FLOOD_CHUNK_BYTES / 1024
    );
    let (_flood_up, flood_down) = open_unread_flood_call(env.channel(7)).await;
    tokio::time::sleep(Duration::from_millis(1000)).await;

    println!("  echo->B WHILE jammed (timeout 8s each):");
    let jam_timeout = Duration::from_secs(8);
    for i in 0..3 {
        let started = Instant::now();
        match invoke_remote_once(
            env.channel(i + 1),
            ECHO_B_URA,
            SWEEP_PAYLOAD.to_vec(),
            jam_timeout,
        )
        .await
        {
            Ok(d) => println!("    call {i}: OK in {}", fmt_dur(d)),
            Err(e) => println!(
                "    call {i}: BLOCKED ({e}, waited {})",
                fmt_dur(started.elapsed())
            ),
        }
    }

    println!("  control echo->C (different device) WHILE B is jammed:");
    for i in 0..3 {
        match invoke_remote_echo_c(env.channel(i + 4), jam_timeout).await {
            Ok(d) => println!("    call {i}: OK in {}", fmt_dur(d)),
            Err(e) => println!("    call {i}: ERROR ({e})"),
        }
    }

    println!("  dropping the unread stream (caller goes away)...");
    drop(flood_down);
    drop(_flood_up);
    tokio::time::sleep(Duration::from_millis(1000)).await;

    println!("  echo->B AFTER release:");
    let mut recovered = Vec::new();
    for i in 0..5 {
        match invoke_remote_once(
            env.channel(i),
            ECHO_B_URA,
            SWEEP_PAYLOAD.to_vec(),
            baseline_timeout,
        )
        .await
        {
            Ok(d) => recovered.push(d),
            Err(e) => println!("    call {i}: ERROR ({e})"),
        }
    }
    recovered.sort();
    if !recovered.is_empty() {
        println!(
            "    recovered p50: {}",
            fmt_dur(percentile(&recovered, 50.0))
        );
    }
    // Leave B in a known-good state for any following scenario.
    env.respawn_device_b().await;
}

/// Same hub, same service, but served over real TCP+TLS on
/// localhost (the hub-mode transport) instead of UDS — isolates the
/// TCP+TLS stack cost. WAN deployments add physical RTT on top.
async fn scenario_tls() {
    use std::io::Write;

    println!("\n=== tls: invoke_remote over real TCP+TLS (localhost) ===");

    let CertifiedKey { cert, key_pair } =
        generate_simple_self_signed(vec!["localhost".to_string(), "127.0.0.1".to_string()])
            .expect("rcgen self-signed cert");
    let cert_pem = cert.pem();
    let key_pem = key_pair.serialize_pem();

    let tempdir = tempfile::tempdir().expect("tempdir");
    let trust_path = tempdir.path().join("realm-trust.toml");
    let mut trust_toml = String::new();
    for ura in [DEVICE_A_URI, DEVICE_B_URI, DEVICE_C_URI] {
        trust_toml.push_str(&format!(
            r#"
[[trusted_agent]]
agent_ura = "{ura}"
public_key_b64 = "AAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAA="
role = "device"
added_at_unix_ms = 0
"#
        ));
    }
    let mut f = std::fs::File::create(&trust_path).expect("write trust toml");
    f.write_all(trust_toml.as_bytes()).expect("write");
    drop(f);

    let trust_anchor = RealmTrustAnchor::try_load_strict(&trust_path).expect("load trust anchor");
    let presence = Arc::new(PresenceRegistry::new());
    let pending = Arc::new(PendingDispatchMap::new());
    let pending_stream = Arc::new(PendingStreamDispatchMap::new());
    let admission = AdmissionFacade::new(Arc::new(trust_anchor), None);
    let service = DaemonInvocationService::new(Arc::clone(&presence), admission)
        .with_pending(Arc::clone(&pending))
        .with_pending_stream(Arc::clone(&pending_stream));

    let listener = tokio::net::TcpListener::bind("127.0.0.1:0")
        .await
        .expect("bind tcp");
    let addr = listener.local_addr().expect("local addr");
    let incoming = TcpListenerStream::new(listener);
    let identity = Identity::from_pem(cert_pem.as_bytes(), key_pem.as_bytes());
    let (shutdown_tx, shutdown_rx) = oneshot::channel::<()>();
    let server = tokio::spawn(async move {
        let _ = Server::builder()
            .tls_config(ServerTlsConfig::new().identity(identity))
            .expect("tls config")
            .add_service(InvocationServer::new(service))
            .serve_with_incoming_shutdown(incoming, async {
                let _ = shutdown_rx.await;
            })
            .await;
    });
    tokio::time::sleep(Duration::from_millis(50)).await;

    let ca = Certificate::from_pem(cert_pem.as_bytes());
    let tls = ClientTlsConfig::new()
        .ca_certificate(ca)
        .domain_name("localhost");
    let endpoint = Endpoint::from_shared(format!("https://{addr}"))
        .expect("endpoint")
        .tls_config(tls)
        .expect("client tls");

    // Channel establishment = TCP connect + TLS handshake.
    let mut handshakes = Vec::new();
    for _ in 0..10 {
        let started = Instant::now();
        let _ch = endpoint.connect().await.expect("tls connect");
        handshakes.push(started.elapsed());
    }
    handshakes.sort();
    println!(
        "  channel setup (TCP+TLS handshake) p50: {}",
        fmt_dur(percentile(&handshakes, 50.0))
    );

    let responder_channel = endpoint.connect().await.expect("responder channel");
    let _responder_b = start_echo_responder(responder_channel, DEVICE_B_URI).await;
    publish_projection_on(
        endpoint.connect().await.expect("publish channel"),
        DEVICE_B_URI,
        vec![
            ability_summary(ECHO_B_URA, DEVICE_B_URI, "echo", "test.echo"),
            ability_summary(FLOOD_B_URA, DEVICE_B_URI, "flood", "test.flood"),
            ability_summary(SLOW_B_URA, DEVICE_B_URI, "slow", "test.slow"),
        ],
    )
    .await;
    tokio::time::sleep(Duration::from_millis(100)).await;

    let mut channels = Vec::new();
    for _ in 0..16 {
        channels.push(endpoint.connect().await.expect("pool channel"));
    }

    // Serial RTT.
    let mut samples = Vec::new();
    for _ in 0..20 {
        let _ = invoke_remote_once(
            channels[0].clone(),
            ECHO_B_URA,
            SWEEP_PAYLOAD.to_vec(),
            Duration::from_secs(10),
        )
        .await;
    }
    for i in 0..200 {
        if let Ok(d) = invoke_remote_once(
            channels[i % channels.len()].clone(),
            ECHO_B_URA,
            SWEEP_PAYLOAD.to_vec(),
            Duration::from_secs(10),
        )
        .await
        {
            samples.push(d);
        }
    }
    samples.sort();
    println!(
        "  serial RTT n={} p50={} p95={} p99={}",
        samples.len(),
        fmt_dur(percentile(&samples, 50.0)),
        fmt_dur(percentile(&samples, 95.0)),
        fmt_dur(percentile(&samples, 99.0)),
    );

    // Small concurrency probe.
    for n in [32usize, 256] {
        let started = Instant::now();
        let mut joins = Vec::with_capacity(n);
        for i in 0..n {
            joins.push(tokio::spawn(invoke_remote_once(
                channels[i % channels.len()].clone(),
                ECHO_B_URA,
                SWEEP_PAYLOAD.to_vec(),
                Duration::from_secs(20),
            )));
        }
        let mut ok = Vec::new();
        let mut errors = 0usize;
        for j in joins {
            match j.await {
                Ok(Ok(d)) => ok.push(d),
                _ => errors += 1,
            }
        }
        let wall = started.elapsed();
        ok.sort();
        println!(
            "  N={n}: p50={} p99={} throughput={:.0}/s errors={errors}",
            fmt_dur(percentile(&ok, 50.0)),
            fmt_dur(percentile(&ok, 99.0)),
            ok.len() as f64 / wall.as_secs_f64(),
        );
    }

    drop(shutdown_tx);
    server.abort();
}

#[tokio::main(flavor = "multi_thread")]
async fn main() {
    let scenario = std::env::args().nth(1).unwrap_or_else(|| "all".to_string());
    println!("invocation_bench — in-process hub, production DaemonInvocationService wiring");
    println!("scenario: {scenario}");

    if scenario == "tls" {
        scenario_tls().await;
        println!("\ndone.");
        return;
    }

    let mut env = setup().await;

    match scenario.as_str() {
        "latency" => scenario_latency(&env).await,
        "sweep" => scenario_sweep(&mut env).await,
        "slowpoke" => scenario_slowpoke(&mut env).await,
        "hol" => scenario_hol(&mut env).await,
        "all" => {
            scenario_latency(&env).await;
            scenario_sweep(&mut env).await;
            scenario_slowpoke(&mut env).await;
            scenario_hol(&mut env).await;
            scenario_tls().await;
        }
        other => {
            eprintln!("unknown scenario `{other}` (use latency|sweep|slowpoke|hol|tls|all)");
            std::process::exit(2);
        }
    }
    println!("\ndone.");
}
