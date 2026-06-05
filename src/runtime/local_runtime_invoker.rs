// EasyNet CLI - Axon LocalRuntime JSON invoker
// ============================================
//
// Small adapter for CLI-owned JSON ability payloads crossing into
// Axon's daemon-hosted LocalRuntime. Callers stay at the EasyNet
// `InvocationTarget` / `serde_json::Value` layer; this module owns
// the byte payload, terminal event, stream frame, and bidi split
// mechanics required by the Axon SDK.

use std::sync::Arc;

use easynet_axon::invocation::{
    AbilityFrame, AgentIdentity, BidiInputSender, BidiOutputReceiver, CausalContext,
    InvocationHandle, InvocationState, LocalRuntime, StreamingInvocationHandle, SubjectIdentity,
    UraProfile,
};
use ed25519_dalek::SigningKey;
use serde_json::Value;

use crate::runtime::invocation_target::{InvocationTarget, TargetScope};

pub struct RuntimeBidiSource {
    pub to_client: BidiInputSender,
    pub from_client: BidiOutputReceiver,
}

const LOCAL_CALLER_URA: &str = "easynet:///r/_system/agent/_system.local";
const LOCAL_CALLEE_URA: &str = "easynet:///r/_system/agent/_system.local";
const LOCAL_SUBJECT_SIGNING_SEED: [u8; 32] = [0x45; 32];

fn local_identity(ura: &str) -> AgentIdentity {
    AgentIdentity::new(ura, UraProfile::EasynetStrictV2)
}

fn local_subject(ura: String) -> SubjectIdentity {
    SubjectIdentity::new(ura, UraProfile::EasynetStrictV2)
}

fn local_invocation_subject(target: &InvocationTarget) -> SubjectIdentity {
    local_subject(
        target
            .subject
            .clone()
            .unwrap_or_else(|| LOCAL_CALLEE_URA.to_string()),
    )
}

fn local_subject_signing_key() -> SigningKey {
    SigningKey::from_bytes(&LOCAL_SUBJECT_SIGNING_SEED)
}

fn local_invocation_causal_context(target: &InvocationTarget) -> CausalContext {
    target.causal_context.clone().unwrap_or(CausalContext::None)
}

pub fn encode_json_payload(value: &Value) -> Result<Vec<u8>, String> {
    serde_json::to_vec(value).map_err(|err| format!("encode JSON payload: {err}"))
}

pub fn decode_json_payload(payload: &[u8]) -> Result<Value, String> {
    if payload.is_empty() {
        Ok(Value::Null)
    } else {
        serde_json::from_slice(payload).map_err(|err| format!("decode JSON payload: {err}"))
    }
}

pub fn ability_frame_to_json(frame: &AbilityFrame) -> Result<Value, String> {
    decode_json_payload(&frame.payload)
}

/// Reason-string fragments that the Axon SDK + the CLI's own local
/// dispatch arm produce when an ability is unknown. Centralised so
/// every consumer (`ability_dispatch`, `services/control/runtime_dispatch`,
/// `services/control/runtime_dispatch_adapter`) classifies "not found" through
/// the same predicate. If a future SDK version rephrases its
/// reason, this is the single grep target.
pub const NOT_FOUND_REASON_FRAGMENTS: &[&str] = &[
    "unknown_ability",
    "no local handler registered",
    "no local stream handler registered",
    "no local bidi handler registered",
    "not registered in Axon LocalRuntime",
];

/// True if `msg` contains any of the canonical "ability not found"
/// reason-string fragments. Defined over `&str` because callers come
/// from three error provenances — `anyhow::Error` (via Display),
/// `String` (from the local-runtime `Result<_, String>`), and
/// `easynet_axon::AxonError::reason`. A typed predicate over
/// `&AxonError` is a follow-up that depends on the SDK growing a
/// `NotFound` variant in its 7-class taxonomy.
pub fn is_not_found_error(msg: &str) -> bool {
    NOT_FOUND_REASON_FRAGMENTS
        .iter()
        .any(|fragment| msg.contains(fragment))
}

// `block_on_runtime` used to live here as a one-line wrapper around
// `crate::support::async_bridge::run_blocking` pinned to the
// `BuildCurrentThreadTokio` fallback. The wrapper hid the policy
// choice from the call site, and a second helper with a generic
// name made `git grep block_on` return adjacent-but-different
// shapes. Per the 2026-05-29 industrial-textbook review, every
// call site that drives a LocalRuntime future from sync code now
// reaches for `support::async_bridge::run_blocking(future,
// NoRuntimeFallback::BuildCurrentThreadTokio)` directly. The
// fallback choice is non-obvious enough (`UseFuturesExecutor`
// deadlocks against tokio resources) that exposing it at the call
// site is the honest shape.

