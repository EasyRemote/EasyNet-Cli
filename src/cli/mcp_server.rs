// EasyNet CLI
// ===========
//
// File: src/cli/mcp_server.rs
// Description: `easynet mcp-server` — Hub-level MCP server on stdio for Claude Code / Codex.
//
// Protocol: JSON-RPC 2.0 over stdin/stdout (MCP specification).
// Provider: HubCaseKit exposes 11 tools covering device management, ability lifecycle,
//           remote execution, and EAL mission orchestration.
//
// Configuration for Claude Code:
//   { "mcpServers": { "easynet": { "command": "easynet", "args": ["mcp-server"] } } }
//
// Author: Silan Hu <silan.hu@u.nus.edu>
// Copyright (c) 2026 EasyNet. All rights reserved.

use anyhow::Context;
use clap::Args;

use crate::shared::config;

#[derive(Debug, Args)]
pub struct McpServerArgs {
    /// Runtime endpoint (auto-detect from ~/.easynet/runtime.json if omitted)
    #[arg(long)]
    pub endpoint: Option<String>,
    /// Tenant ID
    #[arg(long, default_value = "default")]
    pub tenant: String,
    /// Bind node-scoped tools to this `node_id` (device-bound MCP server).
    #[arg(long)]
    pub bound_node: Option<String>,
    /// Allow overriding `node_id` even when `--bound-node` is set.
    #[arg(long)]
    pub allow_node_override: bool,
    /// Agent label (informational; included in server name).
    #[arg(long)]
    pub agent: Option<String>,
    /// Enable the `send_to_agent` MCP tool for agent-to-agent dispatch.
    #[arg(long)]
    pub enable_agent_dispatch: bool,
}

pub fn run(args: McpServerArgs) -> anyhow::Result<()> {
    let ep = match args.endpoint {
        Some(ep) => ep,
        None => config::load()?.endpoint,
    };

    let mut kit = crate::mcp::hub_kit::HubCaseKit::new(ep, args.tenant);
    if let Some(node) = args.bound_node {
        let lock = !args.allow_node_override;
        kit = kit.with_bound_node(node, lock);
    }
    if let Some(agent) = args.agent {
        kit = kit.with_agent(agent);
    }
    if args.enable_agent_dispatch {
        kit = kit.with_agent_dispatch(true);
    }

    let server_name = kit.server_name();

    let server = easynet_axon::mcp::StdioMcpServer::new(kit)
        .with_server_name(server_name)
        .with_server_version(env!("CARGO_PKG_VERSION"));

    server
        .run(std::io::stdin().lock(), &mut std::io::stdout())
        .context("mcp server")
}
