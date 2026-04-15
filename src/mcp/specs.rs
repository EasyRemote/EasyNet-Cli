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
fn str_type() -> Value {
    json!({"type": "string"})
}

pub fn tool_specs(bound_node: Option<&str>, lock_bound_node: bool) -> Vec<Value> {
    let mut specs = base_tool_specs();
    if let Some(bound) = bound_node {
        super::bound_node::apply_spec_patch(&mut specs, bound, lock_bound_node);
    }
    specs
}

/// Advertised range for the `limit` parameter on A2A agent listing.
/// Mirrors the runtime validation in `handlers::parse_list_a2a_limit`.
pub(crate) const LIST_A2A_LIMIT_MIN: u64 = 1;
pub(crate) const LIST_A2A_LIMIT_MAX: u64 = 1000;
pub(crate) const LIST_A2A_LIMIT_DEFAULT: u64 = 100;

fn base_tool_specs() -> Vec<Value> {
    // Short alias for required string schema — avoids repeating `str_type()` on every line.
    let s = str_type;
    let limit_description = format!(
        "Maximum number of entries to return. Integer in [{LIST_A2A_LIMIT_MIN}, {LIST_A2A_LIMIT_MAX}]. Default {LIST_A2A_LIMIT_DEFAULT}."
    );
    vec![
        // ── Federation queries ──────────────────────────────────────────────
        tool(
            "hub_status",
            "Report the local EasyNet Hub's connection health and a rollup of \
             registered devices and abilities. Use before other tools to confirm \
             the Hub is reachable; returns a connection status, device count, and \
             ability count — no arguments.",
            &[],
        ),
        tool(
            "list_devices",
            "List every device registered to the current tenant's Hub, with \
             node_id, label, state (online/offline/draining), OS, and last-seen \
             timestamp. Pair with `state_filter` (e.g. 'online') to narrow the \
             result; omit it to see all devices.",
            &[("state_filter", s(), false)],
        ),
        tool(
            "get_device_detail",
            "Fetch one device's full record: identity (node_id, label, owner), \
             runtime state (online/offline/draining), system fingerprint (OS, \
             arch, CPU), last heartbeat, and the list of abilities currently \
             installed *and* activated on it. Use this when `list_devices` \
             gives you a candidate node and you need to know what it can do \
             before calling `invoke_ability` or `execute_command`.",
            &[("node_id", s(), true)],
        ),
        tool(
            "list_all_abilities",
            "Discover abilities across the federation. Omit `node_id` for the \
             federation-wide view (single RPC, deduplicated by tool_name, each \
             entry carries `node_ids[]` showing every device that serves it). \
             Set `node_id` to scope to one device. `name_pattern` accepts a \
             substring or glob filter on tool_name (e.g. `fs.*` or \
             `photo`). This is the single discovery surface — it replaced the \
             previous pair of (`list_all_abilities`, `search_abilities`) tools \
             which were behaviourally identical at the handler layer. Use \
             `invoke_ability` to actually call what you find.",
            &[("node_id", s(), false), ("name_pattern", s(), false)],
        ),
        // ── A2A (Agent-to-Agent) ────────────────────────────────────────────
        tool(
            "list_a2a_agents",
            "List remote agents published in the current tenant's A2A directory. \
             Each returned entry is a lightweight summary (agent name, owner, \
             tags, skill count). Filter by `tags` (array) or `owner_id` (string); \
             use `limit` to cap the result size. Call `get_a2a_agent_card` on \
             any returned entry to see its full skill surface before issuing \
             `send_a2a_task`.",
            &[
                (
                    "tags",
                    json!({"type": "array", "items": {"type": "string"}}),
                    false,
                ),
                ("owner_id", s(), false),
                (
                    "limit",
                    json!({
                        "type": "integer",
                        "minimum": LIST_A2A_LIMIT_MIN,
                        "maximum": LIST_A2A_LIMIT_MAX,
                        "default": LIST_A2A_LIMIT_DEFAULT,
                        "description": limit_description,
                    }),
                    false,
                ),
            ],
        ),
        tool(
            "get_a2a_agent_card",
            "Fetch one remote agent's public profile — its advertised skills, \
             input/output schemas, and availability. An 'agent card' is the A2A \
             spec's published manifest that describes what tasks the agent can \
             handle. Call this before `send_a2a_task` so the task arguments \
             match the skill's schema.",
            &[("node_id", s(), true)],
        ),
        tool(
            "send_a2a_task",
            "Invoke one skill on a remote A2A agent. `target_agent_id` is the \
             agent's node_id (from `list_a2a_agents`); `skill_id` is a skill \
             name from that agent's card (from `get_a2a_agent_card`); \
             `input_json` carries the skill arguments matching the card's \
             input schema. `idempotency_key` lets you retry safely after a \
             network hiccup — the same key will return the same result.",
            &[
                ("target_agent_id", s(), true),
                ("skill_id", s(), true),
                ("input_json", json!({"type": "object"}), false),
                ("task_id", s(), false),
                ("idempotency_key", s(), false),
            ],
        ),
        // ── Ability lifecycle ───────────────────────────────────────────────
        tool(
            "deploy_ability",
            "Publish, install, and activate an ability on a device in one call. \
             `tool_name` is the public identifier callers will use to invoke it; \
             `command` is the shell/binary the device runs when the ability is \
             invoked; `description` is free-form user-facing text. This replaces \
             any existing installation with the same `tool_name` on the target \
             node.",
            &[
                ("node_id", s(), true),
                ("tool_name", s(), true),
                ("command", s(), true),
                ("description", s(), false),
            ],
        ),
        tool(
            "uninstall_ability",
            "Remove one installed ability from a device. `install_id` is the \
             installation identifier from `get_device_detail` (each device keeps \
             its own install id, since the same ability can be installed on \
             multiple devices with different configurations).",
            &[("node_id", s(), true), ("install_id", s(), true)],
        ),
        // ── Remote execution ────────────────────────────────────────────────
        tool(
            "execute_command",
            "Run a one-shot shell command on a specific device and return its \
             stdout/stderr and exit code. This is the general-purpose escape \
             hatch for tasks that don't have a dedicated ability. Prefer \
             `invoke_ability` when an ability exists: abilities have typed \
             arguments, schema validation, and an audit record.",
            &[("node_id", s(), true), ("command", s(), true)],
        ),
        tool(
            "invoke_ability",
            "Invoke an ability by tool_name. Omit `node_id` to auto-route: the \
             runtime resolves the first activated install exposing this tool \
             within the caller's tenant and returns `selected_node_id` so you \
             see where the call landed. Set `node_id` to pin execution to a \
             specific device. If the same ability is activated on multiple \
             nodes and auto-route's first-match policy isn't what you want, \
             call `list_all_abilities` first — each entry's `node_ids[]` \
             lists every device serving it, then pass the chosen node_id here.",
            &[
                ("node_id", s(), false),
                ("ability", s(), true),
                ("arguments", json!({"type": "object"}), false),
            ],
        ),
        // ── Orchestration ──────────────────────────────────────────────────
        tool(
            "run_mission",
            "Compile and execute an EAL (EasyNet Ability Language) program \
             across one or more devices. EAL is the declarative orchestration \
             language for multi-step, multi-device missions; a program is a \
             sequence of `let x = call <ability> with { … }` steps whose \
             outputs feed later steps. Pass `emit_ir_only=true` to compile \
             without executing (useful for pre-flight validation).",
            &[
                ("eal_source", s(), true),
                ("emit_ir_only", json!({"type": "boolean"}), false),
            ],
        ),
        // ── Device management ──────────────────────────────────────────────
        tool(
            "manage_device",
            "Drain or disconnect a registered device. `drain` gracefully stops \
             accepting new invocations and lets in-flight calls finish; \
             `disconnect` forcibly closes the device's control channel. Use \
             `drain` before a planned maintenance window; use `disconnect` \
             only when a device is misbehaving and must be removed immediately.",
            &[
                ("node_id", s(), true),
                (
                    "action",
                    json!({"type": "string", "enum": ["drain", "disconnect"]}),
                    true,
                ),
            ],
        ),
    ]
}

