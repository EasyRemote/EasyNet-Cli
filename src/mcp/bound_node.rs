// EasyNet CLI — MCP Bound-Node Contract
// =====================================
//
// File: src/mcp/bound_node.rs
// Description: The single home for the "bound node" abstraction used by
//              the Hub MCP server — the list of tools it applies to, and
//              the two transformations that it drives (schema patching
//              at spec-emission time, argument patching at dispatch
//              time).
//
// Why this module exists
// ----------------------
//
// The bound-node feature is one concept expressed in two places at
// runtime:
//
//   1. Schema patching (`apply_spec_patch`) — at the moment the provider
//      advertises its tools, each scoped tool's description picks up a
//      "(bound node: X)" / "(default node: X)" suffix, and under lock
//      the `node_id` property is removed from the schema so the LLM
//      cannot even reference it.
//
//   2. Argument patching (`apply_args_patch`) — at the moment a tool is
//      invoked, the missing/empty `node_id` is injected from the bound
//      value, and a mismatched value under lock is rejected.
//
// Both transforms read the same `NODE_SCOPED_TOOLS` membership list.
// Before this module existed, that list lived in `specs.rs` next to
// the schema builder, and the dispatch-time patcher in `provider.rs`
// reached across the module boundary to consume it. That worked, but
// it split one invariant ("tool X is node-scoped — treat it uniformly
// both at spec time and at dispatch time") across two files; a reader
// asking "what does it mean for a tool to be node-scoped?" had to
// grep for the string.
//
// This module collects all three — the membership list, the schema
// patch, and the args patch — into one file so that any future change
// to the bound-node abstraction is a local edit.
//
// Author: Silan Hu <silan.hu@u.nus.edu>
// Copyright (c) 2026 EasyNet. All rights reserved.

use super::error::McpError;
use serde_json::{Map, Value};

/// Tools whose `node_id` argument, when present, designates a specific
/// device — and which therefore participate in bound-node patching:
///
///   1. At spec emission time, [`apply_spec_patch`] appends a
///      `(default node: X)` / `(bound node: X)` suffix to the
///      description, strips `node_id` from `required`, and (under
///      lock) removes the property entirely.
///   2. At dispatch time, [`apply_args_patch`] injects the bound
///      value when the caller omits `node_id` (or sends null / empty
///      string), matching the description's promise.
///
/// Membership criterion: "would binding this tool to a specific
/// device be a meaningful operator experience?" — not "does the
/// underlying RPC require a node." `invoke_ability`'s `node_id` is
/// optional (auto-route) yet it lives here, because binding a Hub to
/// one device should make `invoke_ability` default to that device
/// just like the other verbs do.
///
/// `list_all_abilities` is intentionally absent: it is discovery-
/// oriented and defaults to a federation-wide view when `node_id` is
/// omitted. Forcing it to the bound node would hide the rest of the
/// TANet from the agent and defeat the single-RPC discovery
/// optimization.
pub const NODE_SCOPED_TOOLS: &[&str] = &[
    "get_device_detail",
    "deploy_ability",
    "execute_command",
    "invoke_ability",
    "manage_device",
    "uninstall_ability",
    // `get_a2a_agent_card` targets exactly one node's A2A card. A Hub
    // bound to `node-x` should pre-fill `node_id=node-x` here just as
    // it does for the other per-device verbs; otherwise the agent can
    // still read any node's card while *invoking* is pinned, which is
    // an asymmetric and confusing binding. Caught by the
    // `node_scoped_tools_matches_tools_with_node_id_parameter` test in
    // `specs.rs` — do NOT remove without also extending the test's
    // DOCUMENTED_EXCLUSIONS list with a rationale.
    "get_a2a_agent_card",
];

