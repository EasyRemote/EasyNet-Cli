//! Real Chrome-over-Axon browser performance verification.
//! =======================================================
//!
//! File: plugins/browser/src/performance.rs
//! Description: Opt-in end-to-end latency and throughput regression probe.
//!
//! Protocol Responsibility:
//! - Exercise browser CDP JSON exclusively through the daemon-owned Axon
//!   LocalRuntime RPC, Stream, and InvokeBidi adapters.
//!
//! Implementation Approach:
//! - Bind the real browser plugin contribution into an executable catalog.
//! - Launch current Chrome, pipeline correlated Runtime.evaluate requests,
//!   capture one real viewport frame, and close the governed resource.
//!
//! Usage Contract:
//! - This ignored test requires an installed Chrome/Chromium executable.
//! - The single JSON output line is the reproducible raw measurement artifact.
//!
//! Architectural Position:
//! - Test-only observer above the plugin and canonical Axon runtime boundary.

use std::collections::{HashMap, HashSet};
use std::path::PathBuf;
use std::sync::{Arc, Barrier};
use std::time::{Duration, Instant, SystemTime, UNIX_EPOCH};

use axon_sdk::invocation::{
    fresh_nonce, AgentIdentity, AxonError, BidiInputFrame, BidiInputSender, BidiOutputReceiver,
    CallMode as AxonCallMode, CausalContext, DescriptorBoundEnvelope, DescriptorBoundEnvelopeParts,
    DescriptorBoundInvocationDraft, DescriptorBoundInvocationRequest, InvocationState, KeyResolver,
    LocalRuntime, SubjectIdentity, UraProfile,
};
use base64::Engine as _;
use ed25519_dalek::{SigningKey, VerifyingKey};
use serde_json::{json, Value};

use crate::daemon::ability::dispatch::{
    AxonAbilityCatalog, BidiSource, StreamSource, BIDI_CHANNEL_BOUND,
};
use crate::daemon::invocation::routing::target::{
    CallMode, InvocationCausalContext, InvocationSubject, InvocationTarget, TargetScope,
};
use crate::daemon::plugins::{
    DaemonPluginBinder, PluginContributionBuilder, PluginContributionSet, PluginKind,
    PluginRequirementSet, PluginRuntimeLimits,
};

use super::constants::{
    ABILITY_ATTACH_SESSION, ABILITY_CAPTURE_VIEWPORT, ABILITY_CLOSE_SESSION, ABILITY_OPEN_SESSION,
    ABILITY_SEND_INPUT, ABILITY_SHOW_SESSION, ATTACH_OPERATION_BOUND, REASON_CALLER_MISMATCH,
};

const BENCHMARK_SUBJECT: &str = "easynet:///r/acme/agent/test.browser-benchmark";
const TEST_DEVICE_URA: &str = "easynet:///r/acme/device/01DEV";
const USER_A_URA: &str = "easynet:///r/acme/user/alice";
const USER_B_URA: &str = "easynet:///r/acme/user/bob";
const INTERACTIVE_COMMANDS: usize = 20;
const BATCH_SIZE: usize = ATTACH_OPERATION_BOUND;
const BATCH_COUNT: usize = 20;
const BATCH_COMMANDS: usize = BATCH_SIZE * BATCH_COUNT;
const BENCHMARK_TIMEOUT: Duration = Duration::from_secs(15);

struct MultiUserKeyResolver {
    keys: HashMap<String, VerifyingKey>,
}

impl KeyResolver for MultiUserKeyResolver {
    fn resolve(&self, agent_ura: &str) -> Result<VerifyingKey, AxonError> {
        self.keys.get(agent_ura).cloned().ok_or_else(|| {
            AxonError::permission_denied(format!(
                "multi_user_browser_test_unknown_caller:{agent_ura}"
            ))
        })
    }
}

#[derive(Debug, Clone)]
struct BrowserAbilityBinding {
    callee_ura: String,
    ability_ura: String,
}

fn browser_catalog_with_runtime(runtime: Arc<LocalRuntime>) -> AxonAbilityCatalog {
    let limits = PluginRuntimeLimits::new(2, 8);
    let mut builder = PluginContributionBuilder::new(
        "easynet.browser",
        env!("CARGO_PKG_VERSION"),
        PluginKind::Builtin,
        limits,
        PluginRequirementSet::default(),
        Vec::new(),
    );
    super::registration::contribute(&mut builder, limits).expect("browser contribution");
    let contributions =
        PluginContributionSet::new(vec![builder.finish().expect("finish browser contribution")]);
    let mut catalog =
        AxonAbilityCatalog::new_test_runtime_for_device_authority(runtime, TEST_DEVICE_URA);
    DaemonPluginBinder::static_catalog(&mut catalog)
        .bind_set(&contributions)
        .expect("bind browser contribution into Axon LocalRuntime");
    catalog
}

fn executable_browser_catalog() -> AxonAbilityCatalog {
    browser_catalog_with_runtime(
        crate::daemon::axon_bridge::runtime_factory::build_local_runtime(
            crate::daemon::axon_bridge::runtime_factory::rejecting_test_key_resolver(),
            None,
        ),
    )
}

fn browser_ability_binding(catalog: &AxonAbilityCatalog, ability: &str) -> BrowserAbilityBinding {
    let row = catalog
        .authority_ability_catalog_snapshot()
        .into_iter()
        .find(|row| row.name == ability)
        .unwrap_or_else(|| panic!("browser ability {ability:?} must be bound"));
    BrowserAbilityBinding {
        callee_ura: row.descriptor.owner_ura.clone(),
        ability_ura: row
            .descriptor
            .canonical_ability_ura()
            .expect("browser descriptor must have a canonical ability URA"),
    }
}

