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
use crate::daemon::invocation::admission::device_caller::{
    admitted_device_policy_purpose, DeviceCallerPolicyScope, DeviceCallerPurpose,
    VerifiedDeviceInvocationPurpose,
};
use crate::daemon::invocation::admission::owner_resolution::{
    OwnerFact, OwnerResolutionInput, OwnerResolver,
};
use crate::daemon::invocation::admission::policy_engine::{
    PolicyEngine, PolicyInput, SystemPolicyRuleMatch,
};
use crate::daemon::persistence::access_control::AccessControlStoreRegistry;
use crate::daemon::trust::anchor::{RealmTrustAnchor, TrustAnchorRole};

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum TrustedCallerPath {
    User,
    Hub,
    Backend,
    DeviceCustody(VerifiedDeviceInvocationPurpose),
    AgentDeviceCustody,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum VerifiedCallerEvidence {
    TrustAnchorRole(TrustAnchorRole),
    PrincipalLifecycleRole(TrustAnchorRole),
    LocalHostedAgentKey,
    Federated,
}

impl TrustedCallerPath {
    /// Classify the immediate actor of a runtime-derived child.
    ///
    /// This path is entered only after Axon has proven that the child caller
    /// is the admitted parent's exact callee and bound the child to the
    /// parent's scalar receipt capability. It therefore classifies the actor
    /// shape without requiring a second, independently registered key for a
    /// transient SystemAgent execution identity.
    pub(crate) fn from_derived_child_caller(caller_ura: &str) -> Result<Self, Status> {
        let caller = parse_ura(caller_ura).map_err(|error| {
            Status::invalid_argument(format!(
                "DERIVED_CHILD_CALLER_URA_INVALID: caller `{caller_ura}` is not canonical: {error}"
            ))
        })?;
        match caller.kind {
            URAKind::Agent | URAKind::Service => Ok(Self::AgentDeviceCustody),
            URAKind::Authority => Ok(Self::Hub),
            _ => Err(Status::invalid_argument(format!(
                "DERIVED_CHILD_CALLER_KIND_MISMATCH: derived child caller `{caller_ura}` must be the parent callable Agent, Service, or Authority"
            ))),
        }
    }

    pub(crate) fn from_verified_invocation_caller(
        caller_ura: &str,
        evidence: VerifiedCallerEvidence,
        public_ability: &str,
        device_purpose: Option<VerifiedDeviceInvocationPurpose>,
    ) -> Result<Self, Status> {
        match evidence {
            VerifiedCallerEvidence::TrustAnchorRole(role)
            | VerifiedCallerEvidence::PrincipalLifecycleRole(role) => {
                Self::from_trust_anchor_role_and_caller(
                    role,
                    caller_ura,
                    public_ability,
                    device_purpose,
                )
            }
            VerifiedCallerEvidence::LocalHostedAgentKey => {
                Self::from_local_hosted_agent_custody(caller_ura)
            }
            VerifiedCallerEvidence::Federated => {
                Self::from_federated_invocation_caller(caller_ura, public_ability, device_purpose)
            }
        }
    }

    pub(crate) fn from_local_hosted_agent_custody(caller_ura: &str) -> Result<Self, Status> {
        let caller = parse_ura(caller_ura).map_err(|error| {
            Status::invalid_argument(format!(
                "LOCAL_HOSTED_AGENT_CALLER_URA_INVALID: caller `{caller_ura}` is not canonical: {error}"
            ))
        })?;
        if caller.kind != URAKind::Agent {
            return Err(Status::invalid_argument(format!(
                "LOCAL_HOSTED_AGENT_CALLER_KIND_MISMATCH: local hosted-Agent key custody requires an Agent caller URA, got `{caller_ura}`"
            )));
        }
        Ok(Self::AgentDeviceCustody)
    }

    pub(crate) fn from_federated_invocation_caller(
        caller_ura: &str,
        public_ability: &str,
        device_purpose: Option<VerifiedDeviceInvocationPurpose>,
    ) -> Result<Self, Status> {
        let caller = parse_ura(caller_ura).map_err(|error| {
            Status::invalid_argument(format!(
                "FEDERATED_CALLER_URA_INVALID: caller `{caller_ura}` is not canonical: {error}"
            ))
        })?;
        match caller.kind {
            URAKind::Agent => Ok(Self::AgentDeviceCustody),
            URAKind::Device => {
                let purpose = require_device_caller_purpose(
                    caller_ura,
                    public_ability,
                    device_purpose,
                )?;
                Ok(Self::DeviceCustody(purpose))
            }
            URAKind::User => Ok(Self::User),
            URAKind::Authority => Ok(Self::Hub),
            _ => Err(Status::invalid_argument(format!(
                "FEDERATED_CALLER_KIND_MISMATCH: federated caller `{caller_ura}` must be a User, Agent, Device, or Authority URA"
            ))),
        }
    }

    fn from_trust_anchor_role_and_caller(
        role: TrustAnchorRole,
        caller_ura: &str,
        public_ability: &str,
        device_purpose: Option<VerifiedDeviceInvocationPurpose>,
    ) -> Result<Self, Status> {
        match role {
            TrustAnchorRole::User => Ok(Self::User),
            TrustAnchorRole::Hub => Ok(Self::Hub),
            TrustAnchorRole::Backend => Ok(Self::Backend),
            TrustAnchorRole::Device => {
                let caller = parse_ura(caller_ura).map_err(|error| {
                    Status::invalid_argument(format!(
                        "TRUST_PATH_CALLER_URA_INVALID: device-custody caller `{caller_ura}` is not canonical: {error}"
                    ))
                })?;
                match caller.kind {
                    URAKind::Agent => Ok(Self::AgentDeviceCustody),
                    URAKind::Device => {
                        let purpose = require_device_caller_purpose(
                            caller_ura,
                            public_ability,
                            device_purpose,
                        )?;
                        Ok(Self::DeviceCustody(purpose))
                    }
                    _ => Err(Status::invalid_argument(format!(
                        "TRUST_PATH_KIND_MISMATCH: device-custody caller `{caller_ura}` must be a Device or Agent URA"
                    ))),
                }
            }
        }
    }
}

fn require_device_caller_purpose(
    caller_ura: &str,
    public_ability: &str,
    device_purpose: Option<VerifiedDeviceInvocationPurpose>,
) -> Result<VerifiedDeviceInvocationPurpose, Status> {
    let Some(purpose) = device_purpose else {
        return Err(Status::permission_denied(format!(
            "DEVICE_CALLER_PURPOSE_UNVERIFIED: Device caller `{caller_ura}` has no verified invocation purpose for `{public_ability}`"
        )));
    };
    if !purpose.supports_public_ability(public_ability) {
        return Err(Status::permission_denied(format!(
            "DEVICE_CALLER_PURPOSE_MISMATCH: Device caller `{caller_ura}` presented {:?} for `{public_ability}`",
            purpose.purpose()
        )));
    }
    Ok(purpose)
}

#[derive(Debug, Clone)]
pub(crate) struct AdmissionPolicyContext<'a> {
    pub(crate) envelope: &'a Envelope,
    pub(crate) ability: &'a str,
    pub(crate) action: AccessAction,
    pub(crate) safe_read: bool,
    pub(crate) trusted_path: TrustedCallerPath,
    pub(crate) daemon_ura: Option<&'a str>,
    pub(crate) trust_anchor: &'a RealmTrustAnchor,
    pub(crate) access_control_stores: &'a AccessControlStoreRegistry,
    pub(crate) canonical_hash: Option<String>,
    pub(crate) signature_key_id: Option<String>,
    pub(crate) verified_authority_id: Option<String>,
    pub(crate) verified_session_id: Option<String>,
    /// Accountable principal proven by an admitted parent Invocation.
    ///
    /// The immediate child caller remains the executing Agent/SystemAgent;
    /// this projection carries the User authority under which that actor is
    /// executing. It is accepted only from the runtime-derived child seam,
    /// never from public request metadata.
    pub(crate) accountable_principal: Option<PrincipalProjection>,
    pub(crate) rejector_ura: Option<String>,
}

