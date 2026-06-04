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
use easynet_cli::persistence::daemon_config::{default_config_path, DaemonConfig};
use easynet_cli::runtime::agents;
use easynet_cli::runtime::domain::{NodeId, ScheduleId, TenantId};
use easynet_cli::runtime::execution::loop_instance::KernelLoopInvocationDriver;
use easynet_cli::runtime::execution::schedule::ScheduleService;
use easynet_cli::runtime::gateway::NoopGateway;
use easynet_cli::runtime::invocation::{CausalContext, Invocation};
use easynet_cli::runtime::invocation_target::{LocalNodeResolver, TargetResolver};
use easynet_cli::runtime::kernel::Kernel;
use easynet_cli::runtime::kernel_api::KernelApi;
use easynet_cli::services::control::ability_proxy::AbilityProxy;
use easynet_cli::services::control::boot_events::{BootBus, BootEvent};
use easynet_cli::services::control::runtime_dispatch;
use easynet_cli::services::control::server;

/// Heartbeat is opt-in: only spawn the legacy loop if the parent
/// process configured an endpoint. This lets `cargo run --bin
/// easynet-daemon` boot in IPC-only mode for FFI smoke tests without
/// requiring a Hub.
const ENV_HB_ENDPOINT: &str = "_EASYNET_HB_ENDPOINT";
const DEFAULT_PAGES_LISTENER_PORT: u16 = 8787;

