// EasyNet CLI — daemon LocalRuntime invocation adapter
// ====================================================
//
// Small daemon-side adapter for JSON ability payloads crossing into
// Axon's embedded LocalRuntime. Callers stay at the EasyNet
// `InvocationTarget` / `serde_json::Value` layer; this module owns
// the byte payload, verified RPC terminal projection, stream frame, and bidi split
// mechanics required by the Axon SDK. It is not an external agent
// runtime adapter and does not own handler bodies.

use std::sync::Arc;

use axon_sdk::invocation::{
    AbilityFrame, BidiInputSender, BidiOutputReceiver, CallMode as AxonInvocationCallMode,
    CausalContext, DescriptorBoundInvocationRequest, InvocationHandle, InvocationState,
    LocalRuntime, StreamingInvocationHandle,
};
use serde_json::Value;

use crate::daemon::axon_bridge::descriptor_ref::{
    ability_descriptor_ref_for_wire, ability_ura_for_wire, registered_descriptor_binding,
};
use crate::daemon::axon_bridge::local_runtime_request::{
    LocalRuntimeRequestOptions, SystemInvocationIssuer,
};
use crate::daemon::identity::local_invocation::local_device_ura;
use crate::daemon::invocation::routing::target::{
    InvocationCausalContext, InvocationTarget, TargetScope,
};

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

fn local_invocation_callee_ura(target: &InvocationTarget) -> Result<String, String> {
    if let Ok(selector) = crate::core::ura::AbilitySelector::parse(&target.ability) {
        return Ok(selector.owner_ura().to_string());
    }

    match &target.causal_context {
        InvocationCausalContext::DaemonSystemRoot => {
            local_device_ura().map_err(|err| err.to_string())
        }
        InvocationCausalContext::Explicit(_) => Err(format!(
            "public LocalRuntime invocation requires a canonical Ability URA; \
             bare ability `{}` cannot infer callee ownership",
            target.ability
        )),
    }
}

#[derive(Debug, Clone)]
struct LocalRuntimeInvocationPolicy {
    subject_ura: String,
    causal_context: CausalContext,
}

impl LocalRuntimeInvocationPolicy {
    fn from_target(target: &InvocationTarget, callee_ura: &str) -> Result<Self, String> {
        Ok(Self {
            subject_ura: target
                .resolved_subject_ura(callee_ura)
                .map_err(|err| err.to_string())?,
            causal_context: target.resolved_causal_context(),
        })
    }
}

