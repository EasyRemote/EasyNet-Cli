// EasyNet CLI - RFC-014 bootstrap authority verifier
// ==================================================
//
// This module turns a narrow paired-device bootstrap fact into the same
// verified-authority input consumed by PolicyEngine. It deliberately does not
// mutate read models and does not special-case federation handlers.

use sha2::{Digest as _, Sha256};

use easynet_axon::pb::axon::v1::Envelope;

use crate::core::ura::{parse_ura, URAKind};
use crate::daemon::ability::names::device_control;
use crate::daemon::invocation::admission::decision::AccessAction;
use crate::daemon::invocation::dispatch::federation_wrappers::{
    AdvertiseAbilitiesRequest, HeartbeatRequest, JoinRequest,
    ABILITY_FEDERATION_ADVERTISE_ABILITIES, ABILITY_FEDERATION_HEARTBEAT, ABILITY_FEDERATION_JOIN,
    ABILITY_FEDERATION_RESOLVE_KEY,
};
use crate::daemon::trust::anchor::{RealmTrustAnchor, TrustedAgentRole};
use easynet_axon::ResolveKeyRequest;

#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) enum BootstrapAuthorityDecision {
    Verified { authority_id: String },
    NotApplicable,
}

pub(crate) struct BootstrapAuthorityVerifier;

impl BootstrapAuthorityVerifier {
    #[must_use]
    pub(crate) fn verify(
        envelope: &Envelope,
        ability: &str,
        action: AccessAction,
        args: &[u8],
        trust_anchor: &RealmTrustAnchor,
        trusted_role: TrustedAgentRole,
        daemon_ura: Option<&str>,
    ) -> BootstrapAuthorityDecision {
        if trusted_role != TrustedAgentRole::Device {
            return BootstrapAuthorityDecision::NotApplicable;
        }

        let Some(caller_ura) = envelope
            .caller
            .as_ref()
            .map(|caller| caller.ura.trim())
            .filter(|ura| !ura.is_empty())
        else {
            return BootstrapAuthorityDecision::NotApplicable;
        };
        if !is_device_ura(caller_ura) {
            return BootstrapAuthorityDecision::NotApplicable;
        }

        let Some(callee_ura) = envelope
            .callee
            .as_ref()
            .map(|callee| callee.ura.trim())
            .filter(|ura| !ura.is_empty())
        else {
            return BootstrapAuthorityDecision::NotApplicable;
        };
        let Some(subject_ura) = envelope
            .subject
            .as_ref()
            .map(|subject| subject.ura.trim())
            .filter(|ura| !ura.is_empty())
        else {
            return BootstrapAuthorityDecision::NotApplicable;
        };
        let Some(owner) = trust_anchor.lookup_principal_owner(caller_ura) else {
            return BootstrapAuthorityDecision::NotApplicable;
        };
        if owner.owner_user_id.trim().is_empty() || owner.owner_ura.trim().is_empty() {
            return BootstrapAuthorityDecision::NotApplicable;
        }

        match (ability, action) {
            (device_control::SESSION_OPEN, AccessAction::Stream) => {
                if callee_ura != caller_ura || subject_ura != caller_ura {
                    return BootstrapAuthorityDecision::NotApplicable;
                }
            }
            (_, AccessAction::Manage) if subject_ura == caller_ura => {
                if !callee_is_selected_hub(callee_ura, caller_ura, daemon_ura) {
                    return BootstrapAuthorityDecision::NotApplicable;
                }
                if verify_hub_bootstrap_mutation(ability, args, caller_ura).is_none() {
                    return BootstrapAuthorityDecision::NotApplicable;
                }
            }
            (ABILITY_FEDERATION_RESOLVE_KEY, AccessAction::Invoke) => {
                if !callee_is_selected_hub(callee_ura, caller_ura, daemon_ura) {
                    return BootstrapAuthorityDecision::NotApplicable;
                }
                if verify_bootstrap_resolve_key(args, callee_ura, &owner.owner_ura).is_none() {
                    return BootstrapAuthorityDecision::NotApplicable;
                }
            }
            _ => return BootstrapAuthorityDecision::NotApplicable,
        }

        BootstrapAuthorityDecision::Verified {
            authority_id: bootstrap_authority_id(caller_ura, ability, &owner.owner_user_id),
        }
    }
}

