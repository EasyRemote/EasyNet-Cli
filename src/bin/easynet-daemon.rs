// EasyNet Daemon — process entry point
// =====================================
//
// File: src/bin/easynet-daemon.rs
// Description: Long-running daemon entry — one process owns the
//              Control-plane IPC server, daemon Invocation, and ability
//              hosting. Directory liveness is owned by the
//              session-lifetime federation.heartbeat loop inside the
//              invocation transport.
//
// Current shape
// -------------
// - Always: spin up a tokio multi-thread runtime and run the daemon
//   IPC surfaces on it: boot/status control and daemon Invocation.
//
// What is NOT here yet
// --------------------
// - Schedule tick (PR-SCHED).
// - Nothing on control.sock dispatches product abilities. Product
//   calls enter through daemon Invocation.
//
// Author: Silan Hu <silan.hu@u.nus.edu>
// Copyright (c) 2026 EasyNet. All rights reserved.

use std::collections::HashMap;
use std::sync::{Arc, Mutex};
use std::time::Duration;

use anyhow::Context as _;
use chrono::Utc;
use easynet_cli::core::domain::{NodeId, ScheduleId, TenantId};
use easynet_cli::daemon::ability::builtins::agents::{
    discover as discover_ability, lifecycle as agent_lifecycle_ability,
};
use easynet_cli::daemon::ability::catalog as ability_catalog;
use easynet_cli::daemon::ability::conformance::{
    BaselineConformanceReport, DeviceBaseline, HubBaseline, RegistryConformance,
};
use easynet_cli::daemon::ability::conformance::{DaemonInvocationSurface, RuntimeAdminConformance};
use easynet_cli::daemon::ability::health as ability_health;
use easynet_cli::daemon::boot::kernel::api::KernelApi;
use easynet_cli::daemon::boot::kernel::Kernel;
use easynet_cli::daemon::control::boot_events::{BootBus, BootEvent};
use easynet_cli::daemon::control::discovery::DaemonIdentity;
use easynet_cli::daemon::control::{discovery, server};
use easynet_cli::daemon::execution::loop_instance::KernelLoopInvocationDriver;
use easynet_cli::daemon::execution::runtime_identity::LocalRuntimeInvocationIdentity;
use easynet_cli::daemon::execution::schedule::ScheduleService;
use easynet_cli::daemon::federation::read_model::authority_published_abilities::AuthorityPublishedAbilityStore;
use easynet_cli::daemon::persistence::config;
use easynet_cli::daemon::persistence::daemon_config::{
    default_config_path, resolved_local_uds_path_with_env_override, DaemonConfig, DaemonMode,
};
use easynet_cli::daemon::resources::context::clipboard_tracker;

const ENV_BOOTSTRAP_MEDIA_RESOURCES: &str = "EASYNET_BOOTSTRAP_MEDIA_RESOURCES";
const DEFAULT_PAGES_LISTENER_PORT: u16 = 8787;

fn device_ability_replay_fatal_message(
    report: &easynet_cli::daemon::ability::builtins::device_control::ability_management::registrar::ReplayReport,
) -> Option<String> {
    if report.runtime_not_ready || report.store_unreadable || report.stale > 0 || report.errored > 0
    {
        return Some(format!(
            "device ability replay failed before daemon start: runtime_not_ready={}, \
             store_unreadable={}, stale={}, quarantined={}, errored={}, outcomes={}",
            report.runtime_not_ready,
            report.store_unreadable,
            report.stale,
            report.quarantined,
            report.errored,
            report.outcomes_json()
        ));
    }
    None
}

fn report_device_ability_replay(
    report: &easynet_cli::daemon::ability::builtins::device_control::ability_management::registrar::ReplayReport,
) -> anyhow::Result<()> {
    if let Some(message) = device_ability_replay_fatal_message(report) {
        anyhow::bail!(message);
    }
    eprintln!(
        "[device-ability] replay: {} registered, {} stale, {} quarantined, {} errored, \
         runtime_not_ready={}, store_unreadable={}, outcomes={}",
        report.registered,
        report.stale,
        report.quarantined,
        report.errored,
        report.runtime_not_ready,
        report.store_unreadable,
        report.outcomes_json()
    );
    Ok(())
}

fn collect_baseline_failure(failures: &mut Vec<String>, report: BaselineConformanceReport) {
    if !report.is_conformant() {
        failures.push(report.panic_message());
    }
}

fn assert_daemon_baseline_conformance(
    mode: DaemonMode,
    registry: &easynet_cli::daemon::ability::dispatch::AxonAbilityCatalog,
) -> Result<(), String> {
    let registry_conformance = RegistryConformance::new(registry);
    let mut failures = Vec::new();

    if matches!(mode, DaemonMode::Device | DaemonMode::Both) {
        let device = DeviceBaseline::required_abilities();
        collect_baseline_failure(&mut failures, registry_conformance.check("device", &device));
    }

    if matches!(mode, DaemonMode::Hub | DaemonMode::Both) {
        let hub = HubBaseline::required_abilities();
        collect_baseline_failure(&mut failures, registry_conformance.check("hub", hub));
        collect_baseline_failure(
            &mut failures,
            DaemonInvocationSurface::from_daemon_surface().check("hub", hub),
        );
        collect_baseline_failure(
            &mut failures,
            RuntimeAdminConformance::from_daemon_surface().check("hub", hub),
        );
    }

    if failures.is_empty() {
        Ok(())
    } else {
        Err(failures.join("\n"))
    }
}

