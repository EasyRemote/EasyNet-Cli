// EasyNet CLI — invocation_transport — daemon route resolver
// ==========================================================
//
// File: src/daemon/invocation/route_resolver.rs
// Description: Daemon-owned RFC-005 route resolver facade.
//
// Protocol Responsibility:
// - The daemon owns EasyNet resolver vocabulary and the runtime facts used to
//   select an executable route: live sessions, hosted-agent placement, and
//   owner ability projection.
// - Axon owns only the generic Invocation transport carrying resolver JSON.
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

use std::collections::HashMap;

use axon_sdk::invocation::CallMode;
use base64::{engine::general_purpose::URL_SAFE_NO_PAD, Engine as _};
use serde_json::{json, Value};

use crate::daemon::ability::catalog::publication::LocalAbilityPublicationSnapshot;
use crate::daemon::federation::directory::{DirectoryEntry, SharedFederatedDirectoryView};
use crate::daemon::federation::peers::SharedFederatedPeers;
use crate::daemon::federation::read_model::ability_catalog::AbilityCatalogStore;
use crate::daemon::federation::read_model::advertised_agents::AdvertisedAgentStore;
use crate::daemon::federation::resolver_contract::{
    GateResult, NegativeReason, RecordType, ResolveAnswerKind, ResolveType, ResolverReleaseProfile,
    RouteHealth, RouteReason, UraKind,
};
use crate::daemon::invocation::bidi::state::presence::PresenceRegistry;
use crate::daemon::invocation::dispatch::federation_wrappers::{
    self, ResolveAgentSummary, ResolveRequest,
};
use crate::daemon::invocation::routing::hub_resolver::{HubResolution, HubResolver};
use crate::daemon::persistence::agent_aggregate::{
    AgentAggregateRepository, AgentHostedPlacementProjection,
};

const DEFAULT_DIRECTORY_LIMIT: usize = 50;
const MAX_DIRECTORY_LIMIT: usize = 500;
const DIRECTORY_CURSOR_PREFIX: &str = "directory:v1:";
const MAX_DIRECTORY_CURSOR_LEN: usize = 4096;

/// Canonical typed request consumed by the daemon namespace resolver.
///
/// JSON is a public transport shape; it is parsed once into this object before
/// route selection so the resolver does not silently default tuple selectors
/// such as `query_name` to empty strings.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct NamespaceResolveQuery {
    query_name: String,
    ability_name: String,
    qtype: ResolveType,
    include_abilities: bool,
    limit: Option<u64>,
    cursor: Option<String>,
}

impl NamespaceResolveQuery {
    pub(crate) fn from_json(value: &Value) -> Result<Self, String> {
        let object = value
            .as_object()
            .ok_or_else(|| "namespace.resolve request must be a JSON object".to_string())?;
        let query_name = required_string(object, "query_name")?;
        let qtype_text = required_qtype_string(object)?;
        let qtype = ResolveType::from_str_name(&qtype_text).ok_or_else(|| {
            format!(
                "namespace.resolve qtype {qtype_text:?} is not a canonical ResolveType enum string"
            )
        })?;
        if qtype == ResolveType::Unspecified {
            return Err("namespace.resolve qtype must not be RESOLVE_TYPE_UNSPECIFIED".to_string());
        }
        let ability_name = optional_string(object, "ability_name")?.unwrap_or_default();
        let include_abilities = optional_bool(object, "include_abilities")?.unwrap_or(true);
        let limit = optional_u64(object, "limit")?;
        let cursor = optional_string(object, "cursor")?;
        Ok(Self {
            query_name,
            ability_name,
            qtype,
            include_abilities,
            limit,
            cursor,
        })
    }
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

/// Closed daemon-internal route-locality classification.
///
/// This is the semantic value dispatchers should consume. Axon
/// `RouteReason` is the product wire projection exposed through resolve JSON,
/// but the daemon must not infer route locality
/// from protobuf strings, owner URA shape, or ad hoc dispatcher branches.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum SelectedRouteKind {
    RealmAuthorityOwned,
    /// Agent-family ability owner selected as the routable callee.
    ///
    /// This covers ordinary hosted Agents and device-sponsored SystemAgents.
    /// The Device remains only the execution host/custody substrate, while the
    /// external Axon/product projection still reports `HostedAgent`.
    RoutableAgentOwned,
}

impl SelectedRouteKind {
    #[must_use]
    pub(crate) fn route_reason(self) -> RouteReason {
        match self {
            Self::RealmAuthorityOwned => RouteReason::LocalHub,
            Self::RoutableAgentOwned => RouteReason::HostedAgent,
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
    pub release_profile: ResolverReleaseProfile,
    kind: SelectedRouteKind,
    ability_record: Value,
    route_record: Value,
    owner_record: Value,
}

impl SelectedInvokeRoute {
    #[cfg(test)]
    pub(crate) fn test_local_runtime(callee_ura: &str, ability: &str, dispatch_name: &str) -> Self {
        let callee = crate::core::ura::parse_ura(callee_ura)
            .expect("test local runtime callee must be canonical");
        let (execution_host_ura, kind) = match callee.kind {
            crate::core::ura::URAKind::Authority => (
                callee_ura.to_string(),
                SelectedRouteKind::RealmAuthorityOwned,
            ),
            crate::core::ura::URAKind::Agent => {
                let (device_id, _) = callee.device_agent_ids().expect(
                    "test local runtime Agent callee must be a device-sponsored SystemAgent",
                );
                (
                    crate::core::ura::device_ura(&callee.realm, device_id),
                    SelectedRouteKind::RoutableAgentOwned,
                )
            }
            crate::core::ura::URAKind::Device => {
                panic!("positive local runtime routes must not use Device as callee")
            }
            other => panic!("unsupported test local runtime callee kind: {other:?}"),
        };
        let ability_ura = crate::core::ura::owner_ability_ura(callee_ura, ability)
            .expect("test ability URA must be canonical");
        let route_ura = format!("route-ref::{ability_ura}");
        let now_unix_ms = 0;
        Self {
            query_name: ability.to_string(),
            owner_ura: callee_ura.to_string(),
            callee_ura: callee_ura.to_string(),
            execution_host_ura: execution_host_ura.clone(),
            host_node_id: None,
            ability_ura: ability_ura.clone(),
            route_ura: route_ura.clone(),
            dispatch_name: dispatch_name.to_string(),
            release_profile: ResolverReleaseProfile::AuthoritativeLocal,
            kind,
            ability_record: local_runtime_ability_record(
                &ability_ura,
                callee_ura,
                ability,
                now_unix_ms,
            ),
            route_record: route_record(
                &route_ura,
                &ability_ura,
                dispatch_name,
                callee_ura,
                &execution_host_ura,
                None,
                now_unix_ms,
            ),
            owner_record: id_record(callee_ura, now_unix_ms),
        }
    }

    #[must_use]
    pub(crate) fn is_authoritative_local_or_better(&self) -> bool {
        matches!(
            self.release_profile,
            ResolverReleaseProfile::AuthoritativeLocal | ResolverReleaseProfile::Production
        )
    }

    #[must_use]
    #[cfg(test)]
    pub(crate) fn dispatch_key(&self) -> String {
        crate::core::ura::local_dispatch_ability_key(&self.callee_ura, &self.dispatch_name)
    }

    #[must_use]
    pub(crate) fn kind(&self) -> SelectedRouteKind {
        self.kind
    }

    #[must_use]
    pub(crate) fn route_reason(&self) -> RouteReason {
        self.kind().route_reason()
    }

    #[must_use]
    pub(crate) fn dispatch_target(
        &self,
        execution_host_is_self: bool,
    ) -> SelectedRouteDispatchTarget {
        match self.kind {
            SelectedRouteKind::RealmAuthorityOwned | SelectedRouteKind::RoutableAgentOwned => {
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
            "authority": GateResult::Pass.as_str_name(),
            "identity": GateResult::Pass.as_str_name(),
            "placement": GateResult::Pass.as_str_name(),
            "ability": GateResult::Pass.as_str_name(),
            "policy": GateResult::Pass.as_str_name(),
        });
        let next_hop = self.next_hop_json();
        let selected_route = json!({
            "next_hop": next_hop.clone(),
            "priority": 0,
            "weight": 1,
            "reason": self.route_reason().as_str_name(),
            "health": RouteHealth::Healthy.as_str_name(),
            "authority": authority.clone(),
            "gates": gates,
        });

        json!({
            "answer_kind": ResolveAnswerKind::FinalRoute.as_str_name(),
            "canonical_name": self.query_name,
            "owner_ura": self.owner_ura,
            "ability_ura": self.ability_ura,
            "route_ura": self.route_ura,
            "next_hop": next_hop,
            "selected_route": selected_route.clone(),
            "route_candidates": [selected_route],
            "route_evidence": {
                "identity": self.owner_record.clone(),
                "owner": self.owner_record.clone(),
                "ability": self.ability_record.clone(),
                "route": self.route_record.clone(),
                "selection_algorithm": "daemon-selected-route-v1",
            },
            "records": [self.ability_record.clone(), self.route_record.clone()],
            "release_profile": self.release_profile.as_str_name(),
            "authority": authority,
            "cache_policy": cache_policy_json(),
        })
    }

    pub(crate) fn from_hub_final_route_answer_json(
        answer: &Value,
        target_ura: &str,
        ability_ura: &str,
    ) -> Result<Self, ResolveRouteFailure> {
        let selector = match route_selector_from_query(ability_ura, "")? {
            Some(selector) => selector,
            None => route_selector_from_query(target_ura, ability_ura)?.ok_or_else(|| {
                ResolveRouteFailure::new(
                    ability_ura,
                    NegativeReason::Refused,
                    "Hub route provider requires a full canonical ability URA, target-bound descriptor ref, or an owner-local ability name with an explicit callee",
                )
            })?,
        };
        let answer_kind = json_string(answer, "answer_kind", &selector.query_name)?;
        if ResolveAnswerKind::from_str_name(answer_kind) != Some(ResolveAnswerKind::FinalRoute) {
            return Err(ResolveRouteFailure::new(
                selector.query_name,
                NegativeReason::Noroute,
                format!("Hub route provider returned non-final answer_kind `{answer_kind}`"),
            ));
        }
        let release_profile_text = json_string(answer, "release_profile", &selector.query_name)?;
        let release_profile = ResolverReleaseProfile::from_str_name(release_profile_text)
            .ok_or_else(|| {
                ResolveRouteFailure::new(
                    selector.query_name.clone(),
                    NegativeReason::Refused,
                    format!(
                        "Hub route provider returned noncanonical release_profile `{release_profile_text}`"
                    ),
                )
            })?;
        if !matches!(
            release_profile,
            ResolverReleaseProfile::AuthoritativeLocal | ResolverReleaseProfile::Production
        ) {
            return Err(ResolveRouteFailure::new(
                selector.query_name,
                NegativeReason::Noroute,
                format!(
                    "Hub route provider returned non-dispatchable release_profile `{release_profile_text}`"
                ),
            ));
        }
        let owner_ura = json_string(answer, "owner_ura", &selector.query_name)?.to_string();
        let answer_ability_ura =
            json_string(answer, "ability_ura", &selector.query_name)?.to_string();
        let route_ura = json_string(answer, "route_ura", &selector.query_name)?.to_string();
        if owner_ura != selector.owner_ura {
            return Err(ResolveRouteFailure::new(
                selector.query_name,
                NegativeReason::Refused,
                format!(
                    "Hub route owner `{owner_ura}` does not match selected target owner `{}`",
                    selector.owner_ura
                ),
            ));
        }
        if answer_ability_ura != selector.ability_ura {
            return Err(ResolveRouteFailure::new(
                selector.query_name,
                NegativeReason::Refused,
                format!(
                    "Hub route ability `{answer_ability_ura}` does not match requested ability `{}`",
                    selector.ability_ura
                ),
            ));
        }

        let next_hop = answer
            .get("selected_route")
            .and_then(|selected| selected.get("next_hop"))
            .or_else(|| answer.get("next_hop"))
            .ok_or_else(|| {
                ResolveRouteFailure::new(
                    selector.query_name.clone(),
                    NegativeReason::Noroute,
                    "Hub final route omitted selected_route.next_hop",
                )
            })?;
        let (kind, callee_ura, execution_host_ura, host_node_id, dispatch_name) =
            parse_final_route_next_hop(next_hop, &selector.query_name, &answer_ability_ura)?;
        if route_ura != json_string_from_next_hop(next_hop, "route_ura", &selector.query_name)? {
            return Err(ResolveRouteFailure::new(
                selector.query_name,
                NegativeReason::Refused,
                "Hub final route route_ura does not match selected next_hop route_ura",
            ));
        }
        let target_matches = owner_ura == target_ura || execution_host_ura == target_ura;
        if !target_matches {
            return Err(ResolveRouteFailure::new(
                selector.query_name,
                NegativeReason::Refused,
                route_owner_mismatch_detail(&execution_host_ura, &answer_ability_ura, target_ura),
            ));
        }
        let route_evidence = answer.get("route_evidence").ok_or_else(|| {
            ResolveRouteFailure::new(
                answer_ability_ura.clone(),
                NegativeReason::Nodata,
                "Hub final route omitted route_evidence",
            )
        })?;
        let owner_record = route_evidence
            .get("identity")
            .or_else(|| route_evidence.get("owner"))
            .cloned()
            .ok_or_else(|| {
                ResolveRouteFailure::new(
                    answer_ability_ura.clone(),
                    NegativeReason::Nodata,
                    "Hub final route omitted owner identity evidence",
                )
            })?;
        let ability_record = route_evidence.get("ability").cloned().ok_or_else(|| {
            ResolveRouteFailure::new(
                answer_ability_ura.clone(),
                NegativeReason::Nodata,
                "Hub final route omitted ability evidence",
            )
        })?;
        let route_record = route_evidence.get("route").cloned().ok_or_else(|| {
            ResolveRouteFailure::new(
                answer_ability_ura.clone(),
                NegativeReason::Nodata,
                "Hub final route omitted route evidence",
            )
        })?;

        Ok(Self {
            query_name: selector.query_name,
            owner_ura,
            callee_ura,
            execution_host_ura,
            host_node_id,
            ability_ura: answer_ability_ura,
            route_ura,
            dispatch_name,
            release_profile,
            kind,
            ability_record,
            route_record,
            owner_record,
        })
    }

    fn next_hop_json(&self) -> Value {
        match self.kind {
            SelectedRouteKind::RealmAuthorityOwned => json!({
                "local_hub_ability": {
                    "ability_ura": self.ability_ura,
                    "route_ura": self.route_ura,
                    "dispatch_name": self.dispatch_name,
                }
            }),
            SelectedRouteKind::RoutableAgentOwned => json!({
                "hosted_agent_via_device": {
                    "agent_ura": self.callee_ura,
                    "host_device_ura": self.execution_host_ura,
                    "host_node_id": self.host_node_id.as_deref().unwrap_or_default(),
                    "ability_ura": self.ability_ura,
                    "route_ura": self.route_ura,
                    "dispatch_name": self.dispatch_name,
                }
            }),
        }
    }
}

fn json_string<'a>(
    value: &'a Value,
    field: &str,
    query_name: &str,
) -> Result<&'a str, ResolveRouteFailure> {
    value
        .get(field)
        .and_then(Value::as_str)
        .map(str::trim)
        .filter(|text| !text.is_empty())
        .ok_or_else(|| {
            ResolveRouteFailure::new(
                query_name,
                NegativeReason::Nodata,
                format!("Hub final route missing canonical `{field}`"),
            )
        })
}

fn json_string_from_next_hop<'a>(
    next_hop: &'a Value,
    field: &str,
    query_name: &str,
) -> Result<&'a str, ResolveRouteFailure> {
    next_hop
        .as_object()
        .and_then(|object| object.values().next())
        .and_then(|payload| payload.get(field))
        .and_then(Value::as_str)
        .map(str::trim)
        .filter(|text| !text.is_empty())
        .ok_or_else(|| {
            ResolveRouteFailure::new(
                query_name,
                NegativeReason::Nodata,
                format!("Hub final route next_hop missing canonical `{field}`"),
            )
        })
}

fn parse_final_route_next_hop(
    next_hop: &Value,
    query_name: &str,
    expected_ability_ura: &str,
) -> Result<(SelectedRouteKind, String, String, Option<String>, String), ResolveRouteFailure> {
    let object = next_hop.as_object().ok_or_else(|| {
        ResolveRouteFailure::new(
            query_name,
            NegativeReason::Nodata,
            "Hub final route next_hop must be an object",
        )
    })?;
    if object.len() != 1 {
        return Err(ResolveRouteFailure::new(
            query_name,
            NegativeReason::Refused,
            "Hub final route next_hop must contain exactly one dispatch arm",
        ));
    }
    if let Some(device) = object.get("device_hosted_ability") {
        let host_device_ura = json_string(device, "host_device_ura", query_name)?.to_string();
        let callee_ura = json_string(device, "callee_ura", query_name)?.to_string();
        let ability_ura = json_string(device, "ability_ura", query_name)?;
        if ability_ura != expected_ability_ura {
            return Err(ResolveRouteFailure::new(
                query_name,
                NegativeReason::Refused,
                format!(
                    "Hub final route device_hosted_ability ability `{ability_ura}` does not match selected ability `{expected_ability_ura}`"
                ),
            ));
        }
        let (kind, callee_ura) =
            device_hosted_next_hop_callee(query_name, &host_device_ura, &callee_ura, ability_ura)?;
        let dispatch_name = json_string(device, "dispatch_name", query_name)?.to_string();
        return Ok((kind, callee_ura, host_device_ura, None, dispatch_name));
    }
    if let Some(hub) = object.get("local_hub_ability") {
        let ability_ura = json_string(hub, "ability_ura", query_name)?;
        if ability_ura != expected_ability_ura {
            return Err(ResolveRouteFailure::new(
                query_name,
                NegativeReason::Refused,
                format!(
                    "Hub final route local_hub_ability ability `{ability_ura}` does not match selected ability `{expected_ability_ura}`"
                ),
            ));
        }
        let owner_ura = crate::core::ura::AbilitySelector::parse(ability_ura)
            .map_err(|error| {
                ResolveRouteFailure::new(
                    query_name,
                    NegativeReason::Refused,
                    format!("Hub final route hub ability selector parse failed: {error}"),
                )
            })?
            .owner_ura()
            .to_string();
        let dispatch_name = json_string(hub, "dispatch_name", query_name)?.to_string();
        return Ok((
            SelectedRouteKind::RealmAuthorityOwned,
            owner_ura.clone(),
            owner_ura,
            None,
            dispatch_name,
        ));
    }
    if let Some(agent) = object.get("hosted_agent_via_device") {
        let agent_ura = json_string(agent, "agent_ura", query_name)?.to_string();
        let ability_ura = json_string(agent, "ability_ura", query_name)?;
        if ability_ura != expected_ability_ura {
            return Err(ResolveRouteFailure::new(
                query_name,
                NegativeReason::Refused,
                format!(
                    "Hub final route hosted_agent_via_device ability `{ability_ura}` does not match selected ability `{expected_ability_ura}`"
                ),
            ));
        }
        let host_device_ura = json_string(agent, "host_device_ura", query_name)?.to_string();
        let host_node_id = agent
            .get("host_node_id")
            .and_then(Value::as_str)
            .map(str::trim)
            .filter(|text| !text.is_empty())
            .map(ToOwned::to_owned);
        let dispatch_name = json_string(agent, "dispatch_name", query_name)?.to_string();
        return Ok((
            SelectedRouteKind::RoutableAgentOwned,
            agent_ura,
            host_device_ura,
            host_node_id,
            dispatch_name,
        ));
    }
    Err(ResolveRouteFailure::new(
        query_name,
        NegativeReason::Noroute,
        "Hub final route next_hop did not select a daemon-dispatchable route",
    ))
}

fn device_hosted_next_hop_callee(
    query_name: &str,
    host_device_ura: &str,
    explicit_callee_ura: &str,
    ability_ura: &str,
) -> Result<(SelectedRouteKind, String), ResolveRouteFailure> {
    let device = crate::core::ura::parse_ura(host_device_ura).map_err(|error| {
        ResolveRouteFailure::new(
            query_name,
            NegativeReason::Refused,
            format!("Hub final route device_hosted_ability host_device_ura is invalid: {error}"),
        )
    })?;
    if device.kind != crate::core::ura::URAKind::Device {
        return Err(ResolveRouteFailure::new(
            query_name,
            NegativeReason::Refused,
            format!(
                "Hub final route device_hosted_ability host must be a Device URA, got {:?}",
                device.kind
            ),
        ));
    }
    let Some(host_device_id) = device.device_id() else {
        return Err(ResolveRouteFailure::new(
            query_name,
            NegativeReason::Refused,
            "Hub final route device_hosted_ability host Device is missing device id",
        ));
    };
    let selector = crate::core::ura::AbilitySelector::parse(ability_ura).map_err(|error| {
        ResolveRouteFailure::new(
            query_name,
            NegativeReason::Refused,
            format!("Hub final route device_hosted_ability ability selector parse failed: {error}"),
        )
    })?;
    let callee_ura = selector.owner_ura().to_string();
    if callee_ura != explicit_callee_ura {
        return Err(ResolveRouteFailure::new(
            query_name,
            NegativeReason::Refused,
            format!(
                "Hub final route device_hosted_ability callee `{explicit_callee_ura}` does not own ability `{ability_ura}`"
            ),
        ));
    }
    let callee = crate::core::ura::parse_ura(&callee_ura).map_err(|error| {
        ResolveRouteFailure::new(
            query_name,
            NegativeReason::Refused,
            format!("Hub final route device_hosted_ability callee owner parse failed: {error}"),
        )
    })?;
    match callee.kind {
        crate::core::ura::URAKind::Device => Err(ResolveRouteFailure::new(
            query_name,
            NegativeReason::Refused,
            format!(
                "Hub final route direct Device ability owner `{callee_ura}` is not a routable public callee; publish the ability under a device-sponsored SystemAgent hosted by `{host_device_ura}`"
            ),
        )),
        crate::core::ura::URAKind::Agent => {
            let Some((owner_device_id, _system_agent_id)) = callee.device_agent_ids() else {
                return Err(ResolveRouteFailure::new(
                    query_name,
                    NegativeReason::Refused,
                    "Hub final route device_hosted_ability must use hosted_agent_via_device for user-hosted Agent callees",
                ));
            };
            if callee.realm != device.realm || owner_device_id != host_device_id {
                return Err(ResolveRouteFailure::new(
                    query_name,
                    NegativeReason::Refused,
                    format!(
                        "Hub final route SystemAgent callee `{callee_ura}` is not sponsored by execution host `{host_device_ura}`"
                    ),
                ));
            }
            Ok((SelectedRouteKind::RoutableAgentOwned, callee_ura))
        }
        other => Err(ResolveRouteFailure::new(
            query_name,
            NegativeReason::Refused,
            format!(
                "Hub final route device_hosted_ability cannot dispatch ability owner kind {other:?} through a Device execution arm"
            ),
        )),
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct DelegatedInvokeRoute {
    pub query_name: String,
    pub owner_ura: String,
    pub realm: String,
    pub hub_ura: String,
    pub endpoints: Vec<DelegatedPeerEndpoint>,
    pub release_profile: ResolverReleaseProfile,
}

/// Resolver-owned dispatch branch for one canonical Invocation.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) enum CanonicalRouteDispatch {
    Local(SelectedInvokeRoute),
    Peer(DelegatedInvokeRoute),
    HubSession(SelectedInvokeRoute),
}

/// Call-mode-bound result of the daemon's canonical route policy.
///
/// Owner/target validation, local route selection, and cross-realm delegation
/// run once regardless of carrier. The selected Axon call mode then travels
/// with the route so descriptor binding and carrier construction cannot drift
/// from the policy decision made at resolution time.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct CanonicalRouteSelection {
    call_mode: CallMode,
    dispatch: CanonicalRouteDispatch,
}

