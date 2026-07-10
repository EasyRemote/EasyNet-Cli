// EasyNet CLI — RFC-014 admission policy gate
// ===========================================
//
// Bridges signed invocation facts into the pure RFC-014 PolicyEngine.
// Signature verification stays in AdmissionFacade; this adapter owns only
// runtime policy facts: principal, owner, action, safe-read, and grants.

use chrono::Utc;
use tonic::Status;

use easynet_axon::pb::axon::v1::Envelope;

use crate::core::ura::{parse_ura, AbilityOwner, URAKind};
use crate::daemon::ability::catalog::catalog_metadata;
use crate::daemon::invocation::admission::decision::{
    AccessAction, OwnerResolution, PolicyDecision, PolicyDecisionOutcome, PrincipalKind, TokenClass,
};
use crate::daemon::invocation::admission::owner_resolution::{
    OwnerFact, OwnerResolutionInput, OwnerResolver,
};
use crate::daemon::invocation::admission::policy_engine::{PolicyEngine, PolicyInput};
use crate::daemon::persistence::access_control::AccessControlStore;
use crate::daemon::trust::anchor::{RealmTrustAnchor, TrustedAgentRole};

#[derive(Debug, Clone)]
pub struct AdmissionPolicyContext<'a> {
    pub envelope: &'a Envelope,
    pub ability: &'a str,
    pub action: AccessAction,
    pub trusted_role: TrustedAgentRole,
    pub daemon_ura: Option<&'a str>,
    pub trust_anchor: &'a RealmTrustAnchor,
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
        let owner = resolve_owner(
            &subject_ura,
            &callee_ura,
            context.daemon_ura,
            context.trust_anchor,
        );
        let principal = principal_for(context.trusted_role, &caller_ura, context.trust_anchor);
        let grants = match owner.owner_user_id.as_deref() {
            Some(owner_user_id) => AccessControlStore::open_or_create(owner_user_id)
                .map(|store| store.grants())
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
            safe_read: safe_read(context.ability, context.action),
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
) -> PrincipalProjection {
    match role {
        TrustedAgentRole::User => {
            let user_id = user_id_from_ura(caller_ura).unwrap_or_else(|| caller_ura.to_string());
            PrincipalProjection {
                kind: PrincipalKind::User,
                id: user_id.clone(),
                token_id: None,
                token_class: None,
                caller_user_id: Some(user_id),
            }
        }
        TrustedAgentRole::Hub => PrincipalProjection {
            kind: PrincipalKind::Token,
            id: caller_ura.to_string(),
            token_id: Some(caller_ura.to_string()),
            token_class: Some(TokenClass::HubLink),
            caller_user_id: None,
        },
        TrustedAgentRole::Device => {
            let owner_user_id = trust_anchor
                .lookup_principal_owner(caller_ura)
                .map(|owner| owner.owner_user_id.clone())
                .or_else(|| local_device_owner_user_id(caller_ura));
            PrincipalProjection {
                kind: PrincipalKind::Device,
                id: caller_ura.to_string(),
                token_id: Some(caller_ura.to_string()),
                token_class: Some(TokenClass::DevicePairing),
                caller_user_id: owner_user_id,
            }
        }
        TrustedAgentRole::Backend => PrincipalProjection {
            kind: PrincipalKind::Service,
            id: caller_ura.to_string(),
            token_id: None,
            token_class: None,
            caller_user_id: None,
        },
    }
}

pub(crate) fn resolve_owner(
    subject_ura: &str,
    callee_ura: &str,
    daemon_ura: Option<&str>,
    trust_anchor: &RealmTrustAnchor,
) -> OwnerResolution {
    OwnerResolver::resolve(&OwnerResolutionInput {
        subject: owner_fact_from_ura(subject_ura, daemon_ura, trust_anchor),
        callee: owner_fact_from_ura(callee_ura, daemon_ura, trust_anchor),
        device: owner_fact_from_trust_anchor(callee_ura, trust_anchor)
            .or_else(|| owner_fact_from_local_device(callee_ura, daemon_ura)),
        session: None,
    })
}