fn ensure_daemon_runtime_identity(config: &DaemonConfig) -> anyhow::Result<()> {
    use easynet_cli::daemon::identity::self_identity::KeyringClient;

    easynet_cli::daemon::keyring::lifecycle::ensure_key_service_running()
        .context("start or attach daemon key service")?;
    let client = KeyringClient::default_path();

    // `_system.local` is the daemon's internal caller identity. It uses the
    // same daemon-owned custody service as Device and Hub identities; a
    // process-local generated key would create a second authentication root.
    easynet_cli::daemon::identity::self_identity::ensure_daemon_local_system_identity(&client)
        .map_err(|error| anyhow::anyhow!("ensure daemon-local runtime identity: {error}"))?;

    match config.mode() {
        DaemonMode::Hub => {
            let hub_ura = easynet_cli::core::ura::hub_ura(config.realm());
            client
                .ensure(&hub_ura)
                .map_err(|error| anyhow::anyhow!("ensure Hub runtime identity: {error}"))?;
        }
        DaemonMode::Device | DaemonMode::Both => {
            let credentials = config::load_credentials().with_context(|| {
                format!(
                    "{} daemon requires paired credentials before identity provisioning",
                    config.mode().as_str()
                )
            })?;
            if credentials.realm_str() != config.realm() {
                anyhow::bail!(
                    "daemon credentials realm `{}` does not match configured realm `{}`",
                    credentials.realm_str(),
                    config.realm()
                );
            }
            let owner_ura =
                easynet_cli::core::ura::device_ura(credentials.realm_str(), &credentials.node_id);
            client
                .ensure(&owner_ura)
                .map_err(|error| anyhow::anyhow!("ensure Device runtime identity: {error}"))?;
            if config.mode() == DaemonMode::Both {
                let hub_ura = easynet_cli::core::ura::hub_ura(config.realm());
                client
                    .ensure(&hub_ura)
                    .map_err(|error| anyhow::anyhow!("ensure Hub runtime identity: {error}"))?;
            }
        }
    }
    Ok(())
}

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

    // The key service is a detached custody process so that it cannot be
    // inherited accidentally by arbitrary child commands. Its lifecycle is
    // nevertheless owned by this daemon: every normal shutdown and every
    // boot failure must reclaim only the child this daemon started. The guard
    // preserves that terminal transition across all early-return boot paths.
    let _key_service_shutdown = KeyServiceShutdownGuard;

    // The daemon installs the interactive Kernel for loop,
    // permission, and session services. A Client UI connected to
    // consent.subscribe sees real pending requests when an agent
    // dispatch is gated. When no Client is subscribed, the broker
    // uses the explicit headless policy so unattended daemon work
    // does not freeze on permission gates.
    let boot_bus = BootBus::new();
    boot_bus.emit_started("kernel");
    let kernel = Arc::new(Kernel::new_interactive());
    boot_bus.emit_ok("kernel");

    boot_bus.emit_started("daemon-config");
    let daemon_config = match DaemonConfig::load(&default_config_path()) {
        Ok(config) => {
            boot_bus.emit_ok("daemon-config");
            config
        }
        Err(err) => {
            boot_bus.emit_failed("daemon-config", err.to_string());
            return Err(err.into());
        }
    };

    boot_bus.emit_started("daemon-key-service");
    if let Err(error) = ensure_daemon_runtime_identity(&daemon_config) {
        boot_bus.emit_failed("daemon-key-service", error.to_string());
        return Err(error);
    }
    boot_bus.emit_ok("daemon-key-service");

    // Bind runtime state services to the current daemon identity.
    // Session/discuss read models must carry the same node/tenant
    // facts that the signer, descriptor resolver, and admission
    // authority validate. ScheduleService and LoopService also bind
    // tenant-scoped stores here so their state survives restarts.
    //
    let tenant = TenantId::new(daemon_config.realm().to_string());
    boot_bus.emit_started("tenant-stores");
    let daemon_identity = ready_daemon_identity(&daemon_config)?;
    if let Some(node_id) = daemon_identity.node_id {
        let runtime_node = NodeId::new(node_id);
        kernel
            .session_service()
            .bind_runtime(runtime_node.clone(), tenant.clone())
            .map_err(|err| anyhow::anyhow!("bind session runtime identity: {err:#}"))?;
        kernel
            .discuss_service()
            .bind_runtime(runtime_node, tenant.clone())
            .map_err(|err| anyhow::anyhow!("bind discuss runtime identity: {err:#}"))?;
    }
    if let Err(e) = kernel.schedule_service().bind(&tenant) {
        eprintln!("[daemon] schedule store bind failed: {e:#}");
    }
    if let Err(e) = kernel.loop_service().bind(&tenant) {
        eprintln!("[daemon] loop store bind failed: {e:#}");
    }
    boot_bus.emit_ok("tenant-stores");

    let runtime_invocation_identity = local_runtime_invocation_identity(&daemon_config)?;

    boot_bus.emit_started("loop-controller");
    if media_resource_bootstrap_enabled() {
        match config::load_credentials() {
            Ok(creds) => {
                let owner_agent =
                    easynet_cli::core::ura::device_ura(creds.realm_str(), &creds.node_id);
                match easynet_cli::daemon::ability::builtins::resources::media::resource_bootstrap::seed_default_device_resources(
                    creds.realm_str(),
                    &owner_agent,
                ) {
                    Ok(count) => eprintln!("[daemon] media resources ready: {count} known"),
                    Err(err) => eprintln!("[daemon] media resource bootstrap failed: {err:#}"),
                }
            }
            Err(err) => {
                eprintln!("[daemon] media resource bootstrap skipped: {err}");
            }
        }
    } else {
        eprintln!("[daemon] media resource bootstrap skipped: {ENV_BOOTSTRAP_MEDIA_RESOURCES}=0");
    }
    let kernel_api: Arc<dyn KernelApi> = Arc::clone(&kernel) as Arc<dyn KernelApi>;
    if let Some(identity) = runtime_invocation_identity.clone() {
        let loop_driver = Arc::new(KernelLoopInvocationDriver::new(
            Arc::clone(&kernel_api),
            identity,
        ));
        if let Err(e) = kernel.loop_service().install_driver(loop_driver) {
            eprintln!("[daemon] loop controller install failed: {e:#}");
        }
        if let Err(e) = kernel.loop_service().resume_inflight() {
            eprintln!("[daemon] loop resume failed: {e:#}");
        }
    } else {
        eprintln!(
            "[daemon] loop controller has no local device invocation identity; driver not installed"
        );
    }
    boot_bus.emit_ok("loop-controller");

    // Build the daemon-owned ability registry off the SAME sub-service
    // handles the Kernel holds. This is the U1 unity property at
    // the boot path: every ability lookup and every KernelApi call
    // observe one set of sub-service state. A regression that built
    // the registry off fresh sub-services (the pre-PR shape) would
    // give the IPC plane a parallel state not reachable from the
    // Kernel — silently breaking session.list / discuss.subscribe.
    // Snapshot the sub-service handles used by the tick runner. The
    // schedule handle reads due work; the kernel handle is the C*
    // unity entry — the tick runner constructs an SDK descriptor-bound
    // request and routes it through Kernel::invoke.
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
    let pages_identity = match daemon_config.mode() {
        DaemonMode::Device | DaemonMode::Both => {
            easynet_cli::daemon::ability::builtins::resources::pages::PagesIdentity::try_from_env()
                .context("resolve Pages identity for ability registry")?
        }
        DaemonMode::Hub => {
            easynet_cli::daemon::ability::builtins::resources::pages::PagesIdentity::default()
        }
    };
    let invocation_ledger = open_invocation_ledger();
    let authority_published_abilities = AuthorityPublishedAbilityStore::new();
    let voice_calls = match daemon_config.mode() {
        DaemonMode::Hub | DaemonMode::Both => {
            easynet_cli::daemon::persistence::voice_calls::HubRealmVoiceCallRepository::from_env(
                daemon_config.realm(),
            )?
        }
        DaemonMode::Device => None,
    };
    let mut receipt_owner_uras = Vec::new();
    let mut hosted_agent_device_ura = None;
    let authority_context = match daemon_config.mode() {
        DaemonMode::Hub => {
            let hub_ura = easynet_cli::core::ura::hub_ura(daemon_config.realm());
            receipt_owner_uras.push(hub_ura.clone());
            easynet_cli::daemon::ability::dispatch::AbilityAuthorityContext::for_realm_authority_root(
                hub_ura,
            )?
        }
        DaemonMode::Device | DaemonMode::Both => {
            let creds = config::load_credentials().map_err(|err| {
                anyhow::anyhow!(
                    "daemon ability registry requires paired credentials in {} mode: {err}",
                    daemon_config.mode().as_str()
                )
            })?;
            let device_ura = easynet_cli::core::ura::device_ura(creds.realm_str(), &creds.node_id);
            receipt_owner_uras.push(device_ura.clone());
            let hosted_agent_uras = easynet_cli::daemon::persistence::hosted_agent_authority_roots(
            )
            .map_err(|err| {
                anyhow::anyhow!(
                    "daemon ability registry requires readable hosted-agent lifecycle state: {err}"
                )
            })?;
            hosted_agent_device_ura = Some(device_ura.clone());
            match daemon_config.mode() {
                DaemonMode::Device => easynet_cli::daemon::ability::dispatch::AbilityAuthorityContext::for_device_authority_root_with_hosted_agents(device_ura, hosted_agent_uras)?,
                DaemonMode::Both => {
                    receipt_owner_uras.push(easynet_cli::core::ura::hub_ura(daemon_config.realm()));
                    easynet_cli::daemon::ability::dispatch::AbilityAuthorityContext::for_combined_authority_roots_with_hosted_agents(device_ura, hosted_agent_uras)?
                },
                DaemonMode::Hub => unreachable!("Hub mode handled above"),
            }
        }
    };
    let mut receipt_authority_config =
        easynet_cli::daemon::axon_bridge::runtime_factory::ProductionReceiptAuthorityConfig::new(
            receipt_owner_uras,
        );
    if let Some(device_ura) = hosted_agent_device_ura {
        let inventory = authority_context
            .hosted_agent_signing_inventory()
            .ok_or_else(|| {
                anyhow::anyhow!(
                    "hosted-Agent receipt authority requires the catalog-owned inventory"
                )
            })?;
        receipt_authority_config =
            receipt_authority_config.with_hosted_agent_inventory(device_ura, inventory);
    }
    let runtime_trust_anchor = easynet_cli::daemon::trust::cell::SharedTrustAnchor::new(Arc::new(
        easynet_cli::daemon::trust::anchor::RealmTrustAnchor::default(),
    ));
    let federation_runtime = easynet_cli::daemon::invocation::build_invocation_federation_runtime(
        &daemon_config,
        runtime_trust_anchor.clone(),
    )
    .context("build canonical invocation federation providers")?;
    let daemon_runtime =
        easynet_cli::daemon::axon_bridge::runtime_factory::build_production_local_runtime(
            receipt_authority_config,
            federation_runtime.trusted_identity_resolver(),
        )
        .map_err(|error| anyhow::anyhow!("build owner-bound Axon receipt runtime: {error}"))?;
    let local_runtime = daemon_runtime.runtime();
    // **Phase 5c**. The `HotAgentRegistrar` cell is constructed
    // here so it can be shared between:
    //   * the registry's `agent.start` / `.stop` handler
    //     closures (capture an Arc clone via
    //     `agent_lifecycle_ability::register`), and
    //   * the Invocation transport's post-`LocalRuntime` wiring
    //     (`start_daemon_invocation_transport`) which populates the cell
    //     ONCE with the actual `HotAgentRegistrar` after
    //     `LocalRuntime` + `dispatch_handle` are wired.
    //
    // Pre-set, dispatches of `agent.start` see an empty
    // cell and skip runtime registration (logged via op_event); the
    // agent still lands on disk so a daemon restart replays it through
    // the dynamic registrar after the catalogue is wired. Post-set,
    // every subsequent dispatch registers catalogue/control-plane and
    // `LocalRuntime` rows in one transaction, so ledger writes start
    // landing.
    let hot_agent_registrar_cell: Arc<agent_lifecycle_ability::SharedHotRegistrarCell> =
        Arc::new(agent_lifecycle_ability::SharedHotRegistrarCell::new());
    let discover_federation_resolver_cell =
        Arc::new(discover_ability::DeferredDiscoverFederationResolver::new());
    let discover_federation_resolver: discover_ability::SharedDiscoverFederationResolver =
        discover_federation_resolver_cell.clone();
    let access_control_stores = Arc::new(
        easynet_cli::daemon::persistence::access_control::AccessControlStoreRegistry::default(),
    );
    let built_registry = ability_catalog::build_registry_for_daemon_result(
        ability_catalog::RegistryDaemonBuildConfig {
            services: ability_catalog::RegistryBuildServices::new(
                kernel.session_service(),
                kernel.permission_service(),
                kernel.discuss_service(),
                kernel.schedule_service(),
                kernel.loop_service(),
            )
            .with_discover_federation_resolver(Arc::clone(&discover_federation_resolver))
            .with_access_control_stores(Arc::clone(&access_control_stores)),
            invocation_ledger: invocation_ledger.clone(),
            loaders: None,
            pages_identity,
            local_runtime: Some(Arc::clone(&local_runtime)),
            authority_context,
            hot_agent_registrar_cell: Arc::clone(&hot_agent_registrar_cell),
            shared_stores: {
                let stores = ability_catalog::RegistrySharedStores::new(Arc::clone(
                    &authority_published_abilities,
                ));
                match voice_calls {
                    Some(repository) => {
                        let provider =
                            easynet_cli::daemon::ability::builtins::resources::voice_contract::VoiceCallProviderAssembly::try_new(
                                repository,
                            )
                            .map_err(|error| anyhow::anyhow!("assemble Hub Voice provider: {error}"))?;
                        stores.with_voice_call_provider_assembly(provider)
                    }
                    None => stores,
                }
            },
        },
    )?;
    let registry = Arc::clone(&built_registry.catalog);
    kernel.set_local_runtime(Arc::clone(&local_runtime));
    boot_bus.emit_ok("ability-registry");

    boot_bus.emit_started("ability-conformance");
    if let Err(message) = assert_daemon_baseline_conformance(daemon_config.mode(), &registry) {
        boot_bus.emit_failed("ability-conformance", message.clone());
        panic!("{message}");
    }
    boot_bus.emit_ok("ability-conformance");

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

    // Attach and replay the Device ability registrar only when this process
    // actually hosts Device authority. Hub-only mode has no Device runtime
    // plane, so it must not read or materialize durable Device deployments.
    // In Device/Both this is the boot half of the `ability.deploy` install
    // transaction and remains fail-closed on stale or errored rows.
    if matches!(daemon_config.mode(), DaemonMode::Device | DaemonMode::Both) {
        if let Some(device_registrar) = built_registry.device_registrar_cell.get() {
            device_registrar.set_control_plane_catalog(Arc::downgrade(&registry))?;
            device_registrar.set_runtime(Arc::clone(&local_runtime))?;
            let report = device_registrar.replay_from_store().await;
            report_device_ability_replay(&report)?;
        }
    }

    // Keep the registry object alive for dynamic side tables whose
    // handlers were installed while building the Axon runtime. Runtime
    // execution itself goes through `local_runtime`.
    let _registry = Arc::clone(&registry);

    // Canonical daemon Invocation transport: gRPC InvocationServer.
    // Start this BEFORE any other daemon listener binds so
    // `daemon-config.toml` is validated at the top of the boot order
    // rather than after the control socket already exists. That preserves
    // the PR-1 invariant that config loads before any listener binds whenever
    // the feature-gated transport is compiled in.
    // Hold the session-shutdown handle for the daemon's lifetime;
    // dropping it at shutdown drains the live `session.open` dial
    // (F-007 — was Box::leak'd). Bound directly from the boot result:
    // Ok yields the handle, Err returns, so there is no never-read
    // placeholder.
    let session_shutdown = {
        boot_bus.emit_started("daemon-invocation-transport");
        let dependencies = easynet_cli::daemon::invocation::InvocationTransportDependencies {
            daemon_runtime: daemon_runtime.clone(),
            federation_runtime: federation_runtime.clone(),
            runtime_trust_anchor: runtime_trust_anchor.clone(),
            local_ability_catalog: Arc::clone(&registry),
            access_control_stores: Arc::clone(&access_control_stores),
            invocation_cancellations: built_registry.invocation_cancellations.clone(),
            invocation_ledger,
            hot_agent_registrar_cell: Arc::clone(&hot_agent_registrar_cell),
            plugin_runtime_manager: Some(Arc::clone(&built_registry.plugin_runtime_manager)),
            authority_published_abilities: Arc::clone(&authority_published_abilities),
            discover_federation_resolver: Some(Arc::clone(&discover_federation_resolver_cell)),
        };
        match easynet_cli::daemon::invocation::start_daemon_invocation_transport(dependencies) {
            Ok(handle) => {
                boot_bus.emit_ok("daemon-invocation-transport");
                handle
            }
            Err(e) => {
                eprintln!("[daemon-invocation] transport boot failed: {e:#}");
                boot_bus.emit_failed("daemon-invocation-transport", e.to_string());
                return Err(e);
            }
        }
    };
    let invocation_capability_flags = session_shutdown.capability_flags().to_vec();

    // Clipboard tracker (Context surface). Always spawned; the thread
    // is inert (config-stat + sleep) until `easynet context clipboard
    // on` flips the persisted flag, so an off-by-default user pays one
    // sleeping thread and zero clipboard access.
    boot_bus.emit_started("clipboard-tracker");
    clipboard_tracker::spawn();
    boot_bus.emit_ok("clipboard-tracker");

    // Ability service-health monitor. Probes manifest abilities that
    // declare `[health]`, runs `[boot]` self-heal when the backing
    // service is down, and feeds `meta.list_abilities` the
    // health_status metadata the catalog surfaces. Abilities without
    // a `[health]` section cost nothing — the thread only ticks over
    // declared probes.
    boot_bus.emit_started("ability-health");
    ability_health::spawn();
    boot_bus.emit_ok("ability-health");

    // Schedule tick runner. Fires due schedules every TICK_PERIOD
    // by constructing a canonical descriptor-bound request per fire and routing
    // it through Kernel::invoke. The Kernel admits the Session,
    // dispatches the agent, and terminates — Clients subscribed
    // to session.attach see the same lifecycle they would
    // see for a Client-initiated invoke.
    boot_bus.emit_started("schedule-tick");
    if let Some(identity) = runtime_invocation_identity {
        spawn_schedule_tick(kernel_for_tick, schedule_for_tick, identity);
        boot_bus.emit_ok("schedule-tick");
    } else {
        eprintln!("[daemon] schedule tick has no local device invocation identity; skipped");
        boot_bus.emit_skipped("schedule-tick");
    }

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
    let pages_port =
        match easynet_cli::daemon::resources::pages::pages_listener::spawn_first_available(
            pages_start_port,
            easynet_cli::daemon::resources::pages::pages_listener::DEFAULT_PORT_PROBE_SPAN,
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
    let runtime_discovery = match ready_runtime_discovery(invocation_capability_flags) {
        Ok(snapshot) => snapshot,
        Err(err) => {
            boot_bus.emit_failed("control-discovery", err.to_string());
            return Err(err);
        }
    };
    if let Err(err) = control_server.write_ready_discovery(Some(pages_port), runtime_discovery) {
        boot_bus.emit_failed("control-discovery", err.to_string());
        return Err(err);
    }

    // The control server remains a boot/status socket. Product
    // ability calls use daemon.sock Invocation and dispatch to the
    // embedded LocalRuntime in process.
    boot_bus.emit_started("control-ready");
    boot_bus.emit_ok("control-ready");
    boot_bus.emit_ready();

    wait_for_shutdown_signal().await;
    // Cancel the session supervisor (drains the live `session.open`
    // dial -> clean Eof at the hub) before tearing down control sockets.
    drop(session_shutdown);
    cleanup_control_discovery();
    Ok(())
}

