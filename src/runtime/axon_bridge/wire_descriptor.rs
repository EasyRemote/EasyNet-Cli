//! Shared proto-to-Axon descriptor-bound envelope reassembly.
//!
//! The daemon receives Axon protobuf envelopes from several transport
//! surfaces. Every one of them must reconstruct the same
//! `DescriptorBoundEnvelope` before admission or dispatch; this module is the
//! single place that owns that conversion.

use easynet_axon::invocation::{
    fresh_nonce, wire, AxonError, DescriptorBoundEnvelope, DescriptorBoundEnvelopeParts, EntityRef,
    SubjectIdentity,
};
use easynet_axon::pb::axon::v1 as pb;

use crate::runtime::axon_bridge::descriptor_ref::ability_descriptor_ref_for_wire;
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
