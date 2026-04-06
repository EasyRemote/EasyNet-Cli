// EasyNet CLI — MCP Group
// =======================
//
// File: src/cli/groups/mcp.rs
// Description: `easynet mcp …` — the local MCP server *process*. Nothing
//              else lives here.
//
// Why so small?
//   The MCP server is host-local infrastructure: it is the stdio bridge
//   that lets a co-located AI client (Claude Code, Codex) make EAL-runtime
//   calls. It is NOT a network first-class object (see ARCHITECTURE.md
//   §6 — interpretation C: device is a hosting substrate, and the MCP
//   server is one of those substrate-local processes).
//
//   Therefore the only verbs that belong here are the ones that touch the
//   local server process itself:
//
//     serve   — run the stdio MCP server
//     status  — report whether the local runtime can answer MCP requests
//
//   Skill / client config installation (`mcp-install`, `skill-install`)
//   stays at the legacy top level for now; their final home depends on
//   open question §13.6 in the consensus spec.
//
// Author: Silan Hu <silan.hu@u.nus.edu>
// Copyright (c) 2026 EasyNet. All rights reserved.

use clap::{Args, Subcommand};

use crate::cli::mcp_server;
use crate::shared::{config, output};

#[derive(Debug, Args)]
pub struct McpArgs {
    #[command(subcommand)]
    pub action: McpAction,
}

#[derive(Debug, Subcommand)]
pub enum McpAction {
    /// Run the local Hub-level MCP server on stdio.
    Serve(mcp_server::McpServerArgs),
    /// Report whether the local runtime can serve MCP requests.
    Status,
}

pub fn run(args: McpArgs) -> anyhow::Result<()> {
    match args.action {
        McpAction::Serve(a) => mcp_server::run(a),
        McpAction::Status => run_status(),
    }
}

fn run_status() -> anyhow::Result<()> {
    match config::load() {
        Ok(state) => {
            output::success("runtime reachable");
            output::detail("endpoint", &state.endpoint);
            output::detail("tenant", state.tenant.as_deref().unwrap_or("default"));
            output::info("`easynet mcp serve` will route MCP calls through this runtime.");
        }
        Err(_) => {
            output::warn("runtime not running — `easynet mcp serve` would fail");
            output::info("Start it with `easynet runtime start`.");
        }
    }
    Ok(())
}
