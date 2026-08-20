// EasyNet CLI - owner ability projection publication authority
// =============================================================
//
// This boundary validates who may publish a complete ability set and proves
// that every canonical Ability publication unit, including its call-mode
// geometry, belongs to that owner before the mutable catalog is touched.

use thiserror::Error;

use axon_sdk::invocation::{AuthorityEvidence, AuthorityOrBootstrap, AuthorityRelation};
use axon_sdk::pb::axon::v1::Envelope;

use crate::core::ura::{parse_ura, URAKind};
use crate::daemon::federation::read_model::advertised_agents::AdvertisedAgentStore;
use crate::daemon::federation::read_model::owner_projection::OwnerProjectionPublication;
use crate::daemon::invocation::admission::device_caller::admits_owner_projection_publication_host;
use crate::daemon::trust::anchor::RealmTrustAnchor;

pub(crate) struct OwnerProjectionPublicationAuthority;

impl OwnerProjectionPublicationAuthority {
    pub(crate) fn verify(
        envelope: &Envelope,
        publication: &OwnerProjectionPublication,
        advertised_agents: &AdvertisedAgentStore,
        trust_anchor: &RealmTrustAnchor,
        daemon_ura: Option<&str>,
        authority_binding: Option<&AuthorityOrBootstrap>,
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
            authority_binding,
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
            None,
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
        authority_binding: Option<&AuthorityOrBootstrap>,
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
            OwnerProjectionPublicationError::InvalidIdentity(
                "callee must be a canonical Authority URA",
            )
        })?;
        if callee.kind != URAKind::Authority
            || callee.realm != caller.realm
            || daemon_ura.is_some_and(|daemon| daemon != callee_ura)
        {
            return Err(OwnerProjectionPublicationError::InvalidIdentity(
                "callee must be the selected Authority in the caller realm",
            ));
        }

        let owner = parse_ura(&publication.owner_ura).map_err(|_| {
            OwnerProjectionPublicationError::InvalidIdentity(
                "owner_ura must be a canonical Agent, Service, Authority, or same-device DeviceProfileProjection URA",
            )
        })?;
        if owner.realm != caller.realm {
            return Err(OwnerProjectionPublicationError::OwnerMismatch);
        }

        let device_publication_custody = admits_owner_projection_publication_host(
            caller_ura,
            callee_ura,
            &publication.owner_ura,
            daemon_ura,
        );

        match (device_publication_custody, caller.kind) {
            (true, URAKind::Device) => match owner.kind {
                // Same-device DeviceProfileProjection is a migration/high-water
                // publication cursor, not a target public AbilityDescriptor
                // owner/callee.
                URAKind::Device if publication.owner_ura == caller_ura => Ok(()),
                URAKind::Agent => {
                    if device_sponsored_system_agent_owned_by_caller(&owner, &caller) {
                        return Ok(());
                    }
                    let caller_owner = trust_anchor
                        .lookup_principal_owner(caller_ura)
                        .ok_or(OwnerProjectionPublicationError::OwnerBindingMissing)?;
                    let agent_owner = trust_anchor
                        .lookup_principal_owner(&publication.owner_ura)
                        .ok_or(OwnerProjectionPublicationError::OwnerBindingMissing)?;
                    let record = advertised_agents
                        .get(&publication.owner_ura)
                        .ok_or(OwnerProjectionPublicationError::HostedAgentRegistrationMissing)?;
                    if record.host_ura() != Some(caller_ura) {
                        return Err(OwnerProjectionPublicationError::HostMismatch);
                    }
                    if record.generation != publication.generation {
                        return Err(OwnerProjectionPublicationError::GenerationMismatch {
                            registered: record.generation,
                            published: publication.generation,
                        });
                    }
                    if caller_owner.owner_user_id != agent_owner.owner_user_id
                        || caller_owner.owner_ura != agent_owner.owner_ura
                    {
                        return Err(OwnerProjectionPublicationError::OwnerMismatch);
                    }
                    Ok(())
                }
                URAKind::Service => {
                    verify_user_service_delegation(&owner, callee_ura, authority_binding)
                }
                _ => Err(OwnerProjectionPublicationError::OwnerMismatch),
            },
            (_, URAKind::Authority)
                if caller_ura == callee_ura
                    && publication.owner_ura == caller_ura
                    && owner.kind == URAKind::Authority =>
            {
                Ok(())
            }
            _ => Err(OwnerProjectionPublicationError::InvalidIdentity(
                "only a host device or the selected Authority may publish ability projections",
            )),
        }
    }
}

