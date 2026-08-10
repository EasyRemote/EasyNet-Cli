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

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum DeviceCallerPurpose {
    Bootstrap,
    Pairing,
    PublicationCustody,
    Liveness,
    SessionControl,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum DeviceCallerPolicyAdmission {
    None,
    AuthoritySelf { action: AccessAction },
    AuthorityOwnerProjection { action: AccessAction },
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
struct DeviceCallerRule {
    public_ability: &'static str,
    purpose: DeviceCallerPurpose,
    policy_admission: DeviceCallerPolicyAdmission,
}

const DEVICE_CALLER_RULES: &[DeviceCallerRule] = &[
    DeviceCallerRule {
        public_ability:
            crate::daemon::ability::conformance::ABILITY_RUNTIME_BOOTSTRAP_SELF_IDENTITY,
        purpose: DeviceCallerPurpose::Bootstrap,
        policy_admission: DeviceCallerPolicyAdmission::None,
    },
    DeviceCallerRule {
        public_ability: crate::daemon::ability::conformance::ABILITY_IDENTITY_REGISTER_PUBKEY,
        purpose: DeviceCallerPurpose::Pairing,
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
        policy_admission: DeviceCallerPolicyAdmission::None,
    },
    DeviceCallerRule {
        public_ability: crate::daemon::ability::conformance::ABILITY_FEDERATION_ADVERTISE_AGENT,
        purpose: DeviceCallerPurpose::PublicationCustody,
        policy_admission: DeviceCallerPolicyAdmission::None,
    },
    DeviceCallerRule {
        public_ability: crate::daemon::ability::conformance::ABILITY_FEDERATION_ADVERTISE_ABILITIES,
        purpose: DeviceCallerPurpose::PublicationCustody,
        policy_admission: DeviceCallerPolicyAdmission::AuthorityOwnerProjection {
            action: AccessAction::Manage,
        },
    },
    DeviceCallerRule {
        public_ability:
            crate::daemon::invocation::dispatch::federation_wrappers::ABILITY_FEDERATION_HEARTBEAT,
        purpose: DeviceCallerPurpose::Liveness,
        // BootstrapAuthorityVerifier validates that every refreshed owner is
        // exactly the caller Device before PolicyEngine sees an authority
        // proof. No generic policy exception is needed here.
        policy_admission: DeviceCallerPolicyAdmission::None,
    },
    DeviceCallerRule {
        public_ability: crate::daemon::ability::conformance::ABILITY_SESSION_OPEN,
        purpose: DeviceCallerPurpose::SessionControl,
        policy_admission: DeviceCallerPolicyAdmission::AuthoritySelf {
            action: AccessAction::Stream,
        },
    },
];

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum CallerKindAdmission {
    NonDevice,
    Device(DeviceCallerPurpose),
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) enum DeviceCallerAdmissionError {
    InvalidCallerUra(String),
    NonActorCaller { kind: URAKind },
    DeviceCallerNotAllowed { public_ability: String },
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
        URAKind::User | URAKind::Agent | URAKind::Authority => Ok(CallerKindAdmission::NonDevice),
        URAKind::Device => public_device_caller_purpose(public_ability)
            .map(CallerKindAdmission::Device)
            .ok_or_else(|| DeviceCallerAdmissionError::DeviceCallerNotAllowed {
                public_ability: public_ability.to_string(),
            }),
        URAKind::Ability | URAKind::Resource | URAKind::Unknown => {
            Err(DeviceCallerAdmissionError::NonActorCaller { kind: caller_kind })
        }
    }
}

pub(crate) fn public_device_caller_purpose(public_ability: &str) -> Option<DeviceCallerPurpose> {
    device_caller_rule(public_ability).map(|rule| rule.purpose)
}

fn device_caller_rule(public_ability: &str) -> Option<&'static DeviceCallerRule> {
    let public_ability = public_ability.trim();
    DEVICE_CALLER_RULES
        .iter()
        .find(|rule| rule.public_ability == public_ability)
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
) -> Option<DeviceCallerPurpose> {
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
        };
        if scope.action != action {
            return None;
        }
        let expected_ability =
            crate::core::ura::owner_ability_ura(scope.callee_ura, rule.public_ability)?;
        (scope.ability_ura == expected_ability).then_some(rule.purpose)
    })
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
        URAKind::Agent => true,
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
    fn classifies_public_device_caller_purposes() {
        assert_eq!(
            public_device_caller_purpose(
                crate::daemon::ability::conformance::ABILITY_RUNTIME_BOOTSTRAP_SELF_IDENTITY
            ),
            Some(DeviceCallerPurpose::Bootstrap)
        );
        assert_eq!(
            public_device_caller_purpose(
                crate::daemon::ability::conformance::ABILITY_IDENTITY_REGISTER_PUBKEY
            ),
            Some(DeviceCallerPurpose::Pairing)
        );
        assert_eq!(
            public_device_caller_purpose(
                crate::daemon::ability::conformance::ABILITY_FEDERATION_RESOLVE_KEY
            ),
            Some(DeviceCallerPurpose::Pairing)
        );
        assert_eq!(
            public_device_caller_purpose(crate::daemon::ability::conformance::ABILITY_SESSION_OPEN),
            Some(DeviceCallerPurpose::SessionControl)
        );
        assert_eq!(
            public_device_caller_purpose(
                crate::daemon::ability::conformance::ABILITY_FEDERATION_ADVERTISE_AGENT
            ),
            Some(DeviceCallerPurpose::PublicationCustody)
        );
        assert_eq!(
            public_device_caller_purpose(
                crate::daemon::ability::conformance::ABILITY_FEDERATION_ADVERTISE_ABILITIES
            ),
            Some(DeviceCallerPurpose::PublicationCustody)
        );
        assert_eq!(
            public_device_caller_purpose(
                crate::daemon::invocation::dispatch::federation_wrappers::ABILITY_FEDERATION_HEARTBEAT
            ),
            Some(DeviceCallerPurpose::Liveness)
        );
        assert_eq!(public_device_caller_purpose("ability.publish"), None);
        assert_eq!(public_device_caller_purpose("ability.deploy"), None);
        assert_eq!(public_device_caller_purpose("ability.uninstall"), None);
        assert_eq!(public_device_caller_purpose("observe.health"), None);
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

        assert_eq!(
            admitted_device_policy_purpose(DeviceCallerPolicyScope {
                caller_ura: device,
                callee_ura: authority,
                subject_ura: device,
                ability_ura: &ability_ura,
                daemon_ura: Some(authority),
                action: AccessAction::Manage,
            }),
            Some(DeviceCallerPurpose::PublicationCustody)
        );
        assert_eq!(
            admitted_device_policy_purpose(DeviceCallerPolicyScope {
                caller_ura: device,
                callee_ura: authority,
                subject_ura: "easynet:///r/test/agent/alice.worker",
                ability_ura: &ability_ura,
                daemon_ura: Some(authority),
                action: AccessAction::Manage,
            }),
            Some(DeviceCallerPurpose::PublicationCustody),
            "Device custody must carry a hosted Agent's owner projection without becoming its owner"
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
    fn admits_owner_projection_publication_with_agent_subject_geometry() {
        let authority = "easynet:///r/test/authority";
        let device = "easynet:///r/test/device/dev-1";
        let agent = "easynet:///r/test/agent/u.chat";

        assert!(admits_owner_projection_publication_host(
            device,
            authority,
            agent,
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
