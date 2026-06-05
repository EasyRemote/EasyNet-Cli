#![cfg(feature = "axon-pb")]

use std::sync::Arc;

use base64::{engine::general_purpose::STANDARD as BASE64_STANDARD, Engine as _};
use easynet_axon::invocation::axiom::{
    canonical_invocation_bytes, AgentIdentity as AxiomAgentIdentity, CausalContext,
    InvocationEnvelope, SubjectIdentity, UraProfile,
};
use easynet_axon::pb::axon::v1::{
    AgentIdentity as PbAgentIdentity, CallerSignature as PbCallerSignature, Envelope, EnvelopeOpen,
    InvocationTarget, InvokeRequest, InvokeServerStreamRequest,
    SubjectIdentity as PbSubjectIdentity,
};
use easynet_cli::services::invocation_transport::AdmissionFacade;
use easynet_cli::services::realm_trust_anchor::RealmTrustAnchor;
use ed25519_dalek::{Signer, SigningKey};
use serde::Serialize;
use sha2::{Digest, Sha256};

const DELEGATION_METADATA_KEY: &str = "x-easynet-delegation";
const SESSION_AUTHORITY_METADATA_KEY: &str = "x-easynet-session-authority";

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
    AdmissionFacade::new(Arc::new(anchor), Some("easynet:///r/realm/hub".to_string()))
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
    let mut hasher = Sha256::new();
    hasher.update(args);
    let args_digest: [u8; 32] = hasher.finalize().into();

    let axiom_env = InvocationEnvelope {
        caller: AxiomAgentIdentity::new(caller_ura, UraProfile::EasynetStrictV2),
        callee: AxiomAgentIdentity::new(callee_ura, UraProfile::EasynetStrictV2),
        subject: SubjectIdentity::new(subject_ura, UraProfile::EasynetStrictV2),
        ability: ability.to_string(),
        args_digest,
        invocation_nonce: nonce,
        causal_context: CausalContext::None,
    };
    let signature = signing_key.sign(&canonical_invocation_bytes(&axiom_env));
    let key_id_hint = BASE64_STANDARD.encode(signing_key.verifying_key().to_bytes());

    InvokeRequest {
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
                algorithm: "ed25519".to_string(),
                signature: signature.to_bytes().to_vec(),
                key_id_hint,
            }),
            ..Envelope::default()
        }),
        function_name: ability.to_string(),
        arguments: args.to_vec(),
        ..InvokeRequest::default()
    }
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
    let payload_bytes = serde_json::to_vec(&payload).expect("canonical delegation payload");
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
    let payload_bytes = serde_json::to_vec(&payload).expect("canonical session authority payload");
    let signature = signer.sign(&payload_bytes);
    let raw = serde_json::json!({
        "payload": serde_json::from_slice::<serde_json::Value>(&payload_bytes)
            .expect("payload value"),
        "signature": BASE64_STANDARD.encode(signature.to_bytes()),
    });
    BASE64_STANDARD.encode(serde_json::to_vec(&raw).expect("raw session authority"))
}

#[test]
fn backend_user_subject_accepts_user_signed_delegation_metadata() {
    let backend_key = SigningKey::from_bytes(&[0x31; 32]);
    let user_key = SigningKey::from_bytes(&[0x41; 32]);
    let caller_ura = "easynet:///r/delegation-it/hub";
    let callee_ura = "easynet:///r/realm/device/device-it";
    let subject_ura = "easynet:///r/realm/user/alice";
    let ability = "device.agent.list";
    let facade =
        admission_facade_with_identities(&[(caller_ura, &backend_key), (subject_ura, &user_key)]);

    let mut request = signed_request(
        caller_ura,
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
            subject_ura,
            subject_ura,
            caller_ura,
            callee_ura,
            &[ability],
        ),
    );

    facade
        .verify_invoke(&request)
        .expect("user-signed delegation admits backend acting for user subject");
}

