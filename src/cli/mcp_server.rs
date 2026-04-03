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
}

pub fn run(args: McpServerArgs) -> anyhow::Result<()> {
    let ep = match args.endpoint {
        Some(ep) => ep,
        None => config::load()?.endpoint,
    };

    let kit = crate::mcp::hub_kit::HubCaseKit::new(ep, args.tenant);

    let server = easynet_axon::mcp::StdioMcpServer::new(kit)
        .with_server_name("easynet-hub")
        .with_server_version(env!("CARGO_PKG_VERSION"));

    server
        .run(std::io::stdin().lock(), &mut std::io::stdout())
        .map_err(|e| anyhow::anyhow!("mcp server: {e}"))
}
