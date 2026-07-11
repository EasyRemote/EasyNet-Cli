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

use crate::daemon::ability::dispatch::AxonAbilityCatalog;
use crate::daemon::persistence::agent_registry::AgentRegistry;
use crate::daemon::persistence::config;

use crate::daemon::ability::dispatch::OwnerKind;
pub const ABILITY_LIST_AGENTS: &str = crate::daemon::ability::names::agents::AGENT_LIST;

/// Register `agent.list` on the registry.
///
/// `registry_provider` runs at handler-call time so a future
/// hot-reload of `agents.json` is reflected without re-registration.
pub fn register<F>(reg: &mut AxonAbilityCatalog, registry_provider: F)
where
    F: Fn() -> anyhow::Result<AgentRegistry> + Send + Sync + 'static,
{
    let provider: Arc<dyn Fn() -> anyhow::Result<AgentRegistry> + Send + Sync> =
        Arc::new(registry_provider);
    reg.register_rpc_with_owner(
        ABILITY_LIST_AGENTS,
        OwnerKind::Device,
        Arc::new(move |_args: Value| list_agents_handler(&provider)),
    );
}

fn list_agents_handler(
    registry_provider: &Arc<dyn Fn() -> anyhow::Result<AgentRegistry> + Send + Sync>,
) -> anyhow::Result<Value> {
    let registry = registry_provider()?;
    let local_agents = crate::daemon::persistence::local_agents::load()
        .map_err(|error| anyhow::anyhow!("agent.list: load hosted-agent URA index: {error:#}"))?;
    Ok(json!({ "agents": agent_rows(&registry, &local_agents) }))
}

fn agent_rows(
    registry: &AgentRegistry,
    local_agents: &crate::daemon::persistence::local_agents::LocalAgentsFile,
) -> Vec<Value> {
    let rows: Vec<Value> = registry
        .agents
        .iter()
        .map(|(name, e)| {
            let root = e
                .root_path
                .clone()
                .unwrap_or_else(|| config::agents_root().join(name));
            let ura = crate::daemon::persistence::local_agents::lookup_hosted_ura(
                local_agents,
                "llm",
                name,
            );
            json!({
                "name": name,
                "ura": ura.map(Value::String).unwrap_or(Value::Null),
                "runtime": e.agent_type.to_string(),
                "model": e.model.clone().map(Value::String).unwrap_or(Value::Null),
                "label": e.label.clone().map(Value::String).unwrap_or(Value::Null),
                "timeout_secs": e.timeout_secs,
                "root_path": root.to_string_lossy(),
                "root_exists": root.exists(),
            })
        })
        .collect();
    rows
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
    use crate::daemon::persistence::agent_registry::{AgentEntry, AgentRegistry, AgentType};

    #[test]
    fn registration_makes_list_agents_dispatchable() {
        let mut reg = AxonAbilityCatalog::new();
        register(&mut reg, || Ok(AgentRegistry::default()));
        assert!(reg.get_rpc(ABILITY_LIST_AGENTS).is_some());
    }

    #[test]
    fn list_agents_empty_registry_returns_empty_array() {
        let mut reg = AxonAbilityCatalog::new();
        register(&mut reg, || Ok(AgentRegistry::default()));
        let handler = reg.get_rpc(ABILITY_LIST_AGENTS).unwrap();
        let resp = handler(json!({})).unwrap();
        assert!(resp["agents"].as_array().unwrap().is_empty());
    }

    #[test]
    fn list_agents_propagates_registry_load_failure() {
        let mut reg = AxonAbilityCatalog::new();
        register(&mut reg, || anyhow::bail!("durable registry is unreadable"));
        let handler = reg.get_rpc(ABILITY_LIST_AGENTS).unwrap();

        let error = handler(json!({})).expect_err("corrupt state must not look like no agents");

        assert!(error.to_string().contains("durable registry is unreadable"));
    }

    #[test]
    fn list_agents_projects_name_runtime_model_label() {
        let mut registry = AgentRegistry::default();
        let mut entry = AgentEntry::new(AgentType::ClaudeCode, Some("sonnet".to_string()));
        entry.with_label(Some("primary".to_string()));
        registry.agents.insert("claude".to_string(), entry);

        let mut reg = AxonAbilityCatalog::new();
        let snapshot = registry.clone();
        register(&mut reg, move || Ok(snapshot.clone()));
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
        let mut registry = AgentRegistry::default();
        registry.agents.insert(
            "claude".to_string(),
            AgentEntry::new(AgentType::ClaudeCode, Some("sonnet".to_string())),
        );
        let mut local_agents = crate::daemon::persistence::local_agents::LocalAgentsFile::default();
        crate::daemon::persistence::local_agents::upsert_hosted_agent(
            &mut local_agents,
            "llm",
            "claude",
            "easynet:///r/acme/agent/alice.claude",
        );

        let rows = agent_rows(&registry, &local_agents);

        assert_eq!(rows.len(), 1);
        assert_eq!(rows[0]["ura"], "easynet:///r/acme/agent/alice.claude");
    }

    #[test]
    fn list_agents_renders_unset_optional_fields_as_null() {
        let mut registry = AgentRegistry::default();
        registry.agents.insert(
            "minimal".to_string(),
            AgentEntry::new(AgentType::Codex, None),
        );

        let mut reg = AxonAbilityCatalog::new();
        let snapshot = registry.clone();
        register(&mut reg, move || Ok(snapshot.clone()));
        let handler = reg.get_rpc(ABILITY_LIST_AGENTS).unwrap();
        let resp = handler(json!({})).unwrap();

        let row = &resp["agents"][0];
        assert_eq!(row["model"], Value::Null);
        assert_eq!(row["label"], Value::Null);
    }

    #[test]
    fn list_agents_does_not_leak_registry_entry_shape() {
        let mut registry = AgentRegistry::default();
        registry.agents.insert(
            "minimal".to_string(),
            AgentEntry::new(AgentType::Codex, None),
        );

        let rows = agent_rows(
            &registry,
            &crate::daemon::persistence::local_agents::LocalAgentsFile::default(),
        );

        assert!(rows[0].get("entry").is_none());
        assert_eq!(rows[0]["runtime"], "codex");
        assert!(rows[0].get("timeout_secs").is_some());
    }

    #[test]
    fn list_agents_input_schema_is_empty_object() {
        let s = list_agents_input_schema();
        assert_eq!(s["type"], "object");
        assert!(s["properties"].as_object().unwrap().is_empty());
        assert_eq!(s["additionalProperties"], false);
    }
}
