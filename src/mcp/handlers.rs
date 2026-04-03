// EasyNet CLI — MCP Tool Handlers
// ================================
//
// File: src/mcp/handlers.rs
// Description: Implementation of all 11 Hub-level MCP tool handlers.
//
// Each handler: (bridge, tenant, args) → ToolResult
//
// Handler Map:
//   hub_status         → list_nodes count
//   list_devices       → list_nodes + state filter
//   get_device_detail  → list_nodes + list_mcp_tools for one node
//   list_all_abilities → list_mcp_tools with optional node/pattern filter
//   search_abilities   → list_mcp_tools with name pattern
//   deploy_ability     → publish_capability + install_capability + activate_capability
//   execute_command    → call_mcp_tool_with_args("session_bridge", ...)
//   invoke_ability     → call_mcp_tool_with_args(ability, node, args)
//   run_mission        → eal::parser + planner + interpreter pipeline
//   manage_device      → drain_node / deregister_node
//   uninstall_ability  → uninstall_capability
//
// Author: Silan Hu <silan.hu@u.nus.edu>
// Copyright (c) 2026 EasyNet. All rights reserved.

use easynet_axon::dendrite_bridge::DendriteBridge;
use easynet_axon::mcp::ToolResult;
use serde_json::{json, Map, Value};
use crate::eal;

fn ok(v: Value) -> ToolResult { ToolResult { payload: v, is_error: false } }
fn err(msg: &str) -> ToolResult { ToolResult { payload: json!({"ok": false, "error": msg}), is_error: true } }
fn req<'a>(args: &'a Map<String, Value>, key: &str) -> Result<&'a str, ToolResult> {
    args.get(key).and_then(|v| v.as_str()).ok_or_else(|| err(&format!("missing: {key}")))
}
fn is_online(n: &Value) -> bool { n.get("state").and_then(|s| s.as_str()).map(|s| s == "HEALTHY" || s == "ONLINE").unwrap_or(false) }

pub fn hub_status(br: &DendriteBridge, tenant: &str, _: &Map<String, Value>) -> ToolResult {
    match br.list_nodes(tenant, None) {
        Ok(nodes) => { let on = nodes.iter().filter(|n| is_online(n)).count(); ok(json!({"nodes_online": on, "nodes_offline": nodes.len() - on})) }
        Err(e) => err(&format!("{e}")),
    }
}

pub fn list_devices(br: &DendriteBridge, tenant: &str, args: &Map<String, Value>) -> ToolResult {
    match br.list_nodes(tenant, None) {
        Ok(nodes) => {
            let sf = args.get("state_filter").and_then(|v| v.as_str());
            let filtered: Vec<_> = nodes.into_iter().filter(|n| sf.map_or(true, |f| match f { "online" => is_online(n), "offline" => !is_online(n), _ => true })).collect();
            ok(json!({"devices": filtered, "count": filtered.len()}))
        }
        Err(e) => err(&format!("{e}")),
    }
}

pub fn get_device_detail(br: &DendriteBridge, tenant: &str, args: &Map<String, Value>) -> ToolResult {
    let node_id = match req(args, "node_id") { Ok(v) => v, Err(e) => return e };
    let nodes = br.list_nodes(tenant, None).unwrap_or_default();
    let node = nodes.iter().find(|n| n.get("node_id").and_then(|v| v.as_str()) == Some(node_id));
    let abilities = br.list_mcp_tools(tenant, "", node_id).unwrap_or_default();
    ok(json!({"node": node, "abilities": abilities}))
}

pub fn list_all_abilities(br: &DendriteBridge, tenant: &str, args: &Map<String, Value>) -> ToolResult {
    let node = args.get("node_id").and_then(|v| v.as_str()).unwrap_or("");
    let pat = args.get("name_pattern").and_then(|v| v.as_str()).unwrap_or("");
    match br.list_mcp_tools(tenant, pat, node) {
        Ok(t) => ok(json!({"abilities": t, "count": t.len()})),
        Err(e) => err(&format!("{e}")),
    }
}

pub fn search_abilities(br: &DendriteBridge, tenant: &str, args: &Map<String, Value>) -> ToolResult {
    let query = match req(args, "query") { Ok(v) => v, Err(e) => return e };
    match br.list_mcp_tools(tenant, query, "") {
        Ok(t) => ok(json!({"results": t, "count": t.len()})),
        Err(e) => err(&format!("{e}")),
    }
}

