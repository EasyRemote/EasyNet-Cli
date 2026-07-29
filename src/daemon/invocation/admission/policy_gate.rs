// EasyNet CLI — RFC-014 admission policy gate
// ===========================================
//
// Bridges signed invocation facts into the pure RFC-014 PolicyEngine.
// Signature verification stays in AdmissionFacade; this adapter owns only
// runtime policy facts: principal, owner, action, safe-read, and grants.

use chrono::Utc;
use tonic::Status;

use axon_sdk::pb::axon::v1::Envelope;

use crate::core::ura::{parse_ura, AbilityOwner, URAKind};
use crate::daemon::invocation::admission::decision::{
    AccessAction, OwnerResolution, PolicyDecision, PolicyDecisionOutcome, PolicyDecisionReason,
    PrincipalKind, TokenClass,
};
use crate::daemon::invocation::admission::owner_resolution::{
    OwnerFact, OwnerResolutionInput, OwnerResolver,
};
use crate::daemon::invocation::admission::policy_engine::{PolicyEngine, PolicyInput};
use crate::daemon::persistence::access_control::AccessControlStoreRegistry;
use crate::daemon::trust::anchor::{RealmTrustAnchor, TrustedAgentRole};

#[derive(Debug, Clone)]
pub struct AdmissionPolicyContext<'a> {
    pub envelope: &'a Envelope,
    pub ability: &'a str,
    pub action: AccessAction,
    pub safe_read: bool,
    pub trusted_role: TrustedAgentRole,
    pub daemon_ura: Option<&'a str>,
    pub trust_anchor: &'a RealmTrustAnchor,
    pub access_control_stores: &'a AccessControlStoreRegistry,
    pub canonical_hash: Option<String>,
    pub signature_key_id: Option<String>,
    pub verified_authority_id: Option<String>,
    pub rejector_ura: Option<String>,
}

pub struct AdmissionPolicyGate;

impl AdmissionPolicyGate {
    pub fn verify(context: AdmissionPolicyContext<'_>) -> Result<PolicyDecision, Status> {
        let caller_ura = agent_ura(context.envelope.caller.as_ref(), "caller")?;
        let callee_ura = agent_ura(context.envelope.callee.as_ref(), "callee")?;
        let subject_ura = subject_ura(context.envelope.subject.as_ref())?;
        let ability_ura = ability_ura_for(&callee_ura, context.ability)?;
        let owner = resolve_owner(&subject_ura, &callee_ura, context.trust_anchor)?;
        let principal = principal_for(context.trusted_role, &caller_ura, context.trust_anchor)?;
        let authority_self_read = authority_self_read_scope(
            &caller_ura,
            &callee_ura,
            &subject_ura,
            &ability_ura,
            context.daemon_ura,
            context.trusted_role,
        );
        let authority_self_manage = authority_self_manage_scope(
            &caller_ura,
            &callee_ura,
            &subject_ura,
            &ability_ura,
            context.daemon_ura,
            context.trusted_role,
            context.action,
        );
        let authority_self_stream = authority_self_stream_scope(
            &caller_ura,
            &callee_ura,
            &subject_ura,
            &ability_ura,
            context.daemon_ura,
            context.trusted_role,
            context.action,
        );
        let realm_authority_public_read = realm_authority_public_read_scope(
            &caller_ura,
            &callee_ura,
            &subject_ura,
            &ability_ura,
            context.daemon_ura,
            context.trusted_role,
            context.action,
            context.safe_read,
        );
        let device_self_publication_manage = device_self_publication_manage_scope(
            &caller_ura,
            &callee_ura,
            &subject_ura,
            &ability_ura,
            context.daemon_ura,
            context.trusted_role,
            context.action,
        );
        let device_self_session_stream = device_self_session_stream_scope(
            &caller_ura,
            &callee_ura,
            &subject_ura,
            &ability_ura,
            context.daemon_ura,
            context.trusted_role,
            context.action,
        );
        if remote_owner_forward_allowed(
            &caller_ura,
            &callee_ura,
            context.daemon_ura,
            context.trust_anchor,
        ) {
            return Ok(PolicyDecision {
                decision: PolicyDecisionOutcome::Allow,
                reason: PolicyDecisionReason::FederationForwardAllow,
                owner_user_id: owner.owner_user_id,
                owner_source: owner.owner_source,
                caller_ura,
                principal_kind: principal.kind,
                principal_id: principal.id,
                token_id: principal.token_id,
                callee_ura,
                subject_ura,
                ability_ura,
                action: context.action,
                rejector_ura: context.rejector_ura,
                policy_rule_id: None,
                grant_id: None,
                prompt_request_id: None,
                canonical_hash: context.canonical_hash,
                signature_key_id: context.signature_key_id,
                authority_proof_id: context.verified_authority_id,
            });
        }
        let grants = match owner.owner_user_id.as_deref() {
            Some(owner_user_id) => context
                .access_control_stores
                .with_store(owner_user_id, |store| store.grants())
                .map_err(|err| {
                    Status::internal(format!(
                        "POLICY_STORE_UNAVAILABLE: owner_user_id={owner_user_id} error={err}"
                    ))
                })?,
            None => Vec::new(),
        };

        let decision = PolicyEngine::check(PolicyInput {
            owner,
            caller_user_id: principal.caller_user_id,
            caller_ura,
            principal_kind: principal.kind,
            principal_id: principal.id,
            token_id: principal.token_id,
            token_class: principal.token_class,
            callee_ura,
            subject_ura,
            ability_ura,
            action: context.action,
            safe_read: context.safe_read,
            authority_self_read,
            authority_self_manage,
            authority_self_stream,
            realm_authority_public_read,
            device_self_publication_manage,
            device_self_session_stream,
            interactive_context_available: false,
            canonical_hash: context.canonical_hash,
            signature_key_id: context.signature_key_id,
            verified_authority_id: context.verified_authority_id,
            rejector_ura: context.rejector_ura,
            now: Utc::now(),
            grants,
        });

        match decision.decision {
            PolicyDecisionOutcome::Allow => Ok(decision),
            PolicyDecisionOutcome::Prompt | PolicyDecisionOutcome::Deny => {
                let encoded = serde_json::to_string(&decision).unwrap_or_else(|_| {
                    format!(
                        "{{\"decision\":\"{:?}\",\"reason\":\"{:?}\"}}",
                        decision.decision, decision.reason
                    )
                });
                Err(Status::permission_denied(format!(
                    "POLICY_DENIED: {encoded}"
                )))
            }
        }
    }
}

