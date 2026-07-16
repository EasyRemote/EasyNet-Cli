use easynet_axon::pb::axon::v1::invoke_bidi_up::Payload as UpPayload;
use easynet_axon::pb::axon::v1::{
    AgentIdentity, Envelope, EnvelopeOpen, InvokeBidiUp, StreamDescriptor, SubjectIdentity,
};
use rand::RngCore as _;

use super::{
    claimant_boot_nonce, ABILITY_SESSION_OPEN, DEVICE_DISPATCH_CONTRACT_VERSION, SESSION_STREAM_ID,
};
use crate::daemon::identity::self_identity::{CanonicalSigner, SelfIdentityError};
use crate::daemon::invocation::DEFAULT_URA_PROFILE;

/// Build the EnvelopeOpen frame 0 a device sends to open
/// `session.open`. Public so PR-2 commit 1/N's hub-side
/// acceptor tests can construct a matching expected frame, and
/// so the integration test in PR-3 commit 3/3 can drive a mock
/// device through the same shape.
pub async fn build_session_envelope_open(
    signer: &dyn CanonicalSigner,
) -> Result<InvokeBidiUp, SelfIdentityError> {
    let caller_ura = signer.owner_ura();
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
    let descriptor_ref =
        crate::daemon::axon_bridge::descriptor_ref::catalog_descriptor_ref_for_wire(
            caller_ura,
            ABILITY_SESSION_OPEN,
            crate::daemon::ability::CallMode::Bidi,
        )
        .expect("session.open descriptor ref is well-formed for the device's own URA");
    let mut nonce = [0_u8; 16];
    rand::rngs::OsRng.fill_bytes(&mut nonce);
    envelope.invocation_nonce = nonce.to_vec();
    let descriptor_bound =
        crate::daemon::axon_bridge::wire_descriptor::descriptor_bound_from_wire_parts(
            envelope.clone(),
            descriptor_ref.clone(),
            &initial_args,
            crate::daemon::axon_bridge::wire_descriptor::WireCallerIdentity::FromEnvelope,
        )
        .expect("session.open descriptor-bound envelope is complete");
    let caller_signature =
        crate::daemon::invocation::caller_signature::sign_canonical_caller_signature(
            signer,
            &descriptor_bound.envelope.canonical_bytes(),
        )
        .await?;
    let mac = caller_signature.signature.clone();
    envelope.caller_signature = Some(caller_signature);
    let mut session_metadata = std::collections::HashMap::new();
    session_metadata.insert(
        crate::daemon::invocation::dispatch::invocation_wire::SIGNED_DESCRIPTOR_REF_METADATA_KEY
            .to_string(),
        descriptor_ref,
    );
    let target = crate::daemon::invocation::dispatch::invocation_wire::wire_invocation_target(
        ABILITY_SESSION_OPEN,
    )
    .map_err(|err| SelfIdentityError::Unexpected(format!("session.open target: {err}")))?;
    Ok(InvokeBidiUp {
        sequence: 0,
        mac,
        payload: Some(UpPayload::EnvelopeOpen(EnvelopeOpen {
            envelope: Some(envelope),
            target: Some(target),
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
    })
}
