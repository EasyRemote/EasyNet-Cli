use easynet_axon::pb::axon::v1::invoke_bidi_up::Payload as UpPayload;
use easynet_axon::pb::axon::v1::{
    AgentIdentity, CallerSignature, Envelope, EnvelopeOpen, InvocationTarget, InvokeBidiUp,
    StreamDescriptor, SubjectIdentity,
};
use ed25519_dalek::{Signer as _, SigningKey};
use rand::RngCore as _;

use super::{
    claimant_boot_nonce, ABILITY_SESSION_OPEN, DEVICE_DISPATCH_CONTRACT_VERSION, SESSION_STREAM_ID,
};
use crate::services::invocation_transport::DEFAULT_URA_PROFILE;

/// Optional deterministic Ed25519 seed used to sign frame 0.
pub type SessionSigningSeed = [u8; 32];

/// Build the EnvelopeOpen frame 0 a device sends to open
/// `session.open`. Public so PR-2 commit 1/N's hub-side
/// acceptor tests can construct a matching expected frame, and
/// so the integration test in PR-3 commit 3/3 can drive a mock
/// device through the same shape.
#[must_use]
pub fn build_session_envelope_open(caller_ura: &str) -> InvokeBidiUp {
    build_session_envelope_open_with_seed(caller_ura, None)
}

/// Build the frame-0 `EnvelopeOpen`, optionally signing it when a
/// deterministic device seed is available.
#[must_use]
pub fn build_session_envelope_open_with_seed(
    caller_ura: &str,
    signing_seed: Option<SessionSigningSeed>,
) -> InvokeBidiUp {
    let initial_args = Vec::new();

    let mut envelope = Envelope {
        caller: Some(AgentIdentity {
            ura: caller_ura.to_string(),
            profile: DEFAULT_URA_PROFILE.to_string(),
        }),
        // `session.open` is the device presenting its own long-
        // lived reverse channel; callee + subject both point at the
        // caller device so the signed tuple is stable and self-
        // describing even before a future hub-URA contract lands.
        callee: Some(AgentIdentity {
            ura: caller_ura.to_string(),
            profile: DEFAULT_URA_PROFILE.to_string(),
        }),
        subject: Some(SubjectIdentity {
            ura: caller_ura.to_string(),
            profile: DEFAULT_URA_PROFILE.to_string(),
        }),
        ..Envelope::default()
    };

    // Descriptor-bound frame-0 signing (mirrors the unary prelude in
    // `prelude.rs::sign_descriptor_bound_prelude_request`). The hub's tightened
    // bidi ingress runs the SAME strict signature gate as unary and rejects an
    // empty `EnvelopeOpen.metadata` with "signed public Invoke for `session.open`
    // is missing `x-easynet-signed-descriptor-ref`". `session.open`'s callee ==
    // caller device URA (above), so the descriptor ref's owner matches the route
    // by construction, and the gate re-derives the exact canonical bytes we sign
    // here via `descriptor_bound_from_wire_parts`. Signing the old axiom bytes
    // (the previous `sign_envelope_with_seed` path) would satisfy the metadata
    // presence check but fail signature verification — the sign target MUST be
    // the descriptor-bound canonical bytes.
    let mut session_metadata = std::collections::HashMap::new();
    let mac = if let Some(seed) = signing_seed {
        let descriptor_ref =
            crate::runtime::axon_bridge::descriptor_ref::ability_descriptor_ref_for_wire(
                caller_ura,
                ABILITY_SESSION_OPEN,
                crate::runtime::ability::DEFAULT_ABILITY_DESCRIPTOR_VERSION,
            )
            .expect("session.open descriptor ref is well-formed for the device's own URA");
        if envelope.invocation_nonce.len() != 16 {
            let mut nonce = [0_u8; 16];
            rand::rngs::OsRng.fill_bytes(&mut nonce);
            envelope.invocation_nonce = nonce.to_vec();
        }
        let descriptor_bound =
            crate::runtime::axon_bridge::wire_descriptor::descriptor_bound_from_wire_parts(
                envelope.clone(),
                descriptor_ref.clone(),
                &initial_args,
                crate::runtime::axon_bridge::wire_descriptor::WireCallerIdentity::FromEnvelope,
            )
            .expect("session.open descriptor-bound envelope is complete");
        let signing_key = SigningKey::from_bytes(&seed);
        let signature = signing_key.sign(&descriptor_bound.envelope.canonical_bytes());
        envelope.caller_signature = Some(CallerSignature {
            algorithm: "ed25519".to_string(),
            signature: signature.to_bytes().to_vec(),
            key_id_hint: caller_ura.to_string(),
        });
        session_metadata.insert(
            crate::services::invocation_transport::invocation_wire::SIGNED_DESCRIPTOR_REF_METADATA_KEY
                .to_string(),
            descriptor_ref,
        );
        signature.to_bytes().to_vec()
    } else {
        Vec::new()
    };

    InvokeBidiUp {
        sequence: 0,
        mac,
        payload: Some(UpPayload::EnvelopeOpen(EnvelopeOpen {
            envelope: Some(envelope),
            target: Some(InvocationTarget {
                ability_name: ABILITY_SESSION_OPEN.to_string(),
                ..InvocationTarget::default()
            }),
            initial_args,
            streams: vec![StreamDescriptor {
                stream_id: SESSION_STREAM_ID,
                content_type: "application/json".to_string(),
                ..StreamDescriptor::default()
            }],
            // Carrier negotiation + claimant fingerprint (DEC-F004 /
            // T2.1 step 3, T1.2): declare contract v1 and this
            // process's boot nonce. A pre-carrier hub ignores the
            // unknown field (proto3) and the session runs as v0.
            session_ext: Some(easynet_axon::pb::axon::v1::SessionOpenExt {
                contract_version: DEVICE_DISPATCH_CONTRACT_VERSION,
                claimant_boot_nonce: claimant_boot_nonce().to_vec(),
            }),
            metadata: session_metadata,
            ..EnvelopeOpen::default()
        })),
    }
}
