//! Shared proto-to-Axon descriptor-bound envelope reassembly.
//!
//! The daemon receives Axon protobuf envelopes from several transport
//! surfaces. Every one of them must reconstruct the same
//! `DescriptorBoundEnvelope` before admission or dispatch; this module is the
//! single place that owns that conversion.

use easynet_axon::invocation::{
    canonical_ability_descriptor_ref, fresh_nonce, wire, AxonError, DescriptorBoundEnvelope,
    DescriptorBoundEnvelopeParts, EntityRef, SubjectIdentity,
};
use easynet_axon::pb::axon::v1 as pb;

use crate::runtime::local_invocation_identity::system_agent_identity;

#[derive(Debug, Clone, Copy)]
pub(crate) enum WireCallerIdentity {
    FromEnvelope,
    LocalSystem,
}

#[derive(Debug)]
pub(crate) struct WireDescriptorBoundEnvelope {
    pub envelope: DescriptorBoundEnvelope,
    pub trace_id: String,
}

pub(crate) fn ability_ura_for_wire(callee_ura: &str, ability: &str) -> Result<String, AxonError> {
    let callee_ura = callee_ura.trim();
    let ability = ability.trim();
    if callee_ura.is_empty() {
        return Err(AxonError::invalid_argument(
            "descriptor-bound ability callee URA is empty",
        ));
    }
    if ability.is_empty() {
        return Err(AxonError::invalid_argument(
            "descriptor-bound ability name is empty",
        ));
    }

    if let Ok(descriptor_ref) = canonical_ability_descriptor_ref(ability) {
        let (ability_ura, _) = descriptor_ref.rsplit_once('@').ok_or_else(|| {
            AxonError::invalid_argument("descriptor-bound ability ref missing version")
        })?;
        let selector = crate::ura::AbilitySelector::parse(ability_ura).map_err(|err| {
            AxonError::invalid_argument(format!(
                "descriptor-bound ability ref carries invalid ability URA `{ability_ura}`: {err}"
            ))
        })?;
        ensure_ability_owner_matches_callee(callee_ura, &selector)?;
        return Ok(selector.ability_ura().to_string());
    }

    let ability_ura = match crate::ura::AbilitySelector::parse(ability) {
        Ok(selector) => {
            ensure_ability_owner_matches_callee(callee_ura, &selector)?;
            selector.ability_ura().to_string()
        }
        Err(_) => crate::ura::owner_ability_ura(callee_ura, ability).ok_or_else(|| {
            AxonError::invalid_argument(format!(
                "derive descriptor-bound ability URA for callee `{callee_ura}` ability `{ability}`"
            ))
        })?,
    };
    Ok(ability_ura)
}

pub(crate) fn ability_descriptor_ref_for_wire(
    callee_ura: &str,
    ability: &str,
    descriptor_version: &str,
) -> Result<String, AxonError> {
    let callee_ura = callee_ura.trim();
    let ability = ability.trim();
    if callee_ura.is_empty() {
        return Err(AxonError::invalid_argument(
            "descriptor-bound ability callee URA is empty",
        ));
    }
    if ability.is_empty() {
        return Err(AxonError::invalid_argument(
            "descriptor-bound ability name is empty",
        ));
    }

    if let Ok(descriptor_ref) = canonical_ability_descriptor_ref(ability) {
        let (ability_ura, _) = descriptor_ref.rsplit_once('@').ok_or_else(|| {
            AxonError::invalid_argument("descriptor-bound ability ref missing version")
        })?;
        crate::ura::AbilitySelector::parse(ability_ura)
            .map_err(|err| {
                AxonError::invalid_argument(format!(
                "descriptor-bound ability ref carries invalid ability URA `{ability_ura}`: {err}"
            ))
            })
            .and_then(|selector| ensure_ability_owner_matches_callee(callee_ura, &selector))?;
        return Ok(descriptor_ref);
    }

    let descriptor_version = descriptor_version.trim();
    if descriptor_version.is_empty() {
        return Err(AxonError::invalid_argument(
            "descriptor-bound ability descriptor version is empty",
        ));
    }
    let ability_ura = ability_ura_for_wire(callee_ura, ability)?;
    Ok(format!("{ability_ura}@{descriptor_version}"))
}

