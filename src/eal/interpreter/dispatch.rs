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
//   Steps within a phase execute under the dispatcher's declared concurrency policy.
//
// Core Capabilities:
//   1. Declared parallel dispatch — rayon::scope + clone_for_thread() per step.
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

use anyhow::Context as _;
use serde_json::Value;

use super::*;
use crate::daemon::execution::child_invocation::ChildInvocationReceiptAnchor;
use crate::daemon::execution::mission::invocation_gateway::{
    MissionInvocationGateway, MissionInvocationRequest,
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
    pub(crate) fn new(gateway: Arc<dyn MissionInvocationGateway>) -> anyhow::Result<Self> {
        let registry = load_registry_projection_for_dispatch()?;
        Ok(Self {
            registry: Arc::new(registry),
            gateway,
        })
    }
}

/// Load the exact Agent registry projection required for EAL dispatch.
///
/// A missing registry file is still a valid first-run empty state because
/// `agent_registry::load_agents()` owns that persistence rule. Any unreadable
/// or malformed registry is unavailable runtime state and must fail before
/// child Invocation planning.
///
/// EAL does not need hosted-Agent identity state to validate an Agent target,
/// so this deliberately reads only the registry projection instead of the
/// broader aggregate snapshot.
fn load_registry_projection_for_dispatch(
) -> anyhow::Result<crate::daemon::persistence::agent_registry::AgentRegistry> {
    AgentAggregateRepository::load_registered_agent_registry_projection()
        .map_err(|error| error.into_source_or_self())
        .context("load Agent registry projection for EAL dispatch")
}

