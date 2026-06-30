// EasyNet CLI — daemon ability-registry assembly
// ==============================================
//
// The build_registry* family: catalogue construction, service
// wiring, plugin-runtime manager assembly, daemon keyring init.
// Split from agents/mod.rs (F-027 / T4.5); bodies are move-only.

use super::{
    a2a_bridge_ability, a2a_client_ability, ability_publish_ability, admin_status_ability,
    agent_lifecycle_ability, agent_list_ability, api_key_ability, browser_session_ability,
    chat_ability, chat_history_ability, context_ability, context_loaders, device_ops_ability,
    discover_ability, discuss_ability, file_transfer_ability, files, fs_ability, fs_edit_ability,
    http_request_ability, invocation_history_ability, list_resources_ability, loop_ability,
    mcp_bridge_ability, mcp_client_ability, media, media_abilities, meta_ability, mission_ability,
    network_health_ability, openai_compat_ability, orchestration_ability, pages,
    permission_ability, ping, plugin_lifecycle_ability, process_exec_ability, profiles,
    pty_attach_ability, pty_io_ability, pty_lifecycle_ability, schedule_ability, session_ability,
    shell_run_ability, skill_install_ability, skill_publish_ability, teach_ability, think_ability,
    voice_call_ability, PagesIdentity,
};
use crate::registry::agents::AgentRegistry;
use crate::runtime::ability_dispatch::AxonAbilityCatalog;
use crate::runtime::execution::discuss::DiscussService;
use crate::runtime::execution::loop_instance::LoopService;
use crate::runtime::execution::permission::PermissionService;
use crate::runtime::execution::pty::PtyService;
use crate::runtime::execution::schedule::ScheduleService;
use crate::runtime::execution::session::SessionService;
use std::sync::Arc;

/// Build a `AxonAbilityCatalog` populated with every v1 system
/// ability handler plus deterministic builtin plugin abilities.
/// Suitable for early-boot smoke tests + the `published_ability_names`
/// helper that the discovery publisher consumes. Tests get fresh empty
/// sub-services and an empty agent registry; the daemon bin calls
/// `build_registry_with_services` instead with its real Kernel handles
/// + loaded agents.
///
/// **No env-var or user plugin-store read**: builtin plugin shape is
/// determined by the compile-time feature set and current target
/// platform only. Installed plugin packages remain daemon-only state.
///
/// **Process-cached (CQRS read model).** This function is
/// deterministic by construction (fixed services, empty agent
/// registry, env-gate-free plugin mode), yet runtime reflection
/// surfaces — `published_abilities()`, MCP reflective refresh,
/// discovery hints, descriptor generation — were calling it per
/// tick, re-running a BOOT-grade construction each time (fresh
/// McpClientService → process-singleton noise, fresh plugin
/// registration → one leaked WebRTC runtime per call until that was
/// made lazy, 2026-06-10 fd exhaustion). Reads of a pure snapshot
/// must not rebuild the system: the snapshot is computed once per
/// process. Tests keep fresh instances (`cfg(test)`) because some
/// suites mutate the catalog via hot-register.
pub fn build_registry() -> Arc<AxonAbilityCatalog> {
    #[cfg(not(test))]
    {
        static SNAPSHOT: std::sync::OnceLock<Arc<AxonAbilityCatalog>> = std::sync::OnceLock::new();
        Arc::clone(SNAPSHOT.get_or_init(build_registry_uncached))
    }
    #[cfg(test)]
    build_registry_uncached()
}

fn build_registry_uncached() -> Arc<AxonAbilityCatalog> {
    build_registry_with_services_result_inner(
        RegistryBuildConfig::new(RegistryBuildServices::fresh(), &AgentRegistry::default()),
        PluginRegistryMode::BuiltinOnlyDeterministic,
    )
    .catalog
}

pub(super) fn build_system_registry() -> Arc<AxonAbilityCatalog> {
    #[cfg(not(test))]
    {
        static SNAPSHOT: std::sync::OnceLock<Arc<AxonAbilityCatalog>> = std::sync::OnceLock::new();
        Arc::clone(SNAPSHOT.get_or_init(build_system_registry_uncached))
    }
    #[cfg(test)]
    build_system_registry_uncached()
}

fn build_system_registry_uncached() -> Arc<AxonAbilityCatalog> {
    build_registry_with_services_result_inner(
        RegistryBuildConfig::new(RegistryBuildServices::fresh(), &AgentRegistry::default()),
        PluginRegistryMode::None,
    )
    .catalog
}

/// Build the standard system catalogue and write every registered
/// handler into the supplied Axon runtime. This is the compact test
/// and compatibility constructor for code paths that need the live
/// daemon execution surface but do not own Kernel sub-services.
pub fn build_registry_with_runtime(
    runtime: Arc<easynet_axon::invocation::LocalRuntime>,
) -> Arc<AxonAbilityCatalog> {
    let agents = AgentRegistry::default();
    let mut config = RegistryBuildConfig::new(RegistryBuildServices::fresh(), &agents);
    config.local_runtime = Some(runtime);
    build_registry_with_services(config)
}

