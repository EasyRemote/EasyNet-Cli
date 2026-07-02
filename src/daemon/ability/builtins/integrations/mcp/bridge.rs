// EasyNet CLI — mcp.bridge.list_tools ability handler
// =====================================================
//
// File: src/daemon/ability/builtins/integrations/mcp/bridge.rs
//
// Edge-adapter ability handler that surfaces the host's local
// ability registry as MCP tools. Per the consensus "MCP at the
// edge, NOT node-to-node" (RFC §A3 / plan §13), this ability is
// how a node-side caller (or a co-located mcp_server stdio
// runner) reaches the registry through the same Invoke pipeline
// every other ability uses, instead of bypassing it via the
// LocalInvoker shortcut.
//
// What lives here
// ---------------
//   * mcp.bridge.list_tools — projects local AbilityDescriptors
//                             to the MCP `tools/list` shape:
//                             { tools: [{name, description,
//                                        inputSchema}, ...] }
//
// What lives here (continued)
// ---------------------------
//   * mcp.bridge.call_tool — dispatches an in-process Invoke
//                            against the advertised MCP tool name
//                            (or legacy dotted ability name) and
//                            wraps the response in MCP `tools/call`
//                            shape. The §A5 visibility filter is
//                            checked client-side BEFORE we hit the
//                            registry: PRIVATE abilities never
//                            leak through this surface (the
//                            descriptors_provider's filter is what
//                            advertises which tools are callable;
//                            we re-check on call to defeat a stale-
//                            client race).
//
// What does NOT live here yet
// ---------------------------
//   * MCP **client** abilities (mcp.client.list / mcp.client.call) —
//     blocked on a stdio MCP client library that doesn't yet
//     exist in axon-sdk. Tracked as C-M9b.
//
// Why this is unary, not bidi
// ---------------------------
// MCP tools/list is request/response. The MCP-native streaming
// shapes (sampling, progress) aren't surfaced as ability calls,
// so unary Invoke is the right primitive.
//
// Author: Silan Hu <silan.hu@u.nus.edu>
// Copyright (c) 2026 EasyNet.

use std::sync::Arc;

use serde_json::{json, Value};

use crate::daemon::ability::catalog::profiles::mcp::{
    canonical_ability_name_for_mcp_tool, tool_specs_from_descriptors,
};
use crate::daemon::ability::catalog::profiles::DEFAULT_MCP_AGENT_ID;
use crate::runtime::ability_descriptor::AbilityDescriptor;
use crate::runtime::ability_dispatch::AxonAbilityCatalog;
use crate::runtime::ability_dispatch::OwnerKind;

pub const ABILITY_LIST_TOOLS: &str =
    crate::daemon::ability::names::integrations::MCP_BRIDGE_LIST_TOOLS;
pub const ABILITY_CALL_TOOL: &str =
    crate::daemon::ability::names::integrations::MCP_BRIDGE_CALL_TOOL;

/// Register both bridge abilities on the registry.
///
/// `descriptors_provider` is invoked at handler-call time so the
/// list reflects the registry's current state (e.g. after a future
/// hot-reload of skills) AND so call_tool can re-check visibility
/// on each call against the same source of truth list_tools used.
///
/// `registry_handle` is a `OnceLock` that the build site populates
/// AFTER `Arc::new(reg)`. The call_tool handler reads through it to
/// look up the target ability's local handler. The build-time
/// chicken-and-egg (registering a handler that needs a reference to
/// the registry being built) is resolved by deferred initialisation:
/// register first, set the lock once the registry is wrapped in an
/// Arc. Same seam admin_status_ability uses.
pub fn register<F>(
    reg: &mut AxonAbilityCatalog,
    descriptors_provider: F,
    registry_handle: Arc<std::sync::OnceLock<Arc<AxonAbilityCatalog>>>,
) where
    F: Fn() -> Vec<AbilityDescriptor> + Send + Sync + 'static,
{
    let provider: Arc<dyn Fn() -> Vec<AbilityDescriptor> + Send + Sync> =
        Arc::new(descriptors_provider);
    let provider_for_list = Arc::clone(&provider);
    reg.register_rpc_with_owner(
        "mcp.bridge.list_tools",
        OwnerKind::Agent(DEFAULT_MCP_AGENT_ID.to_string()),
        Arc::new(move |_args: Value| list_tools_handler(&provider_for_list)),
    );
    reg.register_rpc_with_owner(
        "mcp.bridge.call_tool",
        OwnerKind::Agent(DEFAULT_MCP_AGENT_ID.to_string()),
        Arc::new(move |args: Value| call_tool_handler(&provider, &registry_handle, args)),
    );
}

