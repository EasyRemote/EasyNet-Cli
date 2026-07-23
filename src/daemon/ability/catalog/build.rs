// EasyNet CLI — daemon ability-registry assembly
// ==============================================
//
// The build_registry* family: catalogue construction, service
// wiring, plugin-runtime manager assembly, daemon keyring init.
// Catalog build owner for daemon-owned system abilities.

use super::{daemon_invocation_contracts, profiles, runtime_admin_contracts};
#[cfg(feature = "axon-pb")]
use crate::daemon::ability::builtins::governance::invocation_cancel as invocation_cancel_ability;
use crate::daemon::ability::builtins::{
    agents::{
        chat as chat_ability, chat_history as chat_history_ability, discover as discover_ability,
        lifecycle as agent_lifecycle_ability, list as agent_list_ability,
    },
    automation::{
        discuss as discuss_ability, loop_ability, mission as mission_ability,
        orchestration as orchestration_ability, schedule as schedule_ability,
        think as think_ability,
    },
    device_control::{
        ability_management::{
            ops as device_ops_ability, publish as ability_publish_ability,
            registrar as device_ability_registrar,
        },
        file_edit as fs_edit_ability, file_transfer as file_transfer_ability, files as fs_ability,
        http as http_request_ability, process as process_exec_ability, session as session_ability,
        shell as shell_run_ability,
        terminal::{
            attach as pty_attach_ability, io as pty_io_ability, lifecycle as pty_lifecycle_ability,
        },
    },
    governance::{
        access_control as access_control_ability, admin_status as admin_status_ability,
        api_key as api_key_ability, consent as permission_ability, health as ping,
        invocation_history as invocation_history_ability, meta as meta_ability,
        network_health as network_health_ability, teach as teach_ability,
    },
    integrations::{
        a2a::{bridge as a2a_bridge_ability, client as a2a_client_ability},
        mcp::{bridge as mcp_bridge_ability, client as mcp_ability},
        openai_compat as openai_compat_ability, plugins as plugin_lifecycle_ability,
    },
    resources::{
        context::{ability as context_ability, loaders as context_loaders},
        files_store as files, list as list_resources_ability, media,
        pages::{self, PagesIdentity},
        skills::{install as skill_install_ability, publish as skill_publish_ability},
        voice as voice_call_ability,
    },
};
use crate::daemon::ability::dispatch::{AbilityAuthorityContext, AxonAbilityCatalog};
use crate::daemon::execution::loop_instance::LoopService;
use crate::daemon::execution::mission::discuss::DiscussService;
use crate::daemon::execution::permission::PermissionService;
use crate::daemon::execution::pty::PtyService;
use crate::daemon::execution::schedule::ScheduleService;
use crate::daemon::execution::session::SessionService;
use crate::daemon::persistence::{
    access_control::AccessControlStoreRegistry,
    agent_aggregate::{AgentAggregateRepository, AgentRegistryProjectionLoadError},
    agent_registry::AgentRegistry,
};
use anyhow::Context as _;
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
/// **Deterministic snapshot profile; never daemon boot.** Production daemon
/// assembly goes only through [`build_registry_for_daemon_result`], which is
/// fallible and accepts explicit runtime/authority/lifecycle dependencies.
/// This function is a process-cached CQRS read model: it is
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

/// Build an immutable metadata snapshot for one explicit authority set.
///
/// This is the non-executable counterpart to
/// [`build_registry_with_services_result`]. It never starts daemon runtime
/// services and deliberately has no `LocalRuntime`.
pub fn build_registry_snapshot_with_authority_context(
    authority_context: AbilityAuthorityContext,
) -> anyhow::Result<Arc<AxonAbilityCatalog>> {
    let agents = AgentRegistry::default();
    let config = deterministic_snapshot_build_config_for_profile(
        RegistryBuildServices::fresh(),
        &agents,
        DeterministicAuthorityProfile::DeviceDefault,
    );
    let config = RegistryBuildConfig {
        authority_context,
        ..config
    };
    build_registry_with_services_result_inner(
        config,
        RegistryAssemblyMode::DeterministicSnapshot {
            plugins: PluginRegistryMode::BuiltinOnlyDeterministic,
        },
    )
    .map(|built| built.catalog)
}

#[cfg(test)]
pub(crate) fn build_registry_for_test_execution() -> anyhow::Result<Arc<AxonAbilityCatalog>> {
    let agents = AgentRegistry::default();
    let mut config = deterministic_snapshot_build_config_for_profile(
        RegistryBuildServices::fresh(),
        &agents,
        DeterministicAuthorityProfile::DeviceDefault,
    );
    config.local_runtime = Some(
        crate::daemon::axon_bridge::runtime_factory::build_local_runtime(
            crate::daemon::axon_bridge::runtime_factory::rejecting_test_key_resolver(),
            None,
        ),
    );
    build_registry_with_services_result_inner(
        config,
        RegistryAssemblyMode::DeterministicSnapshot {
            plugins: PluginRegistryMode::BuiltinOnlyDeterministic,
        },
    )
    .map(|built| built.catalog)
}