pub struct AdmissionPolicyGate;

fn push_system_rule(
    system_rule_matches: &mut Vec<SystemPolicyRuleMatch>,
    matched: bool,
    rule_match: SystemPolicyRuleMatch,
) {
    if matched {
        system_rule_matches.push(rule_match);
    }
}

impl AdmissionPolicyGate {
    pub(crate) fn verify(context: AdmissionPolicyContext<'_>) -> Result<PolicyDecision, Status> {
        let caller_ura = agent_ura(context.envelope.caller.as_ref(), "caller")?;
        let callee_ura = agent_ura(context.envelope.callee.as_ref(), "callee")?;
        let subject_ura = subject_ura(context.envelope.subject.as_ref())?;
        let ability_ura = ability_ura_for(&callee_ura, context.ability)?;
        let owner = resolve_owner(&subject_ura, &callee_ura, context.trust_anchor)?;
        let verified_caller = VerifiedCallerProjection::from_trusted_path(
            context.trusted_path,
            caller_ura,
            context.trust_anchor,
        )?;
        debug_assert_eq!(verified_caller.trust_path, context.trusted_path);
        let mut system_rule_matches = Vec::new();
        push_system_rule(
            &mut system_rule_matches,
            authority_self_read_scope(
                &verified_caller.caller_ura,
                &callee_ura,
                &subject_ura,
                &ability_ura,
                context.daemon_ura,
                verified_caller.trust_path,
            ),
            SystemPolicyRuleMatch::AuthoritySelfRead,
        );
        push_system_rule(
            &mut system_rule_matches,
            authority_self_manage_scope(
                &verified_caller.caller_ura,
                &callee_ura,
                &subject_ura,
                &ability_ura,
                context.daemon_ura,
                verified_caller.trust_path,
                context.action,
            ),
            SystemPolicyRuleMatch::AuthoritySelfManage,
        );
        push_system_rule(
            &mut system_rule_matches,
            authority_self_stream_scope(
                &verified_caller.caller_ura,
                &callee_ura,
                &subject_ura,
                &ability_ura,
                context.daemon_ura,
                verified_caller.trust_path,
                context.action,
            ),
            SystemPolicyRuleMatch::AuthoritySelfStream,
        );
        push_system_rule(
            &mut system_rule_matches,
            authority_peer_directory_stream_scope(
                &verified_caller.caller_ura,
                &callee_ura,
                &subject_ura,
                &ability_ura,
                context.daemon_ura,
                verified_caller.trust_path,
                context.action,
                context.trust_anchor,
            ),
            SystemPolicyRuleMatch::AuthorityPeerDirectoryStream,
        );
        push_system_rule(
            &mut system_rule_matches,
            realm_authority_public_read_scope(
                &verified_caller.caller_ura,
                &callee_ura,
                &subject_ura,
                context.daemon_ura,
                verified_caller.trust_path,
                context.action,
                context.safe_read,
            ),
            SystemPolicyRuleMatch::RealmAuthorityPublicRead,
        );
        push_system_rule(
            &mut system_rule_matches,
            device_publication_custody_manage_scope(
                &verified_caller.caller_ura,
                &callee_ura,
                &subject_ura,
                &ability_ura,
                context.daemon_ura,
                verified_caller.trust_path,
                context.action,
            ),
            SystemPolicyRuleMatch::DevicePublicationCustodyManage,
        );
        push_system_rule(
            &mut system_rule_matches,
            device_self_session_stream_scope(
                &verified_caller.caller_ura,
                &callee_ura,
                &subject_ura,
                &ability_ura,
                context.daemon_ura,
                verified_caller.trust_path,
                context.action,
            ),
            SystemPolicyRuleMatch::DeviceSelfSessionStream,
        );
        push_system_rule(
            &mut system_rule_matches,
            device_lifecycle_self_revoke_manage_scope(
                &verified_caller.caller_ura,
                &callee_ura,
                &subject_ura,
                &ability_ura,
                context.daemon_ura,
                verified_caller.trust_path,
                context.action,
            ),
            SystemPolicyRuleMatch::DeviceLifecycleSelfRevokeManage,
        );
        push_system_rule(
            &mut system_rule_matches,
            device_hosted_agent_retraction_manage_scope(
                &verified_caller.caller_ura,
                &callee_ura,
                &subject_ura,
                &ability_ura,
                context.daemon_ura,
                verified_caller.trust_path,
                context.action,
            ),
            SystemPolicyRuleMatch::DeviceHostedAgentRetractionManage,
        );
        push_system_rule(
            &mut system_rule_matches,
            remote_owner_forward_allowed(
                &verified_caller.caller_ura,
                &callee_ura,
                context.daemon_ura,
                verified_caller.trust_path,
                context.trust_anchor,
            ),
            SystemPolicyRuleMatch::RemoteOwnerForward,
        );
        let invocation_lifecycle_control =
            invocation_lifecycle_control_scope(context.ability, context.action);
        let device_invocation_purpose = verified_caller.principal.device_invocation_purpose;
        let accountable_principal = context
            .accountable_principal
            .unwrap_or(verified_caller.principal);
        let policy_input = PolicyInput {
            owner,
            caller_user_ura: accountable_principal.caller_user_ura,
            caller_ura: verified_caller.caller_ura,
            principal_kind: accountable_principal.kind,
            principal_id: accountable_principal.id,
            token_id: accountable_principal.token_id,
            token_class: accountable_principal.token_class,
            device_invocation_purpose,
            callee_ura,
            subject_ura,
            ability_ura,
            action: context.action,
            safe_read: context.safe_read,
            system_rule_matches,
            invocation_lifecycle_control,
            interactive_context_available: false,
            canonical_hash: context.canonical_hash,
            signature_key_id: context.signature_key_id,
            verified_authority_id: context.verified_authority_id,
            verified_session_id: context.verified_session_id,
            rejector_ura: context.rejector_ura,
            now: Utc::now(),
            grants: Vec::new(),
        };

        let decision = match policy_input.owner.owner_user_ura.as_deref() {
            Some(owner_user_ura) => context
                .access_control_stores
                .with_store(owner_user_ura, |store| {
                    let mut input = policy_input.clone();
                    input.grants = store.grants();
                    let decision = PolicyEngine::check(input);
                    if decision.decision == PolicyDecisionOutcome::Allow
                        && decision.reason == PolicyDecisionReason::ExplicitGrantAllow
                    {
                        if let Some(grant_id) = decision.grant_id.as_deref() {
                            store
                                .consume_once_grant_if_applicable(grant_id, &decision.caller_ura)
                                .map_err(|err| {
                                    Status::permission_denied(format!(
                                        "GRANT_CONSUME_FAILED: grant_id={grant_id} error={err}"
                                    ))
                                })?;
                        }
                    }
                    Ok::<PolicyDecision, Status>(decision)
                })
                .map_err(|err| {
                    Status::internal(format!(
                        "POLICY_STORE_UNAVAILABLE: owner_user_ura={owner_user_ura} error={err}"
                    ))
                })??,
            None => PolicyEngine::check(policy_input),
        };

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

fn invocation_lifecycle_control_scope(ability: &str, action: AccessAction) -> bool {
    action == AccessAction::Manage
        && ability.trim() == crate::daemon::ability::names::governance::INVOCATION_CANCEL
}

#[derive(Debug, Clone)]
pub(crate) struct PrincipalProjection {
    pub(crate) kind: PrincipalKind,
    pub(crate) id: String,
    pub(crate) token_id: Option<String>,
    pub(crate) token_class: Option<TokenClass>,
    pub(crate) caller_user_ura: Option<String>,
    pub(crate) device_invocation_purpose: Option<VerifiedDeviceInvocationPurpose>,
}

impl PrincipalProjection {
    pub(crate) fn accountable_user(user_ura: &str) -> Result<Self, Status> {
        let user_ura = canonical_user_principal_ura(user_ura).map_err(|error| {
            Status::invalid_argument(format!(
                "DERIVED_PRINCIPAL_KIND_MISMATCH: parent authority principal `{user_ura}` must be a canonical User URA: {error}"
            ))
        })?;
        Ok(Self {
            kind: PrincipalKind::User,
            id: user_ura.clone(),
            token_id: None,
            token_class: None,
            caller_user_ura: Some(user_ura),
            device_invocation_purpose: None,
        })
    }
}

#[derive(Debug, Clone)]
pub(crate) struct VerifiedCallerProjection {
    pub(crate) caller_ura: String,
    pub(crate) trust_path: TrustedCallerPath,
    pub(crate) principal: PrincipalProjection,
}

impl VerifiedCallerProjection {
    pub(crate) fn from_trusted_path(
        trust_path: TrustedCallerPath,
        caller_ura: String,
        trust_anchor: &RealmTrustAnchor,
    ) -> Result<Self, Status> {
        let principal = principal_for(trust_path, &caller_ura, trust_anchor)?;
        Ok(Self {
            caller_ura,
            trust_path,
            principal,
        })
    }
}

pub(crate) fn principal_for(
    path: TrustedCallerPath,
    caller_ura: &str,
    _trust_anchor: &RealmTrustAnchor,
) -> Result<PrincipalProjection, Status> {
    match path {
        TrustedCallerPath::User => {
            let user_ura = canonical_user_principal_ura(caller_ura).map_err(|error| {
                Status::invalid_argument(format!(
                    "PRINCIPAL_KIND_MISMATCH: user-trusted caller `{caller_ura}` must be a canonical User URA: {error}"
                ))
            })?;
            Ok(PrincipalProjection {
                kind: PrincipalKind::User,
                id: user_ura.clone(),
                token_id: None,
                token_class: None,
                caller_user_ura: Some(user_ura),
                device_invocation_purpose: None,
            })
        }
        TrustedCallerPath::Hub => Ok(PrincipalProjection {
            kind: PrincipalKind::Token,
            id: caller_ura.to_string(),
            token_id: Some(caller_ura.to_string()),
            token_class: Some(TokenClass::HubLink),
            caller_user_ura: None,
            device_invocation_purpose: None,
        }),
        TrustedCallerPath::AgentDeviceCustody => Ok(PrincipalProjection {
            kind: PrincipalKind::Agent,
            id: caller_ura.to_string(),
            token_id: None,
            token_class: None,
            caller_user_ura: None,
            device_invocation_purpose: None,
        }),
        TrustedCallerPath::DeviceCustody(purpose) => Ok(PrincipalProjection {
            kind: PrincipalKind::DeviceCustody,
            id: caller_ura.to_string(),
            token_id: Some(caller_ura.to_string()),
            token_class: purpose
                .carries_pairing_token_scope()
                .then_some(TokenClass::DevicePairing),
            caller_user_ura: None,
            device_invocation_purpose: Some(purpose),
        }),
        TrustedCallerPath::Backend => Ok(PrincipalProjection {
            kind: PrincipalKind::Service,
            id: caller_ura.to_string(),
            token_id: None,
            token_class: None,
            caller_user_ura: None,
            device_invocation_purpose: None,
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
            .map(|_| OwnerFact::user_ura(ura.to_string(), ura.to_string())),
        URAKind::Agent => {
            if let Some((user_id, _)) = parsed.agent_ids() {
                let owner_ura = crate::core::ura::user_ura(&parsed.realm, user_id);
                Some(OwnerFact::user_ura(owner_ura.clone(), owner_ura))
            } else if let Some((device_id, _)) = parsed.device_agent_ids() {
                // A SystemAgent is the behavioral callee, while its sponsoring
                // Device is only the execution/custody boundary.  Accountability
                // therefore follows the Device's durable principal-owner fact;
                // it must never be inferred from ambient pairing credentials.
                let sponsor_device_ura = crate::core::ura::device_ura(&parsed.realm, device_id);
                owner_fact_from_trust_anchor(&sponsor_device_ura, trust_anchor)
            } else {
                None
            }
        }
        URAKind::Service => {
            if let Some((principal_id, _)) = parsed.service_ids() {
                let owner_ura = crate::core::ura::user_ura(&parsed.realm, principal_id);
                Some(OwnerFact::user_ura(owner_ura.clone(), owner_ura))
            } else {
                None
            }
        }
        URAKind::Ability => match parsed.ability().map(|ability| ability.owner) {
            Some(AbilityOwner::Agent { user_id, agent_id }) => owner_fact_from_trust_anchor(
                &crate::core::ura::agent_ura(&parsed.realm, &user_id, &agent_id),
                trust_anchor,
            )
            .or_else(|| {
                let owner_ura = crate::core::ura::user_ura(&parsed.realm, &user_id);
                Some(OwnerFact::user_ura(owner_ura.clone(), owner_ura))
            }),
            Some(AbilityOwner::Device { .. }) => None,
            Some(AbilityOwner::Service { principal_id, .. }) => {
                let owner_ura = crate::core::ura::user_ura(&parsed.realm, &principal_id);
                Some(OwnerFact::user_ura(owner_ura.clone(), owner_ura))
            }
            Some(AbilityOwner::SystemAgent {
                device_id,
                agent_id: _,
            }) => {
                let device_ura = crate::core::ura::device_ura(&parsed.realm, &device_id);
                owner_fact_from_trust_anchor(&device_ura, trust_anchor)
            }
            Some(AbilityOwner::Authority) => None,
            None => None,
        },
        URAKind::Device => owner_fact_from_trust_anchor(ura, trust_anchor),
        URAKind::Authority => owner_fact_from_trust_anchor(ura, trust_anchor),
        URAKind::Resource => resource_owner_user_ura(&parsed)
            .map(|owner_ura| OwnerFact::user_ura(owner_ura.clone(), owner_ura)),
        _ => None,
    };
    Ok(owner)
}

fn owner_fact_from_trust_anchor(ura: &str, trust_anchor: &RealmTrustAnchor) -> Option<OwnerFact> {
    let owner = trust_anchor.lookup_principal_owner(ura)?;
    Some(OwnerFact::user_ura(
        owner.owner_ura.clone(),
        owner.owner_ura.clone(),
    ))
}

fn resource_owner_user_ura(parsed: &crate::core::ura::ParsedURA) -> Option<String> {
    let owner_id = parsed.resource_owner_id()?;
    if let Some(rest) = owner_id.strip_prefix("agent.") {
        let (user_id, _) = rest.split_once('.')?;
        return (!user_id.is_empty()).then(|| crate::core::ura::user_ura(&parsed.realm, user_id));
    }
    owner_id
        .strip_prefix("user.")
        .filter(|user_id| !user_id.is_empty())
        .map(|user_id| crate::core::ura::user_ura(&parsed.realm, user_id))
}

fn canonical_user_principal_ura(ura: &str) -> Result<String, String> {
    let parsed = parse_ura(ura).map_err(|error| error.to_string())?;
    if parsed.kind != URAKind::User || parsed.user_id().is_none() {
        return Err("not a User URA".to_string());
    }
    Ok(ura.to_string())
}

fn remote_owner_forward_allowed(
    caller_ura: &str,
    callee_ura: &str,
    daemon_ura: Option<&str>,
    trusted_path: TrustedCallerPath,
    trust_anchor: &RealmTrustAnchor,
) -> bool {
    if matches!(trusted_path, TrustedCallerPath::DeviceCustody(_)) {
        return false;
    }
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
    trusted_path: TrustedCallerPath,
) -> bool {
    if trusted_path != TrustedCallerPath::Hub {
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
    trusted_path: TrustedCallerPath,
    action: AccessAction,
) -> bool {
    if action != AccessAction::Manage || trusted_path != TrustedCallerPath::Hub {
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
    trusted_path: TrustedCallerPath,
    action: AccessAction,
) -> bool {
    if action != AccessAction::Stream || trusted_path != TrustedCallerPath::Hub {
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

#[expect(
    clippy::too_many_arguments,
    reason = "policy evaluation requires each canonical identity, action, role, and trust fact explicitly"
)]
fn authority_peer_directory_stream_scope(
    caller_ura: &str,
    callee_ura: &str,
    subject_ura: &str,
    ability_ura: &str,
    daemon_ura: Option<&str>,
    trusted_path: TrustedCallerPath,
    action: AccessAction,
    trust_anchor: &RealmTrustAnchor,
) -> bool {
    matches!(
        VerifiedAuthorityPeerDirectoryStream::classify(
            caller_ura,
            callee_ura,
            subject_ura,
            ability_ura,
            daemon_ura,
            trusted_path,
            action,
            trust_anchor,
        ),
        AuthorityPeerDirectoryStreamMatch::Verified(_)
    )
}

/// Typed proof that a signed peer Authority is opening the one canonical
/// cross-realm directory stream owned by that caller. This is deliberately
/// narrower than generic Resource authority and is shared by the pre-metadata
/// authority gate and the RFC-014 system policy rule.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct VerifiedAuthorityPeerDirectoryStream {
    authority_id: String,
    policy_ura: String,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) enum AuthorityPeerDirectoryStreamMatch {
    NotApplicable,
    Verified(VerifiedAuthorityPeerDirectoryStream),
    Denied(&'static str),
}

impl AuthorityPeerDirectoryStreamMatch {
    pub(crate) fn into_result(
        self,
    ) -> Result<Option<VerifiedAuthorityPeerDirectoryStream>, &'static str> {
        match self {
            Self::NotApplicable => Ok(None),
            Self::Verified(authority) => Ok(Some(authority)),
            Self::Denied(reason) => Err(reason),
        }
    }
}

impl VerifiedAuthorityPeerDirectoryStream {
    #[expect(
        clippy::too_many_arguments,
        reason = "the proof binds every signed invocation and trust fact explicitly"
    )]
    pub(crate) fn classify(
        caller_ura: &str,
        callee_ura: &str,
        subject_ura: &str,
        ability_ura: &str,
        daemon_ura: Option<&str>,
        trusted_path: TrustedCallerPath,
        action: AccessAction,
        trust_anchor: &RealmTrustAnchor,
    ) -> AuthorityPeerDirectoryStreamMatch {
        let Ok(caller) = parse_ura(caller_ura) else {
            return AuthorityPeerDirectoryStreamMatch::NotApplicable;
        };
        let Ok(callee) = parse_ura(callee_ura) else {
            return AuthorityPeerDirectoryStreamMatch::NotApplicable;
        };
        let expected_ability_ura = crate::core::ura::owner_ability_ura(
            callee_ura,
            crate::daemon::invocation::dispatch::federation_wrappers::ABILITY_FEDERATION_SUBSCRIBE_DIRECTORY_V2,
        )
        .unwrap_or_default();
        if trusted_path != TrustedCallerPath::Hub
            || caller.kind != URAKind::Authority
            || callee.kind != URAKind::Authority
            || caller_ura == callee_ura
            || ability_ura != expected_ability_ura
        {
            return AuthorityPeerDirectoryStreamMatch::NotApplicable;
        }
        if action != AccessAction::Stream {
            return AuthorityPeerDirectoryStreamMatch::Denied(
                "directory subscription requires stream action",
            );
        }
        if Some(callee_ura) != daemon_ura {
            return AuthorityPeerDirectoryStreamMatch::Denied(
                "callee must be the selected local Authority",
            );
        }
        if caller.realm == callee.realm {
            return AuthorityPeerDirectoryStreamMatch::Denied(
                "peer and local Authority realms must differ",
            );
        }
        if !trust_anchor.has_federation_peer_for_realm(&caller.realm) {
            return AuthorityPeerDirectoryStreamMatch::Denied(
                "caller realm is not a trusted federation peer",
            );
        }
        let Ok(subject) = parse_ura(subject_ura) else {
            return AuthorityPeerDirectoryStreamMatch::Denied("directory subject is not canonical");
        };
        let expected_subject_path = format!("directory/{}", callee.realm);
        if !(subject.kind == URAKind::Resource
            && subject.realm == caller.realm
            && subject.resource_owner_id() == Some("hub.federation")
            && subject.resource_path() == Some(expected_subject_path.as_str()))
        {
            return AuthorityPeerDirectoryStreamMatch::Denied(
                "directory subject does not exactly bind caller and callee realms",
            );
        }
        let policy_ura = crate::core::ura::resource_dot_ura(
            &callee.realm,
            "hub.federation",
            "policy/system.authority.peer_directory_stream",
        );
        AuthorityPeerDirectoryStreamMatch::Verified(Self {
            authority_id: format!("peer-directory-stream:{caller_ura}:{callee_ura}"),
            policy_ura,
        })
    }

    pub(crate) fn authority_id(&self) -> &str {
        &self.authority_id
    }

    pub(crate) fn policy_ura(&self) -> &str {
        &self.policy_ura
    }
}

#[expect(
    clippy::too_many_arguments,
    reason = "policy evaluation requires each canonical identity, action, role, and safety fact explicitly"
)]
fn realm_authority_public_read_scope(
    caller_ura: &str,
    callee_ura: &str,
    subject_ura: &str,
    daemon_ura: Option<&str>,
    trusted_path: TrustedCallerPath,
    action: AccessAction,
    safe_read: bool,
) -> bool {
    if trusted_path != TrustedCallerPath::Hub || action != AccessAction::Read || !safe_read {
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
    // The only pre-owner-binding Hub read exception is the local
    // DeviceProfileProjection itself. Direct Device-owned Ability URAs are
    // migration read-model facts, not public policy subjects or callees.
    subject_ura == callee_ura
}

fn device_publication_custody_manage_scope(
    caller_ura: &str,
    callee_ura: &str,
    subject_ura: &str,
    ability_ura: &str,
    daemon_ura: Option<&str>,
    trusted_path: TrustedCallerPath,
    action: AccessAction,
) -> bool {
    let TrustedCallerPath::DeviceCustody(path_purpose) = trusted_path else {
        return false;
    };
    admitted_device_policy_purpose(DeviceCallerPolicyScope {
        caller_ura,
        callee_ura,
        subject_ura,
        ability_ura,
        daemon_ura,
        action,
    }) == Some(path_purpose)
        && path_purpose.is(DeviceCallerPurpose::PublicationCustody)
}

fn device_self_session_stream_scope(
    caller_ura: &str,
    callee_ura: &str,
    subject_ura: &str,
    ability_ura: &str,
    daemon_ura: Option<&str>,
    trusted_path: TrustedCallerPath,
    action: AccessAction,
) -> bool {
    let TrustedCallerPath::DeviceCustody(path_purpose) = trusted_path else {
        return false;
    };
    admitted_device_policy_purpose(DeviceCallerPolicyScope {
        caller_ura,
        callee_ura,
        subject_ura,
        ability_ura,
        daemon_ura,
        action,
    }) == Some(path_purpose)
        && path_purpose.is(DeviceCallerPurpose::DeviceSelfSession)
}

fn device_lifecycle_self_revoke_manage_scope(
    caller_ura: &str,
    callee_ura: &str,
    subject_ura: &str,
    ability_ura: &str,
    daemon_ura: Option<&str>,
    trusted_path: TrustedCallerPath,
    action: AccessAction,
) -> bool {
    let TrustedCallerPath::DeviceCustody(path_purpose) = trusted_path else {
        return false;
    };
    admitted_device_policy_purpose(DeviceCallerPolicyScope {
        caller_ura,
        callee_ura,
        subject_ura,
        ability_ura,
        daemon_ura,
        action,
    }) == Some(path_purpose)
        && path_purpose.is(DeviceCallerPurpose::LifecycleSelfRevoke)
}

fn device_hosted_agent_retraction_manage_scope(
    caller_ura: &str,
    callee_ura: &str,
    subject_ura: &str,
    ability_ura: &str,
    daemon_ura: Option<&str>,
    trusted_path: TrustedCallerPath,
    action: AccessAction,
) -> bool {
    let TrustedCallerPath::DeviceCustody(path_purpose) = trusted_path else {
        return false;
    };
    admitted_device_policy_purpose(DeviceCallerPolicyScope {
        caller_ura,
        callee_ura,
        subject_ura,
        ability_ura,
        daemon_ura,
        action,
    }) == Some(path_purpose)
        && path_purpose.is(DeviceCallerPurpose::HostedAgentRetraction)
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
#[path = "policy_gate_tests.rs"]
mod policy_gate_tests;