async fn signed_browser_request(
    runtime: &Arc<LocalRuntime>,
    binding: &BrowserAbilityBinding,
    signing_key: &SigningKey,
    caller_ura: &str,
    subject_ura: &str,
    mode: AxonCallMode,
    args: Value,
) -> DescriptorBoundInvocationRequest {
    let payload = serde_json::to_vec(&args).expect("serialize signed browser request");
    let options = runtime
        .ability_options(&binding.ability_ura)
        .await
        .expect("browser ability must have runtime options");
    let proof = options
        .proof_for_mode(mode)
        .expect("browser ability must have descriptor proof for call mode");
    let descriptor_ref = format!(
        "{}@{}#{}!{}",
        binding.ability_ura,
        proof.descriptor_version,
        hex::encode(proof.descriptor_hash),
        proof.admission_action
    );
    let envelope = DescriptorBoundEnvelope::from_parts(DescriptorBoundEnvelopeParts {
        caller: AgentIdentity::new(caller_ura, UraProfile::StrictV2),
        callee: AgentIdentity::new(&binding.callee_ura, UraProfile::StrictV2),
        ability: descriptor_ref,
        subject: SubjectIdentity::new(subject_ura, UraProfile::StrictV2),
        invocation_nonce: fresh_nonce(),
        causal_context: CausalContext::None,
        args_bytes: &payload,
    })
    .expect("complete descriptor-bound browser invocation envelope");
    DescriptorBoundInvocationDraft::from_envelope(envelope)
        .with_payload(payload)
        .signed(mode, signing_key, format!("browser-test:{caller_ura}"))
        .expect("externally signed descriptor-bound browser request")
}

async fn signed_user_rpc(
    runtime: &Arc<LocalRuntime>,
    binding: &BrowserAbilityBinding,
    signing_key: &SigningKey,
    caller_ura: &str,
    subject_ura: &str,
    args: Value,
) -> Result<Value, String> {
    let request = signed_browser_request(
        runtime,
        binding,
        signing_key,
        caller_ura,
        subject_ura,
        AxonCallMode::Rpc,
        args,
    )
    .await;
    let (handle, _) = runtime
        .invoke_descriptor_bound_request_async(request)
        .await
        .map_err(|error| error.to_string())?;
    crate::daemon::invocation::dispatch::local_runtime_invoker::rpc_value_from_handle(handle).await
}

async fn signed_user_bidi(
    runtime: &Arc<LocalRuntime>,
    binding: &BrowserAbilityBinding,
    signing_key: &SigningKey,
    caller_ura: &str,
    subject_ura: &str,
) -> Result<(BidiInputSender, BidiOutputReceiver), String> {
    let request = signed_browser_request(
        runtime,
        binding,
        signing_key,
        caller_ura,
        subject_ura,
        AxonCallMode::Bidi,
        json!({}),
    )
    .await;
    let (handle, _) = runtime
        .invoke_descriptor_bound_bidi_request_async(request)
        .await
        .map_err(|error| error.to_string())?;
    Ok(handle.split())
}

async fn rejected_user_bidi(
    runtime: &Arc<LocalRuntime>,
    binding: &BrowserAbilityBinding,
    signing_key: &SigningKey,
    caller_ura: &str,
    subject_ura: &str,
) -> String {
    let request = signed_browser_request(
        runtime,
        binding,
        signing_key,
        caller_ura,
        subject_ura,
        AxonCallMode::Bidi,
        json!({}),
    )
    .await;
    match runtime
        .invoke_descriptor_bound_bidi_request_async(request)
        .await
    {
        Err(error) => error.to_string(),
        Ok((handle, _)) => {
            let finalized = tokio::time::timeout(BENCHMARK_TIMEOUT, handle.finalized())
                .await
                .expect("timed out waiting for rejected Axon InvokeBidi")
                .expect("rejected Axon InvokeBidi must finalize canonically");
            assert_eq!(finalized.terminal_state, InvocationState::Failed);
            finalized
                .failure
                .map(|error| error.to_string())
                .unwrap_or_else(|| "rejected Axon InvokeBidi omitted failure".to_string())
        }
    }
}

async fn send_signed_bidi_json(sender: &BidiInputSender, value: Value) {
    sender
        .send(
            BidiInputFrame::new(
                serde_json::to_vec(&value).expect("serialize Axon InvokeBidi JSON frame"),
            )
            .with_content_type("application/json"),
        )
        .await
        .expect("send Axon InvokeBidi JSON frame");
}

async fn recv_signed_bidi_json(receiver: &mut BidiOutputReceiver) -> Value {
    loop {
        let frame = tokio::time::timeout(BENCHMARK_TIMEOUT, receiver.next_frame())
            .await
            .expect("timed out waiting for signed Axon InvokeBidi output")
            .expect("signed Axon InvokeBidi output closed")
            .expect("signed Axon InvokeBidi output error");
        if !frame.payload.is_empty() {
            return serde_json::from_slice(&frame.payload)
                .expect("signed Axon InvokeBidi output JSON");
        }
        assert!(!frame.terminal, "empty terminal Axon InvokeBidi frame");
    }
}

async fn recv_signed_bidi_type(
    receiver: &mut BidiOutputReceiver,
    expected_type: &str,
    expected_id: Option<&str>,
) -> Value {
    loop {
        let frame = recv_signed_bidi_json(receiver).await;
        if frame["type"] != expected_type {
            continue;
        }
        if let Some(expected_id) = expected_id {
            if frame["id"].as_str() != Some(expected_id) {
                continue;
            }
        }
        return frame;
    }
}

async fn signed_cdp_command(
    sender: &BidiInputSender,
    receiver: &mut BidiOutputReceiver,
    id: &str,
    method: &str,
    params: Value,
) -> Value {
    send_signed_bidi_json(
        sender,
        json!({
            "type": "cdp.command",
            "id": id,
            "method": method,
            "params": params,
        }),
    )
    .await;
    recv_signed_bidi_type(receiver, "cdp.response", Some(id)).await
}

async fn signed_runtime_evaluate(
    sender: &BidiInputSender,
    receiver: &mut BidiOutputReceiver,
    id: &str,
    expression: &str,
) -> Value {
    let response = signed_cdp_command(
        sender,
        receiver,
        id,
        "Runtime.evaluate",
        json!({
            "expression": expression,
            "returnByValue": true,
            "awaitPromise": true,
        }),
    )
    .await;
    assert!(
        response.get("error").is_none(),
        "Runtime.evaluate transport failed: {response}"
    );
    assert!(
        response["result"].get("exceptionDetails").is_none(),
        "Runtime.evaluate JavaScript failed: {response}"
    );
    response["result"]["result"]["value"].clone()
}

