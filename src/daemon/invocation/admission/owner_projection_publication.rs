// EasyNet CLI - owner ability projection publication authority
// =============================================================
//
// This boundary validates who may publish a complete ability set and proves
// that every canonical Ability publication unit, including its call-mode
// geometry, belongs to that owner before the mutable catalog is touched.

use thiserror::Error;

use easynet_axon::pb::axon::v1::Envelope;

use crate::core::ura::{parse_ura, URAKind};
use crate::daemon::federation::read_model::advertised_agents::AdvertisedAgentStore;
use crate::daemon::federation::read_model::owner_projection::OwnerProjectionPublication;
use crate::daemon::trust::anchor::RealmTrustAnchor;

pub(crate) struct OwnerProjectionPublicationAuthority;

impl OwnerProjectionPublicationAuthority {
    pub(crate) fn verify(
        envelope: &Envelope,
        publication: &OwnerProjectionPublication,
        advertised_agents: &AdvertisedAgentStore,
        trust_anchor: &RealmTrustAnchor,
        daemon_ura: Option<&str>,
    ) -> Result<(), OwnerProjectionPublicationError> {
        let caller_ura = envelope
            .caller
            .as_ref()
            .map(|identity| identity.ura.trim())
            .filter(|ura| !ura.is_empty())
            .ok_or(OwnerProjectionPublicationError::MissingIdentity("caller"))?;
        let callee_ura = envelope
            .callee
            .as_ref()
            .map(|identity| identity.ura.trim())
            .filter(|ura| !ura.is_empty())
            .ok_or(OwnerProjectionPublicationError::MissingIdentity("callee"))?;
        let subject_ura = envelope
            .subject
            .as_ref()
            .map(|identity| identity.ura.trim())
            .filter(|ura| !ura.is_empty())
            .ok_or(OwnerProjectionPublicationError::MissingIdentity("subject"))?;

        Self::verify_uras(
            caller_ura,
            callee_ura,
            subject_ura,
            publication,
            advertised_agents,
            trust_anchor,
            daemon_ura,
        )
    }

    pub(crate) fn verify_admitted_session(
        caller_device_ura: &str,
        hub_ura: &str,
        publication: &OwnerProjectionPublication,
        advertised_agents: &AdvertisedAgentStore,
        trust_anchor: &RealmTrustAnchor,
    ) -> Result<(), OwnerProjectionPublicationError> {
        Self::verify_uras(
            caller_device_ura,
            hub_ura,
            &publication.owner_ura,
            publication,
            advertised_agents,
            trust_anchor,
            Some(hub_ura),
        )
    }

    #[allow(clippy::too_many_arguments)]
    fn verify_uras(
        caller_ura: &str,
        callee_ura: &str,
        subject_ura: &str,
        publication: &OwnerProjectionPublication,
        advertised_agents: &AdvertisedAgentStore,
        trust_anchor: &RealmTrustAnchor,
        daemon_ura: Option<&str>,
    ) -> Result<(), OwnerProjectionPublicationError> {
        publication
            .validate_integrity()
            .map_err(OwnerProjectionPublicationError::Integrity)?;

        if subject_ura != publication.owner_ura {
            return Err(OwnerProjectionPublicationError::SubjectMismatch);
        }
        if caller_ura != publication.host_device_ura {
            return Err(OwnerProjectionPublicationError::HostMismatch);
        }

        let caller = parse_ura(caller_ura).map_err(|_| {
            OwnerProjectionPublicationError::InvalidIdentity(
                "caller must be a canonical device or hub URA",
            )
        })?;
        let callee = parse_ura(callee_ura).map_err(|_| {
            OwnerProjectionPublicationError::InvalidIdentity("callee must be a canonical hub URA")
        })?;
        if callee.kind != URAKind::Hub
            || callee.realm != caller.realm
            || daemon_ura.is_some_and(|daemon| daemon != callee_ura)
        {
            return Err(OwnerProjectionPublicationError::InvalidIdentity(
                "callee must be the selected hub in the caller realm",
            ));
        }

        let owner = parse_ura(&publication.owner_ura).map_err(|_| {
            OwnerProjectionPublicationError::InvalidIdentity(
                "owner_ura must be a canonical Agent, device, or hub URA",
            )
        })?;
        if owner.realm != caller.realm {
            return Err(OwnerProjectionPublicationError::OwnerMismatch);
        }

        match caller.kind {
            URAKind::Device => match owner.kind {
                URAKind::Device if publication.owner_ura == caller_ura => Ok(()),
                URAKind::Agent => {
                    let caller_owner = trust_anchor
                        .lookup_principal_owner(caller_ura)
                        .ok_or(OwnerProjectionPublicationError::OwnerBindingMissing)?;
                    let agent_owner = trust_anchor
                        .lookup_principal_owner(&publication.owner_ura)
                        .ok_or(OwnerProjectionPublicationError::OwnerBindingMissing)?;
                    if let Some(record) = advertised_agents.get(&publication.owner_ura) {
                        if record.host_ura() != Some(caller_ura)
                            && (caller_owner.owner_user_id != agent_owner.owner_user_id
                                || caller_owner.owner_ura != agent_owner.owner_ura)
                        {
                            return Err(OwnerProjectionPublicationError::HostMismatch);
                        }
                    }
                    if caller_owner.owner_user_id != agent_owner.owner_user_id
                        || caller_owner.owner_ura != agent_owner.owner_ura
                    {
                        return Err(OwnerProjectionPublicationError::OwnerMismatch);
                    }
                    Ok(())
                }
                _ => Err(OwnerProjectionPublicationError::OwnerMismatch),
            },
            URAKind::Hub
                if caller_ura == callee_ura
                    && publication.owner_ura == caller_ura
                    && owner.kind == URAKind::Hub =>
            {
                Ok(())
            }
            _ => Err(OwnerProjectionPublicationError::InvalidIdentity(
                "only a host device or the selected hub may publish ability projections",
            )),
        }
    }
}

