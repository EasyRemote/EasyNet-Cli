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

use std::sync::Arc;

use easynet_cli::facade::cli::run_daemon;
use easynet_cli::runtime::ability_dispatch::AbilityDispatcher;
use easynet_cli::runtime::domain::{NodeId, TenantId};
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

    // Foreground: Control-plane IPC server. Returns when the listener
    // is dropped (i.e. never, in v1 — we exit on SIGTERM via the OS).
    server::run(proxy).await
}
