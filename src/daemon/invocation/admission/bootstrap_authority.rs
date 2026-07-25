// EasyNet CLI - RFC-014 bootstrap authority verifier
// ==================================================
//
// This module turns bounded control-plane authority facts into the same
// verified-authority input consumed by PolicyEngine. It deliberately does not
// mutate read models and does not bypass signature admission.

use base64::{engine::general_purpose::STANDARD as BASE64_STANDARD, Engine as _};
use sha2::{Digest as _, Sha256};

use axon_sdk::pb::axon::v1::Envelope;

use crate::core::ura::{parse_ura, URAKind};
use crate::daemon::ability::names::device_control;
use crate::daemon::federation::wire_contract::ResolveKeyRequest;
use crate::daemon::invocation::admission::decision::AccessAction;
use crate::daemon::invocation::admission::hosted_agent_publication::HostedAgentPublication;
use crate::daemon::invocation::admission::owner_resolution::{local_device_owner_fact, OwnerFact};
use crate::daemon::invocation::admission::register_device_pubkey::ABILITY_IDENTITY_REGISTER_PUBKEY;
use crate::daemon::invocation::dispatch::federation_wrappers::{
    AdvertiseAbilitiesRequest, AdvertiseAgentRequest, HeartbeatRequest, JoinRequest,
    ABILITY_FEDERATION_ADVERTISE_ABILITIES, ABILITY_FEDERATION_ADVERTISE_AGENT,
    ABILITY_FEDERATION_HEARTBEAT, ABILITY_FEDERATION_JOIN, ABILITY_FEDERATION_RESOLVE_KEY,
};
use crate::daemon::trust::anchor::{RealmTrustAnchor, TrustedAgentRole};

#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) enum BootstrapAuthorityDecision {
    Verified { authority_id: String },
    Unavailable { message: String },
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
        let Some(caller_ura) = envelope
            .caller
            .as_ref()
            .map(|caller| caller.ura.trim())
            .filter(|ura| !ura.is_empty())
        else {
            return BootstrapAuthorityDecision::NotApplicable;
        };
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

        if trusted_role == TrustedAgentRole::Hub {
            return verify_hub_link_authority(
                caller_ura, callee_ura, ability, action, args, daemon_ura,
            );
        }

        if trusted_role != TrustedAgentRole::Device || !is_device_ura(caller_ura) {
            return BootstrapAuthorityDecision::NotApplicable;
        }

        if ability == ABILITY_FEDERATION_RESOLVE_KEY && action == AccessAction::Read {
            let authority_key_decision = verify_device_authority_key_bootstrap_authority(
                caller_ura,
                callee_ura,
                subject_ura,
                ability,
                args,
                daemon_ura,
            );
            if matches!(
                authority_key_decision,
                BootstrapAuthorityDecision::Verified { .. }
            ) {
                return authority_key_decision;
            }
        }

        let owner = match trust_anchor.lookup_principal_owner(caller_ura) {
            Some(owner) => Some(OwnerFact::user(
                owner.owner_user_id.clone(),
                owner.owner_ura.clone(),
            )),
            None => match local_device_owner_fact(caller_ura) {
                Ok(owner) => owner,
                Err(error) => {
                    return BootstrapAuthorityDecision::Unavailable {
                        message: format!("LOCAL_BOOTSTRAP_OWNER_UNAVAILABLE: {error:#}"),
                    };
                }
            },
        };
        let Some(owner) = owner else {
            return BootstrapAuthorityDecision::NotApplicable;
        };
        let Some(owner_user_id) = owner
            .owner_user_id
            .as_deref()
            .filter(|value| !value.trim().is_empty())
        else {
            return BootstrapAuthorityDecision::NotApplicable;
        };
        let Some(owner_ura) = owner
            .owner_ura
            .as_deref()
            .filter(|value| !value.trim().is_empty())
        else {
            return BootstrapAuthorityDecision::NotApplicable;
        };

        match (ability, action) {
            (device_control::SESSION_OPEN, AccessAction::Stream) => {
                if callee_ura != caller_ura || subject_ura != caller_ura {
                    return BootstrapAuthorityDecision::NotApplicable;
                }
            }
            (ABILITY_FEDERATION_ADVERTISE_AGENT, AccessAction::Manage) => {
                let Ok(request) = serde_json::from_slice::<AdvertiseAgentRequest>(args) else {
                    return BootstrapAuthorityDecision::NotApplicable;
                };
                let Ok(publication) =
                    HostedAgentPublication::verify(envelope, &request, trust_anchor, daemon_ura)
                else {
                    return BootstrapAuthorityDecision::NotApplicable;
                };
                return BootstrapAuthorityDecision::Verified {
                    authority_id: bootstrap_authority_id(
                        publication.caller_device_ura(),
                        ability,
                        publication.owner_user_id(),
                    ),
                };
            }
            (_, AccessAction::Manage) if subject_ura == caller_ura => {
                if !callee_is_selected_authority(callee_ura, caller_ura, daemon_ura) {
                    return BootstrapAuthorityDecision::NotApplicable;
                }
                if verify_authority_bootstrap_mutation(ability, args, caller_ura).is_none() {
                    return BootstrapAuthorityDecision::NotApplicable;
                }
            }
            (ABILITY_FEDERATION_RESOLVE_KEY, AccessAction::Read) => {
                if !callee_is_selected_authority(callee_ura, caller_ura, daemon_ura) {
                    return BootstrapAuthorityDecision::NotApplicable;
                }
                if verify_bootstrap_resolve_key(args, callee_ura, owner_ura).is_none() {
                    return BootstrapAuthorityDecision::NotApplicable;
                }
            }
            (ABILITY_IDENTITY_REGISTER_PUBKEY, AccessAction::Manage) => {
                if !callee_is_selected_authority(callee_ura, caller_ura, daemon_ura) {
                    return BootstrapAuthorityDecision::NotApplicable;
                }
                if verify_bootstrap_owner_user_key(args, owner_ura).is_none() {
                    return BootstrapAuthorityDecision::NotApplicable;
                }
            }
            _ => return BootstrapAuthorityDecision::NotApplicable,
        }

        BootstrapAuthorityDecision::Verified {
            authority_id: bootstrap_authority_id(caller_ura, ability, owner_user_id),
        }
    }
}