fn ensure_ability_owner_matches_callee(
    callee_ura: &str,
    selector: &crate::ura::AbilitySelector,
) -> Result<(), AxonError> {
    if selector.owner_ura() == callee_ura {
        return Ok(());
    }

    Err(AxonError::invalid_argument(format!(
        "descriptor-bound ability owner `{}` does not match callee `{callee_ura}`",
        selector.owner_ura()
    )))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn descriptor_ref_requires_callee_to_own_ability() {
        let callee = crate::ura::device_ura("acme", "host-a");
        let other = crate::ura::device_ura("acme", "host-b");
        let ability_ura = crate::ura::owner_ability_ura(&other, "fs.read").unwrap();
        let ability_ref = format!(
            "{ability_ura}@{}",
            crate::runtime::ability::DEFAULT_ABILITY_DESCRIPTOR_VERSION
        );

        let err = ability_descriptor_ref_for_wire(
            &callee,
            &ability_ref,
            crate::runtime::ability::DEFAULT_ABILITY_DESCRIPTOR_VERSION,
        )
        .unwrap_err();

        assert!(err.to_string().contains("does not match callee"));
    }

    #[test]
    fn descriptor_ref_round_trips_when_callee_owns_ability() {
        let callee = crate::ura::device_ura("acme", "host-a");
        let ability_ura = crate::ura::owner_ability_ura(&callee, "fs.read").unwrap();
        let ability_ref = format!(
            "{ability_ura}@{}",
            crate::runtime::ability::DEFAULT_ABILITY_DESCRIPTOR_VERSION
        );

        let normalized = ability_descriptor_ref_for_wire(
            &callee,
            &ability_ref,
            crate::runtime::ability::DEFAULT_ABILITY_DESCRIPTOR_VERSION,
        )
        .unwrap();

        assert_eq!(normalized, ability_ref);
    }
}

pub(crate) fn descriptor_bound_from_wire_parts(
    envelope: pb::Envelope,
    ability: String,
    payload: &[u8],
    caller_identity: WireCallerIdentity,
) -> Result<WireDescriptorBoundEnvelope, AxonError> {
    let local_system = matches!(caller_identity, WireCallerIdentity::LocalSystem);
    let trace_id = envelope.trace_id.clone();
    let wire_callee = envelope
        .callee
        .ok_or_else(|| AxonError::invalid_argument("wire envelope missing callee"))?;

    let caller = match caller_identity {
        WireCallerIdentity::FromEnvelope => {
            let wire_caller = envelope
                .caller
                .ok_or_else(|| AxonError::invalid_argument("wire envelope missing caller"))?;
            wire::try_agent_identity_from_wire(wire_caller)?
        }
        WireCallerIdentity::LocalSystem => system_agent_identity(),
    };
    let callee = wire::try_agent_identity_from_wire(wire_callee)?;
    let subject = match envelope.subject {
        Some(wire_subject) => wire::try_subject_identity_from_wire(wire_subject)?,
        None if local_system => SubjectIdentity::from_callee(&callee),
        None => {
            return Err(AxonError::invalid_argument("wire envelope missing subject"));
        }
    };
    EntityRef::try_from_subject_identity(&subject).map_err(|err| {
        AxonError::invalid_argument(format!(
            "wire envelope subject is not descriptor-bound: {err}"
        ))
    })?;
    let nonce = match wire::try_invocation_nonce(envelope.invocation_nonce) {
        Ok(nonce) => nonce,
        Err(_) if local_system => fresh_nonce(),
        Err(err) => return Err(err),
    };
    let causal_context = wire::causal_context_from_wire(envelope.causal_context)?;
    let ability = ability_descriptor_ref_for_wire(
        &callee.ura,
        &ability,
        crate::runtime::ability::DEFAULT_ABILITY_DESCRIPTOR_VERSION,
    )?;
    let envelope = DescriptorBoundEnvelope::from_parts(DescriptorBoundEnvelopeParts {
        caller,
        callee,
        ability,
        subject,
        invocation_nonce: nonce,
        causal_context,
        args_bytes: payload,
    })?;
    Ok(WireDescriptorBoundEnvelope { envelope, trace_id })
}
