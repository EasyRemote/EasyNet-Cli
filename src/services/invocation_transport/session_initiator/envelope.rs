use easynet_axon::invocation::axiom::{
    canonical_invocation_bytes, AgentIdentity as AxiomAgentIdentity, CausalContext,
    InvocationEnvelope, SubjectIdentity as AxiomSubjectIdentity, UraProfile,
};
use easynet_axon::pb::axon::v1::invoke_bidi_up::Payload as UpPayload;
use easynet_axon::pb::axon::v1::{
    AgentIdentity, CallerSignature, Envelope, EnvelopeOpen, InvocationTarget, InvokeBidiUp,
    StreamDescriptor, SubjectIdentity,
};
use ed25519_dalek::{Signer as _, SigningKey};
use rand::RngCore as _;
use sha2::{Digest, Sha256};

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
    let args_digest: [u8; 32] = Sha256::digest(&initial_args).into();

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

    let mut mac = Vec::new();
    if let Some(seed) = signing_seed {
        let mut nonce = [0_u8; 16];
        rand::rngs::OsRng.fill_bytes(&mut nonce);
        envelope.invocation_nonce = nonce.to_vec();

        let axiom_env = InvocationEnvelope {
            caller: AxiomAgentIdentity::new(caller_ura, UraProfile::EasynetStrictV2),
            callee: AxiomAgentIdentity::new(caller_ura, UraProfile::EasynetStrictV2),
            subject: AxiomSubjectIdentity::new(caller_ura, UraProfile::EasynetStrictV2),
            ability: ABILITY_SELF_SESSION.to_string(),
            args_digest,
            invocation_nonce: nonce,
            causal_context: CausalContext::None,
        };
        let signing_key = SigningKey::from_bytes(&seed);
        let signature = signing_key.sign(&canonical_invocation_bytes(&axiom_env));
        mac = signature.to_bytes().to_vec();
        envelope.caller_signature = Some(CallerSignature {
            algorithm: "ed25519".to_string(),
            signature: mac.clone(),
            ..CallerSignature::default()
        });
    }

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
