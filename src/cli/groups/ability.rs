// EasyNet CLI — Ability Group
// ===========================
//
// File: src/cli/groups/ability.rs
// Description: `easynet ability …` — full lifecycle for federation abilities
//              (the MCP tools materialised on remote devices).
//
// Verbs:
//   list                       List abilities across the federation         (-> cli::abilities)
//   show <node> <name>         Show one ability's metadata + schema         (NEW)
//   deploy <path> --to <node>  Publish + install + activate                 (-> cli::deploy)
//   update <path> --to <node>  Re-deploy a new version                      (NEW: alias of deploy)
//   uninstall <node> <id>      Remove a previously deployed ability         (NEW)
//   invoke <node> <name>       Call an ability                              (-> cli::invoke)
//   exec <node> -- <cmd>       One-shot remote shell                        (-> cli::exec)
//   logs <node> <name>         Tail the ability's runtime logs              (NEW, best-effort)
//
// Author: Silan Hu <silan.hu@u.nus.edu>
// Copyright (c) 2026 EasyNet. All rights reserved.

use anyhow::Context;
use clap::{Args, Subcommand};
use console::style;
use serde_json::json;

use crate::cli::{abilities, deploy, exec, invoke};
use crate::shared::{self, output};

#[derive(Debug, Args)]
pub struct AbilityArgs {
    #[command(subcommand)]
    pub action: AbilityAction,
}

#[derive(Debug, Subcommand)]
pub enum AbilityAction {
    /// List abilities across federated devices.
    List(abilities::AbilitiesArgs),
    /// Show one ability's full metadata and JSON schema.
    Show(ShowArgs),
    /// Deploy an ability package to a device.
    Deploy(deploy::DeployArgs),
    /// Re-deploy an ability (publish + install + activate a new version).
    Update(deploy::DeployArgs),
    /// Uninstall a previously deployed ability.
    Uninstall(UninstallArgs),
    /// Invoke an ability on a device.
    Invoke(invoke::InvokeArgs),
    /// Run a one-shot shell command on a device.
    Exec(exec::ExecArgs),
    /// Show recent invocation logs for an ability (best-effort).
    Logs(LogsArgs),
}

#[derive(Debug, Args)]
pub struct ShowArgs {
    /// Target device node id.
    pub node_id: String,
    /// Ability tool name.
    pub name: String,
}

#[derive(Debug, Args)]
pub struct UninstallArgs {
    /// Target device node id.
    pub node_id: String,
    /// Install id (from `ability list` or the deploy receipt).
    pub install_id: String,
    /// Skip the interactive confirmation.
    #[arg(long, short = 'y')]
    pub yes: bool,
}

#[derive(Debug, Args)]
pub struct LogsArgs {
    /// Target device node id.
    pub node_id: String,
    /// Ability tool name.
    pub name: String,
    /// Maximum number of recent log lines to fetch.
    #[arg(long, default_value_t = 100)]
    pub tail: usize,
}

pub fn run(args: AbilityArgs) -> anyhow::Result<()> {
    match args.action {
        AbilityAction::List(a) => abilities::run(a),
        AbilityAction::Show(a) => run_show(a),
        AbilityAction::Deploy(a) | AbilityAction::Update(a) => deploy::run(a),
        AbilityAction::Uninstall(a) => run_uninstall(a),
        AbilityAction::Invoke(a) => invoke::run(a),
        AbilityAction::Exec(a) => exec::run(a),
        AbilityAction::Logs(a) => run_logs(a),
    }
}

fn run_show(args: ShowArgs) -> anyhow::Result<()> {
    let (br, rt) = shared::connect_bridge()?;
    let tenant = rt.tenant_or_default();
    let tools = br
        .list_mcp_tools(tenant, "", &args.node_id)
        .with_context(|| format!("list_mcp_tools {}", args.node_id))?;
    let tool = tools
        .iter()
        .find(|t| {
            t.get("tool_name")
                .or_else(|| t.get("ability_name"))
                .and_then(|v| v.as_str())
                == Some(args.name.as_str())
        })
        .ok_or_else(|| {
            anyhow::anyhow!("ability '{}' not found on '{}'", args.name, args.node_id)
        })?;
    println!("{}", serde_json::to_string_pretty(tool)?);
    Ok(())
}

fn run_uninstall(args: UninstallArgs) -> anyhow::Result<()> {
    if !args.yes {
        let prompt = format!(
            "Uninstall ability '{}' from device '{}'?",
            args.install_id, args.node_id
        );
        if !output::confirm(&prompt)? {
            output::info("aborted");
            return Ok(());
        }
    }

    let (br, rt) = shared::connect_bridge()?;
    let tenant = rt.tenant_or_default();
    let result = br
        .uninstall_capability_with_reason(
            tenant,
            &args.node_id,
            &args.install_id,
            "removed via easynet ability uninstall",
        )
        .with_context(|| format!("uninstall {} on {}", args.install_id, args.node_id))?;
    output::success(&format!(
        "uninstalled {} on {}",
        args.install_id, args.node_id
    ));
    if !result.is_null() {
        println!("{}", serde_json::to_string_pretty(&result)?);
    }
    Ok(())
}

fn run_logs(args: LogsArgs) -> anyhow::Result<()> {
    // Logs aren't a first-class SDK concept yet — fall through to the
    // device's session_bridge to tail any local log file the ability author
    // chose to write. We try a couple of conventional locations and surface
    // the first non-empty result.
    let (br, rt) = shared::connect_bridge()?;
    let tenant = rt.tenant_or_default();

    let candidates = [
        format!("~/.easynet/logs/{}.log", args.name),
        format!("/var/log/easynet/{}.log", args.name),
    ];

    for path in &candidates {
        let cmd = format!("tail -n {} {} 2>/dev/null || true", args.tail, shell_escape(path));
        let payload = br.call_mcp_tool_with_timeout(
            tenant,
            "session_bridge",
            &args.node_id,
            &json!({"action": "exec", "command": cmd}),
            Some(15_000),
        );
        if let Ok(v) = payload {
            let p = v.get("result_json").unwrap_or(&v);
            let stdout = p.get("stdout").and_then(|s| s.as_str()).unwrap_or("");
            if !stdout.trim().is_empty() {
                eprintln!(
                    "  {} {}",
                    style("logs").dim(),
                    style(path).cyan(),
                );
                print!("{stdout}");
                return Ok(());
            }
        }
    }

    anyhow::bail!(
        "no logs found for ability '{}' on '{}'. Tried: {}",
        args.name,
        args.node_id,
        candidates.join(", ")
    );
}

fn shell_escape(s: &str) -> String {
    if s.chars()
        .all(|c| c.is_ascii_alphanumeric() || "/_.~-".contains(c))
    {
        s.to_string()
    } else {
        format!("'{}'", s.replace('\'', "'\"'\"'"))
    }
}