fn verify_hub_bootstrap_mutation(ability: &str, args: &[u8], caller_ura: &str) -> Option<()> {
    match ability {
        ABILITY_FEDERATION_JOIN => {
            let Ok(request) = serde_json::from_slice::<JoinRequest>(args) else {
                return None;
            };
            if request.membership_ura.trim() != caller_ura {
                return None;
            }
        }
        ABILITY_FEDERATION_ADVERTISE_ABILITIES => {
            let Ok(request) = serde_json::from_slice::<AdvertiseAbilitiesRequest>(args) else {
                return None;
            };
            if request.owner_ura.trim() != caller_ura
                || request.host_device_ura.trim() != caller_ura
            {
                return None;
            }
        }
        ABILITY_FEDERATION_HEARTBEAT => {
            let Ok(request) = serde_json::from_slice::<HeartbeatRequest>(args) else {
                return None;
            };
            if request
                .refresh_owner_uras
                .iter()
                .any(|owner_ura| owner_ura.trim() != caller_ura)
            {
                return None;
            }
        }
        _ => return None,
    }
    Some(())
}

fn verify_bootstrap_resolve_key(args: &[u8], hub_ura: &str, owner_ura: &str) -> Option<()> {
    let Ok(request) = serde_json::from_slice::<ResolveKeyRequest>(args) else {
        return None;
    };
    let agent_ura = request.agent_ura.trim();
    if agent_ura == hub_ura || agent_ura == owner_ura {
        return Some(());
    }
    None
}

fn is_device_ura(ura: &str) -> bool {
    parse_ura(ura)
        .map(|parsed| parsed.kind == URAKind::Device && parsed.device_id().is_some())
        .unwrap_or(false)
}

fn callee_is_selected_hub(callee_ura: &str, caller_ura: &str, daemon_ura: Option<&str>) -> bool {
    if daemon_ura.is_some_and(|daemon| daemon != callee_ura) {
        return false;
    }
    let Ok(callee) = parse_ura(callee_ura) else {
        return false;
    };
    let Ok(caller) = parse_ura(caller_ura) else {
        return false;
    };
    callee.kind == URAKind::Hub && callee.realm == caller.realm
}

