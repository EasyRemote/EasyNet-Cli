// EasyNet CLI — daemon LocalRuntime invocation adapter
// ====================================================
//
// Small daemon-side adapter for JSON ability payloads crossing into
// Axon's embedded LocalRuntime. Callers stay at the EasyNet
// `InvocationTarget` / `serde_json::Value` layer; this module owns
// the byte payload, terminal event, stream frame, and bidi split
// mechanics required by the Axon SDK. It is not an external agent
// runtime adapter and does not own handler bodies.

use std::sync::Arc;

use easynet_axon::invocation::{
    fresh_nonce, AbilityFrame, AgentIdentity, BidiInputSender, BidiOutputReceiver,
    CallMode as AxonInvocationCallMode, CausalContext, DescriptorBoundEnvelope,
    DescriptorBoundEnvelopeParts, DescriptorBoundInvocationRequest, InvocationHandle,
    InvocationState, LocalRuntime, StreamingInvocationHandle, SubjectIdentity, UraProfile,
};
use serde_json::Value;

use crate::daemon::axon_bridge::descriptor_ref::{
    ability_descriptor_ref_for_wire, ability_ura_for_wire, registered_descriptor_version,
};
use crate::daemon::axon_bridge::local_runtime_request::{
    LocalRuntimeIngress, LocalRuntimeRequestFactory, LocalRuntimeRequestOptions,
};
use crate::daemon::identity::local_invocation::{
    agent_identity, local_device_ura, system_agent_identity,
};
use crate::daemon::invocation::routing::target::{InvocationTarget, TargetScope};

/// Bidirectional LocalRuntime stream halves exposed to daemon dispatchers.
///
/// The source owns the split after Axon's `StreamingInvocationHandle`
/// has accepted a bidi request. It deliberately carries only the Axon
/// sender/receiver pair; session policy, admission, and caller-visible
/// framing remain in daemon invocation/ability dispatch layers.
pub struct RuntimeBidiSource {
    pub to_client: BidiInputSender,
    pub from_client: BidiOutputReceiver,
}

fn local_identity(ura: &str) -> AgentIdentity {
    agent_identity(ura)
}

fn local_subject(ura: String) -> SubjectIdentity {
    SubjectIdentity::new(ura, UraProfile::EasynetStrictV2)
}

fn local_invocation_callee_ura(target: &InvocationTarget) -> String {
    if let Ok(selector) = crate::core::ura::AbilitySelector::parse(&target.ability) {
        return selector.owner_ura().to_string();
    }

    local_device_ura()
}

fn explicit_subject_ura(target: &InvocationTarget) -> Option<String> {
    target
        .subject
        .as_deref()
        .map(str::trim)
        .filter(|subject| !subject.is_empty())
        .map(ToOwned::to_owned)
}

#[derive(Debug, Clone, PartialEq, Eq)]
enum LocalRuntimeSubjectPolicy {
    Explicit(String),
    DescriptorDefault(String),
}

impl LocalRuntimeSubjectPolicy {
    fn from_target(target: &InvocationTarget, callee_ura: &str) -> Result<Self, String> {
        if let Some(subject) = explicit_subject_ura(target) {
            return Self::checked(subject, "InvocationTarget.subject").map(Self::Explicit);
        }
        let descriptor_default = crate::core::ura::AbilitySelector::parse(&target.ability)
            .ok()
            .filter(|selector| selector.owner_kind() == "hub")
            .map(|selector| selector.ability_ura().to_string())
            .unwrap_or_else(|| callee_ura.to_string());
        Self::checked(descriptor_default, "descriptor default subject").map(Self::DescriptorDefault)
    }

    fn into_subject_identity(self) -> SubjectIdentity {
        match self {
            Self::Explicit(subject) | Self::DescriptorDefault(subject) => local_subject(subject),
        }
    }

    fn checked(value: String, field: &str) -> Result<String, String> {
        let value = value.trim();
        if value.is_empty() {
            return Err(format!("{field} must not be empty"));
        }
        crate::core::ura::parse_ura(value)
            .map_err(|err| format!("{field} is not a valid URA: {err}"))?;
        Ok(value.to_string())
    }
}