#[tokio::main(flavor = "multi_thread")]
async fn main() -> anyhow::Result<()> {
    // Refuse non-empty argv. `easynet-daemon` does not parse
    // subcommand arguments at all; an invocation like
    // `easynet-daemon mcp serve --tenant ... --agent ...` would
    // ignore the args, run the full daemon main, and (most
    // dangerously) re-bind the control.sock file via the
    // bind_at() unconditional unlink + bind path — silently
    // taking over the parent daemon's accept loop. If a host AI
    // client's `.mcp.json` is misconfigured to call this binary
    // for MCP serving (the previous bug fixed in `resolve_easynet_binary`),
    // we want a hard error, not silent socket hijacking.
    //
    // The narrow exception is empty argv (the runtime starter
    // case) — that's the only legitimate use.
    let argv: Vec<String> = std::env::args().collect();
    if argv.len() > 1 {
        eprintln!(
            "[easynet-daemon] this binary takes no command arguments and ignores subcommands. \
             You probably want `easynet {}` instead — `easynet-daemon` is the IPC daemon \
             child spawned by `easynet runtime start`.",
            argv[1..].join(" ")
        );
        std::process::exit(2);
    }

    // v1: a Kernel wrapping a NoopGateway is sufficient for the
    // proxy to construct Receipts. The daemon installs the
    // SubscriberBroker permission variant so a Client UI
    // connected to consent.subscribe sees real pending
    // requests when an agent dispatch is gated. (When no Client
    // is subscribed the broker auto-allows — a daemon running
    // headless does not freeze on permission gates.)
    let boot_bus = BootBus::new();
    boot_bus.emit_started("kernel");
    let kernel = Arc::new(Kernel::new_with_subscriber_broker(Arc::new(
        NoopGateway::new(),
    )));
    boot_bus.emit_ok("kernel");

    boot_bus.emit_started("control-server");
    let control_server = match server::spawn_booting(boot_bus.clone()) {
        Ok(handle) => {
            boot_bus.emit_ok("control-server");
            handle
        }
        Err(err) => {
            boot_bus.emit_failed("control-server", err.to_string());
            return Err(err);
        }
    };

    // Bind sub-services that have a disk-backed store to the
    // current tenant so persistence actually works across daemon
    // restarts. Without this call, ScheduleService and LoopService
    // operate on an in-memory cache only — schedules and loops
    // vanish on every reboot.
    //
    // v1 single-tenant: hardcode `TenantId::default_v1()`. v2 will
    // route this from credentials.json via IPC handshake.
    let tenant = TenantId::default_v1();
    boot_bus.emit_started("tenant-stores");
    if let Err(e) = kernel.schedule_service().bind(&tenant) {
        eprintln!("[daemon] schedule store bind failed: {e:#}");
    }
    if let Err(e) = kernel.loop_service().bind(&tenant) {
        eprintln!("[daemon] loop store bind failed: {e:#}");
    }
    boot_bus.emit_ok("tenant-stores");

    boot_bus.emit_started("loop-controller");
    let local_node = std::env::var("EASYNET_NODE_ID")
        .ok()
        .filter(|s| !s.is_empty())
        .map(NodeId::new)
        .unwrap_or_else(|| NodeId::new("self"));
    let kernel_api: Arc<dyn KernelApi> = Arc::clone(&kernel) as Arc<dyn KernelApi>;
    let loop_driver = Arc::new(KernelLoopInvocationDriver::new(
        Arc::clone(&kernel_api),
        local_node.clone(),
    ));
    if let Err(e) = kernel.loop_service().install_driver(loop_driver) {
        eprintln!("[daemon] loop controller install failed: {e:#}");
    }
    if let Err(e) = kernel.loop_service().resume_inflight() {
        eprintln!("[daemon] loop resume failed: {e:#}");
    }
    boot_bus.emit_ok("loop-controller");

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
    // Default v1 context-loader chain (user_profile + schedule +
    // memory) is auto-attached by build_registry_for_daemon when
    // we pass None. A test or the standalone MCP server that
    // wants chat without any context injection passes
    // Some(Arc::new(Vec::new())) instead.
    // Resolve user-rooted ability identity ONCE at daemon boot.
    // EASYNET_PAGES_USER + credentials.json get read here and
    // never again — the resolved value flows through to
    // build_registry_for_daemon as an explicit argument so the
    // registry build is deterministic and free of global env
    // state.
    boot_bus.emit_started("ability-registry");
    let pages_identity = agents::PagesIdentity::from_env();
    let invocation_ledger = open_invocation_ledger();
    let local_runtime = easynet_axon::invocation::LocalRuntime::new();
    // **Phase 5c**. The `HotAgentRegistrar` cell is constructed
    // here so it can be shared between:
    //   * the registry's `device.agent.start` / `.stop` handler
    //     closures (capture an Arc clone via
    //     `agent_lifecycle_ability::register`), and
    //   * the boot sidecar's post-`LocalRuntime` wiring
    //     (`start_axon_serve_sidecar`) which populates the cell
    //     ONCE with the actual `HotAgentRegistrar` after
    //     `LocalRuntime` + `dispatch_handle` are wired.
    //
    // Pre-set, dispatches of `device.agent.start` see an empty
    // cell and skip runtime registration (logged via op_event); the
    // agent still lands on disk so a daemon restart picks it up via
    // the static registration path. Post-set, every subsequent
    // dispatch registers into `LocalRuntime` and ledger writes start
    // landing.
    let hot_agent_registrar_cell: Arc<agents::agent_lifecycle_ability::SharedHotRegistrarCell> =
        Arc::new(agents::agent_lifecycle_ability::SharedHotRegistrarCell::new());
    let registry = agents::build_registry_for_daemon(
        kernel.session_service(),
        kernel.permission_service(),
        kernel.discuss_service(),
        kernel.schedule_service(),
        kernel.loop_service(),
        invocation_ledger.clone(),
        None,
        pages_identity,
        Some(Arc::clone(&local_runtime)),
        Arc::clone(&hot_agent_registrar_cell),
    );
    kernel.set_local_runtime(Arc::clone(&local_runtime));
    boot_bus.emit_ok("ability-registry");

    // Keep the registry object alive for dynamic side tables whose
    // handlers were installed while building the Axon runtime. Runtime
    // execution itself goes through `local_runtime`.
    let _registry = registry;

    // Stage-1 resolver. Local node id from EASYNET_NODE_ID env (set
    // by the supervisor from credentials.json) or "self" as a
    // harness default; controls loopback-vs-remote routing.
    let resolver: Arc<dyn TargetResolver> = Arc::new(LocalNodeResolver::new(local_node));
    let proxy = AbilityProxy::new_with_runtime(
        Arc::clone(&kernel_api),
        Arc::clone(&local_runtime),
        resolver,
    );

    // RFC-003 PR-1 sidecar: gRPC InvocationServer (transport plane).
    // Start this BEFORE any other daemon listener binds so
    // `daemon-config.toml` is validated at the top of the boot order
    // rather than after control/runtime-dispatch sockets already
    // exist. That keeps the PR-1 "load config before any listener
    // bind" invariant honest even while axon_serve is still a soft
    // dependency.
    #[cfg(feature = "axon-pb")]
    {
        boot_bus.emit_started("axon-serve-sidecar");
        if let Err(e) = easynet_cli::services::axon_serve::start_axon_serve_sidecar(
            Arc::clone(&local_runtime),
            invocation_ledger,
            Arc::clone(&hot_agent_registrar_cell),
        ) {
            eprintln!("[axon-serve] sidecar boot failed: {e:#}");
            boot_bus.emit_failed("axon-serve-sidecar", e.to_string());
            return Err(e);
        }
        boot_bus.emit_ok("axon-serve-sidecar");
    }
    #[cfg(not(feature = "axon-pb"))]
    {
        boot_bus.emit_skipped("axon-serve-sidecar");
    }

    // Optional sidecar: heartbeat. Run on a dedicated OS thread
    // because run_daemon() is blocking (ureq + ctrlc handler). Errors
    // are logged but do not tear down the IPC server; if heartbeat
    // dies the device is in a degraded state but Client UIs can still
    // attach via FFI.
    if std::env::var_os(ENV_HB_ENDPOINT).is_some() {
        boot_bus.emit_started("heartbeat");
        if let Err(err) = std::thread::Builder::new()
            .name("easynet-heartbeat".into())
            .spawn(|| {
                if let Err(e) = run_daemon() {
                    eprintln!("[heartbeat] daemon exited with error: {e:#}");
                }
            })
        {
            boot_bus.emit_failed("heartbeat", err.to_string());
            return Err(err.into());
        }
        boot_bus.emit_ok("heartbeat");
    } else {
        boot_bus.emit_skipped("heartbeat");
    }

    // Schedule tick runner. Fires due schedules every TICK_PERIOD
    // by constructing a real Invocation per fire and routing it
    // through Kernel::invoke. The Kernel admits the Session,
    // dispatches the agent, and terminates — Clients subscribed
    // to device.session.attach see the same lifecycle they would
    // see for a Client-initiated invoke.
    boot_bus.emit_started("schedule-tick");
    spawn_schedule_tick(kernel_for_tick, schedule_for_tick);
    boot_bus.emit_ok("schedule-tick");

    // Step-3 sidecar: runtime-dispatch UDS responder. Listens on a
    // separate socket from `control.sock` because the runtime side
    // talks newline-delimited single-line JSON, while the CLI/MCP IPC
    // server speaks length-delimited frames. axon-runtime opens this
    // socket only when it has resolved a `runtime_local_tools` entry
    // whose `dispatch_endpoint` points at it — i.e., one of the
    // abilities the daemon registered via `runtime.register_local_tool`
    // at boot. A failure here logs but does not tear down the daemon.
    boot_bus.emit_started("runtime-dispatch");
    let dispatch_proxy = proxy.clone();
    tokio::spawn(async move {
        if let Err(e) = runtime_dispatch::run(dispatch_proxy).await {
            eprintln!("[runtime-dispatch] responder exited: {e:#}");
        }
    });
    boot_bus.emit_ok("runtime-dispatch");

    // RFC-006-B v0.6 — Pages reference system listener.
    //
    // Spawned by default from EASYNET_PAGES_PORT (or 8787 when unset).
    // If that port is busy, probe the next 20 ports and write the
    // actual choice into control.json for the CLI's final URL line.
    boot_bus.emit_started("pages-listener");
    let pages_start_port = match resolve_pages_start_port() {
        Ok(port) => port,
        Err(err) => {
            boot_bus.emit_failed("pages-listener", err.to_string());
            return Err(err);
        }
    };
    let pages_port = match easynet_cli::runtime::hub::pages_listener::spawn_first_available(
        pages_start_port,
        easynet_cli::runtime::hub::pages_listener::DEFAULT_PORT_PROBE_SPAN,
    )
    .await
    {
        Ok((port, handle)) => {
            tokio::spawn(async move {
                match handle.await {
                    Ok(Ok(())) => {}
                    Ok(Err(e)) => eprintln!("[pages-listener] exited: {e:#}"),
                    Err(e) => eprintln!("[pages-listener] task failed: {e:#}"),
                }
            });
            boot_bus.emit(BootEvent::PortChosen {
                service: "pages".into(),
                port,
                start: Some(pages_start_port),
            });
            boot_bus.emit_ok("pages-listener");
            port
        }
        Err(err) => {
            boot_bus.emit_failed("pages-listener", err.to_string());
            return Err(err);
        }
    };
    if let Err(err) = control_server.write_discovery(Some(pages_port)) {
        boot_bus.emit_failed("control-discovery", err.to_string());
        return Err(err);
    }

    // The control server has been accepting connections since stage
    // "control-server". This stage flips it from BOOTING mode (where
    // every request except `system.watch_boot` answers with
    // code=BOOTING) to fully dispatching mode by injecting the ready
    // proxy. Naming this "accept-invokes" rather than another
    // "control-ready" avoids the impression of two ready signals.
    boot_bus.emit_started("accept-invokes");
    control_server.state().set_ready(proxy).await;
    boot_bus.emit_ok("accept-invokes");
    boot_bus.emit_ready();

    wait_for_shutdown_signal().await;
    Ok(())
}