pub fn ensure_local_target(target: &InvocationTarget) -> Result<(), String> {
    match &target.scope {
        TargetScope::Local => Ok(()),
        TargetScope::Remote { node } => Err(format!(
            "local Axon runtime cannot execute remote target `{}`; route through Axon federation",
            node.as_str()
        )),
    }
}

pub async fn open_local_stream(
    runtime: Arc<LocalRuntime>,
    target: InvocationTarget,
) -> Result<StreamingInvocationHandle, String> {
    ensure_local_target(&target)?;
    let payload = encode_json_payload(&target.normalized_args)?;
    let signing_key = local_subject_signing_key();
    let (handle, _) = runtime
        .invoke_signed_stream_async(
            local_identity(LOCAL_CALLER_URA),
            local_identity(LOCAL_CALLEE_URA),
            &target.ability,
            payload,
            Some(local_invocation_subject(&target)),
            local_invocation_causal_context(&target),
            None,
            &signing_key,
            None,
            None,
        )
        .await
        .map_err(|err| format!("{err}"))?;
    Ok(handle)
}

pub async fn open_local_bidi(
    runtime: Arc<LocalRuntime>,
    target: InvocationTarget,
) -> Result<RuntimeBidiSource, String> {
    ensure_local_target(&target)?;
    let payload = encode_json_payload(&target.normalized_args)?;
    let signing_key = local_subject_signing_key();
    let (handle, _) = runtime
        .invoke_signed_bidi_async(
            local_identity(LOCAL_CALLER_URA),
            local_identity(LOCAL_CALLEE_URA),
            &target.ability,
            payload,
            Some(local_invocation_subject(&target)),
            local_invocation_causal_context(&target),
            None,
            &signing_key,
            None,
            None,
        )
        .await
        .map_err(|err| format!("{err}"))?;
    let (to_client, from_client) = handle.split();
    Ok(RuntimeBidiSource {
        to_client,
        from_client,
    })
}

pub fn invoke_local_rpc_sync(
    runtime: Arc<LocalRuntime>,
    target: InvocationTarget,
) -> Result<Value, String> {
    crate::support::async_bridge::run_blocking(
        invoke_local_rpc(runtime, target),
        crate::support::async_bridge::NoRuntimeFallback::BuildCurrentThreadTokio,
    )
}

pub async fn invoke_local_rpc(
    runtime: Arc<LocalRuntime>,
    target: InvocationTarget,
) -> Result<Value, String> {
    ensure_local_target(&target)?;
    let payload = encode_json_payload(&target.normalized_args)?;
    let signing_key = local_subject_signing_key();
    let (handle, _) = runtime
        .invoke_signed_async(
            local_identity(LOCAL_CALLER_URA),
            local_identity(LOCAL_CALLEE_URA),
            &target.ability,
            payload,
            Some(local_invocation_subject(&target)),
            local_invocation_causal_context(&target),
            None,
            &signing_key,
            None,
            None,
        )
        .await
        .map_err(|err| format!("{err}"))?;
    rpc_value_from_handle(handle).await
}

pub async fn rpc_value_from_handle(handle: InvocationHandle) -> Result<Value, String> {
    let state = handle.wait().await;
    let events = handle.core().snapshot_events().await;
    let terminal = events
        .iter()
        .rev()
        .find(|event| event.state.is_terminal())
        .ok_or_else(|| {
            "Axon invocation reached terminal state without terminal event".to_string()
        })?;
    match state {
        InvocationState::Completed => decode_json_payload(&terminal.payload),
        InvocationState::Failed | InvocationState::TimedOut | InvocationState::Cancelled => {
            Err(if terminal.reason.is_empty() {
                format!("Axon invocation ended as {}", state.as_str())
            } else {
                terminal.reason.clone()
            })
        }
        other => Err(format!(
            "Axon invocation wait returned non-terminal state {}",
            other.as_str()
        )),
    }
}

pub async fn drain_local_stream_frames(
    runtime: Arc<LocalRuntime>,
    target: InvocationTarget,
) -> Result<Vec<Value>, String> {
    let mut stream = open_local_stream(runtime, target).await?;
    let mut frames = Vec::new();
    while let Some(frame_result) = stream.next_frame().await {
        let frame = frame_result.map_err(|err| format!("{err}"))?;
        if !frame.payload.is_empty() {
            frames.push(ability_frame_to_json(&frame)?);
        }
        if frame.terminal {
            break;
        }
    }
    Ok(frames)
}
