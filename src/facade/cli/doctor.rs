// EasyNet CLI — Aggregated Health Check
// =====================================
//
// File: src/cli/doctor.rs
// Description: `easynet doctor` — single-shot health check covering every
//              layer the CLI touches:
//
//                1. Local device pairing      (credentials present?)
//                2. Local Axon runtime        (process up? endpoint reachable?)
//                3. Federation reachability   (can the runtime list nodes?)
//                4. Registered AI agent CLIs  (claude / codex actually installed?)
//                5. MCP server entries        (any AI client wired up?)
//
// The output is one section per check, with `ok` / `warn` / `fail`
// indicators and an actionable hint when something is broken. Exit code
// is 0 if every section is `ok` or `warn`, 1 if any section is `fail`.
//
// Author: Silan Hu <silan.hu@u.nus.edu>
// Copyright (c) 2026 EasyNet. All rights reserved.

use clap::Args;
use console::style;

use crate::runtime::drivers::{claude_code, codex};
use crate::persistence::config;
use crate::registry::agents;
#[derive(Debug, Args)]
pub struct DoctorArgs {
    /// Emit JSON instead of the human-readable report.
    #[arg(long)]
    pub json: bool,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum CheckStatus {
    Ok,
    Warn,
    Fail,
}

struct Check {
    name: String,
    status: CheckStatus,
    detail: String,
    hint: Option<&'static str>,
}

pub fn run(args: DoctorArgs) -> anyhow::Result<()> {
    let mut checks: Vec<Check> = Vec::new();

    checks.push(check_pairing());
    checks.push(check_runtime());
    checks.push(check_federation());
    checks.extend(check_agents());
    checks.push(check_mcp_clients());

    if args.json {
        let payload: Vec<_> = checks
            .iter()
            .map(|c| {
                serde_json::json!({
                    "name": c.name,
                    "status": match c.status {
                        CheckStatus::Ok => "ok",
                        CheckStatus::Warn => "warn",
                        CheckStatus::Fail => "fail",
                    },
                    "detail": c.detail,
                    "hint": c.hint,
                })
            })
            .collect();
        println!("{}", serde_json::to_string_pretty(&payload)?);
    } else {
        eprintln!();
        eprintln!("  {}", style("EasyNet doctor").cyan().bold());
        eprintln!();
        for c in &checks {
            let mark = match c.status {
                CheckStatus::Ok => style("●").green(),
                CheckStatus::Warn => style("●").yellow(),
                CheckStatus::Fail => style("●").red(),
            };
            eprintln!(
                "  {} {:<22} {}",
                mark,
                style(&c.name).bold(),
                style(&c.detail).dim()
            );
            if let Some(h) = c.hint {
                if c.status != CheckStatus::Ok {
                    eprintln!("      {}", style(h).dim());
                }
            }
        }
        eprintln!();
    }

    let any_fail = checks.iter().any(|c| c.status == CheckStatus::Fail);
    if any_fail {
        anyhow::bail!("doctor: one or more checks failed");
    }
    Ok(())
}

fn check_pairing() -> Check {
    match config::load_credentials() {
        Ok(creds) => Check {
            name: "device pairing".to_string(),
            status: CheckStatus::Ok,
            detail: format!("paired as {}", creds.node_id),
            hint: None,
        },
        Err(_) => Check {
            name: "device pairing".to_string(),
            status: CheckStatus::Warn,
            detail: "not paired".to_string(),
            hint: Some("Run `easynet device join <token>` to pair this host."),
        },
    }
}

fn check_runtime() -> Check {
    match config::load() {
        Ok(state) => Check {
            name: "local runtime".to_string(),
            status: CheckStatus::Ok,
            detail: format!("up at {}", state.endpoint),
            hint: None,
        },
        Err(_) => Check {
            name: "local runtime".to_string(),
            status: CheckStatus::Warn,
            detail: "not running".to_string(),
            hint: Some("Run `easynet runtime start` to spawn a local runtime."),
        },
    }
}

fn check_federation() -> Check {
    // Per the ability-only ontology this check goes through
    // `fleet.list_nodes` — the same surface every other CLI
    // surface uses — instead of the dead `bridge.list_nodes`
    // path that pre-rewrite returned a hard error here.
    match crate::support::local_invoke::invoke_local_ability(
        "fleet.list_nodes",
        serde_json::json!({}),
    ) {
        Ok(envelope) => {
            let count = envelope
                .get("nodes")
                .and_then(serde_json::Value::as_array)
                .map(|a| a.len())
                .unwrap_or(0);
            let view = envelope
                .get("federation_view")
                .and_then(serde_json::Value::as_str)
                .unwrap_or("");
            // `local_only` is the expected steady-state today: the
            // federation Invoke replacement for AXON-RFC-001 P1.5's
            // `list_nodes` ships in a follow-up. Surface as Warn
            // (not Fail) — the daemon is healthy, the federation
            // surface is just incomplete by design.
            if view == "local_only" {
                Check {
                    name: "federation".to_string(),
                    status: CheckStatus::Warn,
                    detail: format!(
                        "{count} node(s); local-only view (federation Invoke replacement pending)"
                    ),
                    hint: Some(
                        "This is expected post-AXON-RFC-001 P1.5. Local fleet operations remain available.",
                    ),
                }
            } else {
                Check {
                    name: "federation".to_string(),
                    status: CheckStatus::Ok,
                    detail: format!("{count} node(s) reachable"),
                    hint: None,
                }
            }
        }
        Err(e) => Check {
            name: "federation".to_string(),
            status: CheckStatus::Warn,
            detail: format!("fleet.list_nodes unavailable: {e}"),
            hint: Some("Check that the daemon is running (`easynet runtime status`)."),
        },
    }
}

fn check_agents() -> Vec<Check> {
    let registry = agents::load_agents().unwrap_or_default();
    let mut out = Vec::new();
    let to_check: Vec<(String, agents::AgentType)> = if registry.agents.is_empty() {
        vec![
            ("claude-code".to_string(), agents::AgentType::ClaudeCode),
            ("codex".to_string(), agents::AgentType::Codex),
        ]
    } else {
        registry
            .agents
            .iter()
            .map(|(n, e)| (n.clone(), e.agent_type))
            .collect()
    };

    for (name, ty) in to_check {
        let probe = match ty {
            agents::AgentType::ClaudeCode => claude_code::doctor(),
            agents::AgentType::Codex | agents::AgentType::CodexAppServer => codex::doctor(),
        };
        out.push(match probe {
            Ok(version) => Check {
                name: format!("agent:{name}"),
                status: CheckStatus::Ok,
                detail: version,
                hint: None,
            },
            Err(e) => Check {
                name: format!("agent:{name}"),
                status: CheckStatus::Fail,
                detail: format!("{e}"),
                hint: Some("Install or repair the underlying CLI."),
            },
        });
    }
    out
}

fn check_mcp_clients() -> Check {
    let home = config::home_dir();
    let candidates: Vec<std::path::PathBuf> = vec![
        home.join(".claude").join("mcp.json"),
        home.join("Library/Application Support/Claude/claude_desktop_config.json"),
        home.join(".codex").join("config.toml"),
        home.join(".cursor").join("mcp.json"),
    ];
    let installed: Vec<&std::path::PathBuf> = candidates
        .iter()
        .filter(|p| {
            p.exists()
                && std::fs::read_to_string(p)
                    .map(|c| c.contains("easynet"))
                    .unwrap_or(false)
        })
        .collect();
    if installed.is_empty() {
        Check {
            name: "mcp clients".to_string(),
            status: CheckStatus::Warn,
            detail: "no AI client wired up".to_string(),
            hint: Some("Run `easynet mcp-install` to register EasyNet with Claude Code/Codex."),
        }
    } else {
        Check {
            name: "mcp clients".to_string(),
            status: CheckStatus::Ok,
            detail: format!("{} client(s) configured", installed.len()),
            hint: None,
        }
    }
}
