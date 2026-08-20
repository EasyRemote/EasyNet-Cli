// EasyNet Daemon - Device Caller Classification
// =============================================
//
// File: src/daemon/invocation/admission/device_caller.rs
// Description: Centralized classifier for the narrow invocation purposes where
//              a Device URA may appear as caller.
//
// Protocol Responsibility
// -----------------------
// A Device is substrate/custody, not an ordinary actor. This module is the
// daemon admission boundary that names the few Device-caller exceptions before
// policy or FFI can accidentally treat `URAKind::Device` as a generic allow.
//
// Implementation Approach
// -----------------------
// The classifier is deterministic over the caller kind and selected public
// ability / ability URA. It does not consult mutable product state and does not
// verify keys, ownership, or grants.
//
// Usage Contract
// --------------
// Consumers must use the typed `DeviceCallerPurpose` result. Raw
// `URAKind::Device` is parsing evidence only, never authorization.

use crate::core::ura::{parse_ura, URAKind};
use crate::daemon::invocation::admission::decision::AccessAction;
pub(crate) use crate::daemon::invocation::admission::device_caller_types::{
    DeviceCallerPurpose, VerifiedDeviceInvocationPurpose,
};
use sha2::{Digest as _, Sha256};

impl VerifiedDeviceInvocationPurpose {
    #[must_use]
    pub(crate) fn supports_public_ability(self, public_ability: &str) -> bool {
        device_caller_rule_for_purpose(public_ability, self.purpose).is_some()
    }

