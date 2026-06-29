// EasyNet Daemon — Invocation Service Behavior Tests
// ====================================================
//
// File: src/services/invocation_transport/daemon_invocation_service_tests.rs
// Description: Service-level behavior tests for the daemon Invocation
//              surface (admission, quota, all three RPC shells, and the
//              dispatcher arms they delegate to). Linked from
//              daemon_invocation_service.rs via `#[path]` so `super::*`
//              still resolves to the service module (commit-plan-2 E6:
//              the god-file keeps its tests' coverage, not their lines).
//
//              New tests for a single dispatcher belong in that
//              dispatcher's own module; this file is for cross-surface
//              behavior that needs the assembled service.
//
// Author: Silan Hu <silan.hu@u.nus.edu>
// Copyright (c) 2026 EasyNet. All rights reserved.

use super::*;
use crate::runtime::ability::HOSTED_AGENT_DELEGATION_METADATA_KEY;
use crate::services::invocation_transport::bidi_dispatcher::{
    build_bidi_terminal_receipt, build_remote_bidi_open_dispatch_frame,
    build_remote_bidi_open_frame_for_contract, build_session_request_result_frame,
    extract_envelope_open, map_local_bidi_ability_frame, map_local_bidi_handler_frame,
    map_local_bidi_up_payload, push_session_request_result,
    refresh_session_owner_projection_lease_at, remote_bidi_target_ura, validate_session_realm,
    LocalBidiDownStream, LocalBidiHandlerFrame, LocalBidiUpFrame, LocalBidiWireKind,
    REASON_BIDI_FIRST_FRAME_SEQUENCE, REASON_BIDI_NON_STRICT_ORDERING,
};
use crate::services::invocation_transport::federation_wrappers;
use crate::services::invocation_transport::invocation_wire::FEDERATION_RESULT_CONTENT_TYPE;
use crate::services::invocation_transport::invocation_wire::{
    DELEGATION_METADATA_KEY, SESSION_AUTHORITY_METADATA_KEY,
};
use crate::services::invocation_transport::invoke_remote_initiator::{
    InvokeRemoteDown, RequestOutcome, SessionContentEnvelope, SessionDispatch, SessionRequestError,
    INVOKE_REMOTE_STREAM_ID,
};
use crate::services::invocation_transport::ledger_projection::{
    invocation_resource_ura, ledger_authority_form_for_request, ledger_record_from_remote_receipt,
};
use crate::services::invocation_transport::peer_envelope_signer::sign_peer_request_envelope;
use crate::services::invocation_transport::quota_meter::quota_meters_function;
use crate::services::invocation_transport::register_device_pubkey::parse_realm_from_ura;
use crate::services::invocation_transport::session_initiator::ABILITY_SESSION_OPEN;
use crate::services::invocation_transport::target_gate::ROUTE_NEGATIVE_CODE;
use crate::services::invocation_transport::ProtoEnvelope;
use crate::services::pending_dispatch::DispatchResult;
use crate::services::session_failure::SessionFailure;
use easynet_axon::invocation::{AbilityFrame, BidiInputFrame};
use easynet_axon::pb::axon::v1::Error;
use easynet_axon::pb::axon::v1::{
    invoke_bidi_down::Payload as DownPayload, BinaryChunk, ErrorStage, SecurityClass,
    StreamDescriptor,
};
use std::collections::HashMap;
use tokio::sync::mpsc;

use crate::services::realm_trust_anchor::{RealmTrustAnchor, TrustedAgent, TrustedAgentRole};
use crate::services::usage_quota_store::SharedUsageQuotaGate;
use easynet_axon::pb::axon::v1::{
    AgentIdentity, CallerSignature, Envelope, InvocationReceipt, InvocationUsage, SubjectIdentity,
};

/// Test helper daemon URA — admitted by the test admission
/// facade via the loopback bypass. Tests that exercise
/// admission rejection construct a different facade.
// URA v4.1.4: daemons are devices, not agents. Fixtures use the
// canonical shape because forward_invoke no longer repairs legacy
// `agent/<bare-id>` device aliases at the request boundary.
const TEST_DAEMON_URI: &str = "easynet:///r/test-realm/device/test-daemon";
const TEST_DEVICE_SIGNING_SEED: [u8; 32] = [0x33; 32];

fn make_service() -> DaemonInvocationService {
    let admission = AdmissionFacade::new(
        Arc::new(RealmTrustAnchor::default()),
        Some(TEST_DAEMON_URI.to_string()),
    );
    DaemonInvocationService::new(Arc::new(PresenceRegistry::new()), admission)
        .with_hub_signing_seed([0x11; 32])
}

