// EasyNet CLI — device.agent.list ability handler
// =================================================
//
// File: src/runtime/agents/agent_list_ability.rs
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
// `device.agent.list` is the operational view ("what runs here"),
// `a2a.bridge.list_skills` is the protocol view ("what an A2A peer
// would discover"). Two callers, two shapes, one source registry.
//
// Author: Silan Hu <silan.hu@u.nus.edu>
// Copyright (c) 2026 EasyNet.

use std::sync::Arc;

use serde_json::{json, Value};

use crate::registry::agents::AgentRegistry;
use crate::runtime::ability_dispatch::LocalAbilityRegistry;

use crate::runtime::ability_dispatch::OwnerKind;
pub const ABILITY_LIST_AGENTS: &str = "device.agent.list";

/// Register `device.agent.list` on the registry.
///
/// `registry_provider` runs at handler-call time so a future
/// hot-reload of `agents.json` is reflected without re-registration.
pub fn register<F>(reg: &mut LocalAbilityRegistry, registry_provider: F)
where
    F: Fn() -> AgentRegistry + Send + Sync + 'static,
{
    let provider: Arc<dyn Fn() -> AgentRegistry + Send + Sync> = Arc::new(registry_provider);
    reg.register_rpc_with_owner(
        "device.agent.list",
        OwnerKind::Device,
        Arc::new(move |_args: Value| list_agents_handler(&provider)),
    );
}

fn list_agents_handler(
    registry_provider: &Arc<dyn Fn() -> AgentRegistry + Send + Sync>,
) -> anyhow::Result<Value> {
    let registry = registry_provider();
    let rows: Vec<Value> = registry
        .agents
        .iter()
        .map(|(name, e)| {
            json!({
                "name": name,
                "runtime": e.agent_type.to_string(),
                "model": e.model.clone().map(Value::String).unwrap_or(Value::Null),
                "label": e.label.clone().map(Value::String).unwrap_or(Value::Null),
            })
        })
        .collect();
    Ok(json!({ "agents": rows }))
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
    use crate::registry::agents::{AgentEntry, AgentRegistry, AgentType};

    #[test]
    fn registration_makes_list_agents_dispatchable() {
        let mut reg = LocalAbilityRegistry::new();
        register(&mut reg, AgentRegistry::default);
        assert!(reg.get_rpc(ABILITY_LIST_AGENTS).is_some());
    }

    #[test]
    fn list_agents_empty_registry_returns_empty_array() {
        let mut reg = LocalAbilityRegistry::new();
        register(&mut reg, AgentRegistry::default);
        let handler = reg.get_rpc(ABILITY_LIST_AGENTS).unwrap();
        let resp = handler(json!({})).unwrap();
        assert!(resp["agents"].as_array().unwrap().is_empty());
    }

    #[test]
    fn list_agents_projects_name_runtime_model_label() {
        let mut registry = AgentRegistry::default();
        let mut entry = AgentEntry::new(AgentType::ClaudeCode, Some("sonnet".to_string()));
        entry.with_label(Some("primary".to_string()));
        registry.agents.insert("claude".to_string(), entry);

        let mut reg = LocalAbilityRegistry::new();
        let snapshot = registry.clone();
        register(&mut reg, move || snapshot.clone());
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
    fn list_agents_renders_unset_optional_fields_as_null() {
        let mut registry = AgentRegistry::default();
        registry.agents.insert(
            "minimal".to_string(),
            AgentEntry::new(AgentType::Codex, None),
        );

        let mut reg = LocalAbilityRegistry::new();
        let snapshot = registry.clone();
        register(&mut reg, move || snapshot.clone());
        let handler = reg.get_rpc(ABILITY_LIST_AGENTS).unwrap();
        let resp = handler(json!({})).unwrap();

        let row = &resp["agents"][0];
        assert_eq!(row["model"], Value::Null);
        assert_eq!(row["label"], Value::Null);
    }

    #[test]
    fn list_agents_input_schema_is_empty_object() {
        let s = list_agents_input_schema();
        assert_eq!(s["type"], "object");
        assert!(s["properties"].as_object().unwrap().is_empty());
        assert_eq!(s["additionalProperties"], false);
    }
}