fn build_registry_uncached() -> Arc<AxonAbilityCatalog> {
    build_registry_with_services_result_inner(
        deterministic_snapshot_build_config_for_profile(
            RegistryBuildServices::fresh(),
            &AgentRegistry::default(),
            DeterministicAuthorityProfile::DeviceDefault,
        ),
        RegistryAssemblyMode::DeterministicSnapshot {
            plugins: PluginRegistryMode::BuiltinOnlyDeterministic,
        },
    )
    .expect("canonical builtin ability catalog must assemble")
    .catalog
}

pub(crate) fn build_system_registry() -> Arc<AxonAbilityCatalog> {
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
        deterministic_snapshot_build_config_for_profile(
            RegistryBuildServices::fresh(),
            &AgentRegistry::default(),
            DeterministicAuthorityProfile::SystemInventory,
        ),
        RegistryAssemblyMode::DeterministicSnapshot {
            plugins: PluginRegistryMode::None,
        },
    )
    .expect("canonical system ability catalog must assemble")
    .catalog
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum DeterministicAuthorityProfile {
    /// Live/default read model for this daemon's device authority surface.
    DeviceDefault,
    /// Descriptor inventory read model spanning every static system owner.
    SystemInventory,
}

fn deterministic_snapshot_build_config_for_profile<'a>(
    services: RegistryBuildServices,
    agents: &'a AgentRegistry,
    authority_profile: DeterministicAuthorityProfile,
) -> RegistryBuildConfig<'a> {
    let snapshot_device =
        crate::core::ura::device_ura(crate::core::ura::REALM_EASYNET, "ability-catalog-snapshot");
    let authority_context = match authority_profile {
        DeterministicAuthorityProfile::DeviceDefault => {
            AbilityAuthorityContext::for_device_authority_root(snapshot_device)
                .expect("deterministic Device snapshot authority must be canonical")
        }
        DeterministicAuthorityProfile::SystemInventory => {
            // Descriptor generation and static metadata describe the complete
            // system catalogue, not one daemon deployment mode. Runtime
            // publication never uses this snapshot; it captures the live
            // authority-filtered catalogue.
            AbilityAuthorityContext::for_combined_authority_roots(snapshot_device)
                .expect("deterministic Device+Hub snapshot authorities must be canonical")
        }
    };
    let mut config =
        RegistryBuildConfig::new_with_authority_context(services, agents, authority_context);
    // Descriptor snapshots are metadata-only. Executable daemon assembly must
    // inject its one canonical runtime explicitly.
    config.local_runtime = None;
    config
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
/// `hot_agent_registrar_cell` breaks the catalogue/handler construction cycle.
/// Assembly wires the shared runtime and completed catalogue before returning;
/// lifecycle handlers treat a missing or pending registrar as a hard
/// precondition failure before durable state is mutated.
/// Result of constructing the daemon's local ability registry.
///
/// What this is NOT: a runtime executor. `catalog` owns handler metadata and
/// registration side tables; `plugin_runtime_manager` owns plugin package/load
/// state so boot-time services can derive wire/surface projections from the
/// same snapshot that registered plugin abilities.
pub struct BuiltAbilityRegistry {
    pub catalog: Arc<AxonAbilityCatalog>,
    pub plugin_runtime_manager: Arc<crate::daemon::plugins::PluginRuntimeManager>,
    /// Capability-state evidence derived from this exact assembly. A
    /// repository makes signaling ProviderBacked; executable delivery proof
    /// is a separate cutover input and is never inferred from registration.
    pub voice_capability_state:
        Vec<crate::daemon::ability::conformance::VoiceCapabilityStateEvidence>,
    #[cfg(feature = "axon-pb")]
    pub invocation_cancellations:
        crate::daemon::invocation::dispatch::cancellation::InvocationCancellationRegistry,
    /// Late-wired device-ability registrar cell. Populated during the
    /// build with a pending registrar; boot calls `set_runtime` on it
    /// (and may `replay_from_store`) once the `LocalRuntime` exists, so
    /// `ability.deploy` can run its install transaction. Mirrors
    /// `hot_agent_registrar_cell` but for device-owned deploys.
    pub device_registrar_cell: Arc<device_ops_ability::SharedDeviceRegistrarCell>,
}

#[derive(Clone)]
pub struct RegistrySharedStores {
    pub hub_published_abilities: Arc<
        crate::daemon::federation::read_model::hub_published_abilities::HubPublishedAbilityStore,
    >,
    pub voice_calls: Option<
        crate::daemon::ability::builtins::resources::voice_contract::VoiceCallProviderAssembly,
    >,
}

