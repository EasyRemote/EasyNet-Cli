// EasyNet CLI - owner ability projection publication authority
// =============================================================
//
// This boundary validates who may publish a complete ability set and proves
// that every projected Ability URA belongs to that owner before the mutable
// federation catalog is touched.

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
                    let record = advertised_agents
                        .get(&publication.owner_ura)
                        .ok_or(OwnerProjectionPublicationError::AgentNotAdvertised)?;
                    if record.host_ura() != Some(caller_ura) {
                        return Err(OwnerProjectionPublicationError::HostMismatch);
                    }
                    let caller_owner = trust_anchor
                        .lookup_principal_owner(caller_ura)
                        .ok_or(OwnerProjectionPublicationError::OwnerBindingMissing)?;
                    let agent_owner = trust_anchor
                        .lookup_principal_owner(&publication.owner_ura)
                        .ok_or(OwnerProjectionPublicationError::OwnerBindingMissing)?;
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
    #[error("hosted Agent must be advertised before its abilities")]
    AgentNotAdvertised,
    #[error("projection owner has no authoritative runtime owner binding")]
    OwnerBindingMissing,
    #[error("projection integrity check failed: {0}")]
    Integrity(String),
}
