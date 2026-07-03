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
use serde_json::json;

use crate::cli::commands::skill_install;
use crate::cli::mcp::{install as mcp_install, server as mcp_server};
use crate::daemon::persistence::config;
use crate::support::platform::local_invoke::invoke_local_ability;
use crate::support::platform::output;

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
    let state = config::load().ok();
    match invoke_local_ability("observe.health", json!({"source": "mcp.status"})) {
        Ok(_) => {
            output::success("local daemon MCP surface reachable");
            if let Some(state) = state {
                output::detail("mode", "daemon-only");
                output::detail("grpc_socket", &state.endpoint);
                output::detail("tenant", state.tenant.as_deref().unwrap_or("default"));
            }
            output::info("'easynet mcp serve' will route MCP calls through this daemon.");
        }
        Err(e) => match state {
            Some(state) if state.uses_bridge() => {
                output::warn(
                    "runtime metadata exists, but no local daemon MCP surface is available",
                );
                output::detail("bridge_endpoint", &state.endpoint);
                output::detail("tenant", state.tenant.as_deref().unwrap_or("default"));
                output::info(
                    "'easynet mcp serve' needs easynet-daemon. Hub/bridge-only mode does not provide it.",
                );
            }
            Some(state) => {
                output::warn("runtime metadata exists, but the local daemon is not responding");
                output::detail("grpc_socket", &state.endpoint);
                output::detail("tenant", state.tenant.as_deref().unwrap_or("default"));
                output::info(&format!("health probe failed: {e}"));
            }
            None => {
                output::warn("runtime not running — 'easynet mcp serve' would fail");
                output::info("Start it with 'easynet runtime start'.");
            }
        },
    }
    Ok(())
}
