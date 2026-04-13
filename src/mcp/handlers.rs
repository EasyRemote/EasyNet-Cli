// EasyNet CLI — MCP Tool Handlers
// ================================
//
// File: src/mcp/handlers.rs
// Description: Implementation of Hub-level MCP tool handlers.
//
// Each handler: (bridge, tenant, args) → Result<Value, String>
// The dispatch layer in hub_kit.rs converts to ToolResult.
//
// Author: Silan Hu <silan.hu@u.nus.edu>
// Copyright (c) 2026 EasyNet. All rights reserved.

use easynet_axon::dendrite_bridge::DendriteBridge;
use serde_json::{json, Map, Value};
use crate::eal;
use crate::shared::node::is_online;
use crate::shared::config;

type HandlerResult = Result<Value, String>;

fn req<'a>(args: &'a Map<String, Value>, key: &str) -> Result<&'a str, String> {
    args.get(key)
        .and_then(|v| v.as_str())
        .ok_or_else(|| format!("missing: {key}"))
}

/// Wrap a shell command in a Python subprocess template that returns JSON.
///
/// Uses `python3 -` (stdin) to avoid shell-quoting issues with `-c`.
/// The command is JSON-encoded and embedded as a Python string literal,
/// so no shell interpretation of user input can occur.
///
/// Mirrors `easynet_axon` remote-control preset semantics (Python + Rust).
fn build_python_subprocess_template(command: &str) -> String {
    let quoted = serde_json::to_string(command).unwrap_or_else(|_| "\"\"".to_string());
    let script = format!(
        "import json,subprocess,sys; \
         cmd = {quoted}; \
         proc = subprocess.run(['/bin/sh', '-c', cmd], text=True, capture_output=True); \
         combined = (proc.stdout + proc.stderr).strip(); \
         print(json.dumps({{'entries': [combined], 'command': cmd, \
         'exit_code': proc.returncode, 'stdout': proc.stdout, 'stderr': proc.stderr}}))"
    );
    format!("printf '%s' {json_script} | python3 -",
        json_script = shell_escape_posix(&script))
}