/// `mcp.bridge.list_tools` handler.
///
/// Returns `{ "tools": [<MCP tool spec>, ...] }` where each tool
/// spec is `{ name, description, inputSchema }` per the MCP
/// tools/list convention. The projection is identical to what
/// StdioMcpServer would emit through its own tools/list handler —
/// ensuring an in-process Invoke caller and an external MCP client
/// see the same catalog.
fn list_tools_handler(
    descriptors_provider: &Arc<dyn Fn() -> Vec<AbilityDescriptor> + Send + Sync>,
) -> anyhow::Result<Value> {
    let descriptors = descriptors_provider();
    let tools = tool_specs_from_descriptors(&descriptors);
    Ok(json!({ "tools": tools }))
}

/// `mcp.bridge.call_tool` handler.
///
/// Args: `{ "name": "<advertised-mcp-tool-name>", "arguments": <json-value> }`.
/// Canonical dotted EasyNet ability names are retained only in
/// `x-easynet.ability`; callers must use the advertised MCP tool name.
/// Mirrors the MCP `tools/call` request shape; `arguments` is optional
/// (some tools take none). Returns
/// `{ "content": [<text|json blob>], "isError": bool }` per MCP's
/// `tools/call` response convention.
///
/// Three failure paths, each surfaced with `isError: true` rather
/// than an `Err` so the MCP client sees a structured response (per
/// MCP spec, transport-level errors crash the connection — we
/// reserve those for genuine bugs):
///   1. `name` missing or non-string → input validation error.
///   2. `name` not in the descriptors list (or filtered out by
///      visibility) → "tool not found".
///   3. Local handler returned `Err` → echo the error message.
///
/// The visibility re-check (#2) defends against a stale list_tools
/// cache: if the descriptors_provider's filter changes between a
/// list_tools and a follow-up call_tool, the call must obey the
/// FRESH filter. The MCP-shaped projection is the source of truth
/// for "which names are callable through this surface."
fn call_tool_handler(
    descriptors_provider: &Arc<dyn Fn() -> Vec<AbilityDescriptor> + Send + Sync>,
    registry_handle: &Arc<std::sync::OnceLock<Arc<AxonAbilityCatalog>>>,
    args: Value,
) -> anyhow::Result<Value> {
    let name = match args.get("name").and_then(Value::as_str) {
        Some(s) if !s.is_empty() => s.to_string(),
        _ => {
            return Ok(error_response(
                "`name` is required and must be a non-empty string",
            ));
        }
    };
    let arguments = args.get("arguments").cloned().unwrap_or(Value::Null);

    // Visibility re-check: a stale list_tools client MUST NOT
    // bypass the filter the bridge advertises. We compare against
    // the same descriptors source list_tools projects from.
    let descriptors = descriptors_provider();
    let Some(ability_name) = canonical_ability_name_for_mcp_tool(&descriptors, &name) else {
        return Ok(error_response(&format!(
            "tool `{name}` not found in the bridge's advertised catalogue"
        )));
    };

    // Reach into the registry through the post-build OnceLock seam.
    // If the lock is empty we surface as isError rather than panic
    // so the MCP wire shape is still well-formed; in production the
    // build site populates it before any call lands, so this branch
    // only fires for misconfigured tests.
    let Some(registry) = registry_handle.get() else {
        return Ok(error_response(
            "registry handle not initialised (build-site forgot to set the OnceLock)",
        ));
    };
    if !registry.has_rpc(ability_name) {
        // The descriptor said it exists but the runtime/catalog doesn't
        // have an RPC handler — likely a streaming-only or bidi ability
        // advertised through the catalogue. MCP tools/call is unary;
        // tell the caller honestly.
        return Ok(error_response(&format!(
            "tool `{name}` maps to ability `{ability_name}`, which is not invocable as a unary RPC (may be a stream or bidi handler)"
        )));
    }

    match registry.invoke_rpc_json(ability_name, arguments) {
        Ok(value) => Ok(success_response(value)),
        Err(e) => Ok(error_response(&format!(
            "tool `{name}` maps to ability `{ability_name}`, which returned an error: {e}"
        ))),
    }
}