async fn detach_signed_browser(
    sender: BidiInputSender,
    mut receiver: BidiOutputReceiver,
    expected_session_ura: &str,
) {
    send_signed_bidi_json(&sender, json!({"type": "detach"})).await;
    let detached = recv_signed_bidi_type(&mut receiver, "browser.detached", None).await;
    assert_eq!(detached["session_ura"].as_str(), Some(expected_session_ura));
    let finalized = receiver
        .finalized()
        .await
        .expect("signed browser bidi finalization");
    assert_eq!(finalized.terminal_state, InvocationState::Completed);
}

fn ephemeral_browser_profile_dirs() -> HashSet<PathBuf> {
    std::fs::read_dir(std::env::temp_dir())
        .into_iter()
        .flatten()
        .filter_map(Result::ok)
        .filter_map(|entry| {
            let is_profile = entry
                .file_name()
                .to_str()
                .is_some_and(|name| name.starts_with("easynet-browser-"));
            is_profile.then(|| entry.path())
        })
        .collect()
}

fn cookie_value(response: &Value, cookie_name: &str) -> Option<String> {
    response["result"]["cookies"]
        .as_array()?
        .iter()
        .find(|cookie| cookie["name"].as_str() == Some(cookie_name))
        .and_then(|cookie| cookie["value"].as_str())
        .map(str::to_string)
}

fn assert_caller_mismatch(error: &str, operation: &str) {
    assert!(
        error.contains(REASON_CALLER_MISMATCH),
        "{operation} must fail at browser caller ownership; error={error}"
    );
}

fn invocation(ability: &str, args: Value, call_mode: CallMode, subject: &str) -> InvocationTarget {
    InvocationTarget {
        scope: TargetScope::Local,
        ability: ability.to_string(),
        normalized_args: args,
        call_mode,
        subject: InvocationSubject::explicit(subject),
        causal_context: InvocationCausalContext::daemon_system_root(),
        request_metadata: HashMap::new(),
    }
}

async fn recv_json(bidi: &mut BidiSource) -> Value {
    tokio::time::timeout(BENCHMARK_TIMEOUT, bidi.from_client.recv())
        .await
        .expect("timed out waiting for Axon InvokeBidi output")
        .expect("Axon InvokeBidi output closed")
        .into_json_value()
        .expect("Axon InvokeBidi output JSON")
}

async fn first_stream_frame(source: StreamSource) -> Value {
    match source {
        StreamSource::Snapshot(frames) => frames.into_iter().next().expect("viewport snapshot"),
        StreamSource::SnapshotThenLive(frames, mut receiver) => match frames.into_iter().next() {
            Some(frame) => frame,
            None => tokio::time::timeout(BENCHMARK_TIMEOUT, receiver.recv())
                .await
                .expect("timed out waiting for viewport live frame")
                .expect("viewport live stream closed"),
        },
        StreamSource::Live(mut receiver) => {
            tokio::time::timeout(BENCHMARK_TIMEOUT, receiver.recv())
                .await
                .expect("timed out waiting for viewport live frame")
                .expect("viewport live stream closed")
        }
        StreamSource::Finite(mut receiver) => {
            tokio::time::timeout(BENCHMARK_TIMEOUT, receiver.recv())
                .await
                .expect("timed out waiting for viewport finite frame")
                .expect("viewport finite stream closed")
                .expect("viewport finite stream error")
        }
        StreamSource::BackpressuredLive(mut receiver) => {
            tokio::time::timeout(BENCHMARK_TIMEOUT, receiver.recv())
                .await
                .expect("timed out waiting for viewport backpressured live frame")
                .expect("viewport backpressured live stream closed")
                .expect("viewport backpressured live stream error")
        }
        StreamSource::TypedFinite(_) | StreamSource::TypedBackpressuredLive(_) => {
            panic!("browser viewport benchmark requires JSON stream frames")
        }
    }
}

fn percentile(sorted_micros: &[u64], percentile: usize) -> u64 {
    assert!(!sorted_micros.is_empty());
    let rank = percentile
        .saturating_mul(sorted_micros.len())
        .div_ceil(100)
        .saturating_sub(1)
        .min(sorted_micros.len() - 1);
    sorted_micros[rank]
}

fn duration_ms(duration: Duration) -> f64 {
    (duration.as_secs_f64() * 1_000.0 * 100.0).round() / 100.0
}

fn frame_payload_chars(frame: &Value) -> usize {
    frame
        .get("data")
        .and_then(Value::as_str)
        .map_or(0, str::len)
}

