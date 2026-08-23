// EasyNet Daemon — RFC-005 Target Gate
// =====================================
//
// File: src/daemon/invocation/target_gate.rs
// Description: The resolve-first gate shared by all three Invocation
//              RPC surfaces (unary, server-stream, bidi). Owns the two
//              questions every dispatch path asks before any frame
//              moves:
//
//                1. "Which route does namespace.resolve select for this
//                   (target URA, ability)?" — `route_resolver()`
//                2. "Is this target URA *me* (this daemon, its hub
//                   identity, or an agent it hosts)?" —
//                   `matches_self_target_ura()`
//
//              Extracted from `DaemonInvocationService` (commit-plan-2
//              E2/E3 prerequisite): the gate is a pure consumer of the
//              dependency planes, so dispatcher modules can hold a
//              `TargetGate` instead of borrowing the whole service.
//
// Author: Silan Hu <silan.hu@u.nus.edu>
// Copyright (c) 2026 EasyNet. All rights reserved.

use axon_sdk::invocation::CallMode;
use serde_json::json;
use tonic::Status;

use crate::daemon::ability::catalog::publication::LocalAbilityPublicationSnapshot;
use crate::daemon::axon_bridge::proof_owner::descriptor_bound_canonical_bytes;
use crate::daemon::axon_bridge::wire_descriptor::descriptor_bound_from_wire_parts;
use crate::daemon::federation::resolver_contract::{NegativeReason, ResolveAnswerKind};
use crate::daemon::invocation::admission::admission_facade::AdmissionFacade;
use crate::daemon::invocation::admission::child_invocation_builder::{
    ChildInvocationAuthority, ChildInvocationBuildFailure, ChildInvocationBuildInput,
    ChildInvocationBuilder, ExternallySignedChildInvocation, SelectedChildRoute,
};
use crate::daemon::invocation::admission::decision::{SignatureDecisionReason, TraceStage};
use crate::daemon::invocation::bidi::session_wire::{RequestOutcome, SessionRequestError};
use crate::daemon::invocation::dispatch::deps::{
    DirectoryPlane, FederationDial, IdentityPlane, SessionPlane,
};
use crate::daemon::invocation::dispatch::federation_wrappers::ABILITY_NAMESPACE_RESOLVE;
use crate::daemon::invocation::routing::route_resolver::{
    CanonicalRouteSelection, DaemonRouteResolver, DelegatedInvokeRoute, ResolveRouteFailure,
    SelectedInvokeRoute,
};
use crate::daemon::persistence::agent_aggregate::{
    AgentAggregateRepository, AgentLocalTargetProjection, HostedAgentTarget,
};
use axon_sdk::pb::axon::v1::{Envelope, InvocationTarget};

const HUB_SESSION_ROUTE_CONVERGENCE_RETRY_DELAY: std::time::Duration =
    std::time::Duration::from_millis(150);
const HUB_SESSION_ROUTE_CONVERGENCE_RETRY_ATTEMPTS: usize = 20;
const HUB_ROUTE_CONVERGENCE_DETAIL_OWNER_MISSING_ABILITY: &str =
    "owner is online but does not publish the requested ability";

/// Resolve-first gate over the daemon's routing authorities. Cheap to
/// construct (every plane is `Arc`-shaped); the service builds one per
/// dispatch via `DaemonInvocationService::target_gate()`.
#[derive(Clone)]
pub(crate) struct TargetGate {
    admission: AdmissionFacade,
    directory: DirectoryPlane,
    federation: FederationDial,
    sessions: SessionPlane,
    identity: IdentityPlane,
    local_agent_targets: LocalAgentTargetIndex,
}

impl TargetGate {
    pub(crate) fn new(
        admission: AdmissionFacade,
        directory: DirectoryPlane,
        federation: FederationDial,
        sessions: SessionPlane,
        identity: IdentityPlane,
    ) -> Self {
        Self {
            admission,
            directory,
            federation,
            sessions,
            identity,
            local_agent_targets: LocalAgentTargetIndex::load(),
        }
    }

