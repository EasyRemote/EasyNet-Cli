// EasyNet Daemon — process entry point
// =====================================
//
// File: src/bin/easynet-daemon.rs
// Description: Long-running daemon entry. v10.5 R1 PR-DAEMON Commit 3
//              wires the local Control-plane IPC server alongside the
//              existing heartbeat loop ("scheme X" — one process owns
//              pair, heartbeat, IPC, and (later) ability dispatch).
//
// Current shape
// -------------
// - Always: spin up a tokio multi-thread runtime and run the
//   Control-plane accept loop on it (UDS bind, control.json write,
//   per-connection task spawn). This is the surface FFI clients dial.
// - Optional: if `_EASYNET_HB_ENDPOINT` is set in the environment,
//   start the heartbeat loop on a dedicated OS thread. The heartbeat
//   loop is sync today (uses ureq + ctrlc); embedding it on the tokio
//   runtime would block the accept loop, hence the dedicated thread.
//
// Why both?
// ---------
// Plan v10.5 R1 §"方案 X" pins single-daemon ownership of pair +
// heartbeat + IPC + ability publisher. v1 ships pair + heartbeat from
// `facade::cli::run_daemon` and IPC from `services::control::server`.
// PR-INVOCATION-EXEC-UNITY collapses the two so heartbeat lives on the
// same Kernel handle the IPC server already holds; until then we run
// them as cooperating-but-separate subsystems.
//
// What is NOT here yet
// --------------------
// - Schedule tick (PR-SCHED).
// - System ability dispatch — proxy still returns the v1 skeleton
//   error envelope (PR-INVOCATION-EXEC-UNITY).
// - Graceful shutdown that removes `~/.easynet/control.json` on
//   SIGTERM. Today the file is left behind for the next start to
//   overwrite; the OS frees the UDS file on process exit.
//
// Author: Silan Hu <silan.hu@u.nus.edu>
// Copyright (c) 2026 EasyNet. All rights reserved.

use std::collections::HashMap;
use std::sync::{Arc, Mutex};
use std::time::Duration;

use chrono::Utc;
use easynet_cli::facade::cli::run_daemon;
use easynet_cli::runtime::ability_dispatch::AbilityDispatcher;
use easynet_cli::runtime::domain::{
    AgentId, NodeId, ScheduleId, Session, SessionId, TenantId,
};
use easynet_cli::runtime::execution::schedule::ScheduleService;
use easynet_cli::runtime::execution::session::SessionService;
use easynet_cli::runtime::gateway::NoopGateway;
use easynet_cli::runtime::gateway_api::GatewayApi;
use easynet_cli::runtime::invocation_target::{LocalNodeResolver, TargetResolver};
use easynet_cli::runtime::kernel::Kernel;
use easynet_cli::runtime::kernel_api::KernelApi;
use easynet_cli::runtime::system;
use easynet_cli::services::control::ability_proxy::AbilityProxy;
use easynet_cli::services::control::server;

/// Heartbeat is opt-in: only spawn the legacy loop if the parent
/// process configured an endpoint. This lets `cargo run --bin
/// easynet-daemon` boot in IPC-only mode for FFI smoke tests without
/// requiring a Hub.
const ENV_HB_ENDPOINT: &str = "_EASYNET_HB_ENDPOINT";

#[tokio::main(flavor = "multi_thread")]
async fn main() -> anyhow::Result<()> {
    // v1: a Kernel wrapping a NoopGateway is sufficient for the proxy
    // to construct Receipts; PR-INVOCATION-EXEC-UNITY swaps in the
    // real Gateway impl that talks to Axon.
    let kernel = Arc::new(Kernel::new(Arc::new(NoopGateway::new())));

    // Bind sub-services that have a disk-backed store to the
    // current tenant so persistence actually works across daemon
    // restarts. Without this call, ScheduleService and LoopService
    // operate on an in-memory cache only — schedules and loops
    // vanish on every reboot.
    //
    // v1 single-tenant: hardcode `TenantId::default_v1()`. v2 will
    // route this from credentials.json via IPC handshake.
    let tenant = TenantId::default_v1();
    if let Err(e) = kernel.schedule_service().bind(&tenant) {
        eprintln!("[daemon] schedule store bind failed: {e:#}");
    }
    if let Err(e) = kernel.loop_service().bind(&tenant) {
        eprintln!("[daemon] loop store bind failed: {e:#}");
    }

    // Build the system.* ability registry off the SAME sub-service
    // handles the Kernel holds. This is the U1 unity property at
    // the boot path: every ability lookup and every KernelApi call
    // observe one set of sub-service state. A regression that built
    // the registry off fresh sub-services (the pre-PR shape) would
    // give the IPC plane a parallel state not reachable from the
    // Kernel — silently breaking session.list / discuss.subscribe.
    // Snapshot the sub-service handles we'll need for the tick
    // runner BEFORE moving the kernel into the proxy. session +
    // schedule handles are Arc clones so the tick runner sees the
    // same state as the registry / proxy / KernelApi calls.
    let sessions_for_tick = kernel.session_service();
    let schedule_for_tick = kernel.schedule_service();

    let registry = system::build_registry_with_services(
        kernel.session_service(),
        kernel.permission_service(),
        kernel.discuss_service(),
        kernel.schedule_service(),
        kernel.loop_service(),
    );

    // Stage-2 dispatcher (executor). Wired with the unified registry
    // and the same NoopGateway the Kernel holds — a real Gateway impl
    // pointing at Axon lands in a focused follow-up.
    let gateway: Arc<dyn GatewayApi> = Arc::new(NoopGateway::new());
    let dispatcher = AbilityDispatcher::new(registry, gateway);

    // Stage-1 resolver. Local node id from EASYNET_NODE_ID env (set
    // by the supervisor from credentials.json) or "self" as a
    // harness default; controls loopback-vs-remote routing.
    let local_node = std::env::var("EASYNET_NODE_ID")
        .ok()
        .filter(|s| !s.is_empty())
        .map(NodeId::new)
        .unwrap_or_else(|| NodeId::new("self"));
    let resolver: Arc<dyn TargetResolver> = Arc::new(LocalNodeResolver::new(local_node));

    let kernel_api: Arc<dyn KernelApi> = kernel;
    let proxy = AbilityProxy::new_with_dispatcher(
        Arc::clone(&kernel_api),
        dispatcher,
        resolver,
    );

    // Optional sidecar: heartbeat. Run on a dedicated OS thread
    // because run_daemon() is blocking (ureq + ctrlc handler). Errors
    // are logged but do not tear down the IPC server; if heartbeat
    // dies the device is in a degraded state but Client UIs can still
    // attach via FFI.
    if std::env::var_os(ENV_HB_ENDPOINT).is_some() {
        std::thread::Builder::new()
            .name("easynet-heartbeat".into())
            .spawn(|| {
                if let Err(e) = run_daemon() {
                    eprintln!("[heartbeat] daemon exited with error: {e:#}");
                }
            })?;
    }

    // Schedule tick runner. Fires due schedules every TICK_PERIOD.
    // Each fire creates a session-shaped record in SessionService
    // so a Client subscribed to system.session.attach for the
    // synthetic session id sees the fire event live.
    let session_handle = sessions_for_tick;
    let schedule_handle = schedule_for_tick;
    spawn_schedule_tick(session_handle, schedule_handle);

    // Foreground: Control-plane IPC server. Returns when the listener
    // is dropped (i.e. never, in v1 — we exit on SIGTERM via the OS).
    server::run(proxy).await
}