#[derive(Debug, Clone)]
pub(crate) struct PrincipalProjection {
    pub(crate) kind: PrincipalKind,
    pub(crate) id: String,
    pub(crate) token_id: Option<String>,
    pub(crate) token_class: Option<TokenClass>,
    pub(crate) caller_user_id: Option<String>,
}

pub(crate) fn principal_for(
    role: TrustedAgentRole,
    caller_ura: &str,
    trust_anchor: &RealmTrustAnchor,
) -> Result<PrincipalProjection, Status> {
    match role {
        TrustedAgentRole::User => {
            let user_id = user_id_from_ura(caller_ura).unwrap_or_else(|| caller_ura.to_string());
            Ok(PrincipalProjection {
                kind: PrincipalKind::User,
                id: user_id.clone(),
                token_id: None,
                token_class: None,
                caller_user_id: Some(user_id),
            })
        }
        TrustedAgentRole::Hub => Ok(PrincipalProjection {
            kind: PrincipalKind::Token,
            id: caller_ura.to_string(),
            token_id: Some(caller_ura.to_string()),
            token_class: Some(TokenClass::HubLink),
            caller_user_id: None,
        }),
        TrustedAgentRole::Device => {
            let owner_fact = trust_anchor
                .lookup_principal_owner(caller_ura)
                .map(|owner| OwnerFact::user(owner.owner_user_id.clone(), owner.owner_ura.clone()));
            let owner_user_id = owner_fact.and_then(|owner| owner.owner_user_id);
            Ok(PrincipalProjection {
                kind: PrincipalKind::Device,
                id: caller_ura.to_string(),
                token_id: Some(caller_ura.to_string()),
                token_class: Some(TokenClass::DevicePairing),
                caller_user_id: owner_user_id,
            })
        }
        TrustedAgentRole::Backend => Ok(PrincipalProjection {
            kind: PrincipalKind::Service,
            id: caller_ura.to_string(),
            token_id: None,
            token_class: None,
            caller_user_id: None,
        }),
    }
}

pub(crate) fn resolve_owner(
    subject_ura: &str,
    callee_ura: &str,
    trust_anchor: &RealmTrustAnchor,
) -> Result<OwnerResolution, Status> {
    let subject = owner_fact_from_ura(subject_ura, trust_anchor)?;
    let callee = owner_fact_from_ura(callee_ura, trust_anchor)?;
    let device = owner_fact_from_trust_anchor(callee_ura, trust_anchor);
    Ok(OwnerResolver::resolve(&OwnerResolutionInput {
        subject,
        callee,
        device,
        session: None,
    }))
}

fn owner_fact_from_ura(
    ura: &str,
    trust_anchor: &RealmTrustAnchor,
) -> Result<Option<OwnerFact>, Status> {
    if let Some(owner) = owner_fact_from_trust_anchor(ura, trust_anchor) {
        return Ok(Some(owner));
    }
    let parsed = parse_ura(ura).map_err(|error| {
        Status::invalid_argument(format!("OWNER_FACT_URA_INVALID: {ura}: {error}"))
    })?;
    let owner = match parsed.kind {
        URAKind::User => parsed
            .user_id()
            .map(|user_id| OwnerFact::user(user_id, ura.to_string())),
        URAKind::Agent => parsed.agent_ids().map(|(user_id, _)| {
            OwnerFact::user(user_id, crate::core::ura::user_ura(&parsed.realm, user_id))
        }),
        URAKind::Ability => match parsed.ability().map(|ability| ability.owner) {
            Some(AbilityOwner::Agent { user_id, agent_id }) => owner_fact_from_trust_anchor(
                &crate::core::ura::agent_ura(&parsed.realm, &user_id, &agent_id),
                trust_anchor,
            )
            .or_else(|| {
                Some(OwnerFact::user(
                    user_id.clone(),
                    crate::core::ura::user_ura(&parsed.realm, &user_id),
                ))
            }),
            Some(AbilityOwner::Device { device_id }) => {
                let device_ura = crate::core::ura::device_ura(&parsed.realm, &device_id);
                owner_fact_from_trust_anchor(&device_ura, trust_anchor)
            }
            Some(AbilityOwner::Authority) => None,
            None => None,
        },
        URAKind::Device => owner_fact_from_trust_anchor(ura, trust_anchor),
        URAKind::Authority => owner_fact_from_trust_anchor(ura, trust_anchor),
        URAKind::Resource => resource_owner_user_id(&parsed).map(|user_id| {
            OwnerFact::user(
                user_id.clone(),
                crate::core::ura::user_ura(&parsed.realm, &user_id),
            )
        }),
        _ => None,
    };
    Ok(owner)
}

fn owner_fact_from_trust_anchor(ura: &str, trust_anchor: &RealmTrustAnchor) -> Option<OwnerFact> {
    let owner = trust_anchor.lookup_principal_owner(ura)?;
    Some(OwnerFact::user(
        owner.owner_user_id.clone(),
        owner.owner_ura.clone(),
    ))
}

fn resource_owner_user_id(parsed: &crate::core::ura::ParsedURA) -> Option<String> {
    let owner_id = parsed.resource_owner_id()?;
    if let Some(rest) = owner_id.strip_prefix("agent.") {
        let (user_id, _) = rest.split_once('.')?;
        return (!user_id.is_empty()).then_some(user_id.to_string());
    }
    owner_id
        .strip_prefix("user.")
        .filter(|user_id| !user_id.is_empty())
        .map(ToString::to_string)
}

fn user_id_from_ura(ura: &str) -> Option<String> {
    let parsed = parse_ura(ura).ok()?;
    (parsed.kind == URAKind::User)
        .then(|| parsed.user_id().map(ToString::to_string))
        .flatten()
}

fn remote_owner_forward_allowed(
    caller_ura: &str,
    callee_ura: &str,
    daemon_ura: Option<&str>,
    trust_anchor: &RealmTrustAnchor,
) -> bool {
    let Some(daemon_ura) = daemon_ura else {
        return false;
    };
    let Ok(local) = parse_ura(daemon_ura) else {
        return false;
    };
    if local.kind != URAKind::Authority {
        return false;
    }
    let Ok(caller) = parse_ura(caller_ura) else {
        return false;
    };
    if caller.realm != local.realm {
        return false;
    }
    let Ok(callee) = parse_ura(callee_ura) else {
        return false;
    };
    if callee.realm == local.realm {
        return false;
    }
    trust_anchor.has_federation_peer_for_realm(&callee.realm)
}

