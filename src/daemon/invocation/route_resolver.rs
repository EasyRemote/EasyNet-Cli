// EasyNet CLI — invocation_transport — daemon route resolver
// ==========================================================
//
// File: src/daemon/invocation/route_resolver.rs
// Description: Daemon-owned RFC-005 route resolver facade.
//
// Protocol Responsibility:
// - Axon owns ResolveQuery / ResolveAnswer / NextHop / NegativeReason.
// - The daemon owns the local runtime facts used to select an executable route:
//   live device sessions, hosted-agent placement, and owner ability projection.
//
// Implementation Approach:
// - Resolve once into `SelectedInvokeRoute`.
// - Project that selection into Axon proto-JSON for `namespace.resolve`.
// - Reuse the same selected route for invoke dispatch; callers must not rebuild
//   callee/dispatch identity from `target_ura + ability_name`.
//
// Usage Contract:
// - A route may dispatch only when `release_profile >= AuthoritativeLocal` and
//   `answer_kind == FinalRoute`.
// - Negative answers are typed; no legacy catalog fallback is consulted.
//
// Architectural Position:
// - CLI daemon runtime layer. Backend and frontend consume this through daemon
//   Invocation, never as an in-process library.

use std::collections::{HashMap, HashSet};

use serde_json::{json, Value};

use crate::daemon::invocation::federation_wrappers::{self, ResolveAgentSummary, ResolveRequest};
use crate::daemon::invocation::hub_resolver::{HubResolution, HubResolver};
use crate::services::ability_catalog_store::AbilityCatalogStore;
use crate::services::advertised_agent_store::AdvertisedAgentStore;
use crate::services::federated_peers_cell::SharedFederatedPeers;
use crate::services::federation_directory::SharedFederatedDirectoryView;
use crate::services::presence_registry::PresenceRegistry;

use easynet_axon::pb::axon::v1 as axon_pb;

/// Local runtime namespace authority (RFC-005 §4 / D105).
///
/// A daemon is the authority for the abilities it can execute in its own
/// `LocalRuntime`: its device-owned control-plane abilities and the
/// owner-local abilities of agents hosted by this device. Their `ABILITY`
/// existence and executable `ROUTE` are proven from live local dispatch
/// bindings, never from the hub projection cache. The hub
/// `AbilityCatalogStore` is a signed, lease-bound rendezvous projection
/// for *other* consumers (peer devices, backend discovery) — it is not
/// consulted when this daemon resolves its own executable surface.
///
/// This trait is the dependency-injection seam between the synchronous
/// resolver and the asynchronous `LocalRuntime`. Implementors snapshot
/// the runtime's registered abilities before the resolver runs (the
/// resolver call sites are already async) and answer membership
/// synchronously.
pub(crate) trait LocalRuntimeAuthority: Send + Sync {
    /// Resolve an owner-local ability by public name against the live
    /// local dispatch bindings.
    ///
    /// `owner_ura` is the canonical owner whose ability is being
    /// selected. For device control-plane abilities it equals the daemon
    /// device URA; for hosted agents it is the hosted Agent URA while the
    /// selected execution host remains the daemon device.
    /// `public_name` is the owner-local ability name (e.g. `agent.start`,
    /// `discover`, `fs.read`). Returns the runtime dispatch binding when
    /// the local runtime actually registers a dispatchable route for it
    /// (the D105 ROUTE gate), or `None` when it does not — which the
    /// resolver maps to a typed `NODATA` negative.
    fn resolve_owner_ability(
        &self,
        owner_ura: &str,
        public_name: &str,
    ) -> Option<LocalRuntimeAbility>;
}

/// An ability proven from the local runtime dispatch table.
///
/// `dispatch_name` is the implementation-local registry key the runtime
/// consumes (e.g. `claude.chat`, `fs.read`); it is intentionally distinct
/// from the canonical public name so the resolver keeps `dispatch_name`
/// authoritative for local execution while the ability URA stays
/// owner-canonical.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct LocalRuntimeAbility {
    pub dispatch_name: String,
}

/// `LocalRuntimeAuthority` backed by a snapshot of the live `LocalRuntime`
/// dispatch table.
///
/// The daemon takes this snapshot at the (already async) resolver call
/// sites via [`LocalRuntimeAuthoritySnapshot::capture`], then hands
/// ownership to the synchronous resolver. Membership is exact: a public
/// name resolves only when the runtime registers the matching local
/// dispatch key, which is the literal D105 "owner device has a matching
/// runtime-local dispatch binding" gate.
#[derive(Debug, Clone, Default)]
pub(crate) struct LocalRuntimeAuthoritySnapshot {
    ability_uras: HashSet<String>,
}

impl LocalRuntimeAuthoritySnapshot {
    /// Snapshot every registered dispatch key from the local runtime.
    pub(crate) async fn capture(runtime: &easynet_axon::invocation::LocalRuntime) -> Self {
        let ability_uras = runtime
            .list_abilities()
            .await
            .into_iter()
            .map(|descriptor| descriptor.name)
            .collect();
        Self { ability_uras }
    }
}

impl LocalRuntimeAuthority for LocalRuntimeAuthoritySnapshot {
    fn resolve_owner_ability(
        &self,
        owner_ura: &str,
        public_name: &str,
    ) -> Option<LocalRuntimeAbility> {
        let runtime_key = crate::ura::owner_ability_ura(owner_ura, public_name)?;
        if !self.ability_uras.contains(&runtime_key) {
            return None;
        }
        let dispatch_name = crate::ura::local_dispatch_ability_key(owner_ura, public_name);
        if dispatch_name.is_empty() {
            return None;
        }
        Some(LocalRuntimeAbility { dispatch_name })
    }
}

/// Closed daemon-internal route-locality classification.
///
/// This is the semantic value dispatchers should consume. Axon
/// `RouteReason` remains the wire projection exposed through
/// ResolveAnswer JSON, but the daemon must not infer route locality
/// from protobuf strings, owner URA shape, or ad hoc dispatcher branches.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum SelectedRouteKind {
    LocalDevice,
    SameRealmDevice,
    HubOwned,
    HostedAgent,
}

impl SelectedRouteKind {
    #[must_use]
    pub(crate) fn route_reason(self) -> axon_pb::RouteReason {
        match self {
            Self::LocalDevice | Self::SameRealmDevice => axon_pb::RouteReason::LocalDevice,
            Self::HubOwned => axon_pb::RouteReason::LocalHub,
            Self::HostedAgent => axon_pb::RouteReason::HostedAgent,
        }
    }
}

/// Dispatcher-facing execution target for a selected route.
///
/// `SelectedRouteKind` answers *what locality was selected*; this target
/// answers *which daemon-owned execution plane must consume it*. The only
/// extra input is whether the selected execution host is this daemon, which is
/// a process-local fact supplied by `TargetGate`.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum SelectedRouteDispatchTarget {
    LocalRuntime,
    PresenceSession,
}

impl SelectedRouteDispatchTarget {
    #[must_use]
    pub(crate) fn is_local_runtime(self) -> bool {
        matches!(self, Self::LocalRuntime)
    }

