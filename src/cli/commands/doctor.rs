// EasyNet CLI — Aggregated Health Check
// =====================================
//
// File: src/cli/doctor.rs
// Description: `easynet doctor` — single-shot health check covering every
//              layer the CLI touches:
//
//                1. Local device pairing      (credentials present?)
//                2. Local EasyNet daemon      (process up? endpoint reachable?)
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

use crate::daemon::boot::join_connection_state;
use crate::daemon::execution::mission::drivers::{claude_code, codex};
use crate::daemon::persistence::config;
use crate::support::platform::net;
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
    let connection = join_connection_state::latest_snapshot();

    checks.push(check_connection_state(&connection));
    checks.push(check_pairing());
    checks.push(check_user_signing_key());
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
        println!(
            "{}",
            serde_json::to_string_pretty(&serde_json::json!({
                "connection": connection,
                "checks": payload,
            }))?
        );
    } else {
        eprintln!();
        eprintln!("  {}", style("EasyNet doctor").cyan().bold());
        eprintln!();
        eprintln!(
            "  {} {:<22} {}",
            style("●").cyan(),
            style("connection state").bold(),
            style(connection.to_string()).dim()
        );
        if let Some(failure) = &connection.failure {
            eprintln!("      {}", style(&failure.message).dim());
        }
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

fn check_connection_state(snapshot: &join_connection_state::JoinConnectionSnapshot) -> Check {
    let transition = snapshot
        .interrupted_transition
        .as_deref()
        .or(snapshot.transition_id.as_deref())
        .unwrap_or("-");
    let detail = match snapshot.failure.as_ref() {
        Some(failure) => format!(
            "{} [{}] at {transition}: {}",
            snapshot.state, snapshot.state_code, failure.code
        ),
        None => format!(
            "{} [{}] at {transition}",
            snapshot.state, snapshot.state_code
        ),
    };
    let status = if snapshot.state_code.starts_with('F') && snapshot.state_code != "F560" {
        CheckStatus::Fail
    } else if snapshot.state_code == "F560" || snapshot.state_code == "C440" {
        CheckStatus::Warn
    } else {
        CheckStatus::Ok
    };
    Check {
        name: "connection state".to_string(),
        status,
        detail,
        hint: Some(
            "Run 'easynet runtime status --json' or 'easynet docker status --json' \
             for the full state snapshot.",
        ),
    }
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

// Detect whether the paired user has any signing key registered in the
// daemon's trust anchor. Without one, user-as-caller invokes (the
// browser's mutating reads, or CLI-driven user invocation) fail closed
// with AXON_CALLER_SIGNATURE_INVALID and the failure is otherwise
// invisible until a call is attempted. Query the daemon's
// `identity.list_user_pubkeys` (the same trust source admission verifies
// against) rather than the keyring or the on-disk TOML, so the check
// can't drift from what the gate actually sees.
fn check_user_signing_key() -> Check {
    let name = "user signing key".to_string();
    let creds = match config::load_credentials() {
        Ok(c) => c,
        Err(_) => {
            return Check {
                name,
                status: CheckStatus::Warn,
                detail: "not paired; no user identity".to_string(),
                hint: Some("Run 'easynet device join <token>' first."),
            }
        }
    };
    let user_ura = match creds.user_ura() {
        Ok(u) => u,
        Err(_) => {
            return Check {
                name,
                status: CheckStatus::Fail,
                detail: "credentials missing user_id".to_string(),
                hint: Some("Re-pair with 'easynet device join <token>'."),
            }
        }
    };

    match crate::support::platform::local_invoke::invoke_local_ability(
        "identity.list_user_pubkeys",
        serde_json::json!({ "agent_ura": user_ura }),
    ) {
        Ok(v) => {
            let n = v
                .get("keys")
                .and_then(|k| k.as_array())
                .map_or(0, |a| a.len());
            if n > 0 {
                Check {
                    name,
                    status: CheckStatus::Ok,
                    detail: format!("{n} key(s) registered for {user_ura}"),
                    hint: None,
                }
            } else {
                Check {
                    name,
                    status: CheckStatus::Warn,
                    detail: format!("no signing key registered for {user_ura}"),
                    hint: Some(
                        "Run 'easynet auth signing-key register' to enable user-as-caller invokes.",
                    ),
                }
            }
        }
        Err(_) => Check {
            name,
            status: CheckStatus::Warn,
            detail: "daemon unreachable; cannot verify".to_string(),
            hint: Some("Start the daemon with 'easynet runtime start'."),
        },
    }
}

fn check_runtime() -> Check {
    match config::load() {
        Ok(state) => match state.runtime_kind {
            config::RuntimeKind::DaemonOnly => match crate::support::platform::local_invoke::invoke_local_ability(
                "observe.health",
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
    match crate::daemon::invocation::routing::federation_invoke::invoke_federation_discover(None) {
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
    let daemon_rows = match crate::cli::daemon_client::agent_view::list_agents() {
        Ok(rows) => rows,
        Err(err) => {
            out.push(Check {
                name: "agents".to_string(),
                status: CheckStatus::Warn,
                detail: format!("agent.list unavailable: {err}"),
                hint: Some("Start the daemon before checking registered agent rows."),
            });
            Vec::new()
        }
    };
    let to_check: Vec<(
        String,
        crate::cli::daemon_client::agent_view::AgentRuntimeKind,
    )> = if daemon_rows.is_empty() {
        vec![
            (
                "claude-code".to_string(),
                crate::cli::daemon_client::agent_view::AgentRuntimeKind::ClaudeCode,
            ),
            (
                "codex".to_string(),
                crate::cli::daemon_client::agent_view::AgentRuntimeKind::Codex,
            ),
        ]
    } else {
        daemon_rows
            .iter()
            .filter_map(|row| {
                crate::cli::daemon_client::agent_view::agent_kind(row)
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
            hint: Some("Run 'easynet mcp install' to register EasyNet with Claude Code/Codex."),
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
