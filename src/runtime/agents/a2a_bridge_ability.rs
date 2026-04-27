// EasyNet CLI — a2a.bridge.list_skills ability handler
// =====================================================
//
// File: src/runtime/agents/a2a_bridge_ability.rs
//
// Edge-adapter ability that surfaces the host's local A2A agent card
// catalogue — the same `{"agents": [...]}` envelope this node ships
// to the realm hub via the `a2a.agents_json` node label. Per the
// consensus "A2A at the edge, NOT node-to-node" (RFC §A3 / plan §13),
// this ability is how an in-process caller (or a co-located A2A
// adapter) reaches the local A2A view through the same Invoke pipeline
// every other ability uses, instead of cracking the label string.
//
// What lives here
// ---------------
//   * a2a.bridge.list_skills — projects the local AgentRegistry to
//                              the v2 A2A agent-card envelope:
//                              `{ "agents": [{ name, runtime,
//                                              skills: [...], ... }] }`.
//                              Identical shape to the wire label, so
//                              callers parsing the label and callers
//                              hitting this ability see one truth.
//
// What does NOT live here yet
// ---------------------------
//   * a2a.bridge.send_task — incoming A2A `tasks/send` translated to
//                            an in-process Invocation. Same registry-
//                            self-reference issue as
//                            mcp.bridge.call_tool (C-M9a-ii); resolves
//                            with the same mechanism.
//   * a2a.client.* — outgoing A2A. Lives where the existing Axon
//                    `send_a2a_task` helper does, surfaced as an
//                    ability in a follow-up.
//
// Why this is unary, not bidi
// ---------------------------
// A2A `agents/list` (and the legacy node-roster query) is request /
// response. Streaming applies to A2A `tasks/send` not the catalogue
// fetch — that ability lands separately on Invoke (or InvokeStream
// once a streaming-friendly A2A task takes shape).
//
// Author: Silan Hu <silan.hu@u.nus.edu>
// Copyright (c) 2026 EasyNet.

use std::sync::Arc;

use serde_json::{json, Value};

use crate::registry::agents::AgentRegistry;
use crate::runtime::ability_dispatch::LocalAbilityRegistry;

pub const ABILITY_LIST_SKILLS: &str = "a2a.bridge.list_skills";

/// Register `a2a.bridge.list_skills` on the registry.
///
/// `registry_provider` is invoked at handler-call time so the
/// envelope reflects the latest in-memory `AgentRegistry`. v1 wires a
/// cloned snapshot (the daemon does not yet hot-reload `agents.json`),
/// but the seam is already in place so a future hot-reload propagates
/// without re-registering the handler.
pub fn register<F>(reg: &mut LocalAbilityRegistry, registry_provider: F)
where
    F: Fn() -> AgentRegistry + Send + Sync + 'static,
{
    let provider: Arc<dyn Fn() -> AgentRegistry + Send + Sync> = Arc::new(registry_provider);
    reg.register_rpc(
        ABILITY_LIST_SKILLS,
        Arc::new(move |_args: Value| list_skills_handler(&provider)),
    );
}

/// `a2a.bridge.list_skills` handler.
///
/// Returns the v2 `{"agents": [...]}` envelope as structured JSON
/// (NOT the stringified label form). Keeping the structured shape
/// here means an in-process caller doesn't have to re-parse the
/// label encoding to read it back.
fn list_skills_handler(
    registry_provider: &Arc<dyn Fn() -> AgentRegistry + Send + Sync>,
) -> anyhow::Result<Value> {
    let registry = registry_provider();
    Ok(crate::registry::a2a_labels::build_agents_envelope(&registry))
}

// ── Discovery surfaces ────────────────────────────────────────

pub fn list_skills_input_schema() -> Value {
    json!({
        "type": "object",
        "properties": {},
        "additionalProperties": false,
    })
}

pub fn list_skills_description() -> &'static str {
    "List the host's local A2A agent cards in the v2 envelope shape \
     (`{agents: [...]}`). Identical projection to what this node \
     ships as the `a2a.agents_json` realm label, so external A2A \
     consumers and in-process callers see one catalogue."
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::registry::agents::{AgentEntry, AgentRegistry, AgentType};

    #[test]
    fn registration_makes_list_skills_dispatchable() {
        let mut reg = LocalAbilityRegistry::new();
        register(&mut reg, AgentRegistry::default);
        assert!(reg.get_rpc(ABILITY_LIST_SKILLS).is_some());
    }

    #[test]
    fn list_skills_returns_v2_agents_envelope_for_empty_registry() {
        let mut reg = LocalAbilityRegistry::new();
        register(&mut reg, AgentRegistry::default);
        let handler = reg.get_rpc(ABILITY_LIST_SKILLS).unwrap();
        let resp = handler(json!({})).unwrap();
        // Even with zero registered agents, the envelope key MUST be
        // present and an array. A consumer that special-cased
        // "missing key means empty" is a footgun — assert the shape.
        let agents = resp["agents"]
            .as_array()
            .expect("envelope.agents is an array");
        assert!(agents.is_empty());
    }

    #[test]
    fn list_skills_input_schema_is_empty_object() {
        let s = list_skills_input_schema();
        assert_eq!(s["type"], "object");
        let props = s["properties"].as_object().expect("properties is object");
        assert!(props.is_empty(), "list_skills accepts no arguments");
        assert_eq!(s["additionalProperties"], false);
    }

    #[test]
    fn list_skills_reflects_provider_changes() {
        // The provider closure runs on every call, so a future
        // hot-reload of agents.json (or the test pattern below)
        // surfaces in subsequent list_skills calls without
        // re-registering the handler.
        use std::sync::Mutex;
        let snapshot: Arc<Mutex<AgentRegistry>> = Arc::new(Mutex::new(AgentRegistry::default()));
        let snap_for_provider = Arc::clone(&snapshot);
        let mut reg = LocalAbilityRegistry::new();
        register(&mut reg, move || {
            snap_for_provider.lock().unwrap().clone()
        });
        let handler = reg.get_rpc(ABILITY_LIST_SKILLS).unwrap();

        let first = handler(json!({})).unwrap();
        assert_eq!(first["agents"].as_array().unwrap().len(), 0);

        // Mutate the snapshot — verify the next call reflects it.
        // We don't assert on a specific agent shape because that's
        // covered by the `registry::a2a_labels` byte-stability
        // tests; here we just need the count to advance.
        snapshot.lock().unwrap().agents.insert(
            "probe".to_string(),
            AgentEntry::new(AgentType::ClaudeCode, None),
        );
        let second = handler(json!({})).unwrap();
        assert_eq!(second["agents"].as_array().unwrap().len(), 1);
    }
}