/// Build a `AxonAbilityCatalog` with sub-service handles wired
/// in. The daemon bin calls this with the Kernel's actual handles
/// at boot; tests construct a fresh registry per case.
///
/// `agents` and `loaders` feed hosted-agent dynamic replay:
/// `<agent>.chat`, `<agent>.discover`, `<agent>.invoke`, and
/// executable TOML abilities are not static catalogue rows. They are
/// installed through `HotAgentRegistrar` after the catalogue is wrapped
/// in `Arc`, so boot replay, `agent.start`, `agent.refresh`, and
/// `agent.stop` share the same control-plane/runtime transaction.
///
/// `hot_agent_registrar_cell` is a late-wired
/// `OnceLock<Arc<HotAgentRegistrar>>`. The lifecycle handlers read
/// through this cell at dispatch time. If a call lands before the
/// registrar is available, durable agent state still wins and the
/// handler emits an op_event with runtime sync skipped.
/// Result of constructing the daemon's local ability registry.
///
/// What this is NOT: a runtime executor. `catalog` owns handler metadata and
/// registration side tables; `plugin_runtime_manager` owns plugin package/load
/// state so boot-time services can derive wire/surface projections from the
/// same snapshot that registered plugin abilities.
pub struct BuiltAbilityRegistry {
    pub catalog: Arc<AxonAbilityCatalog>,
    pub plugin_runtime_manager: Arc<crate::runtime::plugin_host::PluginRuntimeManager>,
    /// Late-wired device-ability registrar cell. Populated during the
    /// build with a pending registrar; boot calls `set_runtime` on it
    /// (and may `replay_from_store`) once the `LocalRuntime` exists, so
    /// `ability.deploy` can run its install transaction. Mirrors
    /// `hot_agent_registrar_cell` but for device-owned deploys.
    pub device_registrar_cell:
        Arc<crate::runtime::agents::device_ops_ability::SharedDeviceRegistrarCell>,
}

#[derive(Clone)]
pub struct RegistrySharedStores {
    pub hub_published_abilities:
        Arc<crate::services::hub_published_ability_store::HubPublishedAbilityStore>,
}

impl RegistrySharedStores {
    #[must_use]
    pub fn new(
        hub_published_abilities: Arc<
            crate::services::hub_published_ability_store::HubPublishedAbilityStore,
        >,
    ) -> Self {
        Self {
            hub_published_abilities,
        }
    }
}

impl Default for RegistrySharedStores {
    fn default() -> Self {
        Self::new(crate::services::hub_published_ability_store::HubPublishedAbilityStore::new())
    }
}

#[derive(Clone)]
pub struct RegistryBuildServices {
    pub sessions: Arc<SessionService>,
    pub perms: Arc<PermissionService>,
    pub discuss: Arc<DiscussService>,
    pub schedule: Arc<ScheduleService>,
    pub loop_svc: Arc<LoopService>,
    pub discover_federation_resolver: discover_ability::SharedDiscoverFederationResolver,
}

impl RegistryBuildServices {
    #[must_use]
    pub fn new(
        sessions: Arc<SessionService>,
        perms: Arc<PermissionService>,
        discuss: Arc<DiscussService>,
        schedule: Arc<ScheduleService>,
        loop_svc: Arc<LoopService>,
    ) -> Self {
        Self {
            sessions,
            perms,
            discuss,
            schedule,
            loop_svc,
            discover_federation_resolver: Arc::new(
                discover_ability::BridgeDiscoverFederationResolver,
            ),
        }
    }

    #[must_use]
    pub fn fresh() -> Self {
        Self::new(
            Arc::new(SessionService::new()),
            Arc::new(PermissionService::new()),
            Arc::new(DiscussService::new()),
            Arc::new(ScheduleService::new()),
            Arc::new(LoopService::new()),
        )
    }

    #[must_use]
    pub fn with_discover_federation_resolver(
        mut self,
        resolver: discover_ability::SharedDiscoverFederationResolver,
    ) -> Self {
        self.discover_federation_resolver = resolver;
        self
    }
}

pub struct RegistryBuildConfig<'a> {
    pub services: RegistryBuildServices,
    pub invocation_ledger: Option<Arc<easynet_axon::invocation::InvocationLedger>>,
    pub agents: &'a AgentRegistry,
    pub loaders: Arc<Vec<Arc<dyn chat_ability::ContextLoader>>>,
    pub pages_identity: PagesIdentity,
    pub local_runtime: Option<Arc<easynet_axon::invocation::LocalRuntime>>,
    pub authority_context: Option<crate::runtime::ability_dispatch::AbilityAuthorityContext>,
    pub hot_agent_registrar_cell: Arc<agent_lifecycle_ability::SharedHotRegistrarCell>,
    pub shared_stores: RegistrySharedStores,
}

impl<'a> RegistryBuildConfig<'a> {
    #[must_use]
    pub fn new(services: RegistryBuildServices, agents: &'a AgentRegistry) -> Self {
        Self {
            services,
            invocation_ledger: None,
            agents,
            loaders: Arc::new(Vec::new()),
            pages_identity: PagesIdentity::default(),
            local_runtime: None,
            authority_context: None,
            hot_agent_registrar_cell: Arc::new(
                agent_lifecycle_ability::SharedHotRegistrarCell::new(),
            ),
            shared_stores: RegistrySharedStores::default(),
        }
    }
}

pub struct RegistryDaemonBuildConfig {
    pub services: RegistryBuildServices,
    pub invocation_ledger: Option<Arc<easynet_axon::invocation::InvocationLedger>>,
    pub loaders: Option<Arc<Vec<Arc<dyn chat_ability::ContextLoader>>>>,
    pub pages_identity: PagesIdentity,
    pub local_runtime: Option<Arc<easynet_axon::invocation::LocalRuntime>>,
    pub authority_context: Option<crate::runtime::ability_dispatch::AbilityAuthorityContext>,
    pub hot_agent_registrar_cell: Arc<agent_lifecycle_ability::SharedHotRegistrarCell>,
    pub shared_stores: RegistrySharedStores,
}

impl RegistryDaemonBuildConfig {
    #[must_use]
    pub fn new(services: RegistryBuildServices) -> Self {
        Self {
            services,
            invocation_ledger: None,
            loaders: None,
            pages_identity: PagesIdentity::default(),
            local_runtime: None,
            authority_context: None,
            hot_agent_registrar_cell: Arc::new(
                agent_lifecycle_ability::SharedHotRegistrarCell::new(),
            ),
            shared_stores: RegistrySharedStores::default(),
        }
    }
}