    #[must_use]
    pub(crate) fn matches_scope(self, scope: DeviceInvocationPurposeScope<'_>) -> bool {
        device_caller_rule_for_purpose(scope.public_ability, self.purpose).is_some()
            && self.invocation_binding == device_invocation_binding(scope)
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum DeviceCallerPolicyAdmission {
    None,
    AuthoritySelf { action: AccessAction },
    AuthorityOwnerProjection { action: AccessAction },
    AuthorityHostedAgentRetraction { action: AccessAction },
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum DeviceCallerCalleeGeometry {
    SelectedAuthority,
    DeviceSelfOrSelectedAuthority,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum DeviceCallerSubjectGeometry {
    SameRealm,
    DeviceSelf,
    OwnerProjection,
    HostedAgent,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
struct DeviceCallerRule {
    public_ability: &'static str,
    purpose: DeviceCallerPurpose,
    action: AccessAction,
    callee_geometry: DeviceCallerCalleeGeometry,
    subject_geometry: DeviceCallerSubjectGeometry,
    policy_admission: DeviceCallerPolicyAdmission,
}

const DEVICE_CALLER_RULES: &[DeviceCallerRule] = &[
    DeviceCallerRule {
        public_ability:
            crate::daemon::ability::conformance::ABILITY_RUNTIME_BOOTSTRAP_SELF_IDENTITY,
        purpose: DeviceCallerPurpose::Bootstrap,
        action: AccessAction::Manage,
        // Runtime bootstrap is an Authority-owned, descriptor-bound ability.
        // The Device is the self subject, never the execution callee.
        callee_geometry: DeviceCallerCalleeGeometry::SelectedAuthority,
        subject_geometry: DeviceCallerSubjectGeometry::DeviceSelf,
        policy_admission: DeviceCallerPolicyAdmission::None,
    },
    DeviceCallerRule {
        public_ability: crate::daemon::ability::conformance::ABILITY_IDENTITY_REGISTER_PUBKEY,
        purpose: DeviceCallerPurpose::Pairing,
        action: AccessAction::Manage,
        callee_geometry: DeviceCallerCalleeGeometry::SelectedAuthority,
        subject_geometry: DeviceCallerSubjectGeometry::SameRealm,
        policy_admission: DeviceCallerPolicyAdmission::None,
    },
    DeviceCallerRule {
        // During URA join the Device has completed `federation.join` but the
        // paired User key is not usable locally until the Hub identity key is
        // resolved and pinned. BootstrapAuthorityVerifier validates the exact
        // Authority callee, Ability subject, and resolve-key request geometry;
        // this rule only preserves that bounded pairing caller purpose through
        // the earlier caller-kind classifier.
        public_ability: crate::daemon::ability::conformance::ABILITY_FEDERATION_RESOLVE_KEY,
        purpose: DeviceCallerPurpose::Pairing,
        action: AccessAction::Read,
        callee_geometry: DeviceCallerCalleeGeometry::SelectedAuthority,
        subject_geometry: DeviceCallerSubjectGeometry::SameRealm,
        policy_admission: DeviceCallerPolicyAdmission::None,
    },
    DeviceCallerRule {
        public_ability: crate::daemon::ability::conformance::ABILITY_FEDERATION_ADVERTISE_AGENT,
        purpose: DeviceCallerPurpose::PublicationCustody,
        action: AccessAction::Manage,
        callee_geometry: DeviceCallerCalleeGeometry::SelectedAuthority,
        subject_geometry: DeviceCallerSubjectGeometry::HostedAgent,
        policy_admission: DeviceCallerPolicyAdmission::None,
    },
    DeviceCallerRule {
        public_ability: crate::daemon::ability::conformance::ABILITY_FEDERATION_ADVERTISE_ABILITIES,
        purpose: DeviceCallerPurpose::PublicationCustody,
        action: AccessAction::Manage,
        callee_geometry: DeviceCallerCalleeGeometry::SelectedAuthority,
        subject_geometry: DeviceCallerSubjectGeometry::OwnerProjection,
        policy_admission: DeviceCallerPolicyAdmission::AuthorityOwnerProjection {
            action: AccessAction::Manage,
        },
    },
    DeviceCallerRule {
        public_ability:
            crate::daemon::invocation::dispatch::federation_wrappers::ABILITY_FEDERATION_HEARTBEAT,
        purpose: DeviceCallerPurpose::AbilityCatalogDiff,
        action: AccessAction::Manage,
        callee_geometry: DeviceCallerCalleeGeometry::SelectedAuthority,
        subject_geometry: DeviceCallerSubjectGeometry::DeviceSelf,
        // BootstrapAuthorityVerifier validates that every refreshed owner is
        // exactly the caller Device before PolicyEngine sees an authority
        // proof. No generic policy exception is needed here.
        policy_admission: DeviceCallerPolicyAdmission::None,
    },
    DeviceCallerRule {
        public_ability: crate::daemon::ability::conformance::ABILITY_SESSION_OPEN,
        purpose: DeviceCallerPurpose::DeviceSelfSession,
        action: AccessAction::Stream,
        callee_geometry: DeviceCallerCalleeGeometry::DeviceSelfOrSelectedAuthority,
        subject_geometry: DeviceCallerSubjectGeometry::DeviceSelf,
        policy_admission: DeviceCallerPolicyAdmission::AuthoritySelf {
            action: AccessAction::Stream,
        },
    },
    DeviceCallerRule {
        public_ability: crate::daemon::ability::conformance::ABILITY_FEDERATION_REVOKE,
        purpose: DeviceCallerPurpose::LifecycleSelfRevoke,
        action: AccessAction::Manage,
        callee_geometry: DeviceCallerCalleeGeometry::SelectedAuthority,
        subject_geometry: DeviceCallerSubjectGeometry::DeviceSelf,
        policy_admission: DeviceCallerPolicyAdmission::AuthoritySelf {
            action: AccessAction::Manage,
        },
    },
    DeviceCallerRule {
        public_ability: crate::daemon::ability::conformance::ABILITY_FEDERATION_REVOKE,
        purpose: DeviceCallerPurpose::HostedAgentRetraction,
        action: AccessAction::Manage,
        callee_geometry: DeviceCallerCalleeGeometry::SelectedAuthority,
        subject_geometry: DeviceCallerSubjectGeometry::HostedAgent,
        policy_admission: DeviceCallerPolicyAdmission::AuthorityHostedAgentRetraction {
            action: AccessAction::Manage,
        },
    },
];

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum CallerKindAdmission {
    NonDevice,
    Device,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) enum DeviceCallerAdmissionError {
    InvalidCallerUra(String),
    NonActorCaller { kind: URAKind },
    DeviceCallerNotAllowed { public_ability: String },
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) enum DeviceInvocationPurposeVerificationError {
    Caller(DeviceCallerAdmissionError),
    GeometryDenied {
        public_ability: String,
        reason: &'static str,
    },
}

pub(crate) fn classify_public_invocation_caller(
    caller_ura: &str,
    public_ability: &str,
) -> Result<CallerKindAdmission, DeviceCallerAdmissionError> {
    let parsed = parse_ura(caller_ura).map_err(|error| {
        DeviceCallerAdmissionError::InvalidCallerUra(format!(
            "caller_ura `{caller_ura}` is invalid: {error}"
        ))
    })?;
    classify_public_invocation_caller_kind(parsed.kind, public_ability)
}

pub(crate) fn classify_public_invocation_caller_kind(
    caller_kind: URAKind,
    public_ability: &str,
) -> Result<CallerKindAdmission, DeviceCallerAdmissionError> {
    match caller_kind {
        URAKind::User | URAKind::Service | URAKind::Agent | URAKind::Authority => {
            Ok(CallerKindAdmission::NonDevice)
        }
        URAKind::Device => device_caller_ability_allowed(public_ability)
            .then_some(CallerKindAdmission::Device)
            .ok_or_else(|| DeviceCallerAdmissionError::DeviceCallerNotAllowed {
                public_ability: public_ability.to_string(),
            }),
        URAKind::Ability | URAKind::Resource | URAKind::Unknown => {
            Err(DeviceCallerAdmissionError::NonActorCaller { kind: caller_kind })
        }
    }
}

fn device_caller_ability_allowed(public_ability: &str) -> bool {
    let public_ability = public_ability.trim();
    DEVICE_CALLER_RULES
        .iter()
        .any(|rule| rule.public_ability == public_ability)
}

#[derive(Debug, Clone, Copy)]
pub(crate) struct DeviceInvocationPurposeScope<'a> {
    pub(crate) caller_ura: &'a str,
    pub(crate) callee_ura: &'a str,
    pub(crate) subject_ura: &'a str,
    pub(crate) public_ability: &'a str,
    pub(crate) daemon_ura: Option<&'a str>,
    pub(crate) action: AccessAction,
}

/// Verify the complete signed tuple before minting the opaque purpose proof.
/// This is intentionally stricter than the FFI-facing kind classifier above:
/// classification says a Device may have such a purpose; this function proves
/// that this invocation has it.
pub(crate) fn verify_device_invocation_purpose(
    scope: DeviceInvocationPurposeScope<'_>,
) -> Result<VerifiedDeviceInvocationPurpose, DeviceInvocationPurposeVerificationError> {
    let caller = parse_ura(scope.caller_ura).map_err(|error| {
        DeviceInvocationPurposeVerificationError::Caller(
            DeviceCallerAdmissionError::InvalidCallerUra(format!(
                "caller_ura `{}` is invalid: {error}",
                scope.caller_ura
            )),
        )
    })?;
    if caller.kind != URAKind::Device || caller.device_id().is_none() {
        return Err(DeviceInvocationPurposeVerificationError::Caller(
            DeviceCallerAdmissionError::NonActorCaller { kind: caller.kind },
        ));
    }
    if !device_caller_ability_allowed(scope.public_ability) {
        return Err(DeviceInvocationPurposeVerificationError::Caller(
            DeviceCallerAdmissionError::DeviceCallerNotAllowed {
                public_ability: scope.public_ability.to_string(),
            },
        ));
    }
    let rules = DEVICE_CALLER_RULES.iter().filter(|rule| {
        rule.public_ability == scope.public_ability.trim() && rule.action == scope.action
    });
    let mut has_action_match = false;
    let callee = parse_ura(scope.callee_ura).map_err(|_| {
        DeviceInvocationPurposeVerificationError::GeometryDenied {
            public_ability: scope.public_ability.to_string(),
            reason: "callee is not canonical",
        }
    })?;
    let subject = parse_ura(scope.subject_ura).map_err(|_| {
        DeviceInvocationPurposeVerificationError::GeometryDenied {
            public_ability: scope.public_ability.to_string(),
            reason: "subject is not canonical",
        }
    })?;
    let rule = rules
        .inspect(|_| has_action_match = true)
        .find(|rule| device_rule_geometry_matches(rule, scope, &caller, &callee, &subject))
        .ok_or_else(
            || DeviceInvocationPurposeVerificationError::GeometryDenied {
                public_ability: scope.public_ability.to_string(),
                reason: if has_action_match {
                    "invocation geometry does not match an admitted Device purpose"
                } else {
                    "admission action does not match the Device purpose"
                },
            },
        )?;
    Ok(VerifiedDeviceInvocationPurpose {
        purpose: rule.purpose,
        invocation_binding: device_invocation_binding(scope),
    })
}

fn device_rule_geometry_matches(
    rule: &DeviceCallerRule,
    scope: DeviceInvocationPurposeScope<'_>,
    caller: &crate::core::ura::ParsedURA,
    callee: &crate::core::ura::ParsedURA,
    subject: &crate::core::ura::ParsedURA,
) -> bool {
    let selected_authority = callee.kind == URAKind::Authority
        && callee.realm == caller.realm
        && !scope
            .daemon_ura
            .is_some_and(|daemon| daemon != scope.callee_ura);
    let device_self = callee.kind == URAKind::Device
        && scope.callee_ura == scope.caller_ura
        && callee.realm == caller.realm;
    let callee_matches = match rule.callee_geometry {
        DeviceCallerCalleeGeometry::SelectedAuthority => selected_authority,
        DeviceCallerCalleeGeometry::DeviceSelfOrSelectedAuthority => {
            device_self || selected_authority
        }
    };
    let subject_matches = subject.realm == caller.realm
        && match rule.subject_geometry {
            DeviceCallerSubjectGeometry::SameRealm => true,
            DeviceCallerSubjectGeometry::DeviceSelf => scope.subject_ura == scope.caller_ura,
            DeviceCallerSubjectGeometry::OwnerProjection => {
                matches!(
                    subject.kind,
                    URAKind::Device | URAKind::Agent | URAKind::Service
                )
            }
            DeviceCallerSubjectGeometry::HostedAgent => subject.kind == URAKind::Agent,
        };
    callee_matches && subject_matches
}

fn device_caller_rule_for_purpose(
    public_ability: &str,
    purpose: DeviceCallerPurpose,
) -> Option<&'static DeviceCallerRule> {
    let public_ability = public_ability.trim();
    DEVICE_CALLER_RULES
        .iter()
        .find(|rule| rule.public_ability == public_ability && rule.purpose == purpose)
}

fn device_invocation_binding(scope: DeviceInvocationPurposeScope<'_>) -> [u8; 32] {
    let mut hash = Sha256::new();
    hash.update(b"easynet.device.invocation-purpose\0");
    for value in [
        scope.caller_ura,
        scope.callee_ura,
        scope.subject_ura,
        scope.public_ability,
        scope.daemon_ura.unwrap_or_default(),
        scope.action.as_str(),
    ] {
        hash.update((value.len() as u64).to_be_bytes());
        hash.update(value.as_bytes());
    }
    hash.finalize().into()
}

#[derive(Debug, Clone, Copy)]
pub(crate) struct DeviceCallerPolicyScope<'a> {
    pub(crate) caller_ura: &'a str,
    pub(crate) callee_ura: &'a str,
    pub(crate) subject_ura: &'a str,
    pub(crate) ability_ura: &'a str,
    pub(crate) daemon_ura: Option<&'a str>,
    pub(crate) action: AccessAction,
}

pub(crate) fn admitted_device_policy_purpose(
    scope: DeviceCallerPolicyScope<'_>,
) -> Option<VerifiedDeviceInvocationPurpose> {
    let Some(daemon_ura) = scope.daemon_ura else {
        return None;
    };
    if scope.callee_ura != daemon_ura {
        return None;
    }
    let Ok(caller) = parse_ura(scope.caller_ura) else {
        return None;
    };
    if caller.kind != URAKind::Device {
        return None;
    }
    let Ok(callee) = parse_ura(scope.callee_ura) else {
        return None;
    };
    if callee.kind != URAKind::Authority || callee.realm != caller.realm {
        return None;
    }
    DEVICE_CALLER_RULES.iter().find_map(|rule| {
        let action = match rule.policy_admission {
            DeviceCallerPolicyAdmission::None => return None,
            DeviceCallerPolicyAdmission::AuthoritySelf { action } => {
                if scope.subject_ura != scope.caller_ura {
                    return None;
                }
                action
            }
            DeviceCallerPolicyAdmission::AuthorityOwnerProjection { action } => {
                if !owner_projection_publication_host_geometry(
                    scope.caller_ura,
                    scope.callee_ura,
                    scope.subject_ura,
                    Some(daemon_ura),
                ) {
                    return None;
                }
                action
            }
            DeviceCallerPolicyAdmission::AuthorityHostedAgentRetraction { action } => {
                if !hosted_agent_retraction_geometry(scope.caller_ura, scope.subject_ura) {
                    return None;
                }
                action
            }
        };
        if scope.action != action {
            return None;
        }
        let expected_ability =
            crate::core::ura::owner_ability_ura(scope.callee_ura, rule.public_ability)?;
        (scope.ability_ura == expected_ability).then(|| VerifiedDeviceInvocationPurpose {
            purpose: rule.purpose,
            invocation_binding: device_invocation_binding(DeviceInvocationPurposeScope {
                caller_ura: scope.caller_ura,
                callee_ura: scope.callee_ura,
                subject_ura: scope.subject_ura,
                public_ability: rule.public_ability,
                daemon_ura: scope.daemon_ura,
                action: scope.action,
            }),
        })
    })
}

fn hosted_agent_retraction_geometry(caller_ura: &str, subject_ura: &str) -> bool {
    let (Ok(caller), Ok(subject)) = (parse_ura(caller_ura), parse_ura(subject_ura)) else {
        return false;
    };
    caller.kind == URAKind::Device
        && subject.kind == URAKind::Agent
        && subject.realm == caller.realm
}

pub(crate) fn admits_owner_projection_publication_host(
    caller_ura: &str,
    callee_ura: &str,
    owner_ura: &str,
    daemon_ura: Option<&str>,
) -> bool {
    owner_projection_publication_host_geometry(caller_ura, callee_ura, owner_ura, daemon_ura)
}

fn owner_projection_publication_host_geometry(
    caller_ura: &str,
    callee_ura: &str,
    owner_ura: &str,
    daemon_ura: Option<&str>,
) -> bool {
    let Some(daemon_ura) = daemon_ura else {
        return false;
    };
    if callee_ura != daemon_ura {
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
    let Ok(owner) = parse_ura(owner_ura) else {
        return false;
    };
    if owner.realm != caller.realm {
        return false;
    }
    match owner.kind {
        // DeviceProfileProjection remains a same-device migration cursor only.
        URAKind::Device => owner_ura == caller_ura,
        // Agent ownership is checked against the trust-anchor owner binding by
        // OwnerProjectionPublicationAuthority after this custody geometry check.
        URAKind::Agent | URAKind::Service => true,
        // Authority self-publication is handled by the Authority caller path,
        // not by Device publication custody.
        URAKind::Authority => false,
        _ => false,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn classifies_public_device_caller_ability_surface() {
        for ability in [
            crate::daemon::ability::conformance::ABILITY_RUNTIME_BOOTSTRAP_SELF_IDENTITY,
            crate::daemon::ability::conformance::ABILITY_IDENTITY_REGISTER_PUBKEY,
            crate::daemon::ability::conformance::ABILITY_FEDERATION_RESOLVE_KEY,
            crate::daemon::ability::conformance::ABILITY_SESSION_OPEN,
            crate::daemon::ability::conformance::ABILITY_FEDERATION_ADVERTISE_AGENT,
            crate::daemon::ability::conformance::ABILITY_FEDERATION_ADVERTISE_ABILITIES,
            crate::daemon::invocation::dispatch::federation_wrappers::ABILITY_FEDERATION_HEARTBEAT,
            crate::daemon::ability::conformance::ABILITY_FEDERATION_REVOKE,
        ] {
            assert!(device_caller_ability_allowed(ability), "{ability}");
        }
        for ability in [
            "ability.publish",
            "ability.deploy",
            "ability.uninstall",
            "observe.health",
        ] {
            assert!(!device_caller_ability_allowed(ability), "{ability}");
        }
    }

    #[test]
    fn rejects_public_device_caller_for_ordinary_ability() {
        let err =
            classify_public_invocation_caller("easynet:///r/test/device/dev-1", "observe.health")
                .unwrap_err();

        assert_eq!(
            err,
            DeviceCallerAdmissionError::DeviceCallerNotAllowed {
                public_ability: "observe.health".to_string()
            }
        );
    }

    #[test]
    fn rejects_device_caller_for_ability_management_system_agent_abilities() {
        for ability in [
            "ability.publish",
            "ability.unpublish",
            "ability.deploy",
            "ability.uninstall",
        ] {
            let err = classify_public_invocation_caller("easynet:///r/test/device/dev-1", ability)
                .unwrap_err();

            assert_eq!(
                err,
                DeviceCallerAdmissionError::DeviceCallerNotAllowed {
                    public_ability: ability.to_string()
                }
            );
        }
    }

    #[test]
    fn admits_policy_scope_only_for_exact_publication_geometry() {
        let authority = "easynet:///r/test/authority";
        let device = "easynet:///r/test/device/dev-1";
        let ability_ura = crate::core::ura::owner_ability_ura(
            authority,
            crate::daemon::invocation::dispatch::federation_wrappers::ABILITY_FEDERATION_ADVERTISE_ABILITIES,
        )
        .unwrap();

        assert!(admitted_device_policy_purpose(DeviceCallerPolicyScope {
            caller_ura: device,
            callee_ura: authority,
            subject_ura: device,
            ability_ura: &ability_ura,
            daemon_ura: Some(authority),
            action: AccessAction::Manage,
        })
        .is_some_and(|purpose| purpose.is(DeviceCallerPurpose::PublicationCustody)));
        assert!(admitted_device_policy_purpose(DeviceCallerPolicyScope {
                caller_ura: device,
                callee_ura: authority,
                subject_ura: "easynet:///r/test/agent/alice.worker",
                ability_ura: &ability_ura,
                daemon_ura: Some(authority),
                action: AccessAction::Manage,
            })
            .is_some_and(|purpose| purpose.is(DeviceCallerPurpose::PublicationCustody)),
            "Device custody must carry a hosted Agent's owner projection without becoming its owner");
        assert!(admitted_device_policy_purpose(DeviceCallerPolicyScope {
                caller_ura: device,
                callee_ura: authority,
                subject_ura: "easynet:///r/test/service/alice.pages",
                ability_ura: &ability_ura,
                daemon_ura: Some(authority),
                action: AccessAction::Manage,
            })
            .is_some_and(|purpose| purpose.is(DeviceCallerPurpose::PublicationCustody)),
            "Device custody must carry a hosted Service's owner projection without becoming its owner");
        assert_eq!(
            admitted_device_policy_purpose(DeviceCallerPolicyScope {
                caller_ura: device,
                callee_ura: authority,
                subject_ura: "easynet:///r/test/user/alice",
                ability_ura: &ability_ura,
                daemon_ura: Some(authority),
                action: AccessAction::Manage,
            }),
            None,
            "User principals are not executable owner projections"
        );
        assert_eq!(
            admitted_device_policy_purpose(DeviceCallerPolicyScope {
                caller_ura: device,
                callee_ura: authority,
                subject_ura: device,
                ability_ura: &ability_ura,
                daemon_ura: Some(authority),
                action: AccessAction::Stream,
            }),
            None
        );
    }

    #[test]
    fn advertise_abilities_publication_purpose_accepts_service_owner_projection_subject() {
        let authority = "easynet:///r/test/authority";
        let device = "easynet:///r/test/device/dev-1";
        let service = "easynet:///r/test/service/alice.pages";
        let proof = verify_device_invocation_purpose(DeviceInvocationPurposeScope {
            caller_ura: device,
            callee_ura: authority,
            subject_ura: service,
            public_ability:
                crate::daemon::ability::conformance::ABILITY_FEDERATION_ADVERTISE_ABILITIES,
            daemon_ura: Some(authority),
            action: AccessAction::Manage,
        })
        .expect("Service owner projection is an explicit Device publication-custody purpose");

        assert!(proof.is(DeviceCallerPurpose::PublicationCustody));
    }

    #[test]
    fn lifecycle_revoke_proof_requires_exact_device_self_geometry() {
        let device = "easynet:///r/test/device/dev-1";
        let authority = "easynet:///r/test/authority";
        let scope = |subject_ura| DeviceInvocationPurposeScope {
            caller_ura: device,
            callee_ura: authority,
            subject_ura,
            public_ability: crate::daemon::ability::conformance::ABILITY_FEDERATION_REVOKE,
            daemon_ura: Some(authority),
            action: AccessAction::Manage,
        };

        let proof = verify_device_invocation_purpose(scope(device)).unwrap();
        assert!(proof.is(DeviceCallerPurpose::LifecycleSelfRevoke));
        let hosted =
            verify_device_invocation_purpose(scope("easynet:///r/test/agent/alice.worker"))
                .expect("same-realm Agent is an explicit hosted-Agent retraction purpose");
        assert!(hosted.is(DeviceCallerPurpose::HostedAgentRetraction));
        for denied in [
            "easynet:///r/test/user/alice",
            "easynet:///r/test/authority",
            "easynet:///r/test/device/other",
            "easynet:///r/other/agent/alice.worker",
        ] {
            assert!(
                verify_device_invocation_purpose(scope(denied)).is_err(),
                "{denied}"
            );
        }
        assert!(
            verify_device_invocation_purpose(DeviceInvocationPurposeScope {
                action: AccessAction::Stream,
                ..scope(device)
            })
            .is_err()
        );
    }

    #[test]
    fn bootstrap_self_identity_requires_selected_authority_and_device_self_subject() {
        let device = "easynet:///r/test/device/dev-1";
        let authority = "easynet:///r/test/authority";
        let scope = |callee_ura, subject_ura| DeviceInvocationPurposeScope {
            caller_ura: device,
            callee_ura,
            subject_ura,
            public_ability:
                crate::daemon::ability::conformance::ABILITY_RUNTIME_BOOTSTRAP_SELF_IDENTITY,
            daemon_ura: Some(authority),
            action: AccessAction::Manage,
        };

        let proof = verify_device_invocation_purpose(scope(authority, device))
            .expect("Authority-owned runtime bootstrap has exact Device self geometry");
        assert!(proof.is(DeviceCallerPurpose::Bootstrap));
        assert!(verify_device_invocation_purpose(scope(device, device)).is_err());
        assert!(verify_device_invocation_purpose(scope(
            authority,
            "easynet:///r/test/device/other",
        ))
        .is_err());
        assert!(
            verify_device_invocation_purpose(scope("easynet:///r/other/authority", device,))
                .is_err()
        );
    }

    #[test]
    fn verified_purpose_is_bound_to_the_selected_ability() {
        let device = "easynet:///r/test/device/dev-1";
        let authority = "easynet:///r/test/authority";
        let session = verify_device_invocation_purpose(DeviceInvocationPurposeScope {
            caller_ura: device,
            callee_ura: authority,
            subject_ura: device,
            public_ability: crate::daemon::ability::conformance::ABILITY_SESSION_OPEN,
            daemon_ura: Some(authority),
            action: AccessAction::Stream,
        })
        .unwrap();

        assert!(session.is(DeviceCallerPurpose::DeviceSelfSession));
        assert!(!session.is(DeviceCallerPurpose::PublicationCustody));
        assert!(!session.is(DeviceCallerPurpose::AbilityCatalogDiff));
        assert!(!session.is(DeviceCallerPurpose::LifecycleSelfRevoke));
        assert!(!session.carries_pairing_token_scope());
    }

    #[test]
    fn admits_owner_projection_publication_with_executable_owner_subject_geometry() {
        let authority = "easynet:///r/test/authority";
        let device = "easynet:///r/test/device/dev-1";
        let agent = "easynet:///r/test/agent/u.chat";
        let service = "easynet:///r/test/service/u.pages";

        assert!(admits_owner_projection_publication_host(
            device,
            authority,
            agent,
            Some(authority),
        ));
        assert!(admits_owner_projection_publication_host(
            device,
            authority,
            service,
            Some(authority),
        ));
        assert!(admits_owner_projection_publication_host(
            device,
            authority,
            device,
            Some(authority),
        ));
        assert!(!admits_owner_projection_publication_host(
            device,
            authority,
            "easynet:///r/other/agent/u.chat",
            Some(authority),
        ));
        assert!(!admits_owner_projection_publication_host(
            device,
            authority,
            "easynet:///r/test/device/dev-2",
            Some(authority),
        ));
        assert!(!admits_owner_projection_publication_host(
            device,
            authority,
            authority,
            Some(authority),
        ));
    }
}
