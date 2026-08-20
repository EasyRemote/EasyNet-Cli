// EasyNet CLI - hosted Agent publication authority
// =================================================
//
// A hosted Agent is user-owned by the immutable user id segment in its
// canonical Agent URA. This value object proves the complete link from the
// signed host device to that Agent and projects the authoritative owner fact
// that admission and policy share.

use thiserror::Error;

use axon_sdk::pb::axon::v1::Envelope;

use crate::core::ura::{parse_ura, URAKind};
use crate::daemon::invocation::dispatch::federation_wrappers::AdvertiseAgentRequest;
use crate::daemon::persistence::federation_revoke::{
    DurableSigningAuthority, HostedAgentRegistrationCommand,
};
use crate::daemon::trust::anchor::{RealmTrustAnchor, TrustedPrincipalOwner};

#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct HostedAgentPublication {
    caller_device_ura: String,
    caller_device_id: String,
    agent_ura: String,
    owner_user_segment: String,
    owner_ura: String,
}

impl HostedAgentPublication {
    pub(crate) fn verify(
        envelope: &Envelope,
        request: &AdvertiseAgentRequest,
        trust_anchor: &RealmTrustAnchor,
        daemon_ura: Option<&str>,
    ) -> Result<Self, HostedAgentPublicationError> {
        let caller_device_ura = required_agent_ura(envelope.caller.as_ref(), "caller")?;
        let callee_ura = required_agent_ura(envelope.callee.as_ref(), "callee")?;
        let subject_ura = envelope
            .subject
            .as_ref()
            .map(|subject| subject.ura.trim())
            .filter(|ura| !ura.is_empty())
            .ok_or(HostedAgentPublicationError::MissingIdentity("subject"))?;
        Self::verify_identity_binding(
            caller_device_ura,
            callee_ura,
            subject_ura,
            request,
            trust_anchor,
            daemon_ura,
        )
    }

    fn verify_identity_binding(
        caller_device_ura: &str,
        callee_ura: &str,
        subject_ura: &str,
        request: &AdvertiseAgentRequest,
        trust_anchor: &RealmTrustAnchor,
        daemon_ura: Option<&str>,
    ) -> Result<Self, HostedAgentPublicationError> {
        let caller = parse_ura(caller_device_ura).map_err(|_| {
            HostedAgentPublicationError::InvalidIdentity("caller must be a canonical device URA")
        })?;
        if caller.kind != URAKind::Device {
            return Err(HostedAgentPublicationError::InvalidIdentity(
                "caller must be a canonical device URA",
            ));
        }
        let caller_device_id =
            caller
                .device_id()
                .ok_or(HostedAgentPublicationError::InvalidIdentity(
                    "caller device URA must contain a device id",
                ))?;

        let callee = parse_ura(callee_ura).map_err(|_| {
            HostedAgentPublicationError::InvalidIdentity("callee must be a canonical Authority URA")
        })?;
        if callee.kind != URAKind::Authority || callee.realm != caller.realm {
            return Err(HostedAgentPublicationError::InvalidIdentity(
                "callee must be the caller realm Authority",
            ));
        }
        if daemon_ura.is_some_and(|daemon| daemon != callee_ura) {
            return Err(HostedAgentPublicationError::InvalidIdentity(
                "callee does not identify the selected Authority",
            ));
        }

        let agent_ura = request.agent_ura.trim();
        if agent_ura != request.agent_ura {
            return Err(HostedAgentPublicationError::InvalidIdentity(
                "agent_ura must be canonical without surrounding whitespace",
            ));
        }
        if subject_ura != agent_ura {
            return Err(HostedAgentPublicationError::SubjectMismatch);
        }
        let agent = parse_ura(agent_ura).map_err(|_| {
            HostedAgentPublicationError::InvalidIdentity(
                "agent_ura must be a canonical user-owned Agent URA",
            )
        })?;
        if agent.kind != URAKind::Agent || agent.realm != caller.realm {
            return Err(HostedAgentPublicationError::InvalidIdentity(
                "agent_ura must identify a user-owned Agent in the caller realm",
            ));
        }
        let (agent_owner_user_segment, _) =
            agent
                .agent_ids()
                .ok_or(HostedAgentPublicationError::InvalidIdentity(
                    "device-sponsored Agent URAs cannot enter the user-hosted publication path",
                ))?;

        let owner = trust_anchor
            .lookup_principal_owner(caller_device_ura)
            .ok_or(HostedAgentPublicationError::OwnerBindingMissing)?;
        if owner.owner_user_id != agent_owner_user_segment {
            return Err(HostedAgentPublicationError::OwnerUserMismatch);
        }
        let owner_ura = parse_ura(&owner.owner_ura).map_err(|_| {
            HostedAgentPublicationError::InvalidIdentity(
                "device owner_ura must be a canonical user URA",
            )
        })?;
        if owner_ura.kind != URAKind::User
            || owner_ura.realm != caller.realm
            || owner_ura.user_id() != Some(owner.owner_user_id.as_str())
        {
            return Err(HostedAgentPublicationError::InvalidIdentity(
                "device owner binding is not canonical for the caller realm",
            ));
        }

        Ok(Self {
            caller_device_ura: caller_device_ura.to_string(),
            caller_device_id: caller_device_id.to_string(),
            agent_ura: agent_ura.to_string(),
            owner_user_segment: owner.owner_user_id.clone(),
            owner_ura: owner.owner_ura.clone(),
        })
    }