async fn local_system_request(
    runtime: &Arc<LocalRuntime>,
    mode: AxonInvocationCallMode,
    target: &InvocationTarget,
    payload: Vec<u8>,
) -> Result<DescriptorBoundInvocationRequest, String> {
    let callee_ura = local_invocation_callee_ura(target)?;
    let invocation_policy = LocalRuntimeInvocationPolicy::from_target(target, &callee_ura)?;
    let runtime_ability =
        ability_ura_for_wire(&callee_ura, &target.ability).map_err(|err| format!("{err}"))?;
    let descriptor_binding = registered_descriptor_binding(runtime, &runtime_ability, mode)
        .await
        .map_err(|err| err.message().to_string())?;
    let ability_descriptor_ref =
        ability_descriptor_ref_for_wire(&callee_ura, &target.ability, &descriptor_binding)
            .map_err(|err| format!("{err}"))?;
    #[cfg(feature = "axon-pb")]
    let options = LocalRuntimeRequestOptions::default()
        .with_request_metadata(target.request_metadata.clone());
    #[cfg(not(feature = "axon-pb"))]
    let options = {
        let _ = &target.request_metadata;
        LocalRuntimeRequestOptions::default()
    };

    SystemInvocationIssuer::request_for_descriptor_ref(
        mode,
        &callee_ura,
        ability_descriptor_ref,
        invocation_policy.subject_ura.as_str(),
        payload,
        invocation_policy.causal_context,
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

/// Reason-string fragments that Axon runtime bindings may produce when an
/// ability is unknown. Canonical Invocation dispatch classifies all local
/// misses through this predicate; it is the single update point if the SDK
/// changes its diagnostic vocabulary.
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
/// `axon_sdk::AxonError::reason`. A typed predicate over
/// `&AxonError` is a follow-up that depends on the SDK growing a
/// `NotFound` variant in its 7-class taxonomy.
pub fn is_not_found_error(msg: &str) -> bool {
    NOT_FOUND_REASON_FRAGMENTS
        .iter()
        .any(|fragment| msg.contains(fragment))
}

// `block_on_runtime` used to live here as a one-line wrapper around
// `crate::support::async_bridge::run_blocking` pinned to the
// `BuildCurrentThreadTokio` policy. The wrapper hid the policy
// choice from the call site, and a second helper with a generic
// name made `git grep block_on` return adjacent-but-different
// shapes. Per the 2026-05-29 industrial-textbook review, every
// call site that drives a LocalRuntime future from sync code now
// reaches for `support::async_bridge::run_blocking(future,
// SyncBridgeRuntimePolicy::BuildCurrentThreadTokio)` directly. The
// policy choice is non-obvious enough (`UseFuturesExecutor` deadlocks
// against tokio resources) that exposing it at the call site is the
// honest shape.

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
        crate::support::async_bridge::SyncBridgeRuntimePolicy::BuildCurrentThreadTokio,
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
    let finalized = handle
        .finalized()
        .await
        .map_err(|error| format!("finalize local Axon invocation: {error}"))?;
    match finalized.terminal_state {
        InvocationState::Completed => decode_json_payload(finalized.output()),
        InvocationState::Failed | InvocationState::TimedOut | InvocationState::Cancelled => {
            Err(finalized
                .failure
                .map(|error| error.to_string())
                .unwrap_or_else(|| {
                    format!(
                        "Axon invocation ended as {}",
                        finalized.terminal_state.as_str()
                    )
                }))
        }
        other => Err(format!(
            "Axon finalization returned non-terminal state {}",
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
    use axon_sdk::invocation::{make_ability, AbilityCallModes, AbilityOptions};

    use crate::daemon::invocation::routing::target::CallMode;
    use serde_json::json;

    const TEST_DESCRIPTOR_VERSION: &str = "1.0.0";
    const TEST_DESCRIPTOR_HASH: [u8; 32] = [0x33; 32];
    const TEST_SCHEMA_HASH: [u8; 32] = [0x11; 32];
    const TEST_IMPL_HASH: [u8; 32] = [0x22; 32];

    fn expected_descriptor_ref(ability_ura: &str) -> String {
        format!(
            "{ability_ura}@{TEST_DESCRIPTOR_VERSION}#{}!invoke",
            hex::encode(TEST_DESCRIPTOR_HASH)
        )
    }

    fn target(ability: String, subject: Option<String>) -> InvocationTarget {
        target_with_args(ability, subject, json!({}))
    }

    fn target_with_args(
        ability: String,
        subject: Option<String>,
        normalized_args: Value,
    ) -> InvocationTarget {
        if let Some(subject) = subject {
            crate::daemon::invocation::routing::target::SystemInvocationTargetIssuer::local_root_for_subject(
                ability,
                normalized_args,
                CallMode::Rpc,
                subject,
            )
        } else {
            crate::daemon::invocation::routing::target::SystemInvocationTargetIssuer::local_root(
                ability,
                normalized_args,
                CallMode::Rpc,
            )
        }
    }

    fn provision_test_local_device_ura() -> (crate::cli::commands::test_support::HomeGuard, String)
    {
        let home = crate::cli::commands::test_support::HomeGuard::new();
        crate::daemon::persistence::config::save_credentials(
            &crate::daemon::persistence::config::Credentials {
                node_id: "dev-a".to_string(),
                credential_token: "token".to_string(),
                hub_endpoint: "axon://hub.example:50051".to_string(),
                realm: "acme".to_string(),
                username: Some("alice".to_string()),
                user_id: Some("user-alice".to_string()),
                ..Default::default()
            },
        )
        .expect("write local device credentials");
        let local_device_ura = local_device_ura().expect("credentials-backed local device URA");
        (home, local_device_ura)
    }

    async fn runtime_with_descriptor_bound_ability(
        callee_ura: &str,
        ability: &str,
    ) -> Arc<LocalRuntime> {
        let runtime = crate::daemon::axon_bridge::runtime_factory::build_local_runtime(
            crate::daemon::axon_bridge::runtime_factory::rejecting_test_key_resolver(),
            None,
        );
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
                        "invoke",
                        TEST_DESCRIPTOR_HASH,
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
        let request = local_system_request(
            &runtime,
            AxonInvocationCallMode::Rpc,
            &target(ability.clone(), None),
            b"{}".to_vec(),
        )
        .await
        .expect("descriptor-bound request");

        assert_eq!(request.envelope().envelope().callee.ura, owner);
        assert_eq!(request.envelope().envelope().subject.ura, owner);
        assert_eq!(
            request.envelope().envelope().ability,
            expected_descriptor_ref(&ability)
        );
    }

    #[tokio::test]
    async fn resource_subject_does_not_become_callee() {
        let (_home, local_device_ura) = provision_test_local_device_ura();
        let subject =
            crate::core::ura::resource_dot_ura("acme", "device.dev-a.files", "tmp/report.txt");
        let runtime = runtime_with_descriptor_bound_ability(&local_device_ura, "fs.read").await;
        let request = local_system_request(
            &runtime,
            AxonInvocationCallMode::Rpc,
            &target("fs.read".to_string(), Some(subject.clone())),
            b"{}".to_vec(),
        )
        .await
        .expect("descriptor-bound request");

        assert_eq!(request.envelope().envelope().callee.ura, local_device_ura);
        assert_eq!(request.envelope().envelope().subject.ura, subject);
        assert_eq!(
            request.envelope().envelope().ability,
            expected_descriptor_ref(
                &crate::core::ura::owner_ability_ura(&local_device_ura, "fs.read").unwrap()
            )
        );
    }

    #[tokio::test]
    async fn explicit_device_subject_is_not_reclassified_as_callee() {
        let (_home, local_device_ura) = provision_test_local_device_ura();
        let subject = crate::core::ura::device_ura("acme", "dev-b");
        let runtime =
            runtime_with_descriptor_bound_ability(&local_device_ura, "device.inspect").await;
        let request = local_system_request(
            &runtime,
            AxonInvocationCallMode::Rpc,
            &target("device.inspect".to_string(), Some(subject.clone())),
            b"{}".to_vec(),
        )
        .await
        .expect("descriptor-bound request");

        assert_eq!(request.envelope().envelope().callee.ura, local_device_ura);
        assert_eq!(request.envelope().envelope().subject.ura, subject);
        assert_eq!(
            request.envelope().envelope().ability,
            expected_descriptor_ref(
                &crate::core::ura::owner_ability_ura(&local_device_ura, "device.inspect").unwrap()
            )
        );
    }

    #[tokio::test]
    async fn public_explicit_tuple_rejects_bare_ability_before_local_device_callee_fallback() {
        let runtime = crate::daemon::axon_bridge::runtime_factory::build_local_runtime(
            crate::daemon::axon_bridge::runtime_factory::rejecting_test_key_resolver(),
            None,
        );
        let subject =
            crate::core::ura::resource_dot_ura("acme", "device.dev-a.files", "tmp/report.txt");
        let target = InvocationTarget::local_explicit_tuple(
            "fs.read",
            json!({}),
            CallMode::Rpc,
            subject,
            CausalContext::None,
        );

        let error = match local_system_request(
            &runtime,
            AxonInvocationCallMode::Rpc,
            &target,
            b"{}".to_vec(),
        )
        .await
        {
            Ok(_) => panic!("public bare ability must not infer local-device callee"),
            Err(error) => error,
        };

        assert!(
            error.contains("public LocalRuntime invocation requires a canonical Ability URA"),
            "unexpected error: {error}"
        );
        assert!(
            !error.contains("credentials-backed local device URA"),
            "public ingress must fail before local device fallback: {error}"
        );
    }

    #[tokio::test]
    async fn local_rpc_projects_finalized_output() {
        let (_home, local_device_ura) = provision_test_local_device_ura();
        let ability = "device.inspect".to_string();
        let runtime = runtime_with_descriptor_bound_ability(&local_device_ura, &ability).await;
        let args = json!({"ok": true, "source": "finalized"});

        let output = invoke_local_rpc(runtime, target_with_args(ability, None, args.clone()))
            .await
            .expect("local RPC output");

        assert_eq!(output, args);
    }
}
