// EasyNet CLI — `easynet mcp_server`
// ===================================
//
// File: src/cli/mcp_server.rs
//
// Argument parsing + foreground-server-loop entry point. The actual
// server construction lives in
// `daemon::ability::catalog::profiles::mcp::build_stdio_server`; this file is
// intentionally thin so the MCP edge adapter has exactly one
// construction site, shared with the `easynet runtime start --mcp` path.
//
// RFC-001 §A3 quarantine (P4.8d + P4.9): every tool call routes
// through the daemon-hosted Axon ability surface. No independent
// dispatch, no duplicate catalog, no direct bridge calls.
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
    /// Agent workspace label used in the server name and chat-recursion filter.
    #[arg(long)]
    pub agent: Option<String>,
}

pub fn run(args: McpServerArgs) -> anyhow::Result<()> {
    let server_name = format!("easynet-mcp-{}", args.agent.as_deref().unwrap_or("device"));
    let config = crate::daemon::ability::catalog::profiles::mcp::StdioServerConfig {
        server_name: server_name.clone(),
        tenant_id: args.tenant.clone(),
        // The daemon catalog remains authoritative. The workspace label only
        // enables the chat-recursion filter in the MCP projection.
        agent_name: args.agent.clone(),
    };
    let configured = crate::daemon::ability::catalog::profiles::mcp::build_stdio_server(&config)?;

    eprintln!(
        "[easynet mcp] tenant={} agent={} advertising {} tools (RFC-001 §A3 edge adapter)",
        args.tenant,
        args.agent.as_deref().unwrap_or("?"),
        configured.descriptor_count(),
    );

    let server = crate::daemon::execution::mcp::stdio::StdioMcpServer::new(configured.provider)
        .with_server_name(configured.server_name)
        .with_server_version(env!("CARGO_PKG_VERSION"));
    server
        .run(std::io::stdin().lock(), &mut std::io::stdout())
        .context("mcp server")
}
