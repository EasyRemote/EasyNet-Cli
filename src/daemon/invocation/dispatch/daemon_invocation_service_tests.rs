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
    build_remote_bidi_open_frame, extract_envelope_open, map_local_bidi_ability_frame,
    map_local_bidi_handler_frame, map_local_bidi_up_payload,
    refresh_session_owner_projection_lease_at, remote_bidi_target_ura, validate_session_realm,
    LocalBidiDownStream, LocalBidiHandlerFrame, LocalBidiUpFrame, LocalBidiWireKind,
    REASON_BIDI_FIRST_FRAME_SEQUENCE, REASON_BIDI_NON_STRICT_ORDERING,
};
use crate::daemon::invocation::bidi::session_initiator::ABILITY_SESSION_OPEN;
use crate::daemon::invocation::bidi::state::pending_dispatch::DispatchResult;
use crate::daemon::invocation::dispatch::federation_wrappers;
use crate::daemon::invocation::dispatch::invocation_wire::FEDERATION_RESULT_CONTENT_TYPE;
use crate::daemon::invocation::ProtoEnvelope;
use crate::daemon::persistence::access_control::AccessControlStore;
use easynet_axon::invocation::{AbilityFrame, BidiInputFrame, CallMode};
use easynet_axon::pb::axon::v1::{
    invoke_bidi_down::Payload as DownPayload, BinaryChunk, StreamDescriptor,
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
const TEST_BOOTSTRAP_CALLER_URA: &str = "easynet:///r/test-realm/device/bootstrap-caller";
const TEST_DEVICE_SIGNING_SEED: [u8; 32] = [0x33; 32];
const TEST_BOOTSTRAP_CALLER_SIGNING_SEED: [u8; 32] = [0x44; 32];

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
    make_service_with_daemon_route_owner(TEST_DAEMON_URA)
}

fn make_service_with_daemon_route_owner(route_owner_ura: &str) -> DaemonInvocationService {
    let service = make_unregistered_service_for_route_owner(route_owner_ura);
    register_test_daemon_routes(service, route_owner_ura)
}

fn make_service_with_runtime(
    route_owner_ura: &str,
    runtime: Arc<easynet_axon::invocation::LocalRuntime>,
) -> DaemonInvocationService {
    let service = make_unregistered_service_for_route_owner_and_runtime(route_owner_ura, runtime);
    register_test_daemon_routes(service, route_owner_ura)
}

fn make_service_with_test_runtime(
    runtime: Arc<easynet_axon::invocation::LocalRuntime>,
) -> DaemonInvocationService {
    make_service_with_runtime(TEST_DAEMON_URA, runtime)
}

fn make_service_with_presence(presence: Arc<PresenceRegistry>) -> DaemonInvocationService {
    make_service_with_presence_and_heartbeat(presence, None)
}

fn make_service_with_presence_and_heartbeat(
    presence: Arc<PresenceRegistry>,
    heartbeat_interval_ms: Option<std::num::NonZeroU64>,
) -> DaemonInvocationService {
    let anchor = test_trust_anchor();
    let cell = SharedTrustAnchor::new(Arc::new(anchor));
    let runtime = test_local_runtime(cell.clone());
    let local_ability_catalog = test_catalog_for_route_owner(TEST_DAEMON_URA, Arc::clone(&runtime));
    let admission = AdmissionFacade::with_trust_anchor_cell(
        cell,
        test_admission_daemon_ura_for_route_owner(TEST_DAEMON_URA),
    )
    .with_ability_catalog(Arc::clone(&local_ability_catalog));
    let mut service = DaemonInvocationService::new(presence, admission)
        .with_hub_signer(test_hub_signer("test-realm"))
        .with_local_ability_catalog(local_ability_catalog)
        .with_local_runtime(runtime);
    if let Some(interval) = heartbeat_interval_ms {
        service = service.with_subscribe_v2_heartbeat_interval_ms(interval);
    }
    register_test_daemon_routes(service, TEST_DAEMON_URA)
}

