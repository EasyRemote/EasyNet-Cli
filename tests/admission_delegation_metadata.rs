#![cfg(feature = "axon-pb")]

use std::sync::Arc;

use base64::{engine::general_purpose::STANDARD as BASE64_STANDARD, Engine as _};
use easynet_axon::invocation::axiom::{
    sign_descriptor_bound_invocation, AgentIdentity as AxiomAgentIdentity, CausalContext,
    DescriptorBoundEnvelope, InvocationEnvelope, SubjectIdentity, UraProfile,
};
use easynet_axon::pb::axon::v1::{
    AgentIdentity as PbAgentIdentity, CallerSignature as PbCallerSignature, Envelope, EnvelopeOpen,
    InvocationTarget, InvokeRequest, InvokeServerStreamRequest,
    SubjectIdentity as PbSubjectIdentity,
};
use easynet_cli::daemon::ability::{canonical_json_bytes, DEFAULT_ABILITY_DESCRIPTOR_VERSION};
use easynet_cli::daemon::invocation::AdmissionFacade;
use easynet_cli::daemon::trust::anchor::RealmTrustAnchor;
use ed25519_dalek::{Signer, SigningKey};
use serde::Serialize;
use sha2::Digest as _;

const DELEGATION_METADATA_KEY: &str = "x-easynet-delegation";
const SESSION_AUTHORITY_METADATA_KEY: &str = "x-easynet-session-authority";
const SIGNED_DESCRIPTOR_REF_METADATA_KEY: &str = "x-easynet-signed-descriptor-ref";

#[derive(Serialize)]
struct DelegationPayload {
    issuer_ura: String,
    subject_ura: String,
    caller_ura: String,
    audience: String,
    scopes: Vec<String>,
    issued_at_ms: i64,
    expires_at_ms: i64,
}

#[derive(Serialize)]
struct SessionAuthorityPayload {
    backend_ura: String,
    user_ura: String,
    session_id: String,
    scopes: Vec<String>,
    audiences: Vec<String>,
    issued_at_ms: i64,
    expires_at_ms: i64,
}

fn admission_facade(caller_ura: &str, signing_key: &SigningKey) -> AdmissionFacade {
    admission_facade_with_identities(&[(caller_ura, signing_key)])
}

fn admission_facade_with_identities(identities: &[(&str, &SigningKey)]) -> AdmissionFacade {
    let dir = tempfile::tempdir().expect("tempdir");
    let path = dir.path().join("realm-trust.toml");
    let mut trust = String::new();
    for (ura, signing_key) in identities {
        let public_key_b64 = BASE64_STANDARD.encode(signing_key.verifying_key().to_bytes());
        let role = match easynet_cli::ura::parse_ura(ura).map(|parsed| parsed.kind) {
            Ok(easynet_cli::ura::URAKind::User) => "user",
            Ok(easynet_cli::ura::URAKind::Device) => "device",
            _ => "backend",
        };
        trust.push_str(&format!(
            r#"
[[trusted_agent]]
agent_ura = "{ura}"
public_key_b64 = "{public_key_b64}"
role = "{role}"
added_at_unix_ms = 1714492800000
"#
        ));
    }
    std::fs::write(&path, trust).expect("write trust anchor");
    let anchor = RealmTrustAnchor::try_load_strict(&path).expect("load trust anchor");
    AdmissionFacade::new(Arc::new(anchor), Some(easynet_cli::ura::hub_ura("realm")))
}

