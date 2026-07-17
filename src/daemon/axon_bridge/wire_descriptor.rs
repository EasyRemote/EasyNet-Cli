//! Shared proto-to-Axon descriptor-bound envelope reassembly.
//!
//! The daemon receives Axon protobuf envelopes from several transport
//! surfaces. Every one of them must reconstruct the same
//! `DescriptorBoundEnvelope` before admission or dispatch; this module is the
//! single place that owns that conversion.

use easynet_axon::invocation::{
    wire, AxonError, DescriptorBoundEnvelope, DescriptorBoundEnvelopeParts, EntityRef,
};
use easynet_axon::pb::axon::v1 as pb;

use crate::daemon::axon_bridge::descriptor_ref::require_descriptor_ref_for_wire;

#[derive(Debug)]
pub(crate) struct WireDescriptorBoundEnvelope {
    pub envelope: DescriptorBoundEnvelope,
    pub trace_id: String,
}

pub(crate) fn descriptor_bound_from_wire_parts(
    envelope: pb::Envelope,
    ability: String,
    payload: &[u8],
) -> Result<WireDescriptorBoundEnvelope, AxonError> {
    let trace_id = envelope.trace_id.clone();
    let wire_callee = envelope
        .callee
        .ok_or_else(|| AxonError::invalid_argument("wire envelope missing callee"))?;
    let wire_caller = envelope
        .caller
        .ok_or_else(|| AxonError::invalid_argument("wire envelope missing caller"))?;
    let caller = wire::try_agent_identity_from_wire(wire_caller)?;
    let callee = wire::try_agent_identity_from_wire(wire_callee)?;
    let subject = match envelope.subject {
        Some(wire_subject) => wire::try_subject_identity_from_wire(wire_subject)?,
        None => {
            return Err(AxonError::invalid_argument("wire envelope missing subject"));
        }
    };
    EntityRef::try_from_subject_identity(&subject).map_err(|err| {
        AxonError::invalid_argument(format!(
            "wire envelope subject is not descriptor-bound: {err}"
        ))
    })?;
    let nonce = wire::try_invocation_nonce(envelope.invocation_nonce)?;
    let causal_context = wire::causal_context_from_wire(envelope.causal_context)?;
    let ability = require_descriptor_ref_for_wire(&callee.ura, &ability)?;
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

#[cfg(test)]
mod tests {
    use super::*;

    const CALLEE_URA: &str = "easynet:///r/acme/device/edge-1";

    fn complete_envelope() -> pb::Envelope {
        pb::Envelope {
            caller: Some(pb::AgentIdentity {
                ura: crate::daemon::identity::local_invocation::LOCAL_SYSTEM_AGENT_URA.to_string(),
                profile: "easynet-strict-v2".to_string(),
            }),
            callee: Some(pb::AgentIdentity {
                ura: CALLEE_URA.to_string(),
                profile: "easynet-strict-v2".to_string(),
            }),
            subject: Some(pb::SubjectIdentity {
                ura: CALLEE_URA.to_string(),
                profile: "easynet-strict-v2".to_string(),
            }),
            invocation_nonce: [0x51; 16].to_vec(),
            causal_context: Some(pb::CausalContext {
                form: Some(pb::causal_context::Form::None(pb::Empty {})),
            }),
            ..pb::Envelope::default()
        }
    }

    fn descriptor_ref() -> String {
        let binding = crate::daemon::axon_bridge::descriptor_ref::descriptor_binding_for_wire(
            "1.0.0", [0x33; 32], "invoke",
        )
        .expect("descriptor binding");
        crate::daemon::axon_bridge::descriptor_ref::ability_descriptor_ref_for_wire(
            CALLEE_URA, "job.run", &binding,
        )
        .expect("descriptor ref")
    }

    #[test]
    fn complete_local_system_wire_envelope_reassembles_without_derivation() {
        let reassembled =
            descriptor_bound_from_wire_parts(complete_envelope(), descriptor_ref(), b"{}")
                .expect("complete envelope");
        let envelope = reassembled.envelope.envelope();
        assert_eq!(
            envelope.caller.ura,
            crate::daemon::identity::local_invocation::LOCAL_SYSTEM_AGENT_URA
        );
        assert_eq!(envelope.subject.ura, CALLEE_URA);
        assert_eq!(envelope.invocation_nonce, [0x51; 16]);
    }

    #[test]
    fn local_system_wire_reassembly_rejects_missing_caller() {
        let mut envelope = complete_envelope();
        envelope.caller = None;
        let error = descriptor_bound_from_wire_parts(envelope, descriptor_ref(), b"{}")
            .expect_err("caller is mandatory");
        assert!(error.to_string().contains("wire envelope missing caller"));
    }

    #[test]
    fn local_system_wire_reassembly_rejects_missing_subject() {
        let mut envelope = complete_envelope();
        envelope.subject = None;
        let error = descriptor_bound_from_wire_parts(envelope, descriptor_ref(), b"{}")
            .expect_err("subject is mandatory");
        assert!(error.to_string().contains("wire envelope missing subject"));
    }

    #[test]
    fn local_system_wire_reassembly_rejects_invalid_nonce() {
        let mut envelope = complete_envelope();
        envelope.invocation_nonce.clear();
        let error = descriptor_bound_from_wire_parts(envelope, descriptor_ref(), b"{}")
            .expect_err("nonce is mandatory");
        assert!(error.to_string().contains("invocation_nonce"));
    }
}