    /// Build the RFC-005 route resolver wired with every authority this
    /// daemon owns: local presence, hosted-agent placement, owner
    /// projection, optional peer delegation, and the daemon's live ability
    /// control-plane publication authority (RFC-005 §4 / D105).
    ///
    /// One immutable catalog snapshot proves both local dispatchability and
    /// directory publication, so hot registrations cannot diverge by path.
    pub(crate) async fn route_resolver(&self) -> DaemonRouteResolver<'_> {
        let mut resolver = DaemonRouteResolver::new(
            &self.directory.presence,
            Some(self.directory.advertised_agents.as_ref()),
            self.directory.ability_catalog.as_ref(),
        )
        .with_federated_directory(&self.directory.federated_directory);
        if let Some(local_realm) = self
            .identity
            .session_realm
            .as_deref()
            .filter(|realm| !realm.is_empty())
        {
            resolver = resolver.with_peer_delegation(local_realm, &self.federation.peers);
        }
        if let Some(catalog) = self.directory.local_ability_catalog.as_ref() {
            if let Some(local_authority_ura) = local_runtime_authority_ura(
                self.admission.daemon_ura(),
                self.identity.session_realm.as_deref(),
            ) {
                let snapshot = LocalAbilityPublicationSnapshot::capture(catalog);
                resolver = resolver.with_local_catalog_authority(local_authority_ura, snapshot);
            }
        }
        resolver
    }

    pub(crate) async fn resolve_canonical_route(
        &self,
        target_ura: &str,
        ability_ura: &str,
        call_mode: CallMode,
    ) -> Result<CanonicalRouteSelection, ResolveRouteFailure> {
        let local_result =
            self.route_resolver()
                .await
                .resolve_canonical_route(target_ura, ability_ura, call_mode);
        match local_result {
            Ok(selection) => Ok(selection),
            Err(local_failure) => {
                self.resolve_hub_session_route(target_ura, ability_ura, call_mode, local_failure)
                    .await
            }
        }
    }

    async fn resolve_hub_session_route(
        &self,
        target_ura: &str,
        ability_ura: &str,
        call_mode: CallMode,
        local_failure: ResolveRouteFailure,
    ) -> Result<CanonicalRouteSelection, ResolveRouteFailure> {
        let Some(escalation) = self.sessions.escalation.as_ref() else {
            return Err(local_failure);
        };
        // Device mode owns no peer dialer. Its authenticated upstream Hub
        // session is the route-provider boundary for both same-realm and
        // federated targets: the Hub applies its own peer topology and
        // returns either a same-realm FINAL_ROUTE or a cross-realm
        // DELEGATION. The latter is forwarding authority for the upstream
        // Hub, not an executable route for the Device.
        let args = serde_json::to_vec(&namespace_route_query(target_ura, ability_ura, call_mode))
            .map_err(|error| {
            ResolveRouteFailure::new(
                ability_ura,
                NegativeReason::Refused,
                format!("Hub route provider request encoding failed: {error}"),
            )
        })?;
        let mut attempt = 0usize;
        loop {
            let answer: serde_json::Value = match escalation
                .escalate(ABILITY_NAMESPACE_RESOLVE.to_string(), args.clone())
                .await
            {
                RequestOutcome::Ok { result_bytes } => serde_json::from_slice(&result_bytes)
                    .map_err(|error| {
                        ResolveRouteFailure::new(
                            ability_ura,
                            NegativeReason::Nodata,
                            format!("Hub route provider returned unreadable JSON: {error}"),
                        )
                    })?,
                RequestOutcome::Err { error } => {
                    return Err(session_request_error_route_failure(
                        ability_ura,
                        error,
                        local_failure,
                    ))
                }
            };
            match answer
                .get("answer_kind")
                .and_then(serde_json::Value::as_str)
                .and_then(ResolveAnswerKind::from_str_name)
            {
                Some(ResolveAnswerKind::FinalRoute) => {
                    let selected_route = SelectedInvokeRoute::from_hub_final_route_answer_json(
                        &answer,
                        target_ura,
                        ability_ura,
                    )?;
                    crate::op_event!(
                        component = daemon_invocation,
                        kind = hub_session_final_route_selected,
                        target_ura = target_ura,
                        ability = ability_ura,
                        route_ura = selected_route.route_ura.as_str(),
                        execution_host_ura = selected_route.execution_host_ura.as_str(),
                    );
                    return Ok(CanonicalRouteSelection::hub_session(
                        call_mode,
                        selected_route,
                    ));
                }
                Some(ResolveAnswerKind::Delegation) => {
                    let delegated_route = DelegatedInvokeRoute::from_hub_delegation_answer_json(
                        &answer,
                        target_ura,
                        ability_ura,
                    )?;
                    crate::op_event!(
                        component = daemon_invocation,
                        kind = upstream_hub_peer_delegation_selected,
                        target_ura = target_ura,
                        ability = ability_ura,
                        peer_realm = delegated_route.realm.as_str(),
                        peer_hub_ura = delegated_route.hub_ura.as_str(),
                    );
                    return Ok(CanonicalRouteSelection::upstream_hub(
                        call_mode,
                        delegated_route,
                    ));
                }
                Some(ResolveAnswerKind::Negative) => {
                    if hub_negative_route_is_dynamic_convergence_candidate(&answer)
                        && attempt < HUB_SESSION_ROUTE_CONVERGENCE_RETRY_ATTEMPTS
                    {
                        attempt += 1;
                        crate::op_event!(
                            component = daemon_invocation,
                            kind = hub_session_route_convergence_retry,
                            target_ura = target_ura,
                            ability = ability_ura,
                            attempt = attempt,
                            max_attempts = HUB_SESSION_ROUTE_CONVERGENCE_RETRY_ATTEMPTS,
                            message = "Hub namespace.resolve has the owner online but not the newly published ability; retrying within the bounded dynamic publication convergence window",
                        );
                        tokio::time::sleep(HUB_SESSION_ROUTE_CONVERGENCE_RETRY_DELAY).await;
                        continue;
                    }
                    return Err(hub_negative_route_failure(&answer, ability_ura));
                }
                _ => {
                    return Err(ResolveRouteFailure::new(
                        ability_ura,
                        NegativeReason::Noroute,
                        "Hub route provider returned a missing or unsupported answer_kind",
                    ))
                }
            }
        }
    }

    /// Resolve whether `target_ura` names THIS daemon's own
    /// synchronous-execution surface.
    ///
    /// Three valid shapes per RFC-001 + RFC-006-C v0.1:
    ///   (1) `easynet:///r/<realm>/device/<deviceID>` — the daemon's
    ///       device identity from credentials.json. Standard.
    ///   (2) `easynet:///r/<realm>/authority` — the product Hub projected onto
    ///       Axon's canonical Authority URA; hub-mode daemons answer to this in
    ///       addition to (1).
    ///   (3) `easynet:///r/<realm>/agent/<userID>.<agentID>` — the
    ///       agent URA of an agent the daemon currently hosts. v4.1.5
    ///       §9 callee ∈ {hub, device, agent}; RFC-006-C §INV-2 +
    ///       RFC-006-B v0.6 §URL require the wire callee on a chat-
    ///       base or page.fetch invocation to be the agent URA, not
    ///       the device. Recognise it here so the local fast path
    ///       fires instead of falling through to "target offline".
    ///
    /// Match for (3) uses the Agent aggregate projection, not just the bare
    /// `<agentID>`. A daemon treats an Agent URA as local only when the
    /// target is proven by that projection:
    ///   * the hosted-Agent target set contains the exact
    ///     `(realm,user,agent)` tuple; or
    ///   * the tuple matches this daemon's credential identity and the exact
    ///     bare `agentID` is registered in the aggregate projection.
    ///
    /// The predicate intentionally does not scan LocalRuntime ability
    /// names. Ability names prove dispatch bindings for already-selected
    /// owners; they do not prove agent ownership, and scanning them here
    /// made every route-locality check scale with the full ability table.
    pub(crate) async fn matches_self_target_ura(&self, target_ura: &str) -> bool {
        if self
            .admission
            .daemon_ura()
            .is_some_and(|daemon_ura| daemon_ura == target_ura)
        {
            return true;
        }
        if self
            .identity
            .session_realm
            .as_deref()
            .is_some_and(|realm| crate::core::ura::hub_ura(realm) == target_ura)
        {
            return true;
        }
        if crate::daemon::identity::local_invocation::local_device_ura()
            .ok()
            .is_some_and(|local_device_ura| local_device_ura == target_ura)
        {
            return true;
        }
        if let Some(agent_target) = HostedAgentTarget::parse(target_ura) {
            if self.local_agent_targets.is_local_target(&agent_target) {
                return true;
            }

            let identity_matches_credentials = self
                .local_agent_targets
                .credentials_match_target(&agent_target);
            let registered_agent_match = self
                .local_agent_targets
                .has_registered_agent_id(&agent_target.agent_id);
            if identity_matches_credentials && registered_agent_match {
                return true;
            }

            let credential_identity_miss = if identity_matches_credentials {
                "false"
            } else {
                "true"
            };
            let agent_registry_miss = if registered_agent_match {
                "false"
            } else {
                "true"
            };
            let local_agent_projection_state = self.local_agent_targets.projection_state_label();
            let local_agent_projection_error = self.local_agent_targets.projection_error();
            let local_credential_state = self.local_agent_targets.credential_state_label();
            let local_credential_error = self.local_agent_targets.credential_error();
            crate::op_event!(
                component = daemon_invocation,
                kind = self_target_miss_for_agent_ura,
                target_ura = target_ura,
                realm = agent_target.realm.as_str(),
                user_id = agent_target.user_id.as_str(),
                agent_id = agent_target.agent_id.as_str(),
                local_agents_miss = "true",
                credential_identity_miss = credential_identity_miss,
                agent_registry_miss = agent_registry_miss,
                local_agent_projection_state = local_agent_projection_state,
                local_agent_projection_error = local_agent_projection_error,
                local_credential_state = local_credential_state,
                local_credential_error = local_credential_error,
                message = "matches_self_target_ura: agent URA not local; \
                          no exact local hosted Agent identity matched. Call \
                          will fall through to PresenceRegistry lookup.",
            );
        }
        false
    }
}

