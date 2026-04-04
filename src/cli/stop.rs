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

use crate::shared::{self, config, net, output};

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

    // 2. If no heartbeat daemon, deregister directly.
    if !hb_killed {
        if let Ok(creds) = config::load_credentials() {
            if let Ok(bridge) = shared::connect_bridge_to(&state.endpoint) {
                let _ = bridge.deregister_node(
                    &creds.tenant_id,
                    &creds.node_id,
                    "device shutdown",
                );
                output::info("Node deregistered");
            }
        }
    }

    // 3. Kill axon-runtime.
    let pid = state.pid.or_else(|| net::discover_pid_from_endpoint(&state.endpoint));
    if let Some(pid) = pid {
        stop_pid(pid);
    } else {
        output::info("warning: could not determine runtime pid; clearing state file only");
    }

    // 4. Cleanup state files.
    config::remove()?;
    output::success("Axon runtime stopped");
    Ok(())
}

/// Kill the heartbeat daemon process. Returns true if a daemon was found and successfully stopped.
fn stop_heartbeat_daemon() -> bool {
    let pid_path = config::home_dir().join(".easynet").join("heartbeat.pid");
    let pid: u32 = match std::fs::read_to_string(&pid_path)
        .ok()
        .and_then(|s| s.trim().parse().ok())
    {
        Some(p) => p,
        None => return false,
    };

    // Check if the process is actually alive before trying to stop it.
    if !net::is_pid_alive(pid) {
        output::info(&format!("Heartbeat daemon (pid {pid}) already exited, cleaning up pid file"));
        let _ = std::fs::remove_file(&pid_path);
        return false;
    }

    #[cfg(unix)]
    if let Ok(raw_pid) = i32::try_from(pid) {
        // Send SIGTERM — the daemon's ctrlc handler will deregister + exit.
        unsafe { libc::kill(raw_pid, libc::SIGTERM) };
        // Wait briefly for it to finish deregister.
        let deadline = std::time::Instant::now() + std::time::Duration::from_secs(3);
        while std::time::Instant::now() < deadline {
            if !net::is_pid_alive(pid) {
                break;
            }
            std::thread::sleep(std::time::Duration::from_millis(100));
        }
    }
    #[cfg(windows)]
    {
        let _ = std::process::Command::new("taskkill")
            .args(["/PID", &pid.to_string(), "/F"])
            .output();
        std::thread::sleep(std::time::Duration::from_millis(500));
    }

    let stopped = !net::is_pid_alive(pid);
    if stopped {
        output::info(&format!("Heartbeat daemon stopped (pid {pid})"));
    } else {
        output::info(&format!("Heartbeat daemon (pid {pid}) did not exit in time"));
    }
    let _ = std::fs::remove_file(&pid_path);
    stopped
}

fn stop_pid(pid: u32) {
    #[cfg(unix)]
    if let Ok(raw_pid) = i32::try_from(pid) {
        unsafe { libc::kill(raw_pid, libc::SIGTERM) };
        let deadline = std::time::Instant::now() + std::time::Duration::from_secs(5);
        while std::time::Instant::now() < deadline {
            let alive = unsafe { libc::kill(raw_pid, 0) == 0 };
            if !alive {
                return;
            }
            std::thread::sleep(std::time::Duration::from_millis(100));
        }
    }
    #[cfg(windows)]
    {
        let _ = std::process::Command::new("taskkill")
            .args(["/PID", &pid.to_string(), "/F"])
            .output();
    }
}