fn signed_request(
    caller_ura: &str,
    callee_ura: &str,
    subject_ura: &str,
    ability: &str,
    args: &[u8],
    signing_key: &SigningKey,
    nonce: [u8; 16],
) -> InvokeRequest {
    let caller = AxiomAgentIdentity::new(caller_ura, UraProfile::EasynetStrictV2);
    let callee = AxiomAgentIdentity::new(callee_ura, UraProfile::EasynetStrictV2);
    let subject = SubjectIdentity::new(subject_ura, UraProfile::EasynetStrictV2);
    let ability_ref = format!(
        "{}@{}",
        easynet_cli::ura::owner_ability_ura(callee_ura, ability).expect("callee-owned ability URA"),
        DEFAULT_ABILITY_DESCRIPTOR_VERSION
    );
    let axiom_env = InvocationEnvelope {
        caller,
        callee,
        subject,
        ability: ability_ref.clone(),
        args_digest: sha2::Sha256::digest(args).into(),
        invocation_nonce: nonce,
        causal_context: CausalContext::None,
    };
    let key_id_hint = BASE64_STANDARD.encode(signing_key.verifying_key().to_bytes());
    let descriptor_bound =
        DescriptorBoundEnvelope::new(axiom_env).expect("descriptor-bound test envelope");
    let signature =
        sign_descriptor_bound_invocation(signing_key, &descriptor_bound, key_id_hint.as_str());

    let mut request = InvokeRequest {
        envelope: Some(Envelope {
            caller: Some(PbAgentIdentity {
                ura: caller_ura.to_string(),
                profile: "easynet-strict-v2".to_string(),
            }),
            callee: Some(PbAgentIdentity {
                ura: callee_ura.to_string(),
                profile: "easynet-strict-v2".to_string(),
            }),
            subject: Some(PbSubjectIdentity {
                ura: subject_ura.to_string(),
                profile: "easynet-strict-v2".to_string(),
            }),
            invocation_nonce: nonce.to_vec(),
            caller_signature: Some(PbCallerSignature {
                algorithm: signature.algorithm,
                signature: signature.signature,
                key_id_hint: signature.key_id_hint,
            }),
            ..Envelope::default()
        }),
        function_name: ability.to_string(),
        arguments: args.to_vec(),
        ..InvokeRequest::default()
    };
    request
        .metadata
        .insert(SIGNED_DESCRIPTOR_REF_METADATA_KEY.to_string(), ability_ref);
    request
}

fn delegation_metadata(
    signer: &SigningKey,
    issuer_ura: &str,
    subject_ura: &str,
    caller_ura: &str,
    audience: &str,
    scopes: &[&str],
) -> String {
    let payload = DelegationPayload {
        issuer_ura: issuer_ura.to_string(),
        subject_ura: subject_ura.to_string(),
        caller_ura: caller_ura.to_string(),
        audience: audience.to_string(),
        scopes: scopes.iter().map(|scope| (*scope).to_string()).collect(),
        issued_at_ms: 1_700_000_000_000,
        expires_at_ms: 4_102_444_800_000,
    };
    let payload_bytes = canonical_payload_bytes(&payload);
    let signature = signer.sign(&payload_bytes);
    let raw = serde_json::json!({
        "payload": serde_json::from_slice::<serde_json::Value>(&payload_bytes)
            .expect("payload value"),
        "signature": BASE64_STANDARD.encode(signature.to_bytes()),
    });
    BASE64_STANDARD.encode(serde_json::to_vec(&raw).expect("raw delegation proof"))
}

fn session_authority_metadata(
    signer: &SigningKey,
    backend_ura: &str,
    user_ura: &str,
    session_id: &str,
    audiences: &[&str],
    scopes: &[&str],
) -> String {
    let payload = SessionAuthorityPayload {
        backend_ura: backend_ura.to_string(),
        user_ura: user_ura.to_string(),
        session_id: session_id.to_string(),
        scopes: scopes.iter().map(|scope| (*scope).to_string()).collect(),
        audiences: audiences
            .iter()
            .map(|audience| (*audience).to_string())
            .collect(),
        issued_at_ms: 1_700_000_000_000,
        expires_at_ms: 4_102_444_800_000,
    };
    let payload_bytes = canonical_payload_bytes(&payload);
    let signature = signer.sign(&payload_bytes);
    let raw = serde_json::json!({
        "payload": serde_json::from_slice::<serde_json::Value>(&payload_bytes)
            .expect("payload value"),
        "signature": BASE64_STANDARD.encode(signature.to_bytes()),
    });
    BASE64_STANDARD.encode(serde_json::to_vec(&raw).expect("raw session authority"))
}

fn canonical_payload_bytes<T: Serialize>(payload: &T) -> Vec<u8> {
    let value = serde_json::to_value(payload).expect("payload value");
    canonical_json_bytes(&value)
}

fn delegation_metadata_from_value(signer: &SigningKey, payload: serde_json::Value) -> String {
    let payload_bytes = canonical_json_bytes(&payload);
    let signature = signer.sign(&payload_bytes);
    let raw = serde_json::json!({
        "payload": payload,
        "signature": BASE64_STANDARD.encode(signature.to_bytes()),
    });
    BASE64_STANDARD.encode(serde_json::to_vec(&raw).expect("raw delegation proof"))
}