fn verify_user_service_delegation(
    owner: &crate::core::ura::ParsedURA,
    callee_ura: &str,
    authority_binding: Option<&AuthorityOrBootstrap>,
) -> Result<(), OwnerProjectionPublicationError> {
    let owner_user_ura = owner
        .service_ids()
        .map(|(principal_id, _)| crate::core::ura::user_ura(&owner.realm, principal_id))
        .ok_or(OwnerProjectionPublicationError::ServiceDelegationRequired)?;
    let Some(AuthorityOrBootstrap::Binding(binding)) = authority_binding else {
        return Err(OwnerProjectionPublicationError::ServiceDelegationRequired);
    };
    let (AuthorityRelation::DelegatedBy, AuthorityEvidence::Delegation(evidence)) =
        (&binding.relation, &binding.evidence)
    else {
        return Err(OwnerProjectionPublicationError::ServiceDelegationRequired);
    };
    let scope_matches = evidence.scopes.iter().any(|scope| {
        scope == crate::daemon::ability::conformance::ABILITY_FEDERATION_ADVERTISE_ABILITIES
    });
    // Issuer authenticity (signature really from evidence.issuer) is the
    // SDK's job, proven before this call. This is the issuer AUTHORITY
    // check: is evidence.issuer actually entitled to vouch for
    // binding.authority (the service)? Here: issuer must be the user
    // whose principal ID the service's own URA embeds — the same
    // ownership-derivation pattern used elsewhere in this daemon (see
    // RFC doc "Issuer authenticity vs. issuer authority"). The old
    // caller_ura equality check is removed (field no longer exists — the
    // SDK's signature verification binds envelope.caller as delegatee
    // directly into the signed claim bytes instead).
    if evidence.issuer.ura != owner_user_ura
        || binding.authority.ura != owner.raw
        || evidence.audience != callee_ura
        || !scope_matches
    {
        return Err(OwnerProjectionPublicationError::ServiceDelegationMismatch);
    }
    Ok(())
}

