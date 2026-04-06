// EasyNet CLI — Doctor
// ====================
//
// File: src/cli/doctor.rs
// Description: `easynet doctor` — aggregated health check across every
//              EasyNet subsystem on this host.
//
// Checks:
//   1. Local config + credentials presence
//   2. Local Axon runtime reachability
//   3. Hub bridge connectivity (`list_nodes`)
//   4. Registered AI agents and their CLI availability
//   5. MCP integration files (Claude Code / Codex / Cursor)
//
// Exit code: non-zero if any *critical* check fails (runtime + bridge).
// Agent / MCP failures are reported but do not fail the command, since they
// are optional integrations.
//
// Author: Silan Hu <silan.hu@u.nus.edu>
// Copyright (c) 2026 EasyNet. All rights reserved.

use clap::Args;
use console::style;

use crate::agent::{claude_code, codex};
use crate::shared::{self, agents, config, output};

#[derive(Debug, Args)]
pub struct DoctorArgs {
    /// Emit machine-readable JSON instead of the human report.
    #[arg(long)]
    pub json: bool,
}

pub fn run(args: DoctorArgs) -> anyhow::Result<()> {
    let mut report = serde_json::Map::new();
    let mut critical_fail = false;

    // 1. Credentials.
    let creds = config::load_credentials();
    let creds_ok = creds.is_ok();
    report.insert("credentials".into(), serde_json::json!(creds_ok));

    // 2. Runtime.
    let state = config::load();
    let runtime_ok = state.is_ok();
    report.insert("runtime".into(), serde_json::json!(runtime_ok));

    // 3. Bridge.
    let mut bridge_ok = false;
    let mut node_count: Option<usize> = None;
    if let Ok(state) = &state {
        if let Ok(br) = shared::connect_bridge_to(&state.endpoint) {
            let tenant = state.tenant_or_default();
            if let Ok(nodes) = br.list_nodes(tenant, None) {
                bridge_ok = true;
                node_count = Some(nodes.len());
            }
        }
    }
    report.insert("bridge".into(), serde_json::json!(bridge_ok));
    if !runtime_ok || !bridge_ok {
        critical_fail = true;
    }

    // 4. Agents.
    let registry = agents::load_agents().unwrap_or_default();
    let mut agent_results: Vec<serde_json::Value> = Vec::new();
    for (name, entry) in &registry.agents {
        let res = match entry.agent_type {
            agents::AgentType::ClaudeCode => claude_code::doctor(),
            agents::AgentType::Codex | agents::AgentType::CodexAppServer => codex::doctor(),
        };
        agent_results.push(serde_json::json!({
            "name": name,
            "type": entry.agent_type.to_string(),
            "ok": res.is_ok(),
            "info": res.as_ref().ok().cloned().unwrap_or_default(),
            "error": res.as_ref().err().map(ToString::to_string),
        }));
    }
    report.insert("agents".into(), serde_json::Value::Array(agent_results.clone()));

    // 5. MCP integration files (existence only).
    let home = config::home_dir();
    let mcp_paths = [
        ("claude_project", std::env::current_dir().ok().map(|p| p.join(".mcp.json"))),
        ("claude_user", Some(home.join(".claude/mcp.json"))),
        ("claude_desktop_macos", Some(home.join("Library/Application Support/Claude/claude_desktop_config.json"))),
        ("codex", Some(home.join(".codex/config.toml"))),
        ("cursor", Some(home.join(".cursor/mcp.json"))),
    ];
    let mcp_results: Vec<serde_json::Value> = mcp_paths
        .iter()
        .map(|(k, p)| {
            let exists = p.as_ref().is_some_and(|pp| pp.exists());
            let has_easynet = p
                .as_ref()
                .filter(|pp| pp.exists())
                .and_then(|pp| std::fs::read_to_string(pp).ok())
                .map(|c| c.contains("easynet"))
                .unwrap_or(false);
            serde_json::json!({
                "client": k,
                "exists": exists,
                "easynet_entry": has_easynet,
                "path": p.as_ref().map(|p| p.display().to_string()),
            })
        })
        .collect();
    report.insert("mcp".into(), serde_json::Value::Array(mcp_results));

    if args.json {
        println!("{}", serde_json::to_string_pretty(&report)?);
        return if critical_fail {
            std::process::exit(1)
        } else {
            Ok(())
        };
    }

    // Human report.
    eprintln!();
    eprintln!("  {} {}", style("EasyNet").bold(), style("doctor").dim());
    eprintln!();
    print_check(
        "credentials",
        creds_ok,
        if creds_ok { "device paired" } else { "device not paired" },
    );
    print_check("runtime", runtime_ok, "local axon runtime running");
    let bridge_msg = match node_count {
        Some(n) => format!("hub reachable, {n} nodes"),
        None => "hub bridge unreachable".into(),
    };
    print_check("bridge", bridge_ok, &bridge_msg);

    eprintln!();
    eprintln!("  {}", style("agents").dim());
    if registry.agents.is_empty() {
        eprintln!("    {}", style("(none registered)").dim());
    } else {
        for a in &agent_results {
            let name = a.get("name").and_then(|v| v.as_str()).unwrap_or("?");
            let ok = a.get("ok").and_then(|v| v.as_bool()).unwrap_or(false);
            let info = a.get("info").and_then(|v| v.as_str()).unwrap_or("");
            let err = a.get("error").and_then(|v| v.as_str()).unwrap_or("");
            let mark = if ok { style("●").green() } else { style("○").red() };
            let msg = if ok { info.to_string() } else { format!("unavailable: {err}") };
            eprintln!("    {} {:<14} {}", mark, style(name).bold(), style(msg).dim());
        }
    }

    eprintln!();
    eprintln!("  {}", style("mcp clients").dim());
    if let Some(serde_json::Value::Array(arr)) = report.get("mcp") {
        for c in arr {
            let client = c.get("client").and_then(|v| v.as_str()).unwrap_or("?");
            let exists = c.get("exists").and_then(|v| v.as_bool()).unwrap_or(false);
            let has = c.get("easynet_entry").and_then(|v| v.as_bool()).unwrap_or(false);
            let mark = if has {
                style("●").green()
            } else if exists {
                style("○").yellow()
            } else {
                style("·").dim()
            };
            let state = if has {
                "easynet entry"
            } else if exists {
                "config exists"
            } else {
                "not installed"
            };
            eprintln!("    {} {:<22} {}", mark, style(client).bold(), style(state).dim());
        }
    }
    eprintln!();
    if critical_fail {
        output::warn("doctor: one or more critical checks failed");
        std::process::exit(1);
    } else {
        output::success("doctor: all critical checks passed");
    }
    Ok(())
}

fn print_check(label: &str, ok: bool, msg: &str) {
    let mark = if ok { style("●").green() } else { style("○").red() };
    eprintln!("  {} {:<14} {}", mark, style(label).bold(), style(msg).dim());
}