/// Patch a slice of already-built tool specs in place to reflect the
/// active bound-node binding.
///
/// For every spec whose `name` is in [`NODE_SCOPED_TOOLS`]:
///
/// - Append `" (bound node: X)"` (lock=true) or `" (default node: X)"`
///   (lock=false) to the spec's `description`.
/// - Strip `"node_id"` from the `required` array; if the array becomes
///   empty, drop the field entirely so the patched shape matches the
///   unpatched emission rule.
/// - Under lock, additionally remove `"node_id"` from the
///   `properties` map so the LLM cannot reference the field at all.
///
/// Non-scoped tools are untouched. A malformed spec missing `name`,
/// `description`, or `inputSchema` is skipped silently — callers that
/// build specs through [`super::specs::tool`] never produce such
/// specs, but the defensive path is cheap and keeps this function
/// total.
pub fn apply_spec_patch(specs: &mut [Value], bound: &str, lock: bool) {
    for spec in specs.iter_mut() {
        let Some(name) = spec.get("name").and_then(|v| v.as_str()) else {
            continue;
        };
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

        // Narrow `schema` to its object once, then do the two
        // independent cleanups underneath. Flattening the borrow
        // chain keeps each operation from being shadowed by the
        // other's early-exit — the previous two-step form had a
        // `let Some(props) … else continue` that silently skipped
        // the `required` cleanup when a scoped tool happened to have
        // no `properties` field (Bug #8 regression).
        let Some(schema_obj) = spec.get_mut("inputSchema").and_then(Value::as_object_mut) else {
            continue;
        };

        // (1) Under lock, remove `node_id` from the properties map so
        //     the agent can't override the bound value. If there's no
        //     `properties` field at all, the spec is degenerate; move
        //     on — the `required` cleanup below is still worth
        //     running.
        if lock {
            if let Some(Value::Object(props)) = schema_obj.get_mut("properties") {
                props.remove("node_id");
            }
        }

        // (2) Strip `node_id` from `required` (dispatcher injects the
        //     bound value at call time). If the list becomes empty
        //     as a result, drop the field entirely so the patched
        //     spec mirrors `tool()` construction, which only emits
        //     `required` when non-empty.
        if let Some(Value::Array(arr)) = schema_obj.get_mut("required") {
            arr.retain(|v| v.as_str() != Some("node_id"));
        }
        let required_now_empty = schema_obj
            .get("required")
            .and_then(Value::as_array)
            .is_some_and(Vec::is_empty);
        if required_now_empty {
            schema_obj.remove("required");
        }
    }
}