fn authority_self_read_scope(
    caller_ura: &str,
    callee_ura: &str,
    subject_ura: &str,
    ability_ura: &str,
    daemon_ura: Option<&str>,
    trusted_role: TrustedAgentRole,
) -> bool {
    if trusted_role != TrustedAgentRole::Hub {
        return false;
    }
    if Some(callee_ura) != daemon_ura || caller_ura != callee_ura {
        return false;
    }
    let Ok(callee) = parse_ura(callee_ura) else {
        return false;
    };
    if callee.kind != URAKind::Authority {
        return false;
    }
    if !is_authority_owned_ura_in_realm(ability_ura, &callee.realm) {
        return false;
    }
    subject_ura == callee_ura || is_authority_owned_ura_in_realm(subject_ura, &callee.realm)
}

fn authority_self_manage_scope(
    caller_ura: &str,
    callee_ura: &str,
    subject_ura: &str,
    ability_ura: &str,
    daemon_ura: Option<&str>,
    trusted_role: TrustedAgentRole,
    action: AccessAction,
) -> bool {
    if action != AccessAction::Manage || trusted_role != TrustedAgentRole::Hub {
        return false;
    }
    if Some(callee_ura) != daemon_ura || caller_ura != callee_ura {
        return false;
    }
    let Ok(callee) = parse_ura(callee_ura) else {
        return false;
    };
    if callee.kind != URAKind::Authority {
        return false;
    }
    if !is_authority_owned_ura_in_realm(ability_ura, &callee.realm) {
        return false;
    }
    ura_is_in_realm(subject_ura, &callee.realm)
}

fn authority_self_stream_scope(
    caller_ura: &str,
    callee_ura: &str,
    subject_ura: &str,
    ability_ura: &str,
    daemon_ura: Option<&str>,
    trusted_role: TrustedAgentRole,
    action: AccessAction,
) -> bool {
    if action != AccessAction::Stream || trusted_role != TrustedAgentRole::Hub {
        return false;
    }
    if Some(callee_ura) != daemon_ura || caller_ura != callee_ura {
        return false;
    }
    let Ok(callee) = parse_ura(callee_ura) else {
        return false;
    };
    if callee.kind != URAKind::Authority {
        return false;
    }
    if !is_authority_owned_ura_in_realm(ability_ura, &callee.realm) {
        return false;
    }
    subject_ura == callee_ura || is_authority_resource_subject_in_realm(subject_ura, &callee.realm)
}

fn realm_authority_public_read_scope(
    caller_ura: &str,
    callee_ura: &str,
    subject_ura: &str,
    ability_ura: &str,
    daemon_ura: Option<&str>,
    trusted_role: TrustedAgentRole,
    action: AccessAction,
    safe_read: bool,
) -> bool {
    if trusted_role != TrustedAgentRole::Hub || action != AccessAction::Read || !safe_read {
        return false;
    }
    if Some(callee_ura) != daemon_ura {
        return false;
    }
    let Ok(caller) = parse_ura(caller_ura) else {
        return false;
    };
    if caller.kind != URAKind::Authority {
        return false;
    }
    let Ok(callee) = parse_ura(callee_ura) else {
        return false;
    };
    if callee.kind != URAKind::Device || callee.realm != caller.realm {
        return false;
    }
    let Some(device_id) = callee.device_id() else {
        return false;
    };
    subject_ura == callee_ura || device_owned_ability_matches(ability_ura, &callee.realm, device_id)
}

fn device_self_publication_manage_scope(
    caller_ura: &str,
    callee_ura: &str,
    subject_ura: &str,
    ability_ura: &str,
    daemon_ura: Option<&str>,
    trusted_role: TrustedAgentRole,
    action: AccessAction,
) -> bool {
    if action != AccessAction::Manage || trusted_role != TrustedAgentRole::Device {
        return false;
    }
    let Some(daemon_ura) = daemon_ura else {
        return false;
    };
    if caller_ura != subject_ura || callee_ura != daemon_ura {
        return false;
    }
    let Ok(caller) = parse_ura(caller_ura) else {
        return false;
    };
    if caller.kind != URAKind::Device {
        return false;
    }
    let Ok(callee) = parse_ura(callee_ura) else {
        return false;
    };
    if callee.kind != URAKind::Authority || callee.realm != caller.realm {
        return false;
    }
    ability_ura
        == crate::core::ura::owner_ability_ura(
            callee_ura,
            crate::daemon::invocation::dispatch::federation_wrappers::ABILITY_FEDERATION_ADVERTISE_ABILITIES,
        )
        .unwrap_or_default()
}

fn device_owned_ability_matches(ability_ura: &str, realm: &str, device_id: &str) -> bool {
    let Ok(ability) = parse_ura(ability_ura) else {
        return false;
    };
    if ability.realm != realm || ability.kind != URAKind::Ability {
        return false;
    }
    matches!(
        ability.ability().map(|ability| ability.owner),
        Some(AbilityOwner::Device { device_id: owner_device_id }) if owner_device_id == device_id
    )
}

fn device_self_session_stream_scope(
    caller_ura: &str,
    callee_ura: &str,
    subject_ura: &str,
    ability_ura: &str,
    daemon_ura: Option<&str>,
    trusted_role: TrustedAgentRole,
    action: AccessAction,
) -> bool {
    if action != AccessAction::Stream || trusted_role != TrustedAgentRole::Device {
        return false;
    }
    let Some(daemon_ura) = daemon_ura else {
        return false;
    };
    if caller_ura != subject_ura || callee_ura != daemon_ura {
        return false;
    }
    let Ok(caller) = parse_ura(caller_ura) else {
        return false;
    };
    if caller.kind != URAKind::Device {
        return false;
    }
    let Ok(callee) = parse_ura(callee_ura) else {
        return false;
    };
    if callee.kind != URAKind::Authority || callee.realm != caller.realm {
        return false;
    }
    ability_ura
        == crate::core::ura::owner_ability_ura(
            callee_ura,
            crate::daemon::invocation::bidi::session_initiator::ABILITY_SESSION_OPEN,
        )
        .unwrap_or_default()
}

fn is_authority_owned_ura_in_realm(ura: &str, realm: &str) -> bool {
    let Ok(parsed) = parse_ura(ura) else {
        return false;
    };
    parsed.realm == realm
        && parsed.kind == URAKind::Ability
        && parsed
            .ability()
            .is_some_and(|ability| ability.owner == AbilityOwner::Authority)
}

