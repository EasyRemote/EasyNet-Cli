// EasyNet CLI — HubCaseKit MCP Provider
// ======================================
//
// File: src/mcp/hub_kit.rs
// Description: McpToolProvider implementation for Hub-level device management.
//
// Design:
// - Caches a single DendriteBridge connection via RefCell (single-threaded stdio model).
// - Reconnects lazily on first tool call.
// - Dispatches tool name → handler function via match.
// - Converts handler Result<Value, String> → ToolResult at the boundary.
//
// Thread Safety: intentionally !Send/!Sync (RefCell). Appropriate for MCP stdio protocol.
//
// Author: Silan Hu <silan.hu@u.nus.edu>
// Copyright (c) 2026 EasyNet. All rights reserved.

use std::cell::RefCell;
use easynet_axon::dendrite_bridge::DendriteBridge;
use easynet_axon::mcp::{McpToolProvider, ToolResult};
use serde_json::{json, Map, Value};
use super::{handlers, specs};

pub struct HubCaseKit {
    endpoint: String,
    tenant: String,
    bound_node: Option<String>,
    lock_bound_node: bool,
    agent: Option<String>,
    agent_dispatch_enabled: bool,
    cached: RefCell<Option<DendriteBridge>>,
}

impl HubCaseKit {
    pub fn new(endpoint: String, tenant: String) -> Self {
        Self {
            endpoint,
            tenant,
            bound_node: None,
            lock_bound_node: false,
            agent: None,
            agent_dispatch_enabled: false,
            cached: RefCell::new(None),
        }
    }

    pub fn with_bound_node(mut self, node_id: String, lock: bool) -> Self {
        self.bound_node = Some(node_id);
        self.lock_bound_node = lock;
        self
    }

    pub fn with_agent_dispatch(mut self, enabled: bool) -> Self {
        self.agent_dispatch_enabled = enabled;
        self
    }

    pub fn with_agent(mut self, agent: String) -> Self {
        self.agent = Some(agent);
        self
    }

    pub fn server_name(&self) -> String {
        match (self.agent.as_deref(), self.bound_node.as_deref()) {
            (Some(agent), Some(node)) => format!("easynet-{agent}-{node}"),
            (Some(agent), None) => format!("easynet-{agent}"),
            (None, Some(node)) => format!("easynet-{node}"),
            (None, None) => "easynet-hub".to_string(),
        }
    }

    fn with_bridge<F>(&self, f: F) -> ToolResult
    where
        F: FnOnce(&DendriteBridge, &str) -> Result<Value, String>,
    {
        let mut slot = self.cached.borrow_mut();
        if slot.is_none() {
            match DendriteBridge::connect(&self.endpoint, crate::shared::BRIDGE_CONNECT_TIMEOUT_MS) {
                Ok(b) => *slot = Some(b),
                Err(e) => {
                    return ToolResult {
                        payload: json!({"ok": false, "error": format!("connect: {e}")}),
                        is_error: true,
                    };
                }
            }
        }
        // SAFETY: slot was just checked/initialized above; unwrap is safe.
        let br = slot.as_ref().expect("bridge just initialized above");
        match f(br, &self.tenant) {
            Ok(v) => ToolResult { payload: v, is_error: false },
            Err(msg) => ToolResult {
                payload: json!({"ok": false, "error": msg}),
                is_error: true,
            },
        }
    }

    fn patch_args_for_bound_node(
        &self,
        tool_name: &str,
        args: &Map<String, Value>,
    ) -> Result<Map<String, Value>, String> {
        let Some(bound) = self.bound_node.as_deref() else {
            return Ok(args.clone());
        };

        if !specs::NODE_SCOPED_TOOLS.contains(&tool_name) {
            return Ok(args.clone());
        }

        let mut patched = args.clone();
        let existing = patched.get("node_id").and_then(|v| v.as_str());
        match existing {
            Some(v) if v == bound => {}
            Some(v) if self.lock_bound_node => {
                return Err(format!(
                    "tool '{tool_name}' is bound to node_id '{bound}', but got '{v}'"
                ));
            }
            Some(_) => {}
            None => {
                patched.insert("node_id".to_string(), Value::String(bound.to_string()));
            }
        }
        Ok(patched)
    }
}

impl McpToolProvider for HubCaseKit {
    fn tool_specs(&self) -> Vec<Value> {
        let mut specs = specs::tool_specs(self.bound_node.as_deref(), self.lock_bound_node);
        if self.agent_dispatch_enabled {
            specs.push(specs::send_to_agent_spec());
        }
        specs
    }

    fn handle_tool_call(&self, name: &str, args: &Map<String, Value>) -> ToolResult {
        let patched = match self.patch_args_for_bound_node(name, args) {
            Ok(v) => v,
            Err(msg) => {
                return ToolResult {
                    payload: json!({"ok": false, "error": msg}),
                    is_error: true,
                };
            }
        };

        // Pre-bridge fast path: send_to_agent doesn't need a DendriteBridge.
        if name == "send_to_agent" {
            if !self.agent_dispatch_enabled {
                return ToolResult {
                    payload: json!({"ok": false, "error": "agent dispatch not enabled. Start MCP server with --enable-agent-dispatch."}),
                    is_error: true,
                };
            }
            return match handlers::send_to_agent(&patched) {
                Ok(v) => ToolResult { payload: v, is_error: false },
                Err(msg) => ToolResult {
                    payload: json!({"ok": false, "error": msg}),
                    is_error: true,
                },
            };
        }

        self.with_bridge(|br, tenant| match name {
            "hub_status" => handlers::hub_status(br, tenant, &patched),
            "list_devices" => handlers::list_devices(br, tenant, &patched),
            "get_device_detail" => handlers::get_device_detail(br, tenant, &patched),
            "list_all_abilities" => handlers::list_all_abilities(br, tenant, &patched),
            "search_abilities" => handlers::search_abilities(br, tenant, &patched),
            "list_a2a_agents" => handlers::list_a2a_agents(br, tenant, &patched),
            "get_a2a_agent_card" => handlers::get_a2a_agent_card(br, tenant, &patched),
            "send_a2a_task" => handlers::send_a2a_task(br, tenant, &patched),
            "deploy_ability" => handlers::deploy_ability(br, tenant, &patched),
            "execute_command" => handlers::execute_command(br, tenant, &patched),
            "invoke_ability" => handlers::invoke_ability(br, tenant, &patched),
            "run_mission" => handlers::run_mission(br, tenant, &patched),
            "manage_device" => handlers::manage_device(br, tenant, &patched),
            "uninstall_ability" => handlers::uninstall_ability(br, tenant, &patched),
            _ => Err(format!("unknown tool: {name}")),
        })
    }
}