fn publish_test_route(svc: &DaemonInvocationService, owner_ura: &str, public_name: &str) {
    publish_test_route_hosted_by(svc, owner_ura, public_name, TEST_DAEMON_URI);
}

fn publish_test_route_hosted_by(
    svc: &DaemonInvocationService,
    owner_ura: &str,
    public_name: &str,
    hosted_agent_host_ura: &str,
) {
    let public_name = crate::ura::owner_local_ability_name(owner_ura, public_name);
    let ability_ura = crate::ura::owner_ability_ura(owner_ura, &public_name)
        .unwrap_or_else(|| panic!("derive test ability URA for {owner_ura} {public_name}"));
    let host_ura = match crate::ura::parse_ura(owner_ura).map(|parsed| parsed.kind) {
        Ok(crate::ura::URAKind::Agent) => {
            svc.directory.advertised_agents.upsert(
                crate::services::advertised_agent_store::AdvertisedAgentRecord {
                    agent_ura: owner_ura.to_string(),
                    public_key_hex: String::new(),
                    host_node_id: Some(hosted_agent_host_ura.to_string()),
                    signing_authority:
                        crate::services::advertised_agent_store::AdvertisedAgentSigningAuthority::HostedBy {
                            host_ura: hosted_agent_host_ura.to_string(),
                        },
                },
            );
            hosted_agent_host_ura.to_string()
        }
        _ => owner_ura.to_string(),
    };
    if svc.directory.presence.lookup(&host_ura).is_none() {
        let (tx, _rx) = tokio::sync::mpsc::channel(1);
        svc.directory.presence.insert(host_ura.clone(), tx);
    }
    let (namespace, local_name) = public_name
        .rsplit_once('.')
        .map_or(("", public_name.as_str()), |(namespace, local_name)| {
            (namespace, local_name)
        });
    svc.directory.ability_catalog.upsert_projection(
        crate::services::ability_catalog_store::OwnerAbilityProjectionRow::new(
            owner_ura.to_string(),
            host_ura,
            1,
            "sha256:test".to_string(),
            4_102_444_800_000,
            vec![crate::runtime::owner_projection::AbilityProjectionSummary {
                ability_ura: ability_ura.clone(),
                owner_ura: owner_ura.to_string(),
                namespace: namespace.to_string(),
                local_name: local_name.to_string(),
                descriptor_revision: "sha256:descriptor".to_string(),
                schema_ref: None,
                schema_hash: None,
                policy_ref: "visibility:PUBLIC".to_string(),
                route_summary_ref: Some(format!("route-ref::{ability_ura}")),
                tags: vec!["class:unary".to_string()],
                callable_summary: crate::runtime::owner_projection::AbilityCallableSummary::minimal(
                    public_name.to_string(),
                ),
            }],
        ),
    );
}

fn session_request_ability_ura(realm: &str, ability: &str) -> String {
    crate::ura::hub_ability_ura(realm, ability)
}

fn signed_delegation_metadata_for_test(
    signer: &ed25519_dalek::SigningKey,
    issuer_ura: &str,
    subject_ura: &str,
    caller_ura: &str,
    audience: &str,
    scopes: &[&str],
) -> String {
    use base64::{engine::general_purpose::STANDARD as BASE64_STANDARD, Engine as _};
    use ed25519_dalek::Signer as _;
    use serde::Serialize;

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

    let payload = DelegationPayload {
        issuer_ura: issuer_ura.to_string(),
        subject_ura: subject_ura.to_string(),
        caller_ura: caller_ura.to_string(),
        audience: audience.to_string(),
        scopes: scopes.iter().map(|scope| (*scope).to_string()).collect(),
        issued_at_ms: 1_700_000_000_000,
        expires_at_ms: 4_102_444_800_000,
    };
    let payload_value = serde_json::to_value(&payload).expect("delegation payload");
    let payload_bytes = crate::runtime::ability::canonical_json_bytes(&payload_value);
    let signature = signer.sign(&payload_bytes);
    let raw = serde_json::json!({
        "payload": payload_value,
        "signature": BASE64_STANDARD.encode(signature.to_bytes()),
    });
    BASE64_STANDARD.encode(serde_json::to_vec(&raw).expect("delegation proof"))
}

