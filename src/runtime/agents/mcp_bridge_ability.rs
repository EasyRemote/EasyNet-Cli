// EasyNet CLI — mcp.bridge.list_tools ability handler
// =====================================================
//
// File: src/runtime/agents/mcp_bridge_ability.rs
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
// What does NOT live here yet
// ---------------------------
//   * mcp.bridge.call_tool — needs a registry self-reference so
//     the handler can dispatch back into other local abilities.
//     LocalAbilityRegistry is currently &mut-mutable (no interior
//     mutability), and the registry-construction site can't yield
//     an Arc<self> to a registered handler. Resolving that needs
//     either (a) interior-mutable registry or (b) a separate
//     dispatcher Arc the handler closes over. Tracked as
//     C-M9a-ii; lands separately.
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

use crate::runtime::ability_descriptor::AbilityDescriptor;
use crate::runtime::ability_dispatch::LocalAbilityRegistry;
use crate::runtime::agents::profiles::mcp::tool_specs_from_descriptors;

pub const ABILITY_LIST_TOOLS: &str = "mcp.bridge.list_tools";

/// Register mcp.bridge.list_tools on the registry.
///
/// `descriptors_provider` is invoked at handler-call time so the
/// list reflects the registry's current state (e.g. after a future
/// hot-reload of skills). Today the daemon's registry is built
/// once at boot and read-only thereafter, so the closure typically
/// returns a static snapshot.
pub fn register<F>(reg: &mut LocalAbilityRegistry, descriptors_provider: F)
where
    F: Fn() -> Vec<AbilityDescriptor> + Send + Sync + 'static,
{
    let provider: Arc<dyn Fn() -> Vec<AbilityDescriptor> + Send + Sync> =
        Arc::new(descriptors_provider);
    reg.register_rpc(
        ABILITY_LIST_TOOLS,
        Arc::new(move |_args: Value| list_tools_handler(&provider)),
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

#[cfg(test)]
mod tests {
    use super::*;
    use crate::runtime::ability_descriptor::{AbilityDescriptor, Visibility};

    fn d(name: &str) -> AbilityDescriptor {
        AbilityDescriptor::new(
            name.to_string(),
            "easynet:///r/test/agent/01DEV",
            Visibility::Public,
        )
        .expect("test descriptor")
    }

    #[test]
    fn registration_makes_list_tools_dispatchable() {
        let mut reg = LocalAbilityRegistry::new();
        register(&mut reg, || vec![]);
        assert!(reg.get_rpc(ABILITY_LIST_TOOLS).is_some());
    }

    #[test]
    fn list_tools_returns_projection_of_provider_descriptors() {
        let mut reg = LocalAbilityRegistry::new();
        register(&mut reg, || vec![d("observe.health"), d("fleet.list_agents")]);
        let handler = reg.get_rpc(ABILITY_LIST_TOOLS).unwrap();
        let resp = handler(json!({})).unwrap();
        let tools = resp["tools"].as_array().expect("tools array");
        assert_eq!(tools.len(), 2);
        let names: Vec<&str> = tools.iter().map(|t| t["name"].as_str().unwrap()).collect();
        assert!(names.contains(&"observe.health"));
        assert!(names.contains(&"fleet.list_agents"));
    }

    #[test]
    fn list_tools_reflects_provider_changes() {
        // The provider closure runs on each call so a registry that
        // grows abilities at runtime (e.g. after a skill install) is
        // reflected in subsequent list_tools calls.
        use std::sync::Mutex;
        let snapshot: Arc<Mutex<Vec<AbilityDescriptor>>> = Arc::new(Mutex::new(vec![d("a.x")]));
        let snap_for_provider = Arc::clone(&snapshot);
        let mut reg = LocalAbilityRegistry::new();
        register(&mut reg, move || {
            snap_for_provider.lock().unwrap().clone()
        });
        let handler = reg.get_rpc(ABILITY_LIST_TOOLS).unwrap();

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
        let mut reg = LocalAbilityRegistry::new();
        register(&mut reg, || vec![]);
        let handler = reg.get_rpc(ABILITY_LIST_TOOLS).unwrap();
        let resp = handler(json!({})).unwrap();
        assert_eq!(resp["tools"].as_array().unwrap().len(), 0);
    }
}
