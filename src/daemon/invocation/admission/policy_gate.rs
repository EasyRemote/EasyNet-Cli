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
    local_device_owner_fact, OwnerFact, OwnerResolutionInput, OwnerResolver,
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
        let owner = resolve_owner(
            &subject_ura,
            &callee_ura,
            context.daemon_ura,
            context.trust_anchor,
        );
        let principal = principal_for(context.trusted_role, &caller_ura, context.trust_anchor);
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
                .map(|owner| OwnerFact::user(owner.owner_user_id.clone(), owner.owner_ura.clone()))
                .or_else(|| local_device_owner_fact(caller_ura))
                .and_then(|owner| owner.owner_user_id);
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
    if let Some(owner) = owner_fact_from_trust_anchor(ura, trust_anchor) {
        return Some(owner);
    }
    let parsed = parse_ura(ura).ok()?;
    match parsed.kind {
        URAKind::User => parsed
            .user_id()
            .map(|user_id| OwnerFact::user(user_id, ura.to_string())),
        URAKind::Agent => parsed.agent_ids().map(|(user_id, _)| {
            OwnerFact::user(user_id, crate::core::ura::user_ura(&parsed.realm, user_id))
        }),
        URAKind::Ability => parsed.ability().and_then(|ability| match ability.owner {
            AbilityOwner::Agent { user_id, agent_id } => owner_fact_from_trust_anchor(
                &crate::core::ura::agent_ura(&parsed.realm, &user_id, &agent_id),
                trust_anchor,
            )
            .or_else(|| {
                Some(OwnerFact::user(
                    user_id.clone(),
                    crate::core::ura::user_ura(&parsed.realm, &user_id),
                ))
            }),
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
            AbilityOwner::Authority => {
                owner_fact_from_local_device(&crate::core::ura::hub_ura(&parsed.realm), daemon_ura)
            }
        }),
        URAKind::Device | URAKind::Authority => owner_fact_from_trust_anchor(ura, trust_anchor)
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
        URAKind::Authority => Some(ura) == daemon_ura || parsed.realm == credentials.realm,
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
    use crate::daemon::invocation::admission::decision::PolicyDecisionReason;
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
                owner_username: Some("alice".to_string()),
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
            "easynet:///r/test/authority",
            Some("easynet:///r/test/authority"),
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
            "easynet:///r/test/authority",
            Some("easynet:///r/test/authority"),
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
            "easynet:///r/test/authority",
            Some("easynet:///r/test/authority"),
            &anchor,
        );

        assert_eq!(owner.owner_user_id.as_deref(), Some("alice"));
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