fn make_service_with_runtime_trust_route_owner(
    route_owner_ura: &str,
    daemon_realm: impl Into<String>,
    trust_anchor_path: impl Into<std::path::PathBuf>,
    cell: SharedTrustAnchor,
) -> DaemonInvocationService {
    let service = make_unregistered_service_for_route_owner(route_owner_ura).with_register_pubkey(
        daemon_realm,
        trust_anchor_path,
        cell,
    );
    register_test_daemon_routes(service, route_owner_ura)
}

fn register_test_daemon_routes(
    mut service: DaemonInvocationService,
    route_owner_ura: &str,
) -> DaemonInvocationService {
    let runtime = service
        .runtime
        .local_runtime
        .as_ref()
        .cloned()
        .expect("test service must have a LocalRuntime before route registration");
    let catalog = test_catalog_for_route_owner(route_owner_ura, runtime);
    service.admission = service
        .admission
        .clone()
        .with_ability_catalog(Arc::clone(&catalog));
    service = service.with_local_ability_catalog(catalog);
    futures::executor::block_on(service.register_daemon_unary_routes(route_owner_ura))
        .expect("explicitly assemble daemon exact routes for test service");
    futures::executor::block_on(service.register_daemon_stream_routes(route_owner_ura))
        .expect("explicitly assemble daemon exact stream routes for test service");
    service
}

fn make_unregistered_service_for_route_owner(route_owner_ura: &str) -> DaemonInvocationService {
    let anchor = test_trust_anchor();
    let cell = SharedTrustAnchor::new(Arc::new(anchor));
    let runtime = test_local_runtime(cell.clone());
    make_unregistered_service_for_route_owner_and_runtime(route_owner_ura, runtime)
}

fn make_unregistered_service_for_route_owner_and_runtime(
    route_owner_ura: &str,
    runtime: Arc<easynet_axon::invocation::LocalRuntime>,
) -> DaemonInvocationService {
    let anchor = test_trust_anchor();
    let cell = SharedTrustAnchor::new(Arc::new(anchor));
    let local_ability_catalog = test_catalog_for_route_owner(route_owner_ura, Arc::clone(&runtime));
    let admission_daemon_ura = test_admission_daemon_ura_for_route_owner(route_owner_ura);
    let admission = AdmissionFacade::with_trust_anchor_cell(cell.clone(), admission_daemon_ura)
        .with_ability_catalog(Arc::clone(&local_ability_catalog));
    let signer_realm = crate::core::ura::parse_ura(route_owner_ura)
        .map(|parsed| parsed.realm)
        .unwrap_or_else(|_| "test-realm".to_string());
    DaemonInvocationService::new(Arc::new(PresenceRegistry::new()), admission)
        .with_hub_signer(test_hub_signer(&signer_realm))
        .with_local_ability_catalog(local_ability_catalog)
        .with_local_runtime(runtime)
}

fn test_trust_anchor() -> RealmTrustAnchor {
    use base64::{engine::general_purpose::STANDARD as BASE64_STANDARD, Engine as _};

    RealmTrustAnchor::from_entries(vec![
        TrustedAgent {
            agent_ura: TEST_DAEMON_URA.to_string(),
            public_key_b64: BASE64_STANDARD
                .encode(test_device_signing_key().verifying_key().to_bytes()),
            role: TrustedAgentRole::Device,
            added_at_unix_ms: 1_700_000_000_000,
            origin_realm: None,
            hub_endpoint: None,
            tls_ca_pem_path: None,
        },
        TrustedAgent {
            agent_ura: "easynet:///r/test-realm/device/client-1".to_string(),
            public_key_b64: BASE64_STANDARD
                .encode(test_device_signing_key().verifying_key().to_bytes()),
            role: TrustedAgentRole::Device,
            added_at_unix_ms: 1_700_000_000_000,
            origin_realm: None,
            hub_endpoint: None,
            tls_ca_pem_path: None,
        },
        TrustedAgent {
            agent_ura: TEST_BOOTSTRAP_CALLER_URA.to_string(),
            public_key_b64: BASE64_STANDARD.encode(
                test_bootstrap_caller_signing_key()
                    .verifying_key()
                    .to_bytes(),
            ),
            role: TrustedAgentRole::Device,
            added_at_unix_ms: 1_700_000_000_000,
            origin_realm: None,
            hub_endpoint: None,
            tls_ca_pem_path: None,
        },
    ])
    .expect("test daemon trust anchor")
}

