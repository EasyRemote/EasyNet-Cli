use easynet_axon::invocation::axiom::{
    AgentIdentity as AxiomAgentIdentity, CausalContext, InvocationEnvelope,
    SubjectIdentity as AxiomSubjectIdentity, UraProfile,
};
use easynet_axon::pb::axon::v1::invoke_bidi_up::Payload as UpPayload;
use easynet_axon::pb::axon::v1::{
    AgentIdentity, CallerSignature, Envelope, EnvelopeOpen, InvocationTarget, InvokeBidiUp,
    StreamDescriptor, SubjectIdentity,
};
use ed25519_dalek::{Signer as _, SigningKey};
use rand::RngCore as _;
use sha2::{Digest, Sha256};
use tonic::Status;

use super::{
    claimant_boot_nonce, ABILITY_SELF_SESSION, DEVICE_DISPATCH_CONTRACT_VERSION, SESSION_STREAM_ID,
};
use crate::services::invocation_transport::DEFAULT_URA_PROFILE;

/// Optional deterministic Ed25519 seed used to sign frame 0.
pub type SessionSigningSeed = [u8; 32];

/// Build the EnvelopeOpen frame 0 a device sends to open
/// `<self>.session`. Public so PR-2 commit 1/N's hub-side
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
        // `<self>.session` is the device presenting its own long-
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

    let mac = if let Some(seed) = signing_seed {
        sign_envelope_with_seed(&mut envelope, ABILITY_SELF_SESSION, &initial_args, &seed)
            .expect("self.session frame-0 envelope is complete")
    } else {
        Vec::new()
    };

    InvokeBidiUp {
        sequence: 0,
        mac,
        payload: Some(UpPayload::EnvelopeOpen(EnvelopeOpen {
            envelope: Some(envelope),
            target: Some(InvocationTarget {
                ability_name: ABILITY_SELF_SESSION.to_string(),
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
            ..EnvelopeOpen::default()
        })),
    }
}

#[allow(deprecated)]
pub(super) fn sign_envelope_with_seed(
    envelope: &mut Envelope,
    ability: &str,
    arguments: &[u8],
    seed: &SessionSigningSeed,
) -> Result<Vec<u8>, Status> {
    if ability.trim().is_empty() {
        return Err(Status::invalid_argument("session signing ability is empty"));
    }
    if envelope.invocation_nonce.len() != 16 {
        let mut nonce = [0_u8; 16];
        rand::rngs::OsRng.fill_bytes(&mut nonce);
        envelope.invocation_nonce = nonce.to_vec();
    }
    let invocation_nonce: [u8; 16] =
        envelope
            .invocation_nonce
            .as_slice()
            .try_into()
            .map_err(|_| {
                Status::internal("session signing nonce must be exactly 16 bytes after refresh")
            })?;
    let caller_ura = envelope
        .caller
        .as_ref()
        .map(|caller| caller.ura.trim())
        .filter(|ura| !ura.is_empty())
        .ok_or_else(|| Status::invalid_argument("session signing caller URA missing"))?;
    let callee_ura = envelope
        .callee
        .as_ref()
        .map(|callee| callee.ura.trim())
        .filter(|ura| !ura.is_empty())
        .ok_or_else(|| Status::invalid_argument("session signing callee URA missing"))?;
    let subject_ura = envelope
        .subject
        .as_ref()
        .map(|subject| subject.ura.trim())
        .filter(|ura| !ura.is_empty())
        .ok_or_else(|| Status::invalid_argument("session signing subject URA missing"))?;

    let args_digest: [u8; 32] = Sha256::digest(arguments).into();
    let axiom_env = InvocationEnvelope {
        caller: AxiomAgentIdentity::new(caller_ura, UraProfile::EasynetStrictV2),
        callee: AxiomAgentIdentity::new(callee_ura, UraProfile::EasynetStrictV2),
        subject: AxiomSubjectIdentity::new(subject_ura, UraProfile::EasynetStrictV2),
        ability: ability.to_string(),
        args_digest,
        invocation_nonce,
        causal_context: CausalContext::None,
    };
    let signing_key = SigningKey::from_bytes(seed);
    // `<self>.session` frame-0 signing is a bootstrap MAC over the wire-pinned
    // session-open tuple, not public Invoke admission or receipt proof
    // material. Public invocation signing must use descriptor-bound canonical
    // bytes; this narrow exception remains until the session-open protocol
    // itself carries a versioned descriptor ref.
    let signature =
        signing_key.sign(&easynet_axon::invocation::axiom::canonical_invocation_bytes(&axiom_env));
    let signature_bytes = signature.to_bytes().to_vec();
    envelope.caller_signature = Some(CallerSignature {
        algorithm: "ed25519".to_string(),
        signature: signature_bytes.clone(),
        key_id_hint: caller_ura.to_string(),
    });
    Ok(signature_bytes)
}