/// Spawn the schedule tick runner. Every `TICK_PERIOD` it asks the
/// ScheduleService for due fires and turns each into a session-
/// shaped record so attached Clients see a live `scheduled_fire`
/// frame.
///
/// v1 idempotency: an in-memory `last_fire_at` map keyed by
/// `schedule_id` keeps a fire from re-emitting on the next tick if
/// the cron expression's resolution is finer than the tick period.
/// Daemon restart loses this state — schedules due since the last
/// fire will refire once on resume per their misfire policy. v2
/// will durably persist last-fire-at alongside the schedule entry.
fn spawn_schedule_tick(
    sessions: Arc<SessionService>,
    schedule: Arc<ScheduleService>,
) {
    const TICK_PERIOD: Duration = Duration::from_secs(15);
    tokio::spawn(async move {
        let last_fire: Arc<Mutex<HashMap<ScheduleId, i64>>> =
            Arc::new(Mutex::new(HashMap::new()));
        let mut interval = tokio::time::interval(TICK_PERIOD);
        // First tick fires immediately; skip it so a daemon
        // restart does not double-fire schedules whose cron is at
        // a coarse interval.
        interval.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Skip);
        let _ = interval.tick().await;
        loop {
            interval.tick().await;
            let now = Utc::now();
            let lookup_last = {
                let lf = Arc::clone(&last_fire);
                move |id: &ScheduleId| -> Option<i64> {
                    lf.lock().ok().and_then(|g| g.get(id).copied())
                }
            };
            let due = schedule.due(now, lookup_last);
            if due.is_empty() {
                continue;
            }
            for fire in due {
                let now_ms = now.timestamp_millis();
                if let Ok(mut g) = last_fire.lock() {
                    g.insert(fire.schedule_id.clone(), now_ms);
                }
                // Synthesize a session for the fire so Clients see
                // it on system.session.attach. The session id is a
                // composite "sched-<id>-<fire_ms>" so attach can
                // target one specific fire's timeline.
                let sid = SessionId::new(format!(
                    "sched-{}-{}",
                    fire.schedule_id.as_str(),
                    fire.fire_at.timestamp_millis()
                ));
                let synthetic = Session {
                    id: sid.clone(),
                    agent: AgentId::new(
                        schedule
                            .list()
                            .iter()
                            .find(|s| s.id == fire.schedule_id)
                            .map(|s| s.target_agent.as_str().to_string())
                            .unwrap_or_else(|| "?".into()),
                    ),
                    node: NodeId::new("self"),
                    tenant: TenantId::default_v1(),
                    started_unix_ms: now_ms,
                    ended_unix_ms: None,
                };
                if let Err(e) = sessions.admit(synthetic) {
                    // Already admitted with the same id (= same
                    // minute, same schedule) — idempotent skip.
                    eprintln!(
                        "[schedule-tick] skip duplicate fire for {}: {e}",
                        fire.schedule_id
                    );
                    continue;
                }
                let _ = sessions.emit_event(
                    &sid,
                    serde_json::json!({
                        "kind": "scheduled_fire",
                        "schedule_id": fire.schedule_id.as_str(),
                        "fire_at_unix_ms": fire.fire_at.timestamp_millis(),
                        "catch_up": fire.catch_up,
                    }),
                );
                let _ = sessions.terminate(&sid, now_ms);
                eprintln!(
                    "[schedule-tick] fired {} at {} (catch_up={})",
                    fire.schedule_id,
                    fire.fire_at,
                    fire.catch_up,
                );
            }
        }
    });
}