fn namespace_route_query(
    target_ura: &str,
    ability_ura: &str,
    call_mode: CallMode,
) -> serde_json::Value {
    let ability = ability_ura.trim();
    if crate::core::ura::parse_ura(ability)
        .is_ok_and(|parsed| parsed.kind == crate::core::ura::URAKind::Ability)
    {
        json!({
            "query_name": ability,
            "ability_name": "",
            "call_mode": call_mode.as_str(),
            "qtype": "RESOLVE_TYPE_ROUTE",
            "include_abilities": true,
        })
    } else {
        json!({
            "query_name": target_ura,
            "ability_name": ability,
            "call_mode": call_mode.as_str(),
            "qtype": "RESOLVE_TYPE_ROUTE",
            "include_abilities": true,
        })
    }
}

fn hub_negative_route_failure(
    answer: &serde_json::Value,
    fallback_query: &str,
) -> ResolveRouteFailure {
    let negative = answer
        .get("negative")
        .and_then(serde_json::Value::as_object);
    let query_name = negative
        .and_then(|object| object.get("query_name"))
        .and_then(serde_json::Value::as_str)
        .map(str::trim)
        .filter(|value| !value.is_empty())
        .unwrap_or(fallback_query);
    let reason = negative
        .and_then(|object| object.get("reason"))
        .and_then(serde_json::Value::as_str)
        .and_then(NegativeReason::from_str_name)
        .unwrap_or(NegativeReason::Noroute);
    let detail = negative
        .and_then(|object| object.get("detail"))
        .and_then(serde_json::Value::as_str)
        .map(str::trim)
        .filter(|value| !value.is_empty())
        .unwrap_or("Hub route provider returned a negative namespace.resolve answer");
    if detail == "owner is not online"
        && matches!(reason, NegativeReason::Nxdomain | NegativeReason::Noroute)
    {
        ResolveRouteFailure::owner_offline(query_name, reason)
    } else {
        ResolveRouteFailure::new(query_name, reason, detail)
    }
}