#[derive(Debug, Error, Clone, PartialEq, Eq)]
pub(crate) enum OwnerProjectionPublicationError {
    #[error("missing envelope {0} URA")]
    MissingIdentity(&'static str),
    #[error("{0}")]
    InvalidIdentity(&'static str),
    #[error("projection subject must equal owner_ura")]
    SubjectMismatch,
    #[error("projection host_device_ura must equal the authenticated caller")]
    HostMismatch,
    #[error("projection owner is not controlled by the authenticated caller")]
    OwnerMismatch,
    #[error("projection owner has no authoritative runtime owner binding")]
    OwnerBindingMissing,
    #[error("projection integrity check failed: {0}")]
    Integrity(String),
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::daemon::ability::descriptors::{AbilityDescriptor, CallMode, Visibility};
    use crate::daemon::federation::read_model::owner_projection::prepare_and_persist;
    use crate::daemon::trust::anchor::TrustedPrincipalOwner;

    const DEVICE_URA: &str = "easynet:///r/test/device/dev-1";
    const AGENT_URA: &str = "easynet:///r/test/agent/dev.chat";
    const HUB_URA: &str = "easynet:///r/test/hub";
    const USER_ID: &str = "16567c49-7621-468e-8ed0-273825299cc2";
    const USER_URA: &str = "easynet:///r/test/user/16567c49-7621-468e-8ed0-273825299cc2";

    fn owner_binding(principal_ura: &str) -> TrustedPrincipalOwner {
        TrustedPrincipalOwner {
            principal_ura: principal_ura.to_string(),
            owner_user_id: USER_ID.to_string(),
            owner_ura: USER_URA.to_string(),
            owner_username: Some("dev".to_string()),
            added_at_unix_ms: 1,
        }
    }

    fn descriptor(call_mode: CallMode) -> AbilityDescriptor {
        AbilityDescriptor::new(
            "chat",
            AGENT_URA,
            Visibility::Scoped,
            crate::daemon::ability::descriptors::AdmissionAction::Invoke,
        )
        .expect("agent chat descriptor")
        .with_call_mode(call_mode)
    }

    #[test]
    fn admitted_agent_publication_accepts_canonical_multi_mode_geometry() {
        let _home = crate::cli::commands::test_support::HomeGuard::new();
        let publication = prepare_and_persist(
            AGENT_URA,
            DEVICE_URA,
            &[descriptor(CallMode::Rpc), descriptor(CallMode::Stream)],
        )
        .expect("canonical agent publication");
        let publication: OwnerProjectionPublication = serde_json::from_value(
            serde_json::to_value(publication).expect("publication serializes"),
        )
        .expect("publication wire round-trip");
        assert_eq!(publication.ability_summaries.len(), 1);
        assert_eq!(
            publication.ability_summaries[0]
                .callable_summary
                .mode_geometry
                .len(),
            2
        );

        let trust_anchor = RealmTrustAnchor::from_parts_with_principal_owners(
            Vec::new(),
            vec![owner_binding(DEVICE_URA), owner_binding(AGENT_URA)],
            Vec::new(),
        )
        .expect("owner bindings");
        let advertised_agents = AdvertisedAgentStore::new();

        OwnerProjectionPublicationAuthority::verify_admitted_session(
            DEVICE_URA,
            HUB_URA,
            &publication,
            &advertised_agents,
            &trust_anchor,
        )
        .expect("multi-mode geometry passes receive admission");
        let error = OwnerProjectionPublicationAuthority::verify_admitted_session(
            "easynet:///r/test/device/dev-2",
            HUB_URA,
            &publication,
            &advertised_agents,
            &trust_anchor,
        )
        .expect_err("host identity remains bound");
        assert_eq!(error, OwnerProjectionPublicationError::HostMismatch);
    }
}