#[test]
fn backend_user_subject_accepts_backend_signed_session_authority() {
    let signing_key = SigningKey::from_bytes(&[0x37; 32]);
    let caller_ura = "easynet:///r/session-it/hub";
    let callee_ura = "easynet:///r/realm/device/device-it";
    let subject_ura = "easynet:///r/realm/user/alice";
    let ability = "device.agent.list";
    let facade = admission_facade(caller_ura, &signing_key);

    let mut request = signed_request(
        caller_ura,
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
            caller_ura,
            subject_ura,
            "sess-alice-1",
            &[callee_ura],
            &[ability],
        ),
    );

    facade
        .verify_invoke(&request)
        .expect("backend-signed session authority admits backend acting for user subject");
}

#[test]
fn backend_user_subject_without_delegation_metadata_is_denied() {
    let signing_key = SigningKey::from_bytes(&[0x32; 32]);
    let caller_ura = "easynet:///r/delegation-it-missing/hub";
    let facade = admission_facade(caller_ura, &signing_key);
    let request = signed_request(
        caller_ura,
        "easynet:///r/realm/device/device-it-missing",
        "easynet:///r/realm/user/bob",
        "device.agent.read",
        b"{}",
        &signing_key,
        [0x52; 16],
    );

    let err = facade
        .verify_invoke(&request)
        .expect_err("user subject authority metadata is mandatory");
    assert_eq!(err.code(), tonic::Code::PermissionDenied);
    assert!(err.message().contains("AUTHORITY_REQUIRED"));
}

#[test]
fn bootstrap_authority_user_subject_without_delegation_metadata_is_admitted() {
    let signing_key = SigningKey::from_bytes(&[0x34; 32]);
    let caller_ura = "easynet:///r/bootstrap-authority/hub";
    let callee_ura = caller_ura;
    let subject_ura = "easynet:///r/bootstrap-authority/user/alice";
    let facade = admission_facade(caller_ura, &signing_key);

    for (index, ability) in [
        "<self>.register_device_pubkey",
        "<self>.list_user_pubkeys",
        "<self>.revoke_user_pubkey",
        "runtime.bootstrap_self_identity",
        "federation.advertise_agent",
    ]
    .into_iter()
    .enumerate()
    {
        let request = signed_request(
            caller_ura,
            callee_ura,
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
    let caller_ura = "easynet:///r/bootstrap-authority-bad-sig/hub";
    let facade = admission_facade(caller_ura, &trusted_key);
    let request = signed_request(
        caller_ura,
        caller_ura,
        "easynet:///r/bootstrap-authority-bad-sig/user/alice",
        "<self>.register_device_pubkey",
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
    let caller_ura = "easynet:///r/delegation-it-stream/hub";
    let callee_ura = "easynet:///r/realm/device/device-it-stream";
    let subject_ura = "easynet:///r/realm/user/carla";
    let ability = "device.agent.watch";
    let facade = admission_facade_with_identities(&[
        (caller_ura, &signing_key),
        (subject_ura, &signing_key),
    ]);
    let proof = delegation_metadata(
        &signing_key,
        subject_ura,
        subject_ura,
        caller_ura,
        callee_ura,
        &["device.agent.*"],
    );

    let stream_base = signed_request(
        caller_ura,
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
        ..InvokeServerStreamRequest::default()
    };
    stream_request
        .metadata
        .insert(DELEGATION_METADATA_KEY.to_string(), proof.clone());
    facade
        .verify_invoke_stream(&stream_request)
        .expect("server-stream path consumes delegation metadata");

    let bidi_base = signed_request(
        caller_ura,
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
        ..EnvelopeOpen::default()
    };
    open.metadata
        .insert(DELEGATION_METADATA_KEY.to_string(), proof);
    facade
        .verify_envelope_for_bidi(&open)
        .expect("bidi frame-0 path consumes delegation metadata");
}