#[test]
fn backend_session_subject_accepts_user_signed_delegation_metadata() {
    let backend_key = SigningKey::from_bytes(&[0x31; 32]);
    let user_key = SigningKey::from_bytes(&[0x41; 32]);
    let caller_ura = easynet_cli::ura::hub_ura("delegation-it");
    let callee_ura = "easynet:///r/realm/device/device-it";
    let user_ura = "easynet:///r/realm/user/alice";
    let subject_ura = "easynet:///r/realm/session/sess-alice-1";
    let ability = "agent.list";
    let facade =
        admission_facade_with_identities(&[(&caller_ura, &backend_key), (user_ura, &user_key)]);

    let mut request = signed_request(
        &caller_ura,
        callee_ura,
        subject_ura,
        ability,
        b"{}",
        &backend_key,
        [0x51; 16],
    );
    request.metadata.insert(
        DELEGATION_METADATA_KEY.to_string(),
        delegation_metadata(
            &user_key,
            user_ura,
            subject_ura,
            &caller_ura,
            callee_ura,
            &[ability],
        ),
    );

    facade
        .verify_invoke(&request)
        .expect("user-signed delegation admits backend acting for session subject");
}

#[test]
fn delegation_metadata_verifies_against_canonical_payload_order() {
    let backend_key = SigningKey::from_bytes(&[0x35; 32]);
    let user_key = SigningKey::from_bytes(&[0x45; 32]);
    let caller_ura = easynet_cli::ura::hub_ura("delegation-it-canonical");
    let callee_ura = "easynet:///r/realm/device/device-canonical";
    let user_ura = "easynet:///r/realm/user/alice";
    let subject_ura = "easynet:///r/realm/session/sess-canonical-1";
    let ability = "agent.list";
    let facade =
        admission_facade_with_identities(&[(&caller_ura, &backend_key), (user_ura, &user_key)]);

    let mut request = signed_request(
        &caller_ura,
        callee_ura,
        subject_ura,
        ability,
        b"{}",
        &backend_key,
        [0x55; 16],
    );
    request.metadata.insert(
        DELEGATION_METADATA_KEY.to_string(),
        delegation_metadata_from_value(
            &user_key,
            serde_json::json!({
                "scopes": [ability],
                "expires_at_ms": 4_102_444_800_000_i64,
                "audience": callee_ura,
                "caller_ura": caller_ura,
                "issued_at_ms": 1_700_000_000_000_i64,
                "subject_ura": subject_ura,
                "issuer_ura": user_ura,
            }),
        ),
    );

    facade
        .verify_invoke(&request)
        .expect("delegation signature must verify canonical payload bytes, not struct field order");
}

#[test]
fn backend_session_subject_accepts_backend_signed_session_authority() {
    let signing_key = SigningKey::from_bytes(&[0x37; 32]);
    let caller_ura = easynet_cli::ura::hub_ura("session-it");
    let callee_ura = "easynet:///r/realm/device/device-it";
    let user_ura = "easynet:///r/realm/user/alice";
    let subject_ura = "easynet:///r/realm/session/sess-alice-1";
    let ability = "agent.list";
    let facade = admission_facade(&caller_ura, &signing_key);

    let mut request = signed_request(
        &caller_ura,
        callee_ura,
        subject_ura,
        ability,
        b"{}",
        &signing_key,
        [0x57; 16],
    );
    request.metadata.insert(
        SESSION_AUTHORITY_METADATA_KEY.to_string(),
        session_authority_metadata(
            &signing_key,
            &caller_ura,
            user_ura,
            "sess-alice-1",
            &[callee_ura],
            &[ability],
        ),
    );

    facade
        .verify_invoke(&request)
        .expect("backend-signed session authority admits backend acting for session subject");
}

#[test]
fn backend_session_subject_without_authority_metadata_is_denied() {
    let signing_key = SigningKey::from_bytes(&[0x32; 32]);
    let caller_ura = easynet_cli::ura::hub_ura("delegation-it-missing");
    let facade = admission_facade(&caller_ura, &signing_key);
    let request = signed_request(
        &caller_ura,
        "easynet:///r/realm/device/device-it-missing",
        "easynet:///r/realm/session/sess-bob-1",
        "agent.read",
        b"{}",
        &signing_key,
        [0x52; 16],
    );

    let err = facade
        .verify_invoke(&request)
        .expect_err("session subject authority metadata is mandatory");
    assert_eq!(err.code(), tonic::Code::PermissionDenied);
    assert!(err.message().contains("AUTHORITY_REQUIRED"));
}

