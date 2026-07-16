// Step dispatch plane: agent-aware dispatcher, daemon and
// remote canonical_invoke routes (split from interpreter.rs,
// T4.4 / F-021; bodies are move-only).

// EasyNet CLI — EAL Interpreter
// =============================
//
// File: src/eal/interpreter.rs
// Description: Client-side execution engine for Mission IR v2 (temporary — target: MissionControl v2).
//
// Execution Model:
//   Phases execute sequentially (data-flow barriers between them).
//   Steps within a phase execute in parallel via rayon work-stealing threadpool.
//   When a dispatcher cannot be cloned across worker threads, falls back to sequential.
//
// Core Capabilities:
//   1. True parallel dispatch — rayon::scope + clone_for_thread() per step.
//   2. Structured ExecutionTrace — per-step audit log with timestamps, result hashes, retry history.
//   3. Retry with exponential backoff — delay = min(base * 2^attempt, max) + deterministic jitter.
//   4. Cross-phase data flow — results captured in HashMap, substituted into downstream input_refs.
//   5. Lock-free result collection — crossbeam SegQueue eliminates collector contention.
//   6. Connection pool reuse — BridgePool with adaptive sizing based on CPU cores.
//
// Dispatch Abstraction:
//   `trait StepDispatcher` decouples execution from transport. Production uses
//   AgentAwareDispatcher; tests inject MockDispatcher or a non-cloneable dispatcher.
//
// Author: Silan Hu <silan.hu@u.nus.edu>
// Copyright (c) 2026 EasyNet. All rights reserved.

use std::sync::Arc;

use serde_json::Value;

use super::*;
use crate::daemon::persistence::agent_aggregate::AgentAggregateRepository;

