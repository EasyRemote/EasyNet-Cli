//! EasyNet Axon for AgentNet
//! =========================
//!
//! File: src/daemon/invocation/admission/local_device_resource_authority.rs
//! Description: Authorize a paired User's exact local Device-owned Resource.
//!
//! Protocol Responsibility:
//! - Project the persisted Device-to-User pairing into one bounded ownership fact.
//! - Require exact User, realm, Device, Resource owner, and SystemAgent sponsorship.
//!
//! Implementation Approach:
//! - Load and validate the local Device/User binding fail-closed.
//! - Evaluate the signed session tuple as one explicit terminal decision.
//!
//! Usage Contract:
//! - Call only after signature and exact SessionAuthority bindings are verified.
//! - Never use this policy for Agent subjects or as a substitute for descriptor policy.
//!
//! Architectural Position:
//! - Core daemon runtime admission policy, below handlers and above persistence.

use std::fmt;

use crate::core::ura::{parse_ura, URAKind};

/// Exact signed facts needed to project paired-User authority onto one
/// Device-owned Resource.
pub(crate) struct UserSessionDeviceResourceTuple<'a> {
    pub issuer_ura: &'a str,
    pub caller_ura: &'a str,
    pub session_owner_user_id: &'a str,
    pub callee_ura: &'a str,
    pub subject_ura: &'a str,
}

/// Terminal outcome of local Device-resource ownership evaluation.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum LocalDeviceResourceAuthorityDecision {
    Authorized,
    Denied(LocalDeviceResourceAuthorityDenyReason),
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum LocalDeviceResourceAuthorityDenyReason {
    LocalBindingUnavailable,
    LocalBindingInconsistent,
    IssuerCallerMismatch,
    SessionOwnerMismatch,
    PairedUserMismatch,
    SubjectNotDeviceOwnedResource,
    CalleeNotDeviceSponsoredSystemAgent,
    DeviceMismatch,
    RealmMismatch,
}

impl fmt::Display for LocalDeviceResourceAuthorityDenyReason {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str(match self {
            Self::LocalBindingUnavailable => "local Device/User pairing is unavailable",
            Self::LocalBindingInconsistent => {
                "persisted Device identity and User pairing are inconsistent"
            }
            Self::IssuerCallerMismatch => {
                "authority issuer and invocation caller must be the same canonical User"
            }
            Self::SessionOwnerMismatch => {
                "session owner must equal the canonical authority issuer User"
            }
            Self::PairedUserMismatch => {
                "authority issuer is not the User paired to the local Device"
            }
            Self::SubjectNotDeviceOwnedResource => {
                "subject must be a canonical Device-owned Resource"
            }
            Self::CalleeNotDeviceSponsoredSystemAgent => {
                "callee must be a canonical device-sponsored SystemAgent"
            }
            Self::DeviceMismatch => {
                "Resource owner, SystemAgent sponsor, and local Device must match exactly"
            }
            Self::RealmMismatch => {
                "User, Resource, SystemAgent, local Device, and pairing realms must match exactly"
            }
        })
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
struct LocalDeviceUserBinding {
    realm: String,
    device_id: String,
    user_id: String,
}

impl LocalDeviceUserBinding {
    fn load() -> Result<Self, LocalDeviceResourceAuthorityDenyReason> {
        let credentials = crate::daemon::persistence::config::load_credentials_optional()
            .map_err(|_| LocalDeviceResourceAuthorityDenyReason::LocalBindingUnavailable)?
            .ok_or(LocalDeviceResourceAuthorityDenyReason::LocalBindingUnavailable)?;
        let user_id = credentials
            .user_id()
            .map_err(|_| LocalDeviceResourceAuthorityDenyReason::LocalBindingUnavailable)?;
        let local_device_ura = crate::daemon::identity::local_invocation::local_device_ura()
            .map_err(|_| LocalDeviceResourceAuthorityDenyReason::LocalBindingUnavailable)?;
        let local_device = parse_ura(&local_device_ura)
            .map_err(|_| LocalDeviceResourceAuthorityDenyReason::LocalBindingInconsistent)?;
        let device_id = local_device
            .device_id()
            .ok_or(LocalDeviceResourceAuthorityDenyReason::LocalBindingInconsistent)?;
        if local_device.kind != URAKind::Device
            || credentials.realm.trim() != local_device.realm
            || credentials.node_id.trim() != device_id
        {
            return Err(LocalDeviceResourceAuthorityDenyReason::LocalBindingInconsistent);
        }
        let device_id = device_id.to_string();
        Ok(Self {
            realm: local_device.realm,
            device_id,
            user_id: user_id.to_string(),
        })
    }