fn make_quota_service_for_device_caller(caller_ura: &str, cap: i32) -> DaemonInvocationService {
    use base64::{engine::general_purpose::STANDARD as BASE64_STANDARD, Engine as _};

    let signing_key = test_device_signing_key();
    let anchor = RealmTrustAnchor::from_entries(vec![TrustedAgent {
        agent_ura: caller_ura.to_string(),
        public_key_b64: BASE64_STANDARD.encode(signing_key.verifying_key().to_bytes()),
        role: TrustedAgentRole::Device,
        added_at_unix_ms: 1_700_000_000_000,
        origin_realm: None,
        hub_endpoint: None,
        tls_ca_pem_path: None,
    }])
    .expect("quota test anchor");
    let quota = crate::persistence::daemon_config::QuotaConfig::new(
        cap,
        60_000,
        std::collections::BTreeMap::new(),
    );
    let admission = AdmissionFacade::new(Arc::new(anchor), Some(TEST_DAEMON_URI.to_string()))
        .with_quota_gate(SharedUsageQuotaGate::from_policy(Some(quota)));
    DaemonInvocationService::new(Arc::new(PresenceRegistry::new()), admission)
        .with_hub_signing_seed([0x11; 32])
}

async fn runtime_with_json_echo(
    owner_ura: &str,
    ability: &'static str,
    marker_key: &'static str,
    marker_value: &'static str,
) -> Arc<easynet_axon::invocation::LocalRuntime> {
    use easynet_axon::invocation::{make_ability, AbilityCallModes, AbilityOptions};

    // Register under the canonical owner ability URA the resolver looks up
    // (route_resolver resolve_owner_ability -> owner_ability_ura(owner,
    // name)). A raw LocalRuntime has no AxonAbilityCatalog to mirror a bare
    // key into the canonical one, so the test must register the canonical
    // key directly or resolve-first returns ROUTE_NEGATIVE.
    let runtime_ability = crate::ura::owner_ability_ura(
        owner_ura,
        &crate::ura::owner_local_ability_name(owner_ura, ability),
    )
    .unwrap_or_else(|| panic!("derive runtime ability URA for {owner_ura} {ability}"));
    let rt = easynet_axon::invocation::LocalRuntime::new();
    rt.register_ability_with_options(
        runtime_ability,
        make_ability(move |ctx| async move {
            let echoed_args: serde_json::Value =
                serde_json::from_slice(&ctx.payload).unwrap_or(serde_json::Value::Null);
            Ok(serde_json::to_vec(&serde_json::json!({
                marker_key: marker_value,
                "echoed_args": echoed_args,
            }))
            .unwrap())
        }),
        AbilityOptions::default()
            .with_modes(AbilityCallModes::RPC)
            .with_descriptor_proof(
                crate::runtime::ability::DEFAULT_ABILITY_DESCRIPTOR_VERSION,
                [0x11; 32],
                [0x22; 32],
            ),
    )
    .await
    .unwrap();
    rt
}

fn test_envelope() -> Envelope {
    ProtoEnvelope::targeted(TEST_DAEMON_URI, TEST_DAEMON_URI, TEST_DAEMON_URI)
        .expect("valid test envelope")
        .into_inner()
}

fn test_device_signing_key() -> ed25519_dalek::SigningKey {
    ed25519_dalek::SigningKey::from_bytes(&TEST_DEVICE_SIGNING_SEED)
}

fn next_test_invocation_nonce() -> [u8; 16] {
    use std::sync::atomic::{AtomicU64, Ordering};

    static NONCE_COUNTER: AtomicU64 = AtomicU64::new(1);
    let n = NONCE_COUNTER.fetch_add(1, Ordering::Relaxed);
    let mut nonce = [0u8; 16];
    nonce[..8].copy_from_slice(&n.to_be_bytes());
    nonce[8..].copy_from_slice(&(!n).to_be_bytes());
    nonce
}

fn test_descriptor_ref(callee_ura: &str, ability: &str) -> String {
    crate::runtime::axon_bridge::descriptor_ref::ability_descriptor_ref_for_wire(
        callee_ura,
        ability,
        crate::runtime::ability::DEFAULT_ABILITY_DESCRIPTOR_VERSION,
    )
    .expect("test descriptor ref")
}

