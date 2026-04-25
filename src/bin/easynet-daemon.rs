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
use easynet_cli::runtime::domain::{NodeId, ScheduleId, TenantId};
use easynet_cli::runtime::execution::loop_instance::KernelLoopInvocationDriver;
use easynet_cli::runtime::execution::schedule::ScheduleService;
use easynet_cli::runtime::gateway::NoopGateway;
use easynet_cli::runtime::gateway_api::GatewayApi;
use easynet_cli::runtime::invocation::{
    fresh_nonce_hex, CausalContext, Invocation,
};
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
    // v1: a Kernel wrapping a NoopGateway is sufficient for the
    // proxy to construct Receipts. The daemon installs the
    // SubscriberBroker permission variant so a Client UI
    // connected to system.permission.subscribe sees real pending
    // requests when an agent dispatch is gated. (When no Client
    // is subscribed the broker auto-allows — a daemon running
    // headless does not freeze on permission gates.)
    let kernel = Arc::new(Kernel::new_with_subscriber_broker(Arc::new(
        NoopGateway::new(),
    )));

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

    let local_node = std::env::var("EASYNET_NODE_ID")
        .ok()
        .filter(|s| !s.is_empty())
        .map(NodeId::new)
        .unwrap_or_else(|| NodeId::new("self"));
    let kernel_api: Arc<dyn KernelApi> = Arc::clone(&kernel) as Arc<dyn KernelApi>;
    let loop_driver = Arc::new(KernelLoopInvocationDriver::new(
        Arc::clone(&kernel_api),
        kernel.session_service(),
        local_node.clone(),
    ));
    if let Err(e) = kernel.loop_service().install_driver(loop_driver) {
        eprintln!("[daemon] loop controller install failed: {e:#}");
    }
    if let Err(e) = kernel.loop_service().resume_inflight() {
        eprintln!("[daemon] loop resume failed: {e:#}");
    }

    // Build the system.* ability registry off the SAME sub-service
    // handles the Kernel holds. This is the U1 unity property at
    // the boot path: every ability lookup and every KernelApi call
    // observe one set of sub-service state. A regression that built
    // the registry off fresh sub-services (the pre-PR shape) would
    // give the IPC plane a parallel state not reachable from the
    // Kernel — silently breaking session.list / discuss.subscribe.
    // Snapshot the sub-service handles we'll need for the tick
    // runner BEFORE moving the kernel into the proxy. The schedule
    // handle reads which schedules are due; the kernel handle is
    // the C* unity entry — the tick runner constructs an Invocation
    // and routes through Kernel::invoke (which admits a Session,
    // dispatches the agent, terminates).
    let schedule_for_tick = kernel.schedule_service();
    let kernel_for_tick: Arc<Kernel> = Arc::clone(&kernel);

    // Build the full ability registry. `build_registry_for_daemon`
    // loads the agent registry from disk so `chat_ability::register`
    // can mount one `<agent>.chat` handler per locally-registered
    // agent. A load failure degrades to "no agents" rather than
    // crashing — chat is one ability among many, and a registry that
    // briefly disagrees about agents should not take down
    // ping/session/permission alongside it.
    //
    // No context loaders are registered in v1. The Vec exists so a
    // subsequent PR can plug in user-profile / schedule / memory
    // loaders by appending here, without touching system::mod or the
    // chat handler itself.
    let chat_loaders: Arc<Vec<Arc<dyn system::chat_ability::ContextLoader>>> =
        Arc::new(Vec::new());

    let registry = system::build_registry_for_daemon(
        kernel.session_service(),
        kernel.permission_service(),
        kernel.discuss_service(),
        kernel.schedule_service(),
        kernel.loop_service(),
        chat_loaders,
    );

    // Stage-2 dispatcher (executor). Wired with the unified registry
    // and the same NoopGateway the Kernel holds — a real Gateway impl
    // pointing at Axon lands in a focused follow-up.
    let gateway: Arc<dyn GatewayApi> = Arc::new(NoopGateway::new());
    let dispatcher = AbilityDispatcher::new(registry, gateway);

    // Hand the dispatcher back to the Kernel so Kernel::invoke can
    // route ability dispatch through the same registry the proxy
    // uses. This closes the loop from Phase 4 of the chat-as-ability
    // refactor: Kernel::invoke no longer special-cases <agent>.chat
    // — it delegates to whichever handler the registry has under
    // that name.
    let dispatcher_for_kernel = Arc::new(dispatcher.clone());
    kernel.set_dispatcher(dispatcher_for_kernel);

    // Stage-1 resolver. Local node id from EASYNET_NODE_ID env (set
    // by the supervisor from credentials.json) or "self" as a
    // harness default; controls loopback-vs-remote routing.
    let resolver: Arc<dyn TargetResolver> = Arc::new(LocalNodeResolver::new(local_node));
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

    // Schedule tick runner. Fires due schedules every TICK_PERIOD
    // by constructing a real Invocation per fire and routing it
    // through Kernel::invoke. The Kernel admits the Session,
    // dispatches the agent, and terminates — Clients subscribed
    // to system.session.attach see the same lifecycle they would
    // see for a Client-initiated invoke.
    spawn_schedule_tick(kernel_for_tick, schedule_for_tick);

    // Foreground: Control-plane IPC server. Returns when the listener
    // is dropped (i.e. never, in v1 — we exit on SIGTERM via the OS).
    server::run(proxy).await
}

