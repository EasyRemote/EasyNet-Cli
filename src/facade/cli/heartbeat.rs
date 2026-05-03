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

use easynet_axon::dendrite_bridge::DendriteBridge;
use easynet_axon::error::Result as AxonResult;
use easynet_axon::reconnect::{ReconnectConfig, ReconnectHook, ReconnectingBridge};

use crate::persistence::config;
use crate::support::{output, shutdown::ShutdownSignal};

// The runtime-side federation sweeper starts suspecting stale members
// at ~20 s. The CLI used to wait 30 s before sending the next
// heartbeat, which guaranteed every long-lived device would self-revoke
// between the initial advertise and the first tick. Keep the CLI-side
// cadence aligned with the runtime's 5 s transport heartbeat.
pub const DEFAULT_HEARTBEAT_MS: u64 = 5_000;
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

/// Transport-agnostic view of the one operation `heartbeat_loop` needs: send
/// a heartbeat and get back either a Hub response (to inspect for rejection)
/// or a transport error (to count against the failure budget).
///
/// Two impls live below:
///
///   * `DirectBridge<'_>` — a thin wrapper around an already-open
///     `&DendriteBridge`. Used by the foreground mode in `cli::start`, where
///     the same bridge that registered the node is reused for heartbeats.
///   * `ReconnectingHeartbeat<'_>` — wraps a `&ReconnectingBridge`, so every
///     heartbeat rides the SDK's reconnect + re-register machinery. Used by
///     the background daemon, which has to keep working across long-lived
///     TCP dropouts and Hub restarts.
///
/// The trait method takes `&mut self` (not `&self`) because the reconnecting
/// impl may mutate its internal attempt counter on retry. The loop below
/// expects `&mut dyn HeartbeatTransport`, so callers keep ownership and can
/// introspect the transport after the loop returns (e.g. to issue a final
/// `deregister_node` on the same session).
pub trait HeartbeatTransport {
    /// Perform one heartbeat RPC and return the Hub response or an error.
    /// The loop passes the response to `check_rejection` — all fields the
    /// state machine inspects must survive this round-trip unchanged.
    fn beat(&mut self, tenant: &str, node_id: &str) -> AxonResult<serde_json::Value>;
}

/// Direct-bridge transport — no reconnect. Used in the foreground path where
/// the parent process already holds a bridge and the operator can observe /
/// restart on failure.
pub struct DirectBridge<'a> {
    bridge: &'a DendriteBridge,
}

impl<'a> DirectBridge<'a> {
    pub fn new(bridge: &'a DendriteBridge) -> Self {
        Self { bridge }
    }
}

// Heartbeat is device-scoped — one `federation.heartbeat` per tick from
// the device URA. The hub fans liveness out to every hosted Agent that
// names this device as its host (`AgentsCatalog::refresh_heartbeat_for_device`),
// so consent/policy/mcp/llm/... do NOT need to ping the hub independently.
// Process death takes them all out at once; multiplying ticks per process
// would just amplify hub traffic without adding signal.
impl<'a> HeartbeatTransport for DirectBridge<'a> {
    fn beat(&mut self, tenant: &str, node_id: &str) -> AxonResult<serde_json::Value> {
        // Heartbeat tick: `federation.heartbeat` is the canonical
        // membership-liveness ability (Hub-Profile, RFC-001 §A14).
        // Replaces the legacy `bridge.NodeHeartbeat` gRPC RPC
        // removed by AXON-RFC-001 P1.5. Pre-fix the transport
        // re-used `runtime.bootstrap_self_identity` as a tickle —
        // it accidentally refreshed the same `last_heartbeat_unix_ms`
        // because both paths populate that field, but it was a
        // semantic abuse; the dedicated heartbeat ability is the
        // proper carrier and `federation.advertise_*` /
        // `federation.resolve` use the same caller URI we set
        // here.
        let device_uri = crate::uri::device_uri(&tenant, &node_id);
        let invoker = crate::runtime::advertise::BridgeAbilityInvoker::with_caller_uri(
            self.bridge,
            device_uri,
        );
        crate::runtime::advertise::heartbeat(&invoker, tenant, tenant)
            .map(|_| serde_json::json!({"ok": true}))
            .map_err(|msg| easynet_axon::error::AxonError::Bridge(msg))
    }
}