fn signed_test_envelope(
    caller_ura: &str,
    callee_ura: &str,
    subject_ura: &str,
    ability: &str,
    arguments: &[u8],
    signing_key: &ed25519_dalek::SigningKey,
) -> Envelope {
    use ed25519_dalek::Signer as _;

    let nonce = next_test_invocation_nonce();
    let mut envelope = ProtoEnvelope::targeted(caller_ura, callee_ura, subject_ura)
        .expect("valid signed test envelope")
        .into_inner();
    envelope.invocation_nonce = nonce.to_vec();
    let descriptor_ref = test_descriptor_ref(callee_ura, ability);
    let descriptor_bound =
        crate::runtime::axon_bridge::wire_descriptor::descriptor_bound_from_wire_parts(
            envelope.clone(),
            descriptor_ref,
            arguments,
            crate::runtime::axon_bridge::wire_descriptor::WireCallerIdentity::FromEnvelope,
        )
        .expect("descriptor-bound signed test envelope");
    let signature = signing_key.sign(&descriptor_bound.envelope.canonical_bytes());
    envelope.caller_signature = Some(CallerSignature {
        algorithm: "ed25519".to_string(),
        signature: signature.to_bytes().to_vec(),
        key_id_hint: String::new(),
    });
    envelope
}

fn invoke_request(function_name: &str, args_json: &str) -> Request<InvokeRequest> {
    let arguments = args_json.as_bytes().to_vec();
    let signing_key = test_device_signing_key();
    Request::new(InvokeRequest {
        envelope: Some(signed_test_envelope(
            TEST_DAEMON_URI,
            TEST_DAEMON_URI,
            TEST_DAEMON_URI,
            function_name,
            &arguments,
            &signing_key,
        )),
        function_name: function_name.to_string(),
        arguments,
        metadata: std::collections::HashMap::from([(
            crate::services::invocation_transport::invocation_wire::SIGNED_DESCRIPTOR_REF_METADATA_KEY
                .to_string(),
            test_descriptor_ref(TEST_DAEMON_URI, function_name),
        )]),
        ..InvokeRequest::default()
    })
}

fn invoke_request_from_device(
    caller_ura: &str,
    function_name: &str,
    arguments: Vec<u8>,
) -> Request<InvokeRequest> {
    let signing_key = test_device_signing_key();
    Request::new(InvokeRequest {
        envelope: Some(signed_test_envelope(
            caller_ura,
            TEST_DAEMON_URI,
            TEST_DAEMON_URI,
            function_name,
            &arguments,
            &signing_key,
        )),
        function_name: function_name.to_string(),
        arguments,
        metadata: std::collections::HashMap::from([(
            crate::services::invocation_transport::invocation_wire::SIGNED_DESCRIPTOR_REF_METADATA_KEY
                .to_string(),
            test_descriptor_ref(TEST_DAEMON_URI, function_name),
        )]),
        ..InvokeRequest::default()
    })
}

fn parse_response_body<T: serde::de::DeserializeOwned>(resp: Response<InvokeResponse>) -> T {
    let body = resp.into_inner();
    assert_eq!(body.result_content_type, FEDERATION_RESULT_CONTENT_TYPE);
    serde_json::from_slice(&body.result).expect("response body deserialises")
}

fn assert_route_negative_noroute(message: &str) {
    assert!(
        message.contains(ROUTE_NEGATIVE_CODE),
        "expected typed route negative code, got: {message}"
    );
    assert!(
        message.contains(easynet_axon::pb::axon::v1::NegativeReason::Noroute.as_str_name()),
        "expected NOROUTE negative reason, got: {message}"
    );
}

// Shared invoke_remote frame helpers used by stream and bidi tests.
// ── PR-3 commit 1/3 — runtime.invoke_remote helpers + early returns ────

use crate::services::invocation_transport::invoke_remote_initiator::{
    InvokeRemoteUp, ABILITY_INVOKE_REMOTE,
};
use easynet_axon::pb::axon::v1::invoke_bidi_up::Payload as UpPayload;
use easynet_axon::pb::axon::v1::{BidiControl, EnvelopeOpen, InvocationTarget, InvokeBidiUp};
fn make_envelope_open(ability: &str, initial_args: Vec<u8>) -> EnvelopeOpen {
    let signing_key = test_device_signing_key();
    EnvelopeOpen {
        envelope: Some(signed_test_envelope(
            TEST_DAEMON_URI,
            TEST_DAEMON_URI,
            TEST_DAEMON_URI,
            ability,
            &initial_args,
            &signing_key,
        )),
        target: Some(InvocationTarget {
            ability_name: ability.to_string(),
            ..InvocationTarget::default()
        }),
        initial_args,
        args_content_type: "application/json".to_string(),
        ..EnvelopeOpen::default()
    }
}