impl RegistrySharedStores {
    #[must_use]
    pub fn new(
        hub_published_abilities: Arc<
            crate::daemon::federation::read_model::hub_published_abilities::HubPublishedAbilityStore,
        >,
    ) -> Self {
        Self {
            hub_published_abilities,
            voice_calls: None,
        }
    }

    #[must_use]
    pub fn with_voice_call_provider_assembly(
        mut self,
        provider: crate::daemon::ability::builtins::resources::voice_contract::VoiceCallProviderAssembly,
    ) -> Self {
        self.voice_calls = Some(provider);
        self
    }
}

impl Default for RegistrySharedStores {
    fn default() -> Self {
        Self::new(crate::daemon::federation::read_model::hub_published_abilities::HubPublishedAbilityStore::new())
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
    pub access_control_stores: Arc<AccessControlStoreRegistry>,
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
                discover_ability::DetachedDiscoverFederationResolver,
            ),
            access_control_stores: Arc::new(AccessControlStoreRegistry::default()),
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

    #[must_use]
    pub fn with_access_control_stores(mut self, stores: Arc<AccessControlStoreRegistry>) -> Self {
        self.access_control_stores = stores;
        self
    }
}

pub struct RegistryBuildConfig<'a> {
    pub services: RegistryBuildServices,
    pub invocation_ledger: Option<Arc<axon_sdk::invocation::InvocationLedger>>,
    pub agents: &'a AgentRegistry,
    pub loaders: Arc<Vec<Arc<dyn chat_ability::ContextLoader>>>,
    pub pages_identity: PagesIdentity,
    pub local_runtime: Option<Arc<axon_sdk::invocation::LocalRuntime>>,
    pub authority_context: crate::daemon::ability::dispatch::AbilityAuthorityContext,
    pub hot_agent_registrar_cell: Arc<agent_lifecycle_ability::SharedHotRegistrarCell>,
    pub shared_stores: RegistrySharedStores,
}

impl<'a> RegistryBuildConfig<'a> {
    #[must_use]
    pub fn new(services: RegistryBuildServices, agents: &'a AgentRegistry) -> Self {
        Self::new_with_authority_context(
            services,
            agents,
            AbilityAuthorityContext::from_local_environment(),
        )
    }

    /// Construct a registry assembly request from an already-resolved authority
    /// snapshot. Deterministic read models and daemon boot use this path so
    /// configuration assembly cannot observe HOME before the caller-supplied
    /// authority state is installed.
    #[must_use]
    pub fn new_with_authority_context(
        services: RegistryBuildServices,
        agents: &'a AgentRegistry,
        authority_context: AbilityAuthorityContext,
    ) -> Self {
        Self {
            services,
            invocation_ledger: None,
            agents,
            loaders: Arc::new(Vec::new()),
            pages_identity: PagesIdentity::default(),
            local_runtime: None,
            authority_context,
            hot_agent_registrar_cell: Arc::new(
                agent_lifecycle_ability::SharedHotRegistrarCell::new(),
            ),
            shared_stores: RegistrySharedStores::default(),
        }
    }
}

pub struct RegistryDaemonBuildConfig {
    pub services: RegistryBuildServices,
    pub invocation_ledger: Option<Arc<axon_sdk::invocation::InvocationLedger>>,
    pub loaders: Option<Arc<Vec<Arc<dyn chat_ability::ContextLoader>>>>,
    pub pages_identity: PagesIdentity,
    pub local_runtime: Option<Arc<axon_sdk::invocation::LocalRuntime>>,
    pub authority_context: crate::daemon::ability::dispatch::AbilityAuthorityContext,
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
            authority_context: AbilityAuthorityContext::from_local_environment(),
            hot_agent_registrar_cell: Arc::new(
                agent_lifecycle_ability::SharedHotRegistrarCell::new(),
            ),
            shared_stores: RegistrySharedStores::default(),
        }
    }
}