fn resolve_pages_start_port() -> anyhow::Result<u16> {
    match std::env::var("EASYNET_PAGES_PORT") {
        Ok(raw) => {
            let port = raw
                .parse::<u16>()
                .map_err(|e| anyhow::anyhow!("EASYNET_PAGES_PORT must be a valid u16: {e}"))?;
            if port == 0 {
                anyhow::bail!("EASYNET_PAGES_PORT must be greater than 0");
            }
            Ok(port)
        }
        Err(std::env::VarError::NotPresent) => Ok(DEFAULT_PAGES_LISTENER_PORT),
        Err(std::env::VarError::NotUnicode(_)) => {
            anyhow::bail!("EASYNET_PAGES_PORT is not valid UTF-8")
        }
    }
}

async fn wait_for_shutdown_signal() {
    #[cfg(unix)]
    {
        let mut sigterm =
            match tokio::signal::unix::signal(tokio::signal::unix::SignalKind::terminate()) {
                Ok(sig) => sig,
                Err(e) => {
                    eprintln!("[daemon] could not install SIGTERM handler: {e:#}");
                    let _ = tokio::signal::ctrl_c().await;
                    return;
                }
            };
        tokio::select! {
            _ = tokio::signal::ctrl_c() => {}
            _ = sigterm.recv() => {}
        }
    }
    #[cfg(not(unix))]
    {
        let _ = tokio::signal::ctrl_c().await;
    }
}