fn make_envelope_open_with_callee(callee_ura: &str) -> EnvelopeOpen {
    let ability = crate::runtime::agents::pty_attach_ability::ABILITY_PTY_SESSION_ATTACH;
    let signing_key = test_device_signing_key();
    EnvelopeOpen {
        envelope: Some(signed_test_envelope(
            TEST_DAEMON_URI,
            callee_ura,
            callee_ura,
            ability,
            &[],
            &signing_key,
        )),
        target: Some(InvocationTarget {
            ability_name: ability.to_string(),
            ..InvocationTarget::default()
        }),
        ..EnvelopeOpen::default()
    }
}

// Shared forward_invoke payload helpers used across unary, forward, and session-request tests.
fn forward_invoke_args(target_ura: &str) -> Vec<u8> {
    // Test fixture: a base64-encoded JSON `{ability, args,
    // call_id}` payload that mirrors what `support::
    // federation_invoke::invoke_via_federation_forward`
    // ships from the CLI bridge. PR-N1 commit 11/N decodes
    // this on the peer-dispatch path so the rebuilt
    // `peer_request` carries the real inner ability + args;
    // C1a / DEC-N4 §2.1 added the required `call_id` field
    // for response correlation.
    forward_invoke_args_for_ability(target_ura, "observe.health", serde_json::json!({}))
}

/// Parameterised sibling of `forward_invoke_args` for tests
/// that need to drive a specific inner ability + args
/// (e.g. PR-1 commit 7/9 self-target dispatch tests against
/// `fs.read`).
fn forward_invoke_args_for_ability(
    target_ura: &str,
    ability: &str,
    args: serde_json::Value,
) -> Vec<u8> {
    use base64::{engine::general_purpose::STANDARD, Engine as _};
    let public_ability = crate::ura::owner_local_ability_name(target_ura, ability);
    let ability_ura = crate::ura::owner_ability_ura(target_ura, &public_ability)
        .unwrap_or_else(|| panic!("derive test ability URA for {target_ura} {public_ability}"));
    let inner = serde_json::json!({
        "ability_ura": ability_ura,
        "args": args,
        "call_id": "test-call-id-1",
    });
    let inner_b64 = STANDARD.encode(serde_json::to_vec(&inner).unwrap());
    format!(r#"{{"target_ura":"{target_ura}","inner_envelope_b64":"{inner_b64}"}}"#).into_bytes()
}

fn forward_invoke_args_for_ability_ura(
    target_ura: &str,
    ability_ura: &str,
    args: serde_json::Value,
) -> Vec<u8> {
    use base64::{engine::general_purpose::STANDARD, Engine as _};
    let inner = serde_json::json!({
        "ability_ura": ability_ura,
        "args": args,
        "call_id": "test-call-id-1",
    });
    let inner_b64 = STANDARD.encode(serde_json::to_vec(&inner).unwrap());
    format!(r#"{{"target_ura":"{target_ura}","inner_envelope_b64":"{inner_b64}"}}"#).into_bytes()
}

/// Test fixture: a `FederationClient` that records every
/// `forward_invoke` call and returns a canned response. Lets
/// tests assert the cross-realm arm dialed the right peer
/// hub with the right ability + arguments.
struct RecordingFederationClient {
    recorded: std::sync::Mutex<Vec<(crate::services::federation_client::HubUri, InvokeRequest)>>,
    canned: InvokeResponse,
}

impl RecordingFederationClient {
    fn new(canned: InvokeResponse) -> Self {
        Self {
            recorded: std::sync::Mutex::new(Vec::new()),
            canned,
        }
    }

    fn calls(&self) -> Vec<(crate::services::federation_client::HubUri, InvokeRequest)> {
        self.recorded.lock().expect("mutex").clone()
    }
}

#[async_trait::async_trait]
impl FederationClient for RecordingFederationClient {
    async fn forward_invoke(
        &self,
        target_hub: &crate::services::federation_client::HubUri,
        request: InvokeRequest,
    ) -> Result<InvokeResponse, crate::services::federation_client::FederationClientError> {
        self.recorded
            .lock()
            .expect("mutex")
            .push((target_hub.clone(), request));
        Ok(self.canned.clone())
    }
}

#[path = "daemon_invocation_service_tests/bidi.rs"]
mod bidi;
#[path = "daemon_invocation_service_tests/forward.rs"]
mod forward;
#[path = "daemon_invocation_service_tests/local_rpc.rs"]
mod local_rpc;
#[path = "daemon_invocation_service_tests/session_request.rs"]
mod session_request;
#[path = "daemon_invocation_service_tests/stream.rs"]
mod stream;
#[path = "daemon_invocation_service_tests/unary.rs"]
mod unary;
