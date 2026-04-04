// EasyNet CLI — MCP Tool Specifications
// ======================================
//
// File: src/mcp/specs.rs
// Description: JSON Schema definitions for all 11 Hub-level MCP tools.
//
// Author: Silan Hu <silan.hu@u.nus.edu>
// Copyright (c) 2026 EasyNet. All rights reserved.

use serde_json::{json, Value};

pub fn tool_specs() -> Vec<Value> {
    vec![
        json!({
            "name": "hub_status",
            "description": "Show Hub connection status, node/ability counts.",
            "inputSchema": { "type": "object", "properties": {} }
        }),
        json!({
            "name": "list_devices",
            "description": "List all federation devices.",
            "inputSchema": {
                "type": "object",
                "properties": {
                    "state_filter": { "type": "string" }
                }
            }
        }),
        json!({
            "name": "get_device_detail",
            "description": "Device info + installed abilities.",
            "inputSchema": {
                "type": "object",
                "properties": {
                    "node_id": { "type": "string" }
                },
                "required": ["node_id"]
            }
        }),
        json!({
            "name": "list_all_abilities",
            "description": "List abilities across all nodes.",
            "inputSchema": {
                "type": "object",
                "properties": {
                    "node_id": { "type": "string" },
                    "name_pattern": { "type": "string" }
                }
            }
        }),
        json!({
            "name": "search_abilities",
            "description": "Search abilities by name/tags.",
            "inputSchema": {
                "type": "object",
                "properties": {
                    "query": { "type": "string" }
                },
                "required": ["query"]
            }
        }),
        json!({
            "name": "deploy_ability",
            "description": "Publish+install+activate an ability.",
            "inputSchema": {
                "type": "object",
                "properties": {
                    "node_id": { "type": "string" },
                    "tool_name": { "type": "string" },
                    "description": { "type": "string" },
                    "command": { "type": "string" }
                },
                "required": ["node_id", "tool_name", "command"]
            }
        }),
        json!({
            "name": "execute_command",
            "description": "One-shot command on remote device.",
            "inputSchema": {
                "type": "object",
                "properties": {
                    "node_id": { "type": "string" },
                    "command": { "type": "string" }
                },
                "required": ["node_id", "command"]
            }
        }),
        json!({
            "name": "invoke_ability",
            "description": "Invoke ability on a federated node.",
            "inputSchema": {
                "type": "object",
                "properties": {
                    "node_id": { "type": "string" },
                    "ability": { "type": "string" },
                    "arguments": { "type": "object" }
                },
                "required": ["node_id", "ability"]
            }
        }),
        json!({
            "name": "run_mission",
            "description": "Compile and execute an EAL program.",
            "inputSchema": {
                "type": "object",
                "properties": {
                    "eal_source": { "type": "string" },
                    "emit_ir_only": { "type": "boolean" }
                },
                "required": ["eal_source"]
            }
        }),
        json!({
            "name": "manage_device",
            "description": "Drain or disconnect a device.",
            "inputSchema": {
                "type": "object",
                "properties": {
                    "node_id": { "type": "string" },
                    "action": {
                        "type": "string",
                        "enum": ["drain", "disconnect"]
                    }
                },
                "required": ["node_id", "action"]
            }
        }),
        json!({
            "name": "uninstall_ability",
            "description": "Remove ability from device.",
            "inputSchema": {
                "type": "object",
                "properties": {
                    "node_id": { "type": "string" },
                    "install_id": { "type": "string" }
                },
                "required": ["node_id", "install_id"]
            }
        }),
    ]
}
