// EasyNet CLI — `easynet mcp_server`
// ===================================
//
// File: src/facade/cli/mcp_server.rs
//
// Argument parsing + foreground-server-loop entry point. The actual
// server construction lives in
// `runtime::agents::profiles::mcp::build_stdio_server`; this file is
// intentionally thin so the MCP edge adapter has exactly one
// construction site, shared with the `easynet start --mcp` path.
//
// RFC-001 §A3 quarantine (P4.8d + P4.9): every tool call routes
// through the in-process AbilityProxy via InvokeMcpProvider. No
// independent dispatch, no duplicate catalog, no direct bridge
// calls.
//
// Author: Silan Hu <silan.hu@u.nus.edu>
// Copyright (c) 2026 EasyNet. All rights reserved.

use anyhow::Context;
use clap::Args;

#[derive(Debug, Args)]
pub struct McpServerArgs {
    /// Tenant ID. Surfaces in audit logs only; the actual dispatch
    /// honours whatever tenant the loaded credentials carry.
    #[arg(long, default_value = "default")]
    pub tenant: String,
    /// Agent label (informational; included in server name).
    #[arg(long)]
    pub agent: Option<String>,
}

pub fn run(args: McpServerArgs) -> anyhow::Result<()> {
    let server_name = format!("easynet-mcp-{}", args.agent.as_deref().unwrap_or("device"));
    let config = crate::runtime::agents::profiles::mcp::StdioServerConfig {
        server_name: server_name.clone(),
        tenant_id: args.tenant.clone(),
        // Thread --agent through so the workspace MCP server
        // also exposes the agent's per-workspace abilities.
        // Without this, an agent's own ability TOMLs (declared
        // at <workspace>/abilities/) would be invisible to the
        // LLM running inside that workspace.
        agent_name: args.agent.clone(),
    };
    let configured = crate::runtime::agents::profiles::mcp::build_stdio_server(&config);

    eprintln!(
        "[easynet mcp] tenant={} agent={} advertising {} tools (RFC-001 §A3 edge adapter)",
        args.tenant,
        args.agent.as_deref().unwrap_or("?"),
        configured.descriptor_count(),
    );

    let server = easynet_axon::mcp::StdioMcpServer::new(configured.provider)
        .with_server_name(configured.server_name)
        .with_server_version(env!("CARGO_PKG_VERSION"));
    server
        .run(std::io::stdin().lock(), &mut std::io::stdout())
        .context("mcp server")
}
