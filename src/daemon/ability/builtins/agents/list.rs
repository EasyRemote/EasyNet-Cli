// EasyNet CLI — agent.list ability handler
// =================================================
//
// File: src/daemon/ability/builtins/agents/list.rs
//
// Lists every locally-registered LLM sub-agent (claude, codex, …)
// that this device-profile hosts. Per RFC §18, the device-profile
// owns this ability; per the C-M13 audit, it's a Bucket-A gap that
// previously could only be reached via the CLI subcommand
// `easynet agent list` — now also reachable via Invoke from
// in-process or remote callers.
//
// Output shape
// ------------
//   { "agents": [
//       { "name": "claude",
//         "ura": "easynet:///r/<realm>/agent/<user>.claude",
//         "runtime": "claude-code",
//         "model": "sonnet",          // null if unset
//         "label": "primary"          // null if unset
//       },
//       ...
//     ]
//   }
//
// The shape is deliberately a thin per-row projection — NOT the v2
// A2A agent-card envelope (that lives at `a2a.bridge.list_skills`,
// which adds the per-agent skills list and `a2a_schema_version`).
// `agent.list` is the operational view ("what runs here"),
// `a2a.bridge.list_skills` is the protocol view ("what an A2A peer
// would discover"). Two callers, two shapes, one source registry.
//
// Author: Silan Hu <silan.hu@u.nus.edu>
// Copyright (c) 2026 EasyNet.

use std::sync::Arc;

use serde_json::{json, Value};

use crate::core::agent::id::AgentId;
use crate::daemon::ability::dispatch::AxonAbilityCatalog;
use crate::daemon::persistence::agent_aggregate::AgentAggregateSnapshot;

use crate::daemon::ability::dispatch::OwnerKind;
pub const ABILITY_LIST_AGENTS: &str = crate::daemon::ability::names::agents::AGENT_LIST;

/// Register `agent.list` on the registry.
///
/// `snapshot_provider` runs at handler-call time so a future hot-reload of
/// hosted Agent state is reflected without re-registration.
pub(crate) fn register<F>(reg: &mut AxonAbilityCatalog, snapshot_provider: F)
where
    F: Fn() -> anyhow::Result<AgentAggregateSnapshot> + Send + Sync + 'static,
{
    let provider: Arc<dyn Fn() -> anyhow::Result<AgentAggregateSnapshot> + Send + Sync> =
        Arc::new(snapshot_provider);
    reg.register_rpc_with_owner(
        ABILITY_LIST_AGENTS,
        OwnerKind::agent_management_system(),
        Arc::new(move |_args: Value| list_agents_handler(&provider)),
    );
}

fn list_agents_handler(
    registry_provider: &Arc<dyn Fn() -> anyhow::Result<AgentAggregateSnapshot> + Send + Sync>,
) -> anyhow::Result<Value> {
    let snapshot = registry_provider()?;
    Ok(json!({ "agents": agent_rows(&snapshot)? }))
}

fn agent_rows(snapshot: &AgentAggregateSnapshot) -> anyhow::Result<Vec<Value>> {
    let rows: Vec<Value> = snapshot
        .registered_agents()
        .map(|(registry_key, e)| -> anyhow::Result<Value> {
            let agent_id = AgentId::parse(registry_key).map_err(|error| {
                anyhow::anyhow!("agent.list: invalid registry key {registry_key:?}: {error}")
            })?;
            let name = agent_id.name.as_str();
            let root = e.required_root_path(registry_key, "agent.list")?;
            let ura = snapshot.hosted_llm_agent_ura(name);
            let publication_state = ura
                .as_deref()
                .map(crate::daemon::persistence::hosted_agent_publications::record_for)
                .transpose()?
                .flatten()
                .map(|record| record.publication_state().to_string());
            Ok(json!({
                "name": name,
                "ura": ura.map(|value| Value::String(value.to_string())).unwrap_or(Value::Null),
                "publication_state": publication_state.map(Value::String).unwrap_or(Value::Null),
                "runtime": e.agent_type.to_string(),
                "model": e.model.clone().map(Value::String).unwrap_or(Value::Null),
                "label": e.label.clone().map(Value::String).unwrap_or(Value::Null),
                "timeout_secs": e.timeout_secs,
                "root_path": root.to_string_lossy(),
                "root_exists": root.exists(),
            }))
        })
        .collect::<anyhow::Result<_>>()?;
    Ok(rows)
}