struct KeyServiceShutdownGuard;

impl Drop for KeyServiceShutdownGuard {
    fn drop(&mut self) {
        if let Err(error) = easynet_cli::daemon::keyring::lifecycle::shutdown_key_service() {
            eprintln!("[daemon] key-service shutdown failed: {error:#}");
        }
    }
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

fn ready_runtime_discovery(
    capability_flags: Vec<String>,
) -> anyhow::Result<server::ControlRuntimeDiscovery> {
    let config = DaemonConfig::load(&default_config_path())?;
    let ready_identity = ready_daemon_runtime_identity(&config)?;
    let capabilities = ReadyRuntimeCapabilities::new(capability_flags);
    capabilities.validate_for_mode(
        config.mode(),
        ready_identity.paired_user_runtime_signer_required,
    )?;
    Ok(server::ControlRuntimeDiscovery {
        invocation_endpoint: resolved_local_uds_path_with_env_override(),
        daemon_identity: ready_identity.daemon_identity,
        capability_flags: capabilities.into_flags(),
    })
}

#[derive(Debug, Clone)]
struct ReadyRuntimeCapabilities {
    flags: Vec<String>,
}

impl ReadyRuntimeCapabilities {
    fn new(flags: Vec<String>) -> Self {
        Self { flags }
    }