/// Spawn the schedule tick runner. Every `TICK_PERIOD` it asks the
/// ScheduleService for due fires and routes each through
/// `Kernel::invoke` as a real Invocation:
///
///   ability       = "<target_agent>.chat"
///   caller        = local node URA
///   callee        = local node URA (v1 single-node)
///   subject       = schedule URA
///   nonce         = fresh
///   causal_context = Null   (v1; v2 will cite prior receipt)
///   args          = { "prompt": "scheduled fire of <id> at <time>" }
///
/// Kernel::invoke admits a Session keyed by invocation_id and
/// emits the lifecycle events Clients subscribe to via
/// system.session.attach. Failed agent dispatches surface as
/// `Failed(reason)` Receipts — operators see the same diagnostic
/// they would see if they dispatched the agent manually.
///
/// v1 idempotency: an in-memory `last_fire_at` map keyed by
/// `schedule_id` keeps a fire from re-emitting on the next tick if
/// the cron expression's resolution is finer than the tick period.
/// Daemon restart loses this state — schedules due since the last
/// fire will refire once on resume per their misfire policy.
fn spawn_schedule_tick(kernel: Arc<Kernel>, schedule: Arc<ScheduleService>) {
    const TICK_PERIOD: Duration = Duration::from_secs(15);
    tokio::spawn(async move {
        let last_fire: Arc<Mutex<HashMap<ScheduleId, i64>>> =
            Arc::new(Mutex::new(HashMap::new()));
        let mut interval = tokio::time::interval(TICK_PERIOD);
        interval.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Skip);
        let _ = interval.tick().await; // skip the immediate-fire tick
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
                let entry = match schedule
                    .list()
                    .into_iter()
                    .find(|s| s.id == fire.schedule_id)
                {
                    Some(e) => e,
                    None => {
                        eprintln!(
                            "[schedule-tick] schedule {} vanished before fire",
                            fire.schedule_id
                        );
                        continue;
                    }
                };
                let agent = entry.target_agent.as_str().to_string();
                // Use the schedule's prompt template if present;
                // otherwise fall back to a heartbeat-style placeholder.
                // The template renderer substitutes {{schedule_id}},
                // {{fire_at_iso}}, {{catch_up}}, {{target_agent}}.
                let prompt = match &entry.prompt {
                    Some(template) => easynet_cli::runtime::execution::schedule::render_prompt(
                        template,
                        fire.schedule_id.as_str(),
                        &fire.fire_at,
                        fire.catch_up,
                        &agent,
                    ),
                    None => format!(
                        "Scheduled fire of {} at {} (catch_up={})",
                        fire.schedule_id, fire.fire_at, fire.catch_up
                    ),
                };
                let inv = Invocation {
                    caller: format!("easynet://nodes/{}", entry.target_node.as_str()),
                    callee: format!("easynet://nodes/{}", entry.target_node.as_str()),
                    ability: format!("{}.chat", agent),
                    subject: format!("easynet://schedules/{}", fire.schedule_id),
                    nonce_hex: fresh_nonce_hex(),
                    causal_context: CausalContext::Null,
                    args: serde_json::json!({"prompt": prompt}),
                    caller_signature: None,
                };
                eprintln!(
                    "[schedule-tick] firing {} → {}.chat at {}",
                    fire.schedule_id, agent, fire.fire_at
                );
                let kernel_clone = Arc::clone(&kernel);
                tokio::task::spawn_blocking(move || {
                    match kernel_clone.invoke(inv) {
                        Ok(receipt) => {
                            eprintln!(
                                "[schedule-tick]   receipt {} → {:?}",
                                receipt.invocation_id, receipt.terminal
                            );
                        }
                        Err(e) => {
                            eprintln!("[schedule-tick]   invoke error: {e:#}");
                        }
                    }
                });
            }
        }
    });
}
