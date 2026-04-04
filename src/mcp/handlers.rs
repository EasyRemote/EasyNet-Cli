// EasyNet CLI — MCP Tool Handlers
// ================================
//
// File: src/mcp/handlers.rs
// Description: Implementation of all 11 Hub-level MCP tool handlers.
//
// Each handler: (bridge, tenant, args) → Result<Value, String>
// The dispatch layer in hub_kit.rs converts to ToolResult.
//
// Author: Silan Hu <silan.hu@u.nus.edu>
// Copyright (c) 2026 EasyNet. All rights reserved.

use easynet_axon::dendrite_bridge::DendriteBridge;
use serde_json::{json, Map, Value};
use crate::eal;
use crate::shared::deploy;
use crate::shared::node::is_online;

type HandlerResult = Result<Value, String>;

fn req<'a>(args: &'a Map<String, Value>, key: &str) -> Result<&'a str, String> {
    args.get(key)
        .and_then(|v| v.as_str())
        .ok_or_else(|| format!("missing: {key}"))
}

pub fn hub_status(br: &DendriteBridge, tenant: &str, _: &Map<String, Value>) -> HandlerResult {
    let nodes = br.list_nodes(tenant, None).map_err(|e| e.to_string())?;
    let on = nodes.iter().filter(|n| is_online(n)).count();
    Ok(json!({"nodes_online": on, "nodes_offline": nodes.len() - on}))
}

pub fn list_devices(br: &DendriteBridge, tenant: &str, args: &Map<String, Value>) -> HandlerResult {
    let nodes = br.list_nodes(tenant, None).map_err(|e| e.to_string())?;
    let sf = args.get("state_filter").and_then(|v| v.as_str());
    let filtered: Vec<_> = nodes
        .into_iter()
        .filter(|n| {
            sf.is_none_or(|f| match f {
                "online" => is_online(n),
                "offline" => !is_online(n),
                _ => true,
            })
        })
        .collect();
    Ok(json!({"devices": filtered, "count": filtered.len()}))
}

pub fn get_device_detail(br: &DendriteBridge, tenant: &str, args: &Map<String, Value>) -> HandlerResult {
    let node_id = req(args, "node_id")?;
    let nodes = br.list_nodes(tenant, None).unwrap_or_default();
    let node = nodes
        .iter()
        .find(|n| n.get("node_id").and_then(|v| v.as_str()) == Some(node_id));
    let abilities = br.list_mcp_tools(tenant, "", node_id).unwrap_or_default();
    Ok(json!({"node": node, "abilities": abilities}))
}

pub fn list_all_abilities(br: &DendriteBridge, tenant: &str, args: &Map<String, Value>) -> HandlerResult {
    let node = args.get("node_id").and_then(|v| v.as_str()).unwrap_or("");
    let pat = args.get("name_pattern").and_then(|v| v.as_str()).unwrap_or("");
    let t = br.list_mcp_tools(tenant, pat, node).map_err(|e| e.to_string())?;
    Ok(json!({"abilities": t, "count": t.len()}))
}

pub fn search_abilities(br: &DendriteBridge, tenant: &str, args: &Map<String, Value>) -> HandlerResult {
    let query = req(args, "query")?;
    let t = br.list_mcp_tools(tenant, query, "").map_err(|e| e.to_string())?;
    Ok(json!({"results": t, "count": t.len()}))
}

pub fn deploy_ability(br: &DendriteBridge, tenant: &str, args: &Map<String, Value>) -> HandlerResult {
    let node_id = req(args, "node_id")?;
    let tool_name = req(args, "tool_name")?;
    let command = req(args, "command")?;
    let desc = args.get("description").and_then(|v| v.as_str()).unwrap_or("");

    let result = deploy::run_pipeline(br, &deploy::DeployParams {
        tenant,
        node_id,
        tool_name,
        ability_name: tool_name,
        version: "1.0.0",
        description: desc,
        command,
        signature: None,
        digest: "",
        payload_bytes: None,
        payload_b64: None,
    })
    .map_err(|e| e.to_string())?;

    Ok(json!({"ok": true, "tool_name": tool_name, "install_id": result.install_id}))
}

pub fn execute_command(br: &DendriteBridge, tenant: &str, args: &Map<String, Value>) -> HandlerResult {
    let node_id = req(args, "node_id")?;
    let command = req(args, "command")?;
    br.call_mcp_tool_with_timeout(
        tenant,
        "session_bridge",
        node_id,
        &json!({"action": "exec", "command": command}),
        Some(60),
    )
    .map_err(|e| e.to_string())
}

pub fn invoke_ability(br: &DendriteBridge, tenant: &str, args: &Map<String, Value>) -> HandlerResult {
    let node_id = req(args, "node_id")?;
    let ability = req(args, "ability")?;
    let arguments = args.get("arguments").cloned().unwrap_or(json!({}));
    br.call_mcp_tool_with_timeout(tenant, ability, node_id, &arguments, Some(60))
        .map_err(|e| e.to_string())
}

pub fn run_mission(br: &DendriteBridge, tenant: &str, args: &Map<String, Value>) -> HandlerResult {
    let source = req(args, "eal_source")?;
    let emit_only = args
        .get("emit_ir_only")
        .and_then(Value::as_bool)
        .unwrap_or(false);

    let program = eal::parser::parse(source).map_err(|e| format!("parse: {e}"))?;
    let ir = eal::planner::compile(&program).map_err(|e| format!("compile: {e}"))?;

    if emit_only {
        return Ok(serde_json::to_value(&ir).unwrap_or(json!(null)));
    }

    let r = eal::interpreter::execute(br, tenant, &ir).map_err(|e| e.to_string())?;
    Ok(json!({
        "ok": true,
        "mission": ir.name,
        "steps_completed": r.steps_completed,
        "steps_failed": r.steps_failed,
        "elapsed_ms": r.total_elapsed_ms,
    }))
}

pub fn manage_device(br: &DendriteBridge, tenant: &str, args: &Map<String, Value>) -> HandlerResult {
    let node_id = req(args, "node_id")?;
    let action = req(args, "action")?;
    match action {
        "drain" => br.drain_node(tenant, node_id, "CLI").map_err(|e| e.to_string()),
        "disconnect" => br.deregister_node(tenant, node_id, "CLI").map_err(|e| e.to_string()),
        _ => Err(format!("unknown action: {action} (supported: drain, disconnect)")),
    }
}

pub fn uninstall_ability(br: &DendriteBridge, tenant: &str, args: &Map<String, Value>) -> HandlerResult {
    let node_id = req(args, "node_id")?;
    let install_id = req(args, "install_id")?;
    let r = br.uninstall_capability(tenant, node_id, install_id).map_err(|e| e.to_string())?;
    Ok(json!({"ok": true, "result": r}))
}