    fn contains(&self, flag: &str) -> bool {
        self.flags.iter().any(|candidate| candidate == flag)
    }

    fn validate_for_mode(
        &self,
        mode: DaemonMode,
        paired_user_runtime_signer_required: bool,
    ) -> anyhow::Result<()> {
        match mode {
            DaemonMode::Hub => Ok(()),
            DaemonMode::Device | DaemonMode::Both => {
                if !paired_user_runtime_signer_required
                    || self.contains(discovery::flags::PAIRED_USER_RUNTIME_SIGNER)
                {
                    Ok(())
                } else {
                    anyhow::bail!(
                        "{} daemon ready discovery requires invocation boot proof `{}`; \
                         refusing to advertise Ready before paired User caller-signer custody is available",
                        mode.as_str(),
                        discovery::flags::PAIRED_USER_RUNTIME_SIGNER
                    )
                }
            }
        }
    }

    fn into_flags(self) -> Vec<String> {
        self.flags
    }
}

#[derive(Debug, Clone)]
struct ReadyDaemonRuntimeIdentity {
    daemon_identity: DaemonIdentity,
    paired_user_runtime_signer_required: bool,
}

fn ready_daemon_runtime_identity(
    config: &DaemonConfig,
) -> anyhow::Result<ReadyDaemonRuntimeIdentity> {
    let (node_id, paired_user_runtime_signer_required) = match config.mode() {
        DaemonMode::Hub => (None, false),
        DaemonMode::Device | DaemonMode::Both => {
            let credentials = config::load_credentials().with_context(|| {
                format!(
                    "{} daemon ready discovery requires paired credentials",
                    config.mode().as_str()
                )
            })?;
            if credentials.realm_str() != config.realm() {
                anyhow::bail!(
                    "daemon ready discovery credentials realm `{}` does not match configured realm `{}`",
                    credentials.realm_str(),
                    config.realm()
                );
            }
            let node_id = credentials.node_id.trim();
            if node_id.is_empty() {
                anyhow::bail!("daemon ready discovery paired credentials node_id is empty");
            }
            (
                Some(node_id.to_string()),
                matches!(
                    credentials.runtime_user_binding()?,
                    config::RuntimeUserBinding::Bound { .. }
                ),
            )
        }
    };
    Ok(ReadyDaemonRuntimeIdentity {
        daemon_identity: DaemonIdentity {
            mode: config.mode().as_str().to_string(),
            realm: config.realm().to_string(),
            node_id,
        },
        paired_user_runtime_signer_required,
    })
}

fn ready_daemon_identity(config: &DaemonConfig) -> anyhow::Result<DaemonIdentity> {
    Ok(ready_daemon_runtime_identity(config)?.daemon_identity)
}

fn local_runtime_invocation_identity(
    config: &DaemonConfig,
) -> anyhow::Result<Option<LocalRuntimeInvocationIdentity>> {
    let identity = ready_daemon_identity(config)?;
    let Some(node_id) = identity.node_id else {
        return Ok(None);
    };
    LocalRuntimeInvocationIdentity::new(identity.realm, NodeId::new(node_id)).map(Some)
}

fn media_resource_bootstrap_enabled() -> bool {
    match std::env::var(ENV_BOOTSTRAP_MEDIA_RESOURCES) {
        Ok(value) => !matches!(
            value.trim().to_ascii_lowercase().as_str(),
            "0" | "false" | "no" | "off"
        ),
        Err(_) => true,
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

fn cleanup_control_discovery() {
    let path = discovery::default_path();
    if let Err(err) = discovery::remove(&path) {
        eprintln!(
            "[daemon] failed to remove control discovery file at {}: {err:#}",
            path.display()
        );
    }
}

fn open_invocation_ledger() -> Option<Arc<axon_sdk::invocation::InvocationLedger>> {
    let config = match DaemonConfig::load(&default_config_path()) {
        Ok(config) => config,
        Err(err) => {
            eprintln!("[daemon] invocation ledger disabled: daemon config unavailable: {err}");
            return None;
        }
    };
    let path = config.ledger_dir().join("invocations.redb");
    match axon_sdk::invocation::InvocationLedger::open(&path) {
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
/// `Kernel::invoke` as an SDK descriptor-bound request:
///
///   ability       = "<target_agent>.chat"
///   caller        = `_system.local`
///   callee        = target device URA in the daemon-configured realm
///   subject       = schedule URA
///   nonce         = fresh
///   causal_context = None   (v1; v2 will cite a canonical prior receipt)
///   args          = { "prompt": "scheduled fire of <id> at <time>" }
///
/// Kernel::invoke admits a Session keyed by Axon's invocation id and
/// emits the lifecycle events Clients subscribe to via
/// session.attach. Failed agent dispatches surface through Axon's signed
/// terminal receipt, so operators see the same diagnostic they would see if
/// they dispatched the agent manually.
///
/// v1 idempotency: an in-memory `last_fire_at` map keyed by
/// `schedule_id` keeps a fire from re-emitting on the next tick if
/// the cron expression's resolution is finer than the tick period.
/// Daemon restart loses this state — schedules due since the last
/// fire will refire once on resume per their misfire policy.
fn spawn_schedule_tick(
    kernel: Arc<Kernel>,
    schedule: Arc<ScheduleService>,
    identity: LocalRuntimeInvocationIdentity,
) {
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
            let due = match schedule.due(now, lookup_last) {
                Ok(due) => due,
                Err(err) => {
                    eprintln!("[schedule-tick] due selection failed: {err:#}");
                    continue;
                }
            };
            if due.is_empty() {
                continue;
            }
            for fire in due {
                let now_ms = now.timestamp_millis();
                if let Ok(mut g) = last_fire.lock() {
                    g.insert(fire.schedule_id.clone(), now_ms);
                }
                let schedules = match schedule.list() {
                    Ok(schedules) => schedules,
                    Err(err) => {
                        eprintln!("[schedule-tick] schedule snapshot failed: {err:#}");
                        continue;
                    }
                };
                let entry = match schedules.into_iter().find(|s| s.id == fire.schedule_id) {
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
                // The template renderer substitutes {{schedule_id}},
                // {{fire_at_iso}}, {{catch_up}}, {{target_agent}}.
                let prompt = easynet_cli::daemon::execution::schedule::render_prompt(
                    &entry.prompt,
                    fire.schedule_id.as_str(),
                    &fire.fire_at,
                    fire.catch_up,
                    &agent,
                );
                let (local_device_ura, schedule_subject_ura) =
                    schedule_tick_invocation_uras(&identity, &entry.target_node, &fire.schedule_id);
                let payload = match serde_json::to_vec(&serde_json::json!({"prompt": prompt})) {
                    Ok(payload) => payload,
                    Err(err) => {
                        eprintln!(
                            "[schedule-tick] encode invocation for {}: {err:#}",
                            fire.schedule_id
                        );
                        continue;
                    }
                };
                let request = match kernel.prepare_local_system_rpc(
                    &local_device_ura,
                    &format!("{}.chat", agent),
                    &schedule_subject_ura,
                    payload,
                ) {
                    Ok(request) => request,
                    Err(err) => {
                        eprintln!(
                            "[schedule-tick] prepare canonical invocation for {}: {err:#}",
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
                tokio::task::spawn_blocking(move || match kernel_clone.invoke(request) {
                    Ok(finalized) => {
                        eprintln!(
                            "[schedule-tick]   receipt {} → {:?}",
                            finalized.terminal_receipt.invocation_id(),
                            finalized.terminal_state
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

fn schedule_tick_invocation_uras(
    identity: &LocalRuntimeInvocationIdentity,
    target_node: &NodeId,
    schedule_id: &ScheduleId,
) -> (String, String) {
    (
        identity.device_ura_for_node(target_node.as_str()),
        identity.resource_subject_ura(&format!("schedule.{}", schedule_id.as_str()), ""),
    )
}

#[cfg(test)]
mod tests {
    use super::*;
    use easynet_cli::daemon::ability::builtins::device_control::ability_management::registrar::{
        ReplayOutcome, ReplayOutcomeStatus, ReplayReport,
    };
    use std::sync::{Mutex, MutexGuard, OnceLock};

    struct TestHomeGuard {
        _lock: MutexGuard<'static, ()>,
        temp: tempfile::TempDir,
        previous_home: Option<String>,
        previous_node_id: Option<String>,
    }

    impl TestHomeGuard {
        fn new() -> Self {
            static LOCK: OnceLock<Mutex<()>> = OnceLock::new();
            let lock = LOCK
                .get_or_init(|| Mutex::new(()))
                .lock()
                .unwrap_or_else(|poisoned| poisoned.into_inner());
            let temp = tempfile::tempdir().expect("temp home");
            let previous_home = std::env::var("HOME").ok();
            let previous_node_id = std::env::var("EASYNET_NODE_ID").ok();
            std::env::set_var("HOME", temp.path());
            std::env::remove_var("EASYNET_NODE_ID");
            Self {
                _lock: lock,
                temp,
                previous_home,
                previous_node_id,
            }
        }
    }

    impl Drop for TestHomeGuard {
        fn drop(&mut self) {
            match &self.previous_home {
                Some(value) => std::env::set_var("HOME", value),
                None => std::env::remove_var("HOME"),
            }
            match &self.previous_node_id {
                Some(value) => std::env::set_var("EASYNET_NODE_ID", value),
                None => std::env::remove_var("EASYNET_NODE_ID"),
            }
            let _ = self.temp.path();
        }
    }

    fn write_daemon_config(raw: &str) {
        let path = default_config_path();
        std::fs::create_dir_all(path.parent().expect("daemon config parent")).expect("mkdir");
        std::fs::write(path, raw).expect("write daemon config");
    }

    fn paired_credentials(
        realm: &str,
        node_id: &str,
    ) -> easynet_cli::daemon::persistence::config::Credentials {
        easynet_cli::daemon::persistence::config::Credentials {
            node_id: node_id.into(),
            credential_token: "token".into(),
            hub_endpoint: "https://hub.example:50443".into(),
            realm: realm.into(),
            deploy_signature: String::new(),
            hub_api_base: None,
            username: Some("alice".into()),
            user_id: Some("user-alice".into()),
            hub_pubkey_b64: None,
            hub_tls_ca_pem_b64: None,
            join_receipt_hash: None,
        }
    }

    fn federation_device_only_credentials(
        realm: &str,
        node_id: &str,
    ) -> easynet_cli::daemon::persistence::config::Credentials {
        let mut credentials = paired_credentials(realm, node_id);
        credentials.credential_token.clear();
        credentials.username = None;
        credentials.user_id = None;
        credentials.hub_pubkey_b64 = Some("hub-pubkey".into());
        credentials.join_receipt_hash = Some("sha256:test-join-receipt".into());
        credentials
    }

    #[test]
    fn device_replay_boot_policy_rejects_stale_rows() {
        let report = ReplayReport {
            stale: 1,
            errored: 1,
            ..ReplayReport::default()
        };

        assert!(device_ability_replay_fatal_message(&report)
            .unwrap()
            .contains("stale=1"));
    }

    #[test]
    fn device_replay_boot_policy_still_rejects_runtime_wiring_bug() {
        let report = ReplayReport {
            runtime_not_ready: true,
            ..ReplayReport::default()
        };

        assert!(device_ability_replay_fatal_message(&report)
            .unwrap()
            .contains("runtime_not_ready=true"));
    }

    #[test]
    fn device_replay_boot_policy_reports_outcome_details() {
        let report = ReplayReport {
            errored: 1,
            outcomes: vec![ReplayOutcome {
                public_name: "er.generate".to_string(),
                ability_ura: "easynet:///r/localhost/ability/device.old.er.generate".to_string(),
                install_id: "dev-old".to_string(),
                status: ReplayOutcomeStatus::Errored,
                detail: "explicit authority scope rejected".to_string(),
            }],
            ..ReplayReport::default()
        };

        let message = device_ability_replay_fatal_message(&report).unwrap();
        assert!(message.contains("errored=1"), "{message}");
        assert!(message.contains("er.generate"), "{message}");
        assert!(
            message.contains("explicit authority scope rejected"),
            "{message}"
        );
    }

    #[test]
    fn ready_discovery_uses_paired_credentials_node_id_not_env() {
        let _home = TestHomeGuard::new();
        std::env::set_var("EASYNET_NODE_ID", "stale-env-node");
        write_daemon_config(
            r#"[daemon]
mode = "device"
realm = "tenant-a"
hub_endpoint = "https://hub.example:50443"
"#,
        );
        config::save_credentials(&paired_credentials("tenant-a", "credential-node"))
            .expect("save paired credentials");

        let discovery = ready_runtime_discovery(vec![
            easynet_cli::daemon::control::discovery::flags::PAIRED_USER_RUNTIME_SIGNER.to_string(),
        ])
        .expect("ready discovery");

        assert_eq!(
            discovery.daemon_identity.node_id.as_deref(),
            Some("credential-node")
        );
        assert!(
            discovery.capability_flags.iter().any(|flag| {
                flag == easynet_cli::daemon::control::discovery::flags::PAIRED_USER_RUNTIME_SIGNER
            }),
            "device ready discovery must advertise signer readiness when boot proved it"
        );
    }

    #[test]
    fn ready_discovery_rejects_device_without_paired_user_signer_proof() {
        let _home = TestHomeGuard::new();
        write_daemon_config(
            r#"[daemon]
mode = "device"
realm = "tenant-a"
hub_endpoint = "https://hub.example:50443"
"#,
        );
        config::save_credentials(&paired_credentials("tenant-a", "credential-node"))
            .expect("save paired credentials");

        let error =
            ready_runtime_discovery(Vec::new()).expect_err("device Ready requires signer proof");

        assert!(
            error.to_string().contains(
                easynet_cli::daemon::control::discovery::flags::PAIRED_USER_RUNTIME_SIGNER
            ),
            "missing paired signer proof must be explicit: {error:#}"
        );
    }

    #[test]
    fn ready_discovery_accepts_device_only_credentials_without_paired_user_signer_proof() {
        let _home = TestHomeGuard::new();
        write_daemon_config(
            r#"[daemon]
mode = "device"
realm = "tenant-a"
hub_endpoint = "https://hub.example:50443"
"#,
        );
        config::save_credentials(&federation_device_only_credentials(
            "tenant-a",
            "credential-node",
        ))
        .expect("save device-only credentials");

        let discovery = ready_runtime_discovery(Vec::new())
            .expect("device-only Ready must not require User signer proof");

        assert_eq!(
            discovery.daemon_identity.node_id.as_deref(),
            Some("credential-node")
        );
        assert!(
            discovery.capability_flags.iter().all(|flag| {
                flag != easynet_cli::daemon::control::discovery::flags::PAIRED_USER_RUNTIME_SIGNER
            }),
            "device-only Ready must not advertise unproven paired-user signer readiness"
        );
    }

    #[test]
    fn ready_discovery_keeps_hub_independent_from_paired_user_signer_proof() {
        let _home = TestHomeGuard::new();
        write_daemon_config(
            r#"[daemon]
mode = "hub"
realm = "tenant-a"
"#,
        );

        let discovery = ready_runtime_discovery(Vec::new()).expect("hub ready discovery");

        assert_eq!(discovery.daemon_identity.mode, "hub");
        assert!(discovery.daemon_identity.node_id.is_none());
        assert!(
            discovery.capability_flags.is_empty(),
            "hub ready discovery must not invent device paired-user signer proof"
        );
    }

    #[test]
    fn local_runtime_invocation_identity_uses_paired_credentials_not_env() {
        let _home = TestHomeGuard::new();
        std::env::set_var("EASYNET_NODE_ID", "stale-env-node");
        write_daemon_config(
            r#"[daemon]
mode = "device"
realm = "tenant-a"
hub_endpoint = "https://hub.example:50443"
"#,
        );
        config::save_credentials(&paired_credentials("tenant-a", "credential-node"))
            .expect("save paired credentials");
        let config = DaemonConfig::load(&default_config_path()).expect("load config");

        let identity = local_runtime_invocation_identity(&config)
            .expect("runtime identity")
            .expect("device identity");

        assert_eq!(
            identity.local_device_ura(),
            "easynet:///r/tenant-a/device/credential-node"
        );
        assert!(!identity.local_device_ura().contains("stale-env-node"));
        assert!(!identity.local_device_ura().contains("/r/default/"));
    }

    #[test]
    fn local_runtime_invocation_identity_is_absent_for_hub_without_device_node() {
        let _home = TestHomeGuard::new();
        write_daemon_config(
            r#"[daemon]
mode = "hub"
realm = "tenant-a"
listen_tcp = "127.0.0.1:50443"
tls_cert_pem = "/tmp/cert.pem"
tls_key_pem = "/tmp/key.pem"
"#,
        );
        let config = DaemonConfig::load(&default_config_path()).expect("load config");

        let identity = local_runtime_invocation_identity(&config).expect("runtime identity");

        assert!(identity.is_none());
    }

    #[test]
    fn schedule_tick_invocation_uras_use_runtime_realm() {
        let identity =
            LocalRuntimeInvocationIdentity::new("tenant-a", NodeId::new("local-node")).unwrap();
        let target_node = NodeId::new("target-node");
        let schedule_id = ScheduleId::new("nightly");

        let (callee, subject) =
            schedule_tick_invocation_uras(&identity, &target_node, &schedule_id);

        assert_eq!(callee, "easynet:///r/tenant-a/device/target-node");
        assert_eq!(subject, "easynet:///r/tenant-a/resource/schedule.nightly");
        assert!(!callee.contains("/r/default/"));
        assert!(!subject.contains("/r/default/"));
    }

    #[test]
    fn ready_discovery_rejects_credentials_realm_mismatch() {
        let _home = TestHomeGuard::new();
        write_daemon_config(
            r#"[daemon]
mode = "device"
realm = "tenant-a"
hub_endpoint = "https://hub.example:50443"
"#,
        );
        config::save_credentials(&paired_credentials("tenant-b", "credential-node"))
            .expect("save paired credentials");

        let err =
            ready_runtime_discovery(Vec::new()).expect_err("realm mismatch must not publish ready");

        assert!(
            err.to_string().contains("does not match configured realm"),
            "error should name the ready identity split: {err:#}"
        );
    }
}