pub fn build_registry_with_services_result(
    config: RegistryBuildConfig<'_>,
) -> anyhow::Result<BuiltAbilityRegistry> {
    anyhow::ensure!(
        config.local_runtime.is_some(),
        "daemon registry assembly requires an explicit canonical LocalRuntime"
    );
    build_registry_with_services_result_inner(config, RegistryAssemblyMode::DaemonRuntime)
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

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum RegistryAssemblyMode {
    DeterministicSnapshot { plugins: PluginRegistryMode },
    DaemonRuntime,
}

impl RegistryAssemblyMode {
    fn plugin_registry_mode(self) -> PluginRegistryMode {
        match self {
            Self::DeterministicSnapshot { plugins } => plugins,
            Self::DaemonRuntime => PluginRegistryMode::DefaultDaemon,
        }
    }

    fn starts_runtime_services(self) -> bool {
        matches!(self, Self::DaemonRuntime)
    }

    fn replays_hosted_agent_runtime(self) -> bool {
        matches!(self, Self::DaemonRuntime)
    }
}

fn build_plugin_runtime_manager(
    mode: PluginRegistryMode,
) -> anyhow::Result<Arc<crate::daemon::plugins::PluginRuntimeManager>> {
    let manager = match mode {
        PluginRegistryMode::None => crate::daemon::plugins::PluginRuntimeManager::from_state(
            crate::daemon::plugins::PluginRuntimeState::from_index(
                crate::daemon::plugins::PluginPackageIndex::default(),
            ),
        ),
        PluginRegistryMode::BuiltinOnlyDeterministic => {
            let index = crate::daemon::plugins::PluginPackageIndex::builtin()
                .context("build deterministic builtin plugin package index")?;
            let state = crate::daemon::plugins::PluginRuntimeState::from_index_with_planner(
                index,
                crate::daemon::plugins::PluginLoadPlanner::current_without_env_gates(),
            );
            crate::daemon::plugins::PluginRuntimeManager::from_state(state)
        }
        PluginRegistryMode::DefaultDaemon => crate::daemon::plugins::PluginRuntimeManager::new()
            .context("build daemon plugin runtime manager")?,
    };
    Ok(Arc::new(manager))
}

fn build_registry_with_services_result_inner(
    config: RegistryBuildConfig<'_>,
    assembly_mode: RegistryAssemblyMode,
) -> anyhow::Result<BuiltAbilityRegistry> {
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
        access_control_stores,
    } = services;
    #[cfg(feature = "axon-pb")]
    let invocation_cancellations =
        crate::daemon::invocation::dispatch::cancellation::InvocationCancellationRegistry::default(
        );

    let hosts_device_authority = authority_context.hosts_device_authority();
    let hosts_hub_authority = authority_context.hosts_hub_authority();
    let daemon_runtime_assembly = assembly_mode.starts_runtime_services();
    let replay_hosted_agent_runtime =
        hosts_device_authority && assembly_mode.replays_hosted_agent_runtime();
    let plugin_registry_mode = assembly_mode.plugin_registry_mode();
    let authority_context =
        declare_daemon_native_agent_authorities(authority_context, &pages_identity)?;
    // Hub-only assembly must not consult or replay Device product state. Keep
    // one empty registry view for provider constructors while static owner
    // admission remains centralized in `StaticRegistration::commit`.
    let hub_empty_agents = AgentRegistry::default();
    let agents = if hosts_device_authority {
        agents
    } else {
        &hub_empty_agents
    };
    let pages_identity = if hosts_device_authority {
        pages_identity
    } else {
        PagesIdentity::default()
    };
    let plugin_registry_mode = if hosts_device_authority {
        plugin_registry_mode
    } else {
        PluginRegistryMode::None
    };
    let local_runtime_owners = authority_context.local_runtime_owners();
    let voice_provider_assembly = shared_stores.voice_calls.clone();
    // Shared late-bound view of the completed live catalogue. Every handler
    // that needs post-assembly control-plane truth closes over this one cell.
    let local_registry_handle: Arc<std::sync::OnceLock<Arc<AxonAbilityCatalog>>> =
        Arc::new(std::sync::OnceLock::new());
    let registration_runtime = local_runtime.clone();
    let mut reg = match local_runtime {
        Some(runtime) => {
            AxonAbilityCatalog::new_with_runtime_and_authority_context(runtime, authority_context)
        }
        None => AxonAbilityCatalog::new_metadata_only_with_authority_context(authority_context),
    };
    if hosts_device_authority {
        daemon_invocation_contracts::register_for_owner(
            &mut reg,
            &crate::daemon::ability::dispatch::OwnerKind::Device,
        )
        .context("register Device daemon Invocation descriptor contracts")?;
    }
    if hosts_hub_authority {
        daemon_invocation_contracts::register_for_owner(
            &mut reg,
            &crate::daemon::ability::dispatch::OwnerKind::Hub,
        )
        .context("register Hub daemon Invocation descriptor contracts")?;
        runtime_admin_contracts::register(&mut reg)
            .context("register Hub runtime-admin descriptor contracts")?;
    }
    ping::register(&mut reg);
    network_health_ability::register(&mut reg, Arc::clone(&discover_federation_resolver));
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
    invocation_history_ability::register(&mut reg, invocation_ledger.clone());
    // RFC-014 ability access-control governance surface.
    // These handlers are daemon policy management/read-model adapters over the
    // text-backed access-control store; they do not touch keyring secrets and
    // do not introduce a standalone policy engine.
    access_control_ability::register_with_ledger(
        &mut reg,
        invocation_ledger.clone(),
        Arc::clone(&local_registry_handle),
        access_control_stores,
    );
    #[cfg(feature = "axon-pb")]
    invocation_cancel_ability::register(&mut reg, invocation_cancellations.clone());
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
    // plus meta.list_resources (A9). `resources::media` owns the
    // shared metadata and only registers still-unwired stubs; real
    // modules own their names directly. This keeps each ability +
    // call-mode slot single-owner and avoids precedence-based
    // replacement semantics.
    media::register(&mut reg);
    #[cfg(feature = "native-media")]
    {
        media::camera_snapshot::register(&mut reg);
        media::screen_snapshot::register(&mut reg);
        media::mic_subscribe::register(&mut reg);
    }
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
    let device_registrar_cell: Arc<device_ops_ability::SharedDeviceRegistrarCell> =
        Arc::new(std::sync::OnceLock::new());
    if device_registrar_cell
        .set(device_ability_registrar::DeviceAbilityRegistrar::new_pending())
        .is_err()
    {
        panic!("device registrar cell must be written exactly once during registry build");
    }
    device_ops_ability::register(
        &mut reg,
        Arc::clone(&device_registrar_cell),
        Arc::clone(&local_registry_handle),
        Arc::clone(&discover_federation_resolver),
    );
    // voice.* call signaling abilities — `easynet call …`
    // subcommand surface routes through these via the same
    // ability-only invocation path every other CLI surface uses.
    if hosts_hub_authority {
        if let Some(provider) = voice_provider_assembly.as_ref() {
            voice_call_ability::register(&mut reg, provider.clone());
        }
    }
    // Stateful device plugins. Package discovery, boot-time load decisions, and
    // handler registration stay separate so install/remove/update state cannot
    // leak into runtime call semantics.
    let plugin_runtime_manager = build_plugin_runtime_manager(plugin_registry_mode)?;
    match plugin_registry_mode {
        PluginRegistryMode::None => {}
        PluginRegistryMode::BuiltinOnlyDeterministic => {
            plugin_runtime_manager
                .register_current_plugins(&mut reg)
                .context("register deterministic builtin plugins")?;
        }
        PluginRegistryMode::DefaultDaemon => {
            plugin_runtime_manager
                .register_default_plugins(&mut reg)
                .context("register configured daemon plugins")?;
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
            crate::daemon::axon_bridge::hot_agent_registrar::HotAgentRegistrar::new_pending(
                Arc::clone(&loaders),
                Arc::clone(&local_registry_handle),
                Arc::clone(&discover_federation_resolver),
            );
        if let Some(runtime) = registration_runtime.as_ref() {
            hot_registrar
                .set_runtime(Arc::clone(runtime))
                .context("wire hosted-Agent registrar runtime")?;
        }
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
    // and routes every child turn through daemon Invocation, so
    // resolution, admission, dispatch, and receipt ownership stay
    // identical to SDK and CLI calls.
    orchestration_ability::register(&mut reg, Arc::clone(&discuss));
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
        if let Some(identity) = pages_identity.user_root_identity()? {
            let user = identity.user;
            let realm = identity.realm;
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
                    realm: pages_realm.clone(),
                },
            );
            // RFC-006-C v0.1 — API key abilities. Register under the
            // same `user` identity pages used so a single user owns
            // both surface families on this daemon.
            api_key_ability::register(&mut reg, &user, &pages_realm);
        }
        // RFC-006-C v0.1 — device-local OpenAI shim. Device-owned,
        // no `<user>` slot — registers regardless of pairing state.
        openai_compat_ability::set_dispatch_handle(Arc::clone(&local_registry_handle));
        openai_compat_ability::set_identity(pages_identity.clone())?;
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
    // Worker, judge, curator, and publication child calls re-enter
    // daemon Invocation through the Mission application gateway.
    think_ability::register(&mut reg);
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
    // The device-owned aggregate `agent.discover` owns the top-level view and
    // reloads the Agent aggregate per call, so it never chooses a random first
    // agent as a synthetic self or splits registry reads from hosted identity
    // reads. Per-agent `<agent>.discover` /
    // `<agent>.invoke` are hosted-agent lifecycle rows; they are replayed
    // through HotAgentRegistrar after `Arc::new(reg)` below.
    discover_ability::register_device_aggregate_with_resolver(
        &mut reg,
        || {
            crate::daemon::persistence::agent_aggregate::AgentAggregateRepository::load_snapshot()
                .map(|snapshot| snapshot.registered_agent_registry_projection())
                .map_err(|error| anyhow::anyhow!("load discover Agent aggregate: {error:#}"))
        },
        Arc::clone(&local_registry_handle),
        Arc::clone(&discover_federation_resolver),
    );

    // RFC-002 §3.3: register `device.keyring.*` for the daemon's
    // own self-bundle, scoped under the literal owner `device`.
    // The daemon publishes its 10 keyring abilities under this
    // namespace so any local agent can call them through the
    // standard dispatch path. The ability provider is the daemon-local key
    // service; this process never opens key storage or derives a master key.
    //
    // The legacy owner string was `legacy self alias` — a "this device"
    // alias. v4.1.5 onward names the actor explicitly: keyring
    // belongs to the device, so the owner is `device`. The
    // catalogue now lists these as `device.keyring.<verb>`,
    // matching the URA `callee = device/<id>` that already
    // covers them.
    //
    crate::daemon::keyring::abilities::register_for_owner(
        &mut reg,
        "device",
        key_service_for_daemon(),
    );
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
        local_runtime_owners,
        profiles::load_host_descriptors,
        Arc::clone(&local_registry_handle),
        Arc::clone(&shared_stores.hub_published_abilities),
    );
    // a2a.bridge.list_skills — same edge-adapter pattern as the MCP
    // bridge above, but for the A2A agent-card surface. The provider
    // reloads the Agent aggregate per call and propagates corruption/read
    // failures; it never substitutes the boot snapshot as a plausible stale
    // catalog or reopens agents.json as a registry-only read.
    a2a_bridge_ability::register(
        &mut reg,
        || {
            crate::daemon::persistence::agent_aggregate::AgentAggregateRepository::load_snapshot()
                .map(|snapshot| snapshot.registered_agent_registry_projection())
                .map_err(|error| anyhow::anyhow!("load A2A Agent aggregate: {error:#}"))
        },
        Arc::clone(&local_registry_handle),
    );
    // a2a.client.send_task — outbound A2A. The handler submits a signed
    // descriptor-bound InvokeRequest to the local daemon's canonical
    // Invocation service.
    a2a_client_ability::register(&mut reg, Arc::clone(&discover_federation_resolver));
    // mcp.client.{list,call} — outbound MCP. Boots an
    // McpClientService from ~/.easynet/mcps.json (missing
    // file → empty service, no upstreams). Each upstream MCP
    // server is spawned lazily on first call; subsequent calls
    // reuse the live connection. Parse errors at boot bubble up
    // because a malformed file is an operator typo, not a "no
    // upstreams" condition.
    let mcp_svc = if hosts_device_authority && daemon_runtime_assembly {
        let mcps_path = crate::daemon::execution::mcp::McpClientService::default_config_path();
        Arc::new(
            crate::daemon::execution::mcp::McpClientService::from_path(&mcps_path)
                .with_context(|| format!("load MCP client config {}", mcps_path.display()))?,
        )
    } else {
        Arc::new(crate::daemon::execution::mcp::McpClientService::new())
    };
    mcp_ability::register(&mut reg, mcp_svc.clone());

    // Install the same `Arc<McpClientService>` as the process-wide
    // handle used by `[exec] kind="mcp"` ability dispatch. Before this
    // line `mcp_executor::run_mcp_exec` would return a typed error;
    // after this line every MCP surface in the daemon — outbound
    // `mcp.client.*`, reflective registry below, and exec —
    // shares one connection pool, one config snapshot, one `next_id`
    // sequence per upstream. No silent divergence between surfaces.
    if hosts_device_authority && daemon_runtime_assembly {
        crate::daemon::ability::builtins::integrations::mcp::executor::set_process_client(
            mcp_svc.clone(),
        );
    }

    // MCP reflection policy. Direct MCP client abilities are already
    // registered above; this section only decides whether upstream
    // tools are projected as first-class EasyNet abilities.
    //
    // Default is lazy: `easynet start` must return a ready daemon
    // after the bounded local registry build, while external MCP
    // servers are discovered by a background supervisor against the
    // dynamic registry overlay. Operators that need blocking
    // reflection can set EASYNET_MCP_REFLECTION=eager; production
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
    let reflection_plan = if hosts_device_authority && daemon_runtime_assembly {
        let user_root_identity = pages_identity.user_root_identity()?;
        let (reflection_user, reflection_realm) = user_root_identity
            .as_ref()
            .map(|identity| (Some(identity.user.as_str()), identity.realm.as_str()))
            .unwrap_or((None, ""));
        crate::daemon::ability::builtins::integrations::mcp::reflective_registry::PostArcReflection::plan(
            crate::daemon::ability::builtins::integrations::mcp::reflective_registry::McpReflectionMode::from_env()
                .with_context(|| {
                    format!(
                        "parse {}",
                        crate::daemon::ability::builtins::integrations::mcp::reflective_registry::ENV_MCP_REFLECTION_MODE
                    )
                })?,
            reflection_user,
            reflection_realm,
            &mcp_svc,
            &mut reg,
        )
    } else {
        crate::daemon::ability::builtins::integrations::mcp::reflective_registry::PostArcReflection::Skip
    };
    // agent.list — operational view of registered LLM
    // sub-agents. Cheap-row projection (name, runtime, model, label);
    // for the protocol agent-card view see a2a.bridge.list_skills.
    agent_list_ability::register(&mut reg, || {
        crate::daemon::persistence::agent_aggregate::AgentAggregateRepository::load_snapshot()
            .map_err(|error| anyhow::anyhow!("agent.list: load aggregate snapshot: {error:#}"))
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
    let authority_exclusions = reg.static_authority_exclusion_snapshot();
    if daemon_runtime_assembly && !authority_exclusions.is_empty() {
        let excluded_total = authority_exclusions.values().sum::<usize>().to_string();
        let excluded_by_owner = authority_exclusions
            .iter()
            .map(|(owner, count)| format!("{owner}:{count}"))
            .collect::<Vec<_>>()
            .join(",");
        crate::op_event!(
            component = ability_catalog,
            kind = static_authority_filter_applied,
            authority_set = reg.authority_set_label(),
            excluded_total = excluded_total.as_str(),
            excluded_by_owner = excluded_by_owner.as_str(),
            message =
                "static catalogue excluded abilities from owner planes not hosted by this runtime",
        );
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

    if replay_hosted_agent_runtime {
        if let Some(hot_registrar) = hot_agent_registrar_cell.get().cloned() {
            let recovered_purge =
                agent_lifecycle_ability::recover_pending_purge_before_agent_replay(
                    &hot_agent_registrar_cell,
                )
                .context("recover pending Agent purge before hosted-Agent boot replay")?;
            let recovered_agents;
            let replay_agents = if recovered_purge {
                recovered_agents =
                    AgentAggregateRepository::load_registered_agent_registry_projection()
                        .map_err(AgentRegistryProjectionLoadError::into_source_or_self)
                        .context("reload Agent registry after purge boot recovery")?;
                &recovered_agents
            } else {
                agents
            };
            for (agent_name, entry) in replay_agents.agents.iter() {
                crate::support::async_bridge::run_blocking(
                    hot_registrar.register_agent(agent_name, entry),
                    crate::support::async_bridge::SyncBridgeRuntimePolicy::BuildCurrentThreadTokio,
                )
                .with_context(|| {
                    format!("replay hosted-Agent {agent_name:?} into live catalog/runtime")
                })?;
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
    reflection_plan.apply(Arc::clone(&mcp_svc), Arc::clone(&arc));

    let voice_capability_state =
        crate::daemon::ability::conformance::voice_capability_state_evidence(
            crate::daemon::ability::conformance::VoiceAssemblyEvidence {
                repository_assembled: voice_provider_assembly.is_some(),
                executable_delivery_evidence: false,
            },
        );

    Ok(BuiltAbilityRegistry {
        catalog: arc,
        plugin_runtime_manager,
        voice_capability_state,
        device_registrar_cell,
        #[cfg(feature = "axon-pb")]
        invocation_cancellations,
    })
}

/// Declare the stable Agent execution planes derived from the daemon's paired
/// user identity before the catalog is constructed. These roots are runtime
/// architecture, not hosted-agent lifecycle rows: Pages and Files execute
/// resource-management abilities through `<user>.pages` and `<user>.files`,
/// while reflected MCP tools execute through `<user>.mcp`. Static eager
/// registration and post-boot dynamic overlays therefore pass the same
/// immutable authority gate.
fn declare_daemon_native_agent_authorities(
    mut authority_context: AbilityAuthorityContext,
    identity: &PagesIdentity,
) -> anyhow::Result<AbilityAuthorityContext> {
    if !authority_context.hosts_device_authority() {
        return Ok(authority_context);
    }
    let user_root_identity = identity.user_root_identity()?;
    let Some(user_root_identity) = user_root_identity.as_ref() else {
        return Ok(authority_context);
    };
    let realm = user_root_identity.realm.as_str();
    let user = user_root_identity.user.as_str();
    let declared_roots = [
        ("Pages", pages::management_agent_ura(realm, user)),
        ("Files", files::management_agent_ura(realm, user)),
        (
            "MCP reflection",
            axon_sdk::ura::agent_ura(realm, user, "mcp"),
        ),
    ];
    for (executor, authority_root) in declared_roots {
        authority_context = authority_context
            .with_declared_agent_authority_root(authority_root)
            .with_context(|| {
                format!(
                    "{executor} execution host cannot be admitted by the daemon authority context"
                )
            })?;
    }
    Ok(authority_context)
}

/// Daemon-side assembly entry point. Loads the agent registry and builds the
/// full `AxonAbilityCatalog` in one call. A brand-new installation receives
/// the empty registry from `load_agents`; malformed or inaccessible durable
/// state is a boot error rather than a fabricated empty catalog.
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
/// Build the daemon ability provider against the canonical local key service.
/// Registry shape is stable regardless of sidecar health. Invocation returns
/// the typed transport failure when the lifecycle supervisor reports the key
/// service unavailable; assembly never falls back to a local store or omits
/// the capability.
fn key_service_for_daemon(
) -> std::sync::Arc<dyn crate::daemon::keyring::abilities::ManagedSigningProvider> {
    std::sync::Arc::new(crate::daemon::identity::self_identity::KeyringClient::default_path())
}

/// Exists so `bin/easynet-daemon.rs` does not have to reach into the
/// `pub(crate) registry::agents` module — that module's visibility is
/// intentionally crate-private.
/// **Phase 5c**. `hot_agent_registrar_cell` is the OnceLock the
/// boot path populates with `Arc<HotAgentRegistrar>` after the
/// `LocalRuntime` + dispatch handle are wired. Passed through to
/// the `agent.start` / `.stop` handlers so post-boot agent
/// additions are registered into `LocalRuntime`.
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
    let hosts_device_authority = authority_context.hosts_device_authority();
    let agents = if hosts_device_authority {
        recover_descriptor_import_transactions_before_daemon_registry_boot()?;
        // `load_agents` already defines the one legitimate empty state: a
        // missing registry on a brand-new installation. Any other failure is
        // corrupt or inaccessible durable daemon state and must abort boot;
        // replacing it with an empty registry would silently withdraw hosted
        // abilities from the control plane.
        AgentAggregateRepository::load_registered_agent_registry_projection()
            .map_err(AgentRegistryProjectionLoadError::into_source_or_self)
            .map_err(|error| anyhow::anyhow!("load daemon agent registry: {error:#}"))?
    } else {
        AgentRegistry::default()
    };
    let loaders = loaders.unwrap_or_else(|| {
        Arc::new(context_loaders::default_loaders(Arc::clone(
            &services.schedule,
        )))
    });
    build_registry_with_services_result(RegistryBuildConfig {
        services,
        invocation_ledger,
        agents: &agents,
        loaders,
        pages_identity,
        local_runtime,
        authority_context,
        hot_agent_registrar_cell,
        shared_stores,
    })
    .context("assemble daemon ability catalog")
}

pub(crate) fn recover_descriptor_import_transactions_before_daemon_registry_boot(
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

#[cfg(test)]
mod daemon_native_authority_tests {
    use super::*;
    use crate::daemon::ability::dispatch::{ControlPlaneImplementation, OwnerKind, StreamSource};
    use crate::daemon::ability::manifest::AbilityManifest;
    use crate::daemon::ability::AuthorityScope;

    #[test]
    fn paired_identity_declares_dynamic_mcp_execution_authority() {
        let device_ura = crate::core::ura::device_ura("native-authority", "dev-1");
        let identity = PagesIdentity {
            user: Some("alice".to_string()),
            realm: Some("native-authority".to_string()),
            listener_port: None,
        };
        let authority_context = declare_daemon_native_agent_authorities(
            AbilityAuthorityContext::for_combined_authority_roots(device_ura)
                .expect("combined authority context"),
            &identity,
        )
        .expect("declare daemon-native Agent authorities");
        let registry = AxonAbilityCatalog::new_with_runtime_and_authority_context(
            crate::daemon::axon_bridge::runtime_factory::build_local_runtime(
                crate::daemon::axon_bridge::runtime_factory::rejecting_test_key_resolver(),
                None,
            ),
            authority_context,
        );
        let mcp_root = axon_sdk::ura::agent_ura("native-authority", "alice", "mcp");
        let handler = Arc::new(|_args| {
            let (_tx, rx) = tokio::sync::broadcast::channel(1);
            Ok(StreamSource::Live(rx))
        });

        registry
            .hot_register_stream_with_spec_impl_and_authority_scope(
                "mcp_authority_probe",
                OwnerKind::Agent("mcp".to_string()),
                AuthorityScope::new("agent:mcp", &mcp_root).expect("MCP authority scope"),
                AbilityManifest::new(
                    "mcp_authority_probe",
                    "MCP authority assembly probe",
                    serde_json::json!({"type": "object"}),
                )
                .and_then(|manifest| manifest.with_admission_action("stream"))
                .expect("probe manifest"),
                handler,
                ControlPlaneImplementation::native_daemon(),
            )
            .expect("dynamic MCP registration must use the declared execution authority");

        let record = registry
            .control_plane_record_for_authority_mode(
                &mcp_root,
                "mcp_authority_probe",
                crate::daemon::ability::CallMode::Stream,
            )
            .expect("authority-scoped lookup")
            .expect("dynamic MCP control-plane row");
        assert_eq!(record.authority().scope().authority_root(), mcp_root);
    }
}
