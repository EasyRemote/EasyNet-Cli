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
    AccessAction, OwnerResolution, PolicyDecision, PolicyDecisionOutcome, PrincipalKind,
};
use crate::daemon::invocation::admission::owner_resolution::{
    OwnerFact, OwnerResolutionInput, OwnerResolver,
};
use crate::daemon::invocation::admission::policy_engine::{PolicyEngine, PolicyInput};
use crate::daemon::persistence::access_control::AccessControlStore;
use crate::daemon::trust::anchor::TrustedAgentRole;

#[derive(Debug, Clone)]
pub struct AdmissionPolicyContext<'a> {
    pub envelope: &'a Envelope,
    pub ability: &'a str,
    pub action: AccessAction,
    pub trusted_role: TrustedAgentRole,
    pub daemon_ura: Option<&'a str>,
    pub canonical_hash: Option<String>,
    pub signature_key_id: Option<String>,
    pub authority_proof_id: Option<String>,
    pub rejector_ura: Option<String>,
}

pub struct AdmissionPolicyGate;

impl AdmissionPolicyGate {
    pub fn verify(context: AdmissionPolicyContext<'_>) -> Result<PolicyDecision, Status> {
        let caller_ura = agent_ura(context.envelope.caller.as_ref(), "caller")?;
        let callee_ura = agent_ura(context.envelope.callee.as_ref(), "callee")?;
        let subject_ura = subject_ura(context.envelope.subject.as_ref())?;
        let ability_ura = ability_ura_for(&callee_ura, context.ability)?;
        let owner = resolve_owner(&subject_ura, &callee_ura, context.daemon_ura);
        let principal = principal_for(context.trusted_role, &caller_ura);
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
            token_id: None,
            token_class: None,
            callee_ura,
            subject_ura,
            ability_ura,
            action: context.action,
            safe_read: safe_read(context.ability, context.action),
            interactive_context_available: false,
            canonical_hash: context.canonical_hash,
            signature_key_id: context.signature_key_id,
            authority_proof_id: context.authority_proof_id,
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
struct PrincipalProjection {
    kind: PrincipalKind,
    id: String,
    caller_user_id: Option<String>,
}

fn principal_for(role: TrustedAgentRole, caller_ura: &str) -> PrincipalProjection {
    match role {
        TrustedAgentRole::User => {
            let user_id = user_id_from_ura(caller_ura).unwrap_or_else(|| caller_ura.to_string());
            PrincipalProjection {
                kind: PrincipalKind::User,
                id: user_id.clone(),
                caller_user_id: Some(user_id),
            }
        }
        TrustedAgentRole::Hub => PrincipalProjection {
            kind: PrincipalKind::Hub,
            id: caller_ura.to_string(),
            caller_user_id: None,
        },
        TrustedAgentRole::Device => PrincipalProjection {
            kind: PrincipalKind::Device,
            id: caller_ura.to_string(),
            caller_user_id: None,
        },
        TrustedAgentRole::Backend => PrincipalProjection {
            kind: PrincipalKind::Service,
            id: caller_ura.to_string(),
            caller_user_id: None,
        },
    }
}

fn resolve_owner(subject_ura: &str, callee_ura: &str, daemon_ura: Option<&str>) -> OwnerResolution {
    OwnerResolver::resolve(&OwnerResolutionInput {
        subject: owner_fact_from_ura(subject_ura, daemon_ura),
        callee: owner_fact_from_ura(callee_ura, daemon_ura),
        device: owner_fact_from_local_device(callee_ura, daemon_ura),
        session: None,
    })
}

fn owner_fact_from_ura(ura: &str, daemon_ura: Option<&str>) -> Option<OwnerFact> {
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
            AbilityOwner::Device { .. } | AbilityOwner::Hub => {
                owner_fact_from_local_device(ura, daemon_ura)
            }
        }),
        URAKind::Device | URAKind::Hub => owner_fact_from_local_device(ura, daemon_ura),
        URAKind::Resource => resource_owner_user_id(&parsed).map(|user_id| {
            OwnerFact::user(
                user_id.clone(),
                crate::core::ura::user_ura(&parsed.realm, &user_id),
            )
        }),
        _ => None,
    }
}

fn owner_fact_from_local_device(ura: &str, daemon_ura: Option<&str>) -> Option<OwnerFact> {
    let daemon_ura = daemon_ura?;
    if ura != daemon_ura && parse_ura(ura).ok()?.kind != URAKind::Hub {
        return None;
    }
    let credentials = crate::daemon::persistence::config::load_credentials().ok()?;
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

fn ability_ura_for(callee_ura: &str, ability: &str) -> Result<String, Status> {
    crate::core::ura::owner_ability_ura(callee_ura, ability).ok_or_else(|| {
        Status::invalid_argument(format!(
            "ABILITY_URA_PROJECTION_FAILED: callee={callee_ura} ability={ability}"
        ))
    })
}

fn safe_read(ability: &str, action: AccessAction) -> bool {
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
    use easynet_axon::pb::axon::v1::{AgentIdentity, SubjectIdentity};

    fn identity(ura: &str) -> AgentIdentity {
        AgentIdentity {
            ura: ura.to_string(),
            profile: String::new(),
        }
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
            canonical_hash: Some("sha256:test".to_string()),
            signature_key_id: None,
            authority_proof_id: None,
            rejector_ura: None,
        })
        .expect("owner user must pass policy");
        assert_eq!(decision.decision, PolicyDecisionOutcome::Allow);
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
}