fn local_invocation_subject(
    target: &InvocationTarget,
    callee_ura: &str,
) -> Result<SubjectIdentity, String> {
    Ok(LocalRuntimeSubjectPolicy::from_target(target, callee_ura)?.into_subject_identity())
}

fn local_invocation_causal_context(target: &InvocationTarget) -> CausalContext {
    target.causal_context.clone().unwrap_or(CausalContext::None)
}

async fn local_descriptor_bound_envelope(
    runtime: &Arc<LocalRuntime>,
    mode: AxonInvocationCallMode,
    target: &InvocationTarget,
    payload: &[u8],
) -> Result<DescriptorBoundEnvelope, String> {
    let callee_ura = local_invocation_callee_ura(target);
    let subject = local_invocation_subject(target, &callee_ura)?;
    let runtime_ability =
        ability_ura_for_wire(&callee_ura, &target.ability).map_err(|err| format!("{err}"))?;
    let descriptor_version = registered_descriptor_version(runtime, &runtime_ability, mode)
        .await
        .map_err(|err| err.message().to_string())?;
    let ability =
        ability_descriptor_ref_for_wire(&callee_ura, &target.ability, &descriptor_version)
            .map_err(|err| format!("{err}"))?;
    DescriptorBoundEnvelope::from_parts(DescriptorBoundEnvelopeParts {
        caller: system_agent_identity(),
        callee: local_identity(&callee_ura),
        ability,
        subject,
        invocation_nonce: fresh_nonce(),
        causal_context: local_invocation_causal_context(target),
        args_bytes: payload,
    })
    .map_err(|err| format!("{err}"))
}