fn verify_hub_link_authority(
    caller_ura: &str,
    callee_ura: &str,
    ability: &str,
    action: AccessAction,
    args: &[u8],
    daemon_ura: Option<&str>,
) -> BootstrapAuthorityDecision {
    if ability != ABILITY_FEDERATION_RESOLVE_KEY || action != AccessAction::Read {
        return BootstrapAuthorityDecision::NotApplicable;
    }
    if !is_authority_ura(caller_ura) || !callee_is_current_authority(callee_ura, daemon_ura) {
        return BootstrapAuthorityDecision::NotApplicable;
    }
    if verify_hub_link_resolve_key(args, callee_ura).is_none() {
        return BootstrapAuthorityDecision::NotApplicable;
    }
    BootstrapAuthorityDecision::Verified {
        authority_id: hub_link_authority_id(caller_ura, callee_ura, ability),
    }
}

fn verify_device_authority_key_bootstrap_authority(
    caller_ura: &str,
    callee_ura: &str,
    subject_ura: &str,
    ability: &str,
    args: &[u8],
    daemon_ura: Option<&str>,
) -> BootstrapAuthorityDecision {
    if !callee_is_selected_authority(callee_ura, caller_ura, daemon_ura) {
        return BootstrapAuthorityDecision::NotApplicable;
    }
    if crate::core::ura::owner_ability_ura(callee_ura, ability).as_deref() != Some(subject_ura) {
        return BootstrapAuthorityDecision::NotApplicable;
    }
    if verify_bootstrap_selected_authority_key(args, callee_ura).is_none() {
        return BootstrapAuthorityDecision::NotApplicable;
    }
    BootstrapAuthorityDecision::Verified {
        authority_id: device_hub_key_bootstrap_authority_id(caller_ura, callee_ura, ability),
    }
}

