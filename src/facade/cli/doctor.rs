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

use crate::persistence::config;
use crate::runtime::drivers::{claude_code, codex};
use crate::support::net;
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
            hint: Some("Run 'easynet device join <token>' to pair this host."),
        },
    }
}

fn check_runtime() -> Check {
    match config::load() {
        Ok(state) => match state.runtime_kind {
            config::RuntimeKind::DaemonOnly => match crate::support::local_invoke::invoke_local_ability(
                "device.observe.health",
                serde_json::json!({"source": "doctor"}),
            ) {
                Ok(_) => Check {
                    name: "local runtime".to_string(),
                    status: CheckStatus::Ok,
                    detail: format!("daemon up at {}", state.endpoint),
                    hint: None,
                },
                Err(e) => Check {
                    name: "local runtime".to_string(),
                    status: CheckStatus::Fail,
                    detail: format!("metadata present, but observe.health failed: {e}"),
                    hint: Some(
                        "The runtime metadata exists, but the local daemon/control socket is not healthy.",
                    ),
                },
            },
            // Legacy raw axon-runtime state. Unified device and hub paths
            // now record DaemonOnly and flow through the branch above;
            // this arm covers only pre-unification or non-product state.
            config::RuntimeKind::AxonBridge => {
                let alive = state.pid.is_some_and(net::is_pid_alive)
                    || net::discover_pid_from_endpoint(&state.endpoint).is_some();
                if alive {
                    Check {
                        name: "local runtime".to_string(),
                        status: CheckStatus::Ok,
                        detail: format!("bridge runtime (legacy) up at {}", state.endpoint),
                        hint: None,
                    }
                } else {
                    Check {
                        name: "local runtime".to_string(),
                        status: CheckStatus::Fail,
                        detail: "runtime metadata present, but the bridge process is not alive"
                            .to_string(),
                        hint: Some(
                            "Run 'easynet runtime stop' to clear stale state, then 'easynet runtime start'.",
                        ),
                    }
                }
            }
        },
        Err(_) => Check {
            name: "local runtime".to_string(),
            status: CheckStatus::Warn,
            detail: "not running".to_string(),
            hint: Some("Run 'easynet runtime start' to spawn a local runtime."),
        },
    }
}

fn check_federation() -> Check {
    // Joint-plan unified path: cross-device enumeration goes through
    // `federation.discover` (the same surface `easynet device list`
    // and `easynet runtime status` use). DirectoryEntries carry a
    // `status` field (`active` / `stale` / `draining`); `non-active`
    // is the doctor's "peer probe failed" equivalent in the new shape.
    if config::load()
        .map(|state| state.uses_bridge())
        .unwrap_or(false)
    {
        return Check {
            name: "federation".to_string(),
            status: CheckStatus::Warn,
            detail: "bridge/hub mode has no local daemon federation probe".to_string(),
            hint: Some(
                "Start device mode ('easynet runtime start') if you want daemon-backed federation health checks.",
            ),
        };
    }
    federation_check_impl()
}

#[cfg(feature = "axon-pb")]
fn federation_check_impl() -> Check {
    use serde_json::Value;
    match crate::support::federation_invoke::invoke_federation_discover(None, None) {
        Ok(entries) => {
            let total = entries.len();
            let stale = entries
                .iter()
                .filter(|e| {
                    e.get("status")
                        .and_then(Value::as_str)
                        .map(|s| s != "active")
                        .unwrap_or(true)
                })
                .count();
            if total == 0 {
                Check {
                    name: "federation".to_string(),
                    status: CheckStatus::Warn,
                    detail: "no federated directory entries — peers may not be reachable yet, \
                             or no devices are paired across hubs"
                        .to_string(),
                    hint: Some(
                        "Check 'easynet federation peers' to confirm the trust anchor + \
                         peer daemon health.",
                    ),
                }
            } else if stale > 0 {
                Check {
                    name: "federation".to_string(),
                    status: CheckStatus::Warn,
                    detail: format!(
                        "{total} entr{} discovered, but {stale} are not active (stale / draining)",
                        if total == 1 { "y" } else { "ies" }
                    ),
                    hint: Some(
                        "Stale entries indicate the source daemon's heartbeat lapsed; \
                         restart the peer daemon or wait for the next sweep tick.",
                    ),
                }
            } else {
                Check {
                    name: "federation".to_string(),
                    status: CheckStatus::Ok,
                    detail: format!(
                        "{total} entr{} active",
                        if total == 1 { "y" } else { "ies" }
                    ),
                    hint: None,
                }
            }
        }
        Err(e) => Check {
            name: "federation".to_string(),
            status: CheckStatus::Warn,
            detail: format!("federation.discover unavailable: {e}"),
            hint: Some("Check that the daemon is running ('easynet runtime status')."),
        },
    }
}

#[cfg(not(feature = "axon-pb"))]
fn federation_check_impl() -> Check {
    Check {
        name: "federation".to_string(),
        status: CheckStatus::Warn,
        detail: "federation.discover requires the 'axon-pb' build feature".to_string(),
        hint: Some(
            "Production builds always include 'axon-pb'; this is likely a minimal-feature build.",
        ),
    }
}

fn check_agents() -> Vec<Check> {
    let mut out = Vec::new();
    let daemon_rows = match crate::facade::cli::daemon_agent_view::list_agents() {
        Ok(rows) => rows,
        Err(err) => {
            out.push(Check {
                name: "agents".to_string(),
                status: CheckStatus::Warn,
                detail: format!("device.agent.list unavailable: {err}"),
                hint: Some("Start the daemon before checking registered agent rows."),
            });
            Vec::new()
        }
    };
    let to_check: Vec<(
        String,
        crate::facade::cli::daemon_agent_view::AgentRuntimeKind,
    )> = if daemon_rows.is_empty() {
        vec![
            (
                "claude-code".to_string(),
                crate::facade::cli::daemon_agent_view::AgentRuntimeKind::ClaudeCode,
            ),
            (
                "codex".to_string(),
                crate::facade::cli::daemon_agent_view::AgentRuntimeKind::Codex,
            ),
        ]
    } else {
        daemon_rows
            .iter()
            .filter_map(|row| {
                crate::facade::cli::daemon_agent_view::agent_kind(row)
                    .ok()
                    .map(|kind| (row.name.clone(), kind))
            })
            .collect()
    };

    for (name, ty) in to_check {
        let probe = if ty.is_claude_code() {
            claude_code::doctor()
        } else {
            codex::doctor()
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
            hint: Some("Run 'easynet mcp-install' to register EasyNet with Claude Code/Codex."),
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
