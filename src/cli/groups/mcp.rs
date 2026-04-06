// EasyNet CLI — MCP Group
// =======================
//
// File: src/cli/groups/mcp.rs
// Description: `easynet mcp …` — Hub-side MCP server lifecycle plus the
//              integration glue for AI clients (Claude Code / Codex) and
//              skill templates.
//
// Verbs:
//   serve            Run the Hub MCP server on stdio        (-> cli::mcp_server)
//   install          Install MCP config for AI clients       (-> cli::mcp_install)
//   skill-install    Install EasyNet skill templates         (-> cli::skill_install)
//   list             Show which AI clients are wired up      (NEW)
//   status           Show whether the runtime can serve MCP  (NEW)
//
// Author: Silan Hu <silan.hu@u.nus.edu>
// Copyright (c) 2026 EasyNet. All rights reserved.

use clap::{Args, Subcommand};
use console::style;

use crate::cli::{mcp_install, mcp_server, skill_install};
use crate::shared::{config, output};

#[derive(Debug, Args)]
pub struct McpArgs {
    #[command(subcommand)]
    pub action: McpAction,
}

#[derive(Debug, Subcommand)]
pub enum McpAction {
    /// Run the Hub-level MCP server on stdio.
    Serve(mcp_server::McpServerArgs),
    /// Install MCP server config into Claude Code / Codex / Cursor.
    Install(mcp_install::McpInstallArgs),
    /// Install EasyNet skill templates into AI client skill directories.
    SkillInstall(skill_install::SkillInstallArgs),
    /// List AI clients that have an EasyNet MCP entry installed.
    List,
    /// Report whether the local runtime can answer MCP requests.
    Status,
}

pub fn run(args: McpArgs) -> anyhow::Result<()> {
    match args.action {
        McpAction::Serve(a) => mcp_server::run(a),
        McpAction::Install(a) => mcp_install::run(a),
        McpAction::SkillInstall(a) => skill_install::run(a),
        McpAction::List => run_list(),
        McpAction::Status => run_status(),
    }
}

fn run_list() -> anyhow::Result<()> {
    let home = config::home_dir();

    let candidates: Vec<(&str, std::path::PathBuf)> = vec![
        ("Claude Code (project)", std::env::current_dir()?.join(".mcp.json")),
        ("Claude Code (user)", home.join(".claude").join("mcp.json")),
        ("Claude Desktop (macOS)", home
            .join("Library/Application Support/Claude/claude_desktop_config.json")),
        ("Codex", home.join(".codex").join("config.toml")),
        ("Cursor", home.join(".cursor").join("mcp.json")),
    ];

    eprintln!();
    eprintln!("  {} clients", style("MCP").bold());
    eprintln!();
    let mut any = false;
    for (name, path) in &candidates {
        let exists = path.exists();
        let has_easynet = exists
            && std::fs::read_to_string(path)
                .map(|c| c.contains("easynet"))
                .unwrap_or(false);
        let mark = if has_easynet {
            style("●").green()
        } else if exists {
            style("○").yellow()
        } else {
            style("·").dim()
        };
        let state = if has_easynet {
            "easynet entry present"
        } else if exists {
            "config exists, no easynet entry"
        } else {
            "not installed"
        };
        if has_easynet || exists {
            any = true;
        }
        eprintln!(
            "  {} {:<26} {}",
            mark,
            style(name).bold(),
            style(state).dim(),
        );
        eprintln!("      {}", style(path.display().to_string()).dim());
    }
    if !any {
        eprintln!();
        output::info("No AI client configs detected. Run `easynet mcp install` to set one up.");
    }
    eprintln!();
    Ok(())
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
