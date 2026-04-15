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

use crate::persistence::config;
use crate::shared::{self, output, shutdown::ShutdownSignal};

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
///
/// Cadence carries a small, *per-device deterministic* jitter (±10 %) so a
/// federation of devices that all start in the same release window does
/// not hammer the Hub on the same wall-clock second. Jitter is derived
/// from `node_id` so a single device's own cadence stays stable across
/// restarts — makes debugging timing issues possible.
pub fn heartbeat_loop(
    bridge: &easynet_axon::dendrite_bridge::DendriteBridge,
    tenant: &str,
    node_id: &str,
    interval_ms: u64,
    shutdown: &ShutdownSignal,
) -> HeartbeatOutcome {
    let interval = jittered_interval(interval_ms, node_id);
    let mut failures = 0u32;
    while !shutdown.is_triggered() {
        if !shutdown.sleep_unless_triggered(interval) {
            break;
        }
        let (next_failures, outcome) =
            next_heartbeat_state(bridge.node_heartbeat(tenant, node_id), node_id, failures);
        failures = next_failures;
        if let Some(outcome) = outcome {
            return outcome;
        }
    }
    HeartbeatOutcome::Shutdown
}

/// Deterministic per-device jitter: offset `interval_ms` by up to ±10 %,
/// keyed on `node_id` so the offset is stable across restarts of the
/// same device. Sub-second intervals are left unjittered — the jitter
/// window would be smaller than OS timer resolution and adds no value.
fn jittered_interval(interval_ms: u64, node_id: &str) -> std::time::Duration {
    if interval_ms < 1_000 {
        return std::time::Duration::from_millis(interval_ms);
    }
    use std::hash::{Hash, Hasher};
    let mut hasher = std::collections::hash_map::DefaultHasher::new();
    node_id.hash(&mut hasher);
    let h = hasher.finish();
    // Map h into the symmetric window [-span_ms, +span_ms] where span_ms
    // is 10 % of the interval.
    let span_ms = interval_ms / 10;
    let offset = (h % (2 * span_ms + 1)) as i64 - span_ms as i64;
    let jittered = (interval_ms as i64 + offset).max(1) as u64;
    std::time::Duration::from_millis(jittered)
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

/// Transition one heartbeat tick.
/// Returns `(next_failures, optional_terminal_outcome)`.
fn next_heartbeat_state<E: std::fmt::Display>(
    result: Result<serde_json::Value, E>,
    node_id: &str,
    failures: u32,
) -> (u32, Option<HeartbeatOutcome>) {
    match result {
        Ok(resp) => {
            if failures > 0 {
                output::info(&format!("heartbeat recovered after {failures} failures"));
            }
            if let Some(outcome) = check_rejection(&resp, node_id) {
                return (0, Some(outcome));
            }
            (0, None)
        }
        Err(e) => {
            let failures = failures.saturating_add(1);
            output::warn(&format!(
                "heartbeat failed ({failures}/{MAX_HEARTBEAT_FAILURES}): {e}"
            ));
            if failures >= MAX_HEARTBEAT_FAILURES {
                output::warn("heartbeat lost — initiating graceful shutdown");
                (failures, Some(HeartbeatOutcome::FailuresExhausted))
            } else {
                (failures, None)
            }
        }
    }
}

/// Fork a background daemon that handles heartbeat + deregister on
/// SIGTERM. The daemon reads tenant/node_id from credentials.json at
/// startup.
///
/// Detachment model
/// ----------------
/// On Unix, the spawned child calls `setsid(2)` as its first action
/// (via `CommandExt::pre_exec`). This does three load-bearing things:
///
/// - Creates a new session and process group, so the daemon no longer
///   receives signals (SIGHUP, SIGINT) sent to the parent's terminal
///   or process group. Without it, closing the launching terminal
///   would kill the daemon.
/// - Detaches from the controlling TTY, which means a subsequent
///   `open("/dev/tty")` from the daemon is harmless instead of
///   grabbing the parent's terminal.
/// - Makes the daemon the leader of its own session, so its children
///   (if any) stay together and can be signalled as a group.
///
/// We deliberately do NOT double-fork. The second fork protects
/// against accidentally acquiring a controlling TTY if the daemon
/// ever opens a tty device, which this daemon does not. Keeping the
/// single fork keeps the code readable and the PID file stable (no
/// need to communicate the grand-child's pid back).
///
/// On non-Unix targets `pre_exec` is unavailable; the child inherits
/// the parent's session, which is an accepted limitation (Windows
/// is not a supported daemon host for this binary).
pub fn spawn_daemon(endpoint: &str, heartbeat_ms: u64) -> anyhow::Result<()> {
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

    let mut cmd = std::process::Command::new(exe);
    cmd.arg("_heartbeat-daemon")
        .env(ENV_ENDPOINT, endpoint)
        .env(ENV_INTERVAL_MS, heartbeat_ms.to_string())
        .stdout(log_fh)
        .stderr(log_err);

    #[cfg(unix)]
    {
        use std::os::unix::process::CommandExt;
        // SAFETY: `setsid(2)` is async-signal-safe and documented as
        // safe to call from a post-fork, pre-exec context. We do
        // nothing else inside this closure, so we cannot accidentally
        // touch any state that would be unsound to touch between
        // fork and exec (locks, allocators, etc.).
        unsafe {
            cmd.pre_exec(|| {
                // `libc::setsid` returns -1 on error but never when
                // called on the fresh post-fork child (the only
                // failure mode is "already a group leader", which
                // cannot happen here). Still, surface the error
                // through the returned io::Result rather than
                // ignoring it — the child will fail its spawn with
                // a clear io error instead of leaking into
                // launched-but-undetached territory.
                if libc::setsid() == -1 {
                    return Err(std::io::Error::last_os_error());
                }
                Ok(())
            });
        }
    }

    let child = cmd.spawn().context("spawn heartbeat daemon")?;

    let hb_pid_path = config::heartbeat_pid_path();
    std::fs::write(&hb_pid_path, child.id().to_string())?;

    output::detail(
        "heartbeat daemon",
        &format!("pid {} (log: {})", child.id(), log_path.display()),
    );
    Ok(())
}

/// Rotate the heartbeat log if it exceeds 2 MiB. Keeps one `.old`
/// backup.
///
/// Atomicity model
/// ---------------
/// The rename is atomic per POSIX. The concern isn't "can two
/// spawners simultaneously rename the same file" (the second rename
/// will just fail harmlessly), but "can two spawners simultaneously
/// *observe* oversized log and one of them truncates after the
/// other rotated".
///
/// We defend against that by only attempting rotation here, where
/// `spawn_daemon` is typically called from `runtime start` — a
/// user-visible command run by one operator. If the rename fails
/// (a racing spawner already moved the old file), we do NOT
/// truncate: the other spawner has already rotated to a fresh
/// file, and truncating our copy would throw away the log lines
/// they wrote in between. The previous fallback `fs::write(path,
/// b"")` traded data loss for code brevity, which is the wrong
/// trade.
fn rotate_log_if_needed(path: &std::path::Path) {
    const MAX_LOG_BYTES: u64 = 2 * 1024 * 1024;
    let size = std::fs::metadata(path).map(|m| m.len()).unwrap_or(0);
    if size <= MAX_LOG_BYTES {
        return;
    }
    let old = path.with_extension("log.old");
    // Best-effort: if rename fails, leave the log alone. A racing
    // rotator has already moved the file; truncating would
    // destroy legitimate data.
    let _ = std::fs::rename(path, &old);
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
    let creds = config::load_credentials().context("heartbeat daemon: cannot load credentials")?;
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
    let deregistered = bridge.deregister_node(tenant, node_id, reason);
    if outcome == HeartbeatOutcome::NodeRejected {
        config::delete_credentials().ok();
        output::warn("device removed by admin — credentials cleaned up");
    }
    match deregistered {
        Ok(_) => output::info(&format!(
            "heartbeat daemon: deregistered {node_id} ({reason}), exiting"
        )),
        Err(e) => output::warn(&format!(
            "heartbeat daemon: deregister {node_id} ({reason}) failed: {e}; exiting anyway"
        )),
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn jitter_stays_within_ten_percent_window() {
        // The jitter window is symmetric ±10 %, so for an interval of
        // 30_000 ms every jittered value must land in [27_000, 33_000].
        // Sampling a handful of node ids is enough to catch a busted
        // modulus or an off-by-one span calculation; we don't need a
        // property-based sweep here.
        for id in ["n1", "node-alpha", "", "a much longer node identifier"] {
            let j = jittered_interval(30_000, id).as_millis() as i64;
            assert!(
                (27_000..=33_000).contains(&j),
                "jittered interval for '{id}' out of window: {j}"
            );
        }
    }

    #[test]
    fn jitter_is_deterministic_per_node_id() {
        // Stability across restarts is load-bearing for debugging timing
        // issues: a given device should always fire on the same offset.
        assert_eq!(
            jittered_interval(30_000, "node-42"),
            jittered_interval(30_000, "node-42")
        );
    }

    #[test]
    fn jitter_skipped_for_sub_second_intervals() {
        // Below 1 s the jitter window is smaller than typical OS timer
        // resolution, so adding it only obscures intent.
        let d = jittered_interval(500, "anything");
        assert_eq!(d, std::time::Duration::from_millis(500));
    }

    #[test]
    fn outcome_reason_strings_are_stable() {
        assert_eq!(HeartbeatOutcome::Shutdown.reason(), "device shutdown");
        assert_eq!(
            HeartbeatOutcome::FailuresExhausted.reason(),
            "heartbeat lost"
        );
        assert_eq!(HeartbeatOutcome::HubRejected.reason(), "hub rejected");
        assert_eq!(HeartbeatOutcome::NodeRejected.reason(), "node rejected");
    }

    #[test]
    fn success_clears_previous_failures() {
        let (failures, outcome) =
            next_heartbeat_state::<&str>(Ok(json!({"ok": true})), "node-1", 3);
        assert_eq!(failures, 0);
        assert_eq!(outcome, None);
    }

    #[test]
    fn transient_error_increments_failure_counter() {
        let (failures, outcome) = next_heartbeat_state::<&str>(Err("net down"), "node-1", 0);
        assert_eq!(failures, 1);
        assert_eq!(outcome, None);
    }

    #[test]
    fn repeated_failures_eventually_exhaust() {
        let mut failures = 0;
        let mut terminal = None;
        for _ in 0..MAX_HEARTBEAT_FAILURES {
            let (next, outcome) = next_heartbeat_state::<&str>(Err("timeout"), "node-1", failures);
            failures = next;
            terminal = outcome;
        }
        assert_eq!(failures, MAX_HEARTBEAT_FAILURES);
        assert_eq!(terminal, Some(HeartbeatOutcome::FailuresExhausted));
    }

    #[test]
    fn hub_permanent_rejection_exits_immediately() {
        let resp = json!({"permanent": true, "status": "evicted"});
        let (failures, outcome) = next_heartbeat_state::<&str>(Ok(resp), "node-1", 0);
        assert_eq!(failures, 0);
        assert_eq!(outcome, Some(HeartbeatOutcome::HubRejected));
    }

    #[test]
    fn node_rejection_list_only_affects_current_node() {
        let resp = json!({
            "rejected_nodes": [
                {"node_id": "node-2"},
                {"node_id": "node-3"}
            ]
        });
        assert_eq!(check_rejection(&resp, "node-1"), None);

        let resp = json!({
            "rejected_nodes": [
                {"node_id": "node-1"}
            ]
        });
        assert_eq!(
            check_rejection(&resp, "node-1"),
            Some(HeartbeatOutcome::NodeRejected)
        );
    }
}
