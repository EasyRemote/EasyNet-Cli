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
use crate::support::{net, output};

#[derive(Debug, Args)]
pub struct StopArgs {}

pub fn run(_args: StopArgs) -> anyhow::Result<()> {
    let state = match config::load() {
        Ok(state) => state,
        Err(_) => {
            let hb_killed = stop_heartbeat_daemon();
            let daemon_killed = stop_easynet_daemon();
            if hb_killed {
                output::success("Heartbeat daemon stopped (no runtime state found)");
            } else if daemon_killed {
                output::success("EasyNet daemon stopped (no runtime state found)");
            } else {
                output::info("No running runtime found.");
            }
            return Ok(());
        }
    };

    if matches!(
        state.runtime_kind,
        crate::persistence::config::RuntimeKind::DaemonOnly
    ) {
        return stop_daemon_only_runtime(&state);
    }

    // 1. Always try to stop heartbeat daemon first (even without runtime.json).
    let hb_killed = stop_heartbeat_daemon();
    // Then easynet-daemon (the IPC daemon child spawned by start.rs).
    // Order: heartbeat first because it may be in the middle of an
    // outbound federation call; killing the daemon underneath it
    // would leave the call half-completed. Heartbeat-then-daemon
    // matches start order's reverse.
    let _ = stop_easynet_daemon();

    let state = state;
    output::info(&format!("Stopping runtime at {}...", state.endpoint));

    // 2. If no heartbeat daemon, deregister directly. Log accurately —
    // the previous unconditional "Node deregistered" message claimed
    // success even on transient Hub errors, hiding inconsistent state
    // from the operator.
    if !hb_killed {
        if config::load_credentials().is_ok() {
            // Per the ability-only ontology this would invoke
            // `fleet.deregister_self` on the daemon. The daemon is
            // about to be torn down here, so going through its IPC
            // surface for one last call would race the shutdown.
            // The legacy `bridge.deregister_node` was removed by
            // AXON-RFC-001 P1.5; the federation Invoke replacement
            // (which the heartbeat thread will issue *while* the
            // daemon is alive, see heartbeat.rs) is the canonical
            // path.
            output::info("Node deregister: deferred to heartbeat exit path");
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

fn stop_daemon_only_runtime(state: &config::RuntimeState) -> anyhow::Result<()> {
    output::info(&format!("Stopping daemon at {}...", state.endpoint));
    best_effort_revoke_via_daemon();
    let killed = stop_easynet_daemon();
    config::remove()?;
    if killed {
        output::success("EasyNet daemon stopped");
    } else {
        output::warn("daemon state cleared, but no easynet-daemon process was found");
    }
    Ok(())
}

fn best_effort_revoke_via_daemon() {
    #[cfg(feature = "axon-pb")]
    {
        let creds = match config::load_credentials() {
            Ok(creds) => creds,
            Err(_) => return,
        };
        let caller_uri = crate::uri::device_uri(&creds.tenant_id, &creds.node_id);
        let revoke = crate::support::federation_invoke::invoke_federation_revoke(
            &caller_uri,
            "device shutdown",
            Some(&caller_uri),
        );
        if let Err(e) = revoke {
            output::warn(&format!(
                "daemon federation.revoke failed before shutdown: {e}"
            ));
        }
    }
    #[cfg(not(feature = "axon-pb"))]
    {
        // Production builds always enable axon-pb; in minimal builds
        // there is no gRPC daemon surface to revoke through.
    }
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

/// Kill the easynet-daemon child spawned by `runtime start`.
/// Signals via the pidfile recorded at spawn time; falls back to
/// `pgrep -f easynet-daemon` when the pidfile is missing or stale.
///
/// Without this, a `runtime stop` followed by a fresh `runtime
/// start` left the previous daemon alive, the new daemon's
/// runtime-dispatch socket bind failed silently, and chat
/// dispatches got "daemon closed the connection" after exactly
/// one successful call.
pub(crate) fn stop_easynet_daemon() -> bool {
    let pid_path = config::easynet_daemon_pid_path();
    let pid: Option<u32> = std::fs::read_to_string(&pid_path)
        .ok()
        .and_then(|s| s.trim().parse().ok());
    let mut stopped_any = false;
    if let Some(pid) = pid {
        if net::is_pid_alive(pid) && net::is_easynet_process(pid) {
            if net::kill_and_wait(pid, std::time::Duration::from_secs(3)) {
                output::info(&format!("EasyNet daemon stopped (pid {pid})"));
                stopped_any = true;
            } else {
                output::warn(&format!("EasyNet daemon (pid {pid}) did not exit in time"));
            }
        }
        let _ = std::fs::remove_file(&pid_path);
    }
    // Belt-and-suspenders: sweep any stragglers via pgrep so a
    // crash-mid-stop or pidfile-write-race can't leave a ghost
    // daemon owning the runtime-dispatch socket.
    sweep_stray_easynet_daemons() || stopped_any
}

/// Pgrep-style sweep that signals every alive easynet-daemon
/// process. Best-effort.
fn sweep_stray_easynet_daemons() -> bool {
    let output_res = std::process::Command::new("pgrep")
        .args(["-f", "easynet-daemon"])
        .output();
    let pids: Vec<u32> = match output_res {
        Ok(out) if out.status.success() => String::from_utf8_lossy(&out.stdout)
            .lines()
            .filter_map(|l| l.trim().parse::<u32>().ok())
            .filter(|pid| *pid != std::process::id())
            .collect(),
        _ => return false,
    };
    let mut stopped_any = false;
    for pid in pids {
        if !net::is_pid_alive(pid) || !net::is_easynet_process(pid) {
            continue;
        }
        if net::kill_and_wait(pid, std::time::Duration::from_secs(3)) {
            output::info(&format!("EasyNet daemon swept (pid {pid})"));
            stopped_any = true;
        }
    }
    stopped_any
}