#[test]
fn bootstrap_authority_session_subject_without_delegation_metadata_is_admitted() {
    let signing_key = SigningKey::from_bytes(&[0x34; 32]);
    let caller_ura = easynet_cli::ura::hub_ura("bootstrap-authority");
    let callee_ura = caller_ura.clone();
    let subject_ura = "easynet:///r/bootstrap-authority/session/bootstrap-alice";
    let facade = admission_facade(&caller_ura, &signing_key);

    for (index, ability) in [
        "identity.register_pubkey",
        "identity.list_user_pubkeys",
        "identity.revoke_user_pubkey",
        "runtime.bootstrap_self_identity",
        "federation.advertise_agent",
    ]
    .into_iter()
    .enumerate()
    {
        let request = signed_request(
            &caller_ura,
            &callee_ura,
            subject_ura,
            ability,
            b"{}",
            &signing_key,
            [0x60 + index as u8; 16],
        );
        facade
            .verify_invoke(&request)
            .unwrap_or_else(|err| panic!("{ability} should use bootstrap authority: {err}"));
    }
}

#[test]
fn bootstrap_authority_still_rejects_bad_caller_signature() {
    let trusted_key = SigningKey::from_bytes(&[0x35; 32]);
    let wrong_key = SigningKey::from_bytes(&[0x36; 32]);
    let caller_ura = easynet_cli::ura::hub_ura("bootstrap-authority-bad-sig");
    let facade = admission_facade(&caller_ura, &trusted_key);
    let request = signed_request(
        &caller_ura,
        &caller_ura,
        "easynet:///r/bootstrap-authority-bad-sig/session/bootstrap-alice",
        "identity.register_pubkey",
        b"{}",
        &wrong_key,
        [0x65; 16],
    );

    let err = facade
        .verify_invoke(&request)
        .expect_err("bootstrap authority must not bypass caller admission");
    assert!(err.message().contains("CALLER_SIGNATURE_INVALID"));
}

#[test]
fn stream_and_bidi_verify_delegation_metadata() {
    let signing_key = SigningKey::from_bytes(&[0x33; 32]);
    let caller_ura = easynet_cli::ura::hub_ura("delegation-it-stream");
    let callee_ura = "easynet:///r/realm/device/device-it-stream";
    let user_ura = "easynet:///r/realm/user/carla";
    let subject_ura = "easynet:///r/realm/session/sess-carla-1";
    let ability = "agent.watch";
    let facade =
        admission_facade_with_identities(&[(&caller_ura, &signing_key), (user_ura, &signing_key)]);
    let proof = delegation_metadata(
        &signing_key,
        user_ura,
        subject_ura,
        &caller_ura,
        callee_ura,
        &["agent.*"],
    );

    let stream_base = signed_request(
        &caller_ura,
        callee_ura,
        subject_ura,
        ability,
        b"{}",
        &signing_key,
        [0x53; 16],
    );
    let mut stream_request = InvokeServerStreamRequest {
        envelope: stream_base.envelope.clone(),
        function_name: ability.to_string(),
        arguments: b"{}".to_vec(),
        metadata: stream_base.metadata.clone(),
        ..InvokeServerStreamRequest::default()
    };
    stream_request
        .metadata
        .insert(DELEGATION_METADATA_KEY.to_string(), proof.clone());
    facade
        .verify_invoke_stream(&stream_request)
        .expect("server-stream path consumes delegation metadata");

    let bidi_base = signed_request(
        &caller_ura,
        callee_ura,
        subject_ura,
        ability,
        b"{}",
        &signing_key,
        [0x54; 16],
    );
    let mut open = EnvelopeOpen {
        envelope: bidi_base.envelope.clone(),
        target: Some(InvocationTarget {
            ability_name: ability.to_string(),
            ..InvocationTarget::default()
        }),
        initial_args: b"{}".to_vec(),
        metadata: bidi_base.metadata.clone(),
        ..EnvelopeOpen::default()
    };
    open.metadata
        .insert(DELEGATION_METADATA_KEY.to_string(), proof);
    facade
        .verify_envelope_for_bidi(&open)
        .expect("bidi frame-0 path consumes delegation metadata");
}