fn device_sponsored_system_agent_owned_by_caller(
    owner: &crate::core::ura::ParsedURA,
    caller: &crate::core::ura::ParsedURA,
) -> bool {
    let Some(caller_device_id) = caller.device_id() else {
        return false;
    };
    owner
        .device_agent_ids()
        .is_some_and(|(sponsor_device_id, system_agent_id)| {
            sponsor_device_id == caller_device_id
                && crate::daemon::ability::catalog::profiles::is_declared_daemon_native_system_agent_id(
                    system_agent_id,
                )
        })
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
    #[error("User-owned Service projection requires an admitted User delegation")]
    ServiceDelegationRequired,
    #[error("User-owned Service projection delegation does not bind the exact owner, host, Authority, and publish scope")]
    ServiceDelegationMismatch,
    #[error("User-owned Agent projection requires a registered active host")]
    HostedAgentRegistrationMissing,
    #[error(
        "User-owned Agent projection generation {published} does not match active identity generation {registered}"
    )]
    GenerationMismatch { registered: u64, published: u64 },
    #[error("projection integrity check failed: {0}")]
    Integrity(String),
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::daemon::ability::descriptors::{AbilityDescriptor, CallMode, Visibility};
    use crate::daemon::federation::read_model::advertised_agents::{
        AdvertisedAgentRecord, AdvertisedAgentSigningAuthority,
    };
    use crate::daemon::federation::read_model::owner_projection::{
        prepare_and_persist, prepare_hosted_and_persist_for_test,
    };
    use crate::daemon::trust::anchor::TrustedPrincipalOwner;
    use axon_sdk::pb::axon::v1::{
        AgentIdentity as PbAgentIdentity, SubjectIdentity as PbSubjectIdentity,
    };

    const DEVICE_URA: &str = "easynet:///r/test/device/dev-1";
    const SECOND_DEVICE_URA: &str = "easynet:///r/test/device/dev-2";
    const AGENT_URA: &str = "easynet:///r/test/agent/dev.chat";
    const SERVICE_URA: &str =
        "easynet:///r/test/service/16567c49-7621-468e-8ed0-273825299cc2.pages";
    const HUB_URA: &str = "easynet:///r/test/authority";
    const USER_ID: &str = "16567c49-7621-468e-8ed0-273825299cc2";
    const USER_URA: &str = "easynet:///r/test/user/16567c49-7621-468e-8ed0-273825299cc2";

    fn owner_binding(principal_ura: &str) -> TrustedPrincipalOwner {
        TrustedPrincipalOwner {
            principal_ura: principal_ura.to_string(),
            owner_user_id: USER_ID.to_string(),
            owner_ura: USER_URA.to_string(),
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

    fn registered_agent(host_ura: &str) -> AdvertisedAgentStore {
        let store = AdvertisedAgentStore::new();
        store.upsert(AdvertisedAgentRecord {
            agent_ura: AGENT_URA.to_string(),
            generation: 1,
            public_key_hex: String::new(),
            host_node_id: parse_ura(host_ura)
                .ok()
                .and_then(|parsed| parsed.device_id().map(str::to_string)),
            signing_authority: AdvertisedAgentSigningAuthority::HostedBy {
                host_ura: host_ura.to_string(),
            },
        });
        store
    }

    fn caller_signed_envelope(subject_ura: &str) -> Envelope {
        Envelope {
            caller: Some(PbAgentIdentity {
                ura: DEVICE_URA.to_string(),
                profile: "axon-strict-v2".to_string(),
            }),
            callee: Some(PbAgentIdentity {
                ura: HUB_URA.to_string(),
                profile: "axon-strict-v2".to_string(),
            }),
            subject: Some(PbSubjectIdentity {
                ura: subject_ura.to_string(),
                profile: "axon-strict-v2".to_string(),
            }),
            ..Envelope::default()
        }
    }

    fn service_delegation(issuer_ura: &str) -> AuthorityOrBootstrap {
        AuthorityOrBootstrap::Binding(axon_sdk::invocation::AuthorityBinding {
            authority: axon_sdk::invocation::axiom::AgentIdentity::new(
                SERVICE_URA,
                axon_sdk::invocation::axiom::UraProfile::StrictV2,
            ),
            relation: AuthorityRelation::DelegatedBy,
            evidence: AuthorityEvidence::Delegation(axon_sdk::invocation::DelegationEvidence {
                issuer: axon_sdk::invocation::axiom::AgentIdentity::new(
                    issuer_ura,
                    axon_sdk::invocation::axiom::UraProfile::StrictV2,
                ),
                scopes: vec![
                    crate::daemon::ability::conformance::ABILITY_FEDERATION_ADVERTISE_ABILITIES
                        .to_string(),
                ],
                audience: HUB_URA.to_string(),
                issued_at_ms: 1,
                expires_at_ms: 2,
                signature: vec![0x51; 64],
            }),
        })
    }

    #[test]
    fn user_service_projection_requires_exact_admitted_delegation() {
        let _home = crate::cli::commands::test_support::HomeGuard::new();
        let service_descriptor = AbilityDescriptor::new(
            "pages.publish",
            SERVICE_URA,
            Visibility::Scoped,
            crate::daemon::ability::descriptors::AdmissionAction::Manage,
        )
        .expect("Pages Service descriptor");
        let publication = prepare_and_persist(SERVICE_URA, DEVICE_URA, &[service_descriptor])
            .expect("canonical Service projection");
        let envelope = caller_signed_envelope(SERVICE_URA);
        let trust_anchor = RealmTrustAnchor::from_entries(Vec::new()).expect("empty trust anchor");
        let advertised_agents = AdvertisedAgentStore::new();
        let admitted_delegation = service_delegation(USER_URA);

        OwnerProjectionPublicationAuthority::verify(
            &envelope,
            &publication,
            &advertised_agents,
            &trust_anchor,
            Some(HUB_URA),
            Some(&admitted_delegation),
        )
        .expect("exact admitted User delegation authorizes its Service projection");

        let missing = OwnerProjectionPublicationAuthority::verify(
            &envelope,
            &publication,
            &advertised_agents,
            &trust_anchor,
            Some(HUB_URA),
            None,
        )
        .expect_err("Device custody alone must not authorize a User Service");
        assert_eq!(
            missing,
            OwnerProjectionPublicationError::ServiceDelegationRequired
        );

        let foreign = service_delegation("easynet:///r/test/user/other");
        let mismatch = OwnerProjectionPublicationAuthority::verify(
            &envelope,
            &publication,
            &advertised_agents,
            &trust_anchor,
            Some(HUB_URA),
            Some(&foreign),
        )
        .expect_err("another User cannot delegate authority over this Service");
        assert_eq!(
            mismatch,
            OwnerProjectionPublicationError::ServiceDelegationMismatch
        );

        let session_bypass = OwnerProjectionPublicationAuthority::verify_admitted_session(
            DEVICE_URA,
            HUB_URA,
            &publication,
            &advertised_agents,
            &trust_anchor,
        )
        .expect_err("an admitted Device session does not replace User delegation");
        assert_eq!(
            session_bypass,
            OwnerProjectionPublicationError::ServiceDelegationRequired
        );
    }

    #[test]
    fn admitted_agent_publication_accepts_canonical_multi_mode_geometry() {
        let _home = crate::cli::commands::test_support::HomeGuard::new();
        let publication = prepare_hosted_and_persist_for_test(
            AGENT_URA,
            DEVICE_URA,
            &[descriptor(CallMode::Rpc), descriptor(CallMode::Stream)],
            1,
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
        let advertised_agents = registered_agent(DEVICE_URA);

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

    #[test]
    fn user_agent_projection_requires_prior_host_registration() {
        let _home = crate::cli::commands::test_support::HomeGuard::new();
        let publication = prepare_hosted_and_persist_for_test(
            AGENT_URA,
            DEVICE_URA,
            &[descriptor(CallMode::Rpc)],
            1,
        )
        .expect("canonical agent publication");
        let trust_anchor = RealmTrustAnchor::from_parts_with_principal_owners(
            Vec::new(),
            vec![owner_binding(DEVICE_URA), owner_binding(AGENT_URA)],
            Vec::new(),
        )
        .expect("owner bindings");

        let error = OwnerProjectionPublicationAuthority::verify_admitted_session(
            DEVICE_URA,
            HUB_URA,
            &publication,
            &AdvertisedAgentStore::new(),
            &trust_anchor,
        )
        .expect_err("unregistered User-Agent owner projection must fail closed");

        assert_eq!(
            error,
            OwnerProjectionPublicationError::HostedAgentRegistrationMissing
        );
    }

    #[test]
    fn user_agent_projection_generation_must_equal_active_identity() {
        let _home = crate::cli::commands::test_support::HomeGuard::new();
        let base = prepare_hosted_and_persist_for_test(
            AGENT_URA,
            DEVICE_URA,
            &[descriptor(CallMode::Rpc)],
            1,
        )
        .expect("canonical agent publication");
        let trust_anchor = RealmTrustAnchor::from_parts_with_principal_owners(
            Vec::new(),
            vec![owner_binding(DEVICE_URA), owner_binding(AGENT_URA)],
            Vec::new(),
        )
        .expect("owner bindings");
        let advertised_agents = AdvertisedAgentStore::new();
        advertised_agents.upsert(AdvertisedAgentRecord {
            agent_ura: AGENT_URA.to_string(),
            generation: 2,
            public_key_hex: String::new(),
            host_node_id: Some("dev-1".to_string()),
            signing_authority: AdvertisedAgentSigningAuthority::HostedBy {
                host_ura: DEVICE_URA.to_string(),
            },
        });

        for published in [1, 3, u64::MAX] {
            let mut publication = base.clone();
            publication.generation = published;
            publication.projection_digest = publication.canonical_digest();
            let error = OwnerProjectionPublicationAuthority::verify_admitted_session(
                DEVICE_URA,
                HUB_URA,
                &publication,
                &advertised_agents,
                &trust_anchor,
            )
            .expect_err("non-current identity generation must fail closed");
            assert_eq!(
                error,
                OwnerProjectionPublicationError::GenerationMismatch {
                    registered: 2,
                    published,
                }
            );
        }

        let mut current = base;
        current.generation = 2;
        current.projection_digest = current.canonical_digest();
        OwnerProjectionPublicationAuthority::verify_admitted_session(
            DEVICE_URA,
            HUB_URA,
            &current,
            &advertised_agents,
            &trust_anchor,
        )
        .expect("exact active identity generation is publishable");
    }

    #[test]
    fn device_sponsorship_authorizes_declared_system_agent_projection() {
        let _home = crate::cli::commands::test_support::HomeGuard::new();
        let system_agent_ura = "easynet:///r/test/agent/device.dev-1.runtime-introspection";
        let descriptor = AbilityDescriptor::new(
            crate::daemon::ability::names::governance::META_LIST_ABILITIES,
            system_agent_ura,
            Visibility::Scoped,
            crate::daemon::ability::descriptors::AdmissionAction::Read,
        )
        .expect("system Agent descriptor");
        let publication = prepare_and_persist(system_agent_ura, DEVICE_URA, &[descriptor])
            .expect("canonical system Agent projection");
        let trust_anchor = RealmTrustAnchor::from_parts_with_principal_owners(
            Vec::new(),
            vec![owner_binding(DEVICE_URA)],
            Vec::new(),
        )
        .expect("device owner binding");

        OwnerProjectionPublicationAuthority::verify_admitted_session(
            DEVICE_URA,
            HUB_URA,
            &publication,
            &AdvertisedAgentStore::new(),
            &trust_anchor,
        )
        .expect("Device sponsor owns the declared SystemAgent publication boundary");
    }

    #[test]
    fn same_user_second_device_cannot_replace_registered_agent_host() {
        let _home = crate::cli::commands::test_support::HomeGuard::new();
        let publication = prepare_hosted_and_persist_for_test(
            AGENT_URA,
            SECOND_DEVICE_URA,
            &[descriptor(CallMode::Rpc)],
            1,
        )
        .expect("canonical second-device publication");
        let trust_anchor = RealmTrustAnchor::from_parts_with_principal_owners(
            Vec::new(),
            vec![owner_binding(SECOND_DEVICE_URA), owner_binding(AGENT_URA)],
            Vec::new(),
        )
        .expect("same-User owner bindings");
        let advertised_agents = registered_agent(DEVICE_URA);

        let error = OwnerProjectionPublicationAuthority::verify_admitted_session(
            SECOND_DEVICE_URA,
            HUB_URA,
            &publication,
            &advertised_agents,
            &trust_anchor,
        )
        .expect_err("registered hosting Device remains exact");
        assert_eq!(error, OwnerProjectionPublicationError::HostMismatch);
    }
}