fn open_invocation_ledger() -> Option<Arc<easynet_axon::invocation::InvocationLedger>> {
    let config = match DaemonConfig::load(&default_config_path()) {
        Ok(config) => config,
        Err(err) => {
            eprintln!("[daemon] invocation ledger disabled: daemon config unavailable: {err}");
            return None;
        }
    };
    let path = config.ledger_dir().join("invocations.redb");
    match easynet_axon::invocation::InvocationLedger::open(&path) {
        Ok(ledger) => Some(Arc::new(ledger)),
        Err(err) => {
            eprintln!(
                "[daemon] invocation ledger disabled at {}: {err}",
                path.display()
            );
            None
        }
    }
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
/// device.session.attach. Failed agent dispatches surface as
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
        let last_fire: Arc<Mutex<HashMap<ScheduleId, i64>>> = Arc::new(Mutex::new(HashMap::new()));
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
                let local_device_ura =
                    easynet_cli::ura::device_ura("default", entry.target_node.as_str());
                let schedule_subject_ura = easynet_cli::ura::resource_dot_ura(
                    "default",
                    &format!("schedule.{}", fire.schedule_id.as_str()),
                    "",
                );
                let inv = match Invocation::try_new(
                    local_device_ura.clone(),
                    local_device_ura,
                    format!("{}.chat", agent),
                    schedule_subject_ura,
                    CausalContext::Null,
                    serde_json::json!({"prompt": prompt}),
                ) {
                    Ok(inv) => inv,
                    Err(err) => {
                        eprintln!(
                            "[schedule-tick] invalid invocation for {}: {err:#}",
                            fire.schedule_id
                        );
                        continue;
                    }
                };
                eprintln!(
                    "[schedule-tick] firing {} → {}.chat at {}",
                    fire.schedule_id, agent, fire.fire_at
                );
                let kernel_clone = Arc::clone(&kernel);
                tokio::task::spawn_blocking(move || match kernel_clone.invoke(inv) {
                    Ok(receipt) => {
                        eprintln!(
                            "[schedule-tick]   receipt {} → {:?}",
                            receipt.invocation_id, receipt.terminal
                        );
                    }
                    Err(e) => {
                        eprintln!("[schedule-tick]   invoke error: {e:#}");
                    }
                });
            }
        }
    });
}
