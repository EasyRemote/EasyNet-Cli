// EasyNet CLI
// ===========
//
// File: src/cli/stop.rs
// Description: `easynet stop` — deregisters node, kills heartbeat daemon, and stops runtime.
//
// Shutdown Strategy:
// 1. Kill heartbeat daemon (reads heartbeat.pid) — triggers deregister via SIGTERM handler.
// 2. If heartbeat daemon doesn't exist, deregister node directly via bridge.
// 3. Kill axon-runtime process (reads PID from runtime.json or discovers via lsof).
// 4. Clears ~/.easynet/runtime.json and heartbeat.pid.
//
// Author: Silan Hu <silan.hu@u.nus.edu>
// Copyright (c) 2026 EasyNet. All rights reserved.

use clap::Args;

use crate::persistence::config;
use crate::support::{self, net, output};

#[derive(Debug, Args)]
pub struct StopArgs {}

pub fn run(_args: StopArgs) -> anyhow::Result<()> {
    // 1. Always try to stop heartbeat daemon first (even without runtime.json).
    let hb_killed = stop_heartbeat_daemon();

    let Ok(state) = config::load() else {
        if hb_killed {
            output::success("Heartbeat daemon stopped (no runtime state found)");
        } else {
            output::info("No running runtime found.");
        }
        return Ok(());
    };

    output::info(&format!("Stopping runtime at {}...", state.endpoint));

    // 2. If no heartbeat daemon, deregister directly. Log accurately —
    // the previous unconditional "Node deregistered" message claimed
    // success even on transient Hub errors, hiding inconsistent state
    // from the operator.
    if !hb_killed {
        if let Ok(creds) = config::load_credentials() {
            if let Ok(bridge) = support::connect_bridge_to(&state.endpoint) {
                match bridge.deregister_node(&creds.tenant_id, &creds.node_id, "device shutdown") {
                    Ok(_) => output::info("Node deregistered"),
                    Err(e) => output::warn(&format!(
                        "Hub deregister failed (continuing local stop): {e}"
                    )),
                }
            }
        }
    }

    // 3. Kill axon-runtime.
    let pid = state
        .pid
        .or_else(|| net::discover_pid_from_endpoint(&state.endpoint));
    if let Some(pid) = pid {
        stop_pid(pid);
    } else {
        output::warn("could not determine runtime pid; clearing state file only");
    }

    // 4. Cleanup state files.
    config::remove()?;
    output::success("Axon runtime stopped");
    Ok(())
}

/// Kill the heartbeat daemon process. Returns true if a daemon was found and successfully stopped.
///
/// Note: PID files are inherently racy — between reading the PID and signaling, the OS
/// could reuse it. The `is_pid_alive` check narrows the window but cannot eliminate it.
/// Acceptable for a CLI tool; a production daemon would use a lockfile or pidfd.
fn stop_heartbeat_daemon() -> bool {
    let pid_path = config::heartbeat_pid_path();
    let pid: u32 = match std::fs::read_to_string(&pid_path)
        .ok()
        .and_then(|s| s.trim().parse().ok())
    {
        Some(p) => p,
        None => return false,
    };

    // Check if the process is actually alive before trying to stop it.
    if !net::is_pid_alive(pid) {
        output::info(&format!(
            "Heartbeat daemon (pid {pid}) already exited, cleaning up pid file"
        ));
        let _ = std::fs::remove_file(&pid_path);
        return false;
    }

    // Verify the PID still belongs to an easynet process (mitigates PID-reuse race).
    if !net::is_easynet_process(pid) {
        output::warn(&format!(
            "pid {pid} is alive but does not appear to be an easynet process; skipping signal"
        ));
        let _ = std::fs::remove_file(&pid_path);
        return false;
    }

    let stopped = net::kill_and_wait(pid, std::time::Duration::from_secs(3));
    if stopped {
        output::info(&format!("Heartbeat daemon stopped (pid {pid})"));
    } else {
        output::warn(&format!(
            "Heartbeat daemon (pid {pid}) did not exit in time"
        ));
    }
    let _ = std::fs::remove_file(&pid_path);
    stopped
}

fn stop_pid(pid: u32) {
    net::kill_and_wait(pid, std::time::Duration::from_secs(5));
}