fn hub_negative_route_is_dynamic_convergence_candidate(answer: &serde_json::Value) -> bool {
    let Some(ResolveAnswerKind::Negative) = answer
        .get("answer_kind")
        .and_then(serde_json::Value::as_str)
        .and_then(ResolveAnswerKind::from_str_name)
    else {
        return false;
    };
    let Some(negative) = answer
        .get("negative")
        .and_then(serde_json::Value::as_object)
    else {
        return false;
    };
    let reason = negative
        .get("reason")
        .and_then(serde_json::Value::as_str)
        .and_then(NegativeReason::from_str_name);
    let detail = negative
        .get("detail")
        .and_then(serde_json::Value::as_str)
        .map(str::trim);
    reason == Some(NegativeReason::Nodata)
        && detail == Some(HUB_ROUTE_CONVERGENCE_DETAIL_OWNER_MISSING_ABILITY)
}

fn session_request_error_route_failure(
    query_name: &str,
    error: SessionRequestError,
    local_failure: ResolveRouteFailure,
) -> ResolveRouteFailure {
    match error {
        SessionRequestError::TargetOffline => {
            ResolveRouteFailure::owner_offline(query_name, NegativeReason::Noroute)
        }
        SessionRequestError::PermissionDenied { reason } => ResolveRouteFailure::new(
            query_name,
            NegativeReason::Refused,
            format!("Hub route provider denied namespace.resolve: {reason}"),
        ),
        SessionRequestError::UpstreamFailure { reason } => ResolveRouteFailure::new(
            query_name,
            NegativeReason::Noroute,
            format!(
                "Hub route provider failed after local route miss `{}`: {reason}",
                local_failure.detail
            ),
        ),
        SessionRequestError::UpstreamTimeout => ResolveRouteFailure::new(
            query_name,
            NegativeReason::Noroute,
            "Hub route provider timed out",
        ),
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
struct LocalCredentialIdentity {
    realm: String,
    user_id: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Default)]
enum LocalCredentialIdentityState {
    Available(LocalCredentialIdentity),
    #[default]
    Unpaired,
    Unavailable {
        reason: String,
    },
}

#[derive(Debug, Clone, Default)]
struct LocalAgentTargetIndex {
    projection: LocalAgentTargetProjectionState,
    credential_identity: LocalCredentialIdentityState,
}

impl LocalAgentTargetIndex {
    fn load() -> Self {
        Self {
            projection: LocalAgentTargetProjectionState::load(),
            credential_identity: load_local_credential_identity(),
        }
    }

    fn hosts_target(&self, target: &HostedAgentTarget) -> bool {
        match &self.projection {
            LocalAgentTargetProjectionState::Available { projection } => {
                projection.hosted_agent_targets.contains(target)
            }
            LocalAgentTargetProjectionState::Unavailable { .. } => false,
        }
    }

    fn is_local_target(&self, target: &HostedAgentTarget) -> bool {
        self.hosts_target(target)
            || (self.credentials_match_target(target)
                && self.has_registered_agent_id(&target.agent_id))
    }

    fn credentials_match_target(&self, target: &HostedAgentTarget) -> bool {
        matches!(
            &self.credential_identity,
            LocalCredentialIdentityState::Available(identity)
                if identity.realm == target.realm && identity.user_id == target.user_id
        )
    }

    fn has_registered_agent_id(&self, agent_id: &str) -> bool {
        match &self.projection {
            LocalAgentTargetProjectionState::Available { projection } => {
                projection.registered_agent_ids.contains(agent_id)
            }
            LocalAgentTargetProjectionState::Unavailable { .. } => false,
        }
    }

    fn projection_state_label(&self) -> &'static str {
        self.projection.label()
    }

    fn projection_error(&self) -> &str {
        self.projection.error()
    }

    fn credential_state_label(&self) -> &'static str {
        self.credential_identity.label()
    }

    fn credential_error(&self) -> &str {
        self.credential_identity.error()
    }
}

