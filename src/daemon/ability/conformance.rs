//! EasyNet Axon for AgentNet
//! =========================
//!
//! File: src/daemon/ability/conformance.rs
//! Description: Typed daemon baseline ability contract for Hub and Device
//! modes.
//!
//! Protocol Responsibility:
//! - Defines which public ability names must exist on the daemon runtime
//!   surface and which transport mode each ability uses.
//! - Separates daemon Invocation control surfaces from local registry
//!   surfaces and Axon runtime-admin handshakes.
//!
//! Implementation Approach:
//! - Model each requirement as a small value object: name, call mode,
//!   serving surface, and semantic domain.
//! - Provide conformance report objects instead of scattering string-list
//!   assertions through unrelated tests.
//!
//! Usage Contract:
//! - New daemon baseline abilities must be added here before they are wired
//!   in dispatch or registry assembly.
//! - Backend-only product aggregators must not be represented as canonical
//!   daemon baseline abilities.
//!
//! Architectural Position:
//! - Lives in `daemon::ability` because this is an ability control-plane
//!   contract, not a Hub handler implementation.

use std::collections::BTreeSet;

use crate::daemon::ability::CallMode;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum BaselineSurface {
    DaemonInvocation,
    LocalRegistry,
    AxonRuntimeAdmin,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum BaselineDomain {
    HubFederation,
    HubNamespace,
    HubIdentity,
    HubRuntimeAdmin,
    HubIntrospection,
    HubMedia,
    DeviceHealth,
    DeviceLifecycle,
    DeviceLocomotion,
    DeviceTransfer,
    DeviceTerminal,
    DeviceSession,
    DeviceConsent,
    DeviceAgent,
    DeviceSkill,
    DeviceBridge,
    DeviceOrchestration,
    DeviceContext,
    DeviceMedia,
    DeviceRemoteDesktop,
    DeviceOpenAiCompat,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct BaselineAbility {
    pub name: &'static str,
    pub call_mode: CallMode,
    pub surface: BaselineSurface,
    pub domain: BaselineDomain,
}

/// Deployment truth for descriptor contracts that are intentionally absent
/// from the live operational inventory.
#[derive(Debug, Clone, Copy, PartialEq, Eq, serde::Deserialize, serde::Serialize)]
#[serde(rename_all = "snake_case")]
pub enum CapabilityState {
    Unsupported,
    /// A provider port exists, but production assembly has no qualifying
    /// realm-scoped implementation.
    Seam,
    ProviderBacked,
    CutoverReady,
}

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct VoiceAssemblyEvidence {
    pub repository_assembled: bool,
    pub executable_delivery_evidence: bool,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct VoiceCapabilityStateEvidence {
    pub name: &'static str,
    pub call_mode: CallMode,
    pub state: CapabilityState,
    pub authority: &'static str,
    pub reason: &'static str,
}

const VOICE_REALM_REPOSITORY_SEAM: &str =
    "no production realm-shared VoiceCallRepository provider is assembled";
const VOICE_MEDIA_UNSUPPORTED: &str = "no Hub voice media provider assembly port is available";
const VOICE_PROVIDER_BACKED: &str = "realm-shared VoiceCallRepository is assembled";
const VOICE_CUTOVER_READY: &str = "executable delivery evidence covers the assembled provider";

pub const VOICE_SIGNALING_CONTRACTS: &[(&str, CallMode)] = &[
    ("voice.create_call", CallMode::Rpc),
    ("voice.show_call", CallMode::Rpc),
    ("voice.join_call", CallMode::Rpc),
    ("voice.leave_call", CallMode::Rpc),
    ("voice.end_call", CallMode::Rpc),
    ("voice.watch_call", CallMode::Rpc),
    ("voice.report_metrics", CallMode::Rpc),
    ("voice.list_calls", CallMode::Rpc),
];

/// Voice descriptors remain contract artifacts, while this table records why
/// each route is absent from the live catalog. It must not be interpreted as
/// registration input.
pub fn voice_capability_state_evidence(
    assembly: VoiceAssemblyEvidence,
) -> Vec<VoiceCapabilityStateEvidence> {
    assert!(
        !assembly.executable_delivery_evidence || assembly.repository_assembled,
        "Voice CutoverReady evidence requires an assembled repository provider"
    );
    let signaling_state = if assembly.executable_delivery_evidence {
        CapabilityState::CutoverReady
    } else if assembly.repository_assembled {
        CapabilityState::ProviderBacked
    } else {
        CapabilityState::Seam
    };
    let signaling_reason = match signaling_state {
        CapabilityState::Seam => VOICE_REALM_REPOSITORY_SEAM,
        CapabilityState::ProviderBacked => VOICE_PROVIDER_BACKED,
        CapabilityState::CutoverReady => VOICE_CUTOVER_READY,
        CapabilityState::Unsupported => unreachable!("signaling has an assembly seam"),
    };
    let mut evidence = VOICE_SIGNALING_CONTRACTS
        .iter()
        .map(|(name, call_mode)| VoiceCapabilityStateEvidence {
            name,
            call_mode: *call_mode,
            state: signaling_state,
            authority: "Hub",
            reason: signaling_reason,
        })
        .collect::<Vec<_>>();
    evidence.push(VoiceCapabilityStateEvidence {
        name: "voice.subscribe",
        call_mode: CallMode::Stream,
        state: CapabilityState::Unsupported,
        authority: "Hub",
        reason: VOICE_MEDIA_UNSUPPORTED,
    });
    evidence.push(VoiceCapabilityStateEvidence {
        name: "voice.transcribe",
        call_mode: CallMode::Bidi,
        state: CapabilityState::Unsupported,
        authority: "Hub",
        reason: VOICE_MEDIA_UNSUPPORTED,
    });
    evidence
}

impl BaselineAbility {
    #[must_use]
    pub const fn rpc(name: &'static str, surface: BaselineSurface, domain: BaselineDomain) -> Self {
        Self {
            name,
            call_mode: CallMode::Rpc,
            surface,
            domain,
        }
    }

    #[must_use]
    pub const fn stream(
        name: &'static str,
        surface: BaselineSurface,
        domain: BaselineDomain,
    ) -> Self {
        Self {
            name,
            call_mode: CallMode::Stream,
            surface,
            domain,
        }
    }

    #[must_use]
    pub const fn bidi(
        name: &'static str,
        surface: BaselineSurface,
        domain: BaselineDomain,
    ) -> Self {
        Self {
            name,
            call_mode: CallMode::Bidi,
            surface,
            domain,
        }
    }
}

macro_rules! daemon_rpc {
    ($name:expr, $domain:ident) => {
        BaselineAbility::rpc(
            $name,
            BaselineSurface::DaemonInvocation,
            BaselineDomain::$domain,
        )
    };
}

macro_rules! daemon_stream {
    ($name:expr, $domain:ident) => {
        BaselineAbility::stream(
            $name,
            BaselineSurface::DaemonInvocation,
            BaselineDomain::$domain,
        )
    };
}

macro_rules! runtime_admin_rpc {
    ($name:expr, $domain:ident) => {
        BaselineAbility::rpc(
            $name,
            BaselineSurface::AxonRuntimeAdmin,
            BaselineDomain::$domain,
        )
    };
}

macro_rules! runtime_admin_bidi {
    ($name:expr, $domain:ident) => {
        BaselineAbility::bidi(
            $name,
            BaselineSurface::AxonRuntimeAdmin,
            BaselineDomain::$domain,
        )
    };
}

macro_rules! local_rpc {
    ($name:expr, $domain:ident) => {
        BaselineAbility::rpc(
            $name,
            BaselineSurface::LocalRegistry,
            BaselineDomain::$domain,
        )
    };
}

macro_rules! local_stream {
    ($name:expr, $domain:ident) => {
        BaselineAbility::stream(
            $name,
            BaselineSurface::LocalRegistry,
            BaselineDomain::$domain,
        )
    };
}

macro_rules! local_bidi {
    ($name:expr, $domain:ident) => {
        BaselineAbility::bidi(
            $name,
            BaselineSurface::LocalRegistry,
            BaselineDomain::$domain,
        )
    };
}

pub const ABILITY_FEDERATION_JOIN: &str = "federation.join";
pub const ABILITY_FEDERATION_ADVERTISE_AGENT: &str = "federation.advertise_agent";
pub const ABILITY_FEDERATION_ADVERTISE_ABILITIES: &str = "federation.advertise_abilities";
pub const ABILITY_FEDERATION_HEARTBEAT: &str = "federation.heartbeat";
pub const ABILITY_FEDERATION_RESOLVE: &str = "federation.resolve";
pub const ABILITY_NAMESPACE_RESOLVE: &str = "namespace.resolve";
pub const ABILITY_NAMESPACE_PROXY_RESOLVE: &str = "namespace.proxy_resolve";
pub const ABILITY_FEDERATION_RESOLVE_KEY: &str = "federation.resolve_key";
pub const ABILITY_FEDERATION_DISCOVER: &str = "federation.discover";
pub const ABILITY_FEDERATION_SUBSCRIBE_DIRECTORY_V2: &str = "federation.subscribe_directory_v2";
pub const ABILITY_FEDERATION_LIST_USER_DEVICES: &str = "federation.list_user_devices";
pub const ABILITY_FEDERATION_PROXY_LIST_USER_DEVICES: &str = "federation.proxy_list_user_devices";
pub const ABILITY_FEDERATION_REVOKE: &str =
    crate::daemon::ability::runtime_admin_routes_gen::FEDERATION_REVOKE;
pub const ABILITY_IDENTITY_REGISTER_PUBKEY: &str = "identity.register_pubkey";
pub const ABILITY_IDENTITY_LIST_USER_PUBKEYS: &str = "identity.list_user_pubkeys";
pub const ABILITY_IDENTITY_REVOKE_USER_PUBKEY: &str = "identity.revoke_user_pubkey";
pub const ABILITY_PRINCIPAL_CREATE: &str =
    crate::daemon::ability::principal_routes_gen::ABILITY_PRINCIPAL_CREATE;
pub const ABILITY_PRINCIPAL_BIND_FIRST_KEY: &str =
    crate::daemon::ability::principal_routes_gen::ABILITY_PRINCIPAL_BIND_FIRST_KEY;
pub const ABILITY_PRINCIPAL_ADD_KEY: &str =
    crate::daemon::ability::principal_routes_gen::ABILITY_PRINCIPAL_ADD_KEY;
pub const ABILITY_PRINCIPAL_ROTATE_KEY: &str =
    crate::daemon::ability::principal_routes_gen::ABILITY_PRINCIPAL_ROTATE_KEY;
pub const ABILITY_PRINCIPAL_REVOKE_KEY: &str =
    crate::daemon::ability::principal_routes_gen::ABILITY_PRINCIPAL_REVOKE_KEY;
pub const ABILITY_PRINCIPAL_CONFIGURE_RECOVERY: &str =
    crate::daemon::ability::principal_routes_gen::ABILITY_PRINCIPAL_CONFIGURE_RECOVERY;
pub const ABILITY_PRINCIPAL_RECOVER: &str =
    crate::daemon::ability::principal_routes_gen::ABILITY_PRINCIPAL_RECOVER;
pub const ABILITY_PRINCIPAL_SUSPEND: &str =
    crate::daemon::ability::principal_routes_gen::ABILITY_PRINCIPAL_SUSPEND;
pub const ABILITY_PRINCIPAL_REACTIVATE: &str =
    crate::daemon::ability::principal_routes_gen::ABILITY_PRINCIPAL_REACTIVATE;
pub const ABILITY_PRINCIPAL_DELETE: &str =
    crate::daemon::ability::principal_routes_gen::ABILITY_PRINCIPAL_DELETE;
pub const ABILITY_PRINCIPAL_ISSUE_ENROLLMENT: &str =
    crate::daemon::ability::principal_routes_gen::ABILITY_PRINCIPAL_ISSUE_ENROLLMENT;
pub const ABILITY_PRINCIPAL_REVOKE_ENROLLMENT: &str =
    crate::daemon::ability::principal_routes_gen::ABILITY_PRINCIPAL_REVOKE_ENROLLMENT;
pub const ABILITY_PRINCIPAL_ISSUE_GRANT: &str =
    crate::daemon::ability::principal_routes_gen::ABILITY_PRINCIPAL_ISSUE_GRANT;
pub const ABILITY_PRINCIPAL_REVOKE_GRANT: &str =
    crate::daemon::ability::principal_routes_gen::ABILITY_PRINCIPAL_REVOKE_GRANT;
pub const ABILITY_PRINCIPAL_GET: &str =
    crate::daemon::ability::principal_routes_gen::ABILITY_PRINCIPAL_GET;
pub const ABILITY_RUNTIME_BOOTSTRAP_SELF_IDENTITY: &str = "runtime.bootstrap_self_identity";
pub const ABILITY_META_LIST_ABILITIES: &str = "meta.list_abilities";
pub const ABILITY_FEDERATION_STATUS: &str = "federation.status";
pub const ABILITY_SESSION_LIST: &str =
    crate::daemon::ability::runtime_admin_routes_gen::SESSION_LIST;
pub const ABILITY_SESSION_OPEN: &str = "session.open";

#[cfg(test)]
const FORBIDDEN_BACKEND_AGGREGATE_ALIAS: &str = "aggregate.list_abilities_catalog";

const HUB_BASELINE: &[BaselineAbility] = &[
    daemon_rpc!(ABILITY_FEDERATION_JOIN, HubFederation),
    daemon_rpc!(ABILITY_FEDERATION_ADVERTISE_AGENT, HubFederation),
    daemon_rpc!(ABILITY_FEDERATION_ADVERTISE_ABILITIES, HubFederation),
    daemon_rpc!(ABILITY_FEDERATION_HEARTBEAT, HubFederation),
    daemon_rpc!(ABILITY_FEDERATION_RESOLVE, HubFederation),
    daemon_rpc!(ABILITY_NAMESPACE_RESOLVE, HubNamespace),
    daemon_rpc!(ABILITY_NAMESPACE_PROXY_RESOLVE, HubNamespace),
    daemon_rpc!(ABILITY_FEDERATION_RESOLVE_KEY, HubFederation),
    daemon_rpc!(ABILITY_FEDERATION_DISCOVER, HubFederation),
    daemon_stream!(ABILITY_FEDERATION_SUBSCRIBE_DIRECTORY_V2, HubFederation),
    daemon_rpc!(ABILITY_FEDERATION_LIST_USER_DEVICES, HubFederation),
    daemon_rpc!(ABILITY_FEDERATION_PROXY_LIST_USER_DEVICES, HubFederation),
    daemon_rpc!(ABILITY_FEDERATION_REVOKE, HubFederation),
    daemon_rpc!(ABILITY_IDENTITY_REGISTER_PUBKEY, HubIdentity),
    daemon_rpc!(ABILITY_IDENTITY_LIST_USER_PUBKEYS, HubIdentity),
    daemon_rpc!(ABILITY_IDENTITY_REVOKE_USER_PUBKEY, HubIdentity),
    daemon_rpc!(ABILITY_PRINCIPAL_CREATE, HubIdentity),
    daemon_rpc!(ABILITY_PRINCIPAL_BIND_FIRST_KEY, HubIdentity),
    daemon_rpc!(ABILITY_PRINCIPAL_ADD_KEY, HubIdentity),
    daemon_rpc!(ABILITY_PRINCIPAL_ROTATE_KEY, HubIdentity),
    daemon_rpc!(ABILITY_PRINCIPAL_REVOKE_KEY, HubIdentity),
    daemon_rpc!(ABILITY_PRINCIPAL_CONFIGURE_RECOVERY, HubIdentity),
    daemon_rpc!(ABILITY_PRINCIPAL_RECOVER, HubIdentity),
    daemon_rpc!(ABILITY_PRINCIPAL_SUSPEND, HubIdentity),
    daemon_rpc!(ABILITY_PRINCIPAL_REACTIVATE, HubIdentity),
    daemon_rpc!(ABILITY_PRINCIPAL_DELETE, HubIdentity),
    daemon_rpc!(ABILITY_PRINCIPAL_ISSUE_ENROLLMENT, HubIdentity),
    daemon_rpc!(ABILITY_PRINCIPAL_REVOKE_ENROLLMENT, HubIdentity),
    daemon_rpc!(ABILITY_PRINCIPAL_ISSUE_GRANT, HubIdentity),
    daemon_rpc!(ABILITY_PRINCIPAL_REVOKE_GRANT, HubIdentity),
    daemon_rpc!(ABILITY_PRINCIPAL_GET, HubIdentity),
    runtime_admin_rpc!(ABILITY_RUNTIME_BOOTSTRAP_SELF_IDENTITY, HubRuntimeAdmin),
    // `session.open` rides the daemon bidi carrier (`InvokeBidi`), not the local registry or the unary/stream
    // Invocation route table. They are prefix-bypassed in dispatch (SPEC
    // §9.1 item 13) and verified against the installed runtime-admin
    // surface by `RuntimeAdminConformance`, never by `RegistryConformance`
    // or `DaemonInvocationSurface`. SPEC §7.1 notes 6/7 fix their owner as
    // the daemon runtime, not an EasyNet backend wrapper.
    runtime_admin_bidi!(ABILITY_SESSION_OPEN, HubRuntimeAdmin),
    local_rpc!(ABILITY_META_LIST_ABILITIES, HubIntrospection),
    daemon_rpc!(ABILITY_FEDERATION_STATUS, HubFederation),
];

const DEVICE_BASELINE: &[BaselineAbility] = &[
    local_rpc!("observe.health", DeviceHealth),
    local_rpc!("observe.network_health", DeviceHealth),
    local_rpc!("admin.status", DeviceHealth),
    local_rpc!("meta.describe", DeviceHealth),
    local_rpc!("meta.list_abilities", DeviceHealth),
    local_rpc!("meta.list_resources", DeviceHealth),
    local_rpc!("node.list", DeviceLifecycle),
    local_rpc!("node.describe", DeviceLifecycle),
    local_rpc!("node.remove", DeviceLifecycle),
    local_rpc!("ability.deploy", DeviceLifecycle),
    local_rpc!("ability.uninstall", DeviceLifecycle),
    local_rpc!("ability.publish", DeviceLifecycle),
    local_rpc!("ability.unpublish", DeviceLifecycle),
    local_rpc!("fs.read", DeviceLocomotion),
    local_rpc!("fs.write", DeviceLocomotion),
    local_rpc!("fs.stat", DeviceLocomotion),
    local_rpc!("fs.list", DeviceLocomotion),
    local_rpc!("fs.edit", DeviceLocomotion),
    local_rpc!("process.exec", DeviceLocomotion),
    local_rpc!("shell.run", DeviceLocomotion),
    local_rpc!("http.request", DeviceLocomotion),
    local_bidi!("fs.transfer", DeviceTransfer),
    local_rpc!("terminal.create", DeviceTerminal),
    local_rpc!("terminal.list", DeviceTerminal),
    local_bidi!("terminal.attach", DeviceTerminal),
    local_rpc!("terminal.input", DeviceTerminal),
    local_rpc!("terminal.read", DeviceTerminal),
    local_rpc!("terminal.resize", DeviceTerminal),
    local_rpc!("terminal.close", DeviceTerminal),
    local_rpc!(ABILITY_SESSION_LIST, DeviceSession),
    local_stream!("session.attach", DeviceSession),
    local_stream!("consent.subscribe", DeviceConsent),
    local_rpc!("consent.decide", DeviceConsent),
    local_rpc!("consent.list_pending", DeviceConsent),
    local_rpc!("agent.list", DeviceAgent),
    local_rpc!("agent.start", DeviceAgent),
    local_rpc!("agent.stop", DeviceAgent),
    local_rpc!("agent.purge", DeviceAgent),
    local_rpc!("agent.refresh", DeviceAgent),
    local_rpc!("agent.ability.put", DeviceAgent),
    local_rpc!("chat.history.list", DeviceAgent),
    local_rpc!("chat.history.get", DeviceAgent),
    local_rpc!("skill.publish", DeviceSkill),
    local_rpc!("skill.unpublish", DeviceSkill),
    local_rpc!("skill.list", DeviceSkill),
    local_rpc!("skill.tree", DeviceSkill),
    local_rpc!("skill.read_file", DeviceSkill),
    local_rpc!("skill.write_file", DeviceSkill),
    local_rpc!("skill.install", DeviceSkill),
    local_rpc!("skill.remove", DeviceSkill),
    local_rpc!("skill.upgrade", DeviceSkill),
    local_rpc!("mcp.bridge.list_tools", DeviceBridge),
    local_rpc!("mcp.bridge.call_tool", DeviceBridge),
    local_rpc!("mcp.client.list", DeviceBridge),
    local_rpc!("mcp.client.call", DeviceBridge),
    local_rpc!("a2a.bridge.list_skills", DeviceBridge),
    local_rpc!("a2a.bridge.send_task", DeviceBridge),
    local_rpc!("a2a.client.send_task", DeviceBridge),
    local_rpc!("mission.run", DeviceOrchestration),
    local_rpc!("mission.track", DeviceOrchestration),
    local_rpc!("mission.cancel", DeviceOrchestration),
    local_rpc!("mission.think", DeviceOrchestration),
    local_rpc!("mission.discuss_round", DeviceOrchestration),
    local_rpc!("discuss.create", DeviceOrchestration),
    local_rpc!("discuss.post", DeviceOrchestration),
    local_stream!("discuss.subscribe", DeviceOrchestration),
    local_rpc!("discuss.list_turns", DeviceOrchestration),
    local_rpc!("loop.create", DeviceOrchestration),
    local_rpc!("loop.status", DeviceOrchestration),
    local_stream!("loop.subscribe", DeviceOrchestration),
    local_rpc!("loop.cancel", DeviceOrchestration),
    local_rpc!("schedule.add", DeviceOrchestration),
    local_rpc!("schedule.list", DeviceOrchestration),
    local_rpc!("schedule.remove", DeviceOrchestration),
    local_rpc!("schedule.enable", DeviceOrchestration),
    local_rpc!("context.clipboard.list", DeviceContext),
    local_rpc!("context.clipboard.get", DeviceContext),
    local_rpc!("context.clipboard.track", DeviceContext),
    local_rpc!("context.clipboard.remove", DeviceContext),
    local_rpc!("context.catalog", DeviceContext),
    local_rpc!("context.folders.list", DeviceContext),
    local_rpc!("context.fs.list", DeviceContext),
    local_rpc!("context.favorites.list", DeviceContext),
    local_rpc!("context.favorites.add", DeviceContext),
    local_rpc!("context.favorites.remove", DeviceContext),
    local_rpc!("context.captures.list", DeviceContext),
    local_rpc!("context.captures.get", DeviceContext),
    local_stream!("mic.subscribe", DeviceMedia),
    local_stream!("camera.subscribe", DeviceMedia),
    local_rpc!("camera.snapshot", DeviceMedia),
    local_stream!("screen.subscribe", DeviceMedia),
    local_rpc!("screen.snapshot", DeviceMedia),
    local_bidi!("speaker.publish", DeviceMedia),
    local_rpc!("openai.chat_completions", DeviceOpenAiCompat),
    local_rpc!("openai.list_models", DeviceOpenAiCompat),
    local_rpc!("openai.files.upload", DeviceOpenAiCompat),
    local_rpc!("openai.files.retrieve", DeviceOpenAiCompat),
    local_rpc!("openai.files.delete", DeviceOpenAiCompat),
];

#[cfg(feature = "remote-desktop")]
const DEVICE_REMOTE_DESKTOP_BASELINE: &[BaselineAbility] = &[
    local_rpc!("remote_desktop.create_session", DeviceRemoteDesktop),
    local_rpc!("remote_desktop.show_session", DeviceRemoteDesktop),
    local_rpc!("remote_desktop.set_description", DeviceRemoteDesktop),
    local_rpc!("remote_desktop.add_ice_candidate", DeviceRemoteDesktop),
    local_stream!("remote_desktop.watch_events", DeviceRemoteDesktop),
    local_rpc!("remote_desktop.refresh_lease", DeviceRemoteDesktop),
    local_rpc!("remote_desktop.end_session", DeviceRemoteDesktop),
    local_bidi!("remote_desktop.attach", DeviceRemoteDesktop),
    local_rpc!("remote_desktop.permission_status", DeviceRemoteDesktop),
    local_rpc!("remote_desktop.request_permission", DeviceRemoteDesktop),
];

pub struct HubBaseline;

impl HubBaseline {
    #[must_use]
    pub const fn required_abilities() -> &'static [BaselineAbility] {
        HUB_BASELINE
    }
}

pub struct DeviceBaseline;

impl DeviceBaseline {
    #[must_use]
    pub fn required_abilities() -> Vec<BaselineAbility> {
        #[cfg(feature = "remote-desktop")]
        {
            let mut abilities = DEVICE_BASELINE.to_vec();
            abilities.extend_from_slice(DEVICE_REMOTE_DESKTOP_BASELINE);
            abilities
        }
        #[cfg(not(feature = "remote-desktop"))]
        {
            DEVICE_BASELINE.to_vec()
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct BaselineConformanceReport {
    profile: &'static str,
    surface: BaselineSurface,
    missing: Vec<BaselineAbility>,
}

impl BaselineConformanceReport {
    #[must_use]
    pub fn new(
        profile: &'static str,
        surface: BaselineSurface,
        missing: Vec<BaselineAbility>,
    ) -> Self {
        Self {
            profile,
            surface,
            missing,
        }
    }

    #[must_use]
    pub fn is_conformant(&self) -> bool {
        self.missing.is_empty()
    }

    #[must_use]
    pub fn panic_message(&self) -> String {
        let missing: Vec<&str> = self.missing.iter().map(|ability| ability.name).collect();
        format!(
            "{} baseline missing {:?} abilities: {missing:?}",
            self.profile, self.surface
        )
    }
}

pub struct RegistryConformance<'a> {
    registry: &'a crate::daemon::ability::dispatch::AxonAbilityCatalog,
}

impl<'a> RegistryConformance<'a> {
    #[must_use]
    pub fn new(registry: &'a crate::daemon::ability::dispatch::AxonAbilityCatalog) -> Self {
        Self { registry }
    }

    #[must_use]
    pub fn check(
        &self,
        profile: &'static str,
        abilities: &[BaselineAbility],
    ) -> BaselineConformanceReport {
        let missing = abilities
            .iter()
            .copied()
            .filter(|ability| ability.surface == BaselineSurface::LocalRegistry)
            .filter(|ability| !registry_supports(self.registry, *ability))
            .collect();
        BaselineConformanceReport::new(profile, BaselineSurface::LocalRegistry, missing)
    }
}

pub struct DaemonInvocationSurface {
    routes: BTreeSet<(&'static str, CallMode)>,
}

impl DaemonInvocationSurface {
    #[must_use]
    pub fn new(routes: impl IntoIterator<Item = (&'static str, CallMode)>) -> Self {
        Self {
            routes: routes.into_iter().collect(),
        }
    }

    /// Build from the daemon's production route surface: the exported
    /// exact-match unary and server-stream tables that live beside the tonic
    /// dispatcher match arms. This is the constructor tests should use; it
    /// keeps conformance from becoming a second hand-maintained route list.
    #[must_use]
    #[cfg(feature = "axon-pb")]
    pub fn from_daemon_surface() -> Self {
        Self::new(
            crate::daemon::invocation::dispatch::daemon_invocation_service::DAEMON_INVOCATION_UNARY_ROUTES
                .iter()
                .map(|route| (route.name(), route.call_mode()))
                .chain(
                    crate::daemon::invocation::dispatch::daemon_invocation_service::DAEMON_INVOCATION_STREAM_ROUTES
                        .iter()
                        .map(|route| (route.name(), route.call_mode())),
                ),
        )
    }

    #[must_use]
    pub fn check(
        &self,
        profile: &'static str,
        abilities: &[BaselineAbility],
    ) -> BaselineConformanceReport {
        let missing = abilities
            .iter()
            .copied()
            .filter(|ability| ability.surface == BaselineSurface::DaemonInvocation)
            .filter(|ability| !self.routes.contains(&(ability.name, ability.call_mode)))
            .collect();
        BaselineConformanceReport::new(profile, BaselineSurface::DaemonInvocation, missing)
    }
}

/// Verifies that `AxonRuntimeAdmin` baseline rows are actually installed
/// on the daemon runtime-admin surface — the named `InvokeBidi` route set
/// the bidi dispatcher serves (`session.open`)
/// plus any RPC-shaped runtime-admin handshakes installed on the Axon
/// `LocalRuntime` (`runtime.bootstrap_self_identity`).
///
/// SPEC §7.3 item 7: a missing runtime-admin ability must surface as a
/// conformance failure here, never be papered over by a CLI wrapper
/// faking success. The installed route set is supplied by the production
/// dispatcher (`bidi_dispatcher::RUNTIME_ADMIN_BIDI_ROUTES`) and the
/// runtime-admin RPC probe, so this model never hand-mirrors a second
/// route list (SPEC §7.3 item 5).
pub struct RuntimeAdminConformance {
    installed: BTreeSet<&'static str>,
}

impl RuntimeAdminConformance {
    #[must_use]
    pub fn new(installed: impl IntoIterator<Item = &'static str>) -> Self {
        Self {
            installed: installed.into_iter().collect(),
        }
    }

    /// Build the conformance checker from the daemon's *actual* installed
    /// runtime-admin surface: the bidi dispatcher's named route table
    /// (`session.open`) plus the runtime-admin
    /// RPC handshake (`runtime.bootstrap_self_identity`).
    ///
    /// This is the production-derived constructor — callers (boot gate,
    /// CI conformance gate) consume it instead of retyping the installed
    /// set, so there is no second hand-mirrored route list (SPEC §7.3
    /// item 5). The dispatcher `match` arms reference the same constants
    /// that feed `RUNTIME_ADMIN_BIDI_ROUTES`, so the installed set cannot
    /// silently drift from what the daemon actually serves.
    #[must_use]
    #[cfg(feature = "axon-pb")]
    pub fn from_daemon_surface() -> Self {
        Self::new(
            crate::daemon::invocation::bidi::bidi_dispatcher::RUNTIME_ADMIN_BIDI_ROUTES
                .iter()
                .map(|route| route.name())
                .chain(std::iter::once(ABILITY_RUNTIME_BOOTSTRAP_SELF_IDENTITY)),
        )
    }

    #[must_use]
    pub fn check(
        &self,
        profile: &'static str,
        abilities: &[BaselineAbility],
    ) -> BaselineConformanceReport {
        let missing = abilities
            .iter()
            .copied()
            .filter(|ability| ability.surface == BaselineSurface::AxonRuntimeAdmin)
            .filter(|ability| !self.installed.contains(ability.name))
            .collect();
        BaselineConformanceReport::new(profile, BaselineSurface::AxonRuntimeAdmin, missing)
    }
}

#[must_use]
pub fn duplicate_ability_names(abilities: &[BaselineAbility]) -> Vec<&'static str> {
    let mut seen = BTreeSet::new();
    let mut dupes = BTreeSet::new();
    for ability in abilities {
        if !seen.insert(ability.name) {
            dupes.insert(ability.name);
        }
    }
    dupes.into_iter().collect()
}

#[must_use]
pub fn baseline_names(abilities: &[BaselineAbility]) -> BTreeSet<&'static str> {
    abilities.iter().map(|ability| ability.name).collect()
}

fn registry_supports(
    registry: &crate::daemon::ability::dispatch::AxonAbilityCatalog,
    ability: BaselineAbility,
) -> bool {
    match ability.call_mode {
        CallMode::Rpc => registry.has_rpc(ability.name),
        CallMode::Stream => registry.has_stream(ability.name),
        CallMode::Bidi => registry.has_bidi(ability.name),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn hub_baseline_includes_status_and_excludes_backend_aggregate_alias() {
        let names = baseline_names(HubBaseline::required_abilities());
        assert!(names.contains(ABILITY_FEDERATION_STATUS));
        assert!(
            !names.contains(FORBIDDEN_BACKEND_AGGREGATE_ALIAS),
            "backend/product aggregate alias must not become canonical daemon baseline"
        );
    }

    #[test]
    fn voice_capability_state_evidence_is_honest_about_provider_boundaries() {
        let operational = baseline_names(HubBaseline::required_abilities());
        let expected = [
            ("voice.create_call", CallMode::Rpc, CapabilityState::Seam),
            ("voice.show_call", CallMode::Rpc, CapabilityState::Seam),
            ("voice.join_call", CallMode::Rpc, CapabilityState::Seam),
            ("voice.leave_call", CallMode::Rpc, CapabilityState::Seam),
            ("voice.end_call", CallMode::Rpc, CapabilityState::Seam),
            ("voice.watch_call", CallMode::Rpc, CapabilityState::Seam),
            ("voice.report_metrics", CallMode::Rpc, CapabilityState::Seam),
            ("voice.list_calls", CallMode::Rpc, CapabilityState::Seam),
            (
                "voice.subscribe",
                CallMode::Stream,
                CapabilityState::Unsupported,
            ),
            (
                "voice.transcribe",
                CallMode::Bidi,
                CapabilityState::Unsupported,
            ),
        ];

        let evidence = voice_capability_state_evidence(VoiceAssemblyEvidence::default());
        assert_eq!(evidence.len(), expected.len());
        for (evidence, (name, mode, state)) in evidence.iter().zip(expected) {
            assert_eq!(
                (evidence.name, evidence.call_mode, evidence.state),
                (name, mode, state)
            );
            assert_eq!(evidence.authority, "Hub");
            assert!(!evidence.reason.is_empty());
            assert!(
                !operational.contains(evidence.name),
                "{} must stay out of the operational Hub baseline",
                evidence.name
            );
        }
    }

    #[test]
    fn voice_capability_state_advances_only_with_assembly_evidence() {
        let provider_backed = voice_capability_state_evidence(VoiceAssemblyEvidence {
            repository_assembled: true,
            executable_delivery_evidence: false,
        });
        assert!(provider_backed[..VOICE_SIGNALING_CONTRACTS.len()]
            .iter()
            .all(|row| row.state == CapabilityState::ProviderBacked));

        let cutover_ready = voice_capability_state_evidence(VoiceAssemblyEvidence {
            repository_assembled: true,
            executable_delivery_evidence: true,
        });
        assert!(cutover_ready[..VOICE_SIGNALING_CONTRACTS.len()]
            .iter()
            .all(|row| row.state == CapabilityState::CutoverReady));
    }

    #[test]
    fn baseline_lists_do_not_duplicate_ability_names() {
        assert_eq!(
            duplicate_ability_names(HubBaseline::required_abilities()),
            Vec::<&str>::new()
        );

        let device = DeviceBaseline::required_abilities();
        assert_eq!(duplicate_ability_names(&device), Vec::<&str>::new());
    }

    #[test]
    fn principal_route_bindings_are_generated_from_manifest() {
        use sha2::Digest as _;

        let manifest = std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
            .join("provider_routes/runtime-principal-lifecycle-routes.v1.json");
        let digest = sha2::Sha256::digest(std::fs::read(manifest).expect("read manifest"));

        assert_eq!(
            crate::daemon::ability::principal_routes_gen::PRINCIPAL_ROUTE_MANIFEST_SHA256,
            hex::encode(digest)
        );
        assert_eq!(
            crate::daemon::ability::principal_routes_gen::PRINCIPAL_LIFECYCLE_PROFILE,
            "principal_lifecycle"
        );
        assert_eq!(
            ABILITY_PRINCIPAL_CREATE,
            crate::daemon::ability::principal_routes_gen::ABILITY_PRINCIPAL_CREATE
        );
    }

    #[test]
    fn runtime_admin_routes_are_generated_from_manifest() {
        use sha2::Digest as _;

        let manifest = std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
            .join("provider_routes/runtime-admin-routes.v1.json");
        let digest = sha2::Sha256::digest(std::fs::read(manifest).expect("read manifest"));

        assert_eq!(
            crate::daemon::ability::runtime_admin_routes_gen::RUNTIME_ADMIN_ROUTE_MANIFEST_SHA256,
            hex::encode(digest)
        );
        assert_eq!(
            crate::daemon::ability::runtime_admin_routes_gen::RUNTIME_ADMIN_PROFILE,
            "runtime_admin"
        );
        assert_eq!(
            ABILITY_SESSION_LIST,
            crate::daemon::ability::runtime_admin_routes_gen::SESSION_LIST
        );
        assert_eq!(
            ABILITY_FEDERATION_REVOKE,
            crate::daemon::ability::runtime_admin_routes_gen::FEDERATION_REVOKE
        );
    }

    #[test]
    fn openai_compat_stays_device_owned() {
        let device = DeviceBaseline::required_abilities();
        let names = baseline_names(&device);
        assert!(names.contains("openai.chat_completions"));
        assert!(names.contains("openai.list_models"));
        assert!(
            !names.iter().any(|name| name.starts_with("hub.openai.")),
            "hub-owned OpenAI gateway needs an explicit future prefix, not this compatibility pair"
        );
    }

    #[test]
    fn local_registry_satisfies_device_baseline() {
        let _home = crate::cli::commands::test_support::HomeGuard::new();
        let registry = crate::daemon::ability::catalog::build_registry();
        let device = DeviceBaseline::required_abilities();
        let report = RegistryConformance::new(&registry).check("device", &device);
        assert!(report.is_conformant(), "{}", report.panic_message());
    }

    #[test]
    fn local_registry_satisfies_hub_introspection_slice() {
        let _home = crate::cli::commands::test_support::HomeGuard::new();
        let hub_ura = crate::core::ura::hub_ura("conformance-test");
        let authority_context =
            crate::daemon::ability::dispatch::AbilityAuthorityContext::for_hub_authority_root(
                &hub_ura,
            )
            .expect("Hub authority context");
        let registry =
            crate::daemon::ability::catalog::build_registry_snapshot_with_authority_context(
                authority_context,
            )
            .expect("build Hub registry snapshot");
        let report =
            RegistryConformance::new(&registry).check("hub", HubBaseline::required_abilities());
        assert!(report.is_conformant(), "{}", report.panic_message());
    }

    #[cfg(feature = "axon-pb")]
    #[test]
    fn daemon_invocation_surface_satisfies_hub_baseline() {
        let report = DaemonInvocationSurface::from_daemon_surface()
            .check("hub", HubBaseline::required_abilities());
        assert!(report.is_conformant(), "{}", report.panic_message());
    }

    #[cfg(feature = "axon-pb")]
    #[test]
    fn daemon_invocation_surface_detects_missing_route() {
        let installed_without_identity_register =
            crate::daemon::invocation::dispatch::daemon_invocation_service::DAEMON_INVOCATION_UNARY_ROUTES
                .iter()
                .filter(|route| route.name() != ABILITY_IDENTITY_REGISTER_PUBKEY)
                .map(|route| (route.name(), route.call_mode()))
                .chain(
                    crate::daemon::invocation::dispatch::daemon_invocation_service::DAEMON_INVOCATION_STREAM_ROUTES
                        .iter()
                        .map(|route| (route.name(), route.call_mode())),
                );
        let report = DaemonInvocationSurface::new(installed_without_identity_register)
            .check("hub", HubBaseline::required_abilities());
        assert!(!report.is_conformant());
        assert!(
            report
                .panic_message()
                .contains(ABILITY_IDENTITY_REGISTER_PUBKEY),
            "missing report must name the absent route: {}",
            report.panic_message()
        );
    }

    #[cfg(feature = "axon-pb")]
    #[test]
    fn daemon_invocation_route_tables_are_classified_by_dispatchers() {
        for route in
            crate::daemon::invocation::dispatch::daemon_invocation_service::DAEMON_INVOCATION_UNARY_ROUTES
        {
            assert_eq!(
                crate::daemon::invocation::dispatch::daemon_invocation_service::DaemonUnaryRoute::from_function(route.name()),
                Some(*route)
            );
        }

        for route in
            crate::daemon::invocation::dispatch::daemon_invocation_service::DAEMON_INVOCATION_STREAM_ROUTES
        {
            assert_eq!(
                crate::daemon::invocation::dispatch::daemon_invocation_service::DaemonStreamRoute::from_function(route.name()),
                Some(*route)
            );
        }

        for route in
            crate::daemon::invocation::dispatch::daemon_invocation_service::DAEMON_INVOCATION_BIDI_ROUTES
        {
            assert_eq!(
                crate::daemon::invocation::dispatch::daemon_invocation_service::DaemonBidiRoute::from_function(route.name()),
                Some(*route)
            );
            assert_eq!(route.call_mode(), CallMode::Bidi);
        }
    }

    #[cfg(feature = "axon-pb")]
    #[test]
    fn hub_baseline_runtime_admin_rows_are_installed_on_daemon_surface() {
        // The installed runtime-admin surface is the production bidi
        // dispatcher route table plus the runtime-admin RPC handshake.
        // We derive it from the production constant rather than retyping a
        // second list (SPEC §7.3 item 5), so a baseline row that names an
        // `AxonRuntimeAdmin` ability the dispatcher does not actually route
        // fails here instead of being silently faked (SPEC §7.3 item 7).
        let report = RuntimeAdminConformance::from_daemon_surface()
            .check("hub", HubBaseline::required_abilities());
        assert!(report.is_conformant(), "{}", report.panic_message());
    }

    #[test]
    fn runtime_admin_conformance_detects_missing_installation() {
        // Negative test: drop `session.open` from the installed set and
        // prove the report flags it. This pins the failure semantics so a
        // future regression that silently removes the dispatcher arm is
        // caught rather than passing on an empty `missing` list.
        let installed = vec![ABILITY_RUNTIME_BOOTSTRAP_SELF_IDENTITY];
        let report =
            RuntimeAdminConformance::new(installed).check("hub", HubBaseline::required_abilities());
        assert!(!report.is_conformant());
        assert!(
            report.panic_message().contains(ABILITY_SESSION_OPEN),
            "missing report must name the absent ability: {}",
            report.panic_message()
        );
    }

    #[test]
    fn session_open_is_a_runtime_admin_bidi_row() {
        // Pin the surface/call-mode classification: this carrier rides
        // the daemon bidi surface, not the local registry. If a refactor
        // ever reclassifies them as `LocalRegistry`, `RegistryConformance`
        // would start asserting a handler that does not exist.
        let hub = HubBaseline::required_abilities();
        let row = hub
            .iter()
            .find(|ability| ability.name == ABILITY_SESSION_OPEN)
            .expect("hub baseline must contain ability.session.open");
        assert_eq!(row.surface, BaselineSurface::AxonRuntimeAdmin);
        assert_eq!(row.call_mode, CallMode::Bidi);
        assert_eq!(row.domain, BaselineDomain::HubRuntimeAdmin);
    }
}