impl CanonicalRouteSelection {
    fn local(call_mode: CallMode, route: SelectedInvokeRoute) -> Self {
        Self {
            call_mode,
            dispatch: CanonicalRouteDispatch::Local(route),
        }
    }

    fn peer(call_mode: CallMode, route: DelegatedInvokeRoute) -> Self {
        Self {
            call_mode,
            dispatch: CanonicalRouteDispatch::Peer(route),
        }
    }

    pub(crate) fn hub_session(call_mode: CallMode, route: SelectedInvokeRoute) -> Self {
        Self {
            call_mode,
            dispatch: CanonicalRouteDispatch::HubSession(route),
        }
    }

    #[must_use]
    pub(crate) fn call_mode(&self) -> CallMode {
        self.call_mode
    }

    #[must_use]
    pub(crate) fn dispatch(&self) -> &CanonicalRouteDispatch {
        &self.dispatch
    }

    #[must_use]
    pub(crate) fn into_dispatch(self) -> CanonicalRouteDispatch {
        self.dispatch
    }
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
            "next_hop": next_hop.clone(),
            "priority": 0,
            "weight": 1,
            "reason": RouteReason::PeerDelegation.as_str_name(),
            "health": RouteHealth::Healthy.as_str_name(),
            "authority": authority.clone(),
            "gates": {
                "authority": GateResult::Pass.as_str_name(),
                "identity": GateResult::NotApplicable.as_str_name(),
                "placement": GateResult::Pass.as_str_name(),
                "ability": GateResult::NotApplicable.as_str_name(),
                "policy": GateResult::Pass.as_str_name(),
            },
        });

        json!({
            "answer_kind": ResolveAnswerKind::Delegation.as_str_name(),
            "canonical_name": self.query_name,
            "owner_ura": self.owner_ura,
            "next_hop": next_hop,
            "selected_route": selected_route.clone(),
            "route_candidates": [selected_route],
            "route_evidence": {
                "owner": {
                    "ura": self.owner_ura,
                    "realm": self.realm,
                },
                "route": {
                    "hub_ura": self.hub_ura,
                    "endpoints": self.endpoints.iter().map(DelegatedPeerEndpoint::json).collect::<Vec<_>>(),
                },
                "selection_algorithm": "daemon-peer-delegation-v1",
            },
            "records": [],
            "release_profile": self.release_profile.as_str_name(),
            "authority": authority,
            "cache_policy": cache_policy_json(),
        })
    }

    fn next_hop_json(&self) -> Value {
        json!({
            "peer_hub": {
                "realm": self.realm,
                "hub_ura": self.hub_ura,
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
                "target_ura".to_string(),
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
    pub reason: NegativeReason,
    pub kind: ResolveRouteFailureKind,
    pub detail: String,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum ResolveRouteFailureKind {
    Generic,
    OwnerOffline,
}

impl ResolveRouteFailure {
    pub(crate) fn new(
        query_name: impl Into<String>,
        reason: NegativeReason,
        detail: impl Into<String>,
    ) -> Self {
        Self {
            query_name: query_name.into(),
            reason,
            kind: ResolveRouteFailureKind::Generic,
            detail: detail.into(),
        }
    }

    pub(crate) fn owner_offline(query_name: impl Into<String>, reason: NegativeReason) -> Self {
        debug_assert!(
            matches!(reason, NegativeReason::Nxdomain | NegativeReason::Noroute),
            "owner-offline route negatives must preserve absence/placement reason"
        );
        Self {
            query_name: query_name.into(),
            reason,
            kind: ResolveRouteFailureKind::OwnerOffline,
            detail: "owner is not online".to_string(),
        }
    }

    #[must_use]
    pub(crate) fn is_owner_offline(&self) -> bool {
        self.kind == ResolveRouteFailureKind::OwnerOffline
    }

    #[must_use]
    pub(crate) fn answer_json(&self) -> Value {
        negative_answer_json(&self.query_name, self.reason, Some(self.detail.as_str()))
    }
}

pub(crate) struct DaemonRouteResolver<'a> {
    registry: &'a PresenceRegistry,
    advertised_agents: Option<&'a AdvertisedAgentStore>,
    catalog: &'a AbilityCatalogStore,
    peer_delegation: Option<PeerDelegationSource<'a>>,
    federated_directory: Option<&'a SharedFederatedDirectoryView>,
    device_local: Option<LocalNamespaceAuthoritySource>,
    now_unix_ms: i64,
}

struct PeerDelegationSource<'a> {
    local_realm: &'a str,
    federated_peers: &'a SharedFederatedPeers,
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
    authority: LocalAbilityPublicationSnapshot,
    hosted_agents: LocalHostedAgentPlacements,
}

impl LocalNamespaceAuthoritySource {
    fn resolve_owner_ability(
        &self,
        owner_ura: &str,
        public_name: &str,
    ) -> Option<LocalRuntimeAbility> {
        if !self.authority.resolves(owner_ura, public_name) {
            return None;
        }
        let dispatch_name = crate::core::ura::local_dispatch_ability_key(owner_ura, public_name);
        (!dispatch_name.is_empty()).then_some(LocalRuntimeAbility { dispatch_name })
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
struct HostedAgentPlacement {
    host_device_ura: String,
    host_node_id: Option<String>,
}

#[derive(Debug, Clone, Default, PartialEq, Eq)]
struct LocalHostedAgentPlacements {
    by_agent_ura: HashMap<String, HostedAgentPlacement>,
    state: HostedPlacementProjectionState,
}

impl LocalHostedAgentPlacements {
    fn load() -> Self {
        match AgentAggregateRepository::try_load_snapshot() {
            Ok(snapshot) => match snapshot.hosted_agent_placements() {
                Ok(projection) => Self::from_projection(projection),
                Err(error) => Self::unavailable(format!("{error:#}")),
            },
            Err(error) => Self::unavailable(format!("{error:#}")),
        }
    }

    fn from_projection(projection: AgentHostedPlacementProjection) -> Self {
        Self {
            by_agent_ura: projection
                .by_agent_ura
                .into_iter()
                .map(|(agent_ura, placement)| {
                    (
                        agent_ura,
                        HostedAgentPlacement {
                            host_device_ura: placement.host_device_ura,
                            host_node_id: placement.host_node_id,
                        },
                    )
                })
                .collect(),
            state: HostedPlacementProjectionState::Available,
        }
    }

    fn unavailable(reason: String) -> Self {
        crate::op_event!(
            component = daemon_invocation,
            kind = route_resolver_agent_placement_projection_unavailable,
            error = reason.as_str(),
            message = "route_resolver: hosted Agent placement matching failed closed because the Agent aggregate snapshot could not be loaded or projected",
        );
        Self {
            by_agent_ura: HashMap::new(),
            state: HostedPlacementProjectionState::Unavailable { reason },
        }
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
            state: HostedPlacementProjectionState::Available,
        }
    }

    fn local_host_for(
        &self,
        agent_ura: &str,
        self_device_ura: &str,
    ) -> Option<&HostedAgentPlacement> {
        if matches!(
            self.state,
            HostedPlacementProjectionState::Unavailable { .. }
        ) {
            return None;
        }
        let placement = self.by_agent_ura.get(agent_ura)?;
        (placement.host_device_ura == self_device_ura).then_some(placement)
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Default)]
enum HostedPlacementProjectionState {
    #[default]
    Available,
    Unavailable {
        reason: String,
    },
}

impl<'a> DaemonRouteResolver<'a> {
    #[must_use]
    pub(crate) fn new(
        registry: &'a PresenceRegistry,
        advertised_agents: Option<&'a AdvertisedAgentStore>,
        catalog: &'a AbilityCatalogStore,
    ) -> Self {
        Self {
            registry,
            advertised_agents,
            catalog,
            peer_delegation: None,
            federated_directory: None,
            device_local: None,
            now_unix_ms: crate::daemon::federation::directory::now_unix_ms(),
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
    pub(crate) fn with_local_catalog_authority(
        mut self,
        local_authority_ura: impl Into<String>,
        authority: LocalAbilityPublicationSnapshot,
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
    ) -> Self {
        self.peer_delegation = Some(PeerDelegationSource {
            local_realm,
            federated_peers,
        });
        self
    }

    /// Attach the daemon-wide federated directory snapshot used by
    /// `federation.discover`.
    ///
    /// Directory entries are read-model observations only. They may enrich
    /// `namespace.resolve` DirectoryListing with peer identity records; they
    /// must not manufacture ROUTE records or bypass peer delegation for
    /// executable dispatch.
    #[must_use]
    pub(crate) fn with_federated_directory(
        mut self,
        federated_directory: &'a SharedFederatedDirectoryView,
    ) -> Self {
        self.federated_directory = Some(federated_directory);
        self
    }

    #[must_use]
    pub(crate) fn at(mut self, now_unix_ms: i64) -> Self {
        self.now_unix_ms = now_unix_ms;
        self
    }

    pub(crate) fn resolve_query_json(&self, query: &Value) -> Value {
        let query = match NamespaceResolveQuery::from_json(query) {
            Ok(query) => query,
            Err(detail) => {
                let query_name = query
                    .get("query_name")
                    .and_then(Value::as_str)
                    .map(str::trim)
                    .unwrap_or_default();
                return negative_answer_json(query_name, NegativeReason::Refused, Some(&detail));
            }
        };
        self.resolve_query(&query)
    }

    pub(crate) fn resolve_query(&self, query: &NamespaceResolveQuery) -> Value {
        match query.qtype {
            ResolveType::Route | ResolveType::Ability => {
                if let Some(answer) =
                    self.delegation_answer_or_negative(&query.query_name, &query.ability_name)
                {
                    answer
                } else {
                    self.resolve_route(&query.query_name, &query.ability_name)
                        .map_or_else(
                            |failure| failure.answer_json(),
                            |route| route.final_route_answer_json(),
                        )
                }
            }
            ResolveType::DirectoryListing | ResolveType::CanonicalIdentity | ResolveType::Owner => {
                self.directory_answer(query)
            }
            ResolveType::Key | ResolveType::Service => negative_answer_json(
                &query.query_name,
                NegativeReason::Nodata,
                Some("daemon namespace.resolve does not serve this qtype yet"),
            ),
            ResolveType::Unspecified => negative_answer_json(
                &query.query_name,
                NegativeReason::Refused,
                Some("resolve qtype is unspecified"),
            ),
        }
    }

    pub(crate) fn resolve_route(
        &self,
        query_name: &str,
        ability_name: &str,
    ) -> Result<SelectedInvokeRoute, ResolveRouteFailure> {
        let selector = route_selector_from_query(query_name, ability_name)?.ok_or_else(|| {
            ResolveRouteFailure::new(
                query_name,
                NegativeReason::Refused,
                "route query must provide an owner URA plus ability_name or a full ability URA",
            )
        })?;

        if selector.owner_kind == RouteOwnerKind::LegacyDeviceProfileProjection {
            return Err(ResolveRouteFailure::new(
                selector.query_name,
                NegativeReason::Refused,
                "Device-owned ability selectors are migration read-models only; public invocation must target an Authority, Agent, or device-sponsored SystemAgent",
            ));
        }

        // RFC-005 §4 / D105: this daemon is authoritative for the
        // executable surface it hosts locally. Device remains the authority
        // root and execution substrate; public descriptors are owned by an
        // Authority, Agent, or declared device-sponsored SystemAgent.
        if let Some(device_local) = self.device_local.as_ref() {
            if device_local.local_authority_ura == selector.owner_ura {
                return self.resolve_route_from_local_runtime(
                    &selector,
                    device_local,
                    device_local.local_authority_ura.as_str(),
                    None,
                    SelectedRouteKind::RealmAuthorityOwned,
                );
            }

            if selector.owner_kind == RouteOwnerKind::Authority
                && device_local
                    .resolve_owner_ability(&selector.owner_ura, &selector.public_name)
                    .is_some()
            {
                return self.resolve_route_from_local_runtime(
                    &selector,
                    device_local,
                    selector.owner_ura.as_str(),
                    None,
                    SelectedRouteKind::RealmAuthorityOwned,
                );
            }

            if let Some(host_node_id) =
                self.local_host_node_id_for_agent(&selector, device_local)?
            {
                return self.resolve_route_from_local_runtime(
                    &selector,
                    device_local,
                    device_local.local_authority_ura.as_str(),
                    host_node_id.as_deref(),
                    SelectedRouteKind::RoutableAgentOwned,
                );
            }

            if matches!(
                selector.owner_kind,
                RouteOwnerKind::Agent | RouteOwnerKind::SystemAgent
            ) && device_local
                .resolve_owner_ability(&selector.owner_ura, &selector.public_name)
                .is_some()
            {
                return self.resolve_route_from_local_runtime(
                    &selector,
                    device_local,
                    device_local.local_authority_ura.as_str(),
                    None,
                    SelectedRouteKind::RoutableAgentOwned,
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
        if !crate::daemon::ability::catalog::is_local_runtime_routable_catalog_name(
            &selector.public_name,
        ) {
            return Err(ResolveRouteFailure::new(
                selector.query_name.clone(),
                NegativeReason::Nodata,
                "daemon-local ability is not routable through the public Invocation surface",
            ));
        }
        let ability = device_local
            .resolve_owner_ability(&selector.owner_ura, &selector.public_name)
            .ok_or_else(|| {
                ResolveRouteFailure::new(
                    selector.query_name.clone(),
                    NegativeReason::Nodata,
                    "local runtime does not register a dispatchable route for this ability",
                )
            })?;

        let route_ura = format!("route-ref::{}", selector.ability_ura);
        let owner_record = id_record(&selector.owner_ura, self.now_unix_ms);
        let ability_record = local_runtime_ability_record(
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
            execution_host_ura,
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
            release_profile: ResolverReleaseProfile::AuthoritativeLocal,
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
    ) -> Result<Option<Option<String>>, ResolveRouteFailure> {
        if selector.owner_kind != RouteOwnerKind::Agent {
            return Ok(None);
        }
        let parsed = crate::core::ura::parse_ura(&selector.owner_ura).map_err(|err| {
            ResolveRouteFailure::new(
                selector.query_name.clone(),
                NegativeReason::Refused,
                format!("selector owner URA is invalid: {err}"),
            )
        })?;

        if let Some(placement) = device_local
            .hosted_agents
            .local_host_for(&selector.owner_ura, &device_local.local_authority_ura)
        {
            return Ok(Some(placement.host_node_id.clone()));
        }

        let hosted_by_this_device = parsed
            .device_agent_ids()
            .and_then(|(device_id, _)| {
                device_id_from_device_ura(&device_local.local_authority_ura)
                    .filter(|self_id| self_id.as_str() == device_id)
            })
            .is_some();
        Ok(hosted_by_this_device
            .then(|| device_id_from_device_ura(&device_local.local_authority_ura)))
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
            &ResolveRequest::with_filter(Some(selector.owner_ura.clone()), true),
            self.registry,
            self.advertised_agents,
            self.catalog,
            self.device_local.as_ref().map(|source| &source.authority),
            self.now_unix_ms,
        )
        .map_err(|detail| {
            ResolveRouteFailure::new(
                selector.query_name.clone(),
                NegativeReason::Refused,
                format!("ability projection unavailable: {detail}"),
            )
        })?;

        let owner = directory
            .agents
            .iter()
            .find(|agent| agent.ura == selector.owner_ura)
            .ok_or_else(|| {
                let reason =
                    if advertised_agent_host_ura(self.advertised_agents, &selector.owner_ura)
                        .is_some()
                    {
                        NegativeReason::Noroute
                    } else {
                        NegativeReason::Nxdomain
                    };
                ResolveRouteFailure::owner_offline(selector.query_name.clone(), reason)
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
            .ok_or_else(|| {
                ResolveRouteFailure::new(
                    selector.query_name.clone(),
                    NegativeReason::Nodata,
                    "owner is online but does not publish the requested ability",
                )
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
            ability_record_from_summary(summary, self.now_unix_ms).map_err(|detail| {
                ResolveRouteFailure::new(
                    selector.query_name.clone(),
                    NegativeReason::Noroute,
                    detail,
                )
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
            release_profile: ResolverReleaseProfile::AuthoritativeLocal,
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
        let selector = route_selector_from_query(query_name, ability_name)?.ok_or_else(|| {
            ResolveRouteFailure::new(
                query_name,
                NegativeReason::Refused,
                "route query must provide an owner URA plus ability_name or a full ability URA",
            )
        })?;
        let Some(peer_source) = self.peer_delegation.as_ref() else {
            return Ok(None);
        };
        let parsed_owner = crate::core::ura::parse_ura(&selector.owner_ura).map_err(|err| {
            ResolveRouteFailure::new(
                selector.query_name.clone(),
                NegativeReason::Refused,
                format!("owner URA is invalid: {err}"),
            )
        })?;
        if parsed_owner.realm == peer_source.local_realm {
            return Ok(None);
        }

        let resolution = HubResolver::new(peer_source.federated_peers).resolve(&parsed_owner.realm);
        let endpoint = match resolution {
            HubResolution::Static { hub_endpoint } => {
                DelegatedPeerEndpoint::new(hub_endpoint, "federated_peers", None)
            }
            HubResolution::Offline => {
                return Err(ResolveRouteFailure::new(
                    selector.query_name,
                    NegativeReason::Noroute,
                    format!(
                        "remote realm `{}` has no configured peer hub route",
                        parsed_owner.realm
                    ),
                ));
            }
        };

        Ok(Some(DelegatedInvokeRoute {
            query_name: selector.query_name,
            owner_ura: selector.owner_ura,
            realm: parsed_owner.realm.clone(),
            hub_ura: crate::core::ura::hub_ura(&parsed_owner.realm),
            endpoints: vec![endpoint],
            release_profile: ResolverReleaseProfile::AuthoritativeLocal,
        }))
    }

    pub(crate) fn resolve_canonical_route(
        &self,
        target_ura: &str,
        ability_ura: &str,
        call_mode: CallMode,
    ) -> Result<CanonicalRouteSelection, ResolveRouteFailure> {
        let selector = match route_selector_from_query(ability_ura, "")? {
            Some(selector) => selector,
            None => route_selector_from_query(target_ura, ability_ura)?.ok_or_else(|| {
                ResolveRouteFailure::new(
                    ability_ura,
                    NegativeReason::Refused,
                    "Invoke requires a full canonical ability URA, target-bound descriptor ref, or an owner-local ability name with an explicit callee",
                )
            })?,
        };
        let owner_is_agent = matches!(
            selector.owner_kind,
            RouteOwnerKind::Agent | RouteOwnerKind::SystemAgent
        );
        if selector.owner_ura != target_ura && !owner_is_agent {
            return Err(ResolveRouteFailure::new(
                selector.query_name,
                NegativeReason::Refused,
                format!("ability_ura `{ability_ura}` does not belong to target `{target_ura}`",),
            ));
        }

        let canonical_query = selector.ability_ura.clone();
        match self.resolve_route(&canonical_query, "") {
            Ok(selected_route) => {
                let target_matches = selected_route.owner_ura == target_ura
                    || selected_route.execution_host_ura == target_ura;
                if !target_matches {
                    return Err(ResolveRouteFailure::new(
                        selected_route.query_name.clone(),
                        NegativeReason::Refused,
                        route_owner_mismatch_detail(
                            &selected_route.execution_host_ura,
                            &canonical_query,
                            target_ura,
                        ),
                    ));
                }
                Ok(CanonicalRouteSelection::local(call_mode, selected_route))
            }
            Err(local_failure) => {
                let Some(peer_source) = self.peer_delegation.as_ref() else {
                    return Err(local_failure);
                };
                let parsed_owner =
                    crate::core::ura::parse_ura(&selector.owner_ura).map_err(|err| {
                        ResolveRouteFailure::new(
                            selector.query_name.clone(),
                            NegativeReason::Refused,
                            format!("owner URA is invalid: {err}"),
                        )
                    })?;
                if parsed_owner.realm == peer_source.local_realm {
                    return Err(local_failure);
                }
                self.resolve_delegation(&canonical_query, "")?
                    .map(|route| CanonicalRouteSelection::peer(call_mode, route))
                    .ok_or_else(|| {
                        ResolveRouteFailure::new(
                            selector.query_name,
                            NegativeReason::Noroute,
                            "cross-realm Invoke had no peer delegation route",
                        )
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

    fn directory_answer(&self, query: &NamespaceResolveQuery) -> Value {
        let query_name = query.query_name.as_str();
        let prefix = Some(query.query_name.clone());
        let directory = federation_wrappers::handle_resolve_at(
            &ResolveRequest::with_filter(prefix, query.include_abilities),
            self.registry,
            self.advertised_agents,
            self.catalog,
            self.device_local.as_ref().map(|source| &source.authority),
            self.now_unix_ms,
        )
        .map_err(|detail| {
            negative_answer_json(
                query_name,
                NegativeReason::Refused,
                Some(&format!("ability projection unavailable: {detail}")),
            )
        });
        let directory = match directory {
            Ok(directory) => directory,
            Err(answer) => return answer,
        };
        let requested_limit = directory_page_limit(query.limit);
        let cursor_anchor = match directory_cursor_anchor(query.cursor.as_deref()) {
            Ok(anchor) => anchor,
            Err(detail) => {
                return negative_answer_json(query_name, NegativeReason::Refused, Some(&detail));
            }
        };
        let mut records = Vec::new();
        for agent in &directory.agents {
            records.push(directory_identity_record_for_agent(agent, self.now_unix_ms));
            match hosted_by_record_for_agent(agent, self.now_unix_ms) {
                Ok(Some(record)) => records.push(record),
                Ok(None) => {}
                Err(detail) => {
                    return negative_answer_json(
                        query_name,
                        NegativeReason::Refused,
                        Some(&detail),
                    );
                }
            }
            for summary in &agent.abilities {
                let ability_record = match ability_record_from_summary(summary, self.now_unix_ms) {
                    Ok(record) => record,
                    Err(detail) => {
                        return negative_answer_json(
                            query_name,
                            NegativeReason::Refused,
                            Some(&detail),
                        );
                    }
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
        if let Some(federated_directory) = self.federated_directory {
            for entry in
                crate::daemon::federation::directory::flatten_federated_view(federated_directory)
                    .into_iter()
                    .filter(|entry| federated_directory_entry_matches_query(entry, query_name))
            {
                let record = match federated_directory_identity_record(&entry, self.now_unix_ms) {
                    Ok(record) => record,
                    Err(detail) => {
                        return negative_answer_json(
                            query_name,
                            NegativeReason::Refused,
                            Some(&detail),
                        );
                    }
                };
                let Some(record_key) = directory_record_cursor_key(&record) else {
                    continue;
                };
                if records.iter().any(|existing| {
                    directory_record_cursor_key(existing).as_deref() == Some(&record_key)
                }) {
                    continue;
                }
                records.push(record);
            }
        }
        if let Err(detail) = apply_directory_cursor(&mut records, cursor_anchor.as_deref()) {
            return negative_answer_json(query_name, NegativeReason::Refused, Some(&detail));
        }
        let next_cursor = next_directory_cursor(&mut records, requested_limit);

        let mut answer = json!({
            "answer_kind": ResolveAnswerKind::NonDispatchable.as_str_name(),
            "canonical_name": (!query_name.is_empty()).then_some(query_name),
            "records": records,
            "release_profile": ResolverReleaseProfile::AuthoritativeLocal.as_str_name(),
            "authority": authority_for_query(query_name),
            "cache_policy": cache_policy_json(),
        });
        if let Some(cursor) = next_cursor {
            answer["next_cursor"] = Value::String(cursor);
        }
        answer
    }
}

fn directory_page_limit(raw: Option<u64>) -> usize {
    raw.map(|n| n as usize)
        .filter(|n| *n > 0)
        .unwrap_or(DEFAULT_DIRECTORY_LIMIT)
        .min(MAX_DIRECTORY_LIMIT)
}

fn directory_cursor_anchor(raw: Option<&str>) -> Result<Option<String>, String> {
    let Some(cursor) = raw.map(str::trim).filter(|value| !value.is_empty()) else {
        return Ok(None);
    };
    if cursor.len() > MAX_DIRECTORY_CURSOR_LEN {
        return Err("namespace.resolve Directory cursor exceeds the maximum bound".to_string());
    }
    let encoded = cursor
        .strip_prefix(DIRECTORY_CURSOR_PREFIX)
        .ok_or_else(|| {
            "namespace.resolve cursor is not a recognized Directory cursor".to_string()
        })?;
    let bytes = URL_SAFE_NO_PAD
        .decode(encoded.as_bytes())
        .map_err(|err| format!("namespace.resolve Directory cursor is not valid: {err}"))?;
    let anchor = String::from_utf8(bytes)
        .map_err(|err| format!("namespace.resolve Directory cursor is not UTF-8: {err}"))?;
    if anchor.trim().is_empty() {
        return Err("namespace.resolve Directory cursor anchor must not be empty".to_string());
    }
    Ok(Some(anchor))
}

fn directory_cursor_for(record: &Value) -> Option<String> {
    let key = directory_record_cursor_key(record)?;
    Some(format!(
        "{DIRECTORY_CURSOR_PREFIX}{}",
        URL_SAFE_NO_PAD.encode(key.as_bytes())
    ))
}

fn directory_record_cursor_key(record: &Value) -> Option<String> {
    let record_type = record.get("record_type")?.as_str()?.trim();
    let name = record.get("name")?.as_str()?.trim();
    if record_type.is_empty() || name.is_empty() {
        return None;
    }
    Some(format!("{record_type}\u{1f}{name}"))
}

fn apply_directory_cursor(
    records: &mut Vec<Value>,
    cursor_anchor: Option<&str>,
) -> Result<(), String> {
    let Some(anchor) = cursor_anchor else {
        return Ok(());
    };
    let Some(position) = records
        .iter()
        .position(|record| directory_record_cursor_key(record).as_deref() == Some(anchor))
    else {
        return Err(
            "namespace.resolve Directory cursor does not match the current query".to_string(),
        );
    };
    records.drain(..=position);
    Ok(())
}

fn next_directory_cursor(records: &mut Vec<Value>, requested_limit: usize) -> Option<String> {
    if records.len() <= requested_limit {
        return None;
    }
    let next = records
        .get(requested_limit.saturating_sub(1))
        .and_then(directory_cursor_for);
    records.truncate(requested_limit);
    next
}

#[derive(Debug, Clone, PartialEq, Eq)]
struct RouteSelector {
    query_name: String,
    owner_ura: String,
    owner_kind: RouteOwnerKind,
    ability_ura: String,
    public_name: String,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum RouteOwnerKind {
    LegacyDeviceProfileProjection,
    Authority,
    Agent,
    SystemAgent,
}

impl RouteOwnerKind {
    fn from_ability_selector(
        selector: &crate::core::ura::AbilitySelector,
        query_name: &str,
    ) -> Result<Self, ResolveRouteFailure> {
        match selector.owner_kind() {
            "device" => Ok(Self::LegacyDeviceProfileProjection),
            "authority" => Ok(Self::Authority),
            "agent" => Ok(Self::Agent),
            "system-agent" => Ok(Self::SystemAgent),
            other => Err(ResolveRouteFailure::new(
                query_name,
                NegativeReason::Refused,
                format!("ability selector owner kind `{other}` is not routable"),
            )),
        }
    }
}

fn route_selector_from_ability_selector(
    query_name: String,
    selector: crate::core::ura::AbilitySelector,
) -> Result<RouteSelector, ResolveRouteFailure> {
    let owner_kind = RouteOwnerKind::from_ability_selector(&selector, &query_name)?;
    Ok(RouteSelector {
        query_name,
        owner_ura: selector.owner_ura().to_string(),
        owner_kind,
        ability_ura: selector.ability_ura().to_string(),
        public_name: selector.public_name().to_string(),
    })
}

fn route_selector_from_query(
    query_name: &str,
    ability_name: &str,
) -> Result<Option<RouteSelector>, ResolveRouteFailure> {
    if ability_name.trim().is_empty() {
        if looks_like_descriptor_ref(query_name) {
            return Err(ResolveRouteFailure::new(
                query_name.trim(),
                NegativeReason::Refused,
                "descriptor_ref cannot stand in for an owner query; provide an explicit target owner URA plus descriptor_ref",
            ));
        }
        if is_ability_ura(query_name) {
            let selector =
                crate::core::ura::AbilitySelector::parse(query_name).map_err(|error| {
                    ResolveRouteFailure::new(
                        query_name,
                        NegativeReason::Refused,
                        format!("ability URA selector parse failed: {error}"),
                    )
                })?;
            return route_selector_from_ability_selector(query_name.to_string(), selector)
                .map(Some);
        }
    }
    let owner_ura = query_name.trim();
    let ability_name = ability_name.trim();
    if owner_ura.is_empty() || ability_name.is_empty() {
        return Ok(None);
    }
    if looks_like_descriptor_ref(ability_name) {
        return route_selector_from_descriptor_ref(owner_ura, ability_name).map(Some);
    }
    if is_ability_ura(ability_name) {
        let selector = crate::core::ura::AbilitySelector::parse(ability_name).map_err(|error| {
            ResolveRouteFailure::new(
                ability_name,
                NegativeReason::Refused,
                format!("ability URA selector parse failed: {error}"),
            )
        })?;
        if selector.owner_ura() != owner_ura {
            return Ok(None);
        }
        return route_selector_from_ability_selector(selector.ability_ura().to_string(), selector)
            .map(Some);
    }
    let public_name = crate::core::ura::descriptor_public_ability_name(owner_ura, ability_name);
    let ability_ura =
        crate::core::ura::owner_ability_ura(owner_ura, &public_name).ok_or_else(|| {
            ResolveRouteFailure::new(
                format!("{owner_ura}#{public_name}"),
                NegativeReason::Refused,
                "owner-local ability URA build failed",
            )
        })?;
    let selector = crate::core::ura::AbilitySelector::parse(&ability_ura).map_err(|error| {
        ResolveRouteFailure::new(
            format!("{owner_ura}#{public_name}"),
            NegativeReason::Refused,
            format!("owner-local ability selector parse failed: {error}"),
        )
    })?;
    route_selector_from_ability_selector(format!("{owner_ura}#{public_name}"), selector).map(Some)
}

fn route_selector_from_descriptor_ref(
    owner_ura: &str,
    descriptor_ref: &str,
) -> Result<RouteSelector, ResolveRouteFailure> {
    let selector = ability_selector_from_descriptor_ref(descriptor_ref)?;
    if selector.owner_ura() != owner_ura {
        return Err(ResolveRouteFailure::new(
            descriptor_ref.trim(),
            NegativeReason::Refused,
            format!(
                "descriptor_ref ability owner `{}` does not match target owner `{owner_ura}`",
                selector.owner_ura()
            ),
        ));
    }
    let public_name = selector.public_name().to_string();
    route_selector_from_ability_selector(format!("{owner_ura}#{public_name}"), selector)
}

fn ability_selector_from_descriptor_ref(
    descriptor_ref: &str,
) -> Result<crate::core::ura::AbilitySelector, ResolveRouteFailure> {
    let query_name = descriptor_ref.trim().to_string();
    crate::daemon::axon_bridge::descriptor_ref::ability_selector_from_descriptor_ref(descriptor_ref)
        .map_err(|error| {
            ResolveRouteFailure::new(
                query_name,
                NegativeReason::Refused,
                format!("descriptor_ref selector projection failed: {error}"),
            )
        })
}

fn selected_execution_for_owner(
    query_name: &str,
    owner_ura: &str,
    owner_host_node_id: Option<&str>,
    advertised_agents: Option<&AdvertisedAgentStore>,
) -> Result<(SelectedRouteKind, String, String, Option<String>), ResolveRouteFailure> {
    match crate::core::ura::parse_ura(owner_ura).map(|parsed| parsed.kind) {
        Ok(crate::core::ura::URAKind::Authority) => {
            let realm = crate::core::ura::parse_ura(owner_ura)
                .ok()
                .map(|parsed| parsed.realm)
                .unwrap_or_default();
            let hub_ura = crate::core::ura::hub_ura(&realm);
            Ok((
                SelectedRouteKind::RealmAuthorityOwned,
                hub_ura.clone(),
                hub_ura,
                None,
            ))
        }
        Ok(crate::core::ura::URAKind::Agent) => {
            let parsed = crate::core::ura::parse_ura(owner_ura).map_err(|err| {
                ResolveRouteFailure::new(
                    query_name,
                    NegativeReason::Refused,
                    format!("route owner Agent URA is invalid: {err}"),
                )
            })?;
            if let Some((device_id, system_agent_id)) = parsed.device_agent_ids() {
                if !crate::daemon::ability::catalog::profiles::is_declared_daemon_native_system_agent_id(
                    system_agent_id,
                ) {
                    return Err(ResolveRouteFailure::new(
                        query_name,
                        NegativeReason::Refused,
                        "device-scoped Agent owner is not a declared daemon-native SystemAgent",
                    ));
                }
                if let Some(host_node_id) =
                    owner_host_node_id.filter(|node| !node.trim().is_empty())
                {
                    if host_node_id != device_id {
                        return Err(ResolveRouteFailure::new(
                            query_name,
                            NegativeReason::Refused,
                            "device-sponsored SystemAgent host_node_id does not match owner device",
                        ));
                    }
                }
                return Ok((
                    SelectedRouteKind::RoutableAgentOwned,
                    owner_ura.to_string(),
                    crate::core::ura::device_ura(&parsed.realm, device_id),
                    Some(device_id.to_string()),
                ));
            }

            let Some(host_device_ura) = advertised_agent_host_ura(advertised_agents, owner_ura)
            else {
                return Err(ResolveRouteFailure::new(
                    query_name,
                    NegativeReason::Noroute,
                    "hosted agent has no resolver-selected host device",
                ));
            };
            Ok((
                SelectedRouteKind::RoutableAgentOwned,
                owner_ura.to_string(),
                host_device_ura,
                owner_host_node_id
                    .filter(|node| !node.trim().is_empty())
                    .map(ToOwned::to_owned),
            ))
        }
        Ok(crate::core::ura::URAKind::Device) => Err(ResolveRouteFailure::new(
            query_name,
            NegativeReason::Refused,
            "direct Device-owned catalog projections are not dispatchable public routes; migrate the descriptor owner to a device-sponsored SystemAgent",
        )),
        _ => Err(ResolveRouteFailure::new(
            query_name,
            NegativeReason::Refused,
            "route owner must be a canonical hub, device, or agent URA",
        )),
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
            .ok_or_else(|| {
                ResolveRouteFailure::new(
                    query_name,
                    NegativeReason::Noroute,
                    "ability projection is missing canonical ability_ura",
                )
            })?
            .to_string();
        let route_ura = summary
            .get("route_summary_ref")
            .and_then(Value::as_str)
            .filter(|value| !value.is_empty())
            .ok_or_else(|| {
                ResolveRouteFailure::new(
                    query_name,
                    NegativeReason::Noroute,
                    "ability projection is missing executable route_summary_ref",
                )
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
            &self.execution_host_ura,
            self.host_node_id.as_deref(),
            now_unix_ms,
        )
    }
}

fn is_ability_ura(value: &str) -> bool {
    crate::core::ura::parse_ura(value)
        .map(|parsed| parsed.kind == crate::core::ura::URAKind::Ability)
        .unwrap_or(false)
}

fn looks_like_descriptor_ref(value: &str) -> bool {
    let value = value.trim();
    value.contains('@') || value.contains('#') || value.contains('!')
}

fn required_string(object: &serde_json::Map<String, Value>, field: &str) -> Result<String, String> {
    optional_string(object, field)?
        .filter(|value| !value.is_empty())
        .ok_or_else(|| format!("namespace.resolve request missing canonical {field}"))
}

fn required_qtype_string(object: &serde_json::Map<String, Value>) -> Result<String, String> {
    match object.get("qtype") {
        Some(Value::String(value)) => {
            let value = value.trim();
            if value.is_empty() {
                Err(
                    "namespace.resolve qtype must be a non-empty canonical ResolveType enum string"
                        .to_string(),
                )
            } else {
                Ok(value.to_string())
            }
        }
        Some(_) => {
            Err("namespace.resolve qtype must be a canonical ResolveType enum string".to_string())
        }
        None => Err("namespace.resolve request missing canonical qtype".to_string()),
    }
}

fn optional_string(
    object: &serde_json::Map<String, Value>,
    field: &str,
) -> Result<Option<String>, String> {
    match object.get(field) {
        Some(Value::String(value)) => Ok(Some(value.trim().to_string())),
        Some(_) => Err(format!("namespace.resolve {field} must be a string")),
        None => Ok(None),
    }
}

fn optional_bool(
    object: &serde_json::Map<String, Value>,
    field: &str,
) -> Result<Option<bool>, String> {
    match object.get(field) {
        Some(Value::Bool(value)) => Ok(Some(*value)),
        Some(_) => Err(format!("namespace.resolve {field} must be a boolean")),
        None => Ok(None),
    }
}

fn optional_u64(
    object: &serde_json::Map<String, Value>,
    field: &str,
) -> Result<Option<u64>, String> {
    match object.get(field) {
        Some(Value::Number(value)) => value
            .as_u64()
            .map(Some)
            .ok_or_else(|| format!("namespace.resolve {field} must be an unsigned integer")),
        Some(_) => Err(format!(
            "namespace.resolve {field} must be an unsigned integer"
        )),
        None => Ok(None),
    }
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
    crate::core::ura::parse_ura(device_ura)
        .ok()
        .filter(|parsed| parsed.kind == crate::core::ura::URAKind::Device)
        .and_then(|parsed| parsed.device_id().map(str::to_string))
}

fn id_record(name: &str, now_unix_ms: i64) -> Value {
    json!({
        "name": name,
        "kind": directory_identity_kind(name),
        "record_type": RecordType::Id.as_str_name(),
        "ura": name,
        "authority": authority_for_query(name),
        "ttl_ms": 0,
        "expires_unix_ms": 0,
        "revision": now_unix_ms.max(0) as u64,
        "value": {
            "id": {
                "ura": name,
                "kind": ura_kind_name(name),
            }
        }
    })
}

fn directory_identity_record_for_agent(agent: &ResolveAgentSummary, now_unix_ms: i64) -> Value {
    let mut record = id_record(&agent.ura, now_unix_ms);
    if let Some(object) = record.as_object_mut() {
        if crate::core::ura::parse_ura(&agent.ura)
            .ok()
            .is_some_and(|parsed| parsed.kind == crate::core::ura::URAKind::Device)
        {
            object.insert("kind".to_string(), Value::String("device".to_string()));
            object.insert("agent_ura".to_string(), Value::String(agent.ura.clone()));
            if let Some(node_id) = device_id_from_device_ura(&agent.ura) {
                object.insert("node_id".to_string(), Value::String(node_id));
            }
            object.insert("status".to_string(), Value::String(agent.status.clone()));
        }
    }
    record
}

fn federated_directory_entry_matches_query(entry: &DirectoryEntry, query_name: &str) -> bool {
    let query_name = query_name.trim();
    !query_name.is_empty()
        && (entry.agent_ura == query_name || entry.agent_ura.starts_with(query_name))
}

fn federated_directory_identity_record(
    entry: &DirectoryEntry,
    now_unix_ms: i64,
) -> Result<Value, String> {
    let parsed = crate::core::ura::parse_ura(&entry.agent_ura)
        .map_err(|err| format!("federated directory entry URA is invalid: {err}"))?;
    if parsed.kind != crate::core::ura::URAKind::Device {
        return Err(format!(
            "federated directory entry must identify a Device URA, got {}",
            parsed.kind
        ));
    }
    let node_id = parsed
        .device_id()
        .map(str::trim)
        .filter(|node_id| !node_id.is_empty())
        .ok_or_else(|| "federated directory Device URA is missing node id".to_string())?;
    if entry.node_id.trim() != node_id {
        return Err(format!(
            "federated directory node_id `{}` does not match Device URA `{}`",
            entry.node_id, entry.agent_ura
        ));
    }

    let mut record = id_record(&entry.agent_ura, now_unix_ms);
    if let Some(object) = record.as_object_mut() {
        object.insert("kind".to_string(), Value::String("device".to_string()));
        object.insert(
            "agent_ura".to_string(),
            Value::String(entry.agent_ura.clone()),
        );
        object.insert("node_id".to_string(), Value::String(node_id.to_string()));
        object.insert("status".to_string(), Value::String(entry.status.clone()));
        if let Some(origin_realm) = entry.origin_realm.as_deref() {
            object.insert(
                "origin_realm".to_string(),
                Value::String(origin_realm.to_string()),
            );
        }
        if let Some(hub_endpoint) = entry.hub_endpoint.as_deref() {
            object.insert(
                "hub_endpoint".to_string(),
                Value::String(hub_endpoint.to_string()),
            );
        }
        if let Some(last_seen_unix_ms) = entry.last_seen_unix_ms {
            object.insert(
                "last_seen_unix_ms".to_string(),
                Value::Number(serde_json::Number::from(last_seen_unix_ms)),
            );
        }
    }
    Ok(record)
}

fn directory_identity_kind(ura: &str) -> &'static str {
    match crate::core::ura::parse_ura(ura).map(|parsed| parsed.kind) {
        Ok(crate::core::ura::URAKind::Device) => "device",
        Ok(crate::core::ura::URAKind::Agent) => "agent",
        Ok(crate::core::ura::URAKind::User) => "user",
        Ok(crate::core::ura::URAKind::Authority) => "authority",
        _ => "identity",
    }
}

fn hosted_by_record_for_agent(
    agent: &ResolveAgentSummary,
    now_unix_ms: i64,
) -> Result<Option<Value>, String> {
    let Some(host_node_id) = agent.host_node_id.as_deref().map(str::trim) else {
        return Ok(None);
    };
    if host_node_id.is_empty() {
        return Ok(None);
    }
    let parsed = crate::core::ura::parse_ura(&agent.ura)
        .map_err(|err| format!("directory agent URA is invalid: {err}"))?;
    if parsed.kind != crate::core::ura::URAKind::Agent {
        return Err(format!(
            "directory hosted_by record requires Agent owner, got {}",
            parsed.kind
        ));
    }
    let host_ura = crate::core::ura::device_ura(&parsed.realm, host_node_id);
    Ok(Some(hosted_by_record(
        &agent.ura,
        &host_ura,
        host_node_id,
        now_unix_ms,
    )))
}

fn hosted_by_record(
    hosted_ura: &str,
    host_ura: &str,
    host_node_id: &str,
    now_unix_ms: i64,
) -> Value {
    json!({
        "name": hosted_ura,
        "kind": "hosted_by",
        "record_type": RecordType::HostedBy.as_str_name(),
        "ura": hosted_ura,
        "owner_ura": host_ura,
        "authority": authority_for_query(hosted_ura),
        "ttl_ms": 0,
        "expires_unix_ms": 0,
        "revision": now_unix_ms.max(0) as u64,
        "value": {
            "hosted_by": {
                "hosted_ura": hosted_ura,
                "host_ura": host_ura,
                "host_node_id": host_node_id,
                "lease_expires_unix_ms": 0,
            }
        }
    })
}

fn ability_record_from_summary(summary: &Value, now_unix_ms: i64) -> Result<Value, String> {
    let ability_ura = summary
        .get("ability_ura")
        .and_then(Value::as_str)
        .filter(|value| !value.is_empty())
        .ok_or_else(|| "ability projection is missing canonical ability_ura".to_string())?;
    let owner_ura = summary
        .get("owner_ura")
        .and_then(Value::as_str)
        .filter(|value| !value.is_empty())
        .ok_or_else(|| "ability projection is missing canonical owner_ura".to_string())?;
    let namespace = summary
        .get("namespace")
        .and_then(Value::as_str)
        .ok_or_else(|| "ability projection is missing namespace".to_string())?;
    let local_name = summary
        .get("local_name")
        .and_then(Value::as_str)
        .filter(|value| !value.is_empty())
        .ok_or_else(|| "ability projection is missing local_name".to_string())?;
    let descriptor_revision = summary
        .get("descriptor_revision")
        .and_then(Value::as_str)
        .filter(|value| !value.is_empty())
        .ok_or_else(|| "ability projection is missing descriptor_revision".to_string())?;
    let policy_ref = summary
        .get("policy_ref")
        .and_then(Value::as_str)
        .filter(|value| !value.is_empty())
        .ok_or_else(|| "ability projection is missing policy_ref".to_string())?;
    let route_summary_ref = match summary.get("route_summary_ref") {
        Some(Value::Null) | None => Value::Null,
        Some(Value::String(value)) if !value.is_empty() => Value::String(value.clone()),
        Some(Value::String(_)) => {
            return Err("ability projection route_summary_ref must not be empty".to_string());
        }
        Some(_) => return Err("ability projection route_summary_ref must be a string".to_string()),
    };
    let tags = match summary.get("tags") {
        Some(Value::Array(_)) => summary.get("tags").cloned().expect("tags present"),
        Some(_) => return Err("ability projection tags must be an array".to_string()),
        None => return Err("ability projection is missing tags".to_string()),
    };
    Ok(json!({
        "name": ability_ura,
        "kind": "ability",
        "record_type": RecordType::Ability.as_str_name(),
        "ability_ura": ability_ura,
        "owner_ura": owner_ura,
        "authority": authority_for_query(ability_ura),
        "ttl_ms": 0,
        "expires_unix_ms": 0,
        "revision": now_unix_ms.max(0) as u64,
        "value": {
            "ability": {
                "ability_ura": ability_ura,
                "owner_ura": owner_ura,
                "namespace": namespace,
                "local_name": local_name,
                "summary": {
                    "ability_ura": ability_ura,
                    "owner_ura": owner_ura,
                    "namespace": namespace,
                    "local_name": local_name,
                    "descriptor_revision": descriptor_revision,
                    "schema_ref": summary.get("schema_ref").cloned().unwrap_or(Value::Null),
                    "schema_hash": summary.get("schema_hash").cloned().unwrap_or(Value::Null),
                    "policy_ref": policy_ref,
                    "route_summary_ref": route_summary_ref,
                    "tags": tags,
                }
            }
        }
    }))
}

/// Build an `ABILITY` record proven by the local runtime table (D105). Unlike
/// [`ability_record_from_summary`], the inputs are the resolver-canonical
/// SystemAgent/Agent/Authority owner and owner-local public name, not a hub
/// projection summary. Device remains only the execution host carried by the
/// paired route record.
fn local_runtime_ability_record(
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
        "kind": "ability",
        "record_type": RecordType::Ability.as_str_name(),
        "ability_ura": ability_ura,
        "owner_ura": owner_ura,
        "authority": authority_for_query(ability_ura),
        "ttl_ms": 0,
        "expires_unix_ms": 0,
        "revision": now_unix_ms.max(0) as u64,
        "value": {
            "ability": {
                "ability_ura": ability_ura,
                "owner_ura": owner_ura,
                "namespace": namespace,
                "local_name": local_name,
                "summary": {
                    "ability_ura": ability_ura,
                    "owner_ura": owner_ura,
                    "namespace": namespace,
                    "local_name": local_name,
                    "policy_ref": "visibility:PUBLIC",
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
    execution_target_ura: &str,
    host_node_id: Option<&str>,
    now_unix_ms: i64,
) -> Value {
    json!({
        "name": route_ura,
        "kind": "route",
        "record_type": RecordType::Route.as_str_name(),
        "route_ura": route_ura,
        "ability_ura": ability_ura,
        "owner_ura": owner_ura,
        "authority": authority_for_query(route_ura),
        "ttl_ms": 0,
        "expires_unix_ms": 0,
        "revision": now_unix_ms.max(0) as u64,
        "value": {
            "route": {
                "route_ura": route_ura,
                "ability_ura": ability_ura,
                "dispatch_name": dispatch_name,
                "execute_on": {
                    "kind": ura_kind_name(execution_target_ura),
                    "target_ura": execution_target_ura,
                    "host_node_id": host_node_id.unwrap_or_default(),
                }
            }
        }
    })
}

fn negative_answer_json(query_name: &str, reason: NegativeReason, detail: Option<&str>) -> Value {
    json!({
        "answer_kind": ResolveAnswerKind::Negative.as_str_name(),
        "next_hop": {
            "no_route": {}
        },
        "records": [],
        "release_profile": ResolverReleaseProfile::AuthoritativeLocal.as_str_name(),
        "authority": authority_for_query(query_name),
        "cache_policy": cache_policy_json(),
        "negative": {
            "reason": reason.as_str_name(),
            "query_name": query_name,
            "detail": detail,
        }
    })
}

pub(crate) fn authority_for_query(query_name: &str) -> Value {
    match authority_realm_for_query(query_name) {
        Some(realm) => json!({
            "authority_ura": crate::core::ura::hub_ura(&realm),
            "zone_ref": format!("realm:{realm}"),
            "algorithm": "daemon-local",
            "signature": "",
            "issued_unix_ms": 0,
        }),
        None => json!({
            "authority_ura": "",
            "zone_ref": "query_name_unavailable",
            "algorithm": "daemon-local-unavailable",
            "signature": "",
            "issued_unix_ms": 0,
            "unavailable": {
                "reason": "query_name is not a canonical URA, route-ref, or descriptor ref"
            }
        }),
    }
}

fn authority_realm_for_query(query_name: &str) -> Option<String> {
    let query_name = query_name.trim();
    let candidate = query_name
        .strip_prefix("route-ref::")
        .map(str::trim)
        .filter(|value| !value.is_empty())
        .unwrap_or(query_name);
    if let Some(realm) = realm_from_ura(candidate) {
        return Some(realm);
    }
    let ability_ura =
        crate::daemon::axon_bridge::descriptor_ref::ability_ura_from_descriptor_ref(candidate)
            .ok()?;
    realm_from_ura(&ability_ura)
}

fn realm_from_ura(value: &str) -> Option<String> {
    crate::core::ura::parse_ura(value)
        .ok()
        .map(|parsed| parsed.realm)
        .filter(|realm| !realm.is_empty())
}

fn cache_policy_json() -> Value {
    json!({
        "ttl_ms": 0,
        "shared_cacheable": false,
        "retry_after_unix_ms": 0,
    })
}

fn ura_kind_name(ura: &str) -> &'static str {
    match crate::core::ura::parse_ura(ura).map(|parsed| parsed.kind) {
        Ok(crate::core::ura::URAKind::Authority) => UraKind::Hub.as_str_name(),
        Ok(crate::core::ura::URAKind::Device) => UraKind::Device.as_str_name(),
        Ok(crate::core::ura::URAKind::User) => UraKind::User.as_str_name(),
        Ok(crate::core::ura::URAKind::Agent) => UraKind::Agent.as_str_name(),
        Ok(crate::core::ura::URAKind::Ability) => UraKind::Ability.as_str_name(),
        Ok(crate::core::ura::URAKind::Resource) => UraKind::Resource.as_str_name(),
        _ => UraKind::Unspecified.as_str_name(),
    }
}

fn summary_public_name(summary: &Value) -> Option<String> {
    let namespace = summary.get("namespace").and_then(Value::as_str)?;
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
    use crate::daemon::persistence::agent_aggregate::AgentHostedPlacement;

    use std::collections::BTreeMap;
    use std::sync::Arc;

    use crate::daemon::federation::directory::{
        DirectoryEntry, DirectoryView, SharedFederatedDirectoryView,
    };
    use crate::daemon::federation::peers::SharedFederatedPeers;
    use crate::daemon::federation::read_model::ability_catalog::{
        AbilityCatalogStore, OwnerAbilityProjectionRow,
    };
    use crate::daemon::federation::read_model::advertised_agents::{
        AdvertisedAgentRecord, AdvertisedAgentSigningAuthority, AdvertisedAgentStore,
    };
    use crate::daemon::invocation::bidi::state::presence::PresenceRegistry;

    const TEST_NOW_MS: i64 = 1_700_000_000_000;
    const LEASE_EXPIRES_MS: i64 = 4_102_444_800_000;

    fn device_owner_ura() -> String {
        crate::core::ura::device_ura("test-realm", "test-daemon")
    }

    fn system_agent_owner_ura_for(public_ability: &str) -> String {
        let owner = crate::daemon::ability::catalog::ownership::device_sponsored_system_agent_owner_for_public_ability(
            public_ability,
        )
        .unwrap_or_else(|| panic!("{public_ability} must have one declared SystemAgent owner"));
        crate::core::ura::device_agent_ura("test-realm", "test-daemon", owner.system_agent_id())
    }

    /// Build a `DispatchSender` whose receiver is dropped immediately.
    /// Presence only needs a live entry (the URA in `snapshot()`); the
    /// resolver never sends on the channel.
    fn make_dispatch_sender() -> crate::daemon::invocation::bidi::state::presence::DispatchSender {
        let (tx, _rx) = tokio::sync::mpsc::channel(1);
        tx
    }

    /// Mark `owner_ura` as online in presence (the same liveness signal
    /// `handle_resolve_at` reads from `registry.snapshot()`).
    fn mark_online(registry: &PresenceRegistry, owner_ura: &str) {
        registry
            .insert_negotiated(
                owner_ura.to_string(),
                make_dispatch_sender(),
                crate::daemon::invocation::bidi::state::presence::SessionContract::new(
                    crate::daemon::invocation::bidi::state::presence::CANONICAL_SESSION_CARRIER_VERSION,
                    vec![0; 16],
                ),
            )
            .expect("canonical presence key");
    }

    fn advertise_hosted_agent(store: &AdvertisedAgentStore, agent_ura: &str, host_ura: &str) {
        store.upsert(AdvertisedAgentRecord {
            agent_ura: agent_ura.to_string(),
            generation: 1,
            public_key_hex: "00".to_string(),
            host_node_id: device_id_from_device_ura(host_ura),
            signing_authority: AdvertisedAgentSigningAuthority::HostedBy {
                host_ura: host_ura.to_string(),
            },
        });
    }

    /// Test builder for the same immutable local publication snapshot used by
    /// production route and directory resolution.
    struct FakeLocalRuntimeAuthority;

    impl FakeLocalRuntimeAuthority {
        fn with_owner_keys(owner_ura: &str, keys: &[&str]) -> LocalAbilityPublicationSnapshot {
            LocalAbilityPublicationSnapshot::from_owner_public_names(owner_ura, keys)
        }
    }

    fn descriptor_ref_for_test(ability_ura: &str) -> String {
        axon_sdk::invocation::canonical_ability_descriptor_ref(&format!(
            "{ability_ura}@1.0.0#{}!invoke",
            "a".repeat(64)
        ))
        .expect("test descriptor ref")
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
        let ability_ura = crate::core::ura::owner_ability_ura(owner_ura, &public_name)
            .expect("owner ability ura");
        catalog.upsert_projection(OwnerAbilityProjectionRow::new(
            owner_ura.to_string(),
            host_device_ura.to_string(),
            1,
            1,
            "sha256:test".to_string(),
            LEASE_EXPIRES_MS,
            vec![crate::daemon::federation::read_model::owner_projection::AbilityProjectionSummary {
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
                callable_summary: crate::daemon::federation::read_model::owner_projection::AbilityCallableSummary::minimal(
                    public_name,
                ),
            }],
        ));
        ability_ura
    }

    fn publish_ability_with_descriptor_revision(
        catalog: &AbilityCatalogStore,
        owner_ura: &str,
        host_device_ura: &str,
        descriptor_revision: &str,
    ) -> String {
        let namespace = "agent";
        let local_name = "list";
        let public_name = format!("{namespace}.{local_name}");
        let ability_ura = crate::core::ura::owner_ability_ura(owner_ura, &public_name)
            .expect("owner ability ura");
        catalog.upsert_projection(OwnerAbilityProjectionRow::new(
            owner_ura.to_string(),
            host_device_ura.to_string(),
            1,
            1,
            "sha256:test".to_string(),
            LEASE_EXPIRES_MS,
            vec![crate::daemon::federation::read_model::owner_projection::AbilityProjectionSummary {
                ability_ura: ability_ura.clone(),
                owner_ura: owner_ura.to_string(),
                namespace: namespace.to_string(),
                local_name: local_name.to_string(),
                descriptor_revision: descriptor_revision.to_string(),
                schema_ref: None,
                schema_hash: None,
                policy_ref: "visibility:PUBLIC".to_string(),
                route_summary_ref: Some(format!("route-ref::{ability_ura}")),
                tags: vec!["class:unary".to_string()],
                callable_summary: crate::daemon::federation::read_model::owner_projection::AbilityCallableSummary::minimal(
                    public_name,
                ),
            }],
        ));
        ability_ura
    }

    #[test]
    fn hub_final_route_answer_rehydrates_selected_route_facts() {
        let registry = PresenceRegistry::default();
        let advertised_agents = AdvertisedAgentStore::default();
        let catalog = AbilityCatalogStore::default();
        let host_device_ura = device_owner_ura();
        let owner_ura = crate::core::ura::device_agent_ura(
            "test-realm",
            "test-daemon",
            crate::daemon::ability::names::federation::ABILITY_MANAGEMENT_SYSTEM_AGENT_ID,
        );
        mark_online(&registry, &host_device_ura);
        advertise_hosted_agent(&advertised_agents, &owner_ura, &host_device_ura);
        let ability_ura =
            publish_ability(&catalog, &owner_ura, &host_device_ura, "ability", "deploy");
        let resolver = DaemonRouteResolver::new(&registry, Some(&advertised_agents), &catalog);
        let route = resolver
            .resolve_route(&ability_ura, "")
            .expect("fixture route resolves");
        let answer = route.final_route_answer_json();

        let rehydrated = SelectedInvokeRoute::from_hub_final_route_answer_json(
            &answer,
            &host_device_ura,
            &ability_ura,
        )
        .expect("hub final route rehydrates");

        assert_eq!(rehydrated.ability_ura, ability_ura);
        assert_eq!(rehydrated.owner_ura, owner_ura);
        assert_eq!(rehydrated.callee_ura, owner_ura);
        assert_eq!(rehydrated.execution_host_ura, host_device_ura);
        assert_eq!(rehydrated.dispatch_name, "ability.deploy");
        assert_eq!(rehydrated.kind(), SelectedRouteKind::RoutableAgentOwned);
    }

    #[test]
    fn hub_final_route_device_hosted_arm_rehydrates_system_agent_callee() {
        let registry = PresenceRegistry::new();
        let catalog = AbilityCatalogStore::new();
        let host_device_ura = device_owner_ura();
        mark_online(&registry, &host_device_ura);
        let system_agent_ura = crate::core::ura::device_agent_ura(
            "test-realm",
            "test-daemon",
            crate::daemon::ability::names::federation::ABILITY_MANAGEMENT_SYSTEM_AGENT_ID,
        );
        let ability = crate::daemon::ability::names::federation::ABILITY_DEPLOY;
        let authority = FakeLocalRuntimeAuthority::with_owner_keys(&system_agent_ura, &[ability]);
        let route = DaemonRouteResolver::new(&registry, None, &catalog)
            .with_local_catalog_authority(host_device_ura.clone(), authority)
            .at(TEST_NOW_MS)
            .resolve_route(&system_agent_ura, ability)
            .expect("ability-management SystemAgent route resolves");
        let mut answer = route.final_route_answer_json();
        let device_hosted_next_hop = json!({
            "device_hosted_ability": {
                "host_device_ura": host_device_ura,
                "callee_ura": system_agent_ura,
                "ability_ura": route.ability_ura,
                "route_ura": route.route_ura,
                "dispatch_name": route.dispatch_name,
            }
        });
        answer["next_hop"] = device_hosted_next_hop.clone();
        answer["selected_route"]["next_hop"] = device_hosted_next_hop;

        let rehydrated = SelectedInvokeRoute::from_hub_final_route_answer_json(
            &answer,
            &host_device_ura,
            &route.ability_ura,
        )
        .expect("Hub SystemAgent final route rehydrates from Device-hosted execution arm");

        assert_eq!(rehydrated.kind(), SelectedRouteKind::RoutableAgentOwned);
        assert_eq!(rehydrated.owner_ura, system_agent_ura);
        assert_eq!(rehydrated.callee_ura, system_agent_ura);
        assert_eq!(rehydrated.execution_host_ura, host_device_ura);
        assert_eq!(rehydrated.ability_ura, route.ability_ura);
        assert_eq!(rehydrated.dispatch_name, ability);
    }

    #[test]
    fn hub_final_route_device_hosted_arm_rejects_callee_ability_mismatch() {
        let registry = PresenceRegistry::new();
        let catalog = AbilityCatalogStore::new();
        let host_device_ura = device_owner_ura();
        mark_online(&registry, &host_device_ura);
        let system_agent_ura = crate::core::ura::device_agent_ura(
            "test-realm",
            "test-daemon",
            crate::daemon::ability::names::federation::ABILITY_MANAGEMENT_SYSTEM_AGENT_ID,
        );
        let ability = crate::daemon::ability::names::federation::ABILITY_DEPLOY;
        let authority = FakeLocalRuntimeAuthority::with_owner_keys(&system_agent_ura, &[ability]);
        let route = DaemonRouteResolver::new(&registry, None, &catalog)
            .with_local_catalog_authority(host_device_ura.clone(), authority)
            .at(TEST_NOW_MS)
            .resolve_route(&system_agent_ura, ability)
            .expect("ability-management SystemAgent route resolves");
        let mut answer = route.final_route_answer_json();
        let mismatched_callee = crate::core::ura::device_agent_ura(
            "test-realm",
            "test-daemon",
            "runtime-introspection",
        );
        let device_hosted_next_hop = json!({
            "device_hosted_ability": {
                "host_device_ura": host_device_ura,
                "callee_ura": mismatched_callee,
                "ability_ura": route.ability_ura,
                "route_ura": route.route_ura,
                "dispatch_name": route.dispatch_name,
            }
        });
        answer["next_hop"] = device_hosted_next_hop.clone();
        answer["selected_route"]["next_hop"] = device_hosted_next_hop;

        let error = SelectedInvokeRoute::from_hub_final_route_answer_json(
            &answer,
            &host_device_ura,
            &route.ability_ura,
        )
        .expect_err("mismatched callee must fail closed");

        assert!(error.detail.contains("does not own ability"));
    }

    #[test]
    fn hub_final_route_rejects_obsolete_local_device_ability_arm() {
        let registry = PresenceRegistry::new();
        let catalog = AbilityCatalogStore::new();
        let host_device_ura = device_owner_ura();
        mark_online(&registry, &host_device_ura);
        let system_agent_ura = crate::core::ura::device_agent_ura(
            "test-realm",
            "test-daemon",
            crate::daemon::ability::names::federation::ABILITY_MANAGEMENT_SYSTEM_AGENT_ID,
        );
        let ability = crate::daemon::ability::names::federation::ABILITY_DEPLOY;
        let authority = FakeLocalRuntimeAuthority::with_owner_keys(&system_agent_ura, &[ability]);
        let route = DaemonRouteResolver::new(&registry, None, &catalog)
            .with_local_catalog_authority(host_device_ura.clone(), authority)
            .at(TEST_NOW_MS)
            .resolve_route(&system_agent_ura, ability)
            .expect("ability-management SystemAgent route resolves");
        let mut answer = route.final_route_answer_json();
        let obsolete_next_hop = json!({
            "local_device_ability": {
                "device_ura": host_device_ura,
                "ability_ura": route.ability_ura,
                "route_ura": route.route_ura,
                "dispatch_name": route.dispatch_name,
            }
        });
        answer["next_hop"] = obsolete_next_hop.clone();
        answer["selected_route"]["next_hop"] = obsolete_next_hop;

        let error = SelectedInvokeRoute::from_hub_final_route_answer_json(
            &answer,
            &host_device_ura,
            &route.ability_ura,
        )
        .expect_err("obsolete direct-Device-shaped next_hop must fail closed");

        assert!(error
            .detail
            .contains("did not select a daemon-dispatchable route"));
    }

    #[test]
    fn hub_route_provider_rejects_non_final_answer() {
        let owner_ura = device_owner_ura();
        let ability_ura = crate::core::ura::owner_ability_ura(&owner_ura, "meta.list_resources")
            .expect("test ability ura");
        let answer = ResolveRouteFailure::owner_offline(&ability_ura, NegativeReason::Nxdomain)
            .answer_json();

        let error = SelectedInvokeRoute::from_hub_final_route_answer_json(
            &answer,
            &owner_ura,
            &ability_ura,
        )
        .expect_err("negative hub answer must not rehydrate a route");

        assert_eq!(error.reason, NegativeReason::Noroute);
        assert!(error.detail.contains("non-final answer_kind"));
    }

    #[test]
    fn ability_record_projection_rejects_missing_descriptor_and_tags_before_empty_defaults() {
        let owner_ura = crate::core::ura::device_agent_ura(
            "test-realm",
            "test-daemon",
            crate::daemon::ability::names::agents::AGENT_MANAGEMENT_SYSTEM_AGENT_ID,
        );
        let ability_ura = crate::core::ura::owner_ability_ura(&owner_ura, "agent.list")
            .expect("system agent ability");
        let summary = json!({
            "ability_ura": ability_ura.as_str(),
            "owner_ura": owner_ura.as_str(),
            "namespace": "agent",
            "local_name": "list",
            "policy_ref": "visibility:PUBLIC",
            "route_summary_ref": format!("route-ref::{ability_ura}")
        });

        let error = ability_record_from_summary(&summary, TEST_NOW_MS)
            .expect_err("missing descriptor_revision must fail closed");
        assert!(
            error.contains("descriptor_revision"),
            "wrong error: {error}"
        );

        let summary = json!({
            "ability_ura": ability_ura.as_str(),
            "owner_ura": owner_ura.as_str(),
            "namespace": "agent",
            "local_name": "list",
            "descriptor_revision": "sha256:descriptor",
            "policy_ref": "visibility:PUBLIC",
            "route_summary_ref": format!("route-ref::{ability_ura}")
        });

        let error = ability_record_from_summary(&summary, TEST_NOW_MS)
            .expect_err("missing tags must fail closed");
        assert!(error.contains("tags"), "wrong error: {error}");
    }

    #[test]
    fn system_agent_ability_online_resolves_final_local_execution_route() {
        let registry = PresenceRegistry::new();
        let catalog = AbilityCatalogStore::new();
        let host_device_ura = device_owner_ura();
        let owner_ura = system_agent_owner_ura_for("agent.list");
        mark_online(&registry, &host_device_ura);
        let ability_ura =
            crate::core::ura::owner_ability_ura(&owner_ura, "agent.list").expect("ability ura");
        let authority = FakeLocalRuntimeAuthority::with_owner_keys(&owner_ura, &["agent.list"]);

        let route = DaemonRouteResolver::new(&registry, None, &catalog)
            .with_local_catalog_authority(host_device_ura.clone(), authority)
            .at(TEST_NOW_MS)
            .resolve_route(&owner_ura, "agent.list")
            .expect("SystemAgent ability online must resolve a final route");

        assert_eq!(route.kind(), SelectedRouteKind::RoutableAgentOwned);
        assert_eq!(route.route_reason(), RouteReason::HostedAgent);
        assert_eq!(
            route.dispatch_target(true),
            SelectedRouteDispatchTarget::LocalRuntime,
            "a route proven from local runtime authority must remain local even if a caller \
             supplies a stale self-host hint"
        );
        assert_eq!(route.owner_ura, owner_ura);
        assert_eq!(route.execution_host_ura, host_device_ura);
        assert_eq!(route.callee_ura, owner_ura);
        assert_eq!(route.ability_ura, ability_ura);
        assert_eq!(route.route_ura, format!("route-ref::{ability_ura}"));
        assert_eq!(route.dispatch_name, "agent.list");
        assert!(route.is_authoritative_local_or_better());

        // next_hop names Device as execution host and keeps SystemAgent callee explicit.
        let next_hop = route.next_hop_json();
        let local = &next_hop["hosted_agent_via_device"];
        assert_eq!(local["host_device_ura"], host_device_ura);
        assert_eq!(local["agent_ura"], owner_ura);
        assert_eq!(local["ability_ura"], ability_ura);
        assert_eq!(local["route_ura"], format!("route-ref::{ability_ura}"));
        assert_eq!(local["dispatch_name"], "agent.list");
    }

    #[test]
    fn canonical_policy_selects_identical_local_route_for_every_carrier() {
        let registry = PresenceRegistry::new();
        let catalog = AbilityCatalogStore::new();
        let host_device_ura = device_owner_ura();
        let owner_ura = system_agent_owner_ura_for("agent.list");
        let ability_ura =
            crate::core::ura::owner_ability_ura(&owner_ura, "agent.list").expect("ability ura");
        let authority = FakeLocalRuntimeAuthority::with_owner_keys(&owner_ura, &["agent.list"]);
        let resolver = DaemonRouteResolver::new(&registry, None, &catalog)
            .with_local_catalog_authority(host_device_ura, authority)
            .at(TEST_NOW_MS);

        let mut fingerprints = Vec::new();
        for call_mode in [CallMode::Rpc, CallMode::Stream, CallMode::Bidi] {
            let selection = resolver
                .resolve_canonical_route(&owner_ura, &ability_ura, call_mode)
                .expect("all carriers use the same canonical local route policy");
            assert_eq!(selection.call_mode(), call_mode);
            let CanonicalRouteDispatch::Local(route) = selection.into_dispatch() else {
                panic!("same-realm local route must not delegate");
            };
            let kind = route.kind();
            fingerprints.push((
                route.owner_ura,
                route.callee_ura,
                route.execution_host_ura,
                route.ability_ura,
                route.route_ura,
                route.dispatch_name,
                kind,
            ));
        }

        assert!(fingerprints.windows(2).all(|pair| pair[0] == pair[1]));
    }

    #[test]
    fn canonical_policy_selects_identical_peer_route_for_every_carrier() {
        let registry = PresenceRegistry::new();
        let peers = SharedFederatedPeers::new(BTreeMap::from([(
            "remote-realm".to_string(),
            "https://remote-hub.example".to_string(),
        )]));
        let owner_ura = crate::core::ura::device_ura("remote-realm", "remote-device");
        let ability_ura =
            crate::core::ura::owner_ability_ura(&owner_ura, "node.describe").expect("ability ura");
        let catalog = AbilityCatalogStore::new();
        let resolver = DaemonRouteResolver::new(&registry, None, &catalog)
            .with_peer_delegation("local-realm", &peers)
            .at(TEST_NOW_MS);

        let mut fingerprints = Vec::new();
        for call_mode in [CallMode::Rpc, CallMode::Stream, CallMode::Bidi] {
            let selection = resolver
                .resolve_canonical_route(&owner_ura, &ability_ura, call_mode)
                .expect("all carriers use the same canonical cross-realm policy");
            assert_eq!(selection.call_mode(), call_mode);
            let CanonicalRouteDispatch::Peer(route) = selection.into_dispatch() else {
                panic!("cross-realm route must delegate");
            };
            let primary_endpoint = route.primary_endpoint().map(ToOwned::to_owned);
            fingerprints.push((
                route.owner_ura,
                route.realm,
                route.hub_ura,
                primary_endpoint,
            ));
        }

        assert!(fingerprints.windows(2).all(|pair| pair[0] == pair[1]));
    }

    #[test]
    fn canonical_policy_rejects_owner_target_mismatch_identically_for_every_carrier() {
        let registry = PresenceRegistry::new();
        let owner_ura = crate::core::ura::device_ura("test-realm", "owner-a");
        let other_target = crate::core::ura::device_ura("test-realm", "owner-b");
        let ability_ura =
            crate::core::ura::owner_ability_ura(&owner_ura, "agent.list").expect("ability ura");
        let catalog = AbilityCatalogStore::new();
        let resolver = DaemonRouteResolver::new(&registry, None, &catalog).at(TEST_NOW_MS);

        let failures = [CallMode::Rpc, CallMode::Stream, CallMode::Bidi]
            .into_iter()
            .map(|call_mode| {
                let failure = resolver
                    .resolve_canonical_route(&other_target, &ability_ura, call_mode)
                    .expect_err("owner/target mismatch must fail before carrier dispatch");
                (failure.reason, failure.detail)
            })
            .collect::<Vec<_>>();

        assert!(failures.windows(2).all(|pair| pair[0] == pair[1]));
        assert_eq!(failures[0].0, NegativeReason::Refused);
    }

    #[test]
    fn daemon_local_discover_resolves_as_local_runtime_front_door() {
        let registry = PresenceRegistry::new();
        let catalog = AbilityCatalogStore::new();
        let host_device_ura = device_owner_ura();
        let owner_ura = system_agent_owner_ura_for("agent.discover");
        mark_online(&registry, &host_device_ura);
        let ability_ura =
            crate::core::ura::owner_ability_ura(&owner_ura, "agent.discover").expect("ability ura");
        let authority = FakeLocalRuntimeAuthority::with_owner_keys(&owner_ura, &["agent.discover"]);

        let route = DaemonRouteResolver::new(&registry, None, &catalog)
            .with_local_catalog_authority(host_device_ura.clone(), authority)
            .at(TEST_NOW_MS)
            .resolve_route(&owner_ura, "agent.discover")
            .expect("daemon-local discover must resolve through local runtime routing");

        assert_eq!(route.kind(), SelectedRouteKind::RoutableAgentOwned);
        assert_eq!(
            route.dispatch_target(true),
            SelectedRouteDispatchTarget::LocalRuntime
        );
        assert_eq!(route.owner_ura, owner_ura);
        assert_eq!(route.execution_host_ura, host_device_ura);
        assert_eq!(route.ability_ura, ability_ura);
        assert_eq!(route.dispatch_name, "agent.discover");
    }

    #[test]
    fn local_runtime_authority_rejects_daemon_local_companion_control_routes() {
        let registry = PresenceRegistry::new();
        let catalog = AbilityCatalogStore::new();
        let host_device_ura = device_owner_ura();
        let owner_ura = crate::core::ura::device_agent_ura(
            "test-realm",
            "test-daemon",
            crate::daemon::ability::names::integrations::PLUGIN_MANAGEMENT_SYSTEM_AGENT_ID,
        );
        mark_online(&registry, &host_device_ura);

        for ability in ["plugin.companion_status", "plugin.companion_reconcile"] {
            let authority = FakeLocalRuntimeAuthority::with_owner_keys(
                &owner_ura,
                &["plugin.companion_status", "plugin.companion_reconcile"],
            );
            let err = DaemonRouteResolver::new(&registry, None, &catalog)
                .with_local_catalog_authority(host_device_ura.clone(), authority)
                .at(TEST_NOW_MS)
                .resolve_route(&owner_ura, ability)
                .expect_err("daemon-local companion control must not resolve as remote route");

            assert_eq!(err.reason, NegativeReason::Nodata);
            assert!(
                err.detail.contains("daemon-local"),
                "route failure should name local-only policy: {}",
                err.detail
            );
        }
    }

    #[test]
    fn system_agent_descriptor_ref_resolves_local_route() {
        let registry = PresenceRegistry::new();
        let catalog = AbilityCatalogStore::new();
        let host_device_ura = device_owner_ura();
        let owner_ura = system_agent_owner_ura_for("agent.list");
        mark_online(&registry, &host_device_ura);
        let ability_ura =
            crate::core::ura::owner_ability_ura(&owner_ura, "agent.list").expect("ability ura");
        let descriptor_ref = descriptor_ref_for_test(&ability_ura);
        let authority = FakeLocalRuntimeAuthority::with_owner_keys(&owner_ura, &["agent.list"]);

        let route = DaemonRouteResolver::new(&registry, None, &catalog)
            .with_local_catalog_authority(host_device_ura.clone(), authority)
            .at(TEST_NOW_MS)
            .resolve_route(&owner_ura, &descriptor_ref)
            .expect("descriptor-bound SystemAgent ability must resolve locally");

        assert_eq!(route.kind(), SelectedRouteKind::RoutableAgentOwned);
        assert_eq!(route.owner_ura, owner_ura);
        assert_eq!(route.callee_ura, owner_ura);
        assert_eq!(route.execution_host_ura, host_device_ura);
        assert_eq!(route.ability_ura, ability_ura);
        assert_eq!(route.route_ura, format!("route-ref::{ability_ura}"));
        assert_eq!(route.dispatch_name, "agent.list");
        assert_eq!(route.query_name, format!("{owner_ura}#agent.list"));
    }

    #[test]
    fn descriptor_ref_query_without_owner_is_rejected() {
        let registry = PresenceRegistry::new();
        let catalog = AbilityCatalogStore::new();
        let owner_ura = device_owner_ura();
        mark_online(&registry, &owner_ura);
        let ability_ura =
            crate::core::ura::owner_ability_ura(&owner_ura, "agent.list").expect("ability ura");
        let descriptor_ref = descriptor_ref_for_test(&ability_ura);
        let authority = FakeLocalRuntimeAuthority::with_owner_keys(&owner_ura, &["agent.list"]);

        let failure = DaemonRouteResolver::new(&registry, None, &catalog)
            .with_local_catalog_authority(owner_ura.clone(), authority)
            .at(TEST_NOW_MS)
            .resolve_route(&descriptor_ref, "")
            .expect_err("descriptor_ref must not be accepted as an owner query");

        assert_eq!(failure.reason, NegativeReason::Refused);
        assert!(
            failure
                .detail
                .contains("cannot stand in for an owner query"),
            "unexpected detail: {}",
            failure.detail
        );
    }

    #[test]
    fn owner_plus_full_ability_ura_resolves_same_final_route() {
        let registry = PresenceRegistry::new();
        let catalog = AbilityCatalogStore::new();
        let host_device_ura = device_owner_ura();
        let owner_ura = system_agent_owner_ura_for("agent.list");
        mark_online(&registry, &host_device_ura);
        let ability_ura =
            crate::core::ura::owner_ability_ura(&owner_ura, "agent.list").expect("ability ura");
        let authority = FakeLocalRuntimeAuthority::with_owner_keys(&owner_ura, &["agent.list"]);

        let route = DaemonRouteResolver::new(&registry, None, &catalog)
            .with_local_catalog_authority(host_device_ura, authority)
            .at(TEST_NOW_MS)
            .resolve_route(&owner_ura, &ability_ura)
            .expect("owner + full Ability URA must resolve through the same route gate");

        assert_eq!(route.kind(), SelectedRouteKind::RoutableAgentOwned);
        assert_eq!(route.owner_ura, owner_ura);
        assert_eq!(route.ability_ura, ability_ura);
        assert_eq!(route.dispatch_name, "agent.list");
    }

    #[test]
    fn route_selector_carries_owner_kind_from_ability_selector() {
        let device_ura = device_owner_ura();
        let device_ability =
            crate::core::ura::owner_ability_ura(&device_ura, "agent.list").expect("device ability");
        let device_selector = route_selector_from_query(&device_ability, "")
            .expect("device selector")
            .expect("device selector present");
        assert_eq!(
            device_selector.owner_kind,
            RouteOwnerKind::LegacyDeviceProfileProjection
        );

        let authority_ura = crate::core::ura::authority_ura("test-realm");
        let authority_ability =
            crate::core::ura::owner_ability_ura(&authority_ura, "meta.list_abilities")
                .expect("authority ability");
        let authority_selector = route_selector_from_query(&authority_ability, "")
            .expect("authority selector")
            .expect("authority selector present");
        assert_eq!(authority_selector.owner_kind, RouteOwnerKind::Authority);

        let agent_ura = crate::core::ura::agent_ura("test-realm", "alice", "worker");
        let agent_ability =
            crate::core::ura::owner_ability_ura(&agent_ura, "chat").expect("agent ability");
        let agent_selector = route_selector_from_query(&agent_ura, &agent_ability)
            .expect("agent selector")
            .expect("agent selector present");
        assert_eq!(agent_selector.owner_kind, RouteOwnerKind::Agent);

        let system_agent_ura = crate::core::ura::device_agent_ura(
            "test-realm",
            "device-a",
            crate::daemon::ability::names::device_control::TERMINAL_SYSTEM_AGENT_ID,
        );
        let system_agent_ability =
            crate::core::ura::owner_ability_ura(&system_agent_ura, "terminal.list")
                .expect("system-agent ability");
        let system_agent_selector = route_selector_from_query(&system_agent_ability, "")
            .expect("system-agent selector")
            .expect("system-agent selector present");
        assert_eq!(
            system_agent_selector.owner_kind,
            RouteOwnerKind::SystemAgent
        );
    }

    #[test]
    fn malformed_descriptor_ref_does_not_fall_through_as_public_name() {
        let registry = PresenceRegistry::new();
        let catalog = AbilityCatalogStore::new();
        let owner_ura = device_owner_ura();
        mark_online(&registry, &owner_ura);
        let ability_ura =
            crate::core::ura::owner_ability_ura(&owner_ura, "agent.list").expect("ability ura");
        let old_short_descriptor_ref = format!("{ability_ura}@1.0.0");
        let authority = FakeLocalRuntimeAuthority::with_owner_keys(&owner_ura, &["agent.list"]);
        let resolver = DaemonRouteResolver::new(&registry, None, &catalog)
            .with_local_catalog_authority(owner_ura.clone(), authority)
            .at(TEST_NOW_MS);

        let target_bound_failure = resolver
            .resolve_route(owner_ura.as_str(), old_short_descriptor_ref.as_str())
            .expect_err(
                "target-bound malformed descriptor refs must fail before public-name fallback",
            );
        assert_eq!(target_bound_failure.reason, NegativeReason::Refused);
        assert!(
            target_bound_failure
                .detail
                .contains("descriptor_ref selector projection failed"),
            "unexpected failure detail: {}",
            target_bound_failure.detail
        );

        let owner_query_failure = resolver
            .resolve_route(old_short_descriptor_ref.as_str(), "")
            .expect_err("descriptor-like input must not be accepted as an owner query");
        assert_eq!(owner_query_failure.reason, NegativeReason::Refused);
        assert!(
            owner_query_failure
                .detail
                .contains("cannot stand in for an owner query"),
            "unexpected failure detail: {}",
            owner_query_failure.detail
        );
    }

    #[test]
    fn descriptor_ref_owner_mismatch_fails_before_route_lookup() {
        let registry = PresenceRegistry::new();
        let catalog = AbilityCatalogStore::new();
        let owner_ura = device_owner_ura();
        let other_owner_ura = crate::core::ura::device_ura("test-realm", "other-daemon");
        mark_online(&registry, &owner_ura);
        let ability_ura = crate::core::ura::owner_ability_ura(&other_owner_ura, "agent.list")
            .expect("ability ura");
        let descriptor_ref = descriptor_ref_for_test(&ability_ura);
        let authority = FakeLocalRuntimeAuthority::with_owner_keys(&owner_ura, &["agent.list"]);
        let resolver = DaemonRouteResolver::new(&registry, None, &catalog)
            .with_local_catalog_authority(owner_ura.clone(), authority)
            .at(TEST_NOW_MS);

        let failure = resolver
            .resolve_route(&owner_ura, &descriptor_ref)
            .expect_err("descriptor ref owner mismatch must not degrade to route lookup miss");
        assert_eq!(failure.reason, NegativeReason::Refused);
        assert!(
            failure.detail.contains("descriptor_ref ability owner"),
            "unexpected failure detail: {}",
            failure.detail
        );
    }

    #[test]
    fn runtime_introspection_resource_abilities_resolve_from_local_authority() {
        let registry = PresenceRegistry::new();
        let catalog = AbilityCatalogStore::new();
        let host_device_ura = device_owner_ura();
        let profile_keys = ["meta.list_resources"];
        let owner_ura = system_agent_owner_ura_for(profile_keys[0]);
        mark_online(&registry, &host_device_ura);
        // The SystemAgent route is proven from the live runtime dispatch
        // table, not from a hub projection — the catalog stays empty.
        let authority = FakeLocalRuntimeAuthority::with_owner_keys(&owner_ura, &profile_keys);

        let resolver = DaemonRouteResolver::new(&registry, None, &catalog)
            .with_local_catalog_authority(host_device_ura.clone(), authority)
            .at(TEST_NOW_MS);
        for ability in profile_keys {
            let route = resolver
                .resolve_route(&owner_ura, ability)
                .unwrap_or_else(|err| {
                    panic!("{ability} must resolve from local SystemAgent authority: {err:?}")
                });
            let expected_ability_ura =
                crate::core::ura::owner_ability_ura(&owner_ura, ability).expect("ability ura");

            assert_eq!(route.kind(), SelectedRouteKind::RoutableAgentOwned);
            assert_eq!(route.owner_ura, owner_ura);
            assert_eq!(route.callee_ura, owner_ura);
            assert_eq!(route.execution_host_ura, host_device_ura);
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
    fn terminal_system_agent_ability_resolves_from_local_device_authority() {
        let registry = PresenceRegistry::new();
        let catalog = AbilityCatalogStore::new();
        let host_device_ura = device_owner_ura();
        mark_online(&registry, &host_device_ura);
        let terminal_owner_ura = crate::core::ura::device_agent_ura(
            "test-realm",
            "device-a",
            crate::daemon::ability::names::device_control::TERMINAL_SYSTEM_AGENT_ID,
        );
        let authority =
            FakeLocalRuntimeAuthority::with_owner_keys(&terminal_owner_ura, &["terminal.list"]);

        let route = DaemonRouteResolver::new(&registry, None, &catalog)
            .with_local_catalog_authority(host_device_ura.clone(), authority)
            .at(TEST_NOW_MS)
            .resolve_route(&terminal_owner_ura, "terminal.list")
            .expect("terminal SystemAgent ability must resolve from local device authority");
        let expected_ability_ura =
            crate::core::ura::owner_ability_ura(&terminal_owner_ura, "terminal.list")
                .expect("terminal ability ura");

        assert_eq!(route.kind(), SelectedRouteKind::RoutableAgentOwned);
        assert_eq!(route.owner_ura, terminal_owner_ura);
        assert_eq!(route.callee_ura, route.owner_ura);
        assert_eq!(route.execution_host_ura, host_device_ura);
        assert_eq!(route.ability_ura, expected_ability_ura);
        assert_eq!(
            route.route_ura,
            format!("route-ref::{expected_ability_ura}")
        );
        assert_eq!(route.dispatch_name, "terminal.list");
        assert!(route.is_authoritative_local_or_better());
    }

    #[test]
    fn session_system_agent_ability_resolves_from_local_device_authority() {
        let registry = PresenceRegistry::new();
        let catalog = AbilityCatalogStore::new();
        let host_device_ura = device_owner_ura();
        mark_online(&registry, &host_device_ura);
        let session_owner_ura = crate::core::ura::device_agent_ura(
            "test-realm",
            "device-a",
            crate::daemon::ability::names::device_control::SESSION_SYSTEM_AGENT_ID,
        );
        let authority = FakeLocalRuntimeAuthority::with_owner_keys(
            &session_owner_ura,
            &[crate::daemon::ability::names::device_control::SESSION_LIST],
        );

        let route = DaemonRouteResolver::new(&registry, None, &catalog)
            .with_local_catalog_authority(host_device_ura.clone(), authority)
            .at(TEST_NOW_MS)
            .resolve_route(
                &session_owner_ura,
                crate::daemon::ability::names::device_control::SESSION_LIST,
            )
            .expect("session SystemAgent ability must resolve from local device authority");
        let expected_ability_ura = crate::core::ura::owner_ability_ura(
            &session_owner_ura,
            crate::daemon::ability::names::device_control::SESSION_LIST,
        )
        .expect("session ability ura");

        assert_eq!(route.kind(), SelectedRouteKind::RoutableAgentOwned);
        assert_eq!(route.owner_ura, session_owner_ura);
        assert_eq!(route.callee_ura, route.owner_ura);
        assert_eq!(route.execution_host_ura, host_device_ura);
        assert_eq!(route.ability_ura, expected_ability_ura);
        assert_eq!(
            route.dispatch_name,
            crate::daemon::ability::names::device_control::SESSION_LIST
        );
        assert!(route.is_authoritative_local_or_better());
    }

    #[test]
    fn runtime_governance_system_agent_ability_resolves_from_local_device_authority() {
        let registry = PresenceRegistry::new();
        let catalog = AbilityCatalogStore::new();
        let host_device_ura = device_owner_ura();
        mark_online(&registry, &host_device_ura);
        let governance_owner_ura = crate::core::ura::device_agent_ura(
            "test-realm",
            "device-a",
            crate::daemon::ability::names::governance::RUNTIME_GOVERNANCE_SYSTEM_AGENT_ID,
        );
        let ability = crate::daemon::ability::names::governance::AUTHORITY_BINDING_LIST;
        let authority =
            FakeLocalRuntimeAuthority::with_owner_keys(&governance_owner_ura, &[ability]);

        let route = DaemonRouteResolver::new(&registry, None, &catalog)
            .with_local_catalog_authority(host_device_ura.clone(), authority)
            .at(TEST_NOW_MS)
            .resolve_route(&governance_owner_ura, ability)
            .expect(
                "runtime-governance SystemAgent ability must resolve from local device authority",
            );
        let expected_ability_ura =
            crate::core::ura::owner_ability_ura(&governance_owner_ura, ability)
                .expect("runtime-governance ability ura");

        assert_eq!(route.kind(), SelectedRouteKind::RoutableAgentOwned);
        assert_eq!(route.owner_ura, governance_owner_ura);
        assert_eq!(route.callee_ura, route.owner_ura);
        assert_eq!(route.execution_host_ura, host_device_ura);
        assert_eq!(route.ability_ura, expected_ability_ura);
        assert_eq!(route.dispatch_name, ability);
        assert!(route.is_authoritative_local_or_better());
    }

    #[test]
    fn runtime_health_system_agent_ability_resolves_from_local_device_authority() {
        let registry = PresenceRegistry::new();
        let catalog = AbilityCatalogStore::new();
        let host_device_ura = device_owner_ura();
        mark_online(&registry, &host_device_ura);
        let health_owner_ura = crate::core::ura::device_agent_ura(
            "test-realm",
            "device-a",
            crate::daemon::ability::names::governance::RUNTIME_HEALTH_SYSTEM_AGENT_ID,
        );
        let ability = crate::daemon::ability::names::governance::OBSERVE_HEALTH;
        let authority = FakeLocalRuntimeAuthority::with_owner_keys(&health_owner_ura, &[ability]);

        let route = DaemonRouteResolver::new(&registry, None, &catalog)
            .with_local_catalog_authority(host_device_ura.clone(), authority)
            .at(TEST_NOW_MS)
            .resolve_route(&health_owner_ura, ability)
            .expect("runtime-health SystemAgent ability must resolve from local device authority");
        let expected_ability_ura = crate::core::ura::owner_ability_ura(&health_owner_ura, ability)
            .expect("runtime-health ability ura");

        assert_eq!(route.kind(), SelectedRouteKind::RoutableAgentOwned);
        assert_eq!(route.owner_ura, health_owner_ura);
        assert_eq!(route.callee_ura, route.owner_ura);
        assert_eq!(route.execution_host_ura, host_device_ura);
        assert_eq!(route.ability_ura, expected_ability_ura);
        assert_eq!(route.dispatch_name, ability);
        assert!(route.is_authoritative_local_or_better());
    }

    #[test]
    fn runtime_introspection_system_agent_ability_resolves_from_local_device_authority() {
        let registry = PresenceRegistry::new();
        let catalog = AbilityCatalogStore::new();
        let host_device_ura = device_owner_ura();
        mark_online(&registry, &host_device_ura);
        let introspection_owner_ura = crate::core::ura::device_agent_ura(
            "test-realm",
            "device-a",
            crate::daemon::ability::names::governance::RUNTIME_INTROSPECTION_SYSTEM_AGENT_ID,
        );
        let ability = crate::daemon::ability::names::governance::META_LIST_ABILITIES;
        let authority =
            FakeLocalRuntimeAuthority::with_owner_keys(&introspection_owner_ura, &[ability]);

        let route = DaemonRouteResolver::new(&registry, None, &catalog)
            .with_local_catalog_authority(host_device_ura.clone(), authority)
            .at(TEST_NOW_MS)
            .resolve_route(&introspection_owner_ura, ability)
            .expect(
                "runtime-introspection SystemAgent ability must resolve from local device authority",
            );
        let expected_ability_ura =
            crate::core::ura::owner_ability_ura(&introspection_owner_ura, ability)
                .expect("runtime-introspection ability ura");

        assert_eq!(route.kind(), SelectedRouteKind::RoutableAgentOwned);
        assert_eq!(route.owner_ura, introspection_owner_ura);
        assert_eq!(route.callee_ura, route.owner_ura);
        assert_eq!(route.execution_host_ura, host_device_ura);
        assert_eq!(route.ability_ura, expected_ability_ura);
        assert_eq!(route.dispatch_name, ability);
        assert!(route.is_authoritative_local_or_better());
    }

    #[test]
    fn descriptor_transfer_system_agent_ability_resolves_from_local_device_authority() {
        let registry = PresenceRegistry::new();
        let catalog = AbilityCatalogStore::new();
        let host_device_ura = device_owner_ura();
        mark_online(&registry, &host_device_ura);
        let descriptor_transfer_owner_ura = crate::core::ura::device_agent_ura(
            "test-realm",
            "device-a",
            crate::daemon::ability::names::governance::DESCRIPTOR_TRANSFER_SYSTEM_AGENT_ID,
        );
        let ability = crate::daemon::ability::names::governance::META_ACQUIRE;
        let authority =
            FakeLocalRuntimeAuthority::with_owner_keys(&descriptor_transfer_owner_ura, &[ability]);

        let route = DaemonRouteResolver::new(&registry, None, &catalog)
            .with_local_catalog_authority(host_device_ura.clone(), authority)
            .at(TEST_NOW_MS)
            .resolve_route(&descriptor_transfer_owner_ura, ability)
            .expect(
                "descriptor-transfer SystemAgent ability must resolve from local device authority",
            );
        let expected_ability_ura =
            crate::core::ura::owner_ability_ura(&descriptor_transfer_owner_ura, ability)
                .expect("descriptor-transfer ability ura");

        assert_eq!(route.kind(), SelectedRouteKind::RoutableAgentOwned);
        assert_eq!(route.owner_ura, descriptor_transfer_owner_ura);
        assert_eq!(route.callee_ura, route.owner_ura);
        assert_eq!(route.execution_host_ura, host_device_ura);
        assert_eq!(route.ability_ura, expected_ability_ura);
        assert_eq!(route.dispatch_name, ability);
        assert!(route.is_authoritative_local_or_better());
    }

    #[test]
    fn node_management_system_agent_ability_resolves_from_local_device_authority() {
        let registry = PresenceRegistry::new();
        let catalog = AbilityCatalogStore::new();
        let host_device_ura = device_owner_ura();
        mark_online(&registry, &host_device_ura);
        let node_management_owner_ura = crate::core::ura::device_agent_ura(
            "test-realm",
            "device-a",
            crate::daemon::ability::names::device_control::NODE_MANAGEMENT_SYSTEM_AGENT_ID,
        );
        let ability = crate::daemon::ability::names::device_control::NODE_DESCRIBE;
        let authority =
            FakeLocalRuntimeAuthority::with_owner_keys(&node_management_owner_ura, &[ability]);

        let route = DaemonRouteResolver::new(&registry, None, &catalog)
            .with_local_catalog_authority(host_device_ura.clone(), authority)
            .at(TEST_NOW_MS)
            .resolve_route(&node_management_owner_ura, ability)
            .expect("node-management SystemAgent ability must resolve from local device authority");
        let expected_ability_ura =
            crate::core::ura::owner_ability_ura(&node_management_owner_ura, ability)
                .expect("node-management ability ura");

        assert_eq!(route.kind(), SelectedRouteKind::RoutableAgentOwned);
        assert_eq!(route.owner_ura, node_management_owner_ura);
        assert_eq!(route.callee_ura, route.owner_ura);
        assert_eq!(route.execution_host_ura, host_device_ura);
        assert_eq!(route.ability_ura, expected_ability_ura);
        assert_eq!(route.dispatch_name, ability);
        assert!(route.is_authoritative_local_or_better());
    }

    #[test]
    fn invocation_governance_system_agent_ability_resolves_from_local_device_authority() {
        let registry = PresenceRegistry::new();
        let catalog = AbilityCatalogStore::new();
        let host_device_ura = device_owner_ura();
        mark_online(&registry, &host_device_ura);
        let governance_owner_ura = crate::core::ura::device_agent_ura(
            "test-realm",
            "device-a",
            crate::daemon::ability::names::governance::RUNTIME_GOVERNANCE_SYSTEM_AGENT_ID,
        );
        let ability = crate::daemon::ability::names::governance::INVOCATION_HISTORY_LIST;
        let authority =
            FakeLocalRuntimeAuthority::with_owner_keys(&governance_owner_ura, &[ability]);

        let route = DaemonRouteResolver::new(&registry, None, &catalog)
            .with_local_catalog_authority(host_device_ura.clone(), authority)
            .at(TEST_NOW_MS)
            .resolve_route(&governance_owner_ura, ability)
            .expect(
                "invocation governance ability must resolve through runtime-governance SystemAgent",
            );
        let expected_ability_ura =
            crate::core::ura::owner_ability_ura(&governance_owner_ura, ability)
                .expect("invocation governance ability ura");

        assert_eq!(route.kind(), SelectedRouteKind::RoutableAgentOwned);
        assert_eq!(route.owner_ura, governance_owner_ura);
        assert_eq!(route.callee_ura, route.owner_ura);
        assert_eq!(route.execution_host_ura, host_device_ura);
        assert_eq!(route.ability_ura, expected_ability_ura);
        assert_eq!(route.dispatch_name, ability);
        assert!(route.is_authoritative_local_or_better());
    }

    #[test]
    fn ability_management_system_agent_ability_resolves_from_local_device_authority() {
        let registry = PresenceRegistry::new();
        let catalog = AbilityCatalogStore::new();
        let host_device_ura = device_owner_ura();
        mark_online(&registry, &host_device_ura);
        let ability_management_owner_ura = crate::core::ura::device_agent_ura(
            "test-realm",
            "device-a",
            crate::daemon::ability::names::federation::ABILITY_MANAGEMENT_SYSTEM_AGENT_ID,
        );
        let ability = crate::daemon::ability::names::federation::ABILITY_DEPLOY;
        let authority =
            FakeLocalRuntimeAuthority::with_owner_keys(&ability_management_owner_ura, &[ability]);

        let route = DaemonRouteResolver::new(&registry, None, &catalog)
            .with_local_catalog_authority(host_device_ura.clone(), authority)
            .at(TEST_NOW_MS)
            .resolve_route(&ability_management_owner_ura, ability)
            .expect(
                "ability-management SystemAgent ability must resolve from local device authority",
            );
        let expected_ability_ura =
            crate::core::ura::owner_ability_ura(&ability_management_owner_ura, ability)
                .expect("ability-management ability ura");

        assert_eq!(route.kind(), SelectedRouteKind::RoutableAgentOwned);
        assert_eq!(route.owner_ura, ability_management_owner_ura);
        assert_eq!(route.callee_ura, route.owner_ura);
        assert_eq!(route.execution_host_ura, host_device_ura);
        assert_eq!(route.ability_ura, expected_ability_ura);
        assert_eq!(route.dispatch_name, ability);
        assert!(route.is_authoritative_local_or_better());
    }

    #[test]
    fn openai_compat_system_agent_ability_resolves_from_local_device_authority() {
        let registry = PresenceRegistry::new();
        let catalog = AbilityCatalogStore::new();
        let host_device_ura = device_owner_ura();
        mark_online(&registry, &host_device_ura);
        let openai_owner_ura = crate::core::ura::device_agent_ura(
            "test-realm",
            "device-a",
            crate::daemon::ability::names::integrations::OPENAI_COMPAT_SYSTEM_AGENT_ID,
        );
        let ability = crate::daemon::ability::names::integrations::OPENAI_LIST_MODELS;
        let authority = FakeLocalRuntimeAuthority::with_owner_keys(&openai_owner_ura, &[ability]);

        let route = DaemonRouteResolver::new(&registry, None, &catalog)
            .with_local_catalog_authority(host_device_ura.clone(), authority)
            .at(TEST_NOW_MS)
            .resolve_route(&openai_owner_ura, ability)
            .expect("openai-compat SystemAgent ability must resolve from local device authority");
        let expected_ability_ura = crate::core::ura::owner_ability_ura(&openai_owner_ura, ability)
            .expect("openai-compat ability ura");

        assert_eq!(route.kind(), SelectedRouteKind::RoutableAgentOwned);
        assert_eq!(route.owner_ura, openai_owner_ura);
        assert_eq!(route.callee_ura, route.owner_ura);
        assert_eq!(route.execution_host_ura, host_device_ura);
        assert_eq!(route.ability_ura, expected_ability_ura);
        assert_eq!(route.dispatch_name, ability);
        assert!(route.is_authoritative_local_or_better());
    }

    #[test]
    fn api_key_management_system_agent_ability_resolves_from_local_device_authority() {
        let registry = PresenceRegistry::new();
        let catalog = AbilityCatalogStore::new();
        let host_device_ura = device_owner_ura();
        mark_online(&registry, &host_device_ura);
        let api_key_owner_ura = crate::core::ura::device_agent_ura(
            "test-realm",
            "device-a",
            crate::daemon::ability::names::governance::API_KEY_MANAGEMENT_SYSTEM_AGENT_ID,
        );
        let ability = "alice.api_key.list";
        let authority = FakeLocalRuntimeAuthority::with_owner_keys(&api_key_owner_ura, &[ability]);

        let route = DaemonRouteResolver::new(&registry, None, &catalog)
            .with_local_catalog_authority(host_device_ura.clone(), authority)
            .at(TEST_NOW_MS)
            .resolve_route(&api_key_owner_ura, ability)
            .expect(
                "api-key-management SystemAgent ability must resolve from local device authority",
            );
        let expected_ability_ura = crate::core::ura::owner_ability_ura(&api_key_owner_ura, ability)
            .expect("api-key-management ability ura");

        assert_eq!(route.kind(), SelectedRouteKind::RoutableAgentOwned);
        assert_eq!(route.owner_ura, api_key_owner_ura);
        assert_eq!(route.callee_ura, route.owner_ura);
        assert_eq!(route.execution_host_ura, host_device_ura);
        assert_eq!(route.ability_ura, expected_ability_ura);
        assert_eq!(route.dispatch_name, ability);
        assert!(route.is_authoritative_local_or_better());
    }

    #[test]
    fn keyring_management_system_agent_ability_resolves_from_local_device_authority() {
        let registry = PresenceRegistry::new();
        let catalog = AbilityCatalogStore::new();
        let host_device_ura = device_owner_ura();
        mark_online(&registry, &host_device_ura);
        let keyring_owner_ura = crate::core::ura::device_agent_ura(
            "test-realm",
            "device-a",
            crate::daemon::ability::names::governance::KEYRING_MANAGEMENT_SYSTEM_AGENT_ID,
        );
        let ability = "device.keyring.list";
        let authority = FakeLocalRuntimeAuthority::with_owner_keys(&keyring_owner_ura, &[ability]);

        let route = DaemonRouteResolver::new(&registry, None, &catalog)
            .with_local_catalog_authority(host_device_ura.clone(), authority)
            .at(TEST_NOW_MS)
            .resolve_route(&keyring_owner_ura, ability)
            .expect(
                "keyring-management SystemAgent ability must resolve from local device authority",
            );
        let expected_ability_ura = crate::core::ura::owner_ability_ura(&keyring_owner_ura, ability)
            .expect("keyring-management ability ura");

        assert_eq!(route.kind(), SelectedRouteKind::RoutableAgentOwned);
        assert_eq!(route.owner_ura, keyring_owner_ura);
        assert_eq!(route.callee_ura, route.owner_ura);
        assert_eq!(route.execution_host_ura, host_device_ura);
        assert_eq!(route.ability_ura, expected_ability_ura);
        assert_eq!(route.dispatch_name, ability);
        assert!(route.is_authoritative_local_or_better());
    }

    #[test]
    fn a2a_integration_system_agent_ability_resolves_from_local_device_authority() {
        let registry = PresenceRegistry::new();
        let catalog = AbilityCatalogStore::new();
        let host_device_ura = device_owner_ura();
        mark_online(&registry, &host_device_ura);
        let a2a_owner_ura = crate::core::ura::device_agent_ura(
            "test-realm",
            "device-a",
            crate::daemon::ability::names::integrations::A2A_INTEGRATION_SYSTEM_AGENT_ID,
        );
        let ability = crate::daemon::ability::names::integrations::A2A_CLIENT_SEND_TASK;
        let authority = FakeLocalRuntimeAuthority::with_owner_keys(&a2a_owner_ura, &[ability]);

        let route = DaemonRouteResolver::new(&registry, None, &catalog)
            .with_local_catalog_authority(host_device_ura.clone(), authority)
            .at(TEST_NOW_MS)
            .resolve_route(&a2a_owner_ura, ability)
            .expect("a2a-integration SystemAgent ability must resolve from local device authority");
        let expected_ability_ura = crate::core::ura::owner_ability_ura(&a2a_owner_ura, ability)
            .expect("a2a-integration ability ura");

        assert_eq!(route.kind(), SelectedRouteKind::RoutableAgentOwned);
        assert_eq!(route.owner_ura, a2a_owner_ura);
        assert_eq!(route.callee_ura, route.owner_ura);
        assert_eq!(route.execution_host_ura, host_device_ura);
        assert_eq!(route.ability_ura, expected_ability_ura);
        assert_eq!(route.dispatch_name, ability);
        assert!(route.is_authoritative_local_or_better());
    }

    #[test]
    fn locomotion_system_agent_ability_resolves_from_local_device_authority() {
        let registry = PresenceRegistry::new();
        let catalog = AbilityCatalogStore::new();
        let host_device_ura = device_owner_ura();
        mark_online(&registry, &host_device_ura);
        let locomotion_owner_ura = crate::core::ura::device_agent_ura(
            "test-realm",
            "device-a",
            crate::daemon::ability::names::device_control::LOCOMOTION_SYSTEM_AGENT_ID,
        );
        let authority = FakeLocalRuntimeAuthority::with_owner_keys(
            &locomotion_owner_ura,
            &[crate::daemon::ability::names::device_control::FS_READ],
        );

        let route = DaemonRouteResolver::new(&registry, None, &catalog)
            .with_local_catalog_authority(host_device_ura.clone(), authority)
            .at(TEST_NOW_MS)
            .resolve_route(
                &locomotion_owner_ura,
                crate::daemon::ability::names::device_control::FS_READ,
            )
            .expect("locomotion SystemAgent ability must resolve from local device authority");
        let expected_ability_ura = crate::core::ura::owner_ability_ura(
            &locomotion_owner_ura,
            crate::daemon::ability::names::device_control::FS_READ,
        )
        .expect("locomotion ability ura");

        assert_eq!(route.kind(), SelectedRouteKind::RoutableAgentOwned);
        assert_eq!(route.owner_ura, locomotion_owner_ura);
        assert_eq!(route.callee_ura, route.owner_ura);
        assert_eq!(route.execution_host_ura, host_device_ura);
        assert_eq!(route.ability_ura, expected_ability_ura);
        assert_eq!(
            route.route_ura,
            format!("route-ref::{expected_ability_ura}")
        );
        assert_eq!(
            route.dispatch_name,
            crate::daemon::ability::names::device_control::FS_READ
        );
        assert!(route.is_authoritative_local_or_better());
    }

    #[test]
    fn registered_skill_list_routes_from_the_live_control_plane_snapshot() {
        let registry = PresenceRegistry::new();
        let projection_store = AbilityCatalogStore::new();
        let host_device_ura = device_owner_ura();
        mark_online(&registry, &host_device_ura);
        let skill_owner_ura = crate::core::ura::device_agent_ura(
            "test-realm",
            "test-daemon",
            crate::daemon::ability::names::resources::SKILL_MANAGEMENT_SYSTEM_AGENT_ID,
        );
        mark_online(&registry, &skill_owner_ura);

        let mut live_catalog =
            crate::daemon::ability::dispatch::AxonAbilityCatalog::new_with_runtime_and_authority_context(
                crate::daemon::axon_bridge::runtime_factory::build_local_runtime(
                    crate::daemon::axon_bridge::runtime_factory::rejecting_test_key_resolver(),
                    None,
                ),
                crate::daemon::ability::dispatch::AbilityAuthorityContext::for_device_authority_root(
                    host_device_ura.clone(),
                )
                .expect("device authority"),
            );
        crate::daemon::ability::builtins::resources::skills::publish::register(&mut live_catalog);
        let authority = LocalAbilityPublicationSnapshot::capture(&live_catalog);

        let route = DaemonRouteResolver::new(&registry, None, &projection_store)
            .with_local_catalog_authority(host_device_ura.clone(), authority)
            .at(TEST_NOW_MS)
            .resolve_route(&skill_owner_ura, "skill.list")
            .expect("registered skill.list must route from the live catalog");
        let expected_ability_ura =
            crate::core::ura::owner_ability_ura(&skill_owner_ura, "skill.list")
                .expect("skill ability ura");

        assert_eq!(route.kind(), SelectedRouteKind::RoutableAgentOwned);
        assert_eq!(route.owner_ura, skill_owner_ura);
        assert_eq!(route.callee_ura, route.owner_ura);
        assert_eq!(route.execution_host_ura, host_device_ura);
        assert_eq!(route.ability_ura, expected_ability_ura);
        assert_eq!(route.dispatch_name, "skill.list");
        assert!(route.is_authoritative_local_or_better());
    }

    #[test]
    fn context_system_agent_ability_resolves_from_local_device_authority() {
        let registry = PresenceRegistry::new();
        let catalog = AbilityCatalogStore::new();
        let host_device_ura = device_owner_ura();
        mark_online(&registry, &host_device_ura);
        let context_owner_ura = crate::core::ura::device_agent_ura(
            "test-realm",
            "test-daemon",
            crate::daemon::ability::names::resources::CONTEXT_SYSTEM_AGENT_ID,
        );
        mark_online(&registry, &context_owner_ura);
        let authority = FakeLocalRuntimeAuthority::with_owner_keys(
            &context_owner_ura,
            &[crate::daemon::ability::names::resources::CONTEXT_FS_LIST],
        );

        let route = DaemonRouteResolver::new(&registry, None, &catalog)
            .with_local_catalog_authority(host_device_ura.clone(), authority)
            .at(TEST_NOW_MS)
            .resolve_route(
                &context_owner_ura,
                crate::daemon::ability::names::resources::CONTEXT_FS_LIST,
            )
            .expect("context SystemAgent ability must resolve from local device authority");
        let expected_ability_ura = crate::core::ura::owner_ability_ura(
            &context_owner_ura,
            crate::daemon::ability::names::resources::CONTEXT_FS_LIST,
        )
        .expect("context ability ura");

        assert_eq!(route.kind(), SelectedRouteKind::RoutableAgentOwned);
        assert_eq!(route.owner_ura, context_owner_ura);
        assert_eq!(route.callee_ura, route.owner_ura);
        assert_eq!(route.execution_host_ura, host_device_ura);
        assert_eq!(route.ability_ura, expected_ability_ura);
        assert_eq!(
            route.dispatch_name,
            crate::daemon::ability::names::resources::CONTEXT_FS_LIST
        );
        assert!(route.is_authoritative_local_or_better());
    }

    #[test]
    fn media_system_agent_ability_resolves_from_local_device_authority() {
        let registry = PresenceRegistry::new();
        let catalog = AbilityCatalogStore::new();
        let host_device_ura = device_owner_ura();
        mark_online(&registry, &host_device_ura);
        let media_owner_ura = crate::core::ura::device_agent_ura(
            "test-realm",
            "test-daemon",
            crate::daemon::ability::names::resources::MEDIA_SYSTEM_AGENT_ID,
        );
        mark_online(&registry, &media_owner_ura);
        let authority = FakeLocalRuntimeAuthority::with_owner_keys(
            &media_owner_ura,
            &[crate::daemon::ability::names::resources::MEDIA_CAMERA_SNAPSHOT],
        );

        let route = DaemonRouteResolver::new(&registry, None, &catalog)
            .with_local_catalog_authority(host_device_ura.clone(), authority)
            .at(TEST_NOW_MS)
            .resolve_route(
                &media_owner_ura,
                crate::daemon::ability::names::resources::MEDIA_CAMERA_SNAPSHOT,
            )
            .expect("media SystemAgent ability must resolve from local device authority");
        let expected_ability_ura = crate::core::ura::owner_ability_ura(
            &media_owner_ura,
            crate::daemon::ability::names::resources::MEDIA_CAMERA_SNAPSHOT,
        )
        .expect("media ability ura");

        assert_eq!(route.kind(), SelectedRouteKind::RoutableAgentOwned);
        assert_eq!(route.owner_ura, media_owner_ura);
        assert_eq!(route.callee_ura, route.owner_ura);
        assert_eq!(route.execution_host_ura, host_device_ura);
        assert_eq!(route.ability_ura, expected_ability_ura);
        assert_eq!(
            route.dispatch_name,
            crate::daemon::ability::names::resources::MEDIA_CAMERA_SNAPSHOT
        );
        assert!(route.is_authoritative_local_or_better());
    }

    #[test]
    fn plugin_management_system_agent_ability_resolves_from_local_device_authority() {
        let registry = PresenceRegistry::new();
        let catalog = AbilityCatalogStore::new();
        let host_device_ura = device_owner_ura();
        mark_online(&registry, &host_device_ura);
        let plugin_owner_ura = crate::core::ura::device_agent_ura(
            "test-realm",
            "test-daemon",
            crate::daemon::ability::names::integrations::PLUGIN_MANAGEMENT_SYSTEM_AGENT_ID,
        );
        mark_online(&registry, &plugin_owner_ura);
        let authority = FakeLocalRuntimeAuthority::with_owner_keys(
            &plugin_owner_ura,
            &[crate::daemon::ability::names::integrations::PLUGIN_STATUS],
        );

        let route = DaemonRouteResolver::new(&registry, None, &catalog)
            .with_local_catalog_authority(host_device_ura.clone(), authority)
            .at(TEST_NOW_MS)
            .resolve_route(
                &plugin_owner_ura,
                crate::daemon::ability::names::integrations::PLUGIN_STATUS,
            )
            .expect(
                "plugin-management SystemAgent ability must resolve from local device authority",
            );
        let expected_ability_ura = crate::core::ura::owner_ability_ura(
            &plugin_owner_ura,
            crate::daemon::ability::names::integrations::PLUGIN_STATUS,
        )
        .expect("plugin-management ability ura");

        assert_eq!(route.kind(), SelectedRouteKind::RoutableAgentOwned);
        assert_eq!(route.owner_ura, plugin_owner_ura);
        assert_eq!(route.callee_ura, route.owner_ura);
        assert_eq!(route.execution_host_ura, host_device_ura);
        assert_eq!(route.ability_ura, expected_ability_ura);
        assert_eq!(
            route.dispatch_name,
            crate::daemon::ability::names::integrations::PLUGIN_STATUS
        );
        assert!(route.is_authoritative_local_or_better());
    }

    #[test]
    fn automation_system_agent_ability_resolves_from_local_device_authority() {
        let registry = PresenceRegistry::new();
        let catalog = AbilityCatalogStore::new();
        let host_device_ura = device_owner_ura();
        mark_online(&registry, &host_device_ura);
        let automation_owner_ura = crate::core::ura::device_agent_ura(
            "test-realm",
            "test-daemon",
            crate::daemon::ability::names::automation::AUTOMATION_SYSTEM_AGENT_ID,
        );
        mark_online(&registry, &automation_owner_ura);
        let authority = FakeLocalRuntimeAuthority::with_owner_keys(
            &automation_owner_ura,
            &[crate::daemon::ability::names::automation::MISSION_RUN],
        );

        let route = DaemonRouteResolver::new(&registry, None, &catalog)
            .with_local_catalog_authority(host_device_ura.clone(), authority)
            .at(TEST_NOW_MS)
            .resolve_route(
                &automation_owner_ura,
                crate::daemon::ability::names::automation::MISSION_RUN,
            )
            .expect("automation SystemAgent ability must resolve from local device authority");
        let expected_ability_ura = crate::core::ura::owner_ability_ura(
            &automation_owner_ura,
            crate::daemon::ability::names::automation::MISSION_RUN,
        )
        .expect("automation ability ura");

        assert_eq!(route.kind(), SelectedRouteKind::RoutableAgentOwned);
        assert_eq!(route.owner_ura, automation_owner_ura);
        assert_eq!(route.callee_ura, route.owner_ura);
        assert_eq!(route.execution_host_ura, host_device_ura);
        assert_eq!(route.ability_ura, expected_ability_ura);
        assert_eq!(
            route.dispatch_name,
            crate::daemon::ability::names::automation::MISSION_RUN
        );
        assert!(route.is_authoritative_local_or_better());
    }

    #[test]
    fn owner_absent_resolves_nxdomain() {
        let registry = PresenceRegistry::new();
        let catalog = AbilityCatalogStore::new();
        let owner_ura = system_agent_owner_ura_for("agent.list");
        // No presence entry, no advertised host record.

        let failure = DaemonRouteResolver::new(&registry, None, &catalog)
            .at(TEST_NOW_MS)
            .resolve_route(&owner_ura, "agent.list")
            .expect_err("absent owner must resolve negative");

        assert_eq!(failure.reason, NegativeReason::Nxdomain);
        assert_eq!(failure.query_name, format!("{owner_ura}#agent.list"));
    }

    #[test]
    fn owner_advertised_but_offline_resolves_noroute() {
        let registry = PresenceRegistry::new();
        let advertised = AdvertisedAgentStore::new();
        let catalog = AbilityCatalogStore::new();
        let agent_ura = crate::core::ura::agent_ura("test-realm", "alice", "assistant");
        let host_ura = device_owner_ura();
        // Advertised with a host linkage, but the host is NOT in presence.
        advertised.upsert(AdvertisedAgentRecord {
            agent_ura: agent_ura.clone(),
            generation: 1,
            public_key_hex: "00".to_string(),
            host_node_id: Some("node-1".to_string()),
            signing_authority: AdvertisedAgentSigningAuthority::HostedBy {
                host_ura: host_ura.clone(),
            },
        });

        let failure = DaemonRouteResolver::new(&registry, Some(&advertised), &catalog)
            .at(TEST_NOW_MS)
            .resolve_route(&agent_ura, "agent.list")
            .expect_err("advertised-but-offline owner must resolve negative");

        assert_eq!(failure.reason, NegativeReason::Noroute);
        assert_eq!(failure.query_name, format!("{agent_ura}#agent.list"));
    }

    #[test]
    fn owner_online_without_ability_resolves_nodata() {
        let registry = PresenceRegistry::new();
        let catalog = AbilityCatalogStore::new();
        let host_device_ura = device_owner_ura();
        let owner_ura = system_agent_owner_ura_for("agent.list");
        mark_online(&registry, &host_device_ura);
        // Owner publishes `agent.list` but the request asks for `fs.read`.
        publish_ability(&catalog, &owner_ura, &host_device_ura, "agent", "list");

        let failure = DaemonRouteResolver::new(&registry, None, &catalog)
            .at(TEST_NOW_MS)
            .resolve_route(&owner_ura, "fs.read")
            .expect_err("online owner missing the ability must resolve negative");

        assert_eq!(failure.reason, NegativeReason::Nodata);
        assert_eq!(failure.query_name, format!("{owner_ura}#fs.read"));
    }

    // ---- RFC-005 §4 / D105: device-local namespace authority ----

    #[test]
    fn system_agent_control_ability_resolves_via_local_authority() {
        // The device execution host proves its SystemAgent-owned control-plane
        // ability from the live runtime table, not from a hub projection row.
        let registry = PresenceRegistry::new();
        let catalog = AbilityCatalogStore::new();
        let host_device_ura = device_owner_ura();
        let owner_ura = system_agent_owner_ura_for("agent.start");
        mark_online(&registry, &host_device_ura);
        let authority = FakeLocalRuntimeAuthority::with_owner_keys(&owner_ura, &["agent.start"]);

        let route = DaemonRouteResolver::new(&registry, None, &catalog)
            .with_local_catalog_authority(host_device_ura.clone(), authority)
            .at(TEST_NOW_MS)
            .resolve_route(&owner_ura, "agent.start")
            .expect("SystemAgent ability must resolve from local authority");

        let ability_ura =
            crate::core::ura::owner_ability_ura(&owner_ura, "agent.start").expect("ability ura");
        assert_eq!(route.kind(), SelectedRouteKind::RoutableAgentOwned);
        assert_eq!(route.owner_ura, owner_ura);
        assert_eq!(route.callee_ura, owner_ura);
        assert_eq!(route.execution_host_ura, host_device_ura);
        assert_eq!(route.ability_ura, ability_ura);
        assert_eq!(route.route_ura, format!("route-ref::{ability_ura}"));
        assert_eq!(route.dispatch_name, "agent.start");
        assert!(route.is_authoritative_local_or_better());
        // Catalog stays empty — authority did not come from a projection.
        assert!(catalog.is_empty());
    }

    #[test]
    fn system_agent_ability_not_registered_in_runtime_resolves_negative() {
        let registry = PresenceRegistry::new();
        let catalog = AbilityCatalogStore::new();
        let host_device_ura = device_owner_ura();
        let owner_ura = system_agent_owner_ura_for("agent.start");
        mark_online(&registry, &host_device_ura);
        let authority = FakeLocalRuntimeAuthority::with_owner_keys(&owner_ura, &["agent.start"]);

        let failure = DaemonRouteResolver::new(&registry, None, &catalog)
            .with_local_catalog_authority(host_device_ura, authority)
            .at(TEST_NOW_MS)
            .resolve_route(&owner_ura, "agent.stop")
            .expect_err("unregistered SystemAgent ability must resolve negative");

        assert_eq!(failure.reason, NegativeReason::Nxdomain);
        assert_eq!(failure.query_name, format!("{owner_ura}#agent.stop"));
    }

    #[test]
    fn system_agent_local_authority_resolves_without_presence_projection() {
        let registry = PresenceRegistry::new();
        let catalog = AbilityCatalogStore::new();
        let host_device_ura = device_owner_ura();
        let owner_ura = system_agent_owner_ura_for("agent.start");
        let authority = FakeLocalRuntimeAuthority::with_owner_keys(&owner_ura, &["agent.start"]);

        let route = DaemonRouteResolver::new(&registry, None, &catalog)
            .with_local_catalog_authority(host_device_ura.clone(), authority)
            .at(TEST_NOW_MS)
            .resolve_route(&owner_ura, "agent.start")
            .expect("local SystemAgent authority must not depend on projection presence");

        assert_eq!(route.kind(), SelectedRouteKind::RoutableAgentOwned);
        assert_eq!(route.owner_ura, owner_ura);
        assert_eq!(route.execution_host_ura, host_device_ura);
    }

    #[test]
    fn hosted_agent_implemented_on_this_device_is_authoritative_local() {
        // A hosted agent's ability implementation lives on THIS device
        // (D44/D105). The owner is the Agent URA, while the execution
        // host is the device that owns the local runtime. The route must
        // be proven from aggregate placement + runtime bindings,
        // not from presence or hub ability projection rows.
        let registry = PresenceRegistry::new();
        let catalog = AbilityCatalogStore::new();
        let host_ura = device_owner_ura();
        let agent_ura = crate::core::ura::agent_ura("test-realm", "alice", "assistant");
        let authority = FakeLocalRuntimeAuthority::with_owner_keys(&agent_ura, &["chat"]);

        let route = DaemonRouteResolver::new(&registry, None, &catalog)
            .with_local_catalog_authority(host_ura.clone(), authority)
            .with_local_hosted_agent_placements(LocalHostedAgentPlacements::single(
                agent_ura.clone(),
                host_ura.clone(),
            ))
            .at(TEST_NOW_MS)
            .resolve_route(&agent_ura, "chat")
            .expect("hosted agent on this device must resolve a route");

        assert_eq!(route.kind(), SelectedRouteKind::RoutableAgentOwned);
        assert_eq!(route.route_reason(), RouteReason::HostedAgent);
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
    fn hosted_agent_placements_consume_aggregate_projection() {
        let agent_ura = crate::core::ura::agent_ura("test-realm", "alice", "assistant");
        let host_ura = device_owner_ura();
        let placements =
            LocalHostedAgentPlacements::from_projection(AgentHostedPlacementProjection {
                by_agent_ura: [(
                    agent_ura.clone(),
                    AgentHostedPlacement {
                        agent_ura: agent_ura.clone(),
                        host_device_ura: host_ura.clone(),
                        host_node_id: Some("device-a".to_string()),
                    },
                )]
                .into(),
            });

        let placement = placements
            .local_host_for(&agent_ura, &host_ura)
            .expect("aggregate placement should match local authority");

        assert_eq!(placement.host_device_ura, host_ura);
        assert_eq!(placement.host_node_id.as_deref(), Some("device-a"));
    }

    #[test]
    fn hosted_agent_placements_unavailable_fails_closed() {
        let agent_ura = crate::core::ura::agent_ura("test-realm", "alice", "assistant");
        let host_ura = device_owner_ura();
        let placements = LocalHostedAgentPlacements::unavailable(
            "load hosted-Agent identity projection: denied".to_string(),
        );

        assert!(
            placements.local_host_for(&agent_ura, &host_ura).is_none(),
            "unavailable aggregate projection must not prove local hosted placement"
        );
        assert!(matches!(
            placements.state,
            HostedPlacementProjectionState::Unavailable { .. }
        ));
    }

    #[test]
    fn hosted_agent_runtime_binding_can_route_without_local_agent_placement() {
        let registry = PresenceRegistry::new();
        let catalog = AbilityCatalogStore::new();
        let host_ura = device_owner_ura();
        let agent_ura = crate::core::ura::agent_ura("test-realm", "dev", "pages");
        let authority = FakeLocalRuntimeAuthority::with_owner_keys(&agent_ura, &["project_list"]);

        let route = DaemonRouteResolver::new(&registry, None, &catalog)
            .with_local_catalog_authority(host_ura.clone(), authority)
            .at(TEST_NOW_MS)
            .resolve_route(&agent_ura, "project_list")
            .expect("hosted agent registry key must resolve through owner-local public name");

        assert_eq!(route.kind(), SelectedRouteKind::RoutableAgentOwned);
        assert_eq!(route.execution_host_ura, host_ura);
        assert_eq!(
            route.ability_ura,
            "easynet:///r/test-realm/ability/dev.pages.project_list"
        );
        assert_eq!(route.dispatch_name, "pages.project_list");
        assert_eq!(route.query_name, format!("{agent_ura}#project_list"));
    }

    #[test]
    fn hub_local_authority_can_route_builtin_pages_agent_without_projection() {
        let registry = PresenceRegistry::new();
        let catalog = AbilityCatalogStore::new();
        let host_ura = crate::core::ura::hub_ura("test-realm");
        let agent_ura = crate::core::ura::agent_ura("test-realm", "dev", "pages");
        let authority = FakeLocalRuntimeAuthority::with_owner_keys(&agent_ura, &["project_list"]);

        let route = DaemonRouteResolver::new(&registry, None, &catalog)
            .with_local_catalog_authority(host_ura.clone(), authority)
            .at(TEST_NOW_MS)
            .resolve_route(&agent_ura, "project_list")
            .expect("hub-mode local runtime authority must route built-in pages agent");

        assert_eq!(route.kind(), SelectedRouteKind::RoutableAgentOwned);
        assert_eq!(route.execution_host_ura, host_ura);
        assert_eq!(
            route.ability_ura,
            "easynet:///r/test-realm/ability/dev.pages.project_list"
        );
        assert_eq!(route.dispatch_name, "pages.project_list");
        assert_eq!(route.query_name, format!("{agent_ura}#project_list"));
        assert!(catalog.is_empty());
    }

    #[test]
    fn realm_authority_resolves_authority_owned_ability_as_local_authority_route() {
        let registry = PresenceRegistry::new();
        let catalog = AbilityCatalogStore::new();
        let host_ura = crate::core::ura::hub_ura("test-realm");
        let authority =
            FakeLocalRuntimeAuthority::with_owner_keys(&host_ura, &["federation.status"]);

        let route = DaemonRouteResolver::new(&registry, None, &catalog)
            .with_local_catalog_authority(host_ura.clone(), authority)
            .at(TEST_NOW_MS)
            .resolve_route(&host_ura, "federation.status")
            .expect("authority-owned runtime ability must resolve through local realm authority");

        assert_eq!(route.kind(), SelectedRouteKind::RealmAuthorityOwned);
        assert_eq!(route.route_reason(), RouteReason::LocalHub);
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
    fn combined_local_authority_resolves_authority_owned_ability_from_catalog_snapshot() {
        let registry = PresenceRegistry::new();
        let catalog = AbilityCatalogStore::new();
        let device_ura = crate::core::ura::device_ura("test-realm", "device-a");
        let hub_ura = crate::core::ura::hub_ura("test-realm");
        let authority = FakeLocalRuntimeAuthority::with_owner_keys(
            &hub_ura,
            &["runtime.bootstrap_self_identity"],
        );

        let route = DaemonRouteResolver::new(&registry, None, &catalog)
            .with_local_catalog_authority(device_ura, authority)
            .at(TEST_NOW_MS)
            .resolve_route(&hub_ura, "runtime.bootstrap_self_identity")
            .expect("combined local authority must route Authority runtime-admin ability");

        assert_eq!(route.kind(), SelectedRouteKind::RealmAuthorityOwned);
        assert_eq!(
            route.dispatch_target(true),
            SelectedRouteDispatchTarget::LocalRuntime
        );
        assert_eq!(route.owner_ura, hub_ura);
        assert_eq!(route.callee_ura, route.owner_ura);
        assert_eq!(route.execution_host_ura, route.owner_ura);
        assert_eq!(route.dispatch_name, "runtime.bootstrap_self_identity");
    }

    #[test]
    fn hosted_agent_descriptor_ref_resolves_owner_local_public_name() {
        let registry = PresenceRegistry::new();
        let catalog = AbilityCatalogStore::new();
        let host_ura = device_owner_ura();
        let agent_ura = crate::core::ura::agent_ura("test-realm", "dev", "pages");
        let ability_ura = crate::core::ura::owner_ability_ura(&agent_ura, "project_list")
            .expect("agent ability ura");
        let descriptor_ref = descriptor_ref_for_test(&ability_ura);
        let authority = FakeLocalRuntimeAuthority::with_owner_keys(&agent_ura, &["project_list"]);

        let route = DaemonRouteResolver::new(&registry, None, &catalog)
            .with_local_catalog_authority(host_ura.clone(), authority)
            .at(TEST_NOW_MS)
            .resolve_route(&agent_ura, &descriptor_ref)
            .expect("descriptor-bound hosted-agent ability must resolve through local authority");

        assert_eq!(route.kind(), SelectedRouteKind::RoutableAgentOwned);
        assert_eq!(route.owner_ura, agent_ura);
        assert_eq!(route.callee_ura, agent_ura);
        assert_eq!(route.execution_host_ura, host_ura);
        assert_eq!(route.ability_ura, ability_ura);
        assert_eq!(route.dispatch_name, "pages.project_list");
        assert_eq!(route.query_name, format!("{agent_ura}#project_list"));
    }

    #[test]
    fn authority_owned_catalog_projection_without_local_authority_fails_closed() {
        let registry = PresenceRegistry::new();
        let catalog = AbilityCatalogStore::new();
        let hub_ura = crate::core::ura::hub_ura("test-realm");
        publish_ability(&catalog, &hub_ura, &hub_ura, "federation", "status");

        let error = DaemonRouteResolver::new(&registry, None, &catalog)
            .at(TEST_NOW_MS)
            .resolve_route(&hub_ura, "federation.status")
            .expect_err("authority-owned catalog projection must not invent authority presence");

        assert_eq!(error.reason, NegativeReason::Nxdomain);
        assert_eq!(error.detail, "owner is not online");
    }

    #[test]
    fn projection_route_for_present_device_owner_is_not_dispatchable() {
        // A DeviceProfileProjection row may remain as a bounded migration
        // read-model, but it must not become an ordinary public route selected
        // by a hub or another daemon. Device is host/custody/sponsor only; a
        // dispatchable public descriptor must be owned by a SystemAgent,
        // hosted Agent, or Authority.
        let registry = PresenceRegistry::new();
        let catalog = AbilityCatalogStore::new();
        let owner_ura = device_owner_ura();
        mark_online(&registry, &owner_ura);
        publish_ability(&catalog, &owner_ura, &owner_ura, "agent", "list");

        let failure = DaemonRouteResolver::new(&registry, None, &catalog)
            .at(TEST_NOW_MS)
            .resolve_route(&owner_ura, "agent.list")
            .expect_err("direct Device-owned projections must not dispatch");

        assert_eq!(failure.reason, NegativeReason::Refused);
        assert!(
            failure.detail.contains("Device-owned ability selectors"),
            "unexpected detail: {}",
            failure.detail
        );
    }

    #[test]
    fn ability_without_route_summary_resolves_noroute() {
        let registry = PresenceRegistry::new();
        let catalog = AbilityCatalogStore::new();
        let host_device_ura = device_owner_ura();
        let owner_ura = system_agent_owner_ura_for("agent.list");
        mark_online(&registry, &host_device_ura);
        let ability_ura = publish_ability_with_route_summary(
            &catalog,
            &owner_ura,
            &host_device_ura,
            "agent",
            "list",
            false,
        );

        let failure = DaemonRouteResolver::new(&registry, None, &catalog)
            .at(TEST_NOW_MS)
            .resolve_route(&owner_ura, "agent.list")
            .expect_err("ability without executable route must not be dispatchable");

        assert_eq!(failure.reason, NegativeReason::Noroute);
        assert_eq!(failure.query_name, format!("{owner_ura}#agent.list"));
        assert!(failure.detail.contains("route_summary_ref"));

        let answer = DaemonRouteResolver::new(&registry, None, &catalog)
            .at(TEST_NOW_MS)
            .resolve_query_json(&json!({
                "qtype": ResolveType::DirectoryListing.as_str_name(),
                "query_name": owner_ura,
            }));
        let records = answer["records"].as_array().expect("records array");
        assert!(
            records.iter().any(|record| {
                record["record_type"] == RecordType::Ability.as_str_name()
                    && record["value"]["ability"]["ability_ura"] == ability_ura.as_str()
            }),
            "directory listing should retain the non-dispatchable ability fact"
        );
        assert!(
            records.iter().all(|record| {
                record["record_type"] != RecordType::Route.as_str_name()
                    || record["value"]["route"]["ability_ura"] != ability_ura.as_str()
            }),
            "directory listing must not manufacture a ROUTE record without route_summary_ref"
        );
    }

    #[test]
    fn directory_listing_rejects_incomplete_ability_summary_before_empty_descriptor_default() {
        let registry = PresenceRegistry::new();
        let catalog = AbilityCatalogStore::new();
        let owner_ura = device_owner_ura();
        mark_online(&registry, &owner_ura);
        publish_ability_with_descriptor_revision(&catalog, &owner_ura, &owner_ura, "");

        let answer = DaemonRouteResolver::new(&registry, None, &catalog)
            .at(TEST_NOW_MS)
            .resolve_query_json(&json!({
                "qtype": ResolveType::DirectoryListing.as_str_name(),
                "query_name": owner_ura,
            }));

        assert_eq!(
            answer["answer_kind"],
            ResolveAnswerKind::Negative.as_str_name()
        );
        assert_eq!(
            answer["negative"]["reason"],
            NegativeReason::Refused.as_str_name()
        );
        assert!(
            answer["negative"]["detail"]
                .as_str()
                .is_some_and(|detail| detail.contains("descriptor_revision")),
            "wrong negative answer: {answer}"
        );
    }

    #[test]
    fn hosted_agent_without_host_placement_resolves_noroute() {
        let registry = PresenceRegistry::new();
        let catalog = AbilityCatalogStore::new();
        let agent_ura = crate::core::ura::agent_ura("test-realm", "alice", "assistant");
        mark_online(&registry, &agent_ura);
        publish_ability(&catalog, &agent_ura, &agent_ura, "chat", "complete");

        let failure = DaemonRouteResolver::new(&registry, None, &catalog)
            .at(TEST_NOW_MS)
            .resolve_route(&agent_ura, "chat.complete")
            .expect_err("hosted agent route must require selected host placement");

        assert_eq!(failure.reason, NegativeReason::Noroute);
        assert_eq!(failure.query_name, format!("{agent_ura}#chat.complete"));
        assert!(failure.detail.contains("host device"));
    }

    #[test]
    fn resolve_query_json_ignores_retired_camel_case_input_aliases() {
        let registry = PresenceRegistry::new();
        let catalog = AbilityCatalogStore::new();
        let owner_ura = device_owner_ura();
        mark_online(&registry, &owner_ura);
        publish_ability(&catalog, &owner_ura, &owner_ura, "agent", "list");

        let answer = DaemonRouteResolver::new(&registry, None, &catalog)
            .at(TEST_NOW_MS)
            .resolve_query_json(&json!({
                "qtype": ResolveType::Route.as_str_name(),
                "queryName": owner_ura,
                "abilityName": "agent.list",
                "realmHint": "example",
            }));

        assert_eq!(
            answer["answer_kind"],
            ResolveAnswerKind::Negative.as_str_name()
        );
        assert_eq!(answer["negative"]["query_name"], "");
        assert!(answer.get("ability_ura").is_none());
    }

    #[test]
    fn resolve_query_json_rejects_missing_qtype_instead_of_shape_guessing() {
        let registry = PresenceRegistry::new();
        let catalog = AbilityCatalogStore::new();
        let owner_ura = device_owner_ura();
        mark_online(&registry, &owner_ura);
        publish_ability(&catalog, &owner_ura, &owner_ura, "agent", "list");

        let answer = DaemonRouteResolver::new(&registry, None, &catalog)
            .at(TEST_NOW_MS)
            .resolve_query_json(&json!({
                "query_name": owner_ura,
                "ability_name": "agent.list",
            }));

        assert_eq!(
            answer["answer_kind"],
            ResolveAnswerKind::Negative.as_str_name()
        );
        assert_eq!(
            answer["negative"]["reason"],
            NegativeReason::Refused.as_str_name()
        );
        assert!(
            answer["negative"]["detail"]
                .as_str()
                .is_some_and(|detail| detail.contains("missing canonical qtype")),
            "missing qtype must fail before route/directory guessing: {answer}"
        );
    }

    #[test]
    fn resolve_query_json_rejects_missing_query_name_before_empty_selector_default() {
        let registry = PresenceRegistry::new();
        let catalog = AbilityCatalogStore::new();
        let owner_ura = device_owner_ura();
        mark_online(&registry, &owner_ura);
        publish_ability(&catalog, &owner_ura, &owner_ura, "agent", "list");

        let answer = DaemonRouteResolver::new(&registry, None, &catalog)
            .at(TEST_NOW_MS)
            .resolve_query_json(&json!({
                "qtype": ResolveType::Route.as_str_name(),
                "ability_name": "agent.list",
            }));

        assert_eq!(
            answer["answer_kind"],
            ResolveAnswerKind::Negative.as_str_name()
        );
        assert_eq!(
            answer["negative"]["reason"],
            NegativeReason::Refused.as_str_name()
        );
        assert!(
            answer["negative"]["detail"]
                .as_str()
                .is_some_and(|detail| detail.contains("missing canonical query_name")),
            "missing query_name must fail before empty-selector route lookup: {answer}"
        );
        assert_eq!(answer["negative"]["query_name"], "");
    }

    #[test]
    fn resolve_query_json_rejects_short_qtype_aliases() {
        let registry = PresenceRegistry::new();
        let catalog = AbilityCatalogStore::new();
        let owner_ura = device_owner_ura();
        mark_online(&registry, &owner_ura);
        publish_ability(&catalog, &owner_ura, &owner_ura, "agent", "list");

        for qtype in [json!("ROUTE"), json!("route"), json!(2)] {
            let answer = DaemonRouteResolver::new(&registry, None, &catalog)
                .at(TEST_NOW_MS)
                .resolve_query_json(&json!({
                    "qtype": qtype,
                    "query_name": owner_ura,
                    "ability_name": "agent.list",
                }));

            assert_eq!(
                answer["answer_kind"],
                ResolveAnswerKind::Negative.as_str_name(),
                "short/numeric qtype aliases must not resolve: {answer}"
            );
            assert_eq!(
                answer["negative"]["reason"],
                NegativeReason::Refused.as_str_name()
            );
            assert!(
                answer["negative"]["detail"]
                    .as_str()
                    .is_some_and(|detail| detail.contains("canonical ResolveType enum string")),
                "rejection must name canonical qtype requirement: {answer}"
            );
        }
    }

    #[test]
    fn malformed_query_resolves_refused() {
        let registry = PresenceRegistry::new();
        let catalog = AbilityCatalogStore::new();

        // Empty owner + empty ability is not a valid route selector.
        let failure = DaemonRouteResolver::new(&registry, None, &catalog)
            .at(TEST_NOW_MS)
            .resolve_route("", "")
            .expect_err("empty selector must be refused");

        assert_eq!(failure.reason, NegativeReason::Refused);
    }

    #[test]
    fn authority_projection_uses_route_ref_embedded_ability_realm() {
        let owner_ura = device_owner_ura();
        let ability_ura =
            crate::core::ura::owner_ability_ura(&owner_ura, "agent.list").expect("ability ura");
        let authority = authority_for_query(&format!("route-ref::{ability_ura}"));

        assert_eq!(
            authority["authority_ura"],
            crate::core::ura::hub_ura("test-realm")
        );
        assert_eq!(authority["zone_ref"], "realm:test-realm");
        assert!(authority.get("unavailable").is_none());
    }

    #[test]
    fn authority_projection_uses_descriptor_ref_embedded_ability_realm() {
        let owner_ura = device_owner_ura();
        let ability_ura =
            crate::core::ura::owner_ability_ura(&owner_ura, "agent.list").expect("ability ura");
        let descriptor_ref = descriptor_ref_for_test(&ability_ura);
        let authority = authority_for_query(&descriptor_ref);

        assert_eq!(
            authority["authority_ura"],
            crate::core::ura::hub_ura("test-realm")
        );
        assert_eq!(authority["zone_ref"], "realm:test-realm");
        assert!(authority.get("unavailable").is_none());
    }

    #[test]
    fn authority_projection_does_not_default_invalid_query_to_localhost() {
        let authority = authority_for_query("not-a-ura");

        assert_eq!(authority["authority_ura"], "");
        assert_eq!(authority["zone_ref"], "query_name_unavailable");
        assert_eq!(authority["algorithm"], "daemon-local-unavailable");
        assert!(authority["unavailable"]["reason"]
            .as_str()
            .is_some_and(|reason| reason.contains("query_name")));
    }

    #[test]
    fn final_route_answer_json_shape_carries_required_keys() {
        let registry = PresenceRegistry::new();
        let catalog = AbilityCatalogStore::new();
        let host_device_ura = device_owner_ura();
        let owner_ura = system_agent_owner_ura_for("agent.list");
        mark_online(&registry, &host_device_ura);
        let ability_ura =
            crate::core::ura::owner_ability_ura(&owner_ura, "agent.list").expect("ability ura");
        let authority = FakeLocalRuntimeAuthority::with_owner_keys(&owner_ura, &["agent.list"]);

        let answer = DaemonRouteResolver::new(&registry, None, &catalog)
            .with_local_catalog_authority(host_device_ura, authority)
            .at(TEST_NOW_MS)
            .resolve_route(&owner_ura, "agent.list")
            .expect("route resolves")
            .final_route_answer_json();

        assert_eq!(
            answer["answer_kind"],
            ResolveAnswerKind::FinalRoute.as_str_name()
        );
        assert_eq!(answer["ability_ura"], ability_ura);
        assert_eq!(answer["owner_ura"], owner_ura);
        assert_eq!(answer["route_ura"], format!("route-ref::{ability_ura}"));
        assert_eq!(
            answer["release_profile"],
            ResolverReleaseProfile::AuthoritativeLocal.as_str_name()
        );
        // Required nested objects are present.
        assert!(answer.get("next_hop").is_some());
        assert!(answer["next_hop"]["hosted_agent_via_device"].is_object());
        assert!(answer.get("selected_route").is_some());
        assert!(answer["selected_route"]["next_hop"].is_object());
    }

    #[test]
    fn remote_owner_resolves_peer_hub_delegation_from_static_peer_map() {
        let registry = PresenceRegistry::new();
        let peers = SharedFederatedPeers::new(BTreeMap::from([(
            "remote-realm".to_string(),
            "https://remote-hub.example".to_string(),
        )]));
        let remote_owner = crate::core::ura::device_ura("remote-realm", "remote-device");
        let ability_ura = crate::core::ura::owner_ability_ura(&remote_owner, "node.describe")
            .expect("ability ura");

        let catalog = AbilityCatalogStore::new();
        let delegation = DaemonRouteResolver::new(&registry, None, &catalog)
            .with_peer_delegation("local-realm", &peers)
            .resolve_delegation(&ability_ura, "")
            .expect("delegation lookup succeeds")
            .expect("remote owner delegates to peer hub");

        assert_eq!(delegation.query_name, ability_ura);
        assert_eq!(delegation.owner_ura, remote_owner);
        assert_eq!(delegation.realm, "remote-realm");
        assert_eq!(
            delegation.hub_ura,
            crate::core::ura::hub_ura("remote-realm")
        );
        assert_eq!(
            delegation.primary_endpoint(),
            Some("https://remote-hub.example")
        );

        let answer = delegation.delegation_answer_json();
        assert_eq!(
            answer["answer_kind"],
            ResolveAnswerKind::Delegation.as_str_name()
        );
        assert_eq!(
            answer["release_profile"],
            ResolverReleaseProfile::AuthoritativeLocal.as_str_name()
        );
        assert_eq!(answer["next_hop"]["peer_hub"]["realm"], "remote-realm");
        assert_eq!(
            answer["next_hop"]["peer_hub"]["endpoints"][0]["endpoint"],
            "https://remote-hub.example"
        );
        assert_eq!(
            answer["next_hop"]["peer_hub"]["endpoints"][0]["metadata"]["source"],
            "federated_peers"
        );
        assert_eq!(
            answer["selected_route"]["reason"],
            RouteReason::PeerDelegation.as_str_name()
        );
        assert_eq!(
            answer["route_evidence"]["selection_algorithm"],
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
        let advertised = AdvertisedAgentStore::new();
        let catalog = AbilityCatalogStore::new();
        let host_device_ura = device_owner_ura();
        let owner_ura = crate::core::ura::device_agent_ura(
            "test-realm",
            "test-daemon",
            crate::daemon::ability::names::governance::RUNTIME_INTROSPECTION_SYSTEM_AGENT_ID,
        );
        mark_online(&registry, &host_device_ura);
        advertise_hosted_agent(&advertised, &owner_ura, &host_device_ura);
        let ability_ura = publish_ability(&catalog, &owner_ura, &host_device_ura, "meta", "list");

        let resolver =
            DaemonRouteResolver::new(&registry, Some(&advertised), &catalog).at(TEST_NOW_MS);
        let answer = resolver.resolve_query_json(&json!({
            "qtype": ResolveType::DirectoryListing.as_str_name(),
            "query_name": owner_ura,
        }));

        let records = answer["records"].as_array().expect("records array");
        let route = records
            .iter()
            .find(|record| {
                record["record_type"] == RecordType::Route.as_str_name()
                    && record["value"]["route"]["ability_ura"] == ability_ura.as_str()
            })
            .expect("directory listing must carry a ROUTE record for the published ability");

        let route_value = &route["value"]["route"];
        assert_eq!(
            route_value["route_ura"],
            format!("route-ref::{ability_ura}")
        );
        assert_eq!(route_value["dispatch_name"], "meta.list");
        assert_eq!(route_value["execute_on"]["target_ura"], host_device_ura);

        // The directory route must equal what the single-route resolve
        // selects — one selection path, no divergence.
        let selected = resolver
            .resolve_route(&owner_ura, "meta.list")
            .expect("single-route resolve must succeed");
        assert_eq!(route_value["ability_ura"], selected.ability_ura.as_str());
        assert_eq!(route_value["route_ura"], selected.route_ura.as_str());
        assert_eq!(
            route_value["dispatch_name"],
            selected.dispatch_name.as_str()
        );
    }

    #[test]
    fn directory_listing_records_carry_canonical_kind() {
        let registry = PresenceRegistry::new();
        let catalog = AbilityCatalogStore::new();
        let owner_ura = device_owner_ura();
        mark_online(&registry, &owner_ura);
        publish_ability(&catalog, &owner_ura, &owner_ura, "agent", "list");

        let answer = DaemonRouteResolver::new(&registry, None, &catalog)
            .at(TEST_NOW_MS)
            .resolve_query_json(&json!({
                "qtype": ResolveType::DirectoryListing.as_str_name(),
                "query_name": owner_ura,
            }));

        let records = answer["records"].as_array().expect("records array");
        assert!(
            !records.is_empty(),
            "directory listing must expose typed records"
        );
        for record in records {
            let kind = record["kind"].as_str().expect("directory record kind");
            let record_type = record["record_type"]
                .as_str()
                .expect("directory record record_type");
            assert!(
                !kind.trim().is_empty(),
                "directory record kind must be a public read-model kind"
            );
            assert!(
                RecordType::from_str_name(record_type).is_some(),
                "directory record_type must remain a canonical RecordType"
            );
            assert!(
                record.get("ura").is_some()
                    || record.get("owner_ura").is_some()
                    || record.get("ability_ura").is_some()
                    || record.get("route_ura").is_some(),
                "directory record must carry at least one top-level canonical URA fact: {record:#?}"
            );
        }
        let device = records
            .iter()
            .find(|record| record["kind"] == "device")
            .expect("device directory listing must expose a device record");
        assert_eq!(device["ura"], owner_ura);
        assert_eq!(device["agent_ura"], owner_ura);
        assert_eq!(device["node_id"], "test-daemon");
        assert_eq!(device["status"], "active");
    }

    #[test]
    fn directory_listing_can_omit_ability_projection_for_presence_reads() {
        let registry = PresenceRegistry::new();
        let catalog = AbilityCatalogStore::new();
        let owner_a = device_owner_ura();
        let owner_b = "easynet:///r/test-realm/device/dev-b";
        mark_online(&registry, &owner_a);
        mark_online(&registry, owner_b);
        publish_ability(&catalog, &owner_a, &owner_a, "agent", "list");
        publish_ability(&catalog, owner_b, owner_b, "fs", "read");

        let answer = DaemonRouteResolver::new(&registry, None, &catalog)
            .at(TEST_NOW_MS)
            .resolve_query_json(&json!({
                "qtype": ResolveType::DirectoryListing.as_str_name(),
                "query_name": "easynet:///r/test-realm/device/",
                "include_abilities": false,
            }));

        let records = answer["records"].as_array().expect("records array");
        let record_types: Vec<&str> = records
            .iter()
            .filter_map(|record| record["record_type"].as_str())
            .collect();
        assert_eq!(
            record_types,
            vec![RecordType::Id.as_str_name(), RecordType::Id.as_str_name()],
            "presence-only listing must not let ability records page device IDs apart"
        );
        let names: Vec<&str> = records
            .iter()
            .filter_map(|record| record["name"].as_str())
            .collect();
        assert!(names.contains(&owner_a.as_str()));
        assert!(names.contains(&owner_b));
    }

    #[test]
    fn directory_listing_includes_federated_directory_device_identity_without_routes() {
        let registry = PresenceRegistry::new();
        let catalog = AbilityCatalogStore::new();
        let peer_device_ura = "easynet:///r/peer-realm/device/peer-device";
        let mut peer_view = DirectoryView::new("peer-realm".to_string());
        peer_view.replace_entries(vec![DirectoryEntry {
            agent_ura: peer_device_ura.to_string(),
            node_id: "peer-device".to_string(),
            display_name: Some("Peer Device".to_string()),
            status: "active".to_string(),
            origin_realm: None,
            hub_endpoint: Some("https://peer.example".to_string()),
            last_seen_unix_ms: Some(TEST_NOW_MS),
        }]);
        let federated_directory = SharedFederatedDirectoryView::new(BTreeMap::from([(
            "peer-realm".to_string(),
            Arc::new(peer_view),
        )]));

        let answer = DaemonRouteResolver::new(&registry, None, &catalog)
            .with_federated_directory(&federated_directory)
            .at(TEST_NOW_MS)
            .resolve_query_json(&json!({
                "qtype": ResolveType::DirectoryListing.as_str_name(),
                "query_name": "easynet:///r/peer-realm/device/",
                "include_abilities": true,
            }));

        assert_eq!(
            answer["answer_kind"],
            ResolveAnswerKind::NonDispatchable.as_str_name()
        );
        let records = answer["records"].as_array().expect("records array");
        let device = records
            .iter()
            .find(|record| {
                record["record_type"] == RecordType::Id.as_str_name()
                    && record["ura"] == peer_device_ura
            })
            .expect("peer device ID record should be projected from federated directory");
        assert_eq!(device["kind"], "device");
        assert_eq!(device["agent_ura"], peer_device_ura);
        assert_eq!(device["node_id"], "peer-device");
        assert_eq!(device["origin_realm"], "peer-realm");
        assert_eq!(device["hub_endpoint"], "https://peer.example");
        assert!(
            records
                .iter()
                .all(|record| record["record_type"] != RecordType::Route.as_str_name()),
            "federated directory observations must not become executable route records: {records:#?}"
        );
    }

    #[test]
    fn directory_listing_returns_stable_cursor_and_resumes() {
        let registry = PresenceRegistry::new();
        let advertised = AdvertisedAgentStore::new();
        let catalog = AbilityCatalogStore::new();
        let host_device_ura = device_owner_ura();
        let owner_ura = crate::core::ura::device_agent_ura(
            "test-realm",
            "test-daemon",
            crate::daemon::ability::names::governance::RUNTIME_INTROSPECTION_SYSTEM_AGENT_ID,
        );
        mark_online(&registry, &host_device_ura);
        advertise_hosted_agent(&advertised, &owner_ura, &host_device_ura);
        publish_ability(&catalog, &owner_ura, &host_device_ura, "meta", "list");
        publish_ability(&catalog, &owner_ura, &host_device_ura, "meta", "inspect");

        let resolver =
            DaemonRouteResolver::new(&registry, Some(&advertised), &catalog).at(TEST_NOW_MS);
        let first = resolver.resolve_query_json(&json!({
            "qtype": ResolveType::DirectoryListing.as_str_name(),
            "query_name": owner_ura,
            "limit": 1,
        }));
        let first_records = first["records"].as_array().expect("first records");
        assert_eq!(first_records.len(), 1);
        let first_cursor = first["next_cursor"].as_str().expect("first cursor");

        let second = resolver.resolve_query_json(&json!({
            "qtype": ResolveType::DirectoryListing.as_str_name(),
            "query_name": owner_ura,
            "limit": 1,
            "cursor": first_cursor,
        }));
        let second_records = second["records"].as_array().expect("second records");
        assert_eq!(second_records.len(), 1);
        assert_ne!(
            directory_record_cursor_key(&first_records[0]),
            directory_record_cursor_key(&second_records[0])
        );
        assert!(second["next_cursor"].as_str().is_some());
    }

    #[test]
    fn directory_listing_rejects_cursor_outside_current_query() {
        let registry = PresenceRegistry::new();
        let catalog = AbilityCatalogStore::new();
        let owner_ura = device_owner_ura();
        mark_online(&registry, &owner_ura);
        publish_ability(&catalog, &owner_ura, &owner_ura, "agent", "list");

        let missing_cursor = directory_cursor_for(&json!({
            "record_type": RecordType::Route.as_str_name(),
            "name": "easynet:///r/test-realm/resource/missing-route",
        }))
        .expect("cursor");
        let answer = DaemonRouteResolver::new(&registry, None, &catalog)
            .at(TEST_NOW_MS)
            .resolve_query_json(&json!({
                "qtype": ResolveType::DirectoryListing.as_str_name(),
                "query_name": owner_ura,
                "cursor": missing_cursor,
            }));
        assert_eq!(
            answer["answer_kind"],
            ResolveAnswerKind::Negative.as_str_name()
        );
        assert!(
            answer["negative"]["detail"]
                .as_str()
                .unwrap_or_default()
                .contains("cursor does not match"),
            "answer = {answer:#?}"
        );
    }
}