/// Patch the arguments of one inbound tool invocation so the bound
/// node is honoured. Pure logic — exposed as a free function so it
/// can be unit-tested without a live `DendriteBridge`.
///
/// Contract (evaluated in the order below):
///
/// - No bound node → pass `args` through unchanged.
/// - Tool is not node-scoped → pass through. The patcher must not
///   touch tools like `list_all_abilities` that are deliberately
///   federation-wide.
/// - `node_id` absent, or present as `null` / empty string → inject
///   the bound value. The empty-string branch guards against
///   over-eager LLMs that "fill in" every property with a
///   placeholder; the spec description promises
///   `(default node: X)`, so we must honour it under both lock
///   modes.
/// - `node_id` present but not a string → reject with a clear type
///   error. Silently overwriting a wrong-type value would hide
///   contract violations.
/// - `node_id` matches bound → accept silently.
/// - `node_id` differs AND lock is on → reject (lock is the hard
///   guarantee).
/// - `node_id` differs AND lock is off → accept (bound was a
///   default, caller explicitly overrode it).
pub fn apply_args_patch(
    bound_node: Option<&str>,
    lock_bound_node: bool,
    tool_name: &str,
    args: &Map<String, Value>,
) -> Result<Map<String, Value>, McpError> {
    let Some(bound) = bound_node else {
        return Ok(args.clone());
    };
    if !NODE_SCOPED_TOOLS.contains(&tool_name) {
        return Ok(args.clone());
    }

    let mut patched = args.clone();
    match patched.get("node_id") {
        // Absent, or a null/empty string that means "I don't have a node".
        None | Some(Value::Null) => {
            patched.insert("node_id".into(), Value::String(bound.to_string()));
        }
        // A real string — check it against bound.
        Some(Value::String(s)) => {
            let trimmed = s.trim();
            if trimmed.is_empty() {
                patched.insert("node_id".into(), Value::String(bound.to_string()));
            } else if trimmed == bound {
                if trimmed != s {
                    patched.insert("node_id".into(), Value::String(trimmed.to_string()));
                }
            } else if lock_bound_node {
                // Bound-node lock violation: caller asked for a node
                // other than the one the server was configured to
                // honour. That's a caller-input contract violation —
                // `validation_error`, not `unavailable`.
                return Err(McpError::Validation(format!(
                    "tool `{tool_name}` is bound to node_id `{bound}`, but got `{trimmed}`"
                )));
            } else if trimmed != s {
                patched.insert("node_id".into(), Value::String(trimmed.to_string()));
            }
        }
        // Anything else — number, bool, object, array — violates the
        // schema. Surface the violation instead of overwriting it.
        Some(other) => {
            return Err(McpError::Validation(format!(
                "tool `{tool_name}` expects `node_id` to be a string, got {other}"
            )));
        }
    }
    Ok(patched)
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    fn args(json: Value) -> Map<String, Value> {
        json.as_object()
            .expect("test payload must be an object")
            .clone()
    }

    // ── apply_args_patch ────────────────────────────────────────────────────

    #[test]
    fn unbound_passes_args_through() {
        let result = apply_args_patch(
            None,
            false,
            "invoke_ability",
            &args(json!({"ability": "foo"})),
        )
        .unwrap();
        assert_eq!(result.get("node_id"), None);
    }

    #[test]
    fn non_node_scoped_tools_are_never_patched() {
        let result = apply_args_patch(
            Some("node-x"),
            true,
            "list_all_abilities",
            &args(json!({})),
        )
        .unwrap();
        assert!(
            !result.contains_key("node_id"),
            "list_all_abilities must remain federation-wide even under lock"
        );
    }

    #[test]
    fn missing_node_id_is_injected_from_bound() {
        let result = apply_args_patch(
            Some("node-x"),
            false,
            "invoke_ability",
            &args(json!({"ability": "foo"})),
        )
        .unwrap();
        assert_eq!(
            result.get("node_id").and_then(Value::as_str),
            Some("node-x")
        );
    }

    #[test]
    fn empty_string_node_id_is_injected_from_bound() {
        // Regression guard for the "(default node: X)" promise: an LLM
        // that dutifully fills every property with `""` must still land
        // on the bound node, not bypass into auto-route.
        let result = apply_args_patch(
            Some("node-x"),
            false,
            "invoke_ability",
            &args(json!({"node_id": "", "ability": "foo"})),
        )
        .unwrap();
        assert_eq!(
            result.get("node_id").and_then(Value::as_str),
            Some("node-x")
        );
    }

    #[test]
    fn whitespace_only_node_id_is_injected_from_bound() {
        let result = apply_args_patch(
            Some("node-x"),
            false,
            "invoke_ability",
            &args(json!({"node_id": "   ", "ability": "foo"})),
        )
        .unwrap();
        assert_eq!(
            result.get("node_id").and_then(Value::as_str),
            Some("node-x")
        );
    }

    #[test]
    fn matching_node_id_is_accepted_silently() {
        let result = apply_args_patch(
            Some("node-x"),
            true,
            "invoke_ability",
            &args(json!({"node_id": "node-x", "ability": "foo"})),
        )
        .unwrap();
        assert_eq!(
            result.get("node_id").and_then(Value::as_str),
            Some("node-x")
        );
    }

    #[test]
    fn differing_node_id_under_lock_is_rejected() {
        // Lock-violations are caller-input contract errors: agents must
        // see `validation_error`, not `unavailable` (retrying the same
        // call will produce the same rejection).
        let err = apply_args_patch(
            Some("node-x"),
            true,
            "invoke_ability",
            &args(json!({"node_id": "node-y", "ability": "foo"})),
        )
        .unwrap_err();
        assert_eq!(err.error_code(), "validation_error");
        let msg = err.message();
        assert!(msg.contains("node-x"), "error must cite bound id: {msg}");
        assert!(msg.contains("node-y"), "error must cite received id: {msg}");
    }

    #[test]
    fn differing_node_id_without_lock_is_accepted() {
        // Without lock, the bound value is a default — the caller is
        // allowed to override it.
        let result = apply_args_patch(
            Some("node-x"),
            false,
            "invoke_ability",
            &args(json!({"node_id": "node-y", "ability": "foo"})),
        )
        .unwrap();
        assert_eq!(
            result.get("node_id").and_then(Value::as_str),
            Some("node-y")
        );
    }

    #[test]
    fn node_id_is_trimmed_when_override_is_allowed() {
        let result = apply_args_patch(
            Some("node-x"),
            false,
            "invoke_ability",
            &args(json!({"node_id": " node-y ", "ability": "foo"})),
        )
        .unwrap();
        assert_eq!(
            result.get("node_id").and_then(Value::as_str),
            Some("node-y")
        );
    }

    #[test]
    fn empty_string_node_id_is_injected_even_under_lock() {
        // Mirror of empty_string_node_id_is_injected_from_bound but
        // with lock=true. The "(bound node: X)" description promises
        // the bound value will always be used; receiving `""` must
        // still honour that, not bypass into auto-route.
        let result = apply_args_patch(
            Some("node-x"),
            true,
            "invoke_ability",
            &args(json!({"node_id": "", "ability": "foo"})),
        )
        .unwrap();
        assert_eq!(
            result.get("node_id").and_then(Value::as_str),
            Some("node-x")
        );
    }

    #[test]
    fn null_node_id_is_injected_from_bound() {
        // JSON null is a common "not set" encoding from some MCP
        // clients. Treat it identically to a missing field.
        let result = apply_args_patch(
            Some("node-x"),
            false,
            "invoke_ability",
            &args(json!({"node_id": null, "ability": "foo"})),
        )
        .unwrap();
        assert_eq!(
            result.get("node_id").and_then(Value::as_str),
            Some("node-x")
        );
    }

    #[test]
    fn non_string_node_id_is_rejected_with_type_error() {
        // A numeric node_id violates the schema. We must surface the
        // contract violation, not silently overwrite it with the
        // bound value (that would hide upstream bugs in the caller).
        let err = apply_args_patch(
            Some("node-x"),
            false,
            "invoke_ability",
            &args(json!({"node_id": 42, "ability": "foo"})),
        )
        .unwrap_err();
        assert_eq!(err.error_code(), "validation_error");
        let msg = err.message();
        assert!(msg.contains("node_id"), "error must name the field: {msg}");
        assert!(msg.contains("string"), "error must explain the type: {msg}");
    }

    #[test]
    fn non_string_node_id_is_rejected_under_lock_too() {
        let err = apply_args_patch(
            Some("node-x"),
            true,
            "invoke_ability",
            &args(json!({"node_id": ["a", "b"], "ability": "foo"})),
        )
        .unwrap_err();
        assert_eq!(err.error_code(), "validation_error");
        assert!(err.message().contains("string"));
    }

    // ── apply_spec_patch ────────────────────────────────────────────────────

    #[test]
    fn spec_patcher_still_cleans_required_when_properties_absent() {
        // Bug #8 regression. If a scoped tool's inputSchema has
        // `required` but no `properties` (a degenerate but
        // syntactically valid spec), an earlier two-step form would
        // `continue` after failing to find `properties` and silently
        // leave `node_id` in `required`. The flattened form must
        // handle this. We synthesize the scenario because our real
        // specs all have `properties`; `get_device_detail` is a
        // node-scoped tool we can masquerade as.
        let mut specs = vec![json!({
            "name": "get_device_detail",
            "description": "synthetic — no properties field",
            "inputSchema": {
                "type": "object",
                "required": ["node_id"]
                // no "properties" on purpose
            }
        })];
        apply_spec_patch(&mut specs, "node-x", true);
        let schema = specs[0]
            .get("inputSchema")
            .and_then(Value::as_object)
            .unwrap();
        assert!(
            !schema.contains_key("required"),
            "required must be dropped (was [node_id], now empty) even without properties"
        );
    }

    #[test]
    fn spec_patcher_appends_default_node_suffix_when_unlocked() {
        let mut specs = vec![json!({
            "name": "invoke_ability",
            "description": "Invoke an ability.",
            "inputSchema": {"type": "object", "properties": {"node_id": {"type": "string"}}}
        })];
        apply_spec_patch(&mut specs, "node-x", false);
        let desc = specs[0]
            .get("description")
            .and_then(Value::as_str)
            .unwrap();
        assert!(desc.ends_with("(default node: node-x)"), "got: {desc}");
    }

    #[test]
    fn spec_patcher_appends_bound_node_suffix_when_locked() {
        let mut specs = vec![json!({
            "name": "invoke_ability",
            "description": "Invoke an ability.",
            "inputSchema": {"type": "object", "properties": {"node_id": {"type": "string"}}}
        })];
        apply_spec_patch(&mut specs, "node-x", true);
        let desc = specs[0]
            .get("description")
            .and_then(Value::as_str)
            .unwrap();
        assert!(desc.ends_with("(bound node: node-x)"), "got: {desc}");
    }

    #[test]
    fn spec_patcher_skips_non_scoped_tools() {
        // A tool not in NODE_SCOPED_TOOLS must be left untouched even
        // when bound_node is set.
        let mut specs = vec![json!({
            "name": "list_all_abilities",
            "description": "Federation-wide discovery.",
            "inputSchema": {"type": "object", "properties": {"node_id": {"type": "string"}}}
        })];
        apply_spec_patch(&mut specs, "node-x", true);
        let desc = specs[0]
            .get("description")
            .and_then(Value::as_str)
            .unwrap();
        assert_eq!(desc, "Federation-wide discovery.", "scoped-only patcher must skip discovery tools");
    }
}