fn verify_authority_bootstrap_mutation(ability: &str, args: &[u8], caller_ura: &str) -> Option<()> {
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

fn verify_bootstrap_selected_authority_key(args: &[u8], authority_ura: &str) -> Option<()> {
    let Ok(request) = serde_json::from_slice::<ResolveKeyRequest>(args) else {
        return None;
    };
    (request.agent_ura.trim() == authority_ura).then_some(())
}

fn verify_bootstrap_resolve_key(args: &[u8], authority_ura: &str, owner_ura: &str) -> Option<()> {
    let Ok(request) = serde_json::from_slice::<ResolveKeyRequest>(args) else {
        return None;
    };
    let agent_ura = request.agent_ura.trim();
    if agent_ura == authority_ura || agent_ura == owner_ura {
        return Some(());
    }
    None
}

/// Pairing bootstrap may seed exactly the paired owner's user signing key at
/// the selected authority. It is intentionally narrower than general identity
/// mutation: a device cannot author another user, a device key, or an authority key.
fn verify_bootstrap_owner_user_key(args: &[u8], owner_ura: &str) -> Option<()> {
    #[derive(serde::Deserialize)]
    struct UserKeyRegistration {
        agent_ura: String,
        public_key_b64: String,
        role: String,
    }

    let request = serde_json::from_slice::<UserKeyRegistration>(args).ok()?;
    if request.role.trim() != "user" || request.agent_ura.trim() != owner_ura {
        return None;
    }
    let public_key = BASE64_STANDARD.decode(request.public_key_b64.trim()).ok()?;
    (public_key.len() == 32).then_some(())
}

fn verify_hub_link_resolve_key(args: &[u8], hub_ura: &str) -> Option<()> {
    let Ok(request) = serde_json::from_slice::<ResolveKeyRequest>(args) else {
        return None;
    };
    let agent_ura = request.agent_ura.trim();
    let hub = parse_ura(hub_ura).ok()?;
    let agent = parse_ura(agent_ura).ok()?;
    if hub.kind != URAKind::Authority || agent.realm != hub.realm {
        return None;
    }
    match agent.kind {
        URAKind::User | URAKind::Agent | URAKind::Device | URAKind::Authority => Some(()),
        _ => None,
    }
}

fn is_device_ura(ura: &str) -> bool {
    parse_ura(ura)
        .map(|parsed| parsed.kind == URAKind::Device && parsed.device_id().is_some())
        .unwrap_or(false)
}

fn is_authority_ura(ura: &str) -> bool {
    parse_ura(ura)
        .map(|parsed| parsed.kind == URAKind::Authority)
        .unwrap_or(false)
}

fn callee_is_current_authority(callee_ura: &str, daemon_ura: Option<&str>) -> bool {
    if daemon_ura.is_some_and(|daemon| daemon != callee_ura) {
        return false;
    }
    parse_ura(callee_ura)
        .map(|callee| callee.kind == URAKind::Authority)
        .unwrap_or(false)
}

fn callee_is_selected_authority(
    callee_ura: &str,
    caller_ura: &str,
    daemon_ura: Option<&str>,
) -> bool {
    if daemon_ura.is_some_and(|daemon| daemon != callee_ura) {
        return false;
    }
    let Ok(callee) = parse_ura(callee_ura) else {
        return false;
    };
    let Ok(caller) = parse_ura(caller_ura) else {
        return false;
    };
    callee.kind == URAKind::Authority && callee.realm == caller.realm
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

fn hub_link_authority_id(caller_ura: &str, callee_ura: &str, ability: &str) -> String {
    let mut hasher = Sha256::new();
    hasher.update(caller_ura.as_bytes());
    hasher.update([0]);
    hasher.update(callee_ura.as_bytes());
    hasher.update([0]);
    hasher.update(ability.as_bytes());
    format!(
        "hub_link_authority:sha256:{}",
        hex::encode(hasher.finalize())
    )
}

fn device_hub_key_bootstrap_authority_id(caller_ura: &str, hub_ura: &str, ability: &str) -> String {
    let mut hasher = Sha256::new();
    hasher.update(caller_ura.as_bytes());
    hasher.update([0]);
    hasher.update(hub_ura.as_bytes());
    hasher.update([0]);
    hasher.update(ability.as_bytes());
    format!(
        "device_hub_key_bootstrap_authority:sha256:{}",
        hex::encode(hasher.finalize())
    )
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::cli::commands::test_support::HomeGuard;
    use crate::daemon::persistence::config::state_dir;
    use crate::daemon::trust::anchor::TrustedPrincipalOwner;
    use axon_sdk::pb::axon::v1::{AgentIdentity, SubjectIdentity};

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
                owner_username: Some("alice".to_string()),
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
            "generation": 1,
            "projection_revision": 1,
            "projection_digest": "digest",
            "lease_expires_unix_ms": 0,
            "ability_summaries": [],
        }))
        .expect("args");

        let got = BootstrapAuthorityVerifier::verify(
            &envelope(
                "easynet:///r/test/device/dev-1",
                "easynet:///r/test/authority",
                "easynet:///r/test/device/dev-1",
            ),
            ABILITY_FEDERATION_ADVERTISE_ABILITIES,
            AccessAction::Manage,
            &args,
            &anchor(),
            TrustedAgentRole::Device,
            Some("easynet:///r/test/authority"),
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
                "easynet:///r/test/authority",
                "easynet:///r/test/device/dev-1",
            ),
            ABILITY_FEDERATION_ADVERTISE_ABILITIES,
            AccessAction::Manage,
            &args,
            &anchor(),
            TrustedAgentRole::Device,
            Some("easynet:///r/test/authority"),
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
            Some("easynet:///r/test/authority"),
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
            Some("easynet:///r/test/authority"),
        );

        assert_eq!(got, BootstrapAuthorityDecision::NotApplicable);
    }

    #[test]
    fn paired_device_can_refresh_own_heartbeat_projection() {
        let args = serde_json::to_vec(&serde_json::json!({
            "since_abilities_revision": 0,
            "refresh_owner_uras": ["easynet:///r/test/device/dev-1"],
        }))
        .expect("args");

        let got = BootstrapAuthorityVerifier::verify(
            &envelope(
                "easynet:///r/test/device/dev-1",
                "easynet:///r/test/authority",
                "easynet:///r/test/device/dev-1",
            ),
            ABILITY_FEDERATION_HEARTBEAT,
            AccessAction::Manage,
            &args,
            &anchor(),
            TrustedAgentRole::Device,
            Some("easynet:///r/test/authority"),
        );

        assert!(matches!(got, BootstrapAuthorityDecision::Verified { .. }));
    }

    #[test]
    fn paired_device_cannot_refresh_other_heartbeat_projection() {
        let args = serde_json::to_vec(&serde_json::json!({
            "since_abilities_revision": 0,
            "refresh_owner_uras": ["easynet:///r/test/agent/alice.worker"],
        }))
        .expect("args");

        let got = BootstrapAuthorityVerifier::verify(
            &envelope(
                "easynet:///r/test/device/dev-1",
                "easynet:///r/test/authority",
                "easynet:///r/test/device/dev-1",
            ),
            ABILITY_FEDERATION_HEARTBEAT,
            AccessAction::Manage,
            &args,
            &anchor(),
            TrustedAgentRole::Device,
            Some("easynet:///r/test/authority"),
        );

        assert_eq!(got, BootstrapAuthorityDecision::NotApplicable);
    }

    #[test]
    fn device_without_owner_binding_has_no_bootstrap_authority() {
        let _home = HomeGuard::new();
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
                "easynet:///r/test/authority",
                "easynet:///r/test/device/dev-1",
            ),
            ABILITY_FEDERATION_JOIN,
            AccessAction::Manage,
            &args,
            &RealmTrustAnchor::default(),
            TrustedAgentRole::Device,
            Some("easynet:///r/test/authority"),
        );

        assert_eq!(got, BootstrapAuthorityDecision::NotApplicable);
    }

    #[test]
    fn malformed_local_credentials_make_bootstrap_owner_unavailable() {
        let _home = HomeGuard::new();
        std::fs::create_dir_all(state_dir()).expect("create isolated state dir");
        std::fs::write(state_dir().join("credentials.json"), b"{")
            .expect("write malformed credentials");
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
                "easynet:///r/test/authority",
                "easynet:///r/test/device/dev-1",
            ),
            ABILITY_FEDERATION_JOIN,
            AccessAction::Manage,
            &args,
            &RealmTrustAnchor::default(),
            TrustedAgentRole::Device,
            Some("easynet:///r/test/authority"),
        );

        match got {
            BootstrapAuthorityDecision::Unavailable { message } => {
                assert!(
                    message.contains("LOCAL_BOOTSTRAP_OWNER_UNAVAILABLE")
                        && message.contains("parse credentials"),
                    "unexpected message: {message}"
                );
            }
            other => panic!("expected unavailable bootstrap owner, got {other:?}"),
        }
    }

    #[test]
    fn ownerless_joined_device_can_resolve_selected_authority_key_during_bootstrap() {
        let args = serde_json::to_vec(&serde_json::json!({
            "agent_ura": "easynet:///r/test/authority",
        }))
        .expect("args");
        let subject = crate::core::ura::owner_ability_ura(
            "easynet:///r/test/authority",
            ABILITY_FEDERATION_RESOLVE_KEY,
        )
        .expect("subject");

        let got = BootstrapAuthorityVerifier::verify(
            &envelope(
                "easynet:///r/test/device/dev-1",
                "easynet:///r/test/authority",
                &subject,
            ),
            ABILITY_FEDERATION_RESOLVE_KEY,
            AccessAction::Read,
            &args,
            &RealmTrustAnchor::default(),
            TrustedAgentRole::Device,
            Some("easynet:///r/test/authority"),
        );

        assert!(matches!(got, BootstrapAuthorityDecision::Verified { .. }));
    }

    #[test]
    fn ownerless_joined_device_cannot_resolve_user_key_during_bootstrap() {
        let _home = HomeGuard::new();
        let args = serde_json::to_vec(&serde_json::json!({
            "agent_ura": "easynet:///r/test/user/alice",
        }))
        .expect("args");
        let subject = crate::core::ura::owner_ability_ura(
            "easynet:///r/test/authority",
            ABILITY_FEDERATION_RESOLVE_KEY,
        )
        .expect("subject");

        let got = BootstrapAuthorityVerifier::verify(
            &envelope(
                "easynet:///r/test/device/dev-1",
                "easynet:///r/test/authority",
                &subject,
            ),
            ABILITY_FEDERATION_RESOLVE_KEY,
            AccessAction::Read,
            &args,
            &RealmTrustAnchor::default(),
            TrustedAgentRole::Device,
            Some("easynet:///r/test/authority"),
        );

        assert_eq!(got, BootstrapAuthorityDecision::NotApplicable);
    }

    #[test]
    fn paired_device_can_resolve_selected_authority_key_during_bootstrap() {
        let args = serde_json::to_vec(&serde_json::json!({
            "agent_ura": "easynet:///r/test/authority",
        }))
        .expect("args");

        let subject = crate::core::ura::owner_ability_ura(
            "easynet:///r/test/authority",
            ABILITY_FEDERATION_RESOLVE_KEY,
        )
        .expect("subject");
        let got = BootstrapAuthorityVerifier::verify(
            &envelope(
                "easynet:///r/test/device/dev-1",
                "easynet:///r/test/authority",
                &subject,
            ),
            ABILITY_FEDERATION_RESOLVE_KEY,
            AccessAction::Read,
            &args,
            &anchor(),
            TrustedAgentRole::Device,
            Some("easynet:///r/test/authority"),
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
            "easynet:///r/test/authority",
            ABILITY_FEDERATION_RESOLVE_KEY,
        )
        .expect("subject");
        let got = BootstrapAuthorityVerifier::verify(
            &envelope(
                "easynet:///r/test/device/dev-1",
                "easynet:///r/test/authority",
                &subject,
            ),
            ABILITY_FEDERATION_RESOLVE_KEY,
            AccessAction::Read,
            &args,
            &anchor(),
            TrustedAgentRole::Device,
            Some("easynet:///r/test/authority"),
        );

        assert!(matches!(got, BootstrapAuthorityDecision::Verified { .. }));
    }

    #[test]
    fn paired_device_can_seed_only_its_owner_user_key_during_bootstrap() {
        let key = ed25519_dalek::SigningKey::from_bytes(&[0x11; 32]);
        let args = serde_json::to_vec(&serde_json::json!({
            "agent_ura": "easynet:///r/test/user/alice",
            "public_key_b64": BASE64_STANDARD.encode(key.verifying_key().to_bytes()),
            "role": "user",
        }))
        .expect("args");
        let subject = crate::core::ura::owner_ability_ura(
            "easynet:///r/test/authority",
            ABILITY_IDENTITY_REGISTER_PUBKEY,
        )
        .expect("subject");
        let got = BootstrapAuthorityVerifier::verify(
            &envelope(
                "easynet:///r/test/device/dev-1",
                "easynet:///r/test/authority",
                &subject,
            ),
            ABILITY_IDENTITY_REGISTER_PUBKEY,
            AccessAction::Manage,
            &args,
            &anchor(),
            TrustedAgentRole::Device,
            Some("easynet:///r/test/authority"),
        );
        assert!(matches!(got, BootstrapAuthorityDecision::Verified { .. }));

        let other_args = serde_json::to_vec(&serde_json::json!({
            "agent_ura": "easynet:///r/test/user/bob",
            "public_key_b64": BASE64_STANDARD.encode(key.verifying_key().to_bytes()),
            "role": "user",
        }))
        .expect("args");
        let rejected = BootstrapAuthorityVerifier::verify(
            &envelope(
                "easynet:///r/test/device/dev-1",
                "easynet:///r/test/authority",
                &subject,
            ),
            ABILITY_IDENTITY_REGISTER_PUBKEY,
            AccessAction::Manage,
            &other_args,
            &anchor(),
            TrustedAgentRole::Device,
            Some("easynet:///r/test/authority"),
        );
        assert_eq!(rejected, BootstrapAuthorityDecision::NotApplicable);
    }

    #[test]
    fn paired_device_cannot_resolve_unowned_agent_key_during_bootstrap() {
        let args = serde_json::to_vec(&serde_json::json!({
            "agent_ura": "easynet:///r/test/agent/alice.worker",
        }))
        .expect("args");

        let subject = crate::core::ura::owner_ability_ura(
            "easynet:///r/test/authority",
            ABILITY_FEDERATION_RESOLVE_KEY,
        )
        .expect("subject");
        let got = BootstrapAuthorityVerifier::verify(
            &envelope(
                "easynet:///r/test/device/dev-1",
                "easynet:///r/test/authority",
                &subject,
            ),
            ABILITY_FEDERATION_RESOLVE_KEY,
            AccessAction::Read,
            &args,
            &anchor(),
            TrustedAgentRole::Device,
            Some("easynet:///r/test/authority"),
        );

        assert_eq!(got, BootstrapAuthorityDecision::NotApplicable);
    }

    #[test]
    fn trusted_peer_hub_can_resolve_current_realm_device_key() {
        let args = serde_json::to_vec(&serde_json::json!({
            "agent_ura": "easynet:///r/test/device/dev-1",
        }))
        .expect("args");

        let subject = crate::core::ura::owner_ability_ura(
            "easynet:///r/test/authority",
            ABILITY_FEDERATION_RESOLVE_KEY,
        )
        .expect("subject");
        let got = BootstrapAuthorityVerifier::verify(
            &envelope(
                "easynet:///r/peer/authority",
                "easynet:///r/test/authority",
                &subject,
            ),
            ABILITY_FEDERATION_RESOLVE_KEY,
            AccessAction::Read,
            &args,
            &anchor(),
            TrustedAgentRole::Hub,
            Some("easynet:///r/test/authority"),
        );

        assert!(matches!(got, BootstrapAuthorityDecision::Verified { .. }));
    }

    #[test]
    fn trusted_peer_hub_cannot_resolve_third_realm_key() {
        let args = serde_json::to_vec(&serde_json::json!({
            "agent_ura": "easynet:///r/third/device/dev-1",
        }))
        .expect("args");

        let subject = crate::core::ura::owner_ability_ura(
            "easynet:///r/test/authority",
            ABILITY_FEDERATION_RESOLVE_KEY,
        )
        .expect("subject");
        let got = BootstrapAuthorityVerifier::verify(
            &envelope(
                "easynet:///r/peer/authority",
                "easynet:///r/test/authority",
                &subject,
            ),
            ABILITY_FEDERATION_RESOLVE_KEY,
            AccessAction::Read,
            &args,
            &anchor(),
            TrustedAgentRole::Hub,
            Some("easynet:///r/test/authority"),
        );

        assert_eq!(got, BootstrapAuthorityDecision::NotApplicable);
    }

    #[test]
    fn trusted_peer_hub_cannot_resolve_against_non_current_hub() {
        let args = serde_json::to_vec(&serde_json::json!({
            "agent_ura": "easynet:///r/test/device/dev-1",
        }))
        .expect("args");

        let subject = crate::core::ura::owner_ability_ura(
            "easynet:///r/test/authority",
            ABILITY_FEDERATION_RESOLVE_KEY,
        )
        .expect("subject");
        let got = BootstrapAuthorityVerifier::verify(
            &envelope(
                "easynet:///r/peer/authority",
                "easynet:///r/test/authority",
                &subject,
            ),
            ABILITY_FEDERATION_RESOLVE_KEY,
            AccessAction::Read,
            &args,
            &anchor(),
            TrustedAgentRole::Hub,
            Some("easynet:///r/other/authority"),
        );

        assert_eq!(got, BootstrapAuthorityDecision::NotApplicable);
    }
}