fn bootstrap_authority_id(caller_ura: &str, ability: &str, owner_user_id: &str) -> String {
    let mut hasher = Sha256::new();
    hasher.update(caller_ura.as_bytes());
    hasher.update([0]);
    hasher.update(ability.as_bytes());
    hasher.update([0]);
    hasher.update(owner_user_id.as_bytes());
    format!(
        "device_bootstrap_authority:sha256:{}",
        hex::encode(hasher.finalize())
    )
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::daemon::trust::anchor::TrustedPrincipalOwner;
    use easynet_axon::pb::axon::v1::{AgentIdentity, SubjectIdentity};

    fn envelope(caller: &str, callee: &str, subject: &str) -> Envelope {
        Envelope {
            caller: Some(AgentIdentity {
                ura: caller.to_string(),
                profile: String::new(),
            }),
            callee: Some(AgentIdentity {
                ura: callee.to_string(),
                profile: String::new(),
            }),
            subject: Some(SubjectIdentity {
                ura: subject.to_string(),
                profile: String::new(),
            }),
            ..Envelope::default()
        }
    }

    fn anchor() -> RealmTrustAnchor {
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
        .expect("anchor")
    }

    #[test]
    fn paired_device_can_publish_own_owner_projection() {
        let args = serde_json::to_vec(&serde_json::json!({
            "owner_ura": "easynet:///r/test/device/dev-1",
            "host_device_ura": "easynet:///r/test/device/dev-1",
            "projection_revision": 1,
            "projection_digest": "digest",
            "lease_expires_unix_ms": 0,
            "ability_summaries": [],
        }))
        .expect("args");

        let got = BootstrapAuthorityVerifier::verify(
            &envelope(
                "easynet:///r/test/device/dev-1",
                "easynet:///r/test/hub",
                "easynet:///r/test/device/dev-1",
            ),
            ABILITY_FEDERATION_ADVERTISE_ABILITIES,
            AccessAction::Manage,
            &args,
            &anchor(),
            TrustedAgentRole::Device,
            Some("easynet:///r/test/hub"),
        );

        assert!(matches!(got, BootstrapAuthorityDecision::Verified { .. }));
    }

    #[test]
    fn paired_device_cannot_publish_other_owner_projection() {
        let args = serde_json::to_vec(&serde_json::json!({
            "owner_ura": "easynet:///r/test/device/other",
            "host_device_ura": "easynet:///r/test/device/dev-1",
            "projection_revision": 1,
            "projection_digest": "digest",
            "lease_expires_unix_ms": 0,
            "ability_summaries": [],
        }))
        .expect("args");

        let got = BootstrapAuthorityVerifier::verify(
            &envelope(
                "easynet:///r/test/device/dev-1",
                "easynet:///r/test/hub",
                "easynet:///r/test/device/dev-1",
            ),
            ABILITY_FEDERATION_ADVERTISE_ABILITIES,
            AccessAction::Manage,
            &args,
            &anchor(),
            TrustedAgentRole::Device,
            Some("easynet:///r/test/hub"),
        );

        assert_eq!(got, BootstrapAuthorityDecision::NotApplicable);
    }

    #[test]
    fn paired_device_can_open_own_session_carrier() {
        let got = BootstrapAuthorityVerifier::verify(
            &envelope(
                "easynet:///r/test/device/dev-1",
                "easynet:///r/test/device/dev-1",
                "easynet:///r/test/device/dev-1",
            ),
            device_control::SESSION_OPEN,
            AccessAction::Stream,
            &[],
            &anchor(),
            TrustedAgentRole::Device,
            Some("easynet:///r/test/hub"),
        );

        assert!(matches!(got, BootstrapAuthorityDecision::Verified { .. }));
    }

    #[test]
    fn paired_device_cannot_open_session_for_other_device() {
        let got = BootstrapAuthorityVerifier::verify(
            &envelope(
                "easynet:///r/test/device/dev-1",
                "easynet:///r/test/device/other",
                "easynet:///r/test/device/dev-1",
            ),
            device_control::SESSION_OPEN,
            AccessAction::Stream,
            &[],
            &anchor(),
            TrustedAgentRole::Device,
            Some("easynet:///r/test/hub"),
        );

        assert_eq!(got, BootstrapAuthorityDecision::NotApplicable);
    }

    #[test]
    fn paired_device_can_refresh_own_heartbeat_projection() {
        let args = serde_json::to_vec(&serde_json::json!({
            "refresh_owner_uras": ["easynet:///r/test/device/dev-1"],
        }))
        .expect("args");

        let got = BootstrapAuthorityVerifier::verify(
            &envelope(
                "easynet:///r/test/device/dev-1",
                "easynet:///r/test/hub",
                "easynet:///r/test/device/dev-1",
            ),
            ABILITY_FEDERATION_HEARTBEAT,
            AccessAction::Manage,
            &args,
            &anchor(),
            TrustedAgentRole::Device,
            Some("easynet:///r/test/hub"),
        );

        assert!(matches!(got, BootstrapAuthorityDecision::Verified { .. }));
    }

    #[test]
    fn paired_device_cannot_refresh_other_heartbeat_projection() {
        let args = serde_json::to_vec(&serde_json::json!({
            "refresh_owner_uras": ["easynet:///r/test/agent/alice.worker"],
        }))
        .expect("args");

        let got = BootstrapAuthorityVerifier::verify(
            &envelope(
                "easynet:///r/test/device/dev-1",
                "easynet:///r/test/hub",
                "easynet:///r/test/device/dev-1",
            ),
            ABILITY_FEDERATION_HEARTBEAT,
            AccessAction::Manage,
            &args,
            &anchor(),
            TrustedAgentRole::Device,
            Some("easynet:///r/test/hub"),
        );

        assert_eq!(got, BootstrapAuthorityDecision::NotApplicable);
    }

    #[test]
    fn device_without_owner_binding_has_no_bootstrap_authority() {
        let args = serde_json::to_vec(&serde_json::json!({
            "membership_ura": "easynet:///r/test/device/dev-1",
            "agent_ura": "easynet:///r/test/device/dev-1",
            "public_key_hex": "00",
            "realm": "test",
        }))
        .expect("args");

        let got = BootstrapAuthorityVerifier::verify(
            &envelope(
                "easynet:///r/test/device/dev-1",
                "easynet:///r/test/hub",
                "easynet:///r/test/device/dev-1",
            ),
            ABILITY_FEDERATION_JOIN,
            AccessAction::Manage,
            &args,
            &RealmTrustAnchor::default(),
            TrustedAgentRole::Device,
            Some("easynet:///r/test/hub"),
        );

        assert_eq!(got, BootstrapAuthorityDecision::NotApplicable);
    }

    #[test]
    fn paired_device_can_resolve_selected_hub_key_during_bootstrap() {
        let args = serde_json::to_vec(&serde_json::json!({
            "agent_ura": "easynet:///r/test/hub",
        }))
        .expect("args");

        let subject = crate::core::ura::owner_ability_ura(
            "easynet:///r/test/hub",
            ABILITY_FEDERATION_RESOLVE_KEY,
        )
        .expect("subject");
        let got = BootstrapAuthorityVerifier::verify(
            &envelope(
                "easynet:///r/test/device/dev-1",
                "easynet:///r/test/hub",
                &subject,
            ),
            ABILITY_FEDERATION_RESOLVE_KEY,
            AccessAction::Invoke,
            &args,
            &anchor(),
            TrustedAgentRole::Device,
            Some("easynet:///r/test/hub"),
        );

        assert!(matches!(got, BootstrapAuthorityDecision::Verified { .. }));
    }

    #[test]
    fn paired_device_can_resolve_own_user_key_during_bootstrap() {
        let args = serde_json::to_vec(&serde_json::json!({
            "agent_ura": "easynet:///r/test/user/alice",
        }))
        .expect("args");

        let subject = crate::core::ura::owner_ability_ura(
            "easynet:///r/test/hub",
            ABILITY_FEDERATION_RESOLVE_KEY,
        )
        .expect("subject");
        let got = BootstrapAuthorityVerifier::verify(
            &envelope(
                "easynet:///r/test/device/dev-1",
                "easynet:///r/test/hub",
                &subject,
            ),
            ABILITY_FEDERATION_RESOLVE_KEY,
            AccessAction::Invoke,
            &args,
            &anchor(),
            TrustedAgentRole::Device,
            Some("easynet:///r/test/hub"),
        );

        assert!(matches!(got, BootstrapAuthorityDecision::Verified { .. }));
    }

    #[test]
    fn paired_device_cannot_resolve_unowned_agent_key_during_bootstrap() {
        let args = serde_json::to_vec(&serde_json::json!({
            "agent_ura": "easynet:///r/test/agent/alice.worker",
        }))
        .expect("args");

        let subject = crate::core::ura::owner_ability_ura(
            "easynet:///r/test/hub",
            ABILITY_FEDERATION_RESOLVE_KEY,
        )
        .expect("subject");
        let got = BootstrapAuthorityVerifier::verify(
            &envelope(
                "easynet:///r/test/device/dev-1",
                "easynet:///r/test/hub",
                &subject,
            ),
            ABILITY_FEDERATION_RESOLVE_KEY,
            AccessAction::Invoke,
            &args,
            &anchor(),
            TrustedAgentRole::Device,
            Some("easynet:///r/test/hub"),
        );

        assert_eq!(got, BootstrapAuthorityDecision::NotApplicable);
    }
}