/// POSIX-safe shell escaping: wraps in single quotes, escapes embedded single quotes.
fn shell_escape_posix(raw: &str) -> String {
    format!("'{}'", raw.replace('\'', "'\"'\"'"))
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
    let nodes = br.list_nodes(tenant, None).map_err(|e| e.to_string())?;
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

pub fn list_a2a_agents(br: &DendriteBridge, tenant: &str, args: &Map<String, Value>) -> HandlerResult {
    let tag_strings: Vec<String> = args
        .get("tags")
        .and_then(|v| v.as_array())
        .map(|arr| {
            arr.iter()
                .filter_map(|v| v.as_str())
                .map(ToString::to_string)
                .collect()
        })
        .unwrap_or_default();
    let tag_refs: Vec<&str> = tag_strings.iter().map(String::as_str).collect();

    let owner_id = args
        .get("owner_id")
        .and_then(|v| v.as_str())
        .filter(|s| !s.trim().is_empty())
        .map(ToString::to_string);
    let limit = args.get("limit").and_then(Value::as_i64).unwrap_or(100) as u32;

    let agents = br
        .list_a2a_agents(tenant, &tag_refs, owner_id.as_deref(), limit)
        .map_err(|e| e.to_string())?;
    Ok(json!({"agents": agents, "count": agents.len()}))
}

pub fn get_a2a_agent_card(br: &DendriteBridge, tenant: &str, args: &Map<String, Value>) -> HandlerResult {
    let node_id = req(args, "node_id")?;
    br.get_a2a_agent_card(tenant, node_id)
        .map_err(|e| e.to_string())
}

pub fn send_a2a_task(br: &DendriteBridge, tenant: &str, args: &Map<String, Value>) -> HandlerResult {
    let target_agent_id = req(args, "target_agent_id")?;
    let skill_id = req(args, "skill_id")?;

    let input_json = args.get("input_json").cloned().unwrap_or(json!({}));
    let task_id = args
        .get("task_id")
        .and_then(|v| v.as_str())
        .filter(|s| !s.trim().is_empty())
        .map(ToString::to_string);
    let idempotency_key = args
        .get("idempotency_key")
        .and_then(|v| v.as_str())
        .filter(|s| !s.trim().is_empty())
        .map(ToString::to_string);

    br.send_a2a_task_with_options(
        tenant,
        target_agent_id,
        skill_id,
        input_json,
        task_id.as_deref(),
        idempotency_key.as_deref(),
    )
    .map_err(|e| e.to_string())
}

pub fn deploy_ability(br: &DendriteBridge, tenant: &str, args: &Map<String, Value>) -> HandlerResult {
    let node_id = req(args, "node_id")?;
    let tool_name = req(args, "tool_name")?;
    let command = req(args, "command")?;
    let desc = args.get("description").and_then(|v| v.as_str()).unwrap_or("");

    let signature = config::load_credentials()
        .ok()
        .map(|c| c.deploy_signature)
        .filter(|s| !s.trim().is_empty())
        .unwrap_or_else(|| {
            eprintln!("deploy_ability: no deploy signature, using ephemeral placeholder (dev only)");
            easynet_axon::EPHEMERAL_SIGNATURE.to_string()
        });

    let mut pkg_args = Map::<String, Value>::new();
    pkg_args.insert("ability_name".to_string(), Value::String(tool_name.to_string()));
    pkg_args.insert("tool_name".to_string(), Value::String(tool_name.to_string()));
    pkg_args.insert("description".to_string(), Value::String(desc.to_string()));
    pkg_args.insert(
        "command_template".to_string(),
        Value::String(build_python_subprocess_template(command)),
    );
    pkg_args.insert("version".to_string(), Value::String("1.0.0".to_string()));

    let descriptor = easynet_axon::ability::build_deploy_package(&pkg_args, &signature)
        .map_err(|e| e.to_string())?;
    let deploy = easynet_axon::ability::deploy_package(br, tenant, node_id, &descriptor, true)
        .map_err(|e| e.to_string())?;
    let deploy_value = easynet_axon::presets::remote_control::deploy_to_value(&deploy, &descriptor);

    Ok(json!({
        "ok": true,
        "node_id": node_id,
        "tool_name": descriptor.tool_name,
        "install_id": deploy.install_id,
        "deploy": deploy_value,
    }))
}

pub fn execute_command(br: &DendriteBridge, tenant: &str, args: &Map<String, Value>) -> HandlerResult {
    let node_id = req(args, "node_id")?;
    let command = req(args, "command")?;
    br.call_mcp_tool_with_timeout(
        tenant,
        "session_bridge",
        node_id,
        &json!({"action": "exec", "command": command}),
        Some(60_000),
    )
    .map_err(|e| e.to_string())
}

pub fn invoke_ability(br: &DendriteBridge, tenant: &str, args: &Map<String, Value>) -> HandlerResult {
    let node_id = req(args, "node_id")?;
    let ability = req(args, "ability")?;
    let arguments = args.get("arguments").cloned().unwrap_or(json!({}));
    br.call_mcp_tool_with_timeout(tenant, ability, node_id, &arguments, Some(60_000))
        .map_err(|e| e.to_string())
}

/// Execute a mission reusing a shared BridgePool for parallel phase execution.
///
/// This is the primary path for MCP server calls. The pool is persisted across
/// the MCP session lifetime, so connections are amortized across missions.
pub fn run_mission_with_pool(
    pool: std::sync::Arc<crate::shared::bridge_pool::BridgePool>,
    tenant: &str,
    args: &Map<String, Value>,
) -> HandlerResult {
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

    let r = eal::interpreter::execute_pooled_shared(pool, tenant, &ir)
        .map_err(|e| e.to_string())?;
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

/// Send a prompt to a registered AI agent (pre-bridge fast path — no DendriteBridge needed).
pub fn send_to_agent(args: &Map<String, Value>) -> HandlerResult {
    use crate::agent::dispatch;
    use crate::shared::agents;

    let agent_name = req(args, "agent")?;
    let prompt = req(args, "prompt")?;
    let context = args.get("context").and_then(|v| v.as_str());

    let registry = agents::load_agents().map_err(|e| format!("load agents: {e}"))?;
    let entry = registry.agents.get(agent_name)
        .ok_or_else(|| format!("agent '{agent_name}' not found in registry"))?;

    let response = dispatch::send_to_agent(agent_name, entry, prompt, context, None, None)
        .map_err(|e| format!("agent dispatch: {e}"))?;

    Ok(json!({
        "ok": true,
        "agent": response.agent,
        "content": response.content,
        "model": response.model,
        "duration_ms": response.duration_ms,
    }))
}
