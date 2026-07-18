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

use tonic::Status;

use crate::daemon::ability::catalog::publication::LocalAbilityPublicationSnapshot;
use crate::daemon::axon_bridge::wire_descriptor::descriptor_bound_from_wire_parts;
use crate::daemon::invocation::admission::admission_facade::AdmissionFacade;
use crate::daemon::invocation::admission::child_invocation_builder::{
    ChildInvocationAuthority, ChildInvocationBuildFailure, ChildInvocationBuildInput,
    ChildInvocationBuilder, ExternallySignedChildInvocation, SelectedChildRoute,
};
use crate::daemon::invocation::admission::decision::{SignatureDecisionReason, TraceStage};
use crate::daemon::invocation::dispatch::deps::{DirectoryPlane, FederationDial, IdentityPlane};
use crate::daemon::invocation::routing::route_resolver::{
    DaemonRouteResolver, ResolveRouteFailure, SelectedInvokeRoute,
};
use crate::daemon::persistence::agent_aggregate::{
    AgentAggregateRepository, AgentLocalTargetProjection, HostedAgentTarget,
};
use axon_sdk::pb::axon::v1::{Envelope, InvocationTarget};

/// Resolve-first gate over the daemon's routing authorities. Cheap to
/// construct (every plane is `Arc`-shaped); the service builds one per
/// dispatch via `DaemonInvocationService::target_gate()`.
#[derive(Clone)]
pub(crate) struct TargetGate {
    admission: AdmissionFacade,
    directory: DirectoryPlane,
    federation: FederationDial,
    identity: IdentityPlane,
    local_agent_targets: LocalAgentTargetIndex,
}

impl TargetGate {
    pub(crate) fn new(
        admission: AdmissionFacade,
        directory: DirectoryPlane,
        federation: FederationDial,
        identity: IdentityPlane,
    ) -> Self {
        Self {
            admission,
            directory,
            federation,
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
            Some(self.directory.ability_catalog.as_ref()),
        );
        if let Some(local_realm) = self
            .identity
            .session_realm
            .as_deref()
            .filter(|realm| !realm.is_empty())
        {
            resolver = resolver.with_peer_delegation(
                local_realm,
                &self.federation.peers,
                &self.directory.federated_directory,
                self.federation.allow_directory_auto_route,
            );
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

    /// Resolve whether `target_ura` names THIS daemon's own
    /// synchronous-execution surface.
    ///
    /// Three valid shapes per RFC-001 + RFC-006-C v0.1:
    ///   (1) `easynet:///r/<realm>/device/<deviceID>` — the daemon's
    ///       device identity from credentials.json. Standard.
    ///   (2) `easynet:///r/<realm>/hub` — the canonical Hub URA;
    ///       hub-mode daemons answer to this in addition to (1).
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
        if crate::daemon::identity::local_invocation::local_device_ura() == target_ura {
            return true;
        }
        if let Some(agent_target) = HostedAgentTarget::parse(target_ura) {
            if self.local_agent_targets.hosts_target(&agent_target) {
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
                message = "matches_self_target_ura: agent URA not local; \
                          no exact local hosted Agent identity matched. Call \
                          will fall through to PresenceRegistry lookup.",
            );
        }
        false
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
struct LocalCredentialIdentity {
    realm: String,
    user_id: String,
}

#[derive(Debug, Clone, Default)]
struct LocalAgentTargetIndex {
    projection: LocalAgentTargetProjectionState,
    credential_identity: Option<LocalCredentialIdentity>,
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

    fn credentials_match_target(&self, target: &HostedAgentTarget) -> bool {
        self.credential_identity
            .as_ref()
            .map(|identity| identity.realm == target.realm && identity.user_id == target.user_id)
            .unwrap_or(false)
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
            Ok(snapshot) => Self::Available {
                projection: snapshot.local_target_projection(),
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

fn load_local_credential_identity() -> Option<LocalCredentialIdentity> {
    crate::daemon::persistence::config::load_credentials()
        .ok()
        .and_then(|creds| {
            let realm = creds.realm.trim().to_string();
            let user_id = creds.user_id().ok()?.trim().to_string();
            if realm.is_empty() || user_id.is_empty() {
                return None;
            }
            Some(LocalCredentialIdentity { realm, user_id })
        })
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
            credential_identity: Some(LocalCredentialIdentity {
                realm: "acme".to_string(),
                user_id: "u1".to_string(),
            }),
        };

        assert!(index.hosts_target(&hosted_target));
        assert!(index.credentials_match_target(&target("acme", "u1", "codex")));
        assert!(index.has_registered_agent_id("codex"));
        assert_eq!(index.projection_state_label(), "available");
        assert_eq!(index.projection_error(), "");
    }

    #[test]
    fn local_agent_target_index_unavailable_projection_fails_closed() {
        let index = LocalAgentTargetIndex {
            projection: LocalAgentTargetProjectionState::Unavailable {
                reason: "load Agent registry projection: denied".to_string(),
            },
            credential_identity: Some(LocalCredentialIdentity {
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
}

fn local_runtime_authority_ura(
    daemon_ura: Option<&str>,
    session_realm: Option<&str>,
) -> Option<String> {
    if let Some(daemon_ura) = daemon_ura.map(str::trim).filter(|ura| !ura.is_empty()) {
        if let Some(hub_realm) = crate::core::ura::parse_ura(daemon_ura)
            .ok()
            .and_then(|parsed| {
                (parsed.kind == crate::core::ura::URAKind::Hub).then_some(parsed.realm)
            })
        {
            let local_device_ura = crate::daemon::identity::local_invocation::local_device_ura();
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
        return Some(daemon_ura.to_string());
    }
    session_realm
        .map(str::trim)
        .filter(|realm| !realm.is_empty())
        .map(crate::core::ura::hub_ura)
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
    use axon_sdk::pb::axon::v1::NegativeReason;

    let message = route_negative_message(&failure);
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
        Some(selected_route.execution_host_ura.clone()),
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
            &descriptor_bound.envelope.canonical_bytes()
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
    use super::{local_runtime_authority_ura, route_negative_status};
    use crate::daemon::invocation::routing::route_resolver::ResolveRouteFailure;

    fn negative_status(reason: axon_sdk::pb::axon::v1::NegativeReason) -> tonic::Status {
        route_negative_status(ResolveRouteFailure {
            query_name: "easynet:///r/acme/device/node-a#skill.list".to_string(),
            reason,
            detail: "test negative".to_string(),
        })
    }

    #[test]
    fn resolver_absence_maps_to_not_found() {
        use axon_sdk::pb::axon::v1::NegativeReason;

        for reason in [NegativeReason::Nxdomain, NegativeReason::Nodata] {
            assert_eq!(negative_status(reason).code(), tonic::Code::NotFound);
        }
    }

    #[test]
    fn resolver_policy_and_capacity_reasons_keep_typed_transport_codes() {
        use axon_sdk::pb::axon::v1::NegativeReason;

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
        use axon_sdk::pb::axon::v1::NegativeReason;

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
    fn local_runtime_authority_falls_back_to_hub_ura_without_daemon_identity() {
        assert_eq!(
            local_runtime_authority_ura(None, Some("test-realm")),
            Some(crate::core::ura::hub_ura("test-realm"))
        );
    }

    #[test]
    fn local_runtime_authority_executes_same_realm_hub_through_local_device() {
        let local_device_ura = crate::daemon::identity::local_invocation::local_device_ura();
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
        let local_device_ura = crate::daemon::identity::local_invocation::local_device_ura();
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