// ── Discovery surfaces ────────────────────────────────────────

pub fn list_agents_input_schema() -> Value {
    json!({
        "type": "object",
        "properties": {},
        "additionalProperties": false,
    })
}

pub fn list_agents_description() -> &'static str {
    "List the LLM sub-agents this device hosts (name, runtime, \
     optional model + label). Operational view; for the protocol \
     A2A view see a2a.bridge.list_skills."
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::core::agent::spec::RuntimeKind;
    use crate::daemon::persistence::agent_registry::{AgentEntry, AgentRegistry};
    use crate::daemon::persistence::local_agents::LocalAgentsFile;
    use std::path::PathBuf;

    fn registered_entry(agent_type: RuntimeKind, model: Option<String>, name: &str) -> AgentEntry {
        let mut entry = AgentEntry::new(agent_type, model);
        entry.root_path = Some(PathBuf::from(format!("/tmp/easynet-test-{name}")));
        entry
    }

    fn snapshot(registry: AgentRegistry) -> AgentAggregateSnapshot {
        AgentAggregateSnapshot::new(registry, LocalAgentsFile::default())
    }

    fn agent_list_test_catalog() -> AxonAbilityCatalog {
        AxonAbilityCatalog::new_test_metadata_for_device_authority(
            "easynet:///r/test/device/agent-list",
        )
    }

    #[test]
    fn registration_makes_list_agents_dispatchable() {
        let mut reg = agent_list_test_catalog();
        register(&mut reg, || Ok(snapshot(AgentRegistry::default())));
        assert!(reg.get_rpc(ABILITY_LIST_AGENTS).is_some());
    }

    #[test]
    fn list_agents_empty_registry_returns_empty_array() {
        let mut reg = agent_list_test_catalog();
        register(&mut reg, || Ok(snapshot(AgentRegistry::default())));
        let handler = reg.get_rpc(ABILITY_LIST_AGENTS).unwrap();
        let resp = handler(json!({})).unwrap();
        assert!(resp["agents"].as_array().unwrap().is_empty());
    }

    #[test]
    fn list_agents_propagates_registry_load_failure() {
        let mut reg = agent_list_test_catalog();
        register(&mut reg, || anyhow::bail!("durable registry is unreadable"));
        let handler = reg.get_rpc(ABILITY_LIST_AGENTS).unwrap();

        let error = handler(json!({})).expect_err("corrupt state must not look like no agents");

        assert!(error.to_string().contains("durable registry is unreadable"));
    }

    #[test]
    fn list_agents_projects_name_runtime_model_label() {
        let mut registry = AgentRegistry::default();
        let mut entry = registered_entry(
            RuntimeKind::ClaudeCode,
            Some("sonnet".to_string()),
            "claude",
        );
        entry.with_label(Some("primary".to_string()));
        registry.agents.insert("default/claude".to_string(), entry);

        let mut reg = agent_list_test_catalog();
        let registry_snapshot = registry.clone();
        register(&mut reg, move || Ok(snapshot(registry_snapshot.clone())));
        let handler = reg.get_rpc(ABILITY_LIST_AGENTS).unwrap();
        let resp = handler(json!({})).unwrap();

        let rows = resp["agents"].as_array().unwrap();
        assert_eq!(rows.len(), 1);
        assert_eq!(rows[0]["name"], "claude");
        assert_eq!(rows[0]["runtime"], "claude-code");
        assert_eq!(rows[0]["model"], "sonnet");
        assert_eq!(rows[0]["label"], "primary");
    }

    #[test]
    fn list_agents_projects_hosted_agent_ura_from_local_agents() {
        let _home = crate::cli::commands::test_support::HomeGuard::new();
        let mut registry = AgentRegistry::default();
        registry.agents.insert(
            "default/claude".to_string(),
            registered_entry(
                RuntimeKind::ClaudeCode,
                Some("sonnet".to_string()),
                "claude",
            ),
        );
        let mut local_agents = crate::daemon::persistence::local_agents::LocalAgentsFile::default();
        crate::daemon::persistence::local_agents::upsert_hosted_agent(
            &mut local_agents,
            "llm",
            "claude",
            "easynet:///r/acme/agent/alice.claude",
        );

        let snapshot = AgentAggregateSnapshot::new(registry, local_agents);
        let rows = agent_rows(&snapshot).unwrap();

        assert_eq!(rows.len(), 1);
        assert_eq!(rows[0]["ura"], "easynet:///r/acme/agent/alice.claude");
        assert_eq!(rows[0]["publication_state"], Value::Null);

        let pending = crate::daemon::persistence::hosted_agent_publications::begin_registration(
            "easynet:///r/acme/agent/alice.claude",
            "easynet:///r/acme/device/dev-1",
            1,
        )
        .unwrap();
        let assignment =
            crate::daemon::federation::hosted_agent_publication::HostedAgentGenerationAssignment {
                agent_ura: "easynet:///r/acme/agent/alice.claude".to_string(),
                host_device_ura: "easynet:///r/acme/device/dev-1".to_string(),
                incarnation_id: pending.incarnation_id().clone(),
                generation: 1,
            };
        crate::daemon::persistence::hosted_agent_publications::bind_assignment(&assignment, 2)
            .unwrap();
        assert_eq!(
            agent_rows(&snapshot).unwrap()[0]["publication_state"],
            "assigned"
        );
        crate::daemon::persistence::hosted_agent_publications::stage_projection(
            &assignment,
            pending.desired_catalog_epoch,
            1,
            "sha256:projection",
            3,
        )
        .unwrap();
        crate::daemon::persistence::hosted_agent_publications::mark_published(
            &assignment,
            pending.desired_catalog_epoch,
            1,
            "sha256:projection",
            4,
        )
        .unwrap();
        assert_eq!(
            agent_rows(&snapshot).unwrap()[0]["publication_state"],
            "published"
        );
    }

    #[test]
    fn list_agents_renders_unset_optional_fields_as_null() {
        let mut registry = AgentRegistry::default();
        registry.agents.insert(
            "default/minimal".to_string(),
            registered_entry(RuntimeKind::Codex, None, "minimal"),
        );

        let mut reg = agent_list_test_catalog();
        let registry_snapshot = registry.clone();
        register(&mut reg, move || Ok(snapshot(registry_snapshot.clone())));
        let handler = reg.get_rpc(ABILITY_LIST_AGENTS).unwrap();
        let resp = handler(json!({})).unwrap();

        let row = &resp["agents"][0];
        assert_eq!(
            row["ura"],
            Value::Null,
            "missing hosted-Agent identity must not synthesize the User account as an Agent"
        );
        assert_eq!(row["model"], Value::Null);
        assert_eq!(row["label"], Value::Null);
    }

    #[test]
    fn list_agents_does_not_leak_registry_entry_shape() {
        let mut registry = AgentRegistry::default();
        registry.agents.insert(
            "minimal".to_string(),
            registered_entry(RuntimeKind::Codex, None, "minimal"),
        );

        let rows = agent_rows(&snapshot(registry)).unwrap();

        assert!(rows[0].get("entry").is_none());
        assert_eq!(rows[0]["runtime"], "codex");
        assert!(rows[0].get("timeout_secs").is_some());
    }

    #[test]
    fn list_agents_rejects_registry_rows_without_canonical_root_path() {
        let mut registry = AgentRegistry::default();
        registry.agents.insert(
            "minimal".to_string(),
            AgentEntry::new(RuntimeKind::Codex, None),
        );

        let error = agent_rows(&snapshot(registry))
            .expect_err("steady-state registry rows must not infer root_path");

        assert!(error.to_string().contains("agent.list"));
        assert!(error.to_string().contains("root_path"));
    }

    #[test]
    fn list_agents_input_schema_is_empty_object() {
        let s = list_agents_input_schema();
        assert_eq!(s["type"], "object");
        assert!(s["properties"].as_object().unwrap().is_empty());
        assert_eq!(s["additionalProperties"], false);
    }
}