/// Reconnecting transport — every heartbeat rides the SDK's
/// `ReconnectingBridge::with_bridge`. On a transport error the bridge
/// reconnects with exponential backoff, invokes the re-register hook
/// (supplied at construction), and retries the beat once on the fresh
/// connection. Application-level errors (e.g. the Hub returning `permanent:
/// true`) propagate unchanged to `check_rejection` — reconnecting a node
/// the Hub just evicted is exactly the wrong move.
pub struct ReconnectingHeartbeat<'a> {
    bridge: &'a ReconnectingBridge,
}

impl<'a> ReconnectingHeartbeat<'a> {
    pub fn new(bridge: &'a ReconnectingBridge) -> Self {
        Self { bridge }
    }
}

impl<'a> HeartbeatTransport for ReconnectingHeartbeat<'a> {
    fn beat(&mut self, tenant: &str, node_id: &str) -> AxonResult<serde_json::Value> {
        // Mirrors DirectBridge::beat — see that impl for the
        // `federation.heartbeat` rationale. The reconnecting
        // wrapper retries the call once on a transport-level
        // failure (its standard contract), so an abilty-level
        // hub rejection still propagates here while a transient
        // dropped connection self-heals.
        let device_uri = crate::uri::device_uri(&tenant, &node_id);
        self.bridge.with_bridge(|br| {
            let invoker = crate::runtime::advertise::BridgeAbilityInvoker::with_caller_uri(
                br,
                device_uri.clone(),
            );
            crate::runtime::advertise::heartbeat(&invoker, tenant, tenant)
                .map(|_| serde_json::json!({"ok": true}))
                .map_err(|msg| easynet_axon::error::AxonError::Bridge(msg))
        })
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
///
/// # Why the transport is abstracted
///
/// The loop's state machine is identical whether heartbeats ride a direct
/// bridge (foreground) or a reconnecting bridge (daemon). Parameterising
/// over `HeartbeatTransport` keeps the two call sites on one implementation
/// of the jitter / backoff / rejection logic — and makes the state machine
/// unit-testable without a real bridge (see `tests` below).
pub fn heartbeat_loop<T: HeartbeatTransport>(
    transport: &mut T,
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
            next_heartbeat_state(transport.beat(tenant, node_id), node_id, failures);
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

/// Build the `on_reconnect` hook used by the daemon's `ReconnectingBridge`.
///
/// The Hub sweeps stale nodes after a heartbeat-timeout window. If the
/// daemon's bridge drops and reconnects later than that window, the Hub no
/// longer has our registration — silently reconnecting without re-
/// registering leaves a phantom heartbeat that the Hub rejects. The hook
/// re-issues `federation.advertise_agent` (Hub-Profile ability) on every
/// successful reconnect.
///
/// Replaces the legacy `register_node_with_options` gRPC RPC removed by
/// AXON-RFC-001 P1.5. The hub-side bookkeeping (directory entry,
/// signing-authority binding) is now done entirely through the
/// federation.* ability surface; no behavioural change at the Hub.
///
/// `_hostname` is preserved on the signature for source-compat with
/// callers that build it for label generation; the federation
/// advertise path doesn't currently consume it (per-node display name
/// is a future RFC-006 hub field).
fn build_reregister_hook(tenant: String, node_id: String, _hostname: String) -> ReconnectHook {
    use std::rc::Rc;
    Rc::new(move |bridge: &DendriteBridge| -> AxonResult<()> {
        let device_uri = crate::uri::device_uri(&tenant, &node_id);
        let invoker = crate::runtime::advertise::BridgeAbilityInvoker::with_caller_uri(
            bridge,
            device_uri.clone(),
        );
        let args = crate::runtime::federation_client::AdvertiseAgentArgs {
            agent_uri: device_uri.clone(),
            public_key_hex: String::new(),
            signing_authority:
                crate::runtime::federation_client::AdvertisedSigningAuthority::SelfSigned,
            host_node_id: Some(node_id.clone()),
        };
        let outcome = crate::runtime::advertise::advertise_agent(&invoker, &tenant, &tenant, &args);
        match outcome.result {
            Ok(_) => Ok(()),
            Err(msg) => Err(easynet_axon::error::AxonError::Bridge(format!(
                "federation.advertise_agent on reconnect failed: {msg}"
            ))),
        }
    })
}

/// Entry point for the heartbeat daemon subprocess (hidden subcommand).
///
/// Bridge lifecycle: the daemon drives heartbeats through a
/// [`ReconnectingBridge`]. On transport failure the SDK reconnects with
/// exponential backoff and invokes the re-register hook (see
/// [`build_reregister_hook`]) so a reconnected bridge immediately re-
/// establishes the Hub-side registration the sweeper may have cleared.
/// Application-level Hub rejections (`permanent: true`, `rejected_nodes`)
/// are surfaced to `check_rejection` unchanged, so the daemon still
/// cleans up credentials on eviction — reconnect does not mask admin
/// rejection.
pub fn run_daemon() -> anyhow::Result<()> {
    let endpoint =
        std::env::var(ENV_ENDPOINT).map_err(|_| anyhow::anyhow!("missing {ENV_ENDPOINT}"))?;
    let interval_ms: u64 = std::env::var(ENV_INTERVAL_MS)
        .unwrap_or_else(|_| DEFAULT_HEARTBEAT_MS.to_string())
        .parse()?;

    // Read tenant/node_id from credentials file (not env vars) to avoid
    // exposing identity in `ps auxe` output.
    let creds = config::load_credentials().context("heartbeat daemon: cannot load credentials")?;
    let tenant = creds.tenant_id.clone();
    let node_id = creds.node_id.clone();
    let hostname = gethostname::gethostname().to_string_lossy().into_owned();

    // Build the reconnecting bridge. `connect` (not `new_deferred`) is the
    // right constructor here: the parent process already registered the
    // node before spawning us, so if we cannot reach the runtime at all
    // the operator should see the error immediately rather than after a
    // first silent heartbeat tick.
    let reconnect_config = ReconnectConfig {
        endpoint: endpoint.clone(),
        connect_timeout_ms: crate::support::timeouts::BRIDGE_CONNECT_TIMEOUT_MS,
        ..Default::default()
    };
    let hook = build_reregister_hook(tenant.clone(), node_id.clone(), hostname);
    let reconnecting = ReconnectingBridge::connect(reconnect_config, Some(hook))
        .with_context(|| format!("heartbeat daemon: initial connect to {endpoint}"))?;

    // Bootstrap self-identity into axon-runtime BEFORE the first
    // heartbeat. Without this the runtime rejects every signed Invoke
    // (heartbeat, advertise, resolve) with
    // `AXON_EASYNET_SUBJECT_KEY_UNREGISTERED` until `easynet runtime
    // start` happens to be the launcher (it bootstraps too). Bare
    // `easynet-daemon` boot — service / launchd / unit-file path —
    // never goes through `runtime start`, so the heartbeat sidecar is
    // the right place: it has the runtime endpoint, runs once per
    // daemon lifetime, and any long-lived federation surface depends
    // on it succeeding. Best-effort: a failure logs and continues —
    // bootstrap is idempotent (first-writer-wins) so subsequent ticks
    // pay zero cost on success.
    //
    // realm == tenant for v1 single-realm; the URI v2 cut-over picks
    // a separate `realm` field. Until then both fields name the same
    // logical scope.
    let realm = tenant.clone();
    let bootstrap_outcome = reconnecting.with_bridge(|br| {
        let device_uri = crate::uri::device_uri(&tenant, &node_id);
        let invoker =
            crate::runtime::advertise::BridgeAbilityInvoker::with_caller_uri(br, device_uri);
        let outcome = crate::runtime::publish::bootstrap_self_identity_via_runtime(
            &invoker, &tenant, &realm, &node_id,
        );
        outcome
            .result
            .map_err(easynet_axon::error::AxonError::Bridge)
    });
    match bootstrap_outcome {
        Ok(()) => output::info(&format!(
            "heartbeat daemon: bootstrapped trusted-key material for {node_id}"
        )),
        Err(e) => output::warn(&format!(
            "heartbeat daemon: runtime.bootstrap_self_identity failed: {e}; signed Invokes \
             will fail until the runtime accepts this node's key"
        )),
    }

    let shutdown = ShutdownSignal::new();
    let s = shutdown.clone();
    ctrlc::set_handler(move || {
        s.trigger();
    })?;

    let mut transport = ReconnectingHeartbeat::new(&reconnecting);
    let outcome = heartbeat_loop(&mut transport, &tenant, &node_id, interval_ms, &shutdown);

    // Federation shutdown: invoke `federation.revoke` on this
    // daemon's directory entry. Replaces the deprecated
    // `bridge.deregister_node` gRPC RPC. Rides the same
    // reconnecting bridge so a transient drop right before
    // shutdown still reaches the hub via one auto-reconnect.
    let reason = outcome.reason();
    let device_uri = crate::uri::device_uri(&tenant, &node_id);
    let revoked = reconnecting.with_bridge(|br| {
        let invoker = crate::runtime::advertise::BridgeAbilityInvoker::with_caller_uri(
            br,
            device_uri.clone(),
        );
        crate::runtime::advertise::revoke_agent(&invoker, &tenant, &tenant, &device_uri, reason)
            .map_err(|e| easynet_axon::error::AxonError::Bridge(e))
    });
    if outcome == HeartbeatOutcome::NodeRejected {
        config::delete_credentials().ok();
        output::warn("device removed by admin — credentials cleaned up");
    }
    match revoked {
        Ok(_) => output::info(&format!(
            "heartbeat daemon: federation.revoke {node_id} ({reason}), exiting"
        )),
        Err(e) => output::warn(&format!(
            "heartbeat daemon: federation.revoke {node_id} ({reason}) failed: {e}; exiting anyway"
        )),
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use easynet_axon::error::AxonError;
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

    // ── HeartbeatTransport + heartbeat_loop end-to-end tests ────────────────
    //
    // These exercise the whole loop against a programmable `FakeTransport`.
    // The state-machine tests above pin individual transitions; these pin
    // the *loop's* composition of them: wake-up via shutdown signal, state
    // carry-across-ticks, terminal-outcome propagation, and rejection
    // checks running on every response (not just the last).
    //
    // A sub-second interval keeps each test well under a wall-clock
    // second. We pick 20 ms so the jitter branch (interval_ms >= 1_000) is
    // not taken — the jitter logic is already covered by its own tests,
    // and we want deterministic tick timing here.

    use std::cell::RefCell;

    /// Programmable transport. Each `beat()` pops the next scripted step
    /// and records that a beat happened. The script lets a test drive an
    /// arbitrary success/error/response sequence without any network.
    ///
    /// # Shutdown coupling
    ///
    /// The fake trips the caller-supplied `ShutdownSignal` once the script
    /// is consumed. That keeps the loop-exit cause fully deterministic and
    /// removes the wall-clock race that a timer-based shutdown would
    /// introduce — tests pass at any sleep interval the loop picks.
    struct FakeTransport {
        script: RefCell<Vec<Step>>,
        calls: RefCell<usize>,
        shutdown: ShutdownSignal,
    }

    enum Step {
        Ok(serde_json::Value),
        Err(AxonError),
    }

    impl FakeTransport {
        fn new(script: Vec<Step>, shutdown: ShutdownSignal) -> Self {
            Self {
                script: RefCell::new(script),
                calls: RefCell::new(0),
                shutdown,
            }
        }

        fn calls(&self) -> usize {
            *self.calls.borrow()
        }
    }

    impl HeartbeatTransport for FakeTransport {
        fn beat(&mut self, _tenant: &str, _node_id: &str) -> AxonResult<serde_json::Value> {
            *self.calls.borrow_mut() += 1;
            let step = {
                let mut script = self.script.borrow_mut();
                if script.is_empty() {
                    // Running out of script mid-test is a test-authoring
                    // bug — panic loudly so the failure surfaces here, not
                    // as a silent hang on a later tick.
                    panic!("FakeTransport script exhausted before loop exit");
                }
                // `remove(0)` (not `pop`) so the script reads front-to-back
                // in call order, matching a reader's intuition.
                script.remove(0)
            };
            // If this was the last scripted step, trip shutdown so the next
            // `sleep_unless_triggered` short-circuits and the loop exits
            // cleanly with `HeartbeatOutcome::Shutdown` (unless this very
            // step drove it to a terminal outcome first).
            if self.script.borrow().is_empty() {
                self.shutdown.trigger();
            }
            match step {
                Step::Ok(v) => Ok(v),
                Step::Err(e) => Err(e),
            }
        }
    }

    #[test]
    fn loop_returns_shutdown_when_signal_fires_before_first_beat() {
        // Pre-triggered shutdown: the loop must observe it on the initial
        // `!shutdown.is_triggered()` gate and exit without calling beat()
        // even once. This pins the "shutdown wins the race" property.
        let shutdown = ShutdownSignal::new();
        shutdown.trigger();
        let mut t = FakeTransport::new(vec![], shutdown.clone());
        let outcome = heartbeat_loop(&mut t, "tenant", "node", 20, &shutdown);
        assert_eq!(outcome, HeartbeatOutcome::Shutdown);
        assert_eq!(
            t.calls(),
            0,
            "no beat must happen after pre-triggered shutdown"
        );
    }

    #[test]
    fn loop_recovers_from_transient_failures() {
        // 3 failures then a success — the loop must not exit on the
        // failures (count < MAX) and must clear the counter on success.
        // `FakeTransport` trips shutdown after the last scripted beat, so
        // the loop returns `Shutdown` deterministically.
        let shutdown = ShutdownSignal::new();
        let mut t = FakeTransport::new(
            vec![
                Step::Err(AxonError::Bridge("transient 1".into())),
                Step::Err(AxonError::Bridge("transient 2".into())),
                Step::Err(AxonError::Bridge("transient 3".into())),
                Step::Ok(json!({"ok": true})),
            ],
            shutdown.clone(),
        );
        let outcome = heartbeat_loop(&mut t, "tenant", "node", 20, &shutdown);
        assert_eq!(outcome, HeartbeatOutcome::Shutdown);
        assert_eq!(
            t.calls(),
            4,
            "all 4 scripted beats must run before shutdown stops the loop"
        );
    }

    #[test]
    fn loop_exits_on_hub_permanent_rejection_even_after_success() {
        // The Hub may reject a member mid-session (e.g. token revoked).
        // The loop must observe the `permanent: true` flag on the
        // *rejecting* response, not the most recent healthy one. This is
        // the property that makes the NodeAgent pivot necessary — the
        // SDK's NodeAgent does not parse this field today.
        let shutdown = ShutdownSignal::new();
        let mut t = FakeTransport::new(
            vec![
                Step::Ok(json!({"ok": true})),
                Step::Ok(json!({"ok": true})),
                Step::Ok(json!({"permanent": true, "status": "evicted"})),
            ],
            shutdown.clone(),
        );
        let outcome = heartbeat_loop(&mut t, "tenant", "node", 20, &shutdown);
        assert_eq!(outcome, HeartbeatOutcome::HubRejected);
        assert_eq!(t.calls(), 3, "loop must exit exactly on the rejecting beat");
    }

    #[test]
    fn loop_exits_on_node_specific_rejection() {
        // Admin removes one node from a fleet. Only the affected node's
        // daemon must exit; other nodes see their own id missing from the
        // rejected list and continue.
        let shutdown = ShutdownSignal::new();
        let mut t = FakeTransport::new(
            vec![Step::Ok(json!({"rejected_nodes": [{"node_id": "node-1"}]}))],
            shutdown.clone(),
        );
        let outcome = heartbeat_loop(&mut t, "tenant", "node-1", 20, &shutdown);
        assert_eq!(outcome, HeartbeatOutcome::NodeRejected);
    }

    #[test]
    fn loop_exits_on_consecutive_failure_exhaustion() {
        // MAX_HEARTBEAT_FAILURES back-to-back errors must exhaust the
        // budget exactly — not one tick earlier, not one tick later.
        let shutdown = ShutdownSignal::new();
        let script: Vec<Step> = (0..MAX_HEARTBEAT_FAILURES)
            .map(|_| Step::Err(AxonError::Bridge("timeout".into())))
            .collect();
        let mut t = FakeTransport::new(script, shutdown.clone());
        let outcome = heartbeat_loop(&mut t, "tenant", "node", 20, &shutdown);
        assert_eq!(outcome, HeartbeatOutcome::FailuresExhausted);
        assert_eq!(t.calls() as u32, MAX_HEARTBEAT_FAILURES);
    }

    #[test]
    fn loop_does_not_exhaust_when_failures_are_interrupted_by_success() {
        // Pattern: (fail × (MAX-1), succeed) repeated. The counter must
        // reset on each success, so the loop never exhausts — a broken
        // "counter never resets" implementation would trip inside any
        // round past the first. We run 3 rounds so a hypothetical
        // "resets once then leaks" bug would also surface.
        let shutdown = ShutdownSignal::new();
        let mut script = Vec::new();
        let interleave_rounds = 3;
        for _ in 0..interleave_rounds {
            for _ in 0..(MAX_HEARTBEAT_FAILURES - 1) {
                script.push(Step::Err(AxonError::Bridge("flap".into())));
            }
            script.push(Step::Ok(json!({"ok": true})));
        }
        let total_beats = script.len();
        let mut t = FakeTransport::new(script, shutdown.clone());
        let outcome = heartbeat_loop(&mut t, "tenant", "node", 20, &shutdown);
        assert_eq!(
            outcome,
            HeartbeatOutcome::Shutdown,
            "recoveries must reset the failure counter — we should never exhaust"
        );
        assert_eq!(
            t.calls(),
            total_beats,
            "every scripted beat must run exactly once"
        );
    }

    /// Rejection must win over an in-flight failure counter. A `permanent`
    /// response arriving on the tick that would otherwise be failure N
    /// (N < MAX) must still terminate with `HubRejected`, not with a
    /// silently-incremented counter. This pins the ordering inside
    /// `next_heartbeat_state`: successes enter `check_rejection` before
    /// the failure path runs.
    #[test]
    fn rejection_preempts_a_partially_exhausted_counter() {
        let shutdown = ShutdownSignal::new();
        let mut script: Vec<Step> = (0..(MAX_HEARTBEAT_FAILURES - 1))
            .map(|_| Step::Err(AxonError::Bridge("flap".into())))
            .collect();
        script.push(Step::Ok(json!({"permanent": true, "status": "banned"})));
        let mut t = FakeTransport::new(script, shutdown.clone());
        let outcome = heartbeat_loop(&mut t, "tenant", "node", 20, &shutdown);
        assert_eq!(outcome, HeartbeatOutcome::HubRejected);
        assert_eq!(
            t.calls() as u32,
            MAX_HEARTBEAT_FAILURES,
            "loop must consume the rejection-carrying beat to observe it"
        );
    }

    /// Sanity: a benign response with neither `permanent` nor a
    /// `rejected_nodes` entry naming this node must be treated as a
    /// healthy heartbeat. Absence of these fields is the common case;
    /// a defensive reader that treated *any* non-`ok: true` response as
    /// suspicious would make every Hub schema evolution a Cli outage.
    #[test]
    fn unknown_response_fields_do_not_trigger_rejection() {
        let shutdown = ShutdownSignal::new();
        let mut t = FakeTransport::new(
            vec![
                Step::Ok(json!({"hub_build": "1.2.3", "next_heartbeat_ms": 30000})),
                Step::Ok(json!({"ok": true})),
            ],
            shutdown.clone(),
        );
        let outcome = heartbeat_loop(&mut t, "tenant", "node", 20, &shutdown);
        assert_eq!(outcome, HeartbeatOutcome::Shutdown);
    }
}