    fn evaluate(
        &self,
        tuple: UserSessionDeviceResourceTuple<'_>,
    ) -> LocalDeviceResourceAuthorityDecision {
        use LocalDeviceResourceAuthorityDecision::{Authorized, Denied};
        use LocalDeviceResourceAuthorityDenyReason as Deny;

        let Ok(issuer) = parse_ura(tuple.issuer_ura) else {
            return Denied(Deny::IssuerCallerMismatch);
        };
        let Ok(caller) = parse_ura(tuple.caller_ura) else {
            return Denied(Deny::IssuerCallerMismatch);
        };
        if issuer.kind != URAKind::User
            || caller.kind != URAKind::User
            || tuple.issuer_ura != tuple.caller_ura
            || issuer.user_id() != caller.user_id()
        {
            return Denied(Deny::IssuerCallerMismatch);
        }
        let Some(issuer_user_id) = issuer.user_id() else {
            return Denied(Deny::IssuerCallerMismatch);
        };
        if tuple.session_owner_user_id != issuer_user_id {
            return Denied(Deny::SessionOwnerMismatch);
        }
        if self.user_id != issuer_user_id {
            return Denied(Deny::PairedUserMismatch);
        }

        let Ok(subject) = parse_ura(tuple.subject_ura) else {
            return Denied(Deny::SubjectNotDeviceOwnedResource);
        };
        let Some(subject_device_id) = subject
            .resource_owner_id()
            .and_then(|owner| owner.strip_prefix("device."))
        else {
            return Denied(Deny::SubjectNotDeviceOwnedResource);
        };
        if subject.kind != URAKind::Resource || subject.resource_path().is_none() {
            return Denied(Deny::SubjectNotDeviceOwnedResource);
        }

        let Ok(callee) = parse_ura(tuple.callee_ura) else {
            return Denied(Deny::CalleeNotDeviceSponsoredSystemAgent);
        };
        let Some((callee_device_id, _system_agent_id)) = callee.device_agent_ids() else {
            return Denied(Deny::CalleeNotDeviceSponsoredSystemAgent);
        };
        if callee.kind != URAKind::Agent {
            return Denied(Deny::CalleeNotDeviceSponsoredSystemAgent);
        }

        if subject_device_id != self.device_id || callee_device_id != self.device_id {
            return Denied(Deny::DeviceMismatch);
        }
        if issuer.realm != self.realm
            || caller.realm != self.realm
            || subject.realm != self.realm
            || callee.realm != self.realm
        {
            return Denied(Deny::RealmMismatch);
        }
        Authorized
    }
}

pub(crate) fn authorize_user_session_device_resource(
    tuple: UserSessionDeviceResourceTuple<'_>,
) -> LocalDeviceResourceAuthorityDecision {
    match LocalDeviceUserBinding::load() {
        Ok(binding) => binding.evaluate(tuple),
        Err(reason) => LocalDeviceResourceAuthorityDecision::Denied(reason),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn binding() -> LocalDeviceUserBinding {
        LocalDeviceUserBinding {
            realm: "example".to_string(),
            device_id: "dev-a".to_string(),
            user_id: "alice".to_string(),
        }
    }

    fn tuple<'a>() -> UserSessionDeviceResourceTuple<'a> {
        UserSessionDeviceResourceTuple {
            issuer_ura: "easynet:///r/example/user/alice",
            caller_ura: "easynet:///r/example/user/alice",
            session_owner_user_id: "alice",
            callee_ura: "easynet:///r/example/agent/device.dev-a.plugin-management",
            subject_ura: "easynet:///r/example/resource/device.dev-a/streams/display.01",
        }
    }

    #[test]
    fn exact_paired_user_device_resource_tuple_is_authorized() {
        assert_eq!(
            binding().evaluate(tuple()),
            LocalDeviceResourceAuthorityDecision::Authorized
        );
    }

    #[test]
    fn another_device_resource_is_denied() {
        let mut tuple = tuple();
        tuple.subject_ura = "easynet:///r/example/resource/device.dev-b/streams/display.01";
        assert_eq!(
            binding().evaluate(tuple),
            LocalDeviceResourceAuthorityDecision::Denied(
                LocalDeviceResourceAuthorityDenyReason::DeviceMismatch
            )
        );
    }

    #[test]
    fn hosted_agent_callee_is_not_a_device_system_agent() {
        let mut tuple = tuple();
        tuple.callee_ura = "easynet:///r/example/agent/service.remote-desktop";
        assert_eq!(
            binding().evaluate(tuple),
            LocalDeviceResourceAuthorityDecision::Denied(
                LocalDeviceResourceAuthorityDenyReason::CalleeNotDeviceSponsoredSystemAgent
            )
        );
    }

    #[test]
    fn wrong_session_owner_is_denied() {
        let mut tuple = tuple();
        tuple.session_owner_user_id = "mallory";
        assert_eq!(
            binding().evaluate(tuple),
            LocalDeviceResourceAuthorityDecision::Denied(
                LocalDeviceResourceAuthorityDenyReason::SessionOwnerMismatch
            )
        );
    }

    #[test]
    fn cross_realm_callee_is_denied() {
        let mut tuple = tuple();
        tuple.callee_ura = "easynet:///r/other/agent/device.dev-a.plugin-management";
        assert_eq!(
            binding().evaluate(tuple),
            LocalDeviceResourceAuthorityDecision::Denied(
                LocalDeviceResourceAuthorityDenyReason::RealmMismatch
            )
        );
    }
}