fn is_authority_resource_subject_in_realm(ura: &str, realm: &str) -> bool {
    let Ok(parsed) = parse_ura(ura) else {
        return false;
    };
    parsed.realm == realm
        && parsed.kind == URAKind::Resource
        && parsed
            .resource_owner_id()
            .is_some_and(|owner| owner == "authority")
}

fn ura_is_in_realm(ura: &str, realm: &str) -> bool {
    parse_ura(ura)
        .map(|parsed| parsed.realm == realm)
        .unwrap_or(false)
}

pub(crate) fn ability_ura_for(callee_ura: &str, ability: &str) -> Result<String, Status> {
    crate::daemon::axon_bridge::descriptor_ref::ability_ura_for_wire(callee_ura, ability).map_err(
        |err| {
            Status::invalid_argument(format!(
                "ABILITY_URA_PROJECTION_FAILED: callee={callee_ura} ability={ability}: {err}"
            ))
        },
    )
}

fn agent_ura(
    identity: Option<&axon_sdk::pb::axon::v1::AgentIdentity>,
    role: &str,
) -> Result<String, Status> {
    identity
        .map(|identity| identity.ura.trim())
        .filter(|ura| !ura.is_empty())
        .map(ToString::to_string)
        .ok_or_else(|| Status::invalid_argument(format!("Invoke envelope missing {role} URA")))
}