// ── Bound-node transforms ──────────────────────────────────────────────────
// The `NODE_SCOPED_TOOLS` list, the spec patcher that reads it at
// spec-emission time, and the argument patcher that reads it at
// dispatch time now live in `super::bound_node`. Keeping them there
// collects the bound-node abstraction into one file and lets this
// module stay focused on pure schema data.

/// Spec for send_to_agent tool (only included when agent dispatch is enabled).
///
/// The description below explicitly frames `send_to_agent` as the
/// wire-level form of `<agent>.chat(<prompt>)`. This matches ontology
/// §6.2 Decision 4: `agent send` is sugar for a single-line External
/// EAL mission invoking the target's default `chat` ability. The MCP
/// tool exists so an in-agent runtime can talk to another agent
/// without writing EAL itself; the EasyNet Hub does the desugar
/// internally so all cross-agent calls flow through the mission
/// runtime regardless of which surface the caller used.
pub fn send_to_agent_spec() -> Value {
    json!({
        "name": "send_to_agent",
        "description": "Send a prompt to another registered agent (e.g. 'claude', 'codex') and return its reply. The EasyNet Hub desugars this to a one-line EAL mission invoking the target's default `chat` ability, so the call is audited and routed through the same mission runtime as any other ability invocation. Use this when the current agent needs to delegate a subtask to a peer agent.",
        "inputSchema": {
            "type": "object",
            "properties": {
                "agent": {
                    "type": "string",
                    "description": "Registered agent name (e.g. 'claude', 'codex'). Equivalent to the `<agent>` in `<agent>.chat(...)`."
                },
                "prompt": {
                    "type": "string",
                    "description": "The prompt to pass as the `prompt:` named argument of the target agent's `chat` ability."
                },
                "context": {
                    "type": "string",
                    "description": "Optional prior-conversation context. Folded into the prompt before dispatch."
                }
            },
            "required": ["agent", "prompt"]
        }
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use super::super::bound_node::NODE_SCOPED_TOOLS;

    fn find_spec<'a>(specs: &'a [Value], name: &str) -> &'a Value {
        specs
            .iter()
            .find(|s| s.get("name").and_then(|v| v.as_str()) == Some(name))
            .unwrap_or_else(|| panic!("spec '{name}' must be present"))
    }

    fn required_set(spec: &Value) -> std::collections::HashSet<String> {
        spec.get("inputSchema")
            .and_then(|s| s.get("required"))
            .and_then(|r| r.as_array())
            .map(|arr| {
                arr.iter()
                    .filter_map(|v| v.as_str().map(String::from))
                    .collect()
            })
            .unwrap_or_default()
    }

    #[test]
    fn list_all_abilities_node_id_is_optional() {
        let specs = base_tool_specs();
        let s = find_spec(&specs, "list_all_abilities");
        assert!(
            !required_set(s).contains("node_id"),
            "list_all_abilities must NOT require node_id — auto-route discovery depends on it"
        );
    }

    #[test]
    fn invoke_ability_node_id_is_optional() {
        // Single-tool auto-route: omitting node_id asks the runtime to pick
        // an activated install. Requiring node_id would defeat that purpose.
        let specs = base_tool_specs();
        let s = find_spec(&specs, "invoke_ability");
        let req = required_set(s);
        assert!(
            req.contains("ability"),
            "invoke_ability must require 'ability'"
        );
        assert!(
            !req.contains("node_id"),
            "invoke_ability must allow auto-route (no required node_id)"
        );
    }

    #[test]
    fn invoke_auto_spec_is_removed() {
        // The separate invoke_auto tool has been merged into invoke_ability.
        // Exposing both forces the agent to pick between two tools that do
        // the same thing when node_id is omitted — a pointless choice.
        let specs = base_tool_specs();
        assert!(
            specs
                .iter()
                .all(|s| s.get("name").and_then(|v| v.as_str()) != Some("invoke_auto")),
            "invoke_auto must not be present; use invoke_ability with or without node_id"
        );
    }

    #[test]
    fn list_all_abilities_is_not_in_node_scoped_tools() {
        // The bound-node patcher uses NODE_SCOPED_TOOLS to decide which
        // tools to auto-fill node_id for. list_all_abilities must NOT be
        // in that list — otherwise binding a Hub to a single device hides
        // the rest of the TANet from the agent.
        assert!(
            !NODE_SCOPED_TOOLS.contains(&"list_all_abilities"),
            "list_all_abilities must remain federation-wide regardless of bound_node"
        );
    }

    #[test]
    fn invoke_ability_is_in_node_scoped_tools() {
        // Even though node_id is optional, invoke_ability stays in
        // NODE_SCOPED_TOOLS so that a bound Hub pre-fills the node for the
        // agent (and, when locked, hides the field entirely).
        assert!(NODE_SCOPED_TOOLS.contains(&"invoke_ability"));
    }

    /// Invariant: every tool whose input schema carries a `node_id`
    /// property must *either* appear in `NODE_SCOPED_TOOLS` *or* be
    /// listed below as an intentional exclusion. This prevents the
    /// silent regression where someone adds a new device-targeted
    /// tool and forgets to register it — the bound-node patcher then
    /// skips it, and a Hub bound to `node-x` lets the agent call that
    /// new tool against arbitrary nodes without pre-fill, which is
    /// exactly the safety the binding was supposed to provide.
    ///
    /// The documented exclusion is `list_all_abilities`: its
    /// `node_id` filter is discovery-oriented ("narrow to one node"),
    /// and auto-filling it from the bound node would hide the rest of
    /// the federation from the caller. When adding new exclusions,
    /// document the rationale here so a future reviewer sees the
    /// intent, not just the exception list.
    #[test]
    fn node_scoped_tools_matches_tools_with_node_id_parameter() {
        // Intentional exclusions — tools that accept `node_id` but are
        // federation-wide by design.
        const DOCUMENTED_EXCLUSIONS: &[&str] = &["list_all_abilities"];

        let specs = base_tool_specs();
        let mut tools_with_node_id: Vec<&str> = Vec::new();
        for spec in &specs {
            let Some(name) = spec.get("name").and_then(|v| v.as_str()) else {
                continue;
            };
            let has_node_id = spec
                .get("inputSchema")
                .and_then(|s| s.get("properties"))
                .and_then(|p| p.as_object())
                .is_some_and(|p| p.contains_key("node_id"));
            if has_node_id {
                tools_with_node_id.push(name);
            }
        }

        let mut missing_from_scoped: Vec<&str> = Vec::new();
        for tool in &tools_with_node_id {
            if NODE_SCOPED_TOOLS.contains(tool) {
                continue;
            }
            if DOCUMENTED_EXCLUSIONS.contains(tool) {
                continue;
            }
            missing_from_scoped.push(tool);
        }
        assert!(
            missing_from_scoped.is_empty(),
            "new tools with a `node_id` parameter must be added to \
             NODE_SCOPED_TOOLS (or to the test's DOCUMENTED_EXCLUSIONS \
             with a rationale comment). Found unregistered: {missing_from_scoped:?}"
        );

        // Mirror check: NODE_SCOPED_TOOLS must not contain a tool that
        // doesn't actually carry `node_id` in its spec (dead entry).
        let mut orphaned_in_scoped: Vec<&str> = Vec::new();
        for &tool in NODE_SCOPED_TOOLS {
            if !tools_with_node_id.contains(&tool) {
                orphaned_in_scoped.push(tool);
            }
        }
        assert!(
            orphaned_in_scoped.is_empty(),
            "NODE_SCOPED_TOOLS contains tools without a `node_id` parameter \
             in their input schema — the list is stale: {orphaned_in_scoped:?}"
        );
    }

    #[test]
    fn bound_node_patcher_does_not_strip_node_id_from_list_all_abilities() {
        // Even when bound_node is set + locked, the discovery tool's
        // node_id property must remain in the schema (so users can still
        // narrow by node when they want), and `node_id` must not become
        // required.
        let specs = tool_specs(Some("node-x"), true);
        let s = find_spec(&specs, "list_all_abilities");
        let req = required_set(s);
        assert!(
            !req.contains("node_id"),
            "even with bound_node, list_all_abilities's node_id stays optional"
        );
    }

    #[test]
    fn bound_node_patcher_locks_invoke_ability_by_default() {
        // Confirm the patcher does its job for the tools that ARE node-scoped.
        // With lock=true the property is removed AND node_id is no longer required.
        let specs = tool_specs(Some("node-x"), true);
        let s = find_spec(&specs, "invoke_ability");
        let req = required_set(s);
        assert!(
            !req.contains("node_id"),
            "locked bound_node must remove node_id from invoke_ability's required set"
        );
        let props = s
            .get("inputSchema")
            .and_then(|x| x.get("properties"))
            .and_then(|p| p.as_object())
            .unwrap();
        assert!(
            !props.contains_key("node_id"),
            "locked bound_node must hide node_id from invoke_ability's properties"
        );
    }

    // `bound_node_patcher_still_cleans_required_when_properties_absent`
    // lives in `super::super::bound_node::tests`; moved with the code it
    // exercises.

    #[test]
    fn bound_node_patcher_drops_empty_required_array() {
        // For a tool whose ONLY required field was node_id (e.g. `execute_command`
        // strips down after bound-node patching), the resulting `required` list
        // becomes empty. Emitting `"required": []` is semantically fine but
        // asymmetric with `tool()` construction, which never emits an empty list.
        // Keep the patched spec shape consistent with the unpatched shape.
        //
        // `deploy_ability` is a better test subject — it still has `tool_name`
        // and `command` in required even after node_id is stripped. So we test
        // `execute_command` which bottoms out at empty.
        let specs = tool_specs(Some("node-x"), true);
        let s = find_spec(&specs, "execute_command");
        let schema = s.get("inputSchema").and_then(Value::as_object).unwrap();
        match schema.get("required") {
            None => {} // expected: dropped entirely
            Some(Value::Array(arr)) if !arr.is_empty() => {
                // Also acceptable: required retained but non-empty.
                // (Guards against accidental over-stripping.)
            }
            Some(other) => panic!(
                "patched spec must have no `required` field or a non-empty array; got {other:?}"
            ),
        }
    }
}