async fn detach(mut bidi: BidiSource) {
    bidi.to_client
        .send(json!({"type":"detach"}))
        .await
        .expect("send detach through Axon InvokeBidi");
    loop {
        if recv_json(&mut bidi).await["type"] == "browser.detached" {
            break;
        }
    }
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
#[ignore = "requires an installed current Chrome/Chromium executable"]
async fn current_chrome_axon_bidi_performance() {
    let catalog = Arc::new(executable_browser_catalog());

    let open_started = Instant::now();
    let opened = catalog
        .execute_rpc(invocation(
            ABILITY_OPEN_SESSION,
            json!({
                "url": "https://example.com/",
                "headless": true,
                "viewport_width": 800,
                "viewport_height": 600,
                "idle_timeout_seconds": 300,
            }),
            CallMode::Rpc,
            BENCHMARK_SUBJECT,
        ))
        .expect("open current Chrome through Axon LocalRuntime");
    let open_elapsed = open_started.elapsed();
    let session_ura = opened["session_ura"]
        .as_str()
        .expect("open returns session_ura")
        .to_string();

    let attach_started = Instant::now();
    let mut bidi = catalog
        .execute_bidi(invocation(
            ABILITY_ATTACH_SESSION,
            json!({}),
            CallMode::Bidi,
            &session_ura,
        ))
        .expect("attach through Axon LocalRuntime InvokeBidi");
    let ready = recv_json(&mut bidi).await;
    assert_eq!(ready["type"], "browser.ready");
    assert_eq!(ready["transport"], "axon_invoke_bidi");
    let attach_ready_elapsed = attach_started.elapsed();

    let mut interactive_latencies_micros = Vec::with_capacity(INTERACTIVE_COMMANDS);
    for id in 0..INTERACTIVE_COMMANDS {
        let sent_at = Instant::now();
        bidi.to_client
            .send(json!({
                "type": "cdp.command",
                "id": id,
                "method": "Runtime.evaluate",
                "params": {
                    "expression": id.to_string(),
                    "returnByValue": true,
                },
            }))
            .await
            .expect("send command through Axon InvokeBidi");
        loop {
            let frame = recv_json(&mut bidi).await;
            if frame["type"] != "cdp.response" {
                continue;
            }
            assert_eq!(frame["id"], id as u64);
            assert_eq!(frame["result"]["result"]["value"], id as u64);
            interactive_latencies_micros.push(sent_at.elapsed().as_micros() as u64);
            break;
        }
    }
    interactive_latencies_micros.sort_unstable();

    let batch_started = Instant::now();
    let mut batch_sent_at = Vec::with_capacity(BATCH_COUNT);
    for batch_id in 0..BATCH_COUNT {
        let first_command_id = batch_id * BATCH_SIZE;
        let commands = (first_command_id..first_command_id + BATCH_SIZE)
            .map(|id| {
                json!({
                    "id": id,
                    "method": "Runtime.evaluate",
                    "params": {
                        "expression": id.to_string(),
                        "returnByValue": true,
                    },
                })
            })
            .collect::<Vec<_>>();
        batch_sent_at.push(Instant::now());
        bidi.to_client
            .send(json!({
                "type": "cdp.batch",
                "id": batch_id,
                "commands": commands,
            }))
            .await
            .expect("send CDP batch through Axon InvokeBidi");
    }

    let mut batch_latencies_micros = Vec::with_capacity(BATCH_COMMANDS);
    let mut received = 0_usize;
    while received < BATCH_COMMANDS {
        let frame = recv_json(&mut bidi).await;
        if frame["type"] != "cdp.batch_response" {
            continue;
        }
        let batch_id = frame["id"].as_u64().expect("numeric batch id") as usize;
        assert!(batch_id < BATCH_COUNT, "batch id outside benchmark window");
        let responses = frame["responses"]
            .as_array()
            .expect("batch responses array");
        assert_eq!(responses.len(), BATCH_SIZE);
        let latency = batch_sent_at[batch_id].elapsed().as_micros() as u64;
        for response in responses {
            let id = response["id"].as_u64().expect("numeric response id") as usize;
            assert!(
                id < BATCH_COMMANDS,
                "response id outside benchmark commands"
            );
            assert_eq!(response["result"]["result"]["value"], id as u64);
            batch_latencies_micros.push(latency);
        }
        received += responses.len();
    }
    let batch_elapsed = batch_started.elapsed();
    batch_latencies_micros.sort_unstable();

    detach(bidi).await;

    let viewport_started = Instant::now();
    let viewport_source = catalog
        .execute_stream(invocation(
            ABILITY_CAPTURE_VIEWPORT,
            json!({
                "format": "jpeg",
                "quality": 70,
                "max_width": 800,
                "max_height": 600,
                "max_frames": 1,
                "timeout_seconds": 10,
            }),
            CallMode::Stream,
            &session_ura,
        ))
        .expect("capture viewport through Axon LocalRuntime stream");
    let viewport = first_stream_frame(viewport_source).await;
    let viewport_elapsed = viewport_started.elapsed();
    assert_eq!(viewport["type"], "browser.viewport_frame");
    assert_eq!(viewport["content_type"], "image/jpeg");
    let viewport_base64_chars = frame_payload_chars(&viewport);
    assert!(
        viewport_base64_chars > 100,
        "real viewport payload is empty"
    );

    let close_started = Instant::now();
    let close_barrier = Arc::new(Barrier::new(2));
    let close_catalog_a = Arc::clone(&catalog);
    let close_session_a = session_ura.clone();
    let close_barrier_a = Arc::clone(&close_barrier);
    let close_a = tokio::task::spawn_blocking(move || {
        close_barrier_a.wait();
        close_catalog_a.execute_rpc(invocation(
            ABILITY_CLOSE_SESSION,
            json!({}),
            CallMode::Rpc,
            &close_session_a,
        ))
    });
    let close_catalog_b = Arc::clone(&catalog);
    let close_session_b = session_ura.clone();
    let close_barrier_b = Arc::clone(&close_barrier);
    let close_b = tokio::task::spawn_blocking(move || {
        close_barrier_b.wait();
        close_catalog_b.execute_rpc(invocation(
            ABILITY_CLOSE_SESSION,
            json!({}),
            CallMode::Rpc,
            &close_session_b,
        ))
    });
    let (closed_a, closed_b) = tokio::join!(close_a, close_b);
    let closed_a = closed_a
        .expect("first close task")
        .expect("first close through Axon LocalRuntime");
    let closed_b = closed_b
        .expect("second close task")
        .expect("second close through Axon LocalRuntime");
    let close_elapsed = close_started.elapsed();
    assert_eq!(closed_a["state"], "closed");
    assert_eq!(closed_b["state"], "closed");
    assert!(
        !closed_a["already_closed"].as_bool().unwrap_or(true)
            || !closed_b["already_closed"].as_bool().unwrap_or(true),
        "one concurrent close must own teardown"
    );

    let throughput_commands_per_second = BATCH_COMMANDS as f64 / batch_elapsed.as_secs_f64();
    let metrics = json!({
        "schema": "easynet.browser.performance.v1",
        "measured_at_unix_ms": SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .expect("system time after epoch")
            .as_millis(),
        "platform": {"os": std::env::consts::OS, "arch": std::env::consts::ARCH},
        "browser_product": opened["browser"]["product"],
        "cdp_protocol": opened["browser"]["protocol_version"],
        "transport": "Axon LocalRuntime InvokeBidi json_frames",
        "command": "Runtime.evaluate(returnByValue=true)",
        "interactive_command_count": INTERACTIVE_COMMANDS,
        "batch_count": BATCH_COUNT,
        "batch_size": BATCH_SIZE,
        "batch_command_count": BATCH_COMMANDS,
        "attachment_operation_bound": ATTACH_OPERATION_BOUND,
        "axon_channel_bound": BIDI_CHANNEL_BOUND,
        "open_ms": duration_ms(open_elapsed),
        "attach_ready_ms": duration_ms(attach_ready_elapsed),
        "batch_ms": duration_ms(batch_elapsed),
        "throughput_commands_per_second":
            (throughput_commands_per_second * 100.0).round() / 100.0,
        "interactive_round_trip_latency_ms": {
            "min": interactive_latencies_micros[0] as f64 / 1_000.0,
            "p50": percentile(&interactive_latencies_micros, 50) as f64 / 1_000.0,
            "p95": percentile(&interactive_latencies_micros, 95) as f64 / 1_000.0,
            "p99": percentile(&interactive_latencies_micros, 99) as f64 / 1_000.0,
            "max": interactive_latencies_micros[interactive_latencies_micros.len() - 1] as f64 / 1_000.0,
        },
        "batch_round_trip_latency_ms": {
            "min": batch_latencies_micros[0] as f64 / 1_000.0,
            "p50": percentile(&batch_latencies_micros, 50) as f64 / 1_000.0,
            "p95": percentile(&batch_latencies_micros, 95) as f64 / 1_000.0,
            "p99": percentile(&batch_latencies_micros, 99) as f64 / 1_000.0,
            "max": batch_latencies_micros[batch_latencies_micros.len() - 1] as f64 / 1_000.0,
        },
        "first_viewport_frame_ms": duration_ms(viewport_elapsed),
        "viewport_base64_chars": viewport_base64_chars,
        "close_ms": duration_ms(close_elapsed),
        "concurrent_close_callers": 2,
    });
    println!(
        "BROWSER_CDP_AXON_METRICS={}",
        serde_json::to_string(&metrics).expect("serialize metrics")
    );

    assert!(open_elapsed < Duration::from_secs(20));
    assert!(attach_ready_elapsed < Duration::from_secs(2));
    assert!(percentile(&interactive_latencies_micros, 95) <= 250_000);
    assert!(throughput_commands_per_second >= 500.0);
    assert!(percentile(&batch_latencies_micros, 95) <= 1_000_000);
    assert!(viewport_elapsed < Duration::from_secs(5));
}

#[tokio::test(flavor = "multi_thread", worker_threads = 6)]
#[ignore = "requires an installed current Chrome/Chromium executable"]
async fn two_signed_users_get_isolated_real_browser_sessions() {
    let user_a_key = SigningKey::from_bytes(&[0xA1; 32]);
    let user_b_key = SigningKey::from_bytes(&[0xB2; 32]);
    let key_resolver = MultiUserKeyResolver {
        keys: HashMap::from([
            (USER_A_URA.to_string(), user_a_key.verifying_key()),
            (USER_B_URA.to_string(), user_b_key.verifying_key()),
        ]),
    };
    let runtime = crate::daemon::axon_bridge::runtime_factory::build_local_runtime(
        Arc::new(key_resolver),
        None,
    );
    let catalog = browser_catalog_with_runtime(Arc::clone(&runtime));
    let open_binding = browser_ability_binding(&catalog, ABILITY_OPEN_SESSION);
    let show_binding = browser_ability_binding(&catalog, ABILITY_SHOW_SESSION);
    let input_binding = browser_ability_binding(&catalog, ABILITY_SEND_INPUT);
    let attach_binding = browser_ability_binding(&catalog, ABILITY_ATTACH_SESSION);
    let close_binding = browser_ability_binding(&catalog, ABILITY_CLOSE_SESSION);
    let profiles_before = ephemeral_browser_profile_dirs();

    let open_args = json!({
        "url": "https://example.com/",
        "headless": true,
        "viewport_width": 800,
        "viewport_height": 600,
        "idle_timeout_seconds": 300,
    });
    let open_started = Instant::now();
    let (opened_a, opened_b) = tokio::join!(
        signed_user_rpc(
            &runtime,
            &open_binding,
            &user_a_key,
            USER_A_URA,
            &open_binding.callee_ura,
            open_args.clone(),
        ),
        signed_user_rpc(
            &runtime,
            &open_binding,
            &user_b_key,
            USER_B_URA,
            &open_binding.callee_ura,
            open_args,
        ),
    );
    let opened_a = opened_a.expect("User A opens Chrome through signed Axon RPC");
    let opened_b = opened_b.expect("User B opens Chrome through signed Axon RPC");
    let open_pair_elapsed = open_started.elapsed();
    let session_a = opened_a["session_ura"]
        .as_str()
        .expect("User A open returns resource URA")
        .to_string();
    let session_b = opened_b["session_ura"]
        .as_str()
        .expect("User B open returns resource URA")
        .to_string();
    assert_ne!(session_a, session_b, "users must own distinct resources");
    assert_ne!(
        opened_a["target_id"], opened_b["target_id"],
        "users must own distinct Chrome targets"
    );
    for opened in [&opened_a, &opened_b] {
        assert_eq!(opened["state"], "active");
        assert_eq!(opened["browser"]["owned"], true);
        assert_eq!(opened["browser"]["profile_mode"], "ephemeral");
    }
    let created_profiles = ephemeral_browser_profile_dirs()
        .difference(&profiles_before)
        .cloned()
        .collect::<HashSet<_>>();
    assert_eq!(
        created_profiles.len(),
        2,
        "each real Chrome session must create one isolated ephemeral profile"
    );

    let attach_started = Instant::now();
    let (attached_a, attached_b) = tokio::join!(
        signed_user_bidi(
            &runtime,
            &attach_binding,
            &user_a_key,
            USER_A_URA,
            &session_a,
        ),
        signed_user_bidi(
            &runtime,
            &attach_binding,
            &user_b_key,
            USER_B_URA,
            &session_b,
        ),
    );
    let (input_a, mut output_a) = attached_a.expect("User A attaches through signed Axon bidi");
    let (input_b, mut output_b) = attached_b.expect("User B attaches through signed Axon bidi");
    let (ready_a, ready_b) = tokio::join!(
        recv_signed_bidi_type(&mut output_a, "browser.ready", None),
        recv_signed_bidi_type(&mut output_b, "browser.ready", None),
    );
    let attach_pair_elapsed = attach_started.elapsed();
    for ready in [&ready_a, &ready_b] {
        assert_eq!(ready["transport"], "axon_invoke_bidi");
        assert_eq!(ready["wire"], "cdp_json_v1");
    }

    let isolation_started = Instant::now();
    send_signed_bidi_json(
        &input_a,
        json!({
            "type": "cdp.command",
            "id": "set-a",
            "method": "Network.setCookie",
            "params": {
                "url": "https://example.com/",
                "name": "easynet_owner",
                "value": "alice",
            },
        }),
    )
    .await;
    send_signed_bidi_json(
        &input_b,
        json!({
            "type": "cdp.command",
            "id": "set-b",
            "method": "Network.setCookie",
            "params": {
                "url": "https://example.com/",
                "name": "easynet_owner",
                "value": "bob",
            },
        }),
    )
    .await;
    let (set_a, set_b) = tokio::join!(
        recv_signed_bidi_type(&mut output_a, "cdp.response", Some("set-a")),
        recv_signed_bidi_type(&mut output_b, "cdp.response", Some("set-b")),
    );
    assert_eq!(set_a["result"]["success"], true);
    assert_eq!(set_b["result"]["success"], true);

    send_signed_bidi_json(
        &input_a,
        json!({
            "type": "cdp.command",
            "id": "get-a",
            "method": "Network.getCookies",
            "params": {"urls": ["https://example.com/"]},
        }),
    )
    .await;
    send_signed_bidi_json(
        &input_b,
        json!({
            "type": "cdp.command",
            "id": "get-b",
            "method": "Network.getCookies",
            "params": {"urls": ["https://example.com/"]},
        }),
    )
    .await;
    let (cookies_a, cookies_b) = tokio::join!(
        recv_signed_bidi_type(&mut output_a, "cdp.response", Some("get-a")),
        recv_signed_bidi_type(&mut output_b, "cdp.response", Some("get-b")),
    );
    assert_eq!(
        cookie_value(&cookies_a, "easynet_owner").as_deref(),
        Some("alice")
    );
    assert_eq!(
        cookie_value(&cookies_b, "easynet_owner").as_deref(),
        Some("bob")
    );
    let isolation_elapsed = isolation_started.elapsed();

    let mut cross_user_denials = 0_usize;
    for (operation, error) in [
        (
            "User B show User A",
            signed_user_rpc(
                &runtime,
                &show_binding,
                &user_b_key,
                USER_B_URA,
                &session_a,
                json!({}),
            )
            .await
            .expect_err("User B must not show User A's session"),
        ),
        (
            "User B input User A",
            signed_user_rpc(
                &runtime,
                &input_binding,
                &user_b_key,
                USER_B_URA,
                &session_a,
                json!({"event": {"kind": "text", "text": "intruder"}}),
            )
            .await
            .expect_err("User B must not input into User A's session"),
        ),
        (
            "User B attach User A",
            rejected_user_bidi(
                &runtime,
                &attach_binding,
                &user_b_key,
                USER_B_URA,
                &session_a,
            )
            .await,
        ),
        (
            "User B close User A",
            signed_user_rpc(
                &runtime,
                &close_binding,
                &user_b_key,
                USER_B_URA,
                &session_a,
                json!({}),
            )
            .await
            .expect_err("User B must not close User A's session"),
        ),
        (
            "User A show User B",
            signed_user_rpc(
                &runtime,
                &show_binding,
                &user_a_key,
                USER_A_URA,
                &session_b,
                json!({}),
            )
            .await
            .expect_err("User A must not show User B's session"),
        ),
        (
            "User A input User B",
            signed_user_rpc(
                &runtime,
                &input_binding,
                &user_a_key,
                USER_A_URA,
                &session_b,
                json!({"event": {"kind": "text", "text": "intruder"}}),
            )
            .await
            .expect_err("User A must not input into User B's session"),
        ),
        (
            "User A attach User B",
            rejected_user_bidi(
                &runtime,
                &attach_binding,
                &user_a_key,
                USER_A_URA,
                &session_b,
            )
            .await,
        ),
        (
            "User A close User B",
            signed_user_rpc(
                &runtime,
                &close_binding,
                &user_a_key,
                USER_A_URA,
                &session_b,
                json!({}),
            )
            .await
            .expect_err("User A must not close User B's session"),
        ),
    ] {
        assert_caller_mismatch(&error, operation);
        cross_user_denials += 1;
    }
    assert_eq!(cross_user_denials, 8);

    let (shown_a, shown_b) = tokio::join!(
        signed_user_rpc(
            &runtime,
            &show_binding,
            &user_a_key,
            USER_A_URA,
            &session_a,
            json!({}),
        ),
        signed_user_rpc(
            &runtime,
            &show_binding,
            &user_b_key,
            USER_B_URA,
            &session_b,
            json!({}),
        ),
    );
    assert_eq!(
        shown_a.expect("User A still owns session")["state"],
        "active"
    );
    assert_eq!(
        shown_b.expect("User B still owns session")["state"],
        "active"
    );

    tokio::join!(
        detach_signed_browser(input_a, output_a, &session_a),
        detach_signed_browser(input_b, output_b, &session_b),
    );

    let close_started = Instant::now();
    let (closed_a, closed_b) = tokio::join!(
        signed_user_rpc(
            &runtime,
            &close_binding,
            &user_a_key,
            USER_A_URA,
            &session_a,
            json!({}),
        ),
        signed_user_rpc(
            &runtime,
            &close_binding,
            &user_b_key,
            USER_B_URA,
            &session_b,
            json!({}),
        ),
    );
    let closed_a = closed_a.expect("User A closes own session through signed Axon RPC");
    let closed_b = closed_b.expect("User B closes own session through signed Axon RPC");
    let close_pair_elapsed = close_started.elapsed();
    for closed in [&closed_a, &closed_b] {
        assert_eq!(closed["state"], "closed");
        assert_eq!(closed["already_closed"], false);
    }
    for profile in &created_profiles {
        assert!(
            !profile.exists(),
            "ephemeral browser profile must be removed: {}",
            profile.display()
        );
    }

    let result = json!({
        "schema": "easynet.browser.multi_user_axon.v1",
        "measured_at_unix_ms": SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .expect("system time after epoch")
            .as_millis(),
        "users": [USER_A_URA, USER_B_URA],
        "transport": "externally_signed_axon_local_runtime_invoke_bidi_json_frames",
        "browser_products": [opened_a["browser"]["product"], opened_b["browser"]["product"]],
        "cdp_protocols": [opened_a["browser"]["protocol_version"], opened_b["browser"]["protocol_version"]],
        "distinct_session_uras": true,
        "distinct_target_ids": true,
        "ephemeral_profiles_created_and_removed": created_profiles.len(),
        "independent_cookie_values": true,
        "cross_user_denials": cross_user_denials,
        "own_sessions_remained_active_after_denials": true,
        "own_closes_succeeded": true,
        "open_pair_ms": duration_ms(open_pair_elapsed),
        "attach_pair_ms": duration_ms(attach_pair_elapsed),
        "isolation_probe_ms": duration_ms(isolation_elapsed),
        "close_pair_ms": duration_ms(close_pair_elapsed),
    });
    println!(
        "BROWSER_MULTI_USER_AXON_RESULT={}",
        serde_json::to_string(&result).expect("serialize multi-user result")
    );
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
#[ignore = "requires current Chrome and public network access to github.com"]
async fn github_frontend_renders_animation_through_signed_axon_cdp() {
    const GITHUB_USER_URA: &str = "easynet:///r/acme/user/github-renderer";
    const GITHUB_URL: &str = "https://github.com/";
    const PAGE_PROBE: &str = r#"(() => ({
        hostname: location.hostname,
        href: location.href,
        readyState: document.readyState,
        title: document.title,
        bodyTextLength: document.body?.innerText?.length ?? 0,
        stylesheets: document.styleSheets.length,
        scripts: document.scripts.length,
        responseStatus: performance.getEntriesByType('navigation')[0]?.responseStatus ?? 0
    }))()"#;
    const INSTALL_ANIMATION: &str = r#"(() => {
        document.getElementById('__easynet_animation_probe')?.remove();
        const probe = document.createElement('div');
        probe.id = '__easynet_animation_probe';
        probe.textContent = 'EasyNet Axon · GitHub render';
        Object.assign(probe.style, {
            position: 'fixed',
            left: '24px',
            top: '96px',
            width: '260px',
            height: '72px',
            display: 'grid',
            placeItems: 'center',
            color: 'white',
            background: 'linear-gradient(90deg, #f97316, #db2777)',
            border: '4px solid white',
            borderRadius: '16px',
            boxShadow: '0 14px 36px rgba(0,0,0,.35)',
            font: '700 16px system-ui',
            zIndex: '2147483647',
            pointerEvents: 'none'
        });
        document.documentElement.appendChild(probe);
        const animation = probe.animate(
            [
                { transform: 'translateX(0px) rotate(-2deg)' },
                { transform: 'translateX(420px) rotate(2deg)' }
            ],
            { duration: 1200, direction: 'alternate', iterations: Infinity, easing: 'ease-in-out' }
        );
        window.__easynetAnimationProbe = { probe, animation };
        return {
            animationCount: document.getAnimations().length,
            playState: animation.playState,
            currentTime: animation.currentTime
        };
    })()"#;
    const ANIMATION_STATE: &str = r#"(() => {
        const value = window.__easynetAnimationProbe;
        return {
            transform: getComputedStyle(value.probe).transform,
            currentTime: value.animation.currentTime,
            playState: value.animation.playState
        };
    })()"#;

    let signing_key = SigningKey::from_bytes(&[0xC3; 32]);
    let runtime = crate::daemon::axon_bridge::runtime_factory::build_local_runtime(
        Arc::new(MultiUserKeyResolver {
            keys: HashMap::from([(GITHUB_USER_URA.to_string(), signing_key.verifying_key())]),
        }),
        None,
    );
    let catalog = browser_catalog_with_runtime(Arc::clone(&runtime));
    let open_binding = browser_ability_binding(&catalog, ABILITY_OPEN_SESSION);
    let attach_binding = browser_ability_binding(&catalog, ABILITY_ATTACH_SESSION);
    let close_binding = browser_ability_binding(&catalog, ABILITY_CLOSE_SESSION);
    let profiles_before = ephemeral_browser_profile_dirs();

    let open_started = Instant::now();
    let opened = signed_user_rpc(
        &runtime,
        &open_binding,
        &signing_key,
        GITHUB_USER_URA,
        &open_binding.callee_ura,
        json!({
            "url": GITHUB_URL,
            "headless": true,
            "viewport_width": 1280,
            "viewport_height": 800,
            "idle_timeout_seconds": 300,
        }),
    )
    .await
    .expect("open real GitHub in Chrome through signed Axon RPC");
    let open_elapsed = open_started.elapsed();
    let session_ura = opened["session_ura"]
        .as_str()
        .expect("GitHub browser session URA")
        .to_string();
    assert_eq!(opened["browser"]["owned"], true);
    assert_eq!(opened["browser"]["profile_mode"], "ephemeral");
    let created_profiles = ephemeral_browser_profile_dirs()
        .difference(&profiles_before)
        .cloned()
        .collect::<HashSet<_>>();
    assert_eq!(created_profiles.len(), 1);

    let attach_started = Instant::now();
    let (input, mut output) = signed_user_bidi(
        &runtime,
        &attach_binding,
        &signing_key,
        GITHUB_USER_URA,
        &session_ura,
    )
    .await
    .expect("attach to GitHub through signed Axon InvokeBidi");
    let ready = recv_signed_bidi_type(&mut output, "browser.ready", None).await;
    assert_eq!(ready["transport"], "axon_invoke_bidi");
    let attach_elapsed = attach_started.elapsed();

    let page_deadline = Instant::now() + Duration::from_secs(20);
    let mut page_attempt = 0_u32;
    let page = loop {
        let probe_id = format!("github-page-{page_attempt}");
        page_attempt += 1;
        let page = signed_runtime_evaluate(&input, &mut output, &probe_id, PAGE_PROBE).await;
        let loaded = page["hostname"] == "github.com"
            && page["readyState"] == "complete"
            && page["bodyTextLength"].as_u64().unwrap_or_default() > 1_000
            && page["stylesheets"].as_u64().unwrap_or_default() > 0
            && page["scripts"].as_u64().unwrap_or_default() > 0;
        if loaded {
            break page;
        }
        assert!(
            Instant::now() < page_deadline,
            "GitHub frontend did not become render-ready: {page}"
        );
        tokio::time::sleep(Duration::from_millis(250)).await;
    };

    let animation =
        signed_runtime_evaluate(&input, &mut output, "install-animation", INSTALL_ANIMATION).await;
    assert_eq!(animation["playState"], "running");
    assert!(animation["animationCount"].as_u64().unwrap_or_default() >= 1);
    tokio::time::sleep(Duration::from_millis(120)).await;

    let state_a =
        signed_runtime_evaluate(&input, &mut output, "animation-state-a", ANIMATION_STATE).await;
    let screenshot_a = signed_cdp_command(
        &input,
        &mut output,
        "screenshot-a",
        "Page.captureScreenshot",
        json!({"format": "png", "fromSurface": true, "captureBeyondViewport": false}),
    )
    .await;
    tokio::time::sleep(Duration::from_millis(450)).await;
    let state_b =
        signed_runtime_evaluate(&input, &mut output, "animation-state-b", ANIMATION_STATE).await;
    let screenshot_b = signed_cdp_command(
        &input,
        &mut output,
        "screenshot-b",
        "Page.captureScreenshot",
        json!({"format": "png", "fromSurface": true, "captureBeyondViewport": false}),
    )
    .await;
    assert_eq!(state_a["playState"], "running");
    assert_eq!(state_b["playState"], "running");
    assert_ne!(state_a["transform"], state_b["transform"]);
    assert_ne!(state_a["currentTime"], state_b["currentTime"]);

    let png_a_base64 = screenshot_a["result"]["data"]
        .as_str()
        .expect("first GitHub screenshot base64");
    let png_b_base64 = screenshot_b["result"]["data"]
        .as_str()
        .expect("second GitHub screenshot base64");
    assert!(png_a_base64.len() > 10_000);
    assert!(png_b_base64.len() > 10_000);
    assert_ne!(png_a_base64, png_b_base64);
    let png_a = base64::engine::general_purpose::STANDARD
        .decode(png_a_base64)
        .expect("decode first GitHub PNG");
    let png_b = base64::engine::general_purpose::STANDARD
        .decode(png_b_base64)
        .expect("decode second GitHub PNG");
    for png in [&png_a, &png_b] {
        assert!(png.starts_with(&[0x89, b'P', b'N', b'G', 0x0D, 0x0A, 0x1A, 0x0A]));
    }
    let frame_a_path = std::env::temp_dir().join("easynet-github-animation-frame-a.png");
    let frame_b_path = std::env::temp_dir().join("easynet-github-animation-frame-b.png");
    std::fs::write(&frame_a_path, &png_a).expect("write first GitHub render artifact");
    std::fs::write(&frame_b_path, &png_b).expect("write second GitHub render artifact");

    detach_signed_browser(input, output, &session_ura).await;
    let close_started = Instant::now();
    let closed = signed_user_rpc(
        &runtime,
        &close_binding,
        &signing_key,
        GITHUB_USER_URA,
        &session_ura,
        json!({}),
    )
    .await
    .expect("close GitHub browser through signed Axon RPC");
    let close_elapsed = close_started.elapsed();
    assert_eq!(closed["state"], "closed");
    assert_eq!(closed["already_closed"], false);
    for profile in &created_profiles {
        assert!(!profile.exists(), "ephemeral GitHub profile survived close");
    }

    let result = json!({
        "schema": "easynet.browser.github_animation.v1",
        "measured_at_unix_ms": SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .expect("system time after epoch")
            .as_millis(),
        "caller": GITHUB_USER_URA,
        "transport": "externally_signed_axon_local_runtime_invoke_bidi_json_frames",
        "browser_product": opened["browser"]["product"],
        "cdp_protocol": opened["browser"]["protocol_version"],
        "page": page,
        "animation": {
            "installed": animation,
            "state_a": state_a,
            "state_b": state_b,
            "transform_changed": true,
            "png_frames_differ": true,
        },
        "png_bytes": [png_a.len(), png_b.len()],
        "frame_paths": [frame_a_path, frame_b_path],
        "open_ms": duration_ms(open_elapsed),
        "attach_ms": duration_ms(attach_elapsed),
        "close_ms": duration_ms(close_elapsed),
        "ephemeral_profiles_removed": created_profiles.len(),
    });
    println!(
        "BROWSER_GITHUB_ANIMATION_RESULT={}",
        serde_json::to_string(&result).expect("serialize GitHub animation result")
    );
}

#[test]
fn percentile_uses_nearest_rank_without_scanning() {
    let values = [1, 2, 3, 4, 5, 6, 7, 8, 9, 10];
    assert_eq!(percentile(&values, 50), 5);
    assert_eq!(percentile(&values, 95), 10);
    assert_eq!(percentile(&values, 99), 10);
}
