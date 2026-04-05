// EasyNet CLI — MCP Tool Specifications
// ======================================
//
// File: src/mcp/specs.rs
// Description: JSON Schema definitions for Hub-level MCP tools.
//
// Author: Silan Hu <silan.hu@u.nus.edu>
// Copyright (c) 2026 EasyNet. All rights reserved.

use serde_json::{json, Value};

/// Build an MCP tool spec.
///
/// `params`: `(name, schema, required)` tuples describing each parameter.
fn tool(name: &str, desc: &str, params: &[(&str, Value, bool)]) -> Value {
    let mut props = serde_json::Map::new();
    let mut required = Vec::new();
    for &(key, ref schema, is_required) in params {
        props.insert(key.to_string(), schema.clone());
        if is_required {
            required.push(json!(key));
        }
    }
    let mut schema = json!({ "type": "object", "properties": props });
    if !required.is_empty() {
        schema["required"] = Value::Array(required);
    }
    json!({ "name": name, "description": desc, "inputSchema": schema })
}

/// Shorthand: `("name", str_type(), true)` for required string params.
fn str_type() -> Value { json!({"type": "string"}) }

pub fn tool_specs(bound_node: Option<&str>, lock_bound_node: bool) -> Vec<Value> {
    let mut specs = base_tool_specs();
    if let Some(bound) = bound_node {
        patch_bound_node(&mut specs, bound, lock_bound_node);
    }
    specs
}

fn base_tool_specs() -> Vec<Value> {
    // Short alias for required string schema — avoids repeating `str_type()` on every line.
    let s = str_type;
    vec![
        // Federation queries
        tool("hub_status", "Show Hub connection status, node/ability counts.", &[]),
        tool("list_devices", "List all federation devices.", &[
            ("state_filter", s(), false),
        ]),
        tool("get_device_detail", "Device info + installed abilities.", &[
            ("node_id", s(), true),
        ]),
        tool("list_all_abilities", "List abilities across all nodes.", &[
            ("node_id", s(), false),
            ("name_pattern", s(), false),
        ]),
        tool("search_abilities", "Search abilities by name/tags.", &[
            ("query", s(), true),
        ]),

        // A2A agents
        tool("list_a2a_agents", "List A2A agents (remote agents) in the tenant.", &[
            ("tags", json!({"type": "array", "items": {"type": "string"}}), false),
            ("owner_id", s(), false),
            ("limit", json!({"type": "integer"}), false),
        ]),
        tool("get_a2a_agent_card", "Get an A2A agent card by node_id.", &[
            ("node_id", s(), true),
        ]),
        tool("send_a2a_task", "Send an A2A task to a target agent (skill invocation).", &[
            ("target_agent_id", s(), true),
            ("skill_id", s(), true),
            ("input_json", json!({"type": "object"}), false),
            ("task_id", s(), false),
            ("idempotency_key", s(), false),
        ]),

        // Ability lifecycle
        tool("deploy_ability", "Publish+install+activate an ability.", &[
            ("node_id", s(), true),
            ("tool_name", s(), true),
            ("command", s(), true),
            ("description", s(), false),
        ]),
        tool("uninstall_ability", "Remove ability from device.", &[
            ("node_id", s(), true),
            ("install_id", s(), true),
        ]),

        // Remote execution
        tool("execute_command", "One-shot command on remote device.", &[
            ("node_id", s(), true),
            ("command", s(), true),
        ]),
        tool("invoke_ability", "Invoke ability on a federated node.", &[
            ("node_id", s(), true),
            ("ability", s(), true),
            ("arguments", json!({"type": "object"}), false),
        ]),

        // Orchestration
        tool("run_mission", "Compile and execute an EAL program.", &[
            ("eal_source", s(), true),
            ("emit_ir_only", json!({"type": "boolean"}), false),
        ]),

        // Device management
        tool("manage_device", "Drain or disconnect a device.", &[
            ("node_id", s(), true),
            ("action", json!({"type": "string", "enum": ["drain", "disconnect"]}), true),
        ]),
    ]
}

/// Tools that operate on a specific node and should be patched when bound.
/// Also used by `hub_kit.rs` to decide which tools get default `node_id` injection.
pub const NODE_SCOPED_TOOLS: &[&str] = &[
    "get_device_detail",
    "list_all_abilities",
    "deploy_ability",
    "execute_command",
    "invoke_ability",
    "manage_device",
    "uninstall_ability",
];

fn patch_bound_node(specs: &mut [Value], bound: &str, lock: bool) {
    for spec in specs.iter_mut() {
        let Some(name) = spec.get("name").and_then(|v| v.as_str()) else { continue };
        if !NODE_SCOPED_TOOLS.contains(&name) {
            continue;
        }

        if let Some(desc) = spec.get_mut("description") {
            if let Some(s) = desc.as_str() {
                let suffix = if lock {
                    format!(" (bound node: {bound})")
                } else {
                    format!(" (default node: {bound})")
                };
                *desc = Value::String(format!("{s}{suffix}"));
            }
        }

        let Some(schema) = spec.get_mut("inputSchema") else { continue };
        let Some(props) = schema.get_mut("properties") else { continue };

        // If node is locked, hide node_id so the agent doesn't try to override it.
        if lock {
            if let Value::Object(map) = props {
                map.remove("node_id");
            }
        }

        // Remove node_id from required list (defaults to bound node).
        if let Some(Value::Array(arr)) = schema.get_mut("required") {
            arr.retain(|v| v.as_str() != Some("node_id"));
        }
    }
}

/// Spec for send_to_agent tool (only included when agent dispatch is enabled).
pub fn send_to_agent_spec() -> Value {
    json!({
        "name": "send_to_agent",
        "description": "Send a prompt to a registered AI agent (Claude Code / Codex) and get their response. Enables agent-to-agent communication through EasyNet.",
        "inputSchema": {
            "type": "object",
            "properties": {
                "agent": {
                    "type": "string",
                    "description": "Registered agent name (e.g. 'claude', 'codex')"
                },
                "prompt": {
                    "type": "string",
                    "description": "The prompt/task to send to the agent"
                },
                "context": {
                    "type": "string",
                    "description": "Optional context from prior conversation or data"
                }
            },
            "required": ["agent", "prompt"]
        }
    })
}
