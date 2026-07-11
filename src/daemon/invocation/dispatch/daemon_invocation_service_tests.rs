// EasyNet Daemon — Invocation Service Behavior Tests
// ====================================================
//
// File: src/daemon/invocation/daemon_invocation_service_tests.rs
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
use crate::daemon::ability::HOSTED_AGENT_DELEGATION_METADATA_KEY;
use crate::daemon::identity::self_identity::{CanonicalSigner, TestCanonicalSigner};
use crate::daemon::invocation::admission::peer_envelope_signer::sign_peer_request_envelope;
use crate::daemon::invocation::admission::quota_meter::quota_meters_function;
use crate::daemon::invocation::admission::register_device_pubkey::parse_realm_from_ura;
use crate::daemon::invocation::admission::target_gate::ROUTE_NEGATIVE_CODE;
use crate::daemon::invocation::admission::{
    decision::{AccessAction, PrincipalKind, TokenClass},
    grant_matcher::{
        PermissionEffect, PermissionGrant, PermissionGrantLifetime, PermissionGrantState,
    },
};
use crate::daemon::invocation::bidi::bidi_dispatcher::{
    build_bidi_terminal_receipt, build_remote_bidi_open_dispatch_frame,
    build_remote_bidi_open_frame_for_contract, extract_envelope_open, map_local_bidi_ability_frame,
    map_local_bidi_handler_frame, map_local_bidi_up_payload,
    refresh_session_owner_projection_lease_at, remote_bidi_target_ura, validate_session_realm,
    LocalBidiDownStream, LocalBidiHandlerFrame, LocalBidiUpFrame, LocalBidiWireKind,
    REASON_BIDI_FIRST_FRAME_SEQUENCE, REASON_BIDI_NON_STRICT_ORDERING,
};
use crate::daemon::invocation::bidi::session_initiator::ABILITY_SESSION_OPEN;
use crate::daemon::invocation::bidi::session_wire::SessionDispatch;
use crate::daemon::invocation::bidi::state::pending_dispatch::DispatchResult;
use crate::daemon::invocation::dispatch::federation_wrappers;
use crate::daemon::invocation::dispatch::invocation_wire::FEDERATION_RESULT_CONTENT_TYPE;
use crate::daemon::invocation::dispatch::invocation_wire::{
    DELEGATION_METADATA_KEY, SESSION_AUTHORITY_METADATA_KEY,
};
use crate::daemon::invocation::receipts::ledger_projection::{
    invocation_resource_ura, ledger_authority_form_for_request,
};
use crate::daemon::invocation::ProtoEnvelope;
use crate::daemon::persistence::access_control::AccessControlStore;
use easynet_axon::invocation::{AbilityFrame, BidiInputFrame};
use easynet_axon::pb::axon::v1::Error;
use easynet_axon::pb::axon::v1::{
    invoke_bidi_down::Payload as DownPayload, BinaryChunk, ErrorStage, SecurityClass,
    StreamDescriptor,
};
use std::collections::HashMap;
use tokio::sync::mpsc;

use crate::daemon::trust::anchor::{RealmTrustAnchor, TrustedAgent, TrustedAgentRole};
use easynet_axon::pb::axon::v1::{AgentIdentity, CallerSignature, Envelope, SubjectIdentity};

/// Test helper daemon URA — admitted by the test admission
/// facade via the loopback bypass. Tests that exercise
/// admission rejection construct a different facade.
// URA v4.1.4: daemons are devices, not agents. Fixtures use the
// canonical shape because invoke no longer repairs legacy
// `agent/<bare-id>` device aliases at the request boundary.
const TEST_DAEMON_URA: &str = "easynet:///r/test-realm/device/test-daemon";
const TEST_DEVICE_SIGNING_SEED: [u8; 32] = [0x33; 32];

fn test_hub_signer(realm: &str) -> Arc<dyn CanonicalSigner> {
    test_hub_signer_with_seed(realm, [0x11; 32])
}

fn test_hub_signer_with_seed(realm: &str, seed: [u8; 32]) -> Arc<dyn CanonicalSigner> {
    Arc::new(TestCanonicalSigner::new(
        crate::core::ura::hub_ura(realm),
        seed,
    ))
}

fn make_service() -> DaemonInvocationService {
    let admission = AdmissionFacade::new(
        Arc::new(RealmTrustAnchor::default()),
        Some(TEST_DAEMON_URA.to_string()),
    );
    DaemonInvocationService::new(Arc::new(PresenceRegistry::new()), admission)
        .with_hub_signer(test_hub_signer("test-realm"))
}

fn publish_test_route(svc: &DaemonInvocationService, owner_ura: &str, public_name: &str) {
    publish_test_route_hosted_by(svc, owner_ura, public_name, TEST_DAEMON_URA);
}

