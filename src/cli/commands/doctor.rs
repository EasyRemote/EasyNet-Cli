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

use crate::cli::commands::agent_cli_probe::LocalAgentCliProbe;
use crate::cli::daemon_client::agent_view::{self, AgentRuntimeKind, DaemonAgentRow};
use crate::daemon::boot::join_connection_state;
use crate::daemon::persistence::config;
use crate::support::platform::local_invoke::{
    LocalRuntimeIdentityReadIssuer, LocalRuntimeOperationalReadIssuer,
};

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
    } else if snapshot.state == "FRONTEND_CONNECTED" {
        CheckStatus::Ok
    } else {
        CheckStatus::Warn
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
    if let Ok(config::RuntimeUserBinding::Unbound { reason }) = creds.runtime_user_binding() {
        return Check {
            name,
            status: CheckStatus::Warn,
            detail: format!("{reason}; user-as-caller signing key not applicable"),
            hint: Some("Bind a product User before invoking user-scoped abilities."),
        };
    }
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

    match LocalRuntimeIdentityReadIssuer::list_user_pubkeys(
        serde_json::json!({ "user_ura": user_ura }),
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

fn unbound_runtime_user_reason() -> Option<&'static str> {
    config::load_credentials_optional()
        .ok()
        .flatten()
        .and_then(
            |credentials| match credentials.runtime_user_binding().ok()? {
                config::RuntimeUserBinding::Unbound { reason } => Some(reason),
                config::RuntimeUserBinding::Bound { .. } => None,
            },
        )
}

fn check_runtime() -> Check {
    match config::load() {
        Ok(state) => match LocalRuntimeOperationalReadIssuer::observe_health(
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
        Err(_) => Check {
            name: "local runtime".to_string(),
            status: CheckStatus::Warn,
            detail: "not running".to_string(),
            hint: Some("Run 'easynet runtime start' to spawn a local runtime."),
        },
    }
}

fn check_federation() -> Check {
    if let Some(reason) = unbound_runtime_user_reason() {
        return Check {
            name: "federation".to_string(),
            status: CheckStatus::Warn,
            detail: format!("{reason}; user-scoped federation directory not applicable"),
            hint: Some("Bind a product User before querying user-scoped federation entries."),
        };
    }
    // Joint-plan unified path: cross-device enumeration goes through
    // `federation.discover` (the same surface `easynet device list`
    // and `easynet runtime status` use). DirectoryEntries carry a
    // `status` field (`active` / `stale` / `draining`); `non-active`
    // is the doctor's "peer probe failed" equivalent in the new shape.
    federation_check_impl()
}

#[cfg(feature = "axon-pb")]
fn federation_check_impl() -> Check {
    use serde_json::Value;
    match crate::daemon::federation::directory_reader::read_federated_directory_for_current_user(
        None,
    ) {
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
            detail: format!("user-scoped federation.discover unavailable: {e}"),
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
    if let Some(reason) = unbound_runtime_user_reason() {
        return vec![Check {
            name: "agents".to_string(),
            status: CheckStatus::Warn,
            detail: format!("{reason}; user-scoped agent registry not applicable"),
            hint: Some("Bind a product User before checking product agent rows."),
        }];
    }

    let daemon_rows = match agent_view::list_agents() {
        Ok(rows) => rows,
        Err(err) => return vec![agent_list_unavailable_check(&err)],
    };

    match agent_doctor_targets_from_daemon_rows(&daemon_rows) {
        Ok(targets) => check_agent_targets(targets),
        Err(err) => vec![agent_projection_invalid_check(&err)],
    }
}

fn agent_list_unavailable_check(err: &anyhow::Error) -> Check {
    Check {
        name: "agents".to_string(),
        status: CheckStatus::Warn,
        detail: format!("agent.list unavailable: {err}"),
        hint: Some("Start the daemon before checking registered agent rows."),
    }
}

fn agent_projection_invalid_check(err: &anyhow::Error) -> Check {
    Check {
        name: "agents".to_string(),
        status: CheckStatus::Fail,
        detail: format!("agent.list returned invalid daemon projection: {err}"),
        hint: Some("Restart the daemon after repairing the agent registry projection."),
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
struct AgentDoctorTarget {
    name: String,
    runtime: AgentRuntimeKind,
    probe: Option<LocalAgentCliProbe>,
}

fn agent_doctor_targets_from_daemon_rows(
    daemon_rows: &[DaemonAgentRow],
) -> anyhow::Result<Vec<AgentDoctorTarget>> {
    if daemon_rows.is_empty() {
        return Ok(vec![
            AgentDoctorTarget {
                name: "claude-code".to_string(),
                runtime: AgentRuntimeKind::ClaudeCode,
                probe: Some(LocalAgentCliProbe::ClaudeCode),
            },
            AgentDoctorTarget {
                name: "codex".to_string(),
                runtime: AgentRuntimeKind::Codex,
                probe: Some(LocalAgentCliProbe::Codex),
            },
        ]);
    }

    daemon_rows
        .iter()
        .map(|row| {
            let runtime = agent_view::agent_kind(row)?;
            Ok(AgentDoctorTarget {
                name: row.name.clone(),
                runtime,
                probe: LocalAgentCliProbe::for_runtime(runtime),
            })
        })
        .collect()
}

fn check_agent_targets(targets: Vec<AgentDoctorTarget>) -> Vec<Check> {
    targets
        .into_iter()
        .map(|target| match target.probe {
            Some(probe) => match probe.run() {
                Ok(version) => Check {
                    name: format!("agent:{}", target.name),
                    status: CheckStatus::Ok,
                    detail: version,
                    hint: None,
                },
                Err(e) => Check {
                    name: format!("agent:{}", target.name),
                    status: CheckStatus::Fail,
                    detail: format!("{e}"),
                    hint: Some("Install or repair the underlying CLI."),
                },
            },
            None => Check {
                name: format!("agent:{}", target.name),
                status: CheckStatus::Ok,
                detail: format!("{} runtime has no local CLI probe", target.runtime),
                hint: None,
            },
        })
        .collect()
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

#[cfg(test)]
mod tests {
    use super::*;

    fn device_only_credentials() -> config::Credentials {
        config::Credentials {
            node_id: "device-a".to_string(),
            credential_token: String::new(),
            hub_endpoint: "axon://hub.example:7700".to_string(),
            realm: "localhost".to_string(),
            deploy_signature: String::new(),
            hub_api_base: None,
            username: None,
            user_id: None,
            hub_pubkey_b64: Some("hub-pubkey".to_string()),
            hub_tls_ca_pem_b64: None,
            join_receipt_hash: Some("sha256:test-join-receipt".to_string()),
        }
    }

    fn daemon_row(name: &str, runtime: &str) -> DaemonAgentRow {
        DaemonAgentRow {
            name: name.to_string(),
            ura: Some(crate::core::ura::agent_ura("test", "user", name)),
            runtime: runtime.to_string(),
            model: None,
            root_path: None,
            timeout_secs: None,
            root_exists: None,
        }
    }

    fn connection_snapshot(
        state: &str,
        state_code: &str,
    ) -> join_connection_state::JoinConnectionSnapshot {
        join_connection_state::JoinConnectionSnapshot {
            state: state.to_string(),
            state_code: state_code.to_string(),
            transition_id: Some("T09_OPEN_SELF_SESSION".to_string()),
            interrupted_transition: None,
            failure: None,
            realm: "localhost".to_string(),
            node_id: "device-a".to_string(),
            device_ura: "easynet:///r/localhost/device/device-a".to_string(),
            hub_endpoint: Some("https://127.0.0.1:50443".to_string()),
            hub_api_endpoint: Some("http://127.0.0.1:8080".to_string()),
            source: "test".to_string(),
            observed_at_unix_ms: 0,
        }
    }

    #[test]
    fn degraded_j800_connection_state_is_warn_not_ok() {
        let check = check_connection_state(&connection_snapshot("DEGRADED", "J800"));

        assert_eq!(check.status, CheckStatus::Warn);
        assert!(check.detail.contains("DEGRADED [J800]"));
    }

    #[test]
    fn frontend_connected_is_the_only_ok_product_connection_state() {
        let check = check_connection_state(&connection_snapshot("FRONTEND_CONNECTED", "J800"));

        assert_eq!(check.status, CheckStatus::Ok);
    }

    #[test]
    fn agent_list_unavailable_is_not_projected_as_default_agent_targets() {
        let error = anyhow::anyhow!("daemon offline");
        let check = agent_list_unavailable_check(&error);

        assert_eq!(check.name, "agents");
        assert_eq!(check.status, CheckStatus::Warn);
        assert!(check.detail.contains("agent.list unavailable"));
        assert!(check.detail.contains("daemon offline"));
    }

    #[test]
    fn device_only_runtime_skips_user_scoped_federation_and_agent_checks() {
        let _home = crate::cli::commands::test_support::HomeGuard::new();
        config::save_credentials(&device_only_credentials()).expect("save device-only credentials");

        let federation = check_federation();
        assert_eq!(federation.status, CheckStatus::Warn);
        assert!(
            federation
                .detail
                .contains("user-scoped federation directory not applicable"),
            "wrong federation detail: {}",
            federation.detail
        );

        let agents = check_agents();
        assert_eq!(agents.len(), 1);
        assert_eq!(agents[0].status, CheckStatus::Warn);
        assert!(
            agents[0]
                .detail
                .contains("user-scoped agent registry not applicable"),
            "wrong agent detail: {}",
            agents[0].detail
        );
    }

    #[test]
    fn empty_agent_registry_is_the_only_default_cli_probe_case() {
        let targets = agent_doctor_targets_from_daemon_rows(&[])
            .expect("empty registry should select default local CLI probes");

        assert_eq!(
            targets,
            vec![
                AgentDoctorTarget {
                    name: "claude-code".to_string(),
                    runtime: AgentRuntimeKind::ClaudeCode,
                    probe: Some(LocalAgentCliProbe::ClaudeCode),
                },
                AgentDoctorTarget {
                    name: "codex".to_string(),
                    runtime: AgentRuntimeKind::Codex,
                    probe: Some(LocalAgentCliProbe::Codex),
                },
            ]
        );
    }

    #[test]
    fn registered_agent_runtime_controls_cli_probe() {
        let rows = vec![
            daemon_row("remote-codex", "codex-app-server"),
            daemon_row("external-worker", "external"),
        ];
        let targets = agent_doctor_targets_from_daemon_rows(&rows)
            .expect("declared runtime kinds should build doctor targets");

        assert_eq!(
            targets,
            vec![
                AgentDoctorTarget {
                    name: "remote-codex".to_string(),
                    runtime: AgentRuntimeKind::CodexAppServer,
                    probe: Some(LocalAgentCliProbe::Codex),
                },
                AgentDoctorTarget {
                    name: "external-worker".to_string(),
                    runtime: AgentRuntimeKind::External,
                    probe: None,
                },
            ]
        );
    }

    #[test]
    fn invalid_daemon_runtime_projection_fails_instead_of_disappearing() {
        let rows = vec![daemon_row("broken", "mystery-runtime")];
        let error = agent_doctor_targets_from_daemon_rows(&rows)
            .expect_err("invalid daemon runtime must not be filtered out");

        assert!(error.to_string().contains("unknown agent runtime"));
    }
}
