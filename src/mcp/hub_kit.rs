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
    cached: RefCell<Option<DendriteBridge>>,
}

impl HubCaseKit {
    pub fn new(endpoint: String, tenant: String) -> Self {
        Self {
            endpoint,
            tenant,
            cached: RefCell::new(None),
        }
    }

    fn with_bridge<F>(&self, f: F) -> ToolResult
    where
        F: FnOnce(&DendriteBridge, &str) -> Result<Value, String>,
    {
        let mut slot = self.cached.borrow_mut();
        if slot.is_none() {
            match DendriteBridge::connect(&self.endpoint, 5000) {
                Ok(b) => *slot = Some(b),
                Err(e) => {
                    return ToolResult {
                        payload: json!({"ok": false, "error": format!("connect: {e}")}),
                        is_error: true,
                    };
                }
            }
        }
        match f(slot.as_ref().unwrap(), &self.tenant) {
            Ok(v) => ToolResult { payload: v, is_error: false },
            Err(msg) => ToolResult {
                payload: json!({"ok": false, "error": msg}),
                is_error: true,
            },
        }
    }
}

impl McpToolProvider for HubCaseKit {
    fn tool_specs(&self) -> Vec<Value> {
        specs::tool_specs()
    }

    fn handle_tool_call(&self, name: &str, args: &Map<String, Value>) -> ToolResult {
        self.with_bridge(|br, tenant| match name {
            "hub_status" => handlers::hub_status(br, tenant, args),
            "list_devices" => handlers::list_devices(br, tenant, args),
            "get_device_detail" => handlers::get_device_detail(br, tenant, args),
            "list_all_abilities" => handlers::list_all_abilities(br, tenant, args),
            "search_abilities" => handlers::search_abilities(br, tenant, args),
            "deploy_ability" => handlers::deploy_ability(br, tenant, args),
            "execute_command" => handlers::execute_command(br, tenant, args),
            "invoke_ability" => handlers::invoke_ability(br, tenant, args),
            "run_mission" => handlers::run_mission(br, tenant, args),
            "manage_device" => handlers::manage_device(br, tenant, args),
            "uninstall_ability" => handlers::uninstall_ability(br, tenant, args),
            _ => Err(format!("unknown tool: {name}")),
        })
    }
}
