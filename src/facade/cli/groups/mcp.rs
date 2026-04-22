// EasyNet CLI — MCP Group
// =======================
//
// File: src/cli/groups/mcp.rs
// Description: `easynet mcp …` — every command that touches the local
//              MCP surface, in one place.
//
// Verbs:
//
//   serve         — run the local Hub-level MCP server on stdio
//   status        — report whether the local runtime can serve MCP requests
//   install       — register an MCP server entry in a host AI client
//                   (Claude Code or Codex) so it can call this runtime
//   skill-install — register an MCP-shaped skill in a Claude Code project
//
// The MCP server is host-local infrastructure: a stdio bridge that lets
// a co-located AI client (Claude Code, Codex) make EAL-runtime calls.
// It is NOT a network first-class object (see ARCHITECTURE.md §6 —
// interpretation C: a device is a hosting substrate, and the MCP server
// is one of those substrate-local processes).
//
// `install` and `skill-install` were originally top-level commands
// (`easynet mcp-install`, `easynet skill-install`). Pre-release we
// consolidate them under `mcp` so the noun-verb shape matches every
// other group (`device join`, `ability deploy`, `agent add`, …) and so
// `easynet mcp --help` is a single place to discover the surface.
// The legacy flat aliases stay (hidden) until the next minor release —
// see `cli/mod.rs` for the deprecation hints.
//
// Author: Silan Hu <silan.hu@u.nus.edu>
// Copyright (c) 2026 EasyNet. All rights reserved.

use clap::{Args, Subcommand};

use crate::facade::cli::{mcp_install, mcp_server, skill_install};
use crate::persistence::config;
use crate::support::output;

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
    /// Register an MCP server entry in a host AI client config (Claude
    /// Code's `~/.claude/settings.json` or Codex's `~/.codex/config.toml`)
    /// so the client can invoke this runtime's tools.
    Install(mcp_install::McpInstallArgs),
    /// Register an MCP-shaped skill in a Claude Code project so the
    /// agent can invoke it as a slash command.
    #[command(name = "skill-install")]
    SkillInstall(skill_install::SkillInstallArgs),
}

pub fn run(args: McpArgs) -> anyhow::Result<()> {
    match args.action {
        McpAction::Serve(a) => mcp_server::run(a),
        McpAction::Status => run_status(),
        McpAction::Install(a) => mcp_install::run(a),
        McpAction::SkillInstall(a) => skill_install::run(a),
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