async fn local_system_request(
    runtime: &Arc<LocalRuntime>,
    mode: AxonInvocationCallMode,
    target: &InvocationTarget,
    payload: Vec<u8>,
) -> Result<DescriptorBoundInvocationRequest, String> {
    let envelope = local_descriptor_bound_envelope(runtime, mode, target, &payload).await?;
    #[cfg(feature = "axon-pb")]
    let options = LocalRuntimeRequestOptions::default()
        .with_request_metadata(target.request_metadata.clone());
    #[cfg(not(feature = "axon-pb"))]
    let options = {
        let _ = &target.request_metadata;
        LocalRuntimeRequestOptions::default()
    };

    LocalRuntimeRequestFactory::request_for(
        mode,
        LocalRuntimeIngress::LocalSystem { envelope, payload },
        options,
    )
    .map_err(|err| format!("{err}"))
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
/// every consumer (`ability_dispatch`, `daemon/control/runtime_dispatch`,
/// `daemon/control/runtime_dispatch_adapter`) classifies "not found" through
/// the same predicate. If a future SDK version rephrases its
/// reason, this is the single grep target.
pub const NOT_FOUND_REASON_FRAGMENTS: &[&str] = &[
    "unknown_ability",
    "no local handler registered",
    "no local stream handler registered",
    "no local bidi handler registered",
    "not registered in Axon LocalRuntime",
    // Control-plane lookup miss: the dispatch never found a record to
    // bind. Recognised here (the single classifier) rather than by
    // editing each producing call site.
    "is not registered in the control plane",
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
    let request =
        local_system_request(&runtime, AxonInvocationCallMode::Stream, &target, payload).await?;
    let (handle, _) = runtime
        .invoke_descriptor_bound_stream_request_async(request)
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
    let request =
        local_system_request(&runtime, AxonInvocationCallMode::Bidi, &target, payload).await?;
    let (handle, _) = runtime
        .invoke_descriptor_bound_bidi_request_async(request)
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
    let request =
        local_system_request(&runtime, AxonInvocationCallMode::Rpc, &target, payload).await?;
    let (handle, _) = runtime
        .invoke_descriptor_bound_request_async(request)
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

#[cfg(test)]
mod tests {
    use super::*;
    use easynet_axon::invocation::{make_ability, AbilityCallModes, AbilityOptions};

    use crate::daemon::invocation::routing::target::CallMode;
    use serde_json::json;

    const TEST_DESCRIPTOR_VERSION: &str = "1.0.0";
    const TEST_SCHEMA_HASH: [u8; 32] = [0x11; 32];
    const TEST_IMPL_HASH: [u8; 32] = [0x22; 32];

    fn target(ability: String, subject: Option<String>) -> InvocationTarget {
        InvocationTarget {
            scope: TargetScope::Local,
            ability,
            normalized_args: json!({}),
            call_mode: CallMode::Rpc,
            subject,
            causal_context: None,
            request_metadata: std::collections::HashMap::new(),
        }
    }

    async fn runtime_with_descriptor_bound_ability(
        callee_ura: &str,
        ability: &str,
    ) -> Arc<LocalRuntime> {
        let runtime = LocalRuntime::new();
        let runtime_ability =
            ability_ura_for_wire(callee_ura, ability).expect("runtime ability URA");
        runtime
            .register_ability_with_options(
                runtime_ability,
                make_ability(|ctx| async move { Ok(ctx.payload.clone()) }),
                AbilityOptions::default()
                    .with_modes(AbilityCallModes::RPC)
                    .with_descriptor_proof(
                        TEST_DESCRIPTOR_VERSION,
                        TEST_SCHEMA_HASH,
                        TEST_IMPL_HASH,
                    ),
            )
            .await
            .expect("register descriptor-bound test ability");
        runtime
    }

    #[tokio::test]
    async fn canonical_ability_ura_projects_owner_as_callee() {
        let owner = crate::core::ura::device_ura("acme", "dev-a");
        let ability = crate::core::ura::owner_ability_ura(&owner, "fs.read").unwrap();
        let runtime = runtime_with_descriptor_bound_ability(&owner, &ability).await;
        let envelope = local_descriptor_bound_envelope(
            &runtime,
            AxonInvocationCallMode::Rpc,
            &target(ability.clone(), None),
            b"{}",
        )
        .await
        .expect("descriptor-bound envelope");

        assert_eq!(envelope.envelope().callee.ura, owner);
        assert_eq!(envelope.envelope().subject.ura, owner);
        assert_eq!(
            envelope.envelope().ability,
            format!("{ability}@{TEST_DESCRIPTOR_VERSION}")
        );
    }

    #[tokio::test]
    async fn resource_subject_does_not_become_callee() {
        let subject =
            crate::core::ura::resource_dot_ura("acme", "device.dev-a.files", "tmp/report.txt");
        let runtime = runtime_with_descriptor_bound_ability(&local_device_ura(), "fs.read").await;
        let envelope = local_descriptor_bound_envelope(
            &runtime,
            AxonInvocationCallMode::Rpc,
            &target("fs.read".to_string(), Some(subject.clone())),
            b"{}",
        )
        .await
        .expect("descriptor-bound envelope");

        assert_eq!(envelope.envelope().callee.ura, local_device_ura());
        assert_eq!(envelope.envelope().subject.ura, subject);
        assert_eq!(
            envelope.envelope().ability,
            format!(
                "{}@{TEST_DESCRIPTOR_VERSION}",
                crate::core::ura::owner_ability_ura(&local_device_ura(), "fs.read").unwrap()
            )
        );
    }

    #[tokio::test]
    async fn explicit_device_subject_is_not_reclassified_as_callee() {
        let subject = crate::core::ura::device_ura("acme", "dev-b");
        let runtime =
            runtime_with_descriptor_bound_ability(&local_device_ura(), "device.inspect").await;
        let envelope = local_descriptor_bound_envelope(
            &runtime,
            AxonInvocationCallMode::Rpc,
            &target("device.inspect".to_string(), Some(subject.clone())),
            b"{}",
        )
        .await
        .expect("descriptor-bound envelope");

        assert_eq!(envelope.envelope().callee.ura, local_device_ura());
        assert_eq!(envelope.envelope().subject.ura, subject);
        assert_eq!(
            envelope.envelope().ability,
            format!(
                "{}@{TEST_DESCRIPTOR_VERSION}",
                crate::core::ura::owner_ability_ura(&local_device_ura(), "device.inspect").unwrap()
            )
        );
    }
}
