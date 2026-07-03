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

use std::collections::HashSet;

use tonic::Status;

use crate::daemon::invocation::admission::admission_facade::AdmissionFacade;
use crate::daemon::invocation::dispatch::deps::{
    DirectoryPlane, FederationDial, IdentityPlane, RuntimePlane,
};
use crate::daemon::invocation::routing::route_resolver::{
    DaemonRouteResolver, LocalRuntimeAuthoritySnapshot, ResolveRouteFailure, SelectedInvokeRoute,
};
use easynet_axon::pb::axon::v1::{AgentIdentity, Envelope};

/// Resolve-first gate over the daemon's routing authorities. Cheap to
/// construct (every plane is `Arc`-shaped); the service builds one per
/// dispatch via `DaemonInvocationService::target_gate()`.
#[derive(Clone)]
pub(crate) struct TargetGate {
    admission: AdmissionFacade,
    directory: DirectoryPlane,
    federation: FederationDial,
    identity: IdentityPlane,
    runtime: RuntimePlane,
    local_agent_targets: LocalAgentTargetIndex,
}

impl TargetGate {
    pub(crate) fn new(
        admission: AdmissionFacade,
        directory: DirectoryPlane,
        federation: FederationDial,
        identity: IdentityPlane,
        runtime: RuntimePlane,
    ) -> Self {
        Self {
            admission,
            directory,
            federation,
            identity,
            runtime,
            local_agent_targets: LocalAgentTargetIndex::load(),
        }
    }

    /// Build the RFC-005 route resolver wired with every authority this
    /// daemon owns: local presence, hosted-agent placement, owner
    /// projection, optional peer delegation, and — when the daemon has a
    /// live `LocalRuntime` — the daemon's local runtime namespace
    /// authority (RFC-005 §4 / D105).
    ///
    /// The local runtime authority is a snapshot of the dispatch table
    /// captured here, so routes for this device and its hosted agents are
    /// proven from live local bindings rather than the hub projection
    /// cache. Capturing the snapshot is the only async step; the resolver
    /// itself stays synchronous.
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
        if let Some(runtime) = self.runtime.local_runtime.as_ref() {
            if let Some(local_authority_ura) = local_runtime_authority_ura(
                self.admission.daemon_ura(),
                self.identity.session_realm.as_deref(),
            ) {
                let snapshot = LocalRuntimeAuthoritySnapshot::capture(runtime).await;
                resolver =
                    resolver.with_local_runtime_authority(local_authority_ura, Box::new(snapshot));
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
    /// Match for (3) uses the hosted-agent identity, not just the bare
    /// `<agentID>`. A daemon treats an Agent URA as local only when the
    /// target is proven by the local hosted-agent read model:
    ///   * `local-agents.json` contains the exact `(realm,user,agent)`
    ///     tuple; or
    ///   * the tuple matches this daemon's credential identity and the
    ///     exact bare `agentID` exists in `agents.json`.
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
        if let Some(agent_target) = parse_agent_target_identity(target_ura) {
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
                message = "matches_self_target_ura: agent URA not local; \
                          no exact local hosted Agent identity matched. Call \
                          will fall through to PresenceRegistry lookup.",
            );
        }
        false
    }
}

#[derive(Debug, Clone, Hash, PartialEq, Eq)]
struct AgentTargetIdentity {
    realm: String,
    user_id: String,
    agent_id: String,
}

#[derive(Debug, Clone, PartialEq, Eq)]
struct LocalCredentialIdentity {
    realm: String,
    user_id: String,
}

#[derive(Debug, Clone, Default)]
struct LocalAgentTargetIndex {
    hosted_agent_targets: HashSet<AgentTargetIdentity>,
    credential_identity: Option<LocalCredentialIdentity>,
    registered_agent_ids: HashSet<String>,
}

impl LocalAgentTargetIndex {
    fn load() -> Self {
        Self {
            hosted_agent_targets: load_hosted_agent_targets(),
            credential_identity: load_local_credential_identity(),
            registered_agent_ids: load_registered_agent_ids(),
        }
    }

    fn hosts_target(&self, target: &AgentTargetIdentity) -> bool {
        self.hosted_agent_targets.contains(target)
    }

    fn credentials_match_target(&self, target: &AgentTargetIdentity) -> bool {
        self.credential_identity
            .as_ref()
            .map(|identity| identity.realm == target.realm && identity.user_id == target.user_id)
            .unwrap_or(false)
    }

    fn has_registered_agent_id(&self, agent_id: &str) -> bool {
        self.registered_agent_ids.contains(agent_id)
    }
}

/// Extract the full hosted-agent identity from an
/// `agent/<userID>.<agentID>` URA. Returns `None` for any other role
/// or for malformed URAs.
fn parse_agent_target_identity(target_ura: &str) -> Option<AgentTargetIdentity> {
    let parsed = crate::core::ura::parse_ura(target_ura).ok()?;
    if !matches!(parsed.kind, crate::core::ura::URAKind::Agent) {
        return None;
    }
    let realm = parsed.realm.clone();
    let (user_id, agent_id) = parsed.agent_ids()?;
    if realm.is_empty() || user_id.is_empty() || agent_id.is_empty() {
        return None;
    }
    Some(AgentTargetIdentity {
        realm,
        user_id: user_id.to_string(),
        agent_id: agent_id.to_string(),
    })
}