pub fn deploy_ability(br: &DendriteBridge, tenant: &str, args: &Map<String, Value>) -> ToolResult {
    let node_id = match req(args, "node_id") { Ok(v) => v, Err(e) => return e };
    let tool_name = match req(args, "tool_name") { Ok(v) => v, Err(e) => return e };
    let command = match req(args, "command") { Ok(v) => v, Err(e) => return e };
    let desc = args.get("description").and_then(|v| v.as_str()).unwrap_or("");
    let meta = json!({"mcp.tool_name": tool_name, "mcp.description": desc, "axon.exec.command": command});
    if let Err(e) = br.publish_capability(tenant, tool_name, tool_name, "1.0.0", "", Some("__AXON_EPHEMERAL_DO_NOT_USE_IN_PROD__"), &[], meta, None, None, None, None, None) { return err(&format!("publish: {e}")); }
    let ir = match br.install_capability(tenant, node_id, tool_name, "1.0.0", "", false, "host", 30) { Ok(r) => r, Err(e) => return err(&format!("install: {e}")) };
    let iid = ir.get("install_id").and_then(|v| v.as_str()).unwrap_or("");
    if let Err(e) = br.activate_capability(tenant, node_id, iid) { return err(&format!("activate: {e}")); }
    ok(json!({"ok": true, "tool_name": tool_name, "install_id": iid}))
}

pub fn execute_command(br: &DendriteBridge, tenant: &str, args: &Map<String, Value>) -> ToolResult {
    let node_id = match req(args, "node_id") { Ok(v) => v, Err(e) => return e };
    let command = match req(args, "command") { Ok(v) => v, Err(e) => return e };
    match br.call_mcp_tool_with_args(tenant, "session_bridge", node_id, &json!({"command": command})) {
        Ok(r) => ok(r), Err(e) => err(&format!("{e}")),
    }
}

pub fn invoke_ability(br: &DendriteBridge, tenant: &str, args: &Map<String, Value>) -> ToolResult {
    let node_id = match req(args, "node_id") { Ok(v) => v, Err(e) => return e };
    let ability = match req(args, "ability") { Ok(v) => v, Err(e) => return e };
    let arguments = args.get("arguments").cloned().unwrap_or(json!({}));
    match br.call_mcp_tool_with_args(tenant, ability, node_id, &arguments) {
        Ok(r) => ok(r), Err(e) => err(&format!("{e}")),
    }
}

pub fn run_mission(br: &DendriteBridge, tenant: &str, args: &Map<String, Value>) -> ToolResult {
    let source = match req(args, "eal_source") { Ok(v) => v, Err(e) => return e };
    let emit_only = args.get("emit_ir_only").and_then(|v| v.as_bool()).unwrap_or(false);
    let program = match eal::parser::parse(source) { Ok(p) => p, Err(e) => return err(&format!("parse: {e}")) };
    let ir = match eal::planner::compile(&program) { Ok(i) => i, Err(e) => return err(&format!("compile: {e}")) };
    if emit_only { return ok(serde_json::to_value(&ir).unwrap_or(json!(null))); }
    match eal::interpreter::execute(br, tenant, &ir) {
        Ok(r) => ok(json!({"ok": true, "mission": ir.name, "steps_completed": r.steps_completed, "steps_failed": r.steps_failed, "elapsed_ms": r.total_elapsed_ms})),
        Err(e) => err(&format!("{e}")),
    }
}

pub fn manage_device(br: &DendriteBridge, tenant: &str, args: &Map<String, Value>) -> ToolResult {
    let node_id = match req(args, "node_id") { Ok(v) => v, Err(e) => return e };
    let action = match req(args, "action") { Ok(v) => v, Err(e) => return e };
    match action {
        "drain" => match br.drain_node(tenant, node_id, "CLI") { Ok(r) => ok(r), Err(e) => err(&format!("{e}")) },
        "undrain" => err("undrain is not supported by the current Axon SDK/FFI"),
        "disconnect" => match br.deregister_node(tenant, node_id, "CLI") { Ok(r) => ok(r), Err(e) => err(&format!("{e}")) },
        _ => err(&format!("unknown action: {action}")),
    }
}

pub fn uninstall_ability(br: &DendriteBridge, tenant: &str, args: &Map<String, Value>) -> ToolResult {
    let node_id = match req(args, "node_id") { Ok(v) => v, Err(e) => return e };
    let install_id = match req(args, "install_id") { Ok(v) => v, Err(e) => return e };
    match br.uninstall_capability(tenant, node_id, install_id) {
        Ok(r) => ok(json!({"ok": true, "result": r})), Err(e) => err(&format!("{e}")),
    }
}