fn owner_fact_from_ura(
    ura: &str,
    daemon_ura: Option<&str>,
    trust_anchor: &RealmTrustAnchor,
) -> Option<OwnerFact> {
    let parsed = parse_ura(ura).ok()?;
    match parsed.kind {
        URAKind::User => parsed
            .user_id()
            .map(|user_id| OwnerFact::user(user_id, ura.to_string())),
        URAKind::Agent => parsed.agent_ids().map(|(user_id, _)| {
            OwnerFact::user(user_id, crate::core::ura::user_ura(&parsed.realm, user_id))
        }),
        URAKind::Ability => parsed.ability().and_then(|ability| match ability.owner {
            AbilityOwner::Agent { user_id, .. } => Some(OwnerFact::user(
                user_id.clone(),
                crate::core::ura::user_ura(&parsed.realm, &user_id),
            )),
            AbilityOwner::Device { device_id } => owner_fact_from_trust_anchor(
                &crate::core::ura::device_ura(&parsed.realm, &device_id),
                trust_anchor,
            )
            .or_else(|| {
                owner_fact_from_local_device(
                    &crate::core::ura::device_ura(&parsed.realm, &device_id),
                    daemon_ura,
                )
            }),
            AbilityOwner::Hub => {
                owner_fact_from_local_device(&crate::core::ura::hub_ura(&parsed.realm), daemon_ura)
            }
        }),
        URAKind::Device | URAKind::Hub => owner_fact_from_trust_anchor(ura, trust_anchor)
            .or_else(|| owner_fact_from_local_device(ura, daemon_ura)),
        URAKind::Resource => resource_owner_user_id(&parsed).map(|user_id| {
            OwnerFact::user(
                user_id.clone(),
                crate::core::ura::user_ura(&parsed.realm, &user_id),
            )
        }),
        _ => None,
    }
}

fn owner_fact_from_trust_anchor(ura: &str, trust_anchor: &RealmTrustAnchor) -> Option<OwnerFact> {
    let owner = trust_anchor.lookup_principal_owner(ura)?;
    Some(OwnerFact::user(
        owner.owner_user_id.clone(),
        owner.owner_ura.clone(),
    ))
}

