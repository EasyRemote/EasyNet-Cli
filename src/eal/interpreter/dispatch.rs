// Step dispatch plane: agent-aware dispatcher, daemon and
// remote canonical_invoke routes (split from interpreter.rs,
// T4.4 / F-021; bodies are move-only).

// EasyNet CLI — EAL Interpreter
// =============================
//
// File: src/eal/interpreter.rs
// Description: Daemon-owned execution engine for Mission IR v2.
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
use crate::daemon::execution::mission::invocation_gateway::{
    MissionInvocationGateway, MissionInvocationRequest, MissionReceiptReference,
};
use crate::daemon::persistence::agent_aggregate::AgentAggregateRepository;

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
pub(crate) struct AgentAwareDispatcher {
    registry: Arc<crate::daemon::persistence::agent_registry::AgentRegistry>,
    gateway: Arc<dyn MissionInvocationGateway>,
}

impl AgentAwareDispatcher {
    pub(crate) fn new(gateway: Arc<dyn MissionInvocationGateway>) -> Self {
        let registry = load_registry_or_warn();
        Self {
            registry: Arc::new(registry),
            gateway,
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
        dependency_receipts: &[MissionReceiptReference],
    ) -> Result<StepDispatchOutcome, EalError> {
        let (request, target_timeout) = match target {
            IrTarget::Agent(agent_id) => {
                let manifest_timeout =
                    validate_agent_target(&self.registry, agent_id, ability)?.timeout_seconds();
                (
                    MissionInvocationRequest::hosted_agent(
                        agent_id.name.clone(),
                        ability.as_str(),
                        arguments.clone(),
                    ),
                    manifest_timeout.map(std::time::Duration::from_secs),
                )
            }
            IrTarget::Device { node_id } => (
                device_request(run.tenant, node_id, ability.as_str(), arguments.clone())?,
                None,
            ),
        };
        let request = request
            .with_dispatch_timeout(
                timeout_ms
                    .map(std::time::Duration::from_millis)
                    .or(target_timeout)
                    .unwrap_or_else(|| std::time::Duration::from_secs(30)),
            )
            .with_dependency_receipts(dependency_receipts.to_vec())
            .with_trace_id(run.trace_id);
        let outcome = self
            .gateway
            .invoke(request)
            .map_err(|error| EalError::Unavailable(format!("Mission child dispatch: {error:#}")))?;
        Ok(StepDispatchOutcome {
            value: outcome.value,
            invocation: outcome.invocation,
        })
    }

    fn clone_for_thread(&self) -> Result<Box<dyn StepDispatcher + Send>, EalError> {
        Ok(Box::new(AgentAwareDispatcher {
            registry: Arc::clone(&self.registry),
            gateway: Arc::clone(&self.gateway),
        }))
    }
}

fn device_request(
    tenant: &str,
    node_id: &str,
    ability: &str,
    arguments: Value,
) -> Result<MissionInvocationRequest, EalError> {
    let node_id = node_id.trim();
    let local_node = crate::daemon::persistence::config::load_credentials()
        .ok()
        .map(|credentials| credentials.node_id);
    if node_id.is_empty()
        || node_id.eq_ignore_ascii_case("local")
        || local_node.as_deref() == Some(node_id)
    {
        return Ok(MissionInvocationRequest::system(ability, arguments));
    }
    let target = if crate::core::ura::parse_ura(node_id).is_ok() {
        node_id.to_string()
    } else if !tenant.trim().is_empty() {
        crate::core::ura::device_ura(tenant.trim(), node_id)
    } else {
        return Err(EalError::Validation(format!(
            "cannot resolve EAL device target {node_id:?}: no tenant in scope"
        )));
    };
    MissionInvocationRequest::remote_node(target, ability, arguments)
        .map_err(|error| EalError::Validation(format!("parse device target: {error}")))
}

fn validate_agent_target(
    registry: &crate::daemon::persistence::agent_registry::AgentRegistry,
    agent_id: &crate::core::agent::id::AgentId,
    ability: &AbilityName,
) -> Result<crate::daemon::ability::manifest::AbilityManifest, EalError> {
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
    let manifest = crate::daemon::execution::mission::agent_ability_specs::manifests_for(
        &agent_id.name,
        entry,
    )
    .into_iter()
    .find(|manifest| manifest.name() == bare_ability)
    .ok_or_else(|| {
        EalError::NotFound(format!("unknown ability: {}.{bare_ability}", agent_id.name))
    })?;
    Ok(manifest)
}
