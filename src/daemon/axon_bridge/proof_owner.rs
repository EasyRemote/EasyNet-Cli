//! Axon descriptor-proof owner helpers.
//!
//! The daemon may need descriptor-bound canonical bytes for signing key-service
//! payloads or comparing already-built envelopes, but it must not call Axon's
//! raw proof helpers directly. This module keeps that boundary explicit: all
//! canonical proof material is obtained from Axon's public draft owner.

use axon_sdk::invocation::{DescriptorBoundEnvelope, DescriptorBoundInvocationDraft};

pub(crate) fn descriptor_bound_canonical_bytes(envelope: &DescriptorBoundEnvelope) -> Vec<u8> {
    DescriptorBoundInvocationDraft::from_envelope(envelope.clone()).canonical_bytes()
}