fn owner_fact_from_local_device(ura: &str, daemon_ura: Option<&str>) -> Option<OwnerFact> {
    let parsed = parse_ura(ura).ok()?;
    let credentials = crate::daemon::persistence::config::load_credentials().ok()?;
    let is_local_identity = match parsed.kind {
        URAKind::Device => {
            parsed.realm == credentials.realm
                && parsed
                    .device_id()
                    .is_some_and(|device_id| device_id == credentials.node_id.as_str())
        }
        URAKind::Hub => Some(ura) == daemon_ura || parsed.realm == credentials.realm,
        _ => Some(ura) == daemon_ura,
    };
    if !is_local_identity {
        return None;
    }
    let user_id = credentials.user_id().ok()?.to_string();
    Some(OwnerFact::user(
        user_id.clone(),
        crate::core::ura::user_ura(&credentials.realm, &user_id),
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

fn local_device_owner_user_id(caller_ura: &str) -> Option<String> {
    let parsed = parse_ura(caller_ura).ok()?;
    if parsed.kind != URAKind::Device {
        return None;
    }
    let credentials = crate::daemon::persistence::config::load_credentials().ok()?;
    if parsed.realm != credentials.realm {
        return None;
    }
    let device_id = parsed.device_id()?;
    if device_id != credentials.node_id.as_str() {
        return None;
    }
    credentials.user_id().ok().map(ToString::to_string)
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

pub(crate) fn safe_read(ability: &str, action: AccessAction) -> bool {
    if action != AccessAction::Read {
        return false;
    }
    catalog_metadata::safe_read_eligible_for_name(ability)
}

fn agent_ura(
    identity: Option<&easynet_axon::pb::axon::v1::AgentIdentity>,
    role: &str,
) -> Result<String, Status> {
    identity
        .map(|identity| identity.ura.trim())
        .filter(|ura| !ura.is_empty())
        .map(ToString::to_string)
        .ok_or_else(|| Status::invalid_argument(format!("Invoke envelope missing {role} URA")))
}

fn subject_ura(
    identity: Option<&easynet_axon::pb::axon::v1::SubjectIdentity>,
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
    use crate::daemon::invocation::admission::decision::PolicyDecisionReason;
    use crate::daemon::persistence::config::{save_credentials, Credentials};
    use crate::daemon::trust::anchor::TrustedPrincipalOwner;
    use easynet_axon::pb::axon::v1::{AgentIdentity, SubjectIdentity};

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

    #[test]
    fn trusted_device_subject_projects_anchor_owner() {
        let anchor = anchor_with_device_owner();
        let owner = resolve_owner(
            "easynet:///r/test/device/dev-1",
            "easynet:///r/test/hub",
            Some("easynet:///r/test/hub"),
            &anchor,
        );

        assert_eq!(owner.owner_user_id.as_deref(), Some("alice"));
        assert_eq!(
            owner.owner_ura.as_deref(),
            Some("easynet:///r/test/user/alice")
        );
    }

    #[test]
    fn paired_device_subject_projects_credentials_owner() {
        let _home = HomeGuard::new();
        save_test_credentials();
        let anchor = empty_anchor();
        let owner = resolve_owner(
            "easynet:///r/test/device/dev-1",
            "easynet:///r/test/hub",
            Some("easynet:///r/test/hub"),
            &anchor,
        );

        assert_eq!(owner.owner_user_id.as_deref(), Some("alice"));
        assert_eq!(
            owner.owner_ura.as_deref(),
            Some("easynet:///r/test/user/alice")
        );
    }

    #[test]
    fn paired_device_ability_projects_credentials_owner() {
        let _home = HomeGuard::new();
        save_test_credentials();
        let anchor = empty_anchor();
        let owner = resolve_owner(
            "easynet:///r/test/ability/device.dev-1.federation.advertise_abilities",
            "easynet:///r/test/hub",
            Some("easynet:///r/test/hub"),
            &anchor,
        );

        assert_eq!(owner.owner_user_id.as_deref(), Some("alice"));
    }

    #[test]
    fn user_subject_projects_owner_policy_allow() {
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
            trusted_role: TrustedAgentRole::User,
            daemon_ura: None,
            trust_anchor: &empty_anchor(),
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
        let envelope = Envelope {
            caller: Some(identity("easynet:///r/test/hub")),
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
            trusted_role: TrustedAgentRole::Hub,
            daemon_ura: None,
            trust_anchor: &empty_anchor(),
            canonical_hash: Some("sha256:test".to_string()),
            signature_key_id: None,
            verified_authority_id: None,
            rejector_ura: None,
        })
        .expect("trusted hub-link principal may read descriptor-safe metadata");
        assert_eq!(decision.decision, PolicyDecisionOutcome::Allow);
        assert_eq!(decision.reason, PolicyDecisionReason::HubTokenReadAllow);
        assert_eq!(decision.principal_kind, PrincipalKind::Token);
        assert_eq!(decision.token_id.as_deref(), Some("easynet:///r/test/hub"));
    }

    #[test]
    fn hub_link_principal_cannot_stream_without_grant() {
        let envelope = Envelope {
            caller: Some(identity("easynet:///r/test/hub")),
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
            trusted_role: TrustedAgentRole::Hub,
            daemon_ura: None,
            trust_anchor: &empty_anchor(),
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
    fn private_read_is_not_hub_safe_read_default() {
        assert!(
            safe_read("meta.list_resources", AccessAction::Read),
            "descriptor-safe metadata stays hub safe-read eligible"
        );
        assert!(
            !safe_read("terminal.list", AccessAction::Read),
            "terminal.list exposes session topology and handles"
        );
        assert!(
            !safe_read("context.clipboard.get", AccessAction::Read),
            "context reads expose private user/device state"
        );
        assert!(
            !safe_read("fs.list", AccessAction::Read),
            "filesystem topology is private read, not hub safe-read"
        );
    }

    #[test]
    fn policy_ability_projection_accepts_descriptor_ref_without_rewrapping() {
        let callee = "easynet:///r/test/hub";
        let ability_ura = crate::core::ura::owner_ability_ura(callee, "identity.register_pubkey")
            .expect("hub ability URA");
        let descriptor_ref = format!("{ability_ura}@1.0.0");

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