    /// Verify a publication carried by an already-admitted `session.open`.
    /// The session's caller is the authenticated device principal; the
    /// request's Agent is projected as the child subject and passes through
    /// the exact same ownership verifier as a standalone unary envelope.
    pub(crate) fn verify_admitted_session(
        caller_device_ura: &str,
        hub_ura: &str,
        request: &AdvertiseAgentRequest,
        trust_anchor: &RealmTrustAnchor,
    ) -> Result<Self, HostedAgentPublicationError> {
        Self::verify_identity_binding(
            caller_device_ura,
            hub_ura,
            &request.agent_ura,
            request,
            trust_anchor,
            Some(hub_ura),
        )
    }

    pub(crate) fn owner_user_segment(&self) -> &str {
        &self.owner_user_segment
    }

    pub(crate) fn caller_device_ura(&self) -> &str {
        &self.caller_device_ura
    }

    pub(crate) fn authority_id(&self) -> String {
        format!(
            "hosted-agent-publication:{}:{}",
            self.caller_device_ura, self.agent_ura
        )
    }

    pub(crate) fn registration_command(
        &self,
        request: &AdvertiseAgentRequest,
    ) -> HostedAgentRegistrationCommand {
        debug_assert_eq!(self.agent_ura, request.agent_ura);
        HostedAgentRegistrationCommand {
            agent_ura: self.agent_ura.clone(),
            incarnation_id: request.incarnation_id.clone(),
            public_key_hex: String::new(),
            host_node_id: Some(self.caller_device_id.clone()),
            signing_authority: DurableSigningAuthority::HostedBy {
                host_ura: self.caller_device_ura.clone(),
            },
        }
    }

    pub(crate) fn into_owner_binding(self, added_at_unix_ms: u64) -> TrustedPrincipalOwner {
        TrustedPrincipalOwner {
            principal_ura: self.agent_ura,
            owner_user_id: self.owner_user_segment,
            owner_ura: self.owner_ura,
            added_at_unix_ms,
        }
    }
}

fn required_agent_ura<'a>(
    identity: Option<&'a axon_sdk::pb::axon::v1::AgentIdentity>,
    role: &'static str,
) -> Result<&'a str, HostedAgentPublicationError> {
    identity
        .map(|identity| identity.ura.trim())
        .filter(|ura| !ura.is_empty())
        .ok_or(HostedAgentPublicationError::MissingIdentity(role))
}

#[derive(Debug, Error, Clone, PartialEq, Eq)]
pub(crate) enum HostedAgentPublicationError {
    #[error("missing envelope {0} URA")]
    MissingIdentity(&'static str),
    #[error("{0}")]
    InvalidIdentity(&'static str),
    #[error("envelope subject must equal request agent_ura")]
    SubjectMismatch,
    #[error("caller device has no authoritative owner binding")]
    OwnerBindingMissing,
    #[error("Agent URA owner user id does not match the caller device owner")]
    OwnerUserMismatch,
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::daemon::federation::hosted_agent_publication::HostedAgentIncarnationId;
    use crate::daemon::invocation::dispatch::federation_wrappers::AdvertiseAgentRequest;
    use axon_sdk::pb::axon::v1::{AgentIdentity, SubjectIdentity};

    fn envelope(subject: &str) -> Envelope {
        Envelope {
            caller: Some(AgentIdentity {
                ura: "easynet:///r/test/device/dev-1".to_string(),
                profile: String::new(),
            }),
            callee: Some(AgentIdentity {
                ura: "easynet:///r/test/authority".to_string(),
                profile: String::new(),
            }),
            subject: Some(SubjectIdentity {
                ura: subject.to_string(),
                profile: String::new(),
            }),
            ..Envelope::default()
        }
    }

    fn request(agent_ura: &str) -> AdvertiseAgentRequest {
        AdvertiseAgentRequest {
            agent_ura: agent_ura.to_string(),
            incarnation_id: HostedAgentIncarnationId::parse("a".repeat(32)).unwrap(),
        }
    }

    fn anchor() -> RealmTrustAnchor {
        RealmTrustAnchor::from_parts_with_principal_owners(
            Vec::new(),
            vec![TrustedPrincipalOwner {
                principal_ura: "easynet:///r/test/device/dev-1".to_string(),
                owner_user_id: "16567c49-7621-468e-8ed0-273825299cc2".to_string(),
                owner_ura: "easynet:///r/test/user/16567c49-7621-468e-8ed0-273825299cc2"
                    .to_string(),
                added_at_unix_ms: 1,
            }],
            Vec::new(),
        )
        .expect("anchor")
    }

    #[test]
    fn binds_agent_ura_to_canonical_uuid_owner() {
        let agent_ura = "easynet:///r/test/agent/16567c49-7621-468e-8ed0-273825299cc2.eval";
        let publication = HostedAgentPublication::verify(
            &envelope(agent_ura),
            &request(agent_ura),
            &anchor(),
            Some("easynet:///r/test/authority"),
        )
        .expect("verified publication");

        let binding = publication.into_owner_binding(2);
        assert_eq!(binding.principal_ura, agent_ura);
        assert_eq!(
            binding.owner_user_id,
            "16567c49-7621-468e-8ed0-273825299cc2"
        );
    }

    #[test]
    fn rejects_agent_user_id_not_owned_by_device_user() {
        let agent_ura = "easynet:///r/test/agent/79f8c6a9-aee3-435c-ab78-cacdd49b3268.eval";
        let err = HostedAgentPublication::verify(
            &envelope(agent_ura),
            &request(agent_ura),
            &anchor(),
            Some("easynet:///r/test/authority"),
        )
        .expect_err("owner user mismatch must reject");

        assert_eq!(err, HostedAgentPublicationError::OwnerUserMismatch);
    }
}