    #[must_use]
    pub(crate) fn is_presence_session(self) -> bool {
        matches!(self, Self::PresenceSession)
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct SelectedInvokeRoute {
    pub query_name: String,
    pub owner_ura: String,
    pub callee_ura: String,
    pub execution_host_ura: String,
    pub host_node_id: Option<String>,
    pub ability_ura: String,
    pub route_ura: String,
    pub dispatch_name: String,
    pub release_profile: axon_pb::ResolverReleaseProfile,
    kind: SelectedRouteKind,
    ability_record: Value,
    route_record: Value,
    owner_record: Value,
}

impl SelectedInvokeRoute {
    #[must_use]
    pub(crate) fn is_authoritative_local_or_better(&self) -> bool {
        matches!(
            self.release_profile,
            axon_pb::ResolverReleaseProfile::AuthoritativeLocal
                | axon_pb::ResolverReleaseProfile::Production
        )
    }

    #[must_use]
    pub(crate) fn dispatch_key(&self) -> String {
        crate::ura::local_dispatch_ability_key(&self.callee_ura, &self.dispatch_name)
    }

    #[must_use]
    pub(crate) fn kind(&self) -> SelectedRouteKind {
        self.kind
    }

    #[must_use]
    pub(crate) fn route_reason(&self) -> axon_pb::RouteReason {
        self.kind().route_reason()
    }

    #[must_use]
    pub(crate) fn dispatch_target(
        &self,
        execution_host_is_self: bool,
    ) -> SelectedRouteDispatchTarget {
        match self.kind {
            SelectedRouteKind::LocalDevice => SelectedRouteDispatchTarget::LocalRuntime,
            SelectedRouteKind::SameRealmDevice
            | SelectedRouteKind::HubOwned
            | SelectedRouteKind::HostedAgent => {
                if execution_host_is_self {
                    SelectedRouteDispatchTarget::LocalRuntime
                } else {
                    SelectedRouteDispatchTarget::PresenceSession
                }
            }
        }
    }

    #[must_use]
    pub(crate) fn final_route_answer_json(&self) -> Value {
        let authority = authority_for_query(&self.query_name);
        let gates = json!({
            "authority": axon_pb::GateResult::Pass.as_str_name(),
            "identity": axon_pb::GateResult::Pass.as_str_name(),
            "placement": axon_pb::GateResult::Pass.as_str_name(),
            "ability": axon_pb::GateResult::Pass.as_str_name(),
            "policy": axon_pb::GateResult::Pass.as_str_name(),
        });
        let next_hop = self.next_hop_json();
        let selected_route = json!({
            "nextHop": next_hop.clone(),
            "priority": 0,
            "weight": 1,
            "reason": self.route_reason().as_str_name(),
            "health": axon_pb::RouteHealth::Healthy.as_str_name(),
            "authority": authority.clone(),
            "gates": gates,
        });

        json!({
            "answerKind": axon_pb::ResolveAnswerKind::FinalRoute.as_str_name(),
            "canonicalName": self.query_name,
            "ownerUra": self.owner_ura,
            "abilityUra": self.ability_ura,
            "routeUra": self.route_ura,
            "nextHop": next_hop,
            "selectedRoute": selected_route.clone(),
            "routeCandidates": [selected_route],
            "routeEvidence": {
                "identity": self.owner_record.clone(),
                "owner": self.owner_record.clone(),
                "ability": self.ability_record.clone(),
                "route": self.route_record.clone(),
                "selectionAlgorithm": "daemon-selected-route-v1",
            },
            "records": [self.ability_record.clone(), self.route_record.clone()],
            "releaseProfile": self.release_profile.as_str_name(),
            "authority": authority,
            "cachePolicy": cache_policy_json(),
        })
    }

    fn next_hop_json(&self) -> Value {
        match self.kind {
            SelectedRouteKind::HubOwned => json!({
                "localHubAbility": {
                    "abilityUra": self.ability_ura,
                    "routeUra": self.route_ura,
                    "dispatchName": self.dispatch_name,
                }
            }),
            SelectedRouteKind::HostedAgent => json!({
                "hostedAgentViaDevice": {
                    "agentUra": self.callee_ura,
                    "hostDeviceUra": self.execution_host_ura,
                    "hostNodeId": self.host_node_id.as_deref().unwrap_or_default(),
                    "abilityUra": self.ability_ura,
                    "routeUra": self.route_ura,
                    "dispatchName": self.dispatch_name,
                }
            }),
            SelectedRouteKind::LocalDevice | SelectedRouteKind::SameRealmDevice => json!({
                "localDeviceAbility": {
                    "deviceUra": self.execution_host_ura,
                    "abilityUra": self.ability_ura,
                    "routeUra": self.route_ura,
                    "dispatchName": self.dispatch_name,
                }
            }),
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct DelegatedInvokeRoute {
    pub query_name: String,
    pub owner_ura: String,
    pub realm: String,
    pub hub_ura: String,
    pub endpoints: Vec<DelegatedPeerEndpoint>,
    pub release_profile: axon_pb::ResolverReleaseProfile,
}

/// Resolver-owned route selection for `federation.forward_invoke`.
///
/// Forward invoke is the only daemon surface that may either execute via a
/// local final route or delegate to a peer hub. Keeping that branch here
/// prevents dispatchers from parsing `target_ura` and re-implementing locality
/// policy outside the resolver.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) enum ForwardInvokeRouteSelection {
    Local(SelectedInvokeRoute),
    Peer(DelegatedInvokeRoute),
}

fn route_owner_mismatch_detail(
    selected_owner_ura: &str,
    ability_ura: &str,
    expected_target_ura: &str,
) -> String {
    format!(
        "namespace.resolve selected owner `{selected_owner_ura}` for ability `{ability_ura}` \
         but request target was `{expected_target_ura}`"
    )
}

impl DelegatedInvokeRoute {
    #[must_use]
    pub(crate) fn primary_endpoint(&self) -> Option<&str> {
        self.endpoints
            .iter()
            .min_by_key(|endpoint| endpoint.priority)
            .map(|endpoint| endpoint.endpoint.as_str())
    }

    #[must_use]
    pub(crate) fn delegation_answer_json(&self) -> Value {
        let authority = authority_for_query(&self.owner_ura);
        let next_hop = self.next_hop_json();
        let selected_route = json!({
            "nextHop": next_hop.clone(),
            "priority": 0,
            "weight": 1,
            "reason": axon_pb::RouteReason::PeerDelegation.as_str_name(),
            "health": axon_pb::RouteHealth::Healthy.as_str_name(),
            "authority": authority.clone(),
            "gates": {
                "authority": axon_pb::GateResult::Pass.as_str_name(),
                "identity": axon_pb::GateResult::NotApplicable.as_str_name(),
                "placement": axon_pb::GateResult::Pass.as_str_name(),
                "ability": axon_pb::GateResult::NotApplicable.as_str_name(),
                "policy": axon_pb::GateResult::Pass.as_str_name(),
            },
        });

        json!({
            "answerKind": axon_pb::ResolveAnswerKind::Delegation.as_str_name(),
            "canonicalName": self.query_name,
            "ownerUra": self.owner_ura,
            "nextHop": next_hop,
            "selectedRoute": selected_route.clone(),
            "routeCandidates": [selected_route],
            "routeEvidence": {
                "owner": {
                    "ura": self.owner_ura,
                    "realm": self.realm,
                },
                "route": {
                    "hubUra": self.hub_ura,
                    "endpoints": self.endpoints.iter().map(DelegatedPeerEndpoint::json).collect::<Vec<_>>(),
                },
                "selectionAlgorithm": "daemon-peer-delegation-v1",
            },
            "records": [],
            "releaseProfile": self.release_profile.as_str_name(),
            "authority": authority,
            "cachePolicy": cache_policy_json(),
        })
    }

    fn next_hop_json(&self) -> Value {
        json!({
            "peerHub": {
                "realm": self.realm,
                "hubUra": self.hub_ura,
                "endpoints": self.endpoints.iter().map(DelegatedPeerEndpoint::json).collect::<Vec<_>>(),
            }
        })
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct DelegatedPeerEndpoint {
    pub endpoint: String,
    pub protocol: String,
    pub priority: u32,
    pub weight: u32,
    pub metadata: serde_json::Map<String, Value>,
}

impl DelegatedPeerEndpoint {
    fn new(endpoint: String, source: &'static str, target_ura: Option<&str>) -> Self {
        let mut metadata = serde_json::Map::new();
        metadata.insert("source".to_string(), Value::String(source.to_string()));
        if let Some(target_ura) = target_ura {
            metadata.insert(
                "targetUra".to_string(),
                Value::String(target_ura.to_string()),
            );
        }
        Self {
            endpoint,
            protocol: "grpc".to_string(),
            priority: 0,
            weight: 1,
            metadata,
        }
    }

    fn json(&self) -> Value {
        json!({
            "endpoint": self.endpoint,
            "protocol": self.protocol,
            "priority": self.priority,
            "weight": self.weight,
            "metadata": self.metadata,
        })
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct ResolveRouteFailure {
    pub query_name: String,
    pub reason: axon_pb::NegativeReason,
    pub detail: String,
}

impl ResolveRouteFailure {
    #[must_use]
    pub(crate) fn answer_json(&self) -> Value {
        negative_answer_json(&self.query_name, self.reason, Some(self.detail.as_str()))
    }
}

pub(crate) struct DaemonRouteResolver<'a> {
    registry: &'a PresenceRegistry,
    advertised_agents: Option<&'a AdvertisedAgentStore>,
    catalog: Option<&'a AbilityCatalogStore>,
    peer_delegation: Option<PeerDelegationSource<'a>>,
    device_local: Option<LocalNamespaceAuthoritySource>,
    now_unix_ms: i64,
}

struct PeerDelegationSource<'a> {
    local_realm: &'a str,
    federated_peers: &'a SharedFederatedPeers,
    federated_directory: &'a SharedFederatedDirectoryView,
    allow_directory_auto_route: bool,
}

/// This daemon's own namespace authority, injected when the daemon
/// resolves a route for a local runtime owner. Device-mode daemons use
/// their device URA; hub-mode daemons without a paired device identity
/// use their canonical hub URA.
///
/// Owned by value (not borrowed) so the resolver is self-contained: the
/// daemon captures a runtime snapshot at the async call site and hands
/// ownership to the resolver, sidestepping a borrow that would otherwise
/// have to outlive the snapshot's stack frame.
struct LocalNamespaceAuthoritySource {
    local_authority_ura: String,
    authority: Box<dyn LocalRuntimeAuthority>,
    hosted_agents: LocalHostedAgentPlacements,
}

#[derive(Debug, Clone, PartialEq, Eq)]
struct HostedAgentPlacement {
    host_device_ura: String,
    host_node_id: Option<String>,
}

#[derive(Debug, Clone, Default, PartialEq, Eq)]
struct LocalHostedAgentPlacements {
    by_agent_ura: HashMap<String, HostedAgentPlacement>,
}

impl LocalHostedAgentPlacements {
    fn load() -> Self {
        crate::persistence::local_agents::load()
            .map(|file| Self::from_file(&file))
            .unwrap_or_default()
    }

    fn from_file(file: &crate::persistence::local_agents::LocalAgentsFile) -> Self {
        let host_device_ura = file.host_device_agent_ura.trim();
        if host_device_ura.is_empty() {
            return Self::default();
        }

        let host_node_id = device_id_from_device_ura(host_device_ura);
        let by_agent_ura = file
            .hosted_agents
            .iter()
            .filter_map(|entry| {
                let agent_ura = entry.agent_ura.trim();
                let parsed = crate::ura::parse_ura(agent_ura).ok()?;
                if parsed.kind != crate::ura::URAKind::Agent {
                    return None;
                }
                Some((
                    agent_ura.to_string(),
                    HostedAgentPlacement {
                        host_device_ura: host_device_ura.to_string(),
                        host_node_id: host_node_id.clone(),
                    },
                ))
            })
            .collect();
        Self { by_agent_ura }
    }

    #[cfg(test)]
    fn single(agent_ura: impl Into<String>, host_device_ura: impl Into<String>) -> Self {
        let host_device_ura = host_device_ura.into();
        let host_node_id = device_id_from_device_ura(&host_device_ura);
        Self {
            by_agent_ura: HashMap::from([(
                agent_ura.into(),
                HostedAgentPlacement {
                    host_device_ura,
                    host_node_id,
                },
            )]),
        }
    }

    fn local_host_for(
        &self,
        agent_ura: &str,
        self_device_ura: &str,
    ) -> Option<&HostedAgentPlacement> {
        let placement = self.by_agent_ura.get(agent_ura)?;
        (placement.host_device_ura == self_device_ura).then_some(placement)
    }
}

impl<'a> DaemonRouteResolver<'a> {
    #[must_use]
    pub(crate) fn new(
        registry: &'a PresenceRegistry,
        advertised_agents: Option<&'a AdvertisedAgentStore>,
        catalog: Option<&'a AbilityCatalogStore>,
    ) -> Self {
        Self {
            registry,
            advertised_agents,
            catalog,
            peer_delegation: None,
            device_local: None,
            now_unix_ms: crate::services::federation_directory::now_unix_ms(),
        }
    }

    /// Inject this daemon's local runtime namespace authority
    /// (RFC-005 §4 / D105).
    ///
    /// When the resolver is asked for a route whose owner is exactly this
    /// daemon authority, or an Agent hosted by this daemon authority, it
    /// proves the ability and route from `authority` (the live local
    /// dispatch table) instead of requiring a hub projection row. This is
    /// what lets a device dispatch its own control-plane abilities (e.g.
    /// `agent.start`) and lets a hub-mode daemon dispatch built-in hosted
    /// agents (e.g. `dev.pages`) before any projection has been
    /// published.
    #[must_use]
    pub(crate) fn with_local_runtime_authority(
        mut self,
        local_authority_ura: impl Into<String>,
        authority: Box<dyn LocalRuntimeAuthority>,
    ) -> Self {
        self.device_local = Some(LocalNamespaceAuthoritySource {
            local_authority_ura: local_authority_ura.into(),
            authority,
            hosted_agents: LocalHostedAgentPlacements::load(),
        });
        self
    }

    #[cfg(test)]
    #[must_use]
    fn with_local_hosted_agent_placements(
        mut self,
        hosted_agents: LocalHostedAgentPlacements,
    ) -> Self {
        if let Some(device_local) = self.device_local.as_mut() {
            device_local.hosted_agents = hosted_agents;
        }
        self
    }

    #[must_use]
    pub(crate) fn with_peer_delegation(
        mut self,
        local_realm: &'a str,
        federated_peers: &'a SharedFederatedPeers,
        federated_directory: &'a SharedFederatedDirectoryView,
        allow_directory_auto_route: bool,
    ) -> Self {
        self.peer_delegation = Some(PeerDelegationSource {
            local_realm,
            federated_peers,
            federated_directory,
            allow_directory_auto_route,
        });
        self
    }

    /// This daemon's own device URA, when local runtime authority is
    /// injected. The directory listing path uses it to include the static
    /// device profile only for this device's own surface.
    fn self_device_ura(&self) -> Option<&str> {
        self.device_local.as_ref().and_then(|device_local| {
            crate::ura::parse_ura(&device_local.local_authority_ura)
                .ok()
                .filter(|parsed| parsed.kind == crate::ura::URAKind::Device)
                .map(|_| device_local.local_authority_ura.as_str())
        })
    }

    #[must_use]
    pub(crate) fn at(mut self, now_unix_ms: i64) -> Self {
        self.now_unix_ms = now_unix_ms;
        self
    }

    pub(crate) fn resolve_query_json(&self, query: &Value) -> Value {
        let query_name = json_string(query, "queryName", "query_name");
        let ability_name = json_string(query, "abilityName", "ability_name");
        let qtype = json_resolve_type(query).unwrap_or_else(|| {
            if !ability_name.is_empty()
                || is_descriptor_ref(&query_name)
                || is_ability_ura(&query_name)
            {
                axon_pb::ResolveType::Route
            } else {
                axon_pb::ResolveType::DirectoryListing
            }
        });

        match qtype {
            axon_pb::ResolveType::Route | axon_pb::ResolveType::Ability => {
                if let Some(answer) = self.delegation_answer_or_negative(&query_name, &ability_name)
                {
                    answer
                } else {
                    self.resolve_route(&query_name, &ability_name).map_or_else(
                        |failure| failure.answer_json(),
                        |route| route.final_route_answer_json(),
                    )
                }
            }
            axon_pb::ResolveType::DirectoryListing
            | axon_pb::ResolveType::CanonicalIdentity
            | axon_pb::ResolveType::Owner => self.directory_answer_json(query, &query_name),
            axon_pb::ResolveType::Key | axon_pb::ResolveType::Service => negative_answer_json(
                &query_name,
                axon_pb::NegativeReason::Nodata,
                Some("daemon namespace.resolve does not serve this qtype yet"),
            ),
            axon_pb::ResolveType::Unspecified => negative_answer_json(
                &query_name,
                axon_pb::NegativeReason::Refused,
                Some("resolve qtype is unspecified"),
            ),
        }
    }

    pub(crate) fn resolve_route(
        &self,
        query_name: &str,
        ability_name: &str,
    ) -> Result<SelectedInvokeRoute, ResolveRouteFailure> {
        let selector = route_selector_from_query(query_name, ability_name).ok_or_else(|| {
            ResolveRouteFailure {
                query_name: query_name.to_string(),
                reason: axon_pb::NegativeReason::Refused,
                detail:
                    "route query must provide an owner URA plus ability_name or a full ability URA"
                        .to_string(),
            }
        })?;

        // RFC-005 §4 / D105: this daemon is authoritative for the
        // executable surface it hosts locally. That includes its device
        // control-plane owner and any Agent owner whose canonical URA is
        // pinned to this device by local-agents.json.
        if let Some(device_local) = self.device_local.as_ref() {
            if device_local.local_authority_ura == selector.owner_ura {
                let kind = local_authority_route_kind(&device_local.local_authority_ura);
                return self.resolve_route_from_local_runtime(
                    &selector,
                    device_local,
                    device_local.local_authority_ura.as_str(),
                    None,
                    kind,
                );
            }

            if let Some(host_node_id) = self.local_host_node_id_for_agent(&selector, device_local) {
                return self.resolve_route_from_local_runtime(
                    &selector,
                    device_local,
                    device_local.local_authority_ura.as_str(),
                    host_node_id.as_deref(),
                    SelectedRouteKind::HostedAgent,
                );
            }

            let is_agent_owner = crate::ura::parse_ura(&selector.owner_ura)
                .ok()
                .is_some_and(|parsed| parsed.kind == crate::ura::URAKind::Agent);
            if is_agent_owner
                && device_local
                    .authority
                    .resolve_owner_ability(&selector.owner_ura, &selector.public_name)
                    .is_some()
            {
                return self.resolve_route_from_local_runtime(
                    &selector,
                    device_local,
                    device_local.local_authority_ura.as_str(),
                    None,
                    SelectedRouteKind::HostedAgent,
                );
            }
        }

        self.resolve_route_from_projection(&selector)
    }

    /// Resolve an ability from this daemon's live runtime dispatch table.
    /// ABILITY existence and the executable ROUTE are both proven locally
    /// (D105), so the answer is `AuthoritativeLocal`.
    fn resolve_route_from_local_runtime(
        &self,
        selector: &RouteSelector,
        device_local: &LocalNamespaceAuthoritySource,
        execution_host_ura: &str,
        host_node_id: Option<&str>,
        kind: SelectedRouteKind,
    ) -> Result<SelectedInvokeRoute, ResolveRouteFailure> {
        let ability = device_local
            .authority
            .resolve_owner_ability(&selector.owner_ura, &selector.public_name)
            .ok_or_else(|| ResolveRouteFailure {
                query_name: selector.query_name.clone(),
                reason: axon_pb::NegativeReason::Nodata,
                detail: "local runtime does not register a dispatchable route for this ability"
                    .to_string(),
            })?;

        let route_ura = format!("route-ref::{}", selector.ability_ura);
        let owner_record = id_record(&selector.owner_ura, self.now_unix_ms);
        let ability_record = device_local_ability_record(
            &selector.ability_ura,
            &selector.owner_ura,
            &selector.public_name,
            self.now_unix_ms,
        );
        let route_record = route_record(
            &route_ura,
            &selector.ability_ura,
            &ability.dispatch_name,
            &selector.owner_ura,
            host_node_id,
            self.now_unix_ms,
        );

        Ok(SelectedInvokeRoute {
            query_name: selector.query_name.clone(),
            owner_ura: selector.owner_ura.clone(),
            callee_ura: selector.owner_ura.clone(),
            execution_host_ura: execution_host_ura.to_string(),
            host_node_id: host_node_id.map(ToOwned::to_owned),
            ability_ura: selector.ability_ura.clone(),
            route_ura,
            dispatch_name: ability.dispatch_name,
            release_profile: axon_pb::ResolverReleaseProfile::AuthoritativeLocal,
            kind,
            ability_record,
            route_record,
            owner_record,
        })
    }

    fn local_host_node_id_for_agent(
        &self,
        selector: &RouteSelector,
        device_local: &LocalNamespaceAuthoritySource,
    ) -> Option<Option<String>> {
        let parsed = crate::ura::parse_ura(&selector.owner_ura).ok()?;
        if parsed.kind != crate::ura::URAKind::Agent {
            return None;
        }

        if let Some(placement) = device_local
            .hosted_agents
            .local_host_for(&selector.owner_ura, &device_local.local_authority_ura)
        {
            return Some(placement.host_node_id.clone());
        }

        let hosted_by_this_device = parsed
            .device_agent_ids()
            .and_then(|(device_id, _)| {
                device_id_from_device_ura(&device_local.local_authority_ura)
                    .filter(|self_id| self_id.as_str() == device_id)
            })
            .is_some();
        hosted_by_this_device.then(|| device_id_from_device_ura(&device_local.local_authority_ura))
    }

    /// Resolve a route for an owner from the resolver's directory: live
    /// presence plus the owner ability projection. Used for owners this
    /// daemon is not itself (hosted agents, and devices a hub routes for).
    /// The placement gate rejects offline owners with NOROUTE; a route
    /// that survives it is the resolver's authoritative selection of where
    /// to dispatch or forward, hence `AuthoritativeLocal`.
    fn resolve_route_from_projection(
        &self,
        selector: &RouteSelector,
    ) -> Result<SelectedInvokeRoute, ResolveRouteFailure> {
        let directory = federation_wrappers::handle_resolve_at(
            &ResolveRequest {
                ura_prefix: Some(selector.owner_ura.clone()),
                include_abilities: true,
                filter: None,
            },
            self.registry,
            self.advertised_agents,
            self.catalog,
            self.self_device_ura(),
            self.now_unix_ms,
        );

        let owner = directory
            .agents
            .iter()
            .find(|agent| agent.ura == selector.owner_ura)
            .ok_or_else(|| {
                let reason =
                    if advertised_agent_host_ura(self.advertised_agents, &selector.owner_ura)
                        .is_some()
                    {
                        axon_pb::NegativeReason::Noroute
                    } else {
                        axon_pb::NegativeReason::Nxdomain
                    };
                ResolveRouteFailure {
                    query_name: selector.query_name.clone(),
                    reason,
                    detail: "owner is not online".to_string(),
                }
            })?;

        let summary = owner
            .abilities
            .iter()
            .find(|summary| {
                summary
                    .get("ability_ura")
                    .and_then(Value::as_str)
                    .is_some_and(|ura| ura == selector.ability_ura)
                    || summary_public_name(summary).as_deref()
                        == Some(selector.public_name.as_str())
            })
            .ok_or_else(|| ResolveRouteFailure {
                query_name: selector.query_name.clone(),
                reason: axon_pb::NegativeReason::Nodata,
                detail: "owner is online but does not publish the requested ability".to_string(),
            })?;

        let selected = SelectedAbilityRoute::from_owner_summary(
            &selector.query_name,
            &selector.owner_ura,
            &selector.public_name,
            owner.host_node_id.as_deref(),
            summary,
            self.advertised_agents,
        )?;

        let owner_record = id_record(&selector.owner_ura, self.now_unix_ms);
        let ability_record =
            ability_record_from_summary(summary, self.now_unix_ms).ok_or_else(|| {
                ResolveRouteFailure {
                    query_name: selector.query_name.clone(),
                    reason: axon_pb::NegativeReason::Noroute,
                    detail: "ability projection is missing canonical ability_ura".to_string(),
                }
            })?;
        let route_record = selected.route_record(self.now_unix_ms);

        // The route was built from the resolver's own directory: presence
        // and the owner ability projection, after the placement gate above
        // already rejected offline owners with NOROUTE. Selecting where a
        // live owner's ability dispatches (locally, or forwarded to the
        // owning device) is exactly what this single-hub resolver is
        // authoritative for, so a successfully built route is
        // `AuthoritativeLocal`. Cross-realm targets never reach here — they
        // are answered as `Delegation`/`PeerHub` before route building.
        Ok(SelectedInvokeRoute {
            query_name: selector.query_name.clone(),
            owner_ura: selector.owner_ura.clone(),
            callee_ura: selected.callee_ura,
            execution_host_ura: selected.execution_host_ura,
            host_node_id: selected.host_node_id,
            ability_ura: selected.ability_ura,
            route_ura: selected.route_ura,
            dispatch_name: selected.dispatch_name,
            release_profile: axon_pb::ResolverReleaseProfile::AuthoritativeLocal,
            kind: selected.kind,
            ability_record,
            route_record,
            owner_record,
        })
    }

    pub(crate) fn resolve_delegation(
        &self,
        query_name: &str,
        ability_name: &str,
    ) -> Result<Option<DelegatedInvokeRoute>, ResolveRouteFailure> {
        let selector = route_selector_from_query(query_name, ability_name).ok_or_else(|| {
            ResolveRouteFailure {
                query_name: query_name.to_string(),
                reason: axon_pb::NegativeReason::Refused,
                detail:
                    "route query must provide an owner URA plus ability_name or a full ability URA"
                        .to_string(),
            }
        })?;
        let Some(peer_source) = self.peer_delegation.as_ref() else {
            return Ok(None);
        };
        let parsed_owner =
            crate::ura::parse_ura(&selector.owner_ura).map_err(|err| ResolveRouteFailure {
                query_name: selector.query_name.clone(),
                reason: axon_pb::NegativeReason::Refused,
                detail: format!("owner URA is invalid: {err}"),
            })?;
        if parsed_owner.realm == peer_source.local_realm {
            return Ok(None);
        }

        let resolution = HubResolver::new(
            peer_source.federated_peers,
            peer_source.federated_directory,
            peer_source.allow_directory_auto_route,
        )
        .resolve(&parsed_owner.realm, &selector.owner_ura);
        let endpoint = match resolution {
            HubResolution::Static { hub_endpoint } => {
                DelegatedPeerEndpoint::new(hub_endpoint, "federated_peers", None)
            }
            HubResolution::DirectoryFallback {
                hub_endpoint,
                target_ura,
            } => DelegatedPeerEndpoint::new(
                hub_endpoint,
                "federated_directory",
                Some(target_ura.as_str()),
            ),
            HubResolution::Offline => {
                return Err(ResolveRouteFailure {
                    query_name: selector.query_name,
                    reason: axon_pb::NegativeReason::Noroute,
                    detail: format!(
                        "remote realm `{}` has no configured peer hub route",
                        parsed_owner.realm
                    ),
                });
            }
        };

        Ok(Some(DelegatedInvokeRoute {
            query_name: selector.query_name,
            owner_ura: selector.owner_ura,
            realm: parsed_owner.realm.clone(),
            hub_ura: crate::ura::hub_ura(&parsed_owner.realm),
            endpoints: vec![endpoint],
            release_profile: axon_pb::ResolverReleaseProfile::AuthoritativeLocal,
        }))
    }

    pub(crate) fn resolve_forward_invoke_route(
        &self,
        target_ura: &str,
        ability_ura: &str,
    ) -> Result<ForwardInvokeRouteSelection, ResolveRouteFailure> {
        let selector =
            route_selector_from_query(ability_ura, "").ok_or_else(|| ResolveRouteFailure {
                query_name: ability_ura.to_string(),
                reason: axon_pb::NegativeReason::Refused,
                detail: "federation.forward_invoke requires a full canonical ability_ura"
                    .to_string(),
            })?;
        let owner_is_agent = crate::ura::parse_ura(&selector.owner_ura)
            .map(|parsed| parsed.kind == crate::ura::URAKind::Agent)
            .unwrap_or(false);
        if selector.owner_ura != target_ura && !owner_is_agent {
            return Err(ResolveRouteFailure {
                query_name: selector.query_name,
                reason: axon_pb::NegativeReason::Refused,
                detail: format!(
                    "ability_ura `{ability_ura}` does not belong to target `{target_ura}`",
                ),
            });
        }

        match self.resolve_route(ability_ura, "") {
            Ok(selected_route) => {
                let target_matches = selected_route.owner_ura == target_ura
                    || selected_route.execution_host_ura == target_ura;
                if !target_matches {
                    return Err(ResolveRouteFailure {
                        query_name: selected_route.query_name.clone(),
                        reason: axon_pb::NegativeReason::Refused,
                        detail: route_owner_mismatch_detail(
                            &selected_route.execution_host_ura,
                            ability_ura,
                            target_ura,
                        ),
                    });
                }
                Ok(ForwardInvokeRouteSelection::Local(selected_route))
            }
            Err(local_failure) => {
                let Some(peer_source) = self.peer_delegation.as_ref() else {
                    return Err(local_failure);
                };
                let parsed_owner = crate::ura::parse_ura(&selector.owner_ura).map_err(|err| {
                    ResolveRouteFailure {
                        query_name: selector.query_name.clone(),
                        reason: axon_pb::NegativeReason::Refused,
                        detail: format!("owner URA is invalid: {err}"),
                    }
                })?;
                if parsed_owner.realm == peer_source.local_realm {
                    return Err(local_failure);
                }
                self.resolve_delegation(ability_ura, "")?
                    .map(ForwardInvokeRouteSelection::Peer)
                    .ok_or(ResolveRouteFailure {
                        query_name: selector.query_name,
                        reason: axon_pb::NegativeReason::Noroute,
                        detail: "cross-realm forward invoke had no peer delegation route"
                            .to_string(),
                    })
            }
        }
    }

    fn delegation_answer_or_negative(&self, query_name: &str, ability_name: &str) -> Option<Value> {
        match self.resolve_delegation(query_name, ability_name) {
            Ok(Some(delegation)) => Some(delegation.delegation_answer_json()),
            Ok(None) => None,
            Err(failure) => Some(failure.answer_json()),
        }
    }

    fn directory_answer_json(&self, query: &Value, query_name: &str) -> Value {
        let prefix = if query_name.is_empty() {
            let realm_hint = json_string(query, "realmHint", "realm_hint");
            (!realm_hint.is_empty()).then_some(realm_hint)
        } else {
            Some(query_name.to_string())
        };
        let directory = federation_wrappers::handle_resolve_at(
            &ResolveRequest {
                ura_prefix: prefix,
                include_abilities: true,
                filter: None,
            },
            self.registry,
            self.advertised_agents,
            self.catalog,
            self.self_device_ura(),
            self.now_unix_ms,
        );
        let mut records = Vec::new();
        for agent in &directory.agents {
            records.push(id_record(&agent.ura, self.now_unix_ms));
            if let Some(record) = hosted_by_record_for_agent(agent, self.now_unix_ms) {
                records.push(record);
            }
            for summary in &agent.abilities {
                let Some(ability_record) = ability_record_from_summary(summary, self.now_unix_ms)
                else {
                    continue;
                };
                records.push(ability_record);
                // Emit the resolver-selected ROUTE record alongside the
                // ABILITY fact so the backend catalog projects routes[]
                // directly from the directory answer instead of
                // re-deriving them. Routes are selected here, the route
                // source — never in a product-layer projection.
                let Some(public_name) = summary_public_name(summary) else {
                    continue;
                };
                let Ok(selected) = SelectedAbilityRoute::from_owner_summary(
                    query_name,
                    &agent.ura,
                    &public_name,
                    agent.host_node_id.as_deref(),
                    summary,
                    self.advertised_agents,
                ) else {
                    continue;
                };
                records.push(selected.route_record(self.now_unix_ms));
            }
        }

        json!({
            "answerKind": axon_pb::ResolveAnswerKind::NonDispatchable.as_str_name(),
            "canonicalName": (!query_name.is_empty()).then_some(query_name),
            "records": records,
            "releaseProfile": axon_pb::ResolverReleaseProfile::AuthoritativeLocal.as_str_name(),
            "authority": authority_for_query(query_name),
            "cachePolicy": cache_policy_json(),
        })
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
struct RouteSelector {
    query_name: String,
    owner_ura: String,
    ability_ura: String,
    public_name: String,
}

fn route_selector_from_query(query_name: &str, ability_name: &str) -> Option<RouteSelector> {
    if ability_name.trim().is_empty() {
        if let Some(selector) = ability_selector_from_descriptor_ref(query_name) {
            return Some(RouteSelector {
                query_name: selector.ability_ura().to_string(),
                owner_ura: selector.owner_ura().to_string(),
                ability_ura: selector.ability_ura().to_string(),
                public_name: selector.public_name().to_string(),
            });
        }
        if is_ability_ura(query_name) {
            let selector = crate::ura::AbilitySelector::parse(query_name).ok()?;
            return Some(RouteSelector {
                query_name: query_name.to_string(),
                owner_ura: selector.owner_ura().to_string(),
                ability_ura: selector.ability_ura().to_string(),
                public_name: selector.public_name().to_string(),
            });
        }
    }
    let owner_ura = query_name.trim();
    let ability_name = ability_name.trim();
    if owner_ura.is_empty() || ability_name.is_empty() {
        return None;
    }
    if let Some(selector) = route_selector_from_descriptor_ref(owner_ura, ability_name) {
        return Some(selector);
    }
    let public_name = crate::ura::owner_local_ability_name(owner_ura, ability_name);
    let ability_ura = crate::ura::owner_ability_ura(owner_ura, &public_name)?;
    Some(RouteSelector {
        query_name: format!("{owner_ura}#{public_name}"),
        owner_ura: owner_ura.to_string(),
        ability_ura,
        public_name,
    })
}

fn route_selector_from_descriptor_ref(
    owner_ura: &str,
    descriptor_ref: &str,
) -> Option<RouteSelector> {
    let selector = ability_selector_from_descriptor_ref(descriptor_ref)?;
    if selector.owner_ura() != owner_ura {
        return None;
    }
    let public_name = selector.public_name().to_string();
    Some(RouteSelector {
        query_name: format!("{owner_ura}#{public_name}"),
        owner_ura: owner_ura.to_string(),
        ability_ura: selector.ability_ura().to_string(),
        public_name,
    })
}

fn ability_selector_from_descriptor_ref(
    descriptor_ref: &str,
) -> Option<crate::ura::AbilitySelector> {
    let descriptor_ref =
        easynet_axon::invocation::canonical_ability_descriptor_ref(descriptor_ref).ok()?;
    let ability_ura = crate::runtime::axon_bridge::descriptor_ref::ability_ura_from_descriptor_ref(
        &descriptor_ref,
    )
    .ok()?;
    crate::ura::AbilitySelector::parse(&ability_ura).ok()
}

fn selected_execution_for_owner(
    query_name: &str,
    owner_ura: &str,
    owner_host_node_id: Option<&str>,
    advertised_agents: Option<&AdvertisedAgentStore>,
) -> Result<(SelectedRouteKind, String, String, Option<String>), ResolveRouteFailure> {
    match crate::ura::parse_ura(owner_ura).map(|parsed| parsed.kind) {
        Ok(crate::ura::URAKind::Hub) => {
            let realm = crate::ura::parse_ura(owner_ura)
                .ok()
                .map(|parsed| parsed.realm)
                .unwrap_or_default();
            let hub_ura = crate::ura::hub_ura(&realm);
            Ok((SelectedRouteKind::HubOwned, hub_ura.clone(), hub_ura, None))
        }
        Ok(crate::ura::URAKind::Agent) => {
            let Some(host_device_ura) = advertised_agent_host_ura(advertised_agents, owner_ura)
            else {
                return Err(ResolveRouteFailure {
                    query_name: query_name.to_string(),
                    reason: axon_pb::NegativeReason::Noroute,
                    detail: "hosted agent has no resolver-selected host device".to_string(),
                });
            };
            Ok((
                SelectedRouteKind::HostedAgent,
                owner_ura.to_string(),
                host_device_ura,
                owner_host_node_id
                    .filter(|node| !node.trim().is_empty())
                    .map(ToOwned::to_owned),
            ))
        }
        Ok(crate::ura::URAKind::Device) => Ok((
            SelectedRouteKind::SameRealmDevice,
            owner_ura.to_string(),
            owner_ura.to_string(),
            None,
        )),
        _ => Err(ResolveRouteFailure {
            query_name: query_name.to_string(),
            reason: axon_pb::NegativeReason::Refused,
            detail: "route owner must be a canonical hub, device, or agent URA".to_string(),
        }),
    }
}

/// The resolver-selected dispatchable route for one (owner, ability)
/// pair. This is the single place that turns a directory ability
/// summary into a route: both the single-route resolve path
/// (`resolve_route`) and the directory listing
/// (`directory_answer_json`) construct it the same way, so a catalog
/// row and an invoke-time route never diverge. The backend projects
/// `routes[]` straight from `route_record()`; it must not re-derive
/// any of these fields.
struct SelectedAbilityRoute {
    ability_ura: String,
    route_ura: String,
    dispatch_name: String,
    callee_ura: String,
    execution_host_ura: String,
    host_node_id: Option<String>,
    kind: SelectedRouteKind,
}

impl SelectedAbilityRoute {
    fn from_owner_summary(
        query_name: &str,
        owner_ura: &str,
        public_name: &str,
        owner_host_node_id: Option<&str>,
        summary: &Value,
        advertised_agents: Option<&AdvertisedAgentStore>,
    ) -> Result<Self, ResolveRouteFailure> {
        let ability_ura = summary
            .get("ability_ura")
            .and_then(Value::as_str)
            .filter(|value| !value.is_empty())
            .ok_or_else(|| ResolveRouteFailure {
                query_name: query_name.to_string(),
                reason: axon_pb::NegativeReason::Noroute,
                detail: "ability projection is missing canonical ability_ura".to_string(),
            })?
            .to_string();
        let route_ura = summary
            .get("route_summary_ref")
            .and_then(Value::as_str)
            .filter(|value| !value.is_empty())
            .ok_or_else(|| ResolveRouteFailure {
                query_name: query_name.to_string(),
                reason: axon_pb::NegativeReason::Noroute,
                detail: "ability projection is missing executable route_summary_ref".to_string(),
            })?
            .to_string();
        let (kind, callee_ura, execution_host_ura, host_node_id) = selected_execution_for_owner(
            query_name,
            owner_ura,
            owner_host_node_id,
            advertised_agents,
        )?;
        Ok(Self {
            ability_ura,
            route_ura,
            dispatch_name: public_name.to_string(),
            callee_ura,
            execution_host_ura,
            host_node_id,
            kind,
        })
    }

    fn route_record(&self, now_unix_ms: i64) -> Value {
        route_record(
            &self.route_ura,
            &self.ability_ura,
            &self.dispatch_name,
            &self.callee_ura,
            self.host_node_id.as_deref(),
            now_unix_ms,
        )
    }
}

fn is_ability_ura(value: &str) -> bool {
    crate::ura::parse_ura(value)
        .map(|parsed| parsed.kind == crate::ura::URAKind::Ability)
        .unwrap_or(false)
}

fn is_descriptor_ref(value: &str) -> bool {
    easynet_axon::invocation::canonical_ability_descriptor_ref(value).is_ok()
}

fn json_string(value: &Value, camel: &str, snake: &str) -> String {
    value
        .get(camel)
        .or_else(|| value.get(snake))
        .and_then(Value::as_str)
        .unwrap_or_default()
        .trim()
        .to_string()
}

fn json_resolve_type(value: &Value) -> Option<axon_pb::ResolveType> {
    let raw = value.get("qtype").or_else(|| value.get("qType"))?;
    if let Some(num) = raw.as_i64() {
        return axon_pb::ResolveType::try_from(num as i32).ok();
    }
    let text = raw.as_str()?.trim();
    if text.is_empty() {
        return None;
    }
    axon_pb::ResolveType::from_str_name(text).or_else(|| {
        let canonical = format!("RESOLVE_TYPE_{}", text.to_ascii_uppercase());
        axon_pb::ResolveType::from_str_name(&canonical)
    })
}

fn advertised_agent_host_ura(
    advertised_agents: Option<&AdvertisedAgentStore>,
    agent_ura: &str,
) -> Option<String> {
    advertised_agents?
        .snapshot()
        .into_iter()
        .find(|record| record.agent_ura == agent_ura)
        .and_then(|record| record.host_ura().map(str::to_string))
}

fn device_id_from_device_ura(device_ura: &str) -> Option<String> {
    crate::ura::parse_ura(device_ura)
        .ok()
        .filter(|parsed| parsed.kind == crate::ura::URAKind::Device)
        .and_then(|parsed| parsed.device_id().map(str::to_string))
}

fn local_authority_route_kind(local_authority_ura: &str) -> SelectedRouteKind {
    if crate::ura::parse_ura(local_authority_ura)
        .ok()
        .is_some_and(|parsed| parsed.kind == crate::ura::URAKind::Hub)
    {
        SelectedRouteKind::HubOwned
    } else {
        SelectedRouteKind::LocalDevice
    }
}

fn id_record(name: &str, now_unix_ms: i64) -> Value {
    json!({
        "name": name,
        "recordType": axon_pb::RecordType::Id.as_str_name(),
        "authority": authority_for_query(name),
        "ttlMs": 0,
        "expiresUnixMs": 0,
        "revision": now_unix_ms.max(0) as u64,
        "value": {
            "id": {
                "ura": name,
                "kind": ura_kind_name(name),
            }
        }
    })
}

fn hosted_by_record_for_agent(agent: &ResolveAgentSummary, now_unix_ms: i64) -> Option<Value> {
    let host_node_id = agent.host_node_id.as_deref()?.trim();
    if host_node_id.is_empty() {
        return None;
    }
    let parsed = crate::ura::parse_ura(&agent.ura).ok()?;
    if parsed.kind != crate::ura::URAKind::Agent {
        return None;
    }
    let host_ura = crate::ura::device_ura(&parsed.realm, host_node_id);
    Some(hosted_by_record(
        &agent.ura,
        &host_ura,
        host_node_id,
        now_unix_ms,
    ))
}

fn hosted_by_record(
    hosted_ura: &str,
    host_ura: &str,
    host_node_id: &str,
    now_unix_ms: i64,
) -> Value {
    json!({
        "name": hosted_ura,
        "recordType": axon_pb::RecordType::HostedBy.as_str_name(),
        "authority": authority_for_query(hosted_ura),
        "ttlMs": 0,
        "expiresUnixMs": 0,
        "revision": now_unix_ms.max(0) as u64,
        "value": {
            "hostedBy": {
                "hostedUra": hosted_ura,
                "hostUra": host_ura,
                "hostNodeId": host_node_id,
                "leaseExpiresUnixMs": 0,
            }
        }
    })
}

fn ability_record_from_summary(summary: &Value, now_unix_ms: i64) -> Option<Value> {
    let ability_ura = summary
        .get("ability_ura")
        .and_then(Value::as_str)
        .filter(|value| !value.is_empty())?;
    let owner_ura = summary
        .get("owner_ura")
        .and_then(Value::as_str)
        .filter(|value| !value.is_empty())?;
    let namespace = summary
        .get("namespace")
        .and_then(Value::as_str)
        .unwrap_or_default();
    let local_name = summary
        .get("local_name")
        .and_then(Value::as_str)
        .filter(|value| !value.is_empty())?;
    Some(json!({
        "name": ability_ura,
        "recordType": axon_pb::RecordType::Ability.as_str_name(),
        "authority": authority_for_query(ability_ura),
        "ttlMs": 0,
        "expiresUnixMs": 0,
        "revision": now_unix_ms.max(0) as u64,
        "value": {
            "ability": {
                "abilityUra": ability_ura,
                "ownerUra": owner_ura,
                "namespace": namespace,
                "localName": local_name,
                "summary": {
                    "abilityUra": ability_ura,
                    "ownerUra": owner_ura,
                    "namespace": namespace,
                    "localName": local_name,
                    "descriptorRevision": summary
                        .get("descriptor_revision")
                        .and_then(Value::as_str)
                        .unwrap_or_default(),
                    "schemaRef": summary.get("schema_ref").cloned().unwrap_or(Value::Null),
                    "schemaHash": summary.get("schema_hash").cloned().unwrap_or(Value::Null),
                    "policyRef": summary
                        .get("policy_ref")
                        .and_then(Value::as_str)
                        .unwrap_or_default(),
                    "routeSummaryRef": summary.get("route_summary_ref").cloned().unwrap_or(Value::Null),
                    "tags": summary.get("tags").cloned().unwrap_or_else(|| json!([])),
                }
            }
        }
    }))
}

/// Build an `ABILITY` record for a device-owned ability proven from the
/// local dispatch table (D105). Unlike [`ability_record_from_summary`],
/// the inputs are the resolver-canonical owner/ability URAs and the
/// owner-local public name, not a hub projection summary — the device is
/// the authority, so no projection row is required.
fn device_local_ability_record(
    ability_ura: &str,
    owner_ura: &str,
    public_name: &str,
    now_unix_ms: i64,
) -> Value {
    let (namespace, local_name) = public_name
        .split_once('.')
        .map_or(("", public_name), |(ns, local)| (ns, local));
    json!({
        "name": ability_ura,
        "recordType": axon_pb::RecordType::Ability.as_str_name(),
        "authority": authority_for_query(ability_ura),
        "ttlMs": 0,
        "expiresUnixMs": 0,
        "revision": now_unix_ms.max(0) as u64,
        "value": {
            "ability": {
                "abilityUra": ability_ura,
                "ownerUra": owner_ura,
                "namespace": namespace,
                "localName": local_name,
                "summary": {
                    "abilityUra": ability_ura,
                    "ownerUra": owner_ura,
                    "namespace": namespace,
                    "localName": local_name,
                    "policyRef": "visibility:PUBLIC",
                }
            }
        }
    })
}

fn route_record(
    route_ura: &str,
    ability_ura: &str,
    dispatch_name: &str,
    owner_ura: &str,
    host_node_id: Option<&str>,
    now_unix_ms: i64,
) -> Value {
    json!({
        "name": route_ura,
        "recordType": axon_pb::RecordType::Route.as_str_name(),
        "authority": authority_for_query(route_ura),
        "ttlMs": 0,
        "expiresUnixMs": 0,
        "revision": now_unix_ms.max(0) as u64,
        "value": {
            "route": {
                "routeUra": route_ura,
                "abilityUra": ability_ura,
                "dispatchName": dispatch_name,
                "executeOn": {
                    "kind": ura_kind_name(owner_ura),
                    "targetUra": owner_ura,
                    "hostNodeId": host_node_id.unwrap_or_default(),
                }
            }
        }
    })
}

fn negative_answer_json(
    query_name: &str,
    reason: axon_pb::NegativeReason,
    detail: Option<&str>,
) -> Value {
    json!({
        "answerKind": axon_pb::ResolveAnswerKind::Negative.as_str_name(),
        "nextHop": {
            "noRoute": {}
        },
        "records": [],
        "releaseProfile": axon_pb::ResolverReleaseProfile::AuthoritativeLocal.as_str_name(),
        "authority": authority_for_query(query_name),
        "cachePolicy": cache_policy_json(),
        "negative": {
            "reason": reason.as_str_name(),
            "queryName": query_name,
            "detail": detail,
        }
    })
}

fn authority_for_query(query_name: &str) -> Value {
    let realm = crate::ura::parse_ura(query_name)
        .ok()
        .map(|parsed| parsed.realm)
        .filter(|realm| !realm.is_empty())
        .unwrap_or_else(|| "localhost".to_string());
    json!({
        "authorityUra": crate::ura::hub_ura(&realm),
        "zoneRef": format!("realm:{realm}"),
        "algorithm": "daemon-local",
        "signature": "",
        "issuedUnixMs": 0,
    })
}

fn cache_policy_json() -> Value {
    json!({
        "ttlMs": 0,
        "sharedCacheable": false,
        "retryAfterUnixMs": 0,
    })
}

fn ura_kind_name(ura: &str) -> &'static str {
    match crate::ura::parse_ura(ura).map(|parsed| parsed.kind) {
        Ok(crate::ura::URAKind::Hub) => axon_pb::UraKind::Hub.as_str_name(),
        Ok(crate::ura::URAKind::Device) => axon_pb::UraKind::Device.as_str_name(),
        Ok(crate::ura::URAKind::User) => axon_pb::UraKind::User.as_str_name(),
        Ok(crate::ura::URAKind::Agent) => axon_pb::UraKind::Agent.as_str_name(),
        Ok(crate::ura::URAKind::Ability) => axon_pb::UraKind::Ability.as_str_name(),
        Ok(crate::ura::URAKind::Resource) => axon_pb::UraKind::Resource.as_str_name(),
        _ => axon_pb::UraKind::Unspecified.as_str_name(),
    }
}

fn summary_public_name(summary: &Value) -> Option<String> {
    let namespace = summary
        .get("namespace")
        .and_then(Value::as_str)
        .unwrap_or_default();
    let local = summary.get("local_name").and_then(Value::as_str)?;
    if namespace.is_empty() {
        Some(local.to_string())
    } else {
        Some(format!("{namespace}.{local}"))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    use std::collections::BTreeMap;

    use crate::services::ability_catalog_store::{AbilityCatalogStore, OwnerAbilityProjectionRow};
    use crate::services::advertised_agent_store::{
        AdvertisedAgentRecord, AdvertisedAgentSigningAuthority, AdvertisedAgentStore,
    };
    use crate::services::federated_peers_cell::SharedFederatedPeers;
    use crate::services::federation_directory::SharedFederatedDirectoryView;
    use crate::services::presence_registry::PresenceRegistry;

    const TEST_NOW_MS: i64 = 1_700_000_000_000;
    const LEASE_EXPIRES_MS: i64 = 4_102_444_800_000;

    fn device_owner_ura() -> String {
        crate::ura::device_ura("test-realm", "test-daemon")
    }

    /// Build a `DispatchSender` whose receiver is dropped immediately.
    /// Presence only needs a live entry (the URA in `snapshot()`); the
    /// resolver never sends on the channel.
    fn make_dispatch_sender() -> crate::services::presence_registry::DispatchSender {
        let (tx, _rx) = tokio::sync::mpsc::channel(1);
        tx
    }

    /// Mark `owner_ura` as online in presence (the same liveness signal
    /// `handle_resolve_at` reads from `registry.snapshot()`).
    fn mark_online(registry: &PresenceRegistry, owner_ura: &str) {
        registry.insert(owner_ura.to_string(), make_dispatch_sender());
    }

    /// Test double for [`LocalRuntimeAuthority`] holding a fixed set of
    /// canonical Ability URAs, matching the membership contract of the
    /// production [`LocalRuntimeAuthoritySnapshot`] without a runtime.
    struct FakeLocalRuntimeAuthority {
        ability_uras: HashSet<String>,
    }

    impl FakeLocalRuntimeAuthority {
        fn with_owner_keys(owner_ura: &str, keys: &[&str]) -> Box<dyn LocalRuntimeAuthority> {
            Box::new(Self {
                ability_uras: keys
                    .iter()
                    .map(|public_name| {
                        crate::ura::owner_ability_ura(owner_ura, public_name)
                            .expect("test owner ability ura")
                    })
                    .collect(),
            })
        }
    }

    impl LocalRuntimeAuthority for FakeLocalRuntimeAuthority {
        fn resolve_owner_ability(
            &self,
            owner_ura: &str,
            public_name: &str,
        ) -> Option<LocalRuntimeAbility> {
            let ability_ura = crate::ura::owner_ability_ura(owner_ura, public_name)?;
            if !self.ability_uras.contains(&ability_ura) {
                return None;
            }
            let dispatch_name = crate::ura::local_dispatch_ability_key(owner_ura, public_name);
            Some(LocalRuntimeAbility { dispatch_name })
        }
    }

    /// Publish a single ability projection for `owner_ura`, mirroring the
    /// `invoke_dispatches_namespace_resolve_to_typed_answer` fixture.
    fn publish_ability(
        catalog: &AbilityCatalogStore,
        owner_ura: &str,
        host_device_ura: &str,
        namespace: &str,
        local_name: &str,
    ) -> String {
        publish_ability_with_route_summary(
            catalog,
            owner_ura,
            host_device_ura,
            namespace,
            local_name,
            true,
        )
    }

    fn publish_ability_with_route_summary(
        catalog: &AbilityCatalogStore,
        owner_ura: &str,
        host_device_ura: &str,
        namespace: &str,
        local_name: &str,
        include_route_summary: bool,
    ) -> String {
        let public_name = if namespace.is_empty() {
            local_name.to_string()
        } else {
            format!("{namespace}.{local_name}")
        };
        let ability_ura =
            crate::ura::owner_ability_ura(owner_ura, &public_name).expect("owner ability ura");
        catalog.upsert_projection(OwnerAbilityProjectionRow::new(
            owner_ura.to_string(),
            host_device_ura.to_string(),
            1,
            "sha256:test".to_string(),
            LEASE_EXPIRES_MS,
            vec![crate::runtime::owner_projection::AbilityProjectionSummary {
                ability_ura: ability_ura.clone(),
                owner_ura: owner_ura.to_string(),
                namespace: namespace.to_string(),
                local_name: local_name.to_string(),
                descriptor_revision: "sha256:descriptor".to_string(),
                schema_ref: None,
                schema_hash: None,
                policy_ref: "visibility:PUBLIC".to_string(),
                route_summary_ref: include_route_summary
                    .then(|| format!("route-ref::{ability_ura}")),
                tags: vec!["class:unary".to_string()],
                callable_summary: crate::runtime::owner_projection::AbilityCallableSummary::minimal(
                    public_name,
                ),
            }],
        ));
        ability_ura
    }

    #[test]
    fn device_owned_ability_online_resolves_final_local_device_route() {
        let registry = PresenceRegistry::new();
        let catalog = AbilityCatalogStore::new();
        let owner_ura = device_owner_ura();
        mark_online(&registry, &owner_ura);
        let ability_ura =
            crate::ura::owner_ability_ura(&owner_ura, "agent.list").expect("ability ura");
        let authority = FakeLocalRuntimeAuthority::with_owner_keys(&owner_ura, &["agent.list"]);

        let route = DaemonRouteResolver::new(&registry, None, Some(&catalog))
            .with_local_runtime_authority(owner_ura.clone(), authority)
            .at(TEST_NOW_MS)
            .resolve_route(&owner_ura, "agent.list")
            .expect("device-owned ability online must resolve a final route");

        assert_eq!(route.kind(), SelectedRouteKind::LocalDevice);
        assert_eq!(route.route_reason(), axon_pb::RouteReason::LocalDevice);
        assert_eq!(
            route.dispatch_target(false),
            SelectedRouteDispatchTarget::LocalRuntime,
            "a route proven from local runtime authority must remain local even if a caller \
             supplies a stale self-host hint"
        );
        assert_eq!(route.owner_ura, owner_ura);
        assert_eq!(route.execution_host_ura, owner_ura);
        assert_eq!(route.callee_ura, owner_ura);
        assert_eq!(route.ability_ura, ability_ura);
        assert_eq!(route.route_ura, format!("route-ref::{ability_ura}"));
        assert_eq!(route.dispatch_name, "agent.list");
        assert!(route.is_authoritative_local_or_better());

        // next_hop must take the localDeviceAbility shape.
        let next_hop = route.next_hop_json();
        let local = &next_hop["localDeviceAbility"];
        assert_eq!(local["deviceUra"], owner_ura);
        assert_eq!(local["abilityUra"], ability_ura);
        assert_eq!(local["routeUra"], format!("route-ref::{ability_ura}"));
        assert_eq!(local["dispatchName"], "agent.list");
    }

    #[test]
    fn device_owned_descriptor_ref_resolves_same_final_route() {
        let registry = PresenceRegistry::new();
        let catalog = AbilityCatalogStore::new();
        let owner_ura = device_owner_ura();
        mark_online(&registry, &owner_ura);
        let ability_ura =
            crate::ura::owner_ability_ura(&owner_ura, "agent.list").expect("ability ura");
        let descriptor_ref = format!("{ability_ura}@1.0.0");
        let authority = FakeLocalRuntimeAuthority::with_owner_keys(&owner_ura, &["agent.list"]);

        let route = DaemonRouteResolver::new(&registry, None, Some(&catalog))
            .with_local_runtime_authority(owner_ura.clone(), authority)
            .at(TEST_NOW_MS)
            .resolve_route(&owner_ura, &descriptor_ref)
            .expect("descriptor-bound device ability must resolve through the same route gate");

        assert_eq!(route.kind(), SelectedRouteKind::LocalDevice);
        assert_eq!(route.owner_ura, owner_ura);
        assert_eq!(route.callee_ura, owner_ura);
        assert_eq!(route.execution_host_ura, owner_ura);
        assert_eq!(route.ability_ura, ability_ura);
        assert_eq!(route.route_ura, format!("route-ref::{ability_ura}"));
        assert_eq!(route.dispatch_name, "agent.list");
        assert_eq!(route.query_name, format!("{owner_ura}#agent.list"));
    }

    #[test]
    fn descriptor_ref_query_without_ability_resolves_same_final_route() {
        let registry = PresenceRegistry::new();
        let catalog = AbilityCatalogStore::new();
        let owner_ura = device_owner_ura();
        mark_online(&registry, &owner_ura);
        let ability_ura =
            crate::ura::owner_ability_ura(&owner_ura, "agent.list").expect("ability ura");
        let descriptor_ref = format!("{ability_ura}@1.0.0");
        let authority = FakeLocalRuntimeAuthority::with_owner_keys(&owner_ura, &["agent.list"]);

        let route = DaemonRouteResolver::new(&registry, None, Some(&catalog))
            .with_local_runtime_authority(owner_ura.clone(), authority)
            .at(TEST_NOW_MS)
            .resolve_route(&descriptor_ref, "")
            .expect("descriptor-bound ability query must resolve through the same route gate");

        assert_eq!(route.kind(), SelectedRouteKind::LocalDevice);
        assert_eq!(route.owner_ura, owner_ura);
        assert_eq!(route.ability_ura, ability_ura);
        assert_eq!(route.dispatch_name, "agent.list");
    }

    #[test]
    fn device_profile_terminal_and_resource_abilities_resolve_from_local_authority() {
        let registry = PresenceRegistry::new();
        let catalog = AbilityCatalogStore::new();
        let owner_ura = device_owner_ura();
        mark_online(&registry, &owner_ura);
        // The device profile is proven from the live runtime dispatch
        // table (D105), not from a hub projection — the catalog stays empty.
        let profile_keys = ["terminal.list", "meta.list_resources", "agent.start"];
        let authority = FakeLocalRuntimeAuthority::with_owner_keys(&owner_ura, &profile_keys);

        let resolver = DaemonRouteResolver::new(&registry, None, Some(&catalog))
            .with_local_runtime_authority(owner_ura.clone(), authority)
            .at(TEST_NOW_MS);
        for ability in profile_keys {
            let route = resolver
                .resolve_route(&owner_ura, ability)
                .unwrap_or_else(|err| {
                    panic!("{ability} must resolve from device-local authority: {err:?}")
                });
            let expected_ability_ura =
                crate::ura::owner_ability_ura(&owner_ura, ability).expect("ability ura");

            assert_eq!(route.kind(), SelectedRouteKind::LocalDevice);
            assert_eq!(route.owner_ura, owner_ura);
            assert_eq!(route.callee_ura, owner_ura);
            assert_eq!(route.execution_host_ura, owner_ura);
            assert_eq!(route.ability_ura, expected_ability_ura);
            assert_eq!(
                route.route_ura,
                format!("route-ref::{expected_ability_ura}")
            );
            assert_eq!(route.dispatch_name, ability);
            assert!(route.is_authoritative_local_or_better());
        }
    }

    #[test]
    fn owner_absent_resolves_nxdomain() {
        let registry = PresenceRegistry::new();
        let catalog = AbilityCatalogStore::new();
        let owner_ura = device_owner_ura();
        // No presence entry, no advertised host record.

        let failure = DaemonRouteResolver::new(&registry, None, Some(&catalog))
            .at(TEST_NOW_MS)
            .resolve_route(&owner_ura, "agent.list")
            .expect_err("absent owner must resolve negative");

        assert_eq!(failure.reason, axon_pb::NegativeReason::Nxdomain);
        assert_eq!(failure.query_name, format!("{owner_ura}#agent.list"));
    }

    #[test]
    fn owner_advertised_but_offline_resolves_noroute() {
        let registry = PresenceRegistry::new();
        let advertised = AdvertisedAgentStore::new();
        let catalog = AbilityCatalogStore::new();
        let agent_ura = crate::ura::agent_ura("test-realm", "alice", "assistant");
        let host_ura = device_owner_ura();
        // Advertised with a host linkage, but the host is NOT in presence.
        advertised.upsert(AdvertisedAgentRecord {
            agent_ura: agent_ura.clone(),
            public_key_hex: "00".to_string(),
            host_node_id: Some("node-1".to_string()),
            signing_authority: AdvertisedAgentSigningAuthority::HostedBy {
                host_ura: host_ura.clone(),
            },
        });

        let failure = DaemonRouteResolver::new(&registry, Some(&advertised), Some(&catalog))
            .at(TEST_NOW_MS)
            .resolve_route(&agent_ura, "agent.list")
            .expect_err("advertised-but-offline owner must resolve negative");

        assert_eq!(failure.reason, axon_pb::NegativeReason::Noroute);
        assert_eq!(failure.query_name, format!("{agent_ura}#agent.list"));
    }

    #[test]
    fn owner_online_without_ability_resolves_nodata() {
        let registry = PresenceRegistry::new();
        let catalog = AbilityCatalogStore::new();
        let owner_ura = device_owner_ura();
        mark_online(&registry, &owner_ura);
        // Owner publishes `agent.list` but the request asks for `fs.read`.
        publish_ability(&catalog, &owner_ura, &owner_ura, "agent", "list");

        let failure = DaemonRouteResolver::new(&registry, None, Some(&catalog))
            .at(TEST_NOW_MS)
            .resolve_route(&owner_ura, "fs.read")
            .expect_err("online owner missing the ability must resolve negative");

        assert_eq!(failure.reason, axon_pb::NegativeReason::Nodata);
        assert_eq!(failure.query_name, format!("{owner_ura}#fs.read"));
    }

    // ---- RFC-005 §4 / D105: device-local namespace authority ----

    #[test]
    fn device_owns_control_ability_via_local_authority_without_any_projection() {
        // The exact production failure this fixes: a device resolves its
        // own control-plane ability (`agent.start`) with an EMPTY catalog.
        // Device-local authority must prove ABILITY + ROUTE from the live
        // runtime dispatch table, not from a hub projection row.
        let registry = PresenceRegistry::new();
        let catalog = AbilityCatalogStore::new();
        let owner_ura = device_owner_ura();
        mark_online(&registry, &owner_ura);
        let authority = FakeLocalRuntimeAuthority::with_owner_keys(&owner_ura, &["agent.start"]);

        let route = DaemonRouteResolver::new(&registry, None, Some(&catalog))
            .with_local_runtime_authority(owner_ura.clone(), authority)
            .at(TEST_NOW_MS)
            .resolve_route(&owner_ura, "agent.start")
            .expect("device-owned ability must resolve from local authority with no catalog row");

        let ability_ura =
            crate::ura::owner_ability_ura(&owner_ura, "agent.start").expect("ability ura");
        assert_eq!(route.kind(), SelectedRouteKind::LocalDevice);
        assert_eq!(route.owner_ura, owner_ura);
        assert_eq!(route.callee_ura, owner_ura);
        assert_eq!(route.execution_host_ura, owner_ura);
        assert_eq!(route.ability_ura, ability_ura);
        assert_eq!(route.route_ura, format!("route-ref::{ability_ura}"));
        assert_eq!(route.dispatch_name, "agent.start");
        assert!(route.is_authoritative_local_or_better());
        // Catalog stays empty — authority did not come from a projection.
        assert!(catalog.is_empty());
    }

    #[test]
    fn device_ability_not_registered_in_runtime_resolves_nodata() {
        // The device is online and is its own authority, but the runtime
        // does not register the requested binding → typed NODATA, not a
        // false-positive route.
        let registry = PresenceRegistry::new();
        let catalog = AbilityCatalogStore::new();
        let owner_ura = device_owner_ura();
        mark_online(&registry, &owner_ura);
        let authority = FakeLocalRuntimeAuthority::with_owner_keys(&owner_ura, &["agent.start"]);

        let failure = DaemonRouteResolver::new(&registry, None, Some(&catalog))
            .with_local_runtime_authority(owner_ura.clone(), authority)
            .at(TEST_NOW_MS)
            .resolve_route(&owner_ura, "fs.read")
            .expect_err("unregistered device ability must resolve negative");

        assert_eq!(failure.reason, axon_pb::NegativeReason::Nodata);
        assert_eq!(failure.query_name, format!("{owner_ura}#fs.read"));
    }

    #[test]
    fn device_local_authority_resolves_without_presence_projection() {
        // Device-local authority is captured from the live LocalRuntime.
        // That runtime snapshot is the local liveness/execution proof for
        // this daemon; it must not require a hub-style presence row.
        let registry = PresenceRegistry::new();
        let catalog = AbilityCatalogStore::new();
        let owner_ura = device_owner_ura();
        let authority = FakeLocalRuntimeAuthority::with_owner_keys(&owner_ura, &["agent.start"]);

        let route = DaemonRouteResolver::new(&registry, None, Some(&catalog))
            .with_local_runtime_authority(owner_ura.clone(), authority)
            .at(TEST_NOW_MS)
            .resolve_route(&owner_ura, "agent.start")
            .expect("device-local authority must not depend on projection presence");

        assert_eq!(route.kind(), SelectedRouteKind::LocalDevice);
        assert_eq!(route.owner_ura, owner_ura);
        assert_eq!(route.execution_host_ura, owner_ura);
    }

    #[test]
    fn hosted_agent_implemented_on_this_device_is_authoritative_local() {
        // A hosted agent's ability implementation lives on THIS device
        // (D44/D105). The owner is the Agent URA, while the execution
        // host is the device that owns the local runtime. The route must
        // be proven from local-agents.json placement + runtime bindings,
        // not from presence or hub ability projection rows.
        let registry = PresenceRegistry::new();
        let catalog = AbilityCatalogStore::new();
        let host_ura = device_owner_ura();
        let agent_ura = crate::ura::agent_ura("test-realm", "alice", "assistant");
        let authority = FakeLocalRuntimeAuthority::with_owner_keys(&agent_ura, &["chat"]);

        let route = DaemonRouteResolver::new(&registry, None, Some(&catalog))
            .with_local_runtime_authority(host_ura.clone(), authority)
            .with_local_hosted_agent_placements(LocalHostedAgentPlacements::single(
                agent_ura.clone(),
                host_ura.clone(),
            ))
            .at(TEST_NOW_MS)
            .resolve_route(&agent_ura, "chat")
            .expect("hosted agent on this device must resolve a route");

        assert_eq!(route.kind(), SelectedRouteKind::HostedAgent);
        assert_eq!(route.route_reason(), axon_pb::RouteReason::HostedAgent);
        assert_eq!(
            route.dispatch_target(true),
            SelectedRouteDispatchTarget::LocalRuntime
        );
        assert_eq!(
            route.dispatch_target(false),
            SelectedRouteDispatchTarget::PresenceSession
        );
        assert_eq!(route.owner_ura, agent_ura);
        assert_eq!(route.callee_ura, agent_ura);
        assert_eq!(route.execution_host_ura, host_ura);
        assert_eq!(route.dispatch_name, "assistant.chat");
        assert!(catalog.is_empty());
        assert!(
            route.is_authoritative_local_or_better(),
            "ability implemented on this device is device-local authority"
        );
    }

    #[test]
    fn hosted_agent_runtime_binding_can_route_without_local_agent_placement() {
        let registry = PresenceRegistry::new();
        let catalog = AbilityCatalogStore::new();
        let host_ura = device_owner_ura();
        let agent_ura = crate::ura::agent_ura("test-realm", "dev", "pages");
        let authority = FakeLocalRuntimeAuthority::with_owner_keys(&agent_ura, &["list"]);

        let route = DaemonRouteResolver::new(&registry, None, Some(&catalog))
            .with_local_runtime_authority(host_ura.clone(), authority)
            .at(TEST_NOW_MS)
            .resolve_route(&agent_ura, "pages.list")
            .expect("hosted agent registry key must resolve through owner-local public name");

        assert_eq!(route.kind(), SelectedRouteKind::HostedAgent);
        assert_eq!(route.execution_host_ura, host_ura);
        assert_eq!(
            route.ability_ura,
            "easynet:///r/test-realm/ability/dev.pages.list"
        );
        assert_eq!(route.dispatch_name, "pages.list");
        assert_eq!(route.query_name, format!("{agent_ura}#list"));
    }

    #[test]
    fn hub_local_authority_can_route_builtin_pages_agent_without_projection() {
        let registry = PresenceRegistry::new();
        let catalog = AbilityCatalogStore::new();
        let host_ura = crate::ura::hub_ura("test-realm");
        let agent_ura = crate::ura::agent_ura("test-realm", "dev", "pages");
        let authority = FakeLocalRuntimeAuthority::with_owner_keys(&agent_ura, &["list"]);

        let route = DaemonRouteResolver::new(&registry, None, Some(&catalog))
            .with_local_runtime_authority(host_ura.clone(), authority)
            .at(TEST_NOW_MS)
            .resolve_route(&agent_ura, "pages.list")
            .expect("hub-mode local runtime authority must route built-in pages agent");

        assert_eq!(route.kind(), SelectedRouteKind::HostedAgent);
        assert_eq!(route.execution_host_ura, host_ura);
        assert_eq!(
            route.ability_ura,
            "easynet:///r/test-realm/ability/dev.pages.list"
        );
        assert_eq!(route.dispatch_name, "pages.list");
        assert_eq!(route.query_name, format!("{agent_ura}#list"));
        assert!(catalog.is_empty());
    }

    #[test]
    fn hub_local_authority_resolves_hub_owned_ability_as_hub_route() {
        let registry = PresenceRegistry::new();
        let catalog = AbilityCatalogStore::new();
        let host_ura = crate::ura::hub_ura("test-realm");
        let authority =
            FakeLocalRuntimeAuthority::with_owner_keys(&host_ura, &["federation.status"]);

        let route = DaemonRouteResolver::new(&registry, None, Some(&catalog))
            .with_local_runtime_authority(host_ura.clone(), authority)
            .at(TEST_NOW_MS)
            .resolve_route(&host_ura, "federation.status")
            .expect("hub-owned runtime ability must resolve through local hub authority");

        assert_eq!(route.kind(), SelectedRouteKind::HubOwned);
        assert_eq!(route.route_reason(), axon_pb::RouteReason::LocalHub);
        assert_eq!(
            route.dispatch_target(true),
            SelectedRouteDispatchTarget::LocalRuntime
        );
        assert_eq!(route.owner_ura, host_ura);
        assert_eq!(route.callee_ura, route.owner_ura);
        assert_eq!(route.execution_host_ura, route.owner_ura);
        assert_eq!(route.dispatch_name, "federation.status");
        assert!(catalog.is_empty());
    }

    #[test]
    fn hosted_agent_descriptor_ref_resolves_owner_local_public_name() {
        let registry = PresenceRegistry::new();
        let catalog = AbilityCatalogStore::new();
        let host_ura = device_owner_ura();
        let agent_ura = crate::ura::agent_ura("test-realm", "dev", "pages");
        let ability_ura =
            crate::ura::owner_ability_ura(&agent_ura, "list").expect("agent ability ura");
        let descriptor_ref = format!("{ability_ura}@1.0.0");
        let authority = FakeLocalRuntimeAuthority::with_owner_keys(&agent_ura, &["list"]);

        let route = DaemonRouteResolver::new(&registry, None, Some(&catalog))
            .with_local_runtime_authority(host_ura.clone(), authority)
            .at(TEST_NOW_MS)
            .resolve_route(&agent_ura, &descriptor_ref)
            .expect("descriptor-bound hosted-agent ability must resolve through local authority");

        assert_eq!(route.kind(), SelectedRouteKind::HostedAgent);
        assert_eq!(route.owner_ura, agent_ura);
        assert_eq!(route.callee_ura, agent_ura);
        assert_eq!(route.execution_host_ura, host_ura);
        assert_eq!(route.ability_ura, ability_ura);
        assert_eq!(route.dispatch_name, "pages.list");
        assert_eq!(route.query_name, format!("{agent_ura}#list"));
    }

    #[test]
    fn hub_owned_ability_projects_hub_route_kind() {
        let registry = PresenceRegistry::new();
        let catalog = AbilityCatalogStore::new();
        let hub_ura = crate::ura::hub_ura("test-realm");
        mark_online(&registry, &hub_ura);
        let ability_ura = publish_ability(&catalog, &hub_ura, &hub_ura, "federation", "status");

        let route = DaemonRouteResolver::new(&registry, None, Some(&catalog))
            .at(TEST_NOW_MS)
            .resolve_route(&hub_ura, "federation.status")
            .expect("hub-owned ability must resolve through the hub route kind");

        assert_eq!(route.kind(), SelectedRouteKind::HubOwned);
        assert_eq!(route.route_reason(), axon_pb::RouteReason::LocalHub);
        assert_eq!(
            route.dispatch_target(true),
            SelectedRouteDispatchTarget::LocalRuntime
        );
        assert_eq!(route.owner_ura, hub_ura);
        assert_eq!(route.callee_ura, hub_ura);
        assert_eq!(route.execution_host_ura, hub_ura);
        assert_eq!(route.ability_ura, ability_ura);

        let answer = route.final_route_answer_json();
        assert_eq!(
            answer["selectedRoute"]["reason"],
            axon_pb::RouteReason::LocalHub.as_str_name()
        );
        assert!(answer["nextHop"]["localHubAbility"].is_object());
        assert_eq!(
            answer["nextHop"]["localHubAbility"]["dispatchName"],
            "federation.status"
        );
    }

    #[test]
    fn projection_route_for_present_device_owner_is_same_realm_device() {
        // A device-owned ability advertised in the catalog whose owner is
        // present, resolved by a node that is NOT the owner's own daemon
        // (e.g. the hub resolving `runtime.invoke_remote` for a device it
        // hosts). Selecting where a live owner's ability dispatches — and
        // forwarding to the owning device — is exactly what this resolver
        // is authoritative for, so the route is AuthoritativeLocal. The
        // placement gate already rejected offline owners with NOROUTE.
        let registry = PresenceRegistry::new();
        let catalog = AbilityCatalogStore::new();
        let owner_ura = device_owner_ura();
        mark_online(&registry, &owner_ura);
        publish_ability(&catalog, &owner_ura, &owner_ura, "agent", "list");

        // No `.with_local_runtime_authority(...)`: this resolver is not the
        // owner's own daemon, but it is still authoritative for routing.
        let route = DaemonRouteResolver::new(&registry, None, Some(&catalog))
            .at(TEST_NOW_MS)
            .resolve_route(&owner_ura, "agent.list")
            .expect("projection route resolves");

        assert_eq!(route.kind(), SelectedRouteKind::SameRealmDevice);
        assert_eq!(route.route_reason(), axon_pb::RouteReason::LocalDevice);
        assert_eq!(
            route.dispatch_target(false),
            SelectedRouteDispatchTarget::PresenceSession
        );
        assert_eq!(route.owner_ura, owner_ura);
        assert_eq!(route.callee_ura, owner_ura);
        assert_eq!(route.execution_host_ura, owner_ura);
        assert!(route.is_authoritative_local_or_better());
    }

    #[test]
    fn ability_without_route_summary_resolves_noroute() {
        let registry = PresenceRegistry::new();
        let catalog = AbilityCatalogStore::new();
        let owner_ura = device_owner_ura();
        mark_online(&registry, &owner_ura);
        let ability_ura = publish_ability_with_route_summary(
            &catalog, &owner_ura, &owner_ura, "agent", "list", false,
        );

        let failure = DaemonRouteResolver::new(&registry, None, Some(&catalog))
            .at(TEST_NOW_MS)
            .resolve_route(&owner_ura, "agent.list")
            .expect_err("ability without executable route must not be dispatchable");

        assert_eq!(failure.reason, axon_pb::NegativeReason::Noroute);
        assert_eq!(failure.query_name, format!("{owner_ura}#agent.list"));
        assert!(failure.detail.contains("route_summary_ref"));

        let answer = DaemonRouteResolver::new(&registry, None, Some(&catalog))
            .at(TEST_NOW_MS)
            .resolve_query_json(&json!({
                "qtype": axon_pb::ResolveType::DirectoryListing.as_str_name(),
                "queryName": owner_ura,
            }));
        let records = answer["records"].as_array().expect("records array");
        assert!(
            records.iter().any(|record| {
                record["recordType"] == axon_pb::RecordType::Ability.as_str_name()
                    && record["value"]["ability"]["abilityUra"] == ability_ura.as_str()
            }),
            "directory listing should retain the non-dispatchable ability fact"
        );
        assert!(
            records.iter().all(|record| {
                record["recordType"] != axon_pb::RecordType::Route.as_str_name()
                    || record["value"]["route"]["abilityUra"] != ability_ura.as_str()
            }),
            "directory listing must not manufacture a ROUTE record without route_summary_ref"
        );
    }

    #[test]
    fn hosted_agent_without_host_placement_resolves_noroute() {
        let registry = PresenceRegistry::new();
        let catalog = AbilityCatalogStore::new();
        let agent_ura = crate::ura::agent_ura("test-realm", "alice", "assistant");
        mark_online(&registry, &agent_ura);
        publish_ability(&catalog, &agent_ura, &agent_ura, "chat", "complete");

        let failure = DaemonRouteResolver::new(&registry, None, Some(&catalog))
            .at(TEST_NOW_MS)
            .resolve_route(&agent_ura, "chat.complete")
            .expect_err("hosted agent route must require selected host placement");

        assert_eq!(failure.reason, axon_pb::NegativeReason::Noroute);
        assert_eq!(failure.query_name, format!("{agent_ura}#chat.complete"));
        assert!(failure.detail.contains("host device"));
    }

    #[test]
    fn malformed_query_resolves_refused() {
        let registry = PresenceRegistry::new();
        let catalog = AbilityCatalogStore::new();

        // Empty owner + empty ability is not a valid route selector.
        let failure = DaemonRouteResolver::new(&registry, None, Some(&catalog))
            .at(TEST_NOW_MS)
            .resolve_route("", "")
            .expect_err("empty selector must be refused");

        assert_eq!(failure.reason, axon_pb::NegativeReason::Refused);
    }

    #[test]
    fn final_route_answer_json_shape_carries_required_keys() {
        let registry = PresenceRegistry::new();
        let catalog = AbilityCatalogStore::new();
        let owner_ura = device_owner_ura();
        mark_online(&registry, &owner_ura);
        let ability_ura =
            crate::ura::owner_ability_ura(&owner_ura, "agent.list").expect("ability ura");
        let authority = FakeLocalRuntimeAuthority::with_owner_keys(&owner_ura, &["agent.list"]);

        let answer = DaemonRouteResolver::new(&registry, None, Some(&catalog))
            .with_local_runtime_authority(owner_ura.clone(), authority)
            .at(TEST_NOW_MS)
            .resolve_route(&owner_ura, "agent.list")
            .expect("route resolves")
            .final_route_answer_json();

        assert_eq!(
            answer["answerKind"],
            axon_pb::ResolveAnswerKind::FinalRoute.as_str_name()
        );
        assert_eq!(answer["abilityUra"], ability_ura);
        assert_eq!(answer["ownerUra"], owner_ura);
        assert_eq!(answer["routeUra"], format!("route-ref::{ability_ura}"));
        assert_eq!(
            answer["releaseProfile"],
            axon_pb::ResolverReleaseProfile::AuthoritativeLocal.as_str_name()
        );
        // Required nested objects are present.
        assert!(answer.get("nextHop").is_some());
        assert!(answer["nextHop"]["localDeviceAbility"].is_object());
        assert!(answer.get("selectedRoute").is_some());
        assert!(answer["selectedRoute"]["nextHop"].is_object());
    }

    #[test]
    fn remote_owner_resolves_peer_hub_delegation_from_static_peer_map() {
        let registry = PresenceRegistry::new();
        let peers = SharedFederatedPeers::new(BTreeMap::from([(
            "remote-realm".to_string(),
            "https://remote-hub.example".to_string(),
        )]));
        let directory = SharedFederatedDirectoryView::default();
        let remote_owner = crate::ura::device_ura("remote-realm", "remote-device");
        let ability_ura =
            crate::ura::owner_ability_ura(&remote_owner, "observe.health").expect("ability ura");

        let delegation = DaemonRouteResolver::new(&registry, None, None)
            .with_peer_delegation("local-realm", &peers, &directory, false)
            .resolve_delegation(&ability_ura, "")
            .expect("delegation lookup succeeds")
            .expect("remote owner delegates to peer hub");

        assert_eq!(delegation.query_name, ability_ura);
        assert_eq!(delegation.owner_ura, remote_owner);
        assert_eq!(delegation.realm, "remote-realm");
        assert_eq!(delegation.hub_ura, crate::ura::hub_ura("remote-realm"));
        assert_eq!(
            delegation.primary_endpoint(),
            Some("https://remote-hub.example")
        );

        let answer = delegation.delegation_answer_json();
        assert_eq!(
            answer["answerKind"],
            axon_pb::ResolveAnswerKind::Delegation.as_str_name()
        );
        assert_eq!(
            answer["releaseProfile"],
            axon_pb::ResolverReleaseProfile::AuthoritativeLocal.as_str_name()
        );
        assert_eq!(answer["nextHop"]["peerHub"]["realm"], "remote-realm");
        assert_eq!(
            answer["nextHop"]["peerHub"]["endpoints"][0]["endpoint"],
            "https://remote-hub.example"
        );
        assert_eq!(
            answer["nextHop"]["peerHub"]["endpoints"][0]["metadata"]["source"],
            "federated_peers"
        );
        assert_eq!(
            answer["selectedRoute"]["reason"],
            axon_pb::RouteReason::PeerDelegation.as_str_name()
        );
        assert_eq!(
            answer["routeEvidence"]["selectionAlgorithm"],
            "daemon-peer-delegation-v1"
        );
    }

    #[test]
    fn directory_listing_emits_selected_route_record_per_ability() {
        // The catalog read model projects routes[] straight from the
        // directory listing, so every advertised ability must carry a
        // ROUTE record whose fields match what resolve_route would
        // select for the same (owner, ability) pair.
        let registry = PresenceRegistry::new();
        let catalog = AbilityCatalogStore::new();
        let owner_ura = device_owner_ura();
        mark_online(&registry, &owner_ura);
        let ability_ura = publish_ability(&catalog, &owner_ura, &owner_ura, "agent", "list");

        let resolver = DaemonRouteResolver::new(&registry, None, Some(&catalog)).at(TEST_NOW_MS);
        let answer = resolver.resolve_query_json(&json!({
            "qtype": axon_pb::ResolveType::DirectoryListing.as_str_name(),
            "queryName": owner_ura,
        }));

        let records = answer["records"].as_array().expect("records array");
        let route = records
            .iter()
            .find(|record| {
                record["recordType"] == axon_pb::RecordType::Route.as_str_name()
                    && record["value"]["route"]["abilityUra"] == ability_ura.as_str()
            })
            .expect("directory listing must carry a ROUTE record for the published ability");

        let route_value = &route["value"]["route"];
        assert_eq!(route_value["routeUra"], format!("route-ref::{ability_ura}"));
        assert_eq!(route_value["dispatchName"], "agent.list");
        assert_eq!(route_value["executeOn"]["targetUra"], owner_ura);

        // The directory route must equal what the single-route resolve
        // selects — one selection path, no divergence.
        let selected = resolver
            .resolve_route(&owner_ura, "agent.list")
            .expect("single-route resolve must succeed");
        assert_eq!(route_value["abilityUra"], selected.ability_ura.as_str());
        assert_eq!(route_value["routeUra"], selected.route_ura.as_str());
        assert_eq!(route_value["dispatchName"], selected.dispatch_name.as_str());
    }
}