impl LocalCredentialIdentityState {
    fn label(&self) -> &'static str {
        match self {
            Self::Available(_) => "available",
            Self::Unpaired => "unpaired",
            Self::Unavailable { .. } => "unavailable",
        }
    }

    fn error(&self) -> &str {
        match self {
            Self::Unavailable { reason } => reason,
            Self::Available(_) | Self::Unpaired => "",
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
enum LocalAgentTargetProjectionState {
    Available {
        projection: AgentLocalTargetProjection,
    },
    Unavailable {
        reason: String,
    },
}

impl Default for LocalAgentTargetProjectionState {
    fn default() -> Self {
        Self::Available {
            projection: AgentLocalTargetProjection::default(),
        }
    }
}

impl LocalAgentTargetProjectionState {
    fn load() -> Self {
        match AgentAggregateRepository::try_load_snapshot() {
            Ok(snapshot) => match snapshot.local_target_projection() {
                Ok(projection) => Self::Available { projection },
                Err(error) => {
                    let reason = format!("{error:#}");
                    crate::op_event!(
                        component = daemon_invocation,
                        kind = agent_local_target_projection_invalid,
                        error = reason.as_str(),
                        message = "target_gate: Agent URA self-target matching failed closed because the hosted-Agent target projection is invalid",
                    );
                    Self::Unavailable { reason }
                }
            },
            Err(error) => {
                let reason = format!("{error:#}");
                crate::op_event!(
                    component = daemon_invocation,
                    kind = agent_aggregate_target_index_load_failed,
                    error = reason.as_str(),
                    message = "target_gate: Agent URA self-target matching failed closed because the Agent aggregate snapshot could not be loaded",
                );
                Self::Unavailable { reason }
            }
        }
    }

    fn label(&self) -> &'static str {
        match self {
            Self::Available { .. } => "available",
            Self::Unavailable { .. } => "unavailable",
        }
    }

    fn error(&self) -> &str {
        match self {
            Self::Available { .. } => "",
            Self::Unavailable { reason } => reason,
        }
    }
}

fn load_local_credential_identity() -> LocalCredentialIdentityState {
    match project_local_credential_identity() {
        Ok(Some(identity)) => LocalCredentialIdentityState::Available(identity),
        Ok(None) => LocalCredentialIdentityState::Unpaired,
        Err(error) => {
            let reason = format!("{error:#}");
            crate::op_event!(
                component = daemon_invocation,
                kind = target_gate_credential_identity_load_failed,
                error = reason.as_str(),
                message = "target_gate: local credential identity matching failed closed because credentials could not be projected",
            );
            LocalCredentialIdentityState::Unavailable { reason }
        }
    }
}

fn project_local_credential_identity() -> anyhow::Result<Option<LocalCredentialIdentity>> {
    let Some(creds) = crate::daemon::persistence::config::load_credentials_optional()? else {
        return Ok(None);
    };
    let realm = creds.realm.trim().to_string();
    if realm.is_empty() {
        anyhow::bail!("local credentials realm is empty");
    }
    let user_id = creds.user_id()?.trim().to_string();
    if user_id.is_empty() {
        anyhow::bail!("local credentials user_id is empty");
    }
    Ok(Some(LocalCredentialIdentity { realm, user_id }))
}

#[cfg(test)]
mod local_agent_target_tests {
    use super::*;

    fn target(realm: &str, user_id: &str, agent_id: &str) -> HostedAgentTarget {
        HostedAgentTarget {
            realm: realm.to_string(),
            user_id: user_id.to_string(),
            agent_id: agent_id.to_string(),
        }
    }

    #[test]
    fn local_agent_target_index_available_projection_matches_targets() {
        let hosted_target = target("acme", "u1", "claude");
        let index = LocalAgentTargetIndex {
            projection: LocalAgentTargetProjectionState::Available {
                projection: AgentLocalTargetProjection {
                    hosted_agent_targets: [hosted_target.clone()].into(),
                    registered_agent_ids: ["codex".to_string()].into(),
                },
            },
            credential_identity: LocalCredentialIdentityState::Available(LocalCredentialIdentity {
                realm: "acme".to_string(),
                user_id: "u1".to_string(),
            }),
        };

        assert!(index.hosts_target(&hosted_target));
        assert!(index.credentials_match_target(&target("acme", "u1", "codex")));
        assert!(index.has_registered_agent_id("codex"));
        assert_eq!(index.projection_state_label(), "available");
        assert_eq!(index.projection_error(), "");
        assert_eq!(index.credential_state_label(), "available");
        assert_eq!(index.credential_error(), "");
    }

    #[test]
    fn local_agent_target_index_unavailable_projection_fails_closed() {
        let index = LocalAgentTargetIndex {
            projection: LocalAgentTargetProjectionState::Unavailable {
                reason: "load Agent registry projection: denied".to_string(),
            },
            credential_identity: LocalCredentialIdentityState::Available(LocalCredentialIdentity {
                realm: "acme".to_string(),
                user_id: "u1".to_string(),
            }),
        };

        assert!(!index.hosts_target(&target("acme", "u1", "claude")));
        assert!(index.credentials_match_target(&target("acme", "u1", "codex")));
        assert!(!index.has_registered_agent_id("codex"));
        assert_eq!(index.projection_state_label(), "unavailable");
        assert!(index.projection_error().contains("denied"));
    }

    #[test]
    fn local_agent_target_index_unpaired_credentials_do_not_match_targets() {
        let hosted_target = target("acme", "u1", "codex");
        let index = LocalAgentTargetIndex {
            projection: LocalAgentTargetProjectionState::Available {
                projection: AgentLocalTargetProjection::default(),
            },
            credential_identity: LocalCredentialIdentityState::Unpaired,
        };

        assert!(!index.credentials_match_target(&hosted_target));
        assert_eq!(index.credential_state_label(), "unpaired");
        assert_eq!(index.credential_error(), "");
    }

    #[test]
    fn local_agent_target_index_unavailable_credentials_fail_closed() {
        let hosted_target = target("acme", "u1", "codex");
        let index = LocalAgentTargetIndex {
            projection: LocalAgentTargetProjectionState::Available {
                projection: AgentLocalTargetProjection::default(),
            },
            credential_identity: LocalCredentialIdentityState::Unavailable {
                reason: "validate credentials: all-zero user_id".to_string(),
            },
        };

        assert!(!index.credentials_match_target(&hosted_target));
        assert_eq!(index.credential_state_label(), "unavailable");
        assert!(index.credential_error().contains("all-zero user_id"));
    }
}

fn local_runtime_authority_ura(
    daemon_ura: Option<&str>,
    session_realm: Option<&str>,
) -> Option<String> {
    if let Some(daemon_ura) = daemon_ura.map(str::trim).filter(|ura| !ura.is_empty()) {
        if let Some(hub_realm) = crate::core::ura::parse_ura(daemon_ura)
            .ok()
            .and_then(|parsed| {
                (parsed.kind == crate::core::ura::URAKind::Authority).then_some(parsed.realm)
            })
        {
            if let Ok(local_device_ura) =
                crate::daemon::identity::local_invocation::local_device_ura()
            {
                if crate::core::ura::parse_ura(&local_device_ura)
                    .ok()
                    .is_some_and(|parsed| {
                        parsed.realm == hub_realm
                            && session_realm.is_none_or(|realm| realm == parsed.realm)
                    })
                {
                    return Some(local_device_ura);
                }
            }
        }
        return Some(daemon_ura.to_string());
    }
    None
}

// ── Route-outcome wire mapping ─────────────────────────────────────
//
// Stable error codes + `Status` constructors for every way a
// namespace.resolve outcome can refuse dispatch. They live with the
// gate because they are the wire-visible half of its verdicts; the
// dispatch surfaces consume them verbatim.

pub(crate) const ROUTE_NEGATIVE_CODE: &str = "ROUTE_NEGATIVE";
pub(crate) const ROUTE_PROFILE_BLOCKED_CODE: &str = "ROUTE_PROFILE_BLOCKED";

pub(crate) fn route_negative_message(failure: &ResolveRouteFailure) -> String {
    format!(
        "{ROUTE_NEGATIVE_CODE}: namespace.resolve negative for `{}`: {}: {}",
        failure.query_name,
        failure.reason.as_str_name(),
        failure.detail,
    )
}

pub(crate) fn route_negative_status(failure: ResolveRouteFailure) -> Status {
    use NegativeReason;

    let message = route_negative_message(&failure);
    if route_negative_owner_offline(&failure) {
        return Status::unavailable(message);
    }
    match failure.reason {
        NegativeReason::Nxdomain | NegativeReason::Nodata => Status::not_found(message),
        NegativeReason::Unauthorized => Status::permission_denied(message),
        NegativeReason::Throttled => Status::resource_exhausted(message),
        NegativeReason::Overloaded => Status::unavailable(message),
        NegativeReason::Refused => Status::invalid_argument(message),
        NegativeReason::Unspecified
        | NegativeReason::Noroute
        | NegativeReason::Stale
        | NegativeReason::Loop => Status::failed_precondition(message),
    }
}

fn route_negative_owner_offline(failure: &ResolveRouteFailure) -> bool {
    failure.is_owner_offline()
}

pub(crate) fn route_profile_blocked_message(selected_route: &SelectedInvokeRoute) -> String {
    format!(
        "{ROUTE_PROFILE_BLOCKED_CODE}: namespace.resolve selected route `{}` with \
         non-dispatchable release profile {}",
        selected_route.route_ura,
        selected_route.release_profile.as_str_name(),
    )
}

pub(crate) fn route_profile_blocked_status(selected_route: &SelectedInvokeRoute) -> Status {
    Status::failed_precondition(route_profile_blocked_message(selected_route))
}

/// Build the descriptor-bound child invocation facts for a caller-signed
/// envelope selected for carrier dispatch.
///
/// Signed envelopes are immutable: changing callee, descriptor ref, subject,
/// dispatch payload, or descriptor version after prepare changes Axon's
/// canonical bytes and turns a valid signature into
/// `CALLER_SIGNATURE_INVALID` on the executing daemon. RFC-014 centralizes
/// that drift check in `ChildInvocationBuilder`; this helper only adapts the
/// resolver-selected route plus canonical typed target into the builder input.
pub(crate) fn signed_envelope_for_selected_route(
    envelope: Envelope,
    selected_route: &SelectedInvokeRoute,
    target: Option<&InvocationTarget>,
    args: &[u8],
) -> Result<Envelope, Status> {
    let signed_callee = envelope
        .callee
        .as_ref()
        .map(|callee| callee.ura.as_str())
        .map(str::trim)
        .filter(|callee| !callee.is_empty())
        .ok_or_else(|| Status::invalid_argument("signed remote Invoke envelope missing callee"))?
        .to_string();
    let signed_descriptor_ref =
        crate::daemon::invocation::dispatch::invocation_wire::descriptor_ref_from_invocation_target(
            "signed remote Invoke",
            &signed_callee,
            target,
        )
        .map_err(|status| {
            Status::invalid_argument(format!(
                "{}: selected route `{}` requires a typed descriptor target: {}",
                SignatureDecisionReason::SignedDescriptorRefMissing.as_str(),
                selected_route.route_ura,
                status.message()
            ))
        })?;
    let route = SelectedChildRoute::descriptor_bound(
        selected_route.route_ura.clone(),
        selected_route.callee_ura.clone(),
        selected_route.execution_host_ura.clone(),
        selected_route.ability_ura.clone(),
        selected_route.dispatch_name.clone(),
        signed_descriptor_ref.clone(),
    )
    .map_err(status_from_child_invocation_failure)?;
    let signed_caller = envelope
        .caller
        .as_ref()
        .map(|caller| caller.ura.as_str())
        .map(str::trim)
        .filter(|caller| !caller.is_empty())
        .ok_or_else(|| Status::invalid_argument("signed remote Invoke envelope missing caller"))?
        .to_string();
    if envelope.caller_signature.is_none() {
        return Err(Status::invalid_argument(format!(
            "{}: externally signed child invocation missing caller signature",
            SignatureDecisionReason::CallerSignatureMissing.as_str()
        )));
    }
    let signed_subject = envelope
        .subject
        .as_ref()
        .map(|subject| subject.ura.as_str())
        .map(str::trim)
        .filter(|subject| !subject.is_empty())
        .ok_or_else(|| Status::invalid_argument("signed remote Invoke envelope missing subject"))?
        .to_string();
    let canonical_hash = signed_child_canonical_hash(&envelope, &signed_descriptor_ref, args)?;

    ChildInvocationBuilder::build(ChildInvocationBuildInput {
        route,
        child_subject_ura: signed_subject.clone(),
        args: args.to_vec(),
        authority: ChildInvocationAuthority::ExternallySigned(ExternallySignedChildInvocation {
            caller_ura: signed_caller,
            signed_callee_ura: signed_callee,
            signed_descriptor_ref,
            signed_subject_ura: signed_subject,
            canonical_hash,
        }),
    })
    .map_err(status_from_child_invocation_failure)?;
    Ok(envelope)
}

fn signed_child_canonical_hash(
    envelope: &Envelope,
    signed_descriptor_ref: &str,
    args: &[u8],
) -> Result<String, Status> {
    let descriptor_bound =
        descriptor_bound_from_wire_parts(envelope.clone(), signed_descriptor_ref.to_string(), args)
            .map_err(|err| {
                Status::invalid_argument(format!(
                    "{}: rebuild descriptor-bound child invocation bytes: {err}",
                    SignatureDecisionReason::CanonicalHashMismatch.as_str()
                ))
            })?;
    Ok(format!(
        "sha256:{}",
        hex::encode(axon_sdk::invocation::sha256(
            &descriptor_bound_canonical_bytes(&descriptor_bound.envelope)
        ))
    ))
}

fn status_from_child_invocation_failure(failure: ChildInvocationBuildFailure) -> Status {
    let reason = failure
        .signature_reason
        .map(SignatureDecisionReason::as_str)
        .map(ToOwned::to_owned)
        .unwrap_or_else(|| failure.code.as_str().to_string());
    let message = format!("{reason}: {}", failure.reason);
    match failure.stage {
        TraceStage::PolicyDenied | TraceStage::AuthorityDenied => {
            Status::permission_denied(message)
        }
        _ => Status::invalid_argument(message),
    }
}

#[cfg(test)]
mod tests {
    use super::{
        hub_negative_route_failure, hub_negative_route_is_dynamic_convergence_candidate,
        local_runtime_authority_ura, namespace_route_query, route_negative_status,
        HUB_ROUTE_CONVERGENCE_DETAIL_OWNER_MISSING_ABILITY, ROUTE_NEGATIVE_CODE,
    };
    use crate::daemon::federation::resolver_contract::{NegativeReason, ResolveAnswerKind};
    use crate::daemon::invocation::routing::route_resolver::ResolveRouteFailure;
    use axon_sdk::invocation::CallMode;

    fn negative_status(reason: NegativeReason) -> tonic::Status {
        negative_status_with_detail(reason, "test negative")
    }

    fn negative_status_with_detail(reason: NegativeReason, detail: &str) -> tonic::Status {
        route_negative_status(ResolveRouteFailure::new(
            "easynet:///r/acme/device/node-a#skill.list",
            reason,
            detail,
        ))
    }

    fn owner_offline_status(reason: NegativeReason) -> tonic::Status {
        route_negative_status(ResolveRouteFailure::owner_offline(
            "easynet:///r/acme/device/node-a#skill.list",
            reason,
        ))
    }

    #[test]
    fn resolver_absence_maps_to_not_found() {
        for reason in [NegativeReason::Nxdomain, NegativeReason::Nodata] {
            assert_eq!(negative_status(reason).code(), tonic::Code::NotFound);
        }
    }

    #[test]
    fn resolver_owner_offline_maps_to_availability_not_absence() {
        for reason in [NegativeReason::Nxdomain, NegativeReason::Noroute] {
            let status = owner_offline_status(reason);

            assert_eq!(status.code(), tonic::Code::Unavailable);
            assert!(status.message().contains(ROUTE_NEGATIVE_CODE));
            assert!(
                status.message().contains("owner is not online"),
                "owner-offline route negative must remain diagnosable: {}",
                status.message()
            );
        }
    }

    #[test]
    fn resolver_owner_offline_detail_is_not_semantic_authority() {
        let status = negative_status_with_detail(NegativeReason::Nxdomain, "owner is not online");

        assert_eq!(status.code(), tonic::Code::NotFound);
    }

    #[test]
    fn resolver_policy_and_capacity_reasons_keep_typed_transport_codes() {
        assert_eq!(
            negative_status(NegativeReason::Unauthorized).code(),
            tonic::Code::PermissionDenied
        );
        assert_eq!(
            negative_status(NegativeReason::Throttled).code(),
            tonic::Code::ResourceExhausted
        );
        assert_eq!(
            negative_status(NegativeReason::Overloaded).code(),
            tonic::Code::Unavailable
        );
        assert_eq!(
            negative_status(NegativeReason::Refused).code(),
            tonic::Code::InvalidArgument
        );
    }

    #[test]
    fn resolver_placement_reasons_remain_failed_precondition() {
        for reason in [
            NegativeReason::Unspecified,
            NegativeReason::Noroute,
            NegativeReason::Stale,
            NegativeReason::Loop,
        ] {
            assert_eq!(
                negative_status(reason).code(),
                tonic::Code::FailedPrecondition
            );
        }
    }

    #[test]
    fn local_runtime_authority_prefers_device_daemon_ura() {
        let device_ura = "easynet:///r/test-realm/device/dev-1";

        assert_eq!(
            local_runtime_authority_ura(Some(device_ura), Some("test-realm")),
            Some(device_ura.to_string())
        );
    }

    #[test]
    fn local_runtime_authority_rejects_session_realm_without_daemon_identity() {
        assert_eq!(local_runtime_authority_ura(None, Some("test-realm")), None);
    }

    #[test]
    fn hub_session_route_query_preserves_bidi_call_mode() {
        let query = namespace_route_query(
            "easynet:///r/acme/device/node-a",
            "fs.transfer",
            CallMode::Bidi,
        );

        assert_eq!(query["query_name"], "easynet:///r/acme/device/node-a");
        assert_eq!(query["ability_name"], "fs.transfer");
        assert_eq!(query["call_mode"], "bidi");
        assert_eq!(query["qtype"], "RESOLVE_TYPE_ROUTE");
    }

    #[test]
    fn hub_session_route_query_preserves_canonical_ability_call_mode() {
        let ability = "easynet:///r/acme/ability/system-agent.node-a.locomotion.fs.transfer";
        let query =
            namespace_route_query("easynet:///r/acme/device/node-a", ability, CallMode::Bidi);

        assert_eq!(query["query_name"], ability);
        assert_eq!(query["ability_name"], "");
        assert_eq!(query["call_mode"], "bidi");
        assert_eq!(query["qtype"], "RESOLVE_TYPE_ROUTE");
    }

    #[test]
    fn hub_negative_route_answer_preserves_reason_and_detail() {
        let failure = hub_negative_route_failure(
            &serde_json::json!({
                "answer_kind": ResolveAnswerKind::Negative.as_str_name(),
                "negative": {
                    "reason": NegativeReason::Nodata.as_str_name(),
                    "query_name": "easynet:///r/acme/device/node-a#fs.transfer",
                    "detail": "ability projection has no bidi descriptor geometry",
                }
            }),
            "fs.transfer",
        );

        assert_eq!(
            failure.query_name,
            "easynet:///r/acme/device/node-a#fs.transfer"
        );
        assert_eq!(failure.reason, NegativeReason::Nodata);
        assert_eq!(
            failure.detail,
            "ability projection has no bidi descriptor geometry"
        );
    }

    #[test]
    fn hub_negative_dynamic_convergence_retry_is_exactly_scoped() {
        let candidate = serde_json::json!({
            "answer_kind": ResolveAnswerKind::Negative.as_str_name(),
            "negative": {
                "reason": NegativeReason::Nodata.as_str_name(),
                "query_name": "easynet:///r/acme/device/node-a#user_plugin.echo",
                "detail": HUB_ROUTE_CONVERGENCE_DETAIL_OWNER_MISSING_ABILITY,
            }
        });
        assert!(hub_negative_route_is_dynamic_convergence_candidate(
            &candidate
        ));

        let owner_offline = serde_json::json!({
            "answer_kind": ResolveAnswerKind::Negative.as_str_name(),
            "negative": {
                "reason": NegativeReason::Nxdomain.as_str_name(),
                "query_name": "easynet:///r/acme/device/node-a#user_plugin.echo",
                "detail": "owner is not online",
            }
        });
        assert!(!hub_negative_route_is_dynamic_convergence_candidate(
            &owner_offline
        ));

        let policy_denied = serde_json::json!({
            "answer_kind": ResolveAnswerKind::Negative.as_str_name(),
            "negative": {
                "reason": NegativeReason::Refused.as_str_name(),
                "query_name": "easynet:///r/acme/device/node-a#user_plugin.echo",
                "detail": HUB_ROUTE_CONVERGENCE_DETAIL_OWNER_MISSING_ABILITY,
            }
        });
        assert!(!hub_negative_route_is_dynamic_convergence_candidate(
            &policy_denied
        ));
    }

    #[test]
    fn local_runtime_authority_executes_same_realm_hub_through_local_device() {
        let _home = crate::cli::commands::test_support::HomeGuard::new();
        crate::daemon::persistence::config::save_credentials(
            &crate::daemon::persistence::config::Credentials {
                node_id: "local".to_string(),
                credential_token: "token".to_string(),
                hub_endpoint: "axon://hub.example:50051".to_string(),
                realm: "test-realm".to_string(),
                username: Some("alice".to_string()),
                user_id: Some("user-alice".to_string()),
                ..Default::default()
            },
        )
        .expect("write local device credentials");
        let local_device_ura = crate::daemon::identity::local_invocation::local_device_ura()
            .expect("credentials-backed local device URA");
        let local_realm = crate::core::ura::parse_ura(&local_device_ura)
            .expect("local device URA parses")
            .realm;

        assert_eq!(
            local_runtime_authority_ura(
                Some(&crate::core::ura::hub_ura(&local_realm)),
                Some(&local_realm)
            ),
            Some(local_device_ura)
        );
    }

    #[test]
    fn local_runtime_authority_keeps_cross_realm_hub_as_callee_authority() {
        let _home = crate::cli::commands::test_support::HomeGuard::new();
        crate::daemon::persistence::config::save_credentials(
            &crate::daemon::persistence::config::Credentials {
                node_id: "local".to_string(),
                credential_token: "token".to_string(),
                hub_endpoint: "axon://hub.example:50051".to_string(),
                realm: "test-realm".to_string(),
                username: Some("alice".to_string()),
                user_id: Some("user-alice".to_string()),
                ..Default::default()
            },
        )
        .expect("write local device credentials");
        let local_device_ura = crate::daemon::identity::local_invocation::local_device_ura()
            .expect("credentials-backed local device URA");
        let local_realm = crate::core::ura::parse_ura(&local_device_ura)
            .expect("local device URA parses")
            .realm;
        let remote_realm = format!("{local_realm}-remote");
        let hub_ura = crate::core::ura::hub_ura(&remote_realm);

        assert_eq!(
            local_runtime_authority_ura(Some(&hub_ura), Some(&local_realm)),
            Some(hub_ura)
        );
    }

    #[test]
    fn local_runtime_authority_rejects_empty_inputs() {
        assert_eq!(local_runtime_authority_ura(Some("  "), Some("  ")), None);
        assert_eq!(local_runtime_authority_ura(None, None), None);
    }
}