fn test_admission_daemon_ura_for_route_owner(route_owner_ura: &str) -> Option<String> {
    let parsed = crate::core::ura::parse_ura(route_owner_ura).ok()?;
    match parsed.kind {
        crate::core::ura::URAKind::Hub => Some(route_owner_ura.to_string()),
        _ => Some(TEST_DAEMON_URA.to_string()),
    }
}

fn test_catalog_for_route_owner(
    route_owner_ura: &str,
    runtime: Arc<easynet_axon::invocation::LocalRuntime>,
) -> Arc<crate::daemon::ability::dispatch::AxonAbilityCatalog> {
    let authority_context = test_authority_context_for_route_owner(route_owner_ura);
    let agents = crate::daemon::persistence::agent_registry::AgentRegistry::default();
    let mut catalog_config =
        crate::daemon::ability::catalog::RegistryBuildConfig::new_with_authority_context(
            crate::daemon::ability::catalog::RegistryBuildServices::fresh(),
            &agents,
            authority_context,
        );
    catalog_config.local_runtime = Some(runtime);
    crate::daemon::ability::catalog::build_registry_with_services_result(catalog_config)
        .expect("assemble production-shaped test ability catalog")
        .catalog
}

fn test_authority_context_for_route_owner(
    route_owner_ura: &str,
) -> crate::daemon::ability::dispatch::AbilityAuthorityContext {
    let parsed = crate::core::ura::parse_ura(route_owner_ura)
        .unwrap_or_else(|err| panic!("route owner URA must parse: {route_owner_ura}: {err}"));
    match parsed.kind {
        crate::core::ura::URAKind::Hub => {
            crate::daemon::ability::dispatch::AbilityAuthorityContext::for_hub_authority_root(
                route_owner_ura,
            )
            .expect("test Hub authority context")
        }
        crate::core::ura::URAKind::Device => {
            crate::daemon::ability::dispatch::AbilityAuthorityContext::for_combined_authority_roots(
                route_owner_ura,
            )
            .expect("test Device+Hub authority context")
        }
        _ => panic!("daemon route owner must be a Hub or Device URA: {route_owner_ura}"),
    }
}

fn test_local_runtime(cell: SharedTrustAnchor) -> Arc<easynet_axon::invocation::LocalRuntime> {
    crate::daemon::axon_bridge::runtime_factory::build_local_runtime(
        Some(Arc::new(TestRuntimeKeyResolver::new(cell))),
        None,
    )
}

fn test_runtime_with_default_trust() -> Arc<easynet_axon::invocation::LocalRuntime> {
    test_local_runtime(SharedTrustAnchor::new(Arc::new(test_trust_anchor())))
}

struct TestRuntimeKeyResolver {
    trust: crate::daemon::trust::key_resolver::RealmTrustAnchorKeyResolver,
}

impl TestRuntimeKeyResolver {
    fn new(cell: SharedTrustAnchor) -> Self {
        Self {
            trust: crate::daemon::trust::key_resolver::RealmTrustAnchorKeyResolver::new(cell),
        }
    }
}

impl easynet_axon::invocation::KeyResolver for TestRuntimeKeyResolver {
    fn resolve(
        &self,
        agent_ura: &str,
    ) -> Result<ed25519_dalek::VerifyingKey, easynet_axon::invocation::AxonError> {
        if agent_ura == TEST_DAEMON_URA {
            return Ok(test_device_signing_key().verifying_key());
        }
        easynet_axon::invocation::KeyResolver::resolve(&self.trust, agent_ura)
    }