pub fn build_registry_with_services_result(
    config: RegistryBuildConfig<'_>,
) -> BuiltAbilityRegistry {
    build_registry_with_services_result_inner(config, PluginRegistryMode::DefaultDaemon)
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum PluginRegistryMode {
    /// Do not include package-owned plugin abilities. Used for system descriptor
    /// generation, where plugin descriptors are rendered separately from the
    /// plugin package index.
    None,
    /// Include compile-time builtin plugin packages without reading `$HOME` or
    /// env-disable gates. Used by `published_abilities()` and smoke tests.
    BuiltinOnlyDeterministic,
    /// Load builtin plus installed plugin packages using daemon boot policy.
    DefaultDaemon,
}

fn build_plugin_runtime_manager(
    mode: PluginRegistryMode,
) -> Arc<crate::runtime::plugin_host::PluginRuntimeManager> {
    match mode {
        PluginRegistryMode::None => Arc::new(
            crate::runtime::plugin_host::PluginRuntimeManager::from_state(
                crate::runtime::plugin_host::PluginRuntimeState::from_index(
                    crate::runtime::plugin_host::PluginPackageIndex::default(),
                ),
            ),
        ),
        PluginRegistryMode::BuiltinOnlyDeterministic => {
            let state = match crate::runtime::plugin_host::PluginPackageIndex::builtin() {
                Ok(index) => {
                    crate::runtime::plugin_host::PluginRuntimeState::from_index_with_planner(
                        index,
                        crate::runtime::plugin_host::PluginLoadPlanner::current_without_env_gates(),
                    )
                }
                Err(err) => {
                    let error = err.to_string();
                    crate::op_event!(
                        component = plugin_host,
                        kind = deterministic_builtin_index_failed,
                        error = error.as_str(),
                        message = "deterministic builtin plugin index failed; daemon core abilities remain registered",
                    );
                    crate::runtime::plugin_host::PluginRuntimeState::from_index(
                        crate::runtime::plugin_host::PluginPackageIndex::default(),
                    )
                }
            };
            Arc::new(crate::runtime::plugin_host::PluginRuntimeManager::from_state(state))
        }
        PluginRegistryMode::DefaultDaemon => {
            Arc::new(crate::runtime::plugin_host::PluginRuntimeManager::new())
        }
    }
}

fn build_registry_with_services_result_inner(
    config: RegistryBuildConfig<'_>,
    plugin_registry_mode: PluginRegistryMode,
) -> BuiltAbilityRegistry {
    let RegistryBuildConfig {
        services,
        invocation_ledger,
        agents,
        loaders,
        pages_identity,
        local_runtime,
        authority_context,
        hot_agent_registrar_cell,
        shared_stores,
    } = config;
    let RegistryBuildServices {
        sessions,
        perms,
        discuss,
        schedule,
        loop_svc,
        discover_federation_resolver,
    } = services;

    let authority_context = authority_context.unwrap_or_default();
    let runtime = local_runtime.unwrap_or_else(easynet_axon::invocation::LocalRuntime::new);
    let mut reg = AxonAbilityCatalog::new_with_runtime_and_authority_context(
        Arc::clone(&runtime),
        authority_context,
    );
    ping::register(&mut reg);
    network_health_ability::register(&mut reg);
    // AXIOM §"Tier 2.5" Baseline Locomotion Profile, filesystem
    // half. Three stateless handlers (fs.read / fs.write /
    // fs.list) — every host-embodied agent claiming
    // `baseline-locomotion-v1` MUST expose them.
    fs_ability::register(&mut reg);
    // AXIOM §"Tier 2.5" Baseline Locomotion — surgical text
    // edit. Sibling of fs.read / fs.write; uses the SAME
    // atomic-write path (tempfile + fdatasync + rename) so
    // the crash-resilience story is uniform.
    fs_edit_ability::register(&mut reg);
    // AXIOM §"Tier 2.5" Baseline Locomotion Profile —
    // structured execution. `process.exec` shares the
    // destructive command list and process-execution
    // hardening (tempfile-backed output, tree-kill on
    // timeout, env defaults) with `shell.run` via the
    // `support::shellguard` subsystem.
    process_exec_ability::register(&mut reg);
    // AXIOM §"Tier 2.5" Baseline Locomotion Profile —
    // shell-interpreted execution. `shell.run` is the only
    // member of the profile that takes a bash command STRING;
    // the 8-stage shellguard pipeline (ast → security →
    // permissions → pathconstraints → readonly → destructive)
    // gates every dispatch.
    shell_run_ability::register(&mut reg);
    // AXIOM §"Tier 2.5" Baseline Locomotion — HTTP client.
    // Last member of the seven-ability profile; first-class
    // surface for outbound network so receivers can audit
    // every external call uniformly instead of going through
    // a shell.run-wrapped curl.
    http_request_ability::register(&mut reg);
    // invocation.history.* / invocation.trace.* — read-only audit surfaces over
    // the Axon invocation ledger. The ledger is written by the
    // gRPC invocation service; these handlers only expose persisted
    // URA-complete records for UI/backend tracing.
    invocation_history_ability::register(&mut reg, invocation_ledger);
    // AXIOM §"Tier 2.5" Baseline Locomotion — PTY data-plane and
    // its lifecycle control-plane. terminal.create /
    // terminal.close manage the session catalog;
    // terminal.attach pumps stdin/stdout bidirectionally
    // over InvokeBidi for interactive workloads (REPLs, editors,
    // text-mode TUI). All three share one process-wide PtyService
    // (single Arc, lazy init): a session created by …_create
    // is the same session …_attach pumps and …_close tears down,
    // so the three abilities cohere even though they're three
    // separate handlers.
    let pty = Arc::new(PtyService::new());
    let pty_io = pty_io_ability::PtyIoService::new();
    pty_lifecycle_ability::register(&mut reg, Arc::clone(&pty), Some(pty_io.clone()));
    pty_attach_ability::register(&mut reg, Arc::clone(&pty));
    // terminal.input / _read / _resize — unary-RPC data
    // plane. The backend's PTYDriver invokes these for the
    // production HTTP-session terminal flow before the WebSocket
    // bidi optimisation kicks in. Sharing the PtyService Arc with
    // the lifecycle + attach handlers means a session created by
    // …_create is reachable through all three surfaces (unary,
    // bidi, lifecycle) — operators choose one mode per session.
    pty_io_ability::register(&mut reg, pty, pty_io);
    // fs.transfer — bidi chunked file upload/download.
    // Pairs with the EasyNet backend's /api/v1/files/{upload,
    // download} routes. No shared service state needed; the
    // handler opens its own per-session FS handle on each
    // OpenBidi.
    file_transfer_ability::register(&mut reg);
    // RFC-005 v3.2 — physical-channel media abilities (A1–A8)
    // plus meta.list_resources (A9). `media_abilities` owns the
    // shared metadata and only registers still-unwired stubs; real
    // modules own their names directly. This keeps each ability +
    // call-mode slot single-owner and avoids precedence-based
    // replacement semantics.
    media_abilities::register(&mut reg);
    media::camera_snapshot::register(&mut reg);
    media::screen_snapshot::register(&mut reg);
    media::mic_subscribe::register(&mut reg);
    list_resources_ability::register(&mut reg);
    // agent.start / agent.stop / agent.refresh —
    // Invoke-side surface of `easynet agent add/remove/refresh`. LLM sub-agents are registry
    // rows (not resident processes), so start ≡ insert into
    // ~/.easynet/agents.json and return the canonical URA;
    // stop ≡ delete the row (idempotent).
    //
    // The lifecycle handler receives the shared dynamic registrar cell.
    // `agent.start` / `agent.refresh` replay the hosted agent through
    // HotAgentRegistrar; `agent.stop` removes the same dynamic catalogue
    // rows. There is no static hosted-agent fallback path.
    agent_lifecycle_ability::register(&mut reg, Arc::clone(&hot_agent_registrar_cell));
    // device-hosted node/ability operations (list_nodes, describe_node,
    // remove_node, deploy_ability, uninstall_ability). These are the
    // canonical ability surfaces backing the CLI's device + ability
    // subcommands.
    //
    // Construct the device-ability registrar pending (runtime attached
    // by boot) and stash it in the cell `ability.deploy`'s handler
    // closes over — the install transaction reads it. Same late-wiring
    // pattern as `hot_agent_registrar_cell`.
    let device_registrar_cell: Arc<
        crate::runtime::agents::device_ops_ability::SharedDeviceRegistrarCell,
    > = Arc::new(std::sync::OnceLock::new());
    if device_registrar_cell
        .set(
            crate::runtime::agents::device_ability_registrar::DeviceAbilityRegistrar::new_pending(),
        )
        .is_err()
    {
        panic!("device registrar cell must be written exactly once during registry build");
    }
    device_ops_ability::register(&mut reg, Arc::clone(&device_registrar_cell));
    // browser.* — RFC-012 §RemoteWebSurface; v0 mock
    // handlers per RFC-013 plan. capture_viewport is a streaming
    // verb; the other three are unary RPC.
    browser_session_ability::register(&mut reg);
    // voice.* call signaling abilities — `easynet call …`
    // subcommand surface routes through these via the same
    // ability-only invocation path every other CLI surface uses.
    voice_call_ability::register(&mut reg);
    // Stateful device plugins. Package discovery, boot-time load decisions, and
    // handler registration stay separate so install/remove/update state cannot
    // leak into runtime call semantics.
    let plugin_runtime_manager = build_plugin_runtime_manager(plugin_registry_mode);
    match plugin_registry_mode {
        PluginRegistryMode::None => {}
        PluginRegistryMode::BuiltinOnlyDeterministic => {
            if let Err(err) = plugin_runtime_manager.register_current_plugins(&mut reg) {
                let error = err.to_string();
                crate::op_event!(
                    component = plugin_host,
                    kind = deterministic_builtin_registration_failed,
                    error = error.as_str(),
                    message = "deterministic builtin plugin registration failed; daemon core abilities remain registered",
                );
            }
        }
        PluginRegistryMode::DefaultDaemon => {
            if let Err(err) = plugin_runtime_manager.register_default_plugins(&mut reg) {
                let error = err.to_string();
                crate::op_event!(
                    component = plugin_host,
                    kind = default_registration_failed,
                    error = error.as_str(),
                    message = "default plugin host registration failed; daemon core abilities remain registered",
                );
            }
        }
    }
    session_ability::register(&mut reg, sessions);
    // chat.history.{list,get} — read-only access to the per-agent
    // chat transcripts the chat ability already persists on disk.
    chat_history_ability::register(&mut reg);
    // context.* — device-global clipboard history, mapped project
    // folders, and favorites (the Frontend Context page surface).
    context_ability::register(&mut reg);
    permission_ability::register(&mut reg, perms);
    discuss_ability::register(&mut reg, Arc::clone(&discuss));
    schedule_ability::register(&mut reg, schedule);
    loop_ability::register(&mut reg, loop_svc);
    // The shared OnceLock consumed by every ability that needs
    // to resolve against the live catalogue post-boot:
    // mcp.bridge.call_tool, a2a.bridge.send_task, meta.list_abilities,
    // and per-agent <agent>.invoke. Created before agent ability
    // registration so every handler can close over the same live
    // registry handle. Set once after `Arc::new(reg)` below.
    let local_registry_handle: Arc<std::sync::OnceLock<Arc<AxonAbilityCatalog>>> =
        Arc::new(std::sync::OnceLock::new());
    plugin_lifecycle_ability::register(
        &mut reg,
        Arc::clone(&local_registry_handle),
        Arc::clone(&plugin_runtime_manager),
    );

    // Construct the hot hosted-agent registrar HERE so it can close over
    // the exact `loaders` Arc + `local_registry_handle` OnceLock used by
    // invoke/discover recursion. Boot replay below and every post-boot
    // lifecycle mutation use this same object; hosted-agent abilities are
    // never written directly into the static catalogue.
    //
    // We stash the constructed registrar into the shared
    // `hot_agent_registrar_cell` immediately so the
    // `agent.start` / `.stop` handler closures resolve to a
    // populated cell as soon as registration completes.
    {
        let hot_registrar =
            crate::runtime::axon_bridge::hot_agent_registrar::HotAgentRegistrar::new_pending(
                Arc::clone(&loaders),
                Arc::clone(&local_registry_handle),
                Arc::clone(&discover_federation_resolver),
            );
        hot_registrar.set_runtime(Arc::clone(&runtime));
        // Mirror ProcessSingleton::once()::set's diagnostic: a
        // second writer on this `OnceLock` is a boot-wiring bug
        // (`build_registry` is supposed to run exactly once per
        // process). Leaving it as `let _ = …` would hide that.
        if hot_agent_registrar_cell.set(hot_registrar).is_err() {
            crate::op_event!(
                component = agents_boot,
                kind = second_writer_rejected,
                level = "warn",
                cell = "hot_agent_registrar_cell",
            );
        }
    }

    // mission.discuss_round — sub-turn orchestration ability.
    // The CLI `easynet mission discuss …` and any EAL caller drive
    // multi-agent discussions through this name. Shares the
    // DiscussService with the discuss.* triple (same room state)
    // and consumes the shared registry handle so per-cycle
    // <agent>.chat invocations stay in-process — going through
    // IPC would deadlock the daemon's accept loop.
    orchestration_ability::register(
        &mut reg,
        Arc::clone(&discuss),
        Arc::clone(&local_registry_handle),
    );
    // Hosted-agent abilities are deliberately NOT registered into the static
    // boot maps. Static maps cannot be removed through `agent.stop` after the
    // catalog is wrapped in `Arc`, which previously left catalog/control-plane
    // residue after the runtime row was removed. The current agents are
    // replayed through `HotAgentRegistrar` after `local_registry_handle` is set
    // below, so boot-time and post-boot lifecycle use one dynamic transaction.
    // RFC-006-B v0.6 — Pages reference system. Management verbs are
    // registered directly into the daemon-hosted Axon LocalRuntime;
    // per-project fetch/API verbs are hot-registered at publish or
    // restore time. The default user identity is sourced from the
    // daemon's published self-identity; for the MVP we accept the env
    // var `EASYNET_PAGES_USER` so the demo and tests can pin a
    // deterministic user without reaching into the keyring resolver.
    // Listener port comes from `EASYNET_PAGES_PORT` (default 8787).
    {
        // User-rooted ability families (`<user>.api_key.*`,
        // `<user>.pages.*`, `<user>.files.*`). Identity sourced
        // explicitly from the `pages_identity` argument — no
        // env-var read here.
        //
        // M5 of the system-namespace migration banned the `legacy self alias`
        // placeholder; an unpaired daemon (`pages_identity.user`
        // is None) skips the user-rooted family entirely. The
        // ability surface returns once pairing completes and the
        // supervisor rebuilds the registry with a populated
        // identity.
        if let Some(user) = pages_identity.user.clone() {
            let realm = pages_identity
                .realm
                .clone()
                .unwrap_or_else(|| crate::ura::REALM_EASYNET.to_string());
            let listener_port = pages_identity.listener_port.unwrap_or(8787);
            let pages_realm = realm.clone();
            pages::register(
                &mut reg,
                pages::PagesConfig {
                    user: user.clone(),
                    realm,
                    listener_port,
                },
                Arc::clone(&local_registry_handle),
            );
            // Files reference system: content-addressed blob store
            // serving `/v1/files{,/<id>/content}` + chat-multimodal
            // URA dereferences. Same `<user>` identity as pages so
            // one user owns both surface families.
            files::register(
                &mut reg,
                files::FilesConfig {
                    user: user.clone(),
                    realm: pages_realm,
                },
            );
            // RFC-006-C v0.1 — API key abilities. Register under the
            // same `user` identity pages used so a single user owns
            // both surface families on this daemon.
            api_key_ability::register(&mut reg, &user);
        }
        // RFC-006-C v0.1 — device-local OpenAI shim. Device-owned,
        // no `<user>` slot — registers regardless of pairing state.
        openai_compat_ability::set_dispatch_handle(Arc::clone(&local_registry_handle));
        openai_compat_ability::set_identity(pages_identity.clone());
        openai_compat_ability::register(&mut reg);
    }
    skill_install_ability::register(&mut reg);
    // ability.publish + ability.unpublish — root meta-abilities. See
    // module preamble for trust model and on-disk layout. Stateless
    // handlers (no captured registry handle), so order vs other
    // registrations is irrelevant.
    ability_publish_ability::register(&mut reg);
    // skill.publish + skill.unpublish + skill.list/file operations —
    // sibling of ability_publish_ability. Same statelessness; same
    // order independence.
    skill_publish_ability::register(&mut reg);
    // mission.think — long-running worker+judge orchestration.
    // Consumes the shared catalogue handle so per-cycle
    // <agent>.chat invocations stay in-process; same rationale as
    // mission.discuss_round (going back through the IPC client
    // would deadlock the daemon's accept loop).
    think_ability::register(&mut reg, Arc::clone(&local_registry_handle));
    // mcp.bridge.{list_tools, call_tool} — MCP edge adapter.
    //
    // list_tools projects local AbilityDescriptors to the MCP
    // tools/list shape. Provider runs on every call so a daemon
    // restart that picks up a freshly-canonicalised URA (or a
    // future hot-add of a hosted Agent) is reflected without re-
    // registering the handler. `load_host_descriptors` is the same
    // recipe the MCP stdio server uses, so an external MCP client
    // and an in-process Invoke caller see one catalog.
    //
    // call_tool invokes the named local ability in-process. The
    // shared OnceLock (declared before the dynamic hosted-agent
    // registrar is constructed) is the chicken-and-egg fix:
    // every consumer needs an `Arc` to the registry being built,
    // but the registry isn't yet wrapped in an `Arc` at
    // registration time. Set the lock once after `Arc::new(reg)`
    // completes; every closure's `get()` returns the populated
    // handle.
    mcp_bridge_ability::register(
        &mut reg,
        profiles::load_host_descriptors,
        Arc::clone(&local_registry_handle),
    );
    // mission.run — single ability surface for EAL execution. The
    // canonical orchestration entry point referenced by AGENTS.md
    // ("cross-agent calls go through the mission runtime; there is
    // no second path"). Without this an LLM inside an agent had to
    // shell out to `easynet mission run`, which depended on shell
    // access and bypassed daemon runtime invariants.
    //
    // Registered BEFORE meta.list_abilities so the live-registry
    // merge inside that handler picks up the mission entry point
    // — otherwise the LLM's discovery flow would not see this
    // ability and would fall back to fabricating answers.
    mission_ability::register(&mut reg);
    // The device-owned aggregate `discover` owns the top-level view and
    // reloads `agents.json` per call, so it never chooses a random first
    // agent as a synthetic self. Per-agent `<agent>.discover` /
    // `<agent>.invoke` are hosted-agent lifecycle rows; they are replayed
    // through HotAgentRegistrar after `Arc::new(reg)` below.
    discover_ability::register_device_aggregate_with_resolver(
        &mut reg,
        || crate::registry::agents::load_agents().unwrap_or_default(),
        Arc::clone(&local_registry_handle),
        Arc::clone(&discover_federation_resolver),
    );

    // RFC-002 §3.3: register `device.keyring.*` for the daemon's
    // own self-bundle, scoped under the literal owner `device`.
    // The daemon publishes its 10 keyring abilities under this
    // namespace so any local agent can call them through the
    // standard dispatch path. Auto-init the on-disk store when
    // absent — passphrase comes from EASYNET_KEYRING_PASS or
    // falls back to a fixed deterministic local pass for the
    // local-fast default. Failures here MUST NOT block daemon
    // boot; we log the error and skip keyring registration. The
    // resolver layer copes with absence by treating every URA
    // as Unknown.
    //
    // The legacy owner string was `legacy self alias` — a "this device"
    // alias. v4.1.5 onward names the actor explicitly: keyring
    // belongs to the device, so the owner is `device`. The
    // catalogue now lists these as `device.keyring.<verb>`,
    // matching the URA `callee = device/<id>` that already
    // covers them.
    //
    // EASYNET_KEYRING_DISABLE=1 skips auto-init entirely. Tests
    // that don't want side effects on the user's real keyring
    // file set this; production daemons leave it unset.
    if std::env::var("EASYNET_KEYRING_DISABLE").is_err() {
        match init_keyring_for_daemon() {
            Ok(handle) => {
                crate::runtime::keyring::abilities::register_for_owner(&mut reg, "device", handle);
            }
            Err(e) => {
                let err_msg = format!("{e}");
                crate::op_event!(
                    component = device_keyring,
                    kind = auto_init_failed,
                    level = "warn",
                    error = err_msg,
                );
            }
        }
    }
    // meta.{describe,list_abilities} — Agent self-introspection on
    // the same descriptor catalogue PLUS the live registry. describe
    // is the lightweight identity+summary surface; list_abilities
    // merges the static profile catalogue with everything currently
    // registered in `reg` (mission.run, per-agent <agent>.<verb>
    // verbs, hot-reloaded TOMLs) so a discover-then-invoke flow sees
    // every callable name. Visibility filtering per §1.6 happens at
    // the admission gate, not here.
    meta_ability::register(
        &mut reg,
        profiles::load_host_descriptors,
        Arc::clone(&local_registry_handle),
        pages_identity.user.clone(),
        Arc::clone(&shared_stores.hub_published_abilities),
    );
    // a2a.bridge.list_skills — same edge-adapter pattern as the MCP
    // bridge above, but for the A2A agent-card surface. Closes over
    // a clone of the AgentRegistry passed in here. v1 has no
    // hot-reload of `agents.json`, so the snapshot stays accurate
    // for the daemon's lifetime; the closure is still cheap to call.
    let agents_for_a2a = agents.clone();
    a2a_bridge_ability::register(
        &mut reg,
        move || crate::registry::agents::load_agents().unwrap_or_else(|_| agents_for_a2a.clone()),
        Arc::clone(&local_registry_handle),
    );
    // a2a.client.send_task — outbound A2A. The handler dials the
    // daemon-hosted `federation.forward_invoke` Axon ability; tests
    // run without a daemon socket and verify it returns ok:false
    // instead of panicking.
    a2a_client_ability::register(&mut reg);
    // mcp.client.{list,call} — outbound MCP. Boots an
    // McpClientService from ~/.easynet/mcp_clients.json (missing
    // file → empty service, no upstreams). Each upstream MCP
    // server is spawned lazily on first call; subsequent calls
    // reuse the live connection. Parse errors at boot bubble up
    // because a malformed file is an operator typo, not a "no
    // upstreams" condition.
    let mcp_clients_path =
        crate::runtime::execution::mcp_client::McpClientService::default_config_path();
    let mcp_client_svc =
        match crate::runtime::execution::mcp_client::McpClientService::from_path(&mcp_clients_path)
        {
            Ok(svc) => Arc::new(svc),
            Err(e) => {
                let path_display = format!("{}", mcp_clients_path.display());
                let err_msg = format!("{e}");
                crate::op_event!(
                    component = mcp_client,
                    kind = config_load_failed,
                    level = "warn",
                    path = path_display,
                    error = err_msg,
                    fallback = "empty_service",
                );
                Arc::new(crate::runtime::execution::mcp_client::McpClientService::new())
            }
        };
    mcp_client_ability::register(&mut reg, mcp_client_svc.clone());

    // Install the same `Arc<McpClientService>` as the process-wide
    // handle used by `[exec] kind="mcp"` ability dispatch. Before this
    // line `mcp_executor::run_mcp_exec` would return a typed error;
    // after this line every MCP surface in the daemon — outbound
    // `mcp.client.*`, reflective registry below, and exec —
    // shares one connection pool, one config snapshot, one `next_id`
    // sequence per upstream. No silent divergence between surfaces.
    crate::runtime::agents::mcp_executor::set_process_client(mcp_client_svc.clone());

    // MCP reflection policy. Direct MCP client abilities are already
    // registered above; this section only decides whether upstream
    // tools are projected as first-class EasyNet abilities.
    //
    // Default is lazy: `easynet start` must return a ready daemon
    // after the bounded local registry build, while external MCP
    // servers are discovered by a background supervisor against the
    // dynamic registry overlay. Operators that need legacy blocking
    // behaviour can set EASYNET_MCP_REFLECTION=eager; production
    // operators can set `off` and rely solely on
    // `mcp.client.{list,call}`.
    //
    // **Identity invariant.** The owner URA for reflected abilities
    // is the mcp-profile agent under the daemon's paired user. An
    // unpaired daemon has no `pages_identity.user` and we therefore
    // SKIP reflective registration entirely — we will not fabricate
    // a synthetic `user_id = "device"` to mint an agent URA, because
    // per AGENT_IDENTITY.md §2 ("identity, not locator") that would
    // forge an agent identity that no `easynet:///r/.../user/...`
    // backs. The outbound `mcp.client.*` family remains
    // available so operators can still reach upstream tools through
    // the explicit-server-name shape; only the bare-name projection
    // is gated on a paired user.
    // Resolve the post-Arc reflection plan in one place. The plan
    // enum (`PostArcReflection`) encodes the four terminal outcomes
    // of `(mode, paired?)`: skip, attach-after-eager, spawn-lazy. No
    // exclusive-pair-of-Option threading across the Arc::new(reg)
    // boundary — every branch is a named variant carrying exactly
    // the data the apply step needs.
    let reflection_realm = pages_identity
        .realm
        .clone()
        .unwrap_or_else(|| easynet_axon::ura::REALM_EASYNET.to_string());
    let reflection_plan = crate::runtime::agents::mcp_reflective_registry::PostArcReflection::plan(
        crate::runtime::agents::mcp_reflective_registry::McpReflectionMode::from_env(),
        pages_identity.user.as_deref(),
        &reflection_realm,
        &mcp_client_svc,
        &mut reg,
    );
    // agent.list — operational view of registered LLM
    // sub-agents. Cheap-row projection (name, runtime, model, label);
    // for the protocol agent-card view see a2a.bridge.list_skills.
    let agents_for_device_view = agents.clone();
    agent_list_ability::register(&mut reg, move || {
        crate::registry::agents::load_agents().unwrap_or_else(|_| agents_for_device_view.clone())
    });
    // meta.teach / meta.acquire / meta.forget — GET route B
    // (seven-axes T3.3): owner-conferred capability transfer.
    // No grant = allow_transferred_code=false, the InstallPolicy
    // default — meta.acquire refuses.
    teach_ability::register(&mut reg, Arc::clone(&hot_agent_registrar_cell));
    // admin.status — operator-facing component snapshot. The
    // ability-count provider reads through the same OnceLock the
    // bridge handlers use, so the count is accurate at call time
    // (post-Arc-wrap; pre-set the OnceLock returns 0 which only
    // happens during the brief window before `.set()` below).
    {
        let handle_for_admin = Arc::clone(&local_registry_handle);
        admin_status_ability::register(&mut reg, move || {
            handle_for_admin
                .get()
                .map(|r| r.list_abilities().len())
                .unwrap_or(0)
        });
    }
    let arc = Arc::new(reg);
    // Populate the shared OnceLock now that the registry is wrapped.
    // Both mcp.bridge.call_tool and a2a.bridge.send_task read through
    // it to dispatch into other local abilities; until this line runs
    // they each return isError("not initialised") on every call.
    //
    // Mirror ProcessSingleton::once()::set diagnostic — a second
    // writer here means `build_registry` ran twice in one process
    // (a boot-wiring bug), which would silently pin handlers to the
    // first registry and orphan the second.
    if local_registry_handle.set(Arc::clone(&arc)).is_err() {
        crate::op_event!(
            component = agents_boot,
            kind = second_writer_rejected,
            level = "warn",
            cell = "local_registry_handle",
        );
    }

    if let Some(hot_registrar) = hot_agent_registrar_cell.get().cloned() {
        for (agent_name, entry) in agents.agents.iter() {
            let outcome = crate::support::async_bridge::run_blocking(
                hot_registrar.register_agent(agent_name, entry),
                crate::support::async_bridge::NoRuntimeFallback::BuildCurrentThreadTokio,
            );
            if outcome.runtime_not_ready || outcome.catalog_not_ready || outcome.failed > 0 {
                let failed = outcome.failed.to_string();
                let runtime_not_ready = outcome.runtime_not_ready.to_string();
                let catalog_not_ready = outcome.catalog_not_ready.to_string();
                crate::op_event!(
                    component = agents_boot,
                    kind = hosted_agent_dynamic_replay_failed,
                    level = "warn",
                    agent = agent_name.as_str(),
                    failed = failed.as_str(),
                    runtime_not_ready = runtime_not_ready.as_str(),
                    catalog_not_ready = catalog_not_ready.as_str(),
                    message = "hosted-agent ability replay did not fully register",
                );
            }
        }

        // Forget tombstones converge here, after the learner runtimes are
        // replayed above: a forget that degraded on the "runtime not yet
        // wired" path left its row in Forgetting, occupying the slot forever
        // until an explicit retry. Re-drive those rows now that convergence is
        // possible so the slot is freed and the descriptor can be re-acquired.
        match teach_ability::recover_forget_transactions(Some(&hot_agent_registrar_cell)) {
            Ok(recovered) if recovered > 0 => {
                let recovered = recovered.to_string();
                crate::op_event!(
                    component = agents_boot,
                    kind = forget_tombstone_recovery_completed,
                    recovered = recovered.as_str(),
                    message = "converged stuck forget tombstones after hosted-agent replay",
                );
            }
            Ok(_) => {}
            Err(err) => {
                let err_msg = format!("{err}");
                crate::op_event!(
                    component = agents_boot,
                    kind = forget_tombstone_recovery_failed,
                    level = "warn",
                    error = err_msg,
                    message = "forget tombstone recovery sweep did not complete",
                );
            }
        }
    }

    // Hot-reload sinks. Wired after Arc::new(reg) so each sink can
    // hold a `Weak<AxonAbilityCatalog>` that survives daemon
    // shutdown gracefully (the sink becomes a no-op when the registry
    // is dropped, rather than blocking shutdown by keeping a strong
    // ref). Sinks live inside `McpClientService::notification_sinks`
    // for the lifetime of the daemon process — they're never
    // explicitly unregistered, which matches the daemon's whole-
    // process lifecycle.
    //
    // Both eager and lazy modes funnel through `McpReflectionSupervisor`
    // via `PostArcReflection::apply` so the sync-bridge logic lives
    // in exactly one module. Eager hands the supervisor the
    // per-server index it computed at boot; lazy lets the supervisor
    // compute its own once the background reflection pass finishes.
    reflection_plan.apply(Arc::clone(&mcp_client_svc), Arc::clone(&arc));

    BuiltAbilityRegistry {
        catalog: arc,
        plugin_runtime_manager,
        device_registrar_cell,
    }
}

pub fn build_registry_with_services(config: RegistryBuildConfig<'_>) -> Arc<AxonAbilityCatalog> {
    build_registry_with_services_result(config).catalog
}

/// Daemon-side convenience wrapper. Loads the agent registry and
/// builds the full `AxonAbilityCatalog` in one call, swallowing a
/// load failure into the empty-registry case (so a brand-new install
/// without `~/.easynet/agents.json` still boots).
///
/// `loaders`:
/// * `Some(vec)` — caller-provided context-loader chain. Tests
///   pass `Some(Arc::new(Vec::new()))` to get no loaders attached.
/// * `None` — auto-attach the daemon's default chain
///   (`user_profile` + `schedule` + `memory`). This is the path
///   `easynet-daemon` boots through: it called the explicit
///   variant before slice 35, which made every library / smoke
///   caller hand-build the chain or get silently empty
///   `context_used`.
///
/// RFC-002 §3.2 keyring auto-init for the daemon. Locates the
/// keyring file at `$XDG_CONFIG_HOME/easynet/keyring.json` (or
/// platform fallback), opens it under the passphrase from
/// `EASYNET_KEYRING_PASS` env var, and falls back to a deterministic
/// local-only passphrase when none is set. The local fallback is
/// fine for the `.localhost` default — federation peers never see
/// the master key, and the file is mode 0o600.
///
/// Returns `Err` only on filesystem / decode / KDF errors; absence
/// of an existing file is the happy path (creates a fresh ring).
fn init_keyring_for_daemon(
) -> anyhow::Result<std::sync::Arc<crate::runtime::keyring::KeyringHandle>> {
    use crate::runtime::keyring::store::default_keyring_path;
    use crate::runtime::keyring::KeyringHandle;
    let path = std::env::var("EASYNET_KEYRING_PATH")
        .map(std::path::PathBuf::from)
        .ok()
        .map_or_else(default_keyring_path, Ok)?;
    let pass = std::env::var("EASYNET_KEYRING_PASS").unwrap_or_else(|_| {
        // Local-fast default. Operators wanting stronger isolation
        // set EASYNET_KEYRING_PASS to a real secret. The literal
        // here is NOT a security boundary — the threat model for
        // local-fast assumes the host filesystem is the trust
        // boundary anyway. RFC-002 §3.2.
        "easynet-local-default-passphrase-v1".into()
    });
    Ok(std::sync::Arc::new(KeyringHandle::open_or_create(
        path, &pass,
    )?))
}

/// Exists so `bin/easynet-daemon.rs` does not have to reach into the
/// `pub(crate) registry::agents` module — that module's visibility is
/// intentionally crate-private.
/// **Phase 5c**. `hot_agent_registrar_cell` is the OnceLock the
/// boot path populates with `Arc<HotAgentRegistrar>` after the
/// `LocalRuntime` + dispatch handle are wired. Passed through to
/// the `agent.start` / `.stop` handlers so post-boot agent
/// additions are registered into `LocalRuntime`.
pub fn build_registry_for_daemon(config: RegistryDaemonBuildConfig) -> Arc<AxonAbilityCatalog> {
    build_registry_for_daemon_result(config)
        .expect("build daemon ability registry")
        .catalog
}

pub fn build_registry_for_daemon_result(
    config: RegistryDaemonBuildConfig,
) -> anyhow::Result<BuiltAbilityRegistry> {
    let RegistryDaemonBuildConfig {
        services,
        invocation_ledger,
        loaders,
        pages_identity,
        local_runtime,
        authority_context,
        hot_agent_registrar_cell,
        shared_stores,
    } = config;
    recover_descriptor_import_transactions_before_daemon_registry_boot()?;
    let agents = match crate::registry::agents::load_agents() {
        Ok(r) => r,
        Err(e) => {
            let err_msg = format!("{e}");
            crate::op_event!(
                component = agent_registry,
                kind = load_failed,
                level = "warn",
                error = err_msg,
                fallback = "empty_registry",
            );
            AgentRegistry::default()
        }
    };
    let loaders = loaders.unwrap_or_else(|| {
        Arc::new(context_loaders::default_loaders(Arc::clone(
            &services.schedule,
        )))
    });
    Ok(build_registry_with_services_result(RegistryBuildConfig {
        services,
        invocation_ledger,
        agents: &agents,
        loaders,
        pages_identity,
        local_runtime,
        authority_context,
        hot_agent_registrar_cell,
        shared_stores,
    }))
}

pub(super) fn recover_descriptor_import_transactions_before_daemon_registry_boot(
) -> anyhow::Result<usize> {
    let recovered_descriptor_imports = teach_ability::recover_descriptor_import_transactions()?;
    if recovered_descriptor_imports > 0 {
        let recovered = recovered_descriptor_imports.to_string();
        crate::op_event!(
            component = agents_boot,
            kind = descriptor_import_recovery_completed,
            recovered = recovered.as_str(),
            message =
                "recovered descriptor-import acquire transactions before daemon registry boot",
        );
    }
    Ok(recovered_descriptor_imports)
}