fn publish_test_route_hosted_by(
    svc: &DaemonInvocationService,
    owner_ura: &str,
    public_name: &str,
    hosted_agent_host_ura: &str,
) {
    let public_name = crate::core::ura::owner_local_ability_name(owner_ura, public_name);
    let ability_ura = crate::core::ura::owner_ability_ura(owner_ura, &public_name)
        .unwrap_or_else(|| panic!("derive test ability URA for {owner_ura} {public_name}"));
    let host_ura = match crate::core::ura::parse_ura(owner_ura).map(|parsed| parsed.kind) {
        Ok(crate::core::ura::URAKind::Agent) => {
            svc.directory.advertised_agents.upsert(
                crate::daemon::federation::read_model::advertised_agents::AdvertisedAgentRecord {
                    agent_ura: owner_ura.to_string(),
                    public_key_hex: String::new(),
                    host_node_id: Some(hosted_agent_host_ura.to_string()),
                    signing_authority:
                        crate::daemon::federation::read_model::advertised_agents::AdvertisedAgentSigningAuthority::HostedBy {
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
        crate::daemon::federation::read_model::ability_catalog::OwnerAbilityProjectionRow::new(
            owner_ura.to_string(),
            host_ura,
            1,
            "sha256:test".to_string(),
            4_102_444_800_000,
            vec![crate::daemon::federation::read_model::owner_projection::AbilityProjectionSummary {
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
                callable_summary: crate::daemon::federation::read_model::owner_projection::AbilityCallableSummary::minimal(
                    public_name.to_string(),
                ),
            }],
        ),
    );
}

async fn signed_delegation_metadata_for_test(
    signer: &dyn CanonicalSigner,
    issuer_ura: &str,
    subject_ura: &str,
    caller_ura: &str,
    audience: &str,
    scopes: &[&str],
) -> String {
    use base64::{engine::general_purpose::STANDARD as BASE64_STANDARD, Engine as _};
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
    let payload_bytes = crate::daemon::ability::canonical_json_bytes(&payload_value);
    let signature = signer
        .sign_canonical(&payload_bytes)
        .await
        .expect("test canonical signer");
    let raw = serde_json::json!({
        "payload": payload_value,
        "signature": BASE64_STANDARD.encode(signature.to_bytes()),
    });
    BASE64_STANDARD.encode(serde_json::to_vec(&raw).expect("delegation proof"))
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
    let runtime_ability = crate::core::ura::owner_ability_ura(
        owner_ura,
        &crate::core::ura::owner_local_ability_name(owner_ura, ability),
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
                crate::daemon::ability::DEFAULT_ABILITY_DESCRIPTOR_VERSION,
                [0x11; 32],
                [0x22; 32],
            ),
    )
    .await
    .unwrap();
    rt
}

fn test_envelope() -> Envelope {
    ProtoEnvelope::targeted(TEST_DAEMON_URA, TEST_DAEMON_URA, TEST_DAEMON_URA)
        .expect("valid test envelope")
        .into_inner()
}

#[test]
fn route_table_match_projects_descriptor_ref_to_public_name() {
    let ability =
        crate::daemon::invocation::dispatch::federation_wrappers::ABILITY_FEDERATION_RESOLVE;
    let descriptor_ref = test_descriptor_ref(TEST_DAEMON_URA, ability);
    let envelope = test_envelope();

    assert_eq!(
        dispatch_function_name_for_route_table(&descriptor_ref, Some(&envelope)),
        ability
    );
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
    crate::daemon::axon_bridge::descriptor_ref::ability_descriptor_ref_for_wire(
        callee_ura,
        ability,
        crate::daemon::ability::DEFAULT_ABILITY_DESCRIPTOR_VERSION,
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
        crate::daemon::axon_bridge::wire_descriptor::descriptor_bound_from_wire_parts(
            envelope.clone(),
            descriptor_ref,
            arguments,
            crate::daemon::axon_bridge::wire_descriptor::WireCallerIdentity::FromEnvelope,
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
    invoke_request_for_callee(TEST_DAEMON_URA, function_name, args_json)
}

fn invoke_request_for_callee(
    callee_ura: &str,
    function_name: &str,
    args_json: &str,
) -> Request<InvokeRequest> {
    let arguments = args_json.as_bytes().to_vec();
    let signing_key = test_device_signing_key();
    let descriptor_ref = test_descriptor_ref(callee_ura, function_name);
    Request::new(InvokeRequest {
        envelope: Some(signed_test_envelope(
            TEST_DAEMON_URA,
            callee_ura,
            TEST_DAEMON_URA,
            function_name,
            &arguments,
            &signing_key,
        )),
        function_name: descriptor_ref.clone(),
        arguments,
        metadata: std::collections::HashMap::from([(
            crate::daemon::invocation::dispatch::invocation_wire::SIGNED_DESCRIPTOR_REF_METADATA_KEY
                .to_string(),
            descriptor_ref,
        )]),
        ..InvokeRequest::default()
    })
}

fn parse_response_body<T: serde::de::DeserializeOwned>(resp: Response<InvokeResponse>) -> T {
    let body = resp.into_inner();
    assert_eq!(body.result_content_type, FEDERATION_RESULT_CONTENT_TYPE);
    serde_json::from_slice(&body.result).expect("response body deserialises")
}

// Shared invoke_remote frame helpers used by stream and bidi tests.
// Canonical session dispatch helpers.

use easynet_axon::pb::axon::v1::invoke_bidi_up::Payload as UpPayload;
use easynet_axon::pb::axon::v1::{BidiControl, EnvelopeOpen, InvocationTarget, InvokeBidiUp};
fn make_envelope_open(ability: &str, initial_args: Vec<u8>) -> EnvelopeOpen {
    let signing_key = test_device_signing_key();
    EnvelopeOpen {
        envelope: Some(signed_test_envelope(
            TEST_DAEMON_URA,
            TEST_DAEMON_URA,
            TEST_DAEMON_URA,
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
    let ability = crate::daemon::ability::builtins::device_control::terminal::attach::ABILITY_PTY_SESSION_ATTACH;
    let signing_key = test_device_signing_key();
    EnvelopeOpen {
        envelope: Some(signed_test_envelope(
            TEST_DAEMON_URA,
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

fn test_owner_ability_ura(target_ura: &str, ability: &str) -> String {
    let public_ability = crate::core::ura::owner_local_ability_name(target_ura, ability);
    crate::core::ura::owner_ability_ura(target_ura, &public_ability)
        .unwrap_or_else(|| panic!("derive test ability URA for {target_ura} {public_ability}"))
}

fn grant_child_access_for_test(
    owner_user_id: &str,
    principal_kind: PrincipalKind,
    principal_ura: &str,
    token_class: Option<TokenClass>,
    callee_ura: &str,
    subject_ura: &str,
    ability_ura: &str,
    action: AccessAction,
) {
    use std::sync::atomic::{AtomicU64, Ordering};

    static GRANT_COUNTER: AtomicU64 = AtomicU64::new(1);
    let n = GRANT_COUNTER.fetch_add(1, Ordering::Relaxed);
    let token_id = token_class.map(|_| principal_ura.to_string());
    let mut store =
        AccessControlStore::open_or_create(owner_user_id).expect("open test access-control store");
    store
        .create_grant(
            PermissionGrant {
                grant_id: format!("test-grant-{n}"),
                owner_user_id: owner_user_id.to_string(),
                principal_kind,
                principal_id: principal_ura.to_string(),
                token_id,
                token_class,
                callee_ura: Some(callee_ura.to_string()),
                subject_ura_pattern: Some(subject_ura.to_string()),
                ability_ura_pattern: Some(ability_ura.to_string()),
                actions: vec![action],
                constraints: None,
                effect: PermissionEffect::Allow,
                lifetime: PermissionGrantLifetime::Session,
                state: PermissionGrantState::Active,
                expires_at: None,
                review_required_after: None,
                last_reviewed_at: None,
                last_used_at: None,
                created_by: crate::core::ura::user_ura("test-realm", owner_user_id),
                created_at: "2026-07-09T00:00:00Z".to_string(),
                updated_at: None,
                revoked_at: None,
                reason: Some("forward-invoke test fixture".to_string()),
            },
            &crate::core::ura::user_ura("test-realm", owner_user_id),
        )
        .expect("create test child access grant");
}

/// Test fixture: a `FederationClient` that records every
/// `invoke` call and returns a canned response. Lets
/// tests assert the cross-realm arm dialed the right peer
/// hub with the right ability + arguments.
struct RecordingFederationClient {
    recorded: std::sync::Mutex<
        Vec<(
            crate::daemon::federation::client::HubEndpoint,
            InvokeRequest,
        )>,
    >,
    canned: InvokeResponse,
}

impl RecordingFederationClient {
    fn new(canned: InvokeResponse) -> Self {
        Self {
            recorded: std::sync::Mutex::new(Vec::new()),
            canned,
        }
    }

    fn calls(
        &self,
    ) -> Vec<(
        crate::daemon::federation::client::HubEndpoint,
        InvokeRequest,
    )> {
        self.recorded.lock().expect("mutex").clone()
    }
}

#[async_trait::async_trait]
impl FederationClient for RecordingFederationClient {
    async fn invoke(
        &self,
        target_hub_endpoint: &crate::daemon::federation::client::HubEndpoint,
        request: InvokeRequest,
    ) -> Result<InvokeResponse, crate::daemon::federation::client::FederationClientError> {
        self.recorded
            .lock()
            .expect("mutex")
            .push((target_hub_endpoint.clone(), request));
        Ok(self.canned.clone())
    }
}

#[path = "daemon_invocation_service_tests/bidi.rs"]
mod bidi;
#[path = "daemon_invocation_service_tests/forward.rs"]
mod canonical_relay;
#[path = "daemon_invocation_service_tests/local_rpc.rs"]
mod local_rpc;
#[path = "daemon_invocation_service_tests/stream.rs"]
mod stream;
#[path = "daemon_invocation_service_tests/unary.rs"]
mod unary;