impl StepDispatcher for AgentAwareDispatcher {
    fn dispatch(
        &self,
        run: RunContext<'_>,
        target: &IrTarget,
        ability: &AbilityName,
        arguments: &Value,
        timeout_ms: Option<u64>,
        dependency_receipts: &[ChildInvocationReceiptAnchor],
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

    fn dispatch_concurrency(&self) -> StepDispatchConcurrency {
        StepDispatchConcurrency::Parallel
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
    if node_id.is_empty() || node_id.eq_ignore_ascii_case("local") {
        return Ok(MissionInvocationRequest::system(ability, arguments));
    }
    let local_identity = EalLocalNodeIdentity::load();
    if local_identity.matches_node(node_id)? {
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

#[derive(Debug, Clone, PartialEq, Eq)]
enum EalLocalNodeIdentity {
    Known(String),
    Unpaired,
    Unavailable { reason: String },
}

impl EalLocalNodeIdentity {
    fn load() -> Self {
        match crate::daemon::persistence::config::load_credentials_optional() {
            Ok(Some(credentials)) => {
                let node_id = credentials.node_id.trim();
                if node_id.is_empty() {
                    return Self::Unavailable {
                        reason:
                            "EAL device target resolution requires non-empty local node identity"
                                .to_string(),
                    };
                }
                Self::Known(node_id.to_string())
            }
            Ok(None) => Self::Unpaired,
            Err(error) => Self::Unavailable {
                reason: format!(
                    "load local credentials for EAL device target resolution: {error:#}"
                ),
            },
        }
    }

    fn matches_node(&self, node_id: &str) -> Result<bool, EalError> {
        match self {
            Self::Known(local_node_id) => Ok(local_node_id == node_id),
            Self::Unpaired => Err(EalError::Unavailable(
                "EAL device target resolution requires paired local credentials before remote device dispatch".to_string(),
            )),
            Self::Unavailable { reason } => Err(EalError::Unavailable(reason.clone())),
        }
    }
}

fn validate_agent_target(
    registry: &crate::daemon::persistence::agent_registry::AgentRegistry,
    agent_id: &crate::core::agent::id::AgentId,
    ability: &AbilityName,
) -> Result<crate::daemon::ability::manifest::AbilityManifest, EalError> {
    // Registry lookup is canonical-only: AgentId::Display emits the
    // full `tenant/name` key used by current registry rows. Bare
    // default-tenant keys are retired local state; callers must
    // migrate/re-publish the registry instead of dispatching through a
    // compatibility alias.
    let key = agent_id.to_string();
    let entry = registry
        .agents
        .get(&key)
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

#[cfg(test)]
mod tests {
    use super::*;
    use crate::core::agent::id::{AbilityName, AgentId};
    use crate::daemon::execution::mission::invocation_gateway::MissionInvocationTarget;
    use crate::daemon::persistence::agent_registry::{AgentEntry, AgentRegistry, AgentType};
    use crate::daemon::persistence::config;
    use serde_json::json;

    struct UnusedMissionGateway;

    impl MissionInvocationGateway for UnusedMissionGateway {
        fn invoke(
            &self,
            _request: MissionInvocationRequest,
        ) -> anyhow::Result<crate::daemon::execution::child_invocation::ChildInvocationOutcome>
        {
            panic!("registry-load tests must not invoke the Mission gateway");
        }
    }

    fn claude_chat_target() -> (AgentId, AbilityName) {
        (
            AgentId::parse("claude").expect("valid shorthand agent id"),
            AbilityName::parse("chat").expect("valid ability name"),
        )
    }

    #[test]
    fn validate_agent_target_rejects_bare_default_registry_key() {
        let (agent_id, ability) = claude_chat_target();
        let mut registry = AgentRegistry::default();
        registry.agents.insert(
            "claude".to_string(),
            AgentEntry::new(AgentType::ClaudeCode, None),
        );

        let error = validate_agent_target(&registry, &agent_id, &ability).unwrap_err();

        assert_eq!(error.error_code(), "not_found");
        assert!(
            error
                .message()
                .contains("agent 'default/claude' not found in registry"),
            "unexpected error: {error}"
        );
    }

    #[test]
    fn validate_agent_target_uses_canonical_registry_key_only() {
        let (agent_id, ability) = claude_chat_target();
        let mut registry = AgentRegistry::default();
        registry.agents.insert(
            "default/claude".to_string(),
            AgentEntry::new(AgentType::ClaudeCode, None),
        );

        let error = validate_agent_target(&registry, &agent_id, &ability).unwrap_err();

        assert_eq!(error.error_code(), "not_found");
        assert!(
            error.message().contains("unknown ability: claude.chat"),
            "canonical row was not used; unexpected error: {error}"
        );
    }

    #[test]
    fn device_request_rejects_malformed_credentials_before_remote_guess() {
        let _home = crate::cli::commands::test_support::HomeGuard::new();
        std::fs::create_dir_all(config::state_dir()).expect("create state dir");
        std::fs::write(config::state_dir().join("credentials.json"), b"{")
            .expect("write malformed credentials");

        let error = device_request("acme", "node-b", "observe.health", json!({}))
            .expect_err("malformed credentials must not collapse to remote target guessing");

        assert_eq!(error.error_code(), "unavailable");
        assert!(
            error
                .message()
                .contains("load local credentials for EAL device target resolution"),
            "unexpected error: {error}"
        );
        assert!(
            error.message().contains("parse credentials"),
            "malformed credentials must surface parse failure: {error}"
        );
    }

    #[test]
    fn device_request_rejects_unpaired_credentials_before_remote_guess() {
        let _home = crate::cli::commands::test_support::HomeGuard::new();

        let request = device_request("acme", "node-b", "observe.health", json!({}))
            .expect_err("unpaired credentials must not synthesize a remote device target");

        assert_eq!(request.error_code(), "unavailable");
        assert!(
            request
                .message()
                .contains("requires paired local credentials"),
            "unexpected error: {request}"
        );
    }

    #[test]
    fn device_request_resolves_known_local_node_to_system_target() {
        let _home = crate::cli::commands::test_support::HomeGuard::new();
        config::save_credentials(&config::Credentials {
            node_id: "node-a".to_string(),
            credential_token: "token".to_string(),
            hub_endpoint: "axon://hub.example:50051".to_string(),
            realm: "acme".to_string(),
            username: Some("alice".to_string()),
            user_id: Some("user-alice".to_string()),
            ..Default::default()
        })
        .expect("write credentials");

        let request = device_request("acme", "node-a", "observe.health", json!({}))
            .expect("known local node resolves to local system target");

        assert_eq!(request.target(), &MissionInvocationTarget::LocalDevice);
    }

    #[test]
    fn dispatcher_accepts_missing_registry_as_first_run_empty_state() {
        let _home = crate::cli::commands::test_support::HomeGuard::new();

        let dispatcher = AgentAwareDispatcher::new(Arc::new(UnusedMissionGateway));

        assert!(
            dispatcher.is_ok(),
            "missing registry file is a valid first-run empty registry"
        );
    }

    #[test]
    fn dispatcher_rejects_malformed_registry_instead_of_empty_fallback() {
        let _home = crate::cli::commands::test_support::HomeGuard::new();
        let state_dir = config::state_dir();
        std::fs::create_dir_all(&state_dir).expect("create isolated state dir");
        std::fs::write(state_dir.join("agents.json"), "{not json")
            .expect("seed malformed registry");

        let error = match AgentAwareDispatcher::new(Arc::new(UnusedMissionGateway)) {
            Ok(_) => panic!("malformed registry must not construct an EAL dispatcher"),
            Err(error) => error,
        };
        let message = format!("{error:#}");

        assert!(
            message.contains("load Agent registry projection for EAL dispatch"),
            "unexpected error: {message}"
        );
        assert!(
            message.contains("parse"),
            "malformed registry must surface as parse failure, got: {message}"
        );
    }
}