/// without changing the EAL surface.
fn dispatch_remote_via_canonical_invoke(
    tenant: &str,
    node_id: &str,
    ability_name: &str,
    arguments: &Value,
    causal_parents: &[Value],
    timeout_ms: Option<u64>,
) -> Result<Value, EalError> {
    #[cfg(feature = "axon-pb")]
    {
        let trimmed = node_id.trim();

        // Local short-circuit: `local`, empty, or this device's own
        // node id all dispatch through the local daemon's control
        // socket, the same surface every other in-process invocation
        // uses. Skip the canonical_invoke envelope entirely — the
        // self-target shortcut on the daemon side covers a different
        // case (canonical self URA), not the keyword `local`.
        let self_node = crate::daemon::persistence::config::load_credentials()
            .ok()
            .map(|c| c.node_id);
        let is_local = trimmed.is_empty()
            || trimmed.eq_ignore_ascii_case("local")
            || self_node
                .as_deref()
                .is_some_and(|n| !n.is_empty() && trimmed == n);
        if is_local {
            let timeout = timeout_ms
                .map(std::time::Duration::from_millis)
                .unwrap_or_else(|| std::time::Duration::from_secs(30));
            return dispatch_local_device_ability(ability_name, arguments, timeout);
        }

        let target_ura = if crate::core::ura::parse_ura(trimmed).is_ok() {
            crate::daemon::invocation::routing::remote_invoke::parse_node_ura(trimmed)
                .map_err(|e| EalError::Validation(format!("parse target URA: {e}")))?
        } else if !tenant.is_empty() {
            crate::core::ura::device_ura(tenant, trimmed)
        } else {
            return Err(EalError::Validation(format!(
                "cannot resolve EAL device target {trimmed:?}: no tenant in scope; \
                 pass a canonical `easynet:///r/<realm>/device/<id>` URA"
            )));
        };

        let caller_ura = crate::daemon::persistence::config::load_credentials()
            .ok()
            .filter(|c| !c.realm.trim().is_empty() && !c.node_id.trim().is_empty())
            .map(|c| crate::core::ura::device_ura(c.realm.trim(), c.node_id.trim()));
        let target_call = crate::daemon::invocation::routing::remote_invoke::RemoteAbilityInvocationTarget::for_target_owned_selector(
            &target_ura,
            ability_name,
        )
        .map_err(|e| EalError::Validation(format!("derive Ability URA for {ability_name}: {e}")))?;
        crate::daemon::invocation::routing::remote_invoke::invoke_remote_target_with_causal_parents(
            &target_call,
            arguments.clone(),
            caller_ura.as_deref(),
            causal_parents,
        )
        .map_err(|e| {
            EalError::Unavailable(format!(
                "canonical_invoke {ability_name} → {target_ura}: {e}"
            ))
        })
    }
    #[cfg(not(feature = "axon-pb"))]
    {
        let _ = (
            tenant,
            node_id,
            ability_name,
            arguments,
            causal_parents,
            timeout_ms,
        );
        Err(EalError::Unavailable(
            "EAL device-targeted dispatch requires the `axon-pb` feature; \
             rebuild with `--features axon-pb` (production builds always do)."
                .to_string(),
        ))
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum LocalDeviceDispatchMode {
    Rpc,
    StreamFirstPayload,
    BidiUnsupported,
}

fn dispatch_local_device_ability(
    ability_name: &str,
    arguments: &Value,
    timeout: std::time::Duration,
) -> Result<Value, EalError> {
    match local_device_dispatch_mode(ability_name) {
        LocalDeviceDispatchMode::Rpc => {
            crate::support::platform::local_invoke::invoke_local_ability_with_subject_timeout(
                ability_name,
                arguments.clone(),
                None,
                timeout,
            )
            .map_err(|e| {
                EalError::Unavailable(format!("invoke_local_ability {ability_name} (local): {e}"))
            })
        }
        LocalDeviceDispatchMode::StreamFirstPayload => {
            crate::support::platform::local_invoke::invoke_local_stream_ability_first_payload(
                ability_name,
                arguments.clone(),
                None,
                timeout,
            )
            .map_err(|e| {
                EalError::Unavailable(format!(
                    "invoke_local_stream_ability {ability_name} (local): {e}"
                ))
            })
        }
        LocalDeviceDispatchMode::BidiUnsupported => Err(EalError::Validation(format!(
            "local ability `{ability_name}` is bidirectional; EAL scalar call steps cannot open \
             InvokeBidi sessions"
        ))),
    }
}

fn local_device_dispatch_mode(ability_name: &str) -> LocalDeviceDispatchMode {
    crate::daemon::ability::catalog::published_abilities()
        .into_iter()
        .find(|descriptor| descriptor.name == ability_name)
        .map(|descriptor| dispatch_mode_from_call_mode(descriptor.call_mode()))
        .unwrap_or(LocalDeviceDispatchMode::Rpc)
}

fn dispatch_mode_from_call_mode(
    call_mode: crate::daemon::ability::CallMode,
) -> LocalDeviceDispatchMode {
    match call_mode {
        crate::daemon::ability::CallMode::Rpc => LocalDeviceDispatchMode::Rpc,
        crate::daemon::ability::CallMode::Stream => LocalDeviceDispatchMode::StreamFirstPayload,
        crate::daemon::ability::CallMode::Bidi => LocalDeviceDispatchMode::BidiUnsupported,
    }
}

/// Per-mission-run dispatch context.
///
/// `trace_id` is the mission run's id (minted once per run in
/// `execute_with_dispatcher`, equal to `ExecutionTrace::mission_id`).
/// Dispatchers that lower steps onto the Axon Invocation surface stamp
/// it on every envelope's `trace_id` operational-metadata field, so the
/// daemon ledger groups one mission run's invocations and
/// `easynet invocation trace <mission_id>` reconstructs the run's
/// receipt graph. It is not an Invocation axiom field — causal
/// placement stays in `causal_context`.
pub struct AgentAwareDispatcher {
    registry: Arc<crate::daemon::persistence::agent_registry::AgentRegistry>,
}

impl AgentAwareDispatcher {
    pub fn new(_endpoint: &str, _timeout_ms: u64) -> Self {
        let registry = load_registry_or_warn();
        Self {
            registry: Arc::new(registry),
        }
    }
}

/// Load the agent registry, logging a visible warning if the load fails.
///
/// Previously this was `load_agents().unwrap_or_default()`, which turned
/// "registry file is corrupt / home dir missing / permission denied"
/// into "you have no registered agents", so an EAL member-call like
/// `claude.chat(...)` would fail downstream with `agent '…' not found
/// in registry` — a classic false-negative that sends operators hunting
/// for a mis-registered agent when the real problem is upstream.
///
/// We still want a usable dispatcher when no agents are registered
/// (that is a legitimate first-run state), so we return an empty
/// registry on failure *after* logging. The distinction between
/// "empty by design" and "empty by failure" is preserved in operator-
/// visible logs rather than hidden from the caller.
fn load_registry_or_warn() -> crate::daemon::persistence::agent_registry::AgentRegistry {
    match AgentAggregateRepository::load_snapshot()
        .map(|snapshot| snapshot.registered_agent_registry_projection())
    {
        Ok(registry) => registry,
        Err(e) => {
            eprintln!(
                "[easynet eal] warning: Agent aggregate load failed ({e}); \
                 dispatching with an empty registry. Any agent-target call \
                 will fail with `not_found` until the registry is repaired."
            );
            crate::daemon::persistence::agent_registry::AgentRegistry::default()
        }
    }
}

impl StepDispatcher for AgentAwareDispatcher {
    fn dispatch(
        &self,
        run: RunContext<'_>,
        target: &IrTarget,
        ability: &AbilityName,
        arguments: &Value,
        timeout_ms: Option<u64>,
        causal_parents: &[Value],
    ) -> Result<StepDispatchOutcome, EalError> {
        match target {
            IrTarget::Agent(agent_id) => dispatch_to_agent(
                &self.registry,
                agent_id,
                ability,
                arguments,
                causal_parents,
                run.trace_id,
            ),
            IrTarget::Device { node_id } => {
                // Thread the live causal parents onto the device forward hop
                // too — the sibling agent branch already lowers them, and
                // dropping them here re-rooted the receipt DAG (SPEC §15.1-1).
                dispatch_remote_via_canonical_invoke(
                    run.tenant,
                    node_id,
                    ability.as_str(),
                    arguments,
                    causal_parents,
                    timeout_ms,
                )
                .map(Into::into)
            }
        }
    }

    fn clone_for_thread(&self) -> Result<Box<dyn StepDispatcher + Send>, EalError> {
        Ok(Box::new(AgentAwareDispatcher {
            registry: Arc::clone(&self.registry),
        }))
    }
}

/// Shared agent dispatch logic used by AgentAwareDispatcher.
pub(super) fn dispatch_to_agent(
    registry: &crate::daemon::persistence::agent_registry::AgentRegistry,
    agent_id: &crate::core::agent::id::AgentId,
    ability: &AbilityName,
    arguments: &Value,
    causal_parents: &[Value],
    trace_id: &str,
) -> Result<StepDispatchOutcome, EalError> {
    // Registry is keyed by string today (see Step 4
    // follow-up: registry will be keyed by AgentId itself).
    // For now, look up by the canonical Display form.
    let key = agent_id.to_string();
    let entry = registry
        .agents
        .get(&key)
        .or_else(|| {
            // Backwards-compat: registry files written
            // before the migration may use the bare name
            // form (`"claude"` instead of `"default/claude"`).
            // Fall back to the bare name when the agent
            // is in the default tenant.
            if agent_id.tenant == crate::core::agent::id::DEFAULT_TENANT {
                registry.agents.get(&agent_id.name)
            } else {
                None
            }
        })
        // Missing agent in registry is `not_found`, not `unavailable` —
        // the caller's identifier doesn't resolve and a retry of the
        // same id will not help.
        .ok_or_else(|| EalError::NotFound(format!("agent '{key}' not found in registry")))?;

    let bare_ability = ability.as_str();
    let timeout = crate::daemon::execution::mission::agent_ability_specs::manifests_for(
        &agent_id.name,
        entry,
    )
    .into_iter()
    .find(|manifest| manifest.name() == bare_ability)
    .and_then(|manifest| manifest.timeout_seconds())
    .map(std::time::Duration::from_secs);

    // Every Mission/EAL agent step is one daemon-owned Axon Invocation.
    // The daemon resolves the registered implementation and owns the only
    // execution and terminal-receipt path. Transport or registration
    // failure is terminal here: executing the manifest or chat handler in
    // this process would create a second semantic model and can duplicate
    // side effects after an uncertain transport outcome.
    let display_name = format!("{}.{}", agent_id.name, ability.as_str());
    use crate::support::platform::local_invoke::{LocalInvokeErrorKind, classify_invoke_error};
    match crate::support::platform::local_invoke::invoke_local_ability_with_invocation_meta(
        bare_ability,
        arguments.clone(),
        None,
        causal_parents,
        timeout,
        Some(trace_id),
        Some(&agent_id.name),
    ) {
        Ok((value, meta)) => Ok(StepDispatchOutcome {
            value,
            invocation: Some(meta),
        }),
        Err(err) => match classify_invoke_error(&err) {
            LocalInvokeErrorKind::AbilityUnregistered => Err(EalError::NotFound(format!(
                "unknown ability: {display_name}"
            ))),
            LocalInvokeErrorKind::DaemonOffline | LocalInvokeErrorKind::Failed => Err(
                EalError::Unavailable(format!("daemon invoke {display_name}: {err}")),
            ),
        },
    }
}

#[cfg(test)]
mod tests {
    use super::{LocalDeviceDispatchMode, dispatch_mode_from_call_mode};
    use crate::daemon::ability::CallMode;

    #[test]
    fn local_device_dispatch_mode_is_derived_from_canonical_call_mode() {
        assert_eq!(
            dispatch_mode_from_call_mode(CallMode::Rpc),
            LocalDeviceDispatchMode::Rpc
        );
        assert_eq!(
            dispatch_mode_from_call_mode(CallMode::Stream),
            LocalDeviceDispatchMode::StreamFirstPayload
        );
        assert_eq!(
            dispatch_mode_from_call_mode(CallMode::Bidi),
            LocalDeviceDispatchMode::BidiUnsupported
        );
    }
}
