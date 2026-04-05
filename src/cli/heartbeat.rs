// EasyNet CLI — Heartbeat
// ========================
//
// File: src/cli/heartbeat.rs
// Description: Heartbeat loop, daemon lifecycle, and outcome handling.
//
// Extracted from start.rs to isolate the heartbeat state machine from runtime
// bootstrap logic. Used by both foreground mode (in-process) and the background
// heartbeat daemon subprocess.
//
// Author: Silan Hu <silan.hu@u.nus.edu>
// Copyright (c) 2026 EasyNet. All rights reserved.

use anyhow::Context;

use crate::shared::{self, config, output, shutdown::ShutdownSignal};

pub const DEFAULT_HEARTBEAT_MS: u64 = 30_000;
const MAX_HEARTBEAT_FAILURES: u32 = 10;

// ── Heartbeat env var keys (daemon ↔ parent contract) ───────────────────────
// Only runtime-specific values are passed via env vars. Tenant and node_id
// are read from credentials.json by the daemon, avoiding exposure in `ps`.

const ENV_ENDPOINT: &str = "_EASYNET_HB_ENDPOINT";
const ENV_INTERVAL_MS: &str = "_EASYNET_HB_INTERVAL_MS";

/// Outcome of the heartbeat loop — signals why it exited.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum HeartbeatOutcome {
    /// User requested shutdown (Ctrl-C / SIGTERM).
    Shutdown,
    /// Too many consecutive heartbeat failures.
    FailuresExhausted,
    /// Hub sent a permanent rejection (evicted).
    HubRejected,
    /// This node was administratively removed by the Hub.
    NodeRejected,
}

impl HeartbeatOutcome {
    /// Deregister reason string sent to Hub.
    pub fn reason(self) -> &'static str {
        match self {
            Self::Shutdown => "device shutdown",
            Self::FailuresExhausted => "heartbeat lost",
            Self::HubRejected => "hub rejected",
            Self::NodeRejected => "node rejected",
        }
    }
}

/// Blocking heartbeat loop — runs until shutdown is signaled, failures exhaust,
/// or the Hub rejects this member/node.
pub fn heartbeat_loop(
    bridge: &easynet_axon::dendrite_bridge::DendriteBridge,
    tenant: &str,
    node_id: &str,
    interval_ms: u64,
    shutdown: &ShutdownSignal,
) -> HeartbeatOutcome {
    let interval = std::time::Duration::from_millis(interval_ms);
    let mut failures = 0u32;
    while !shutdown.is_triggered() {
        if !shutdown.sleep_unless_triggered(interval) {
            break;
        }
        match bridge.node_heartbeat(tenant, node_id) {
            Ok(resp) => {
                if failures > 0 {
                    output::info(&format!("heartbeat recovered after {failures} failures"));
                    failures = 0;
                }
                if let Some(outcome) = check_rejection(&resp, node_id) {
                    return outcome;
                }
            }
            Err(e) => {
                failures += 1;
                output::warn(&format!(
                    "heartbeat failed ({failures}/{MAX_HEARTBEAT_FAILURES}): {e}"
                ));
                if failures >= MAX_HEARTBEAT_FAILURES {
                    output::warn("heartbeat lost — initiating graceful shutdown");
                    return HeartbeatOutcome::FailuresExhausted;
                }
            }
        }
    }
    HeartbeatOutcome::Shutdown
}

/// Check heartbeat response for permanent rejection or node removal.
fn check_rejection(resp: &serde_json::Value, node_id: &str) -> Option<HeartbeatOutcome> {
    // Hub has evicted this member entirely.
    if resp
        .get("permanent")
        .and_then(serde_json::Value::as_bool)
        .unwrap_or(false)
    {
        let status = resp
            .get("status")
            .and_then(serde_json::Value::as_str)
            .unwrap_or("unknown");
        output::warn(&format!(
            "heartbeat permanently rejected by hub (status: {status}), disconnecting"
        ));
        return Some(HeartbeatOutcome::HubRejected);
    }
    // This specific node was administratively removed.
    let self_rejected = resp
        .get("rejected_nodes")
        .and_then(|v| v.as_array())
        .is_some_and(|arr| {
            arr.iter()
                .filter_map(|v| v.get("node_id").and_then(|n| n.as_str()))
                .any(|id| id == node_id)
        });
    if self_rejected {
        output::warn(&format!(
            "this node ({node_id}) was rejected by hub, disconnecting"
        ));
        return Some(HeartbeatOutcome::NodeRejected);
    }
    None
}