/// Build an MCP `tools/call` success response. The MCP spec
/// expects `content` as an array of typed parts; we serialise the
/// ability's JSON response into ONE `text` part containing the
/// JSON-encoded value. Future enhancement: detect string responses
/// and emit them as raw text rather than JSON-quoted strings.
fn success_response(value: Value) -> Value {
    // `serde_json::to_string` on a Value cannot fail — Value is
    // by construction always JSON-encodable (no NaN, no cycles).
    // expect() makes the invariant explicit; a fallback string
    // would just pretend to handle a case that doesn't exist.
    let text =
        serde_json::to_string(&value).expect("serde_json::Value is always JSON-serializable");
    json!({
        "content": [{
            "type": "text",
            "text": text,
        }],
        "isError": false,
    })
}

fn error_response(message: &str) -> Value {
    json!({
        "content": [{
            "type": "text",
            "text": message,
        }],
        "isError": true,
    })
}

// ── Discovery surfaces ────────────────────────────────────────

pub fn list_tools_input_schema() -> Value {
    json!({
        "type": "object",
        "properties": {},
        "additionalProperties": false,
    })
}

pub fn list_tools_description() -> &'static str {
    "List the host's local abilities projected to the MCP tools/list \
     shape. Used by in-process MCP-aware callers; the same projection \
     drives the stdio MCP server's tools/list response so external \
     and internal callers see one catalog."
}

pub fn call_tool_input_schema() -> Value {
    json!({
        "type": "object",
        "required": ["name"],
        "properties": {
            "name": {"type": "string", "minLength": 1},
            "arguments": {
                "description": "Free-form per-tool args; shape per the tool's own input_schema."
            },
        },
        "additionalProperties": false,
    })
}