    fn resolve_all(
        &self,
        agent_ura: &str,
    ) -> Result<Vec<ed25519_dalek::VerifyingKey>, easynet_axon::invocation::AxonError> {
        if agent_ura == TEST_DAEMON_URA {
            return Ok(vec![test_device_signing_key().verifying_key()]);
        }
        easynet_axon::invocation::KeyResolver::resolve_all(&self.trust, agent_ura)
    }
}

fn publish_test_route(svc: &DaemonInvocationService, owner_ura: &str, public_name: &str) {
    publish_test_route_with_mode(
        svc,
        owner_ura,
        public_name,
        crate::daemon::ability::CallMode::Rpc,
    );
}

fn publish_test_stream_route(svc: &DaemonInvocationService, owner_ura: &str, public_name: &str) {
    publish_test_route_with_mode(
        svc,
        owner_ura,
        public_name,
        crate::daemon::ability::CallMode::Stream,
    );
}

fn publish_test_route_with_mode(
    svc: &DaemonInvocationService,
    owner_ura: &str,
    public_name: &str,
    call_mode: crate::daemon::ability::CallMode,
) {
    register_test_catalog_route(svc, owner_ura, public_name, call_mode);
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
                    generation: 1,
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

fn register_test_catalog_route(
    svc: &DaemonInvocationService,
    owner_ura: &str,
    public_name: &str,
    call_mode: crate::daemon::ability::CallMode,
) {
    let Some(catalog) = svc.directory.local_ability_catalog.as_ref() else {
        return;
    };
    let Some(owner) = local_test_owner_kind(svc, owner_ura) else {
        return;
    };
    if catalog
        .authority_ability_catalog_snapshot()
        .into_iter()
        .any(|row| {
            row.descriptor.owner_ura == owner_ura
                && row.name == public_name
                && row.descriptor.call_mode() == call_mode
        })
    {
        return;
    }
    let manifest = test_route_manifest(public_name);
    catalog
        .register_control_plane_descriptor_with_owner(
            public_name,
            &owner,
            &manifest,
            call_mode,
            crate::daemon::ability::descriptors::ReceiptSemantics::Operational,
            &crate::daemon::ability::dispatch::ControlPlaneImplementation::native_daemon(),
        )
        .unwrap_or_else(|error| {
            panic!("register test catalog route {owner_ura}#{public_name}: {error}")
        });
}

fn local_test_owner_kind(
    svc: &DaemonInvocationService,
    owner_ura: &str,
) -> Option<crate::daemon::ability::dispatch::OwnerKind> {
    let parsed = crate::core::ura::parse_ura(owner_ura).ok()?;
    match parsed.kind {
        crate::core::ura::URAKind::Device => svc
            .admission
            .daemon_ura()
            .is_some_and(|daemon_ura| daemon_ura == owner_ura)
            .then_some(crate::daemon::ability::dispatch::OwnerKind::Device),
        crate::core::ura::URAKind::Hub => svc
            .identity
            .session_realm
            .as_deref()
            .is_some_and(|realm| crate::core::ura::hub_ura(realm) == owner_ura)
            .then_some(crate::daemon::ability::dispatch::OwnerKind::Hub),
        crate::core::ura::URAKind::Agent => {
            let (_, agent_id) = parsed.agent_ids()?;
            Some(crate::daemon::ability::dispatch::OwnerKind::Agent(
                agent_id.to_string(),
            ))
            .filter(|_| {
                svc.directory
                    .advertised_agents
                    .get(owner_ura)
                    .and_then(|record| record.host_ura().map(str::to_string))
                    .as_deref()
                    == svc.admission.daemon_ura()
            })
        }
        _ => None,
    }
}

fn test_route_manifest(public_name: &str) -> crate::daemon::ability::manifest::AbilityManifest {
    crate::daemon::ability::manifest::AbilityManifest::new(
        public_name.rsplit('.').next().unwrap_or(public_name),
        "Test route descriptor",
        serde_json::json!({"type": "object"}),
    )
    .and_then(|manifest| manifest.with_admission_action("invoke"))
    .expect("test route manifest")
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
    catalog_with_json_echo(owner_ura, ability, marker_key, marker_value)
        .runtime()
        .expect("catalog-backed echo runtime")
}

fn catalog_with_json_echo(
    owner_ura: &str,
    ability: &'static str,
    marker_key: &'static str,
    marker_value: &'static str,
) -> Arc<crate::daemon::ability::dispatch::AxonAbilityCatalog> {
    catalog_with_json_echo_on_runtime(
        owner_ura,
        ability,
        marker_key,
        marker_value,
        test_runtime_with_default_trust(),
    )
}

fn catalog_with_json_echo_on_runtime(
    owner_ura: &str,
    ability: &'static str,
    marker_key: &'static str,
    marker_value: &'static str,
    runtime: Arc<easynet_axon::invocation::LocalRuntime>,
) -> Arc<crate::daemon::ability::dispatch::AxonAbilityCatalog> {
    use crate::daemon::ability::dispatch::{
        AbilityAuthorityContext, AxonAbilityCatalog, LocalRpcHandler, OwnerKind,
    };
    use crate::daemon::ability::{descriptors::AdmissionAction, manifest::AbilityManifest};

    let mut catalog = AxonAbilityCatalog::new_with_runtime_and_authority_context(
        runtime,
        AbilityAuthorityContext::for_device_authority_root(owner_ura)
            .expect("echo fixture owner URA is a device authority root"),
    );
    let handler: LocalRpcHandler = Arc::new(move |args| {
        Ok(serde_json::json!({
            marker_key: marker_value,
            "echoed_args": args,
        }))
    });
    let manifest = AbilityManifest::new(
        ability.rsplit('.').next().unwrap_or(ability),
        "JSON echo fixture",
        serde_json::json!({"type": "object"}),
    )
    .and_then(|manifest| manifest.with_admission_action(AdmissionAction::Invoke.as_str()))
    .expect("echo fixture manifest is well-formed");
    catalog.register_rpc_with_spec_and_action(
        ability,
        OwnerKind::Device,
        AdmissionAction::Invoke,
        manifest,
        handler,
    );
    Arc::new(catalog)
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

fn test_bootstrap_caller_signing_key() -> ed25519_dalek::SigningKey {
    ed25519_dalek::SigningKey::from_bytes(&TEST_BOOTSTRAP_CALLER_SIGNING_SEED)
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
    if let Ok(descriptor_ref) =
        crate::daemon::axon_bridge::descriptor_ref::catalog_descriptor_ref_for_wire(
            callee_ura,
            ability,
            crate::daemon::ability::CallMode::Rpc,
        )
    {
        return descriptor_ref;
    }
    let descriptor_binding =
        crate::daemon::axon_bridge::descriptor_ref::descriptor_binding_for_wire(
            crate::daemon::ability::DEFAULT_ABILITY_DESCRIPTOR_VERSION,
            [0x33; 32],
            "invoke",
        )
        .expect("test descriptor binding");
    crate::daemon::axon_bridge::descriptor_ref::ability_descriptor_ref_for_wire(
        callee_ura,
        ability,
        &descriptor_binding,
    )
    .expect("test descriptor ref")
}

fn catalog_test_descriptor_ref(
    catalog: &crate::daemon::ability::dispatch::AxonAbilityCatalog,
    callee_ura: &str,
    ability: &str,
    call_mode: crate::daemon::ability::CallMode,
) -> String {
    let row = catalog
        .authority_ability_catalog_snapshot()
        .into_iter()
        .find(|row| row.name == ability && row.descriptor.call_mode() == call_mode)
        .unwrap_or_else(|| panic!("catalog descriptor row for {ability} in {call_mode:?}"));
    let descriptor_binding =
        crate::daemon::axon_bridge::descriptor_ref::descriptor_binding_for_wire(
            &row.descriptor.version,
            row.descriptor.descriptor_hash_bytes(),
            row.descriptor.admission_action().as_str(),
        )
        .expect("catalog descriptor binding for test");
    crate::daemon::axon_bridge::descriptor_ref::ability_descriptor_ref_for_wire(
        callee_ura,
        ability,
        &descriptor_binding,
    )
    .expect("catalog descriptor ref for test")
}

fn bind_invoke_request_to_descriptor_ref(
    request: &mut InvokeRequest,
    caller_ura: &str,
    callee_ura: &str,
    subject_ura: &str,
    descriptor_ref: String,
    signing_key: &ed25519_dalek::SigningKey,
) {
    request.envelope = Some(signed_test_envelope_with_descriptor_ref(
        caller_ura,
        callee_ura,
        subject_ura,
        descriptor_ref.clone(),
        &request.arguments,
        signing_key,
    ));
    request.function_name = descriptor_ref.clone();
    request.metadata.insert(
        crate::daemon::invocation::dispatch::invocation_wire::SIGNED_DESCRIPTOR_REF_METADATA_KEY
            .to_string(),
        descriptor_ref,
    );
}

async fn sync_runtime_proof_from_catalog(
    svc: &DaemonInvocationService,
    owner_ura: &str,
    ability: &str,
    call_mode: crate::daemon::ability::CallMode,
) {
    use easynet_axon::invocation::{AbilityCallModes, AbilityOptions, CallMode as AxonCallMode};

    let catalog = svc
        .directory
        .local_ability_catalog
        .as_ref()
        .expect("test service has local ability catalog");
    let record = catalog
        .control_plane_record_for_authority_mode(owner_ura, ability, call_mode)
        .expect("catalog proof lookup is unambiguous")
        .unwrap_or_else(|| panic!("catalog proof row exists for {owner_ura}#{ability}"));
    let descriptor = record.descriptor();
    let implementation = record.implementation();
    let options = match call_mode {
        crate::daemon::ability::CallMode::Rpc => AbilityOptions::default()
            .with_modes(AbilityCallModes::RPC)
            .with_descriptor_proof(
                descriptor.version.as_str(),
                descriptor.admission_action().as_str(),
                descriptor.descriptor_hash_bytes(),
                descriptor.schema_hash_bytes(),
                implementation.impl_hash(),
            ),
        crate::daemon::ability::CallMode::Stream => AbilityOptions::streaming()
            .with_mode_descriptor_proof(
                AxonCallMode::Stream,
                descriptor.version.as_str(),
                descriptor.admission_action().as_str(),
                descriptor.descriptor_hash_bytes(),
                descriptor.schema_hash_bytes(),
                implementation.impl_hash(),
            ),
        crate::daemon::ability::CallMode::Bidi => AbilityOptions::bidi()
            .with_mode_descriptor_proof(
                AxonCallMode::Bidi,
                descriptor.version.as_str(),
                descriptor.admission_action().as_str(),
                descriptor.descriptor_hash_bytes(),
                descriptor.schema_hash_bytes(),
                implementation.impl_hash(),
            ),
    };
    let runtime_ability =
        crate::core::ura::owner_ability_ura(owner_ura, ability).expect("test runtime ability URA");
    let runtime = svc
        .runtime
        .local_runtime
        .as_ref()
        .expect("test service has local runtime");
    runtime
        .update_ability_options(&runtime_ability, options)
        .await
        .expect("runtime option update succeeds")
        .unwrap_or_else(|| panic!("runtime row exists for {runtime_ability}"));
}

async fn signed_federation_join_request(
    realm: &str,
    membership_ura: &str,
    join_args: Vec<u8>,
    test_seed: [u8; 32],
) -> InvokeRequest {
    let signing_key = ed25519_dalek::SigningKey::from_bytes(&test_seed);
    let public_key = signing_key.verifying_key().to_bytes();
    let provisional = crate::core::ura::provisional::provisional_ura_for_pubkey(&public_key);
    let hub_ura = crate::core::ura::hub_ura(realm);
    let descriptor_ref = test_descriptor_ref(&hub_ura, ABILITY_FEDERATION_JOIN);
    let signer = TestCanonicalSigner::new(provisional.clone(), test_seed);
    crate::daemon::invocation::ProtoEnvelope::federation_join_genesis(
        provisional,
        hub_ura,
        membership_ura,
    )
    .expect("genesis envelope")
    .signed_descriptor_ref_invoke_request_with_signer(
        ABILITY_FEDERATION_JOIN,
        descriptor_ref,
        join_args,
        &signer,
    )
    .await
    .expect("signed federation.join request")
}

fn signed_test_envelope(
    caller_ura: &str,
    callee_ura: &str,
    subject_ura: &str,
    ability: &str,
    arguments: &[u8],
    signing_key: &ed25519_dalek::SigningKey,
) -> Envelope {
    let descriptor_ref = test_descriptor_ref(callee_ura, ability);
    signed_test_envelope_with_descriptor_ref(
        caller_ura,
        callee_ura,
        subject_ura,
        descriptor_ref,
        arguments,
        signing_key,
    )
}

fn signed_test_envelope_with_descriptor_ref(
    caller_ura: &str,
    callee_ura: &str,
    subject_ura: &str,
    descriptor_ref: String,
    arguments: &[u8],
    signing_key: &ed25519_dalek::SigningKey,
) -> Envelope {
    use ed25519_dalek::Signer as _;

    let nonce = next_test_invocation_nonce();
    let mut envelope = ProtoEnvelope::targeted(caller_ura, callee_ura, subject_ura)
        .expect("valid signed test envelope")
        .into_inner();
    envelope.invocation_nonce = nonce.to_vec();
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
    signed_invoke_request(
        TEST_DAEMON_URA,
        callee_ura,
        TEST_DAEMON_URA,
        function_name,
        args_json,
        &test_device_signing_key(),
    )
}

fn signed_invoke_request(
    caller_ura: &str,
    callee_ura: &str,
    subject_ura: &str,
    function_name: &str,
    args_json: &str,
    signing_key: &ed25519_dalek::SigningKey,
) -> Request<InvokeRequest> {
    let arguments = args_json.as_bytes().to_vec();
    let descriptor_ref = test_descriptor_ref(callee_ura, function_name);
    Request::new(InvokeRequest {
        envelope: Some(signed_test_envelope(
            caller_ura,
            callee_ura,
            subject_ura,
            function_name,
            &arguments,
            signing_key,
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
        streams: vec![StreamDescriptor {
            stream_id: 1,
            content_type: "application/json".to_string(),
            ordering: "STRICT".to_string(),
            ..StreamDescriptor::default()
        }],
        initial_args,
        args_content_type: "application/json".to_string(),
        ..EnvelopeOpen::default()
    }
}

fn make_descriptor_ref_envelope_open(descriptor_ref: &str, initial_args: Vec<u8>) -> EnvelopeOpen {
    let signing_key = test_device_signing_key();
    EnvelopeOpen {
        envelope: Some(signed_test_envelope_with_descriptor_ref(
            TEST_DAEMON_URA,
            TEST_DAEMON_URA,
            TEST_DAEMON_URA,
            descriptor_ref.to_string(),
            &initial_args,
            &signing_key,
        )),
        target: Some(InvocationTarget {
            ability_name: descriptor_ref.to_string(),
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

struct ChildAccessGrantInput<'a> {
    owner_user_id: &'a str,
    principal_kind: PrincipalKind,
    principal_ura: &'a str,
    token_class: Option<TokenClass>,
    callee_ura: &'a str,
    subject_ura: &'a str,
    ability_ura: &'a str,
    action: AccessAction,
}

fn grant_child_access_for_test(input: ChildAccessGrantInput<'_>) {
    use std::sync::atomic::{AtomicU64, Ordering};

    let ChildAccessGrantInput {
        owner_user_id,
        principal_kind,
        principal_ura,
        token_class,
        callee_ura,
        subject_ura,
        ability_ura,
        action,
    } = input;
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