fn load_hosted_agent_targets() -> HashSet<AgentTargetIdentity> {
    crate::daemon::persistence::local_agents::load()
        .map(|file| {
            file.hosted_agents
                .into_iter()
                .filter_map(|entry| parse_agent_target_identity(&entry.agent_ura))
                .collect()
        })
        .unwrap_or_default()
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

fn load_registered_agent_ids() -> HashSet<String> {
    crate::daemon::persistence::agent_registry::load_agents()
        .map(|registry| registry.agents.into_keys().collect())
        .unwrap_or_default()
}

fn local_runtime_authority_ura(
    daemon_ura: Option<&str>,
    session_realm: Option<&str>,
) -> Option<String> {
    if let Some(daemon_ura) = daemon_ura.map(str::trim).filter(|ura| !ura.is_empty()) {
        return Some(daemon_ura.to_string());
    }
    session_realm
        .map(str::trim)
        .filter(|realm| !realm.is_empty())
        .map(crate::core::ura::hub_ura)
}

#[cfg(test)]
mod tests {
    use super::local_runtime_authority_ura;

    #[test]
    fn local_runtime_authority_prefers_daemon_ura() {
        let device_ura = "easynet:///r/test-realm/device/dev-1";

        assert_eq!(
            local_runtime_authority_ura(Some(device_ura), Some("test-realm")),
            Some(device_ura.to_string())
        );
    }

    #[test]
    fn local_runtime_authority_falls_back_to_hub_ura_for_hub_mode_daemon() {
        assert_eq!(
            local_runtime_authority_ura(None, Some("test-realm")),
            Some(crate::core::ura::hub_ura("test-realm"))
        );
    }

    #[test]
    fn local_runtime_authority_rejects_empty_inputs() {
        assert_eq!(local_runtime_authority_ura(Some("  "), Some("  ")), None);
        assert_eq!(local_runtime_authority_ura(None, None), None);
    }
}

// ── Route-outcome wire mapping ─────────────────────────────────────
//
// Stable error codes + `Status` constructors for every way a
// namespace.resolve outcome can refuse dispatch. They live with the
// gate because they are the wire-visible half of its verdicts; the
// dispatch surfaces consume them verbatim.

pub(crate) const ROUTE_NEGATIVE_CODE: &str = "ROUTE_NEGATIVE";
pub(crate) const RESOLVE_SELECTED_HOST_UNAVAILABLE_CODE: &str = "RESOLVE_UNAVAILABLE";
pub(crate) const ROUTE_PROFILE_BLOCKED_CODE: &str = "ROUTE_PROFILE_BLOCKED";
pub(crate) const ROUTE_OWNER_MISMATCH_CODE: &str = "ROUTE_OWNER_MISMATCH";
pub(crate) const ROUTE_SELECTED_REMOTE_HOST_CODE: &str = "ROUTE_SELECTED_REMOTE_HOST";

pub(crate) fn route_negative_message(failure: &ResolveRouteFailure) -> String {
    format!(
        "{ROUTE_NEGATIVE_CODE}: namespace.resolve negative for `{}`: {}: {}",
        failure.query_name,
        failure.reason.as_str_name(),
        failure.detail,
    )
}

pub(crate) fn route_negative_status(failure: ResolveRouteFailure) -> Status {
    Status::failed_precondition(route_negative_message(&failure))
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

pub(crate) fn route_owner_mismatch_message(
    selected_owner_ura: &str,
    ability_ura: &str,
    expected_target_ura: &str,
) -> String {
    format!(
        "{ROUTE_OWNER_MISMATCH_CODE}: namespace.resolve selected owner `{selected_owner_ura}` \
         for ability `{ability_ura}` but request target was `{expected_target_ura}`"
    )
}

pub(crate) fn route_selected_remote_host_status(
    label: &str,
    selected_route: &SelectedInvokeRoute,
) -> Status {
    Status::failed_precondition(format!(
        "{ROUTE_SELECTED_REMOTE_HOST_CODE}: {label} selected execution host `{}` for route `{}`; \
         direct local dispatch can execute only routes hosted by this daemon",
        selected_route.execution_host_ura, selected_route.route_ura,
    ))
}

pub(crate) fn selected_host_unavailable_message(selected_route: &SelectedInvokeRoute) -> String {
    format!(
        "{RESOLVE_SELECTED_HOST_UNAVAILABLE_CODE}: namespace.resolve selected execution host `{}` \
         for route `{}` but the session disappeared before dispatch",
        selected_route.execution_host_ura, selected_route.route_ura,
    )
}

/// Stamp the resolver-selected callee onto an envelope. Every
/// dispatch surface must send the *selected* callee downstream, not
/// the caller-supplied one — the resolver's verdict is authoritative.
pub(crate) fn envelope_with_selected_callee(
    mut envelope: Envelope,
    selected_route: &SelectedInvokeRoute,
) -> Envelope {
    envelope.callee = Some(AgentIdentity {
        ura: selected_route.callee_ura.clone(),
        profile: crate::daemon::invocation::DEFAULT_URA_PROFILE.to_string(),
    });
    envelope
}