pub fn call_tool_description() -> &'static str {
    "Invoke a tool advertised by mcp.bridge.list_tools. Mirrors the \
     MCP tools/call shape: returns {content:[{type:\"text\",text}], \
     isError}. Visibility is re-checked against the live descriptor \
     catalogue on each call, so a stale list_tools cache cannot \
     bypass the bridge filter."
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::runtime::ability_descriptor::{AbilityDescriptor, Visibility};
    use std::sync::OnceLock;

    fn d(name: &str) -> AbilityDescriptor {
        AbilityDescriptor::new(
            name.to_string(),
            "easynet:///r/test/device/01DEV",
            Visibility::Public,
        )
        .expect("test descriptor")
    }

    /// Test fixture: register list_tools + call_tool against a
    /// catalogue, then wire the OnceLock to the resulting Arc'd
    /// registry. Mirrors the build-site OnceLock seam exactly.
    fn build_bridge_registry<F>(provider: F) -> Arc<AxonAbilityCatalog>
    where
        F: Fn() -> Vec<AbilityDescriptor> + Send + Sync + 'static,
    {
        let mut reg = AxonAbilityCatalog::new();
        let handle: Arc<OnceLock<Arc<AxonAbilityCatalog>>> = Arc::new(OnceLock::new());
        // Pre-register one trivial ability the bridge can dispatch
        // into, so call_tool tests have something real to invoke.
        reg.register_rpc_with_owner(
            "test.echo",
            OwnerKind::Device,
            Arc::new(|args: Value| Ok(json!({"echoed": args}))),
        );
        register(&mut reg, provider, Arc::clone(&handle));
        let arc = Arc::new(reg);
        let _ = handle.set(arc.clone());
        arc
    }

    #[test]
    fn registration_makes_both_dispatchable() {
        let arc = build_bridge_registry(|| vec![d("observe.health")]);
        assert!(arc.get_rpc(ABILITY_LIST_TOOLS).is_some());
        assert!(arc.get_rpc(ABILITY_CALL_TOOL).is_some());
    }

    #[test]
    fn list_tools_returns_projection_of_provider_descriptors() {
        let arc = build_bridge_registry(|| vec![d("observe.health"), d("agent.list")]);
        let handler = arc.get_rpc(ABILITY_LIST_TOOLS).unwrap();
        let resp = handler(json!({})).unwrap();
        let tools = resp["tools"].as_array().expect("tools array");
        assert_eq!(tools.len(), 2);
        let names: Vec<&str> = tools.iter().map(|t| t["name"].as_str().unwrap()).collect();
        assert!(names.contains(&"observe_health"));
        assert!(names.contains(&"agent_list"));
    }

    #[test]
    fn list_tools_reflects_provider_changes() {
        // The provider closure runs on each call so a registry that
        // grows abilities at runtime (e.g. after a skill install) is
        // reflected in subsequent list_tools calls.
        use std::sync::Mutex;
        let snapshot: Arc<Mutex<Vec<AbilityDescriptor>>> = Arc::new(Mutex::new(vec![d("a.x")]));
        let snap_for_provider = Arc::clone(&snapshot);
        let arc = build_bridge_registry(move || snap_for_provider.lock().unwrap().clone());
        let handler = arc.get_rpc(ABILITY_LIST_TOOLS).unwrap();

        let first = handler(json!({})).unwrap();
        assert_eq!(first["tools"].as_array().unwrap().len(), 1);

        snapshot.lock().unwrap().push(d("a.y"));
        let second = handler(json!({})).unwrap();
        assert_eq!(second["tools"].as_array().unwrap().len(), 2);
    }

    #[test]
    fn list_tools_input_schema_is_empty_object() {
        let s = list_tools_input_schema();
        assert_eq!(s["type"], "object");
        let props = s["properties"].as_object().expect("properties is object");
        assert!(props.is_empty(), "list_tools accepts no arguments");
        assert_eq!(s["additionalProperties"], false);
    }

    #[test]
    fn empty_provider_yields_empty_tools() {
        let arc = build_bridge_registry(std::vec::Vec::new);
        let handler = arc.get_rpc(ABILITY_LIST_TOOLS).unwrap();
        let resp = handler(json!({})).unwrap();
        assert_eq!(resp["tools"].as_array().unwrap().len(), 0);
    }

    // ── call_tool ─────────────────────────────────────────────

    #[test]
    fn call_tool_round_trips_listed_tool_name_through_registered_ability() {
        // Happy path: descriptor advertises `test.echo`, registry has
        // a handler for it, call_tool forwards args and wraps the
        // response in MCP tools/call shape.
        let arc = build_bridge_registry(|| vec![d("test.echo")]);
        let handler = arc.get_rpc(ABILITY_CALL_TOOL).unwrap();
        let resp = handler(json!({
            "name": "test_echo",
            "arguments": {"hello": "world"}
        }))
        .unwrap();
        assert_eq!(resp["isError"], false);
        let text = resp["content"][0]["text"].as_str().expect("text frame");
        // Echo handler wraps args in {"echoed": ...}; the bridge
        // serialises that JSON into the text part.
        assert!(text.contains("hello"));
        assert!(text.contains("world"));
    }

    #[test]
    fn call_tool_rejects_retired_canonical_dotted_alias() {
        let arc = build_bridge_registry(|| vec![d("test.echo")]);
        let handler = arc.get_rpc(ABILITY_CALL_TOOL).unwrap();
        let resp = handler(json!({
            "name": "test.echo",
            "arguments": {"hello": "world"}
        }))
        .unwrap();
        assert_eq!(resp["isError"], true);
        let text = resp["content"][0]["text"].as_str().unwrap();
        assert!(
            text.contains("not found"),
            "retired dotted alias must not dispatch; got {text:?}"
        );
    }

    #[test]
    fn call_tool_visibility_filter_rejects_unadvertised_names() {
        // §A5 enforcement: the descriptor list is the source of
        // truth for which names are callable. test.echo IS in the
        // registry but NOT in the catalogue → call_tool refuses.
        let arc = build_bridge_registry(|| vec![d("observe.health")]); // no test.echo
        let handler = arc.get_rpc(ABILITY_CALL_TOOL).unwrap();
        let resp = handler(json!({
            "name": "test_echo",
            "arguments": {}
        }))
        .unwrap();
        assert_eq!(
            resp["isError"], true,
            "names absent from the descriptor list MUST surface as isError"
        );
        let text = resp["content"][0]["text"].as_str().unwrap();
        assert!(
            text.contains("not found"),
            "error must say `not found`, got {text:?}"
        );
    }

    #[test]
    fn call_tool_unknown_name_in_descriptors_but_not_in_registry_returns_error() {
        // Descriptor advertises something with no matching RPC
        // handler (e.g. a stream-only ability registered through
        // the same descriptor catalogue). call_tool refuses cleanly.
        let arc = build_bridge_registry(|| vec![d("nonexistent.ability")]);
        let handler = arc.get_rpc(ABILITY_CALL_TOOL).unwrap();
        let resp = handler(json!({
            "name": "nonexistent_ability",
            "arguments": {}
        }))
        .unwrap();
        assert_eq!(resp["isError"], true);
        let text = resp["content"][0]["text"].as_str().unwrap();
        assert!(
            text.contains("not invocable as a unary RPC"),
            "should distinguish stream/bidi from unknown; got {text:?}"
        );
    }

    #[test]
    fn call_tool_missing_name_arg_returns_error_not_panic() {
        let arc = build_bridge_registry(|| vec![d("test.echo")]);
        let handler = arc.get_rpc(ABILITY_CALL_TOOL).unwrap();
        let resp = handler(json!({})).unwrap();
        assert_eq!(resp["isError"], true);
        let text = resp["content"][0]["text"].as_str().unwrap();
        assert!(text.contains("`name` is required"));
    }

    #[test]
    fn call_tool_empty_name_string_returns_error() {
        let arc = build_bridge_registry(|| vec![d("test.echo")]);
        let handler = arc.get_rpc(ABILITY_CALL_TOOL).unwrap();
        let resp = handler(json!({"name": "", "arguments": {}})).unwrap();
        assert_eq!(resp["isError"], true);
    }

    #[test]
    fn call_tool_handler_error_is_surfaced_as_error_text() {
        // A handler that returns Err must NOT crash the bridge —
        // it surfaces the error message inside an isError frame so
        // the MCP client sees a structured response.
        let mut reg = AxonAbilityCatalog::new();
        let handle: Arc<OnceLock<Arc<AxonAbilityCatalog>>> = Arc::new(OnceLock::new());
        reg.register_rpc_with_owner(
            "always.fails",
            OwnerKind::Device,
            Arc::new(|_args: Value| anyhow::bail!("planned failure for the test")),
        );
        register(&mut reg, || vec![d("always.fails")], Arc::clone(&handle));
        let arc = Arc::new(reg);
        let _ = handle.set(arc.clone());

        let handler = arc.get_rpc(ABILITY_CALL_TOOL).unwrap();
        let resp = handler(json!({"name": "always_fails", "arguments": {}})).unwrap();
        assert_eq!(resp["isError"], true);
        let text = resp["content"][0]["text"].as_str().unwrap();
        assert!(text.contains("planned failure"));
    }

    #[test]
    fn call_tool_tolerates_missing_arguments_field() {
        // Some MCP tools take no args; a tools/call without an
        // `arguments` key must still dispatch (we substitute Null).
        let arc = build_bridge_registry(|| vec![d("test.echo")]);
        let handler = arc.get_rpc(ABILITY_CALL_TOOL).unwrap();
        let resp = handler(json!({"name": "test_echo"})).unwrap();
        assert_eq!(resp["isError"], false);
    }

    #[test]
    fn call_tool_input_schema_requires_name() {
        let s = call_tool_input_schema();
        let req = s["required"].as_array().unwrap();
        assert!(req.iter().any(|v| v == "name"));
        assert_eq!(s["properties"]["name"]["minLength"], 1);
    }

    #[test]
    fn call_tool_response_shape_is_mcp_tools_call_compliant() {
        // Regression guard: the wire shape MUST be
        // {content: [...], isError: bool}. A typo'd refactor that
        // emitted {body: ...} would break every MCP client.
        let arc = build_bridge_registry(|| vec![d("test.echo")]);
        let handler = arc.get_rpc(ABILITY_CALL_TOOL).unwrap();
        let resp = handler(json!({"name": "test_echo", "arguments": null})).unwrap();
        assert!(resp.get("content").is_some(), "missing `content` key");
        assert!(resp.get("isError").is_some(), "missing `isError` key");
        let content = resp["content"].as_array().expect("content is array");
        assert_eq!(content[0]["type"], "text");
        assert!(content[0].get("text").is_some());
    }

    #[test]
    fn call_tool_with_unset_registry_handle_returns_is_error() {
        // Defensive: if the build site forgets to populate the
        // OnceLock, surface as isError instead of panicking. This
        // pins the "test bug not crash" contract from the comment
        // in call_tool_handler.
        let mut reg = AxonAbilityCatalog::new();
        let handle: Arc<OnceLock<Arc<AxonAbilityCatalog>>> = Arc::new(OnceLock::new());
        register(&mut reg, || vec![d("test.echo")], Arc::clone(&handle));
        let arc = Arc::new(reg);
        // Deliberately do NOT set the handle.
        let handler = arc.get_rpc(ABILITY_CALL_TOOL).unwrap();
        let resp = handler(json!({"name": "test_echo"})).unwrap();
        assert_eq!(resp["isError"], true);
        let text = resp["content"][0]["text"].as_str().unwrap();
        assert!(text.contains("not initialised"));
    }
}