/// Fork a background daemon that handles heartbeat + deregister on SIGTERM.
/// The daemon reads tenant/node_id from credentials.json at startup.
pub fn spawn_daemon(
    endpoint: &str,
    heartbeat_ms: u64,
) -> anyhow::Result<()> {
    let exe = std::env::current_exe().context("resolve exe path")?;

    let log_dir = config::state_dir().join("logs");
    std::fs::create_dir_all(&log_dir)?;
    let log_path = log_dir.join("heartbeat.log");
    rotate_log_if_needed(&log_path);
    let log_fh = std::fs::OpenOptions::new()
        .create(true)
        .append(true)
        .open(&log_path)?;
    let log_err = log_fh.try_clone()?;

    let child = std::process::Command::new(exe)
        .arg("_heartbeat-daemon")
        .env(ENV_ENDPOINT, endpoint)
        .env(ENV_INTERVAL_MS, heartbeat_ms.to_string())
        .stdout(log_fh)
        .stderr(log_err)
        .spawn()
        .context("spawn heartbeat daemon")?;

    let hb_pid_path = config::heartbeat_pid_path();
    std::fs::write(&hb_pid_path, child.id().to_string())?;

    output::detail(
        "heartbeat daemon",
        &format!("pid {} (log: {})", child.id(), log_path.display()),
    );
    Ok(())
}

/// Rotate the heartbeat log if it exceeds 2 MiB. Keeps one `.old` backup.
fn rotate_log_if_needed(path: &std::path::Path) {
    const MAX_LOG_BYTES: u64 = 2 * 1024 * 1024;
    let size = std::fs::metadata(path).map(|m| m.len()).unwrap_or(0);
    if size > MAX_LOG_BYTES {
        let old = path.with_extension("log.old");
        // Best-effort: if rename fails, truncate instead.
        if std::fs::rename(path, &old).is_err() {
            let _ = std::fs::write(path, b"");
        }
    }
}

/// Entry point for the heartbeat daemon subprocess (hidden subcommand).
pub fn run_daemon() -> anyhow::Result<()> {
    let endpoint =
        std::env::var(ENV_ENDPOINT).map_err(|_| anyhow::anyhow!("missing {ENV_ENDPOINT}"))?;
    let interval_ms: u64 = std::env::var(ENV_INTERVAL_MS)
        .unwrap_or_else(|_| DEFAULT_HEARTBEAT_MS.to_string())
        .parse()?;

    // Read tenant/node_id from credentials file (not env vars) to avoid
    // exposing identity in `ps auxe` output.
    let creds = config::load_credentials()
        .context("heartbeat daemon: cannot load credentials")?;
    let tenant = &creds.tenant_id;
    let node_id = &creds.node_id;

    let bridge = shared::connect_bridge_to(&endpoint)?;

    let shutdown = ShutdownSignal::new();
    let s = shutdown.clone();
    ctrlc::set_handler(move || {
        s.trigger();
    })?;

    let outcome = heartbeat_loop(&bridge, tenant, node_id, interval_ms, &shutdown);

    let reason = outcome.reason();
    let _ = bridge.deregister_node(tenant, node_id, reason);
    if outcome == HeartbeatOutcome::NodeRejected {
        config::delete_credentials().ok();
        output::warn("device removed by admin — credentials cleaned up");
    }
    output::info(&format!(
        "heartbeat daemon: deregistered {node_id} ({reason}), exiting"
    ));
    Ok(())
}
