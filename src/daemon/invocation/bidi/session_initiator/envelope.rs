use axon_sdk::pb::axon::v1::invoke_bidi_up::Payload as UpPayload;
use axon_sdk::pb::axon::v1::{EnvelopeOpen, InvokeBidiUp, StreamDescriptor};

use super::{
    claimant_boot_nonce, ABILITY_SESSION_OPEN, DEVICE_DISPATCH_CONTRACT_VERSION, SESSION_STREAM_ID,
};
use crate::daemon::identity::self_identity::{CanonicalSigner, SelfIdentityError};

/// Build the EnvelopeOpen frame 0 a device sends to open
/// `session.open`. Public so PR-2 commit 1/N's hub-side
/// acceptor tests can construct a matching expected frame, and
/// so the integration test in PR-3 commit 3/3 can drive a mock
/// device through the same shape.
pub async fn build_session_envelope_open(
    signer: &dyn CanonicalSigner,
) -> Result<InvokeBidiUp, SelfIdentityError> {
    let caller_ura = signer.owner_ura();
    let caller = crate::core::ura::parse_ura(caller_ura).map_err(|error| {
        SelfIdentityError::Unexpected(format!(
            "session.open caller URA `{caller_ura}` is invalid: {error}"
        ))
    })?;
    let hub_ura = crate::core::ura::hub_ura(&caller.realm);
    let initial_args = Vec::new();

    // Descriptor-bound frame-0 signing mirrors the unary prelude. The
    // descriptor owner and callee are the realm Authority, while the Device
    // remains the signed caller and subject. The canonical typed target carries
    // the descriptor ref; route and product metadata remain separate. The gate
    // re-derives these exact canonical bytes through
    // `descriptor_bound_from_wire_parts`; signing the old axiom bytes would
    // satisfy metadata presence but fail signature verification.
    let descriptor_ref =
        crate::daemon::axon_bridge::descriptor_ref::catalog_descriptor_ref_for_wire(
            &hub_ura,
            ABILITY_SESSION_OPEN,
            crate::daemon::ability::CallMode::Bidi,
        )
        .expect("session.open descriptor ref is well-formed for the realm Authority URA");
    let request = crate::daemon::invocation::ProtoEnvelope::from_target(
        caller_ura,
        &hub_ura,
        caller_ura,
        crate::daemon::invocation::RootInvocationDerivationIssuer::fresh_root(),
    )
    .map_err(|error| SelfIdentityError::Unexpected(format!("session.open envelope: {error}")))?
    .signed_descriptor_ref_invoke_request_with_signer(
        ABILITY_SESSION_OPEN,
        descriptor_ref,
        initial_args.clone(),
        signer,
    )
    .await
    .map_err(|error| SelfIdentityError::Unexpected(format!("session.open signing: {error}")))?;
    let envelope = request.envelope.ok_or_else(|| {
        SelfIdentityError::Unexpected("session.open builder omitted envelope".to_string())
    })?;
    let target = request.target.ok_or_else(|| {
        SelfIdentityError::Unexpected("session.open builder omitted typed target".to_string())
    })?;
    let mac = envelope
        .caller_signature
        .as_ref()
        .map(|signature| signature.signature.clone())
        .ok_or_else(|| {
            SelfIdentityError::Unexpected(
                "session.open builder omitted caller signature".to_string(),
            )
        })?;
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
            // Canonical carrier negotiation + claimant fingerprint
            // (DEC-F004 / T2.1 step 3, T1.2). Contract v1 is mandatory;
            // the Hub provider rejects absent or retired v0 negotiation.
            session_ext: Some(axon_sdk::pb::axon::v1::SessionOpenExt {
                contract_version: DEVICE_DISPATCH_CONTRACT_VERSION,
                claimant_boot_nonce: claimant_boot_nonce().to_vec(),
            }),
            metadata: request.metadata,
            ..EnvelopeOpen::default()
        })),
    })
}
