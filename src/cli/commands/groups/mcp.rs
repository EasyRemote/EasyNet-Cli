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
// `install` and `skill-install` live only under `mcp` so the noun-verb
// shape matches every other group (`device join`, `ability deploy`,
// `agent add`, …) and so `easynet mcp --help` is a single place to
// discover the surface. No top-level spellings are registered for
// these verbs.
//
// Author: Silan Hu <silan.hu@u.nus.edu>
// Copyright (c) 2026 EasyNet. All rights reserved.

use clap::{Args, Subcommand};
use serde_json::json;

use crate::cli::commands::skill_install;
use crate::cli::mcp::{install as mcp_install, server as mcp_server};
use crate::daemon::lifecycle::RuntimeStatusReport;
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
    let report = crate::daemon::lifecycle::RuntimeLifecycleService::new().status()?;
    match invoke_local_ability("observe.health", json!({"source": "mcp.status"})) {
        Ok(_) => {
            output::success("local daemon MCP surface reachable");
            render_lifecycle_details(&report);
            output::info("'easynet mcp serve' will route MCP calls through this daemon.");
        }
        Err(e) => {
            if report.projection().is_some() {
                output::warn("runtime metadata exists, but the local daemon is not responding");
                render_lifecycle_details(&report);
                output::info(&format!("health probe failed: {e}"));
            } else if report.daemon().has_daemon_fact() {
                output::warn("daemon lifecycle facts exist, but MCP health probe failed");
                render_lifecycle_details(&report);
                output::info(&format!("health probe failed: {e}"));
            } else {
                output::warn("runtime not running — 'easynet mcp serve' would fail");
                output::info("Start it with 'easynet runtime start'.");
            }
        }
    }
    Ok(())
}

fn render_lifecycle_details(report: &RuntimeStatusReport) {
    if let Some(projection) = report.projection() {
        let state = projection.as_runtime_state();
        output::detail("mode", "daemon-only");
        output::detail("grpc_socket", &state.endpoint);
        output::detail("tenant", state.tenant.as_deref().unwrap_or("default"));
        return;
    }
    if let Some(discovery) = report.daemon().control_discovery() {
        if let Some(identity) = discovery.daemon_identity.as_ref() {
            output::detail("mode", &identity.mode);
        }
        if let Some(endpoint) = discovery.invocation_endpoint.as_ref() {
            output::detail("grpc_socket", &endpoint.display().to_string());
        }
        if let Some(socket) = discovery.socket_path.as_ref() {
            output::detail("control_socket", &socket.display().to_string());
        }
        output::detail("pid", &discovery.pid.to_string());
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::cli::commands::test_support::HomeGuard;
    use crate::daemon::persistence::config;

    #[test]
    fn mcp_status_rejects_malformed_runtime_projection() {
        let _home = HomeGuard::new();
        std::fs::create_dir_all(config::state_dir()).expect("state dir");
        std::fs::write(config::runtime_state_path(), "{ not json").expect("runtime projection");

        let error = run_status().expect_err("malformed runtime projection must fail MCP status");

        assert!(
            error.to_string().contains("load runtime projection failed"),
            "wrong error: {error:#}"
        );
    }
}