fn subject_ura(
    identity: Option<&axon_sdk::pb::axon::v1::SubjectIdentity>,
) -> Result<String, Status> {
    identity
        .map(|identity| identity.ura.trim())
        .filter(|ura| !ura.is_empty())
        .map(ToString::to_string)
        .ok_or_else(|| Status::invalid_argument("Invoke envelope missing subject URA"))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::cli::commands::test_support::HomeGuard;
    use crate::daemon::invocation::admission::decision::{OwnerSource, PolicyDecisionReason};
    use crate::daemon::persistence::config::{save_credentials, Credentials};
    use crate::daemon::trust::anchor::{TrustedAgent, TrustedPrincipalOwner};
    use axon_sdk::pb::axon::v1::{AgentIdentity, SubjectIdentity};
    use std::path::PathBuf;

    fn identity(ura: &str) -> AgentIdentity {
        AgentIdentity {
            ura: ura.to_string(),
            profile: String::new(),
        }
    }

    fn save_test_credentials() {
        save_credentials(&Credentials {
            node_id: "dev-1".to_string(),
            credential_token: "token".to_string(),
            hub_endpoint: "https://127.0.0.1:50443".to_string(),
            realm: "test".to_string(),
            deploy_signature: String::new(),
            hub_api_base: Some("http://127.0.0.1:8080".to_string()),
            username: Some("alice".to_string()),
            user_id: Some("alice".to_string()),
            hub_pubkey_b64: None,
            hub_tls_ca_pem_b64: None,
            join_receipt_hash: Some("join-hash".to_string()),
        })
        .expect("save test credentials");
    }

    fn empty_anchor() -> RealmTrustAnchor {
        RealmTrustAnchor::default()
    }

    fn anchor_with_device_owner() -> RealmTrustAnchor {
        RealmTrustAnchor::from_parts_with_principal_owners(
            Vec::new(),
            vec![TrustedPrincipalOwner {
                principal_ura: "easynet:///r/test/device/dev-1".to_string(),
                owner_user_id: "alice".to_string(),
                owner_ura: "easynet:///r/test/user/alice".to_string(),
                added_at_unix_ms: 1,
            }],
            Vec::new(),
        )
        .expect("owner anchor")
    }

    fn anchor_with_peer_realm() -> RealmTrustAnchor {
        RealmTrustAnchor::from_parts_with_principal_owners(
            vec![TrustedAgent {
                agent_ura: "easynet:///r/peer/authority".to_string(),
                public_key_b64: "AAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAA=".to_string(),
                role: TrustedAgentRole::Hub,
                added_at_unix_ms: 1,
                origin_realm: Some("peer".to_string()),
                hub_endpoint: Some("https://peer-hub.example:50443".to_string()),
                tls_ca_pem_path: Some(PathBuf::from("/tmp/peer-ca.pem")),
            }],
            Vec::new(),
            Vec::new(),
        )
        .expect("peer anchor")
    }

    #[test]
    fn trusted_device_subject_projects_anchor_owner() {
        let anchor = anchor_with_device_owner();
        let owner = resolve_owner(
            "easynet:///r/test/device/dev-1",
            "easynet:///r/test/device/dev-1",
            &anchor,
        )
        .expect("anchor owner resolution");

        assert_eq!(owner.owner_user_id.as_deref(), Some("alice"));
        assert_eq!(
            owner.owner_ura.as_deref(),
            Some("easynet:///r/test/user/alice")
        );
    }

    #[test]
    fn paired_device_subject_does_not_project_credentials_owner() {
        let _home = HomeGuard::new();
        save_test_credentials();
        let anchor = empty_anchor();
        let owner = resolve_owner(
            "easynet:///r/test/device/dev-1",
            "easynet:///r/test/device/dev-1",
            &anchor,
        )
        .expect("ordinary policy owner resolution must ignore local credentials");

        assert!(owner.owner_user_id.is_none());
        assert!(owner.owner_ura.is_none());
        assert_eq!(
            owner.owner_source,
            crate::daemon::invocation::admission::decision::OwnerSource::Unresolved
        );
    }

    #[test]
    fn device_principal_projection_ignores_malformed_local_credentials() {
        let _home = HomeGuard::new();
        let state_dir = crate::daemon::persistence::config::state_dir();
        std::fs::create_dir_all(&state_dir).expect("create isolated state dir");
        std::fs::write(state_dir.join("credentials.json"), b"{")
            .expect("write malformed credentials");

        let principal = principal_for(
            TrustedAgentRole::Device,
            "easynet:///r/test/device/dev-1",
            &empty_anchor(),
        )
        .expect("ordinary policy principal projection must not read local credentials");

        assert_eq!(principal.kind, PrincipalKind::Device);
        assert_eq!(principal.caller_user_id, None);
    }

    #[test]
    fn local_device_owner_resolution_ignores_malformed_credentials() {
        let _home = HomeGuard::new();
        let state_dir = crate::daemon::persistence::config::state_dir();
        std::fs::create_dir_all(&state_dir).expect("create isolated state dir");
        std::fs::write(state_dir.join("credentials.json"), b"{")
            .expect("write malformed credentials");
        let anchor = empty_anchor();

        let owner = resolve_owner(
            "easynet:///r/test/device/dev-1",
            "easynet:///r/test/authority",
            &anchor,
        )
        .expect("ordinary policy owner resolution must not read local credentials");

        assert_eq!(
            owner.owner_source,
            crate::daemon::invocation::admission::decision::OwnerSource::Unresolved
        );
        assert!(owner.owner_user_id.is_none());
    }

    #[test]
    fn paired_device_ability_does_not_project_credentials_owner() {
        let _home = HomeGuard::new();
        save_test_credentials();
        let anchor = empty_anchor();
        let owner = resolve_owner(
            "easynet:///r/test/ability/device.dev-1.federation.advertise_abilities",
            "easynet:///r/test/authority",
            &anchor,
        )
        .expect("ordinary policy device ability owner resolution must ignore local credentials");

        assert!(owner.owner_user_id.is_none());
        assert_eq!(
            owner.owner_source,
            crate::daemon::invocation::admission::decision::OwnerSource::Unresolved
        );
    }

    #[test]
    fn local_authority_ability_without_trust_owner_stays_unresolved() {
        let _home = HomeGuard::new();
        let anchor = empty_anchor();
        let owner = resolve_owner(
            "easynet:///r/test/ability/authority.federation.discover",
            "easynet:///r/test/authority",
            &anchor,
        )
        .expect("authority owner resolution");

        assert!(owner.owner_user_id.is_none());
        assert!(owner.owner_ura.is_none());
        assert_eq!(
            owner.owner_source,
            crate::daemon::invocation::admission::decision::OwnerSource::Unresolved
        );
    }

    #[test]
    fn authority_ability_does_not_project_paired_device_credentials_owner() {
        let _home = HomeGuard::new();
        save_test_credentials();
        let anchor = empty_anchor();
        let owner = resolve_owner(
            "easynet:///r/test/ability/authority.federation.discover",
            "easynet:///r/test/authority",
            &anchor,
        )
        .expect("authority owner resolution should not fail for saved device credentials");

        assert!(owner.owner_user_id.is_none());
        assert!(owner.owner_ura.is_none());
        assert_eq!(
            owner.owner_source,
            crate::daemon::invocation::admission::decision::OwnerSource::Unresolved
        );
        assert!(
            owner
                .audit_warnings
                .iter()
                .any(|warning| warning.contains("no authoritative owner source")),
            "authority owner without explicit authority fact must stay unresolved: {owner:?}"
        );
    }

    #[test]
    fn authority_subject_does_not_project_paired_device_credentials_owner() {
        let _home = HomeGuard::new();
        save_test_credentials();
        let anchor = empty_anchor();
        let owner = resolve_owner(
            "easynet:///r/test/authority",
            "easynet:///r/test/authority",
            &anchor,
        )
        .expect("authority subject resolution should not fail for saved device credentials");

        assert!(owner.owner_user_id.is_none());
        assert!(owner.owner_ura.is_none());
        assert_eq!(
            owner.owner_source,
            crate::daemon::invocation::admission::decision::OwnerSource::Unresolved
        );
    }

    #[test]
    fn user_subject_projects_owner_policy_allow() {
        let _home = crate::cli::commands::test_support::HomeGuard::new();
        let stores = AccessControlStoreRegistry::ephemeral();
        let envelope = Envelope {
            caller: Some(identity("easynet:///r/test/user/alice")),
            callee: Some(identity("easynet:///r/test/agent/alice.worker")),
            subject: Some(SubjectIdentity {
                ura: "easynet:///r/test/user/alice".to_string(),
                profile: String::new(),
            }),
            ..Envelope::default()
        };
        let decision = AdmissionPolicyGate::verify(AdmissionPolicyContext {
            envelope: &envelope,
            ability: "meta.list_resources",
            action: AccessAction::Read,
            safe_read: true,
            trusted_role: TrustedAgentRole::User,
            daemon_ura: None,
            trust_anchor: &empty_anchor(),
            access_control_stores: &stores,
            canonical_hash: Some("sha256:test".to_string()),
            signature_key_id: None,
            verified_authority_id: None,
            rejector_ura: None,
        })
        .expect("owner user must pass policy");
        assert_eq!(decision.decision, PolicyDecisionOutcome::Allow);
    }

    #[test]
    fn hub_link_principal_gets_descriptor_safe_read_default() {
        let _home = crate::cli::commands::test_support::HomeGuard::new();
        let stores = AccessControlStoreRegistry::ephemeral();
        let envelope = Envelope {
            caller: Some(identity("easynet:///r/test/authority")),
            callee: Some(identity("easynet:///r/test/agent/alice.worker")),
            subject: Some(SubjectIdentity {
                ura: "easynet:///r/test/user/alice".to_string(),
                profile: String::new(),
            }),
            ..Envelope::default()
        };
        let decision = AdmissionPolicyGate::verify(AdmissionPolicyContext {
            envelope: &envelope,
            ability: "meta.list_resources",
            action: AccessAction::Read,
            safe_read: true,
            trusted_role: TrustedAgentRole::Hub,
            daemon_ura: None,
            trust_anchor: &empty_anchor(),
            access_control_stores: &stores,
            canonical_hash: Some("sha256:test".to_string()),
            signature_key_id: None,
            verified_authority_id: None,
            rejector_ura: None,
        })
        .expect("trusted hub-link principal may read descriptor-safe metadata");
        assert_eq!(decision.decision, PolicyDecisionOutcome::Allow);
        assert_eq!(decision.reason, PolicyDecisionReason::HubTokenReadAllow);
        assert_eq!(decision.principal_kind, PrincipalKind::Token);
        assert_eq!(
            decision.token_id.as_deref(),
            Some("easynet:///r/test/authority")
        );
    }

    #[test]
    fn realm_authority_can_read_descriptor_safe_device_metadata_before_owner_binding() {
        let _home = crate::cli::commands::test_support::HomeGuard::new();
        let stores = AccessControlStoreRegistry::ephemeral();
        let device = "easynet:///r/test/device/dev-1";
        let envelope = Envelope {
            caller: Some(identity("easynet:///r/test/authority")),
            callee: Some(identity(device)),
            subject: Some(SubjectIdentity {
                ura: device.to_string(),
                profile: String::new(),
            }),
            ..Envelope::default()
        };
        let decision = AdmissionPolicyGate::verify(AdmissionPolicyContext {
            envelope: &envelope,
            ability: "node.describe",
            action: AccessAction::Read,
            safe_read: true,
            trusted_role: TrustedAgentRole::Hub,
            daemon_ura: Some(device),
            trust_anchor: &empty_anchor(),
            access_control_stores: &stores,
            canonical_hash: Some("sha256:test".to_string()),
            signature_key_id: Some("ed25519:key".to_string()),
            verified_authority_id: None,
            rejector_ura: Some(device.to_string()),
        })
        .expect("realm Authority must read public Device runtime metadata");

        assert_eq!(decision.decision, PolicyDecisionOutcome::Allow);
        assert_eq!(decision.reason, PolicyDecisionReason::HubTokenReadAllow);
        assert_eq!(decision.owner_source, OwnerSource::Unresolved);
        assert!(decision.owner_user_id.is_none());
        assert_eq!(decision.caller_ura, "easynet:///r/test/authority");
        assert_eq!(decision.callee_ura, device);
    }

    #[test]
    fn realm_authority_public_device_read_stays_bound_to_local_device() {
        let _home = crate::cli::commands::test_support::HomeGuard::new();
        let stores = AccessControlStoreRegistry::ephemeral();
        let local_device = "easynet:///r/test/device/local-dev";
        let other_device = "easynet:///r/test/device/other-dev";
        let envelope = Envelope {
            caller: Some(identity("easynet:///r/test/authority")),
            callee: Some(identity(other_device)),
            subject: Some(SubjectIdentity {
                ura: other_device.to_string(),
                profile: String::new(),
            }),
            ..Envelope::default()
        };
        let err = AdmissionPolicyGate::verify(AdmissionPolicyContext {
            envelope: &envelope,
            ability: "node.describe",
            action: AccessAction::Read,
            safe_read: true,
            trusted_role: TrustedAgentRole::Hub,
            daemon_ura: Some(local_device),
            trust_anchor: &empty_anchor(),
            access_control_stores: &stores,
            canonical_hash: Some("sha256:test".to_string()),
            signature_key_id: Some("ed25519:key".to_string()),
            verified_authority_id: None,
            rejector_ura: Some(local_device.to_string()),
        })
        .expect_err("authority public read must not target a different local daemon owner");

        assert_eq!(err.code(), tonic::Code::PermissionDenied);
        assert!(
            err.message().contains("\"reason\":\"OWNER_UNRESOLVED\""),
            "expected owner unresolved outside local daemon scope, got: {}",
            err.message()
        );
    }

    #[test]
    fn local_authority_self_read_enters_policy_without_user_owner() {
        let _home = crate::cli::commands::test_support::HomeGuard::new();
        let stores = AccessControlStoreRegistry::ephemeral();
        let authority = "easynet:///r/test/authority";
        let subject = crate::core::ura::owner_ability_ura(authority, "federation.discover")
            .expect("authority ability subject");
        let envelope = Envelope {
            caller: Some(identity(authority)),
            callee: Some(identity(authority)),
            subject: Some(SubjectIdentity {
                ura: subject,
                profile: String::new(),
            }),
            ..Envelope::default()
        };
        let decision = AdmissionPolicyGate::verify(AdmissionPolicyContext {
            envelope: &envelope,
            ability: "federation.discover",
            action: AccessAction::Read,
            safe_read: true,
            trusted_role: TrustedAgentRole::Hub,
            daemon_ura: Some(authority),
            trust_anchor: &empty_anchor(),
            access_control_stores: &stores,
            canonical_hash: Some("sha256:test".to_string()),
            signature_key_id: None,
            verified_authority_id: None,
            rejector_ura: Some(authority.to_string()),
        })
        .expect("local authority must read its descriptor-bound system catalog");

        assert_eq!(decision.decision, PolicyDecisionOutcome::Allow);
        assert_eq!(decision.reason, PolicyDecisionReason::HubTokenReadAllow);
        assert!(decision.owner_user_id.is_none());
        assert_eq!(decision.caller_ura, authority);
        assert_eq!(decision.callee_ura, authority);
    }

    #[test]
    fn local_authority_descriptor_ref_self_read_enters_policy_without_user_owner() {
        let _home = crate::cli::commands::test_support::HomeGuard::new();
        let stores = AccessControlStoreRegistry::ephemeral();
        let authority = "easynet:///r/hub/authority";
        let subject = crate::core::ura::owner_ability_ura(authority, "federation.discover")
            .expect("authority ability subject");
        let descriptor_ref =
            crate::daemon::axon_bridge::descriptor_ref::catalog_descriptor_ref_for_wire(
                authority,
                "federation.discover",
                crate::daemon::ability::CallMode::Rpc,
            )
            .expect("descriptor ref");
        let envelope = Envelope {
            caller: Some(identity(authority)),
            callee: Some(identity(authority)),
            subject: Some(SubjectIdentity {
                ura: subject,
                profile: String::new(),
            }),
            ..Envelope::default()
        };
        let decision = AdmissionPolicyGate::verify(AdmissionPolicyContext {
            envelope: &envelope,
            ability: &descriptor_ref,
            action: AccessAction::Read,
            safe_read: true,
            trusted_role: TrustedAgentRole::Hub,
            daemon_ura: Some(authority),
            trust_anchor: &empty_anchor(),
            access_control_stores: &stores,
            canonical_hash: Some("sha256:test".to_string()),
            signature_key_id: None,
            verified_authority_id: None,
            rejector_ura: Some(authority.to_string()),
        })
        .expect("local authority must read descriptor-bound system catalog");

        assert_eq!(decision.decision, PolicyDecisionOutcome::Allow);
        assert_eq!(decision.reason, PolicyDecisionReason::HubTokenReadAllow);
        assert_eq!(
            decision.ability_ura,
            "easynet:///r/hub/ability/authority.federation.discover"
        );
    }

    #[test]
    fn local_authority_self_stream_enters_policy_without_user_session_authority() {
        let _home = crate::cli::commands::test_support::HomeGuard::new();
        let stores = AccessControlStoreRegistry::ephemeral();
        let authority = "easynet:///r/test/authority";
        let subject = crate::core::ura::resource_dot_ura(
            "test",
            "authority",
            "invoke/federation.subscribe_directory_v2",
        );
        let envelope = Envelope {
            caller: Some(identity(authority)),
            callee: Some(identity(authority)),
            subject: Some(SubjectIdentity {
                ura: subject,
                profile: String::new(),
            }),
            ..Envelope::default()
        };
        let decision = AdmissionPolicyGate::verify(AdmissionPolicyContext {
            envelope: &envelope,
            ability: "federation.subscribe_directory_v2",
            action: AccessAction::Stream,
            safe_read: false,
            trusted_role: TrustedAgentRole::Hub,
            daemon_ura: Some(authority),
            trust_anchor: &empty_anchor(),
            access_control_stores: &stores,
            canonical_hash: Some("sha256:test".to_string()),
            signature_key_id: None,
            verified_authority_id: None,
            rejector_ura: Some(authority.to_string()),
        })
        .expect("local authority must stream its own directory events without a user authority");

        assert_eq!(decision.decision, PolicyDecisionOutcome::Allow);
        assert_eq!(decision.reason, PolicyDecisionReason::ExplicitGrantAllow);
        assert!(decision.owner_user_id.is_none());
        assert_eq!(
            decision.ability_ura,
            "easynet:///r/test/ability/authority.federation.subscribe_directory_v2"
        );
    }

    #[test]
    fn local_authority_self_manage_can_revoke_owned_device_directory_entry() {
        let _home = crate::cli::commands::test_support::HomeGuard::new();
        let stores = AccessControlStoreRegistry::ephemeral();
        let authority = "easynet:///r/test/authority";
        let envelope = Envelope {
            caller: Some(identity(authority)),
            callee: Some(identity(authority)),
            subject: Some(SubjectIdentity {
                ura: "easynet:///r/test/device/dev-1".to_string(),
                profile: String::new(),
            }),
            ..Envelope::default()
        };
        let decision = AdmissionPolicyGate::verify(AdmissionPolicyContext {
            envelope: &envelope,
            ability: "federation.revoke",
            action: AccessAction::Manage,
            safe_read: false,
            trusted_role: TrustedAgentRole::Hub,
            daemon_ura: Some(authority),
            trust_anchor: &anchor_with_device_owner(),
            access_control_stores: &stores,
            canonical_hash: Some("sha256:test".to_string()),
            signature_key_id: None,
            verified_authority_id: None,
            rejector_ura: Some(authority.to_string()),
        })
        .expect("realm authority must manage its own federation directory");

        assert_eq!(decision.decision, PolicyDecisionOutcome::Allow);
        assert_eq!(decision.reason, PolicyDecisionReason::ExplicitGrantAllow);
        assert_eq!(decision.owner_user_id.as_deref(), Some("alice"));
        assert_eq!(
            decision.ability_ura,
            "easynet:///r/test/ability/authority.federation.revoke"
        );
    }

    #[test]
    fn device_self_publication_manage_can_advertise_authority_scoped_abilities_without_user_owner()
    {
        let _home = crate::cli::commands::test_support::HomeGuard::new();
        let stores = AccessControlStoreRegistry::ephemeral();
        let authority = "easynet:///r/test/authority";
        let device = "easynet:///r/test/device/dev-1";
        let envelope = Envelope {
            caller: Some(identity(device)),
            callee: Some(identity(authority)),
            subject: Some(SubjectIdentity {
                ura: device.to_string(),
                profile: String::new(),
            }),
            ..Envelope::default()
        };
        let decision = AdmissionPolicyGate::verify(AdmissionPolicyContext {
            envelope: &envelope,
            ability: crate::daemon::invocation::dispatch::federation_wrappers::ABILITY_FEDERATION_ADVERTISE_ABILITIES,
            action: AccessAction::Manage,
            safe_read: false,
            trusted_role: TrustedAgentRole::Device,
            daemon_ura: Some(authority),
            trust_anchor: &empty_anchor(),
            access_control_stores: &stores,
            canonical_hash: Some("sha256:test".to_string()),
            signature_key_id: Some("ed25519:device".to_string()),
            verified_authority_id: None,
            rejector_ura: Some(authority.to_string()),
        })
        .expect("device must advertise its own authority-scoped publication after federation join");

        assert_eq!(decision.decision, PolicyDecisionOutcome::Allow);
        assert_eq!(decision.reason, PolicyDecisionReason::ExplicitGrantAllow);
        assert!(decision.owner_user_id.is_none());
        assert_eq!(decision.owner_source, OwnerSource::Unresolved);
        assert_eq!(decision.principal_kind, PrincipalKind::Device);
        assert_eq!(decision.caller_ura, device);
        assert_eq!(decision.subject_ura, device);
        assert_eq!(decision.callee_ura, authority);
    }

    #[test]
    fn device_self_session_stream_can_open_authority_carrier_without_user_owner() {
        let _home = crate::cli::commands::test_support::HomeGuard::new();
        let stores = AccessControlStoreRegistry::ephemeral();
        let authority = "easynet:///r/test/authority";
        let device = "easynet:///r/test/device/dev-1";
        let envelope = Envelope {
            caller: Some(identity(device)),
            callee: Some(identity(authority)),
            subject: Some(SubjectIdentity {
                ura: device.to_string(),
                profile: String::new(),
            }),
            ..Envelope::default()
        };
        let decision = AdmissionPolicyGate::verify(AdmissionPolicyContext {
            envelope: &envelope,
            ability: crate::daemon::invocation::bidi::session_initiator::ABILITY_SESSION_OPEN,
            action: AccessAction::Stream,
            safe_read: false,
            trusted_role: TrustedAgentRole::Device,
            daemon_ura: Some(authority),
            trust_anchor: &empty_anchor(),
            access_control_stores: &stores,
            canonical_hash: Some("sha256:test".to_string()),
            signature_key_id: Some("ed25519:device".to_string()),
            verified_authority_id: None,
            rejector_ura: Some(authority.to_string()),
        })
        .expect("device must open its own authority session carrier after federation join");

        assert_eq!(decision.decision, PolicyDecisionOutcome::Allow);
        assert_eq!(decision.reason, PolicyDecisionReason::ExplicitGrantAllow);
        assert!(decision.owner_user_id.is_none());
        assert_eq!(decision.owner_source, OwnerSource::Unresolved);
        assert_eq!(decision.principal_kind, PrincipalKind::Device);
        assert_eq!(
            decision.ability_ura,
            "easynet:///r/test/ability/authority.session.open"
        );
    }

    #[test]
    fn local_authority_self_manage_rejects_cross_realm_subject() {
        let _home = crate::cli::commands::test_support::HomeGuard::new();
        let stores = AccessControlStoreRegistry::ephemeral();
        let authority = "easynet:///r/test/authority";
        let envelope = Envelope {
            caller: Some(identity(authority)),
            callee: Some(identity(authority)),
            subject: Some(SubjectIdentity {
                ura: "easynet:///r/other/device/dev-1".to_string(),
                profile: String::new(),
            }),
            ..Envelope::default()
        };
        let err = AdmissionPolicyGate::verify(AdmissionPolicyContext {
            envelope: &envelope,
            ability: "federation.revoke",
            action: AccessAction::Manage,
            safe_read: false,
            trusted_role: TrustedAgentRole::Hub,
            daemon_ura: Some(authority),
            trust_anchor: &empty_anchor(),
            access_control_stores: &stores,
            canonical_hash: Some("sha256:test".to_string()),
            signature_key_id: None,
            verified_authority_id: None,
            rejector_ura: Some(authority.to_string()),
        })
        .expect_err("authority self-manage must stay realm-bounded");

        assert_eq!(err.code(), tonic::Code::PermissionDenied);
        assert!(
            err.message().contains("\"reason\":\"OWNER_UNRESOLVED\""),
            "expected owner unresolved after authority self-manage gate rejects cross-realm subject, got: {}",
            err.message()
        );
    }

    #[test]
    fn hub_link_principal_cannot_stream_without_grant() {
        let _home = crate::cli::commands::test_support::HomeGuard::new();
        let stores = AccessControlStoreRegistry::ephemeral();
        let envelope = Envelope {
            caller: Some(identity("easynet:///r/test/authority")),
            callee: Some(identity("easynet:///r/test/agent/alice.worker")),
            subject: Some(SubjectIdentity {
                ura: "easynet:///r/test/user/alice".to_string(),
                profile: String::new(),
            }),
            ..Envelope::default()
        };
        let err = AdmissionPolicyGate::verify(AdmissionPolicyContext {
            envelope: &envelope,
            ability: "remote_desktop.attach",
            action: AccessAction::Stream,
            safe_read: false,
            trusted_role: TrustedAgentRole::Hub,
            daemon_ura: None,
            trust_anchor: &empty_anchor(),
            access_control_stores: &stores,
            canonical_hash: Some("sha256:test".to_string()),
            signature_key_id: None,
            verified_authority_id: None,
            rejector_ura: None,
        })
        .expect_err("trusted hub-link principal cannot stream without grant");
        assert_eq!(err.code(), tonic::Code::PermissionDenied);
        assert!(
            err.message().contains("\"reason\":\"TOKEN_SCOPE_DENIED\""),
            "expected token scope denial, got: {}",
            err.message()
        );
    }

    #[test]
    fn local_hub_allows_forwarding_to_trusted_remote_owner_realm() {
        let _home = crate::cli::commands::test_support::HomeGuard::new();
        let stores = AccessControlStoreRegistry::ephemeral();
        let envelope = Envelope {
            caller: Some(identity("easynet:///r/local/device/caller")),
            callee: Some(identity("easynet:///r/peer/device/callee")),
            subject: Some(SubjectIdentity {
                ura: "easynet:///r/peer/resource/user.bob/invoke/shell.run".to_string(),
                profile: String::new(),
            }),
            ..Envelope::default()
        };
        let decision = AdmissionPolicyGate::verify(AdmissionPolicyContext {
            envelope: &envelope,
            ability: "shell.run",
            action: AccessAction::Invoke,
            safe_read: false,
            trusted_role: TrustedAgentRole::Device,
            daemon_ura: Some("easynet:///r/local/authority"),
            trust_anchor: &anchor_with_peer_realm(),
            access_control_stores: &stores,
            canonical_hash: Some("sha256:test".to_string()),
            signature_key_id: None,
            verified_authority_id: None,
            rejector_ura: Some("easynet:///r/local/authority".to_string()),
        })
        .expect("local hub may forward to an operator-pinned peer realm");

        assert_eq!(decision.decision, PolicyDecisionOutcome::Allow);
        assert_eq!(
            decision.reason,
            PolicyDecisionReason::FederationForwardAllow
        );
        assert_eq!(decision.owner_user_id.as_deref(), Some("bob"));
        assert_eq!(
            decision.rejector_ura.as_deref(),
            Some("easynet:///r/local/authority")
        );
    }

    #[test]
    fn local_hub_does_not_forward_to_untrusted_remote_owner_realm() {
        let _home = crate::cli::commands::test_support::HomeGuard::new();
        let stores = AccessControlStoreRegistry::ephemeral();
        let envelope = Envelope {
            caller: Some(identity("easynet:///r/local/device/caller")),
            callee: Some(identity("easynet:///r/peer/device/callee")),
            subject: Some(SubjectIdentity {
                ura: "easynet:///r/peer/resource/user.bob/invoke/shell.run".to_string(),
                profile: String::new(),
            }),
            ..Envelope::default()
        };
        let err = AdmissionPolicyGate::verify(AdmissionPolicyContext {
            envelope: &envelope,
            ability: "shell.run",
            action: AccessAction::Invoke,
            safe_read: false,
            trusted_role: TrustedAgentRole::Device,
            daemon_ura: Some("easynet:///r/local/authority"),
            trust_anchor: &empty_anchor(),
            access_control_stores: &stores,
            canonical_hash: Some("sha256:test".to_string()),
            signature_key_id: None,
            verified_authority_id: None,
            rejector_ura: Some("easynet:///r/local/authority".to_string()),
        })
        .expect_err("untrusted remote realm cannot use the forward allow state");

        assert_eq!(err.code(), tonic::Code::PermissionDenied);
        assert!(
            err.message()
                .contains("\"reason\":\"NON_INTERACTIVE_DENY\""),
            "expected ordinary policy denial without peer trust, got: {}",
            err.message()
        );
    }

    #[test]
    fn policy_ability_projection_accepts_descriptor_ref_without_rewrapping() {
        let callee = "easynet:///r/test/authority";
        let ability_ura = crate::core::ura::owner_ability_ura(callee, "identity.register_pubkey")
            .expect("hub ability URA");
        let descriptor_binding =
            crate::daemon::axon_bridge::descriptor_ref::descriptor_binding_for_wire(
                crate::daemon::ability::DEFAULT_ABILITY_DESCRIPTOR_VERSION,
                [0x44; 32],
                "manage",
            )
            .expect("test descriptor binding");
        let descriptor_ref = format!("{ability_ura}@{descriptor_binding}");

        let projected = ability_ura_for(callee, &descriptor_ref)
            .expect("descriptor ref projects to ability URA");

        assert_eq!(projected, ability_ura);
        assert!(
            !projected.contains("@"),
            "policy input ability_ura must not carry descriptor version"
        );
        assert!(
            !projected.contains("hub.easynet:///"),
            "descriptor ref must not be treated as a public ability name"
        );
    }
}
