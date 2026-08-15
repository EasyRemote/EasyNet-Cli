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
use crate::core::ura::realm_from_ura;
use crate::daemon::axon_bridge::proof_owner::descriptor_bound_canonical_bytes;
use crate::daemon::identity::self_identity::{CanonicalSigner, TestCanonicalSigner};
use crate::daemon::invocation::admission::peer_envelope_signer::sign_peer_request_envelope;
use crate::daemon::invocation::admission::quota_meter::quota_meters_function;
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
use crate::daemon::invocation::dispatch::invocation_wire::{
    wire_invocation_target, FEDERATION_RESULT_CONTENT_TYPE,
};
use crate::daemon::invocation::{InvocationDerivationPolicy, ProtoEnvelope};
use axon_sdk::invocation::{AbilityFrame, BidiInputFrame, CallMode};
use axon_sdk::pb::axon::v1::{
    invoke_bidi_down::Payload as DownPayload, BinaryChunk, StreamDescriptor,
};
use std::collections::HashMap;
use tokio::sync::mpsc;

use crate::daemon::trust::anchor::{
    RealmTrustAnchor, TrustAnchorRole, TrustedAgent, TrustedPrincipalOwner,
};
use axon_sdk::pb::axon::v1::{AgentIdentity, CallerSignature, Envelope, SubjectIdentity};

/// Test helper daemon URA — admitted by the test admission
/// facade via the loopback bypass. Tests that exercise
/// admission rejection construct a different facade.
// URA v4.1.4: daemons are devices, not agents. Fixtures use the
// canonical shape because invoke no longer repairs legacy
// `agent/<bare-id>` device aliases at the request boundary.
const TEST_DAEMON_URA: &str = "easynet:///r/test-realm/device/test-daemon";
const TEST_HUB_URA: &str = "easynet:///r/test-realm/authority";
const TEST_BOOTSTRAP_CALLER_URA: &str = "easynet:///r/test-realm/device/bootstrap-caller";
const TEST_DISCOVER_USER_URA: &str = "easynet:///r/test-realm/user/test-user";
const TEST_DEVICE_SIGNING_SEED: [u8; 32] = [0x33; 32];
const TEST_BOOTSTRAP_CALLER_SIGNING_SEED: [u8; 32] = [0x44; 32];
const TEST_DISCOVER_USER_SIGNING_SEED: [u8; 32] = [0x55; 32];
const TEST_DISPATCH_SYSTEM_AGENT_ID: &str =
    crate::daemon::ability::names::device_control::LOCOMOTION_SYSTEM_AGENT_ID;

fn test_dispatch_system_agent_ura() -> String {
    test_device_system_agent_ura(TEST_DAEMON_URA, TEST_DISPATCH_SYSTEM_AGENT_ID)
}

fn test_device_system_agent_ura(device_ura: &str, agent_id: &str) -> String {
    let parsed = crate::core::ura::parse_ura(device_ura)
        .unwrap_or_else(|error| panic!("test Device URA must parse: {device_ura}: {error}"));
    let device_id = parsed
        .device_id()
        .unwrap_or_else(|| panic!("test Device URA must carry device id: {device_ura}"));
    crate::core::ura::device_agent_ura(&parsed.realm, device_id, agent_id)
}

fn canonical_realm_authority_for_runtime_root(root_ura: &str) -> String {
    let parsed = crate::core::ura::parse_ura(root_ura)
        .unwrap_or_else(|error| panic!("test runtime root URA must parse: {root_ura}: {error}"));
    crate::core::ura::hub_ura(&parsed.realm)
}

fn test_device_authority_root_for_realm(realm: &str) -> String {
    let parsed =
        crate::core::ura::parse_ura(TEST_DAEMON_URA).expect("test daemon Device URA must parse");
    let device_id = parsed
        .device_id()
        .expect("test daemon Device URA must carry a device id");
    crate::core::ura::device_ura(realm, device_id)
}

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
    make_service_with_daemon_route_owner(TEST_HUB_URA)
}

fn make_service_with_daemon_route_owner(route_owner_ura: &str) -> DaemonInvocationService {
    let service = make_unregistered_service_for_route_owner(route_owner_ura);
    register_test_daemon_routes(service, route_owner_ura)
}

fn make_service_with_runtime(
    route_owner_ura: &str,
    runtime_assembly: crate::daemon::axon_bridge::runtime_factory::DaemonRuntimeAssembly,
) -> DaemonInvocationService {
    let service =
        make_unregistered_service_for_route_owner_and_runtime(route_owner_ura, runtime_assembly);
    register_test_daemon_routes(service, route_owner_ura)
}

fn make_service_with_test_runtime(
    runtime_assembly: crate::daemon::axon_bridge::runtime_factory::DaemonRuntimeAssembly,
) -> DaemonInvocationService {
    make_service_with_runtime(TEST_HUB_URA, runtime_assembly)
}

fn make_service_with_presence(presence: Arc<PresenceRegistry>) -> DaemonInvocationService {
    make_service_with_presence_and_heartbeat(presence, None)
}

fn insert_test_dispatch_presence(
    presence: &PresenceRegistry,
    ura: impl Into<String>,
    sender: crate::daemon::invocation::bidi::state::presence::DispatchSender,
) -> Result<crate::daemon::invocation::bidi::state::presence::PresenceRegistration, String> {
    presence.insert_negotiated(
        ura.into(),
        sender,
        crate::daemon::invocation::bidi::state::presence::SessionContract::new(
            crate::daemon::invocation::bidi::state::presence::CANONICAL_SESSION_CARRIER_VERSION,
            vec![0; 16],
        ),
    )
}

fn make_service_with_presence_and_heartbeat(
    presence: Arc<PresenceRegistry>,
    heartbeat_interval_ms: Option<std::num::NonZeroU64>,
) -> DaemonInvocationService {
    let anchor = test_trust_anchor();
    let cell = SharedTrustAnchor::new(Arc::new(anchor));
    let runtime_assembly = test_runtime_assembly(cell.clone());
    let runtime = runtime_assembly.runtime();
    let access_control_stores = Arc::new(
        crate::daemon::persistence::access_control::AccessControlStoreRegistry::ephemeral(),
    );
    let local_ability_catalog = test_catalog_for_route_owner(
        TEST_HUB_URA,
        Arc::clone(&runtime),
        Arc::clone(&access_control_stores),
    );
    let admission = AdmissionFacade::with_trust_anchor_cell(
        cell,
        test_admission_daemon_ura_for_route_owner(TEST_HUB_URA),
    )
    .with_transport_boundary(AdmissionTransportBoundary::LocalOnlyIpc)
    .with_access_control_stores(access_control_stores)
    .with_ability_catalog(Arc::clone(&local_ability_catalog));
    let mut service = DaemonInvocationService::new(presence, admission)
        .with_hub_signer(test_hub_signer("test-realm"))
        .with_local_ability_catalog(local_ability_catalog)
        .with_daemon_runtime(runtime_assembly)
        .with_invocation_attempt_ledger(test_attempt_ledger());
    if let Some(interval) = heartbeat_interval_ms {
        service = service.with_subscribe_v2_heartbeat_interval_ms(interval);
    }
    register_test_daemon_routes(service, TEST_HUB_URA)
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
    install_test_device_authority_grants(service.admission_plane.access_control_stores().as_ref());
    install_test_user_authority_grants(service.admission_plane.access_control_stores().as_ref());
    let runtime = service
        .runtime
        .local_runtime()
        .expect("test service must have a LocalRuntime before route registration");
    let catalog = test_catalog_for_route_owner(
        route_owner_ura,
        runtime,
        service.admission_plane.access_control_stores(),
    );
    service.admission_plane = service
        .admission_plane
        .clone()
        .with_ability_catalog(Arc::clone(&catalog));
    service = service.with_local_ability_catalog(catalog);
    let exact_route_owner = canonical_realm_authority_for_runtime_root(route_owner_ura);
    futures::executor::block_on(service.register_daemon_unary_routes(&exact_route_owner))
        .expect("explicitly assemble daemon exact routes for test service");
    futures::executor::block_on(service.register_daemon_stream_routes(&exact_route_owner))
        .expect("explicitly assemble daemon exact stream routes for test service");
    service
}

fn make_unregistered_service_for_route_owner(route_owner_ura: &str) -> DaemonInvocationService {
    let anchor = test_trust_anchor();
    let cell = SharedTrustAnchor::new(Arc::new(anchor));
    let runtime_assembly = test_runtime_assembly(cell);
    make_unregistered_service_for_route_owner_and_runtime(route_owner_ura, runtime_assembly)
}

fn make_unregistered_service_for_route_owner_and_runtime(
    route_owner_ura: &str,
    runtime_assembly: crate::daemon::axon_bridge::runtime_factory::DaemonRuntimeAssembly,
) -> DaemonInvocationService {
    let anchor = test_trust_anchor();
    let cell = SharedTrustAnchor::new(Arc::new(anchor));
    make_unregistered_service_for_route_owner_and_runtime_trust(
        route_owner_ura,
        runtime_assembly,
        cell,
    )
}

fn make_unregistered_service_for_route_owner_and_runtime_trust(
    route_owner_ura: &str,
    runtime_assembly: crate::daemon::axon_bridge::runtime_factory::DaemonRuntimeAssembly,
    cell: SharedTrustAnchor,
) -> DaemonInvocationService {
    let runtime = runtime_assembly.runtime();
    let access_control_stores = Arc::new(
        crate::daemon::persistence::access_control::AccessControlStoreRegistry::ephemeral(),
    );
    let local_ability_catalog = test_catalog_for_route_owner(
        route_owner_ura,
        Arc::clone(&runtime),
        Arc::clone(&access_control_stores),
    );
    let admission_daemon_ura = test_admission_daemon_ura_for_route_owner(route_owner_ura);
    let admission = AdmissionFacade::with_trust_anchor_cell(cell.clone(), admission_daemon_ura)
        .with_transport_boundary(AdmissionTransportBoundary::LocalOnlyIpc)
        .with_access_control_stores(access_control_stores)
        .with_ability_catalog(Arc::clone(&local_ability_catalog));
    let signer_realm = crate::core::ura::parse_ura(route_owner_ura)
        .map(|parsed| parsed.realm)
        .unwrap_or_else(|_| "test-realm".to_string());
    DaemonInvocationService::new(Arc::new(PresenceRegistry::new()), admission)
        .with_hub_signer(test_hub_signer(&signer_realm))
        .with_local_ability_catalog(local_ability_catalog)
        .with_daemon_runtime(runtime_assembly)
        .with_invocation_attempt_ledger(test_attempt_ledger())
}

fn test_attempt_ledger(
) -> Arc<crate::daemon::invocation::dispatch::attempt_audit::InvocationAttemptLedger> {
    static SEQ: std::sync::atomic::AtomicU64 = std::sync::atomic::AtomicU64::new(1);
    let path = std::env::temp_dir().join(format!(
        "easynet-test-invocation-attempts-{}-{}.jsonl",
        std::process::id(),
        SEQ.fetch_add(1, std::sync::atomic::Ordering::Relaxed)
    ));
    Arc::new(
        crate::daemon::invocation::dispatch::attempt_audit::InvocationAttemptLedger::open(path)
            .expect("test invocation attempt ledger"),
    )
}

/// Install the explicit authority that production now requires for Device
/// principals. The fixture intentionally uses class-level subject/ability
/// matching inside the isolated test User URA store: owner resolution still
/// selects that store first, so this does not reintroduce Device-to-User owner
/// inheritance or authorize a different owner's resources.
fn install_test_device_authority_grants(
    stores: &crate::daemon::persistence::access_control::AccessControlStoreRegistry,
) {
    for (index, principal_ura) in [
        TEST_DAEMON_URA,
        "easynet:///r/test-realm/device/client-1",
        TEST_BOOTSTRAP_CALLER_URA,
    ]
    .into_iter()
    .enumerate()
    {
        stores
            .with_store(TEST_DISCOVER_USER_URA, |store| {
                store.create_grant(
                    PermissionGrant {
                        grant_id: format!("test-device-authority-{index}"),
                        owner_user_ura: TEST_DISCOVER_USER_URA.to_string(),
                        principal_kind: PrincipalKind::DeviceCustody,
                        principal_id: principal_ura.to_string(),
                        token_id: Some(principal_ura.to_string()),
                        token_class: Some(TokenClass::DevicePairing),
                        session_id: None,
                        session_expires_at: None,
                        callee_ura: None,
                        subject_ura_pattern: None,
                        ability_ura_pattern: None,
                        actions: vec![
                            AccessAction::Read,
                            AccessAction::Invoke,
                            AccessAction::Stream,
                            AccessAction::Manage,
                            AccessAction::Grant,
                        ],
                        constraints: None,
                        effect: PermissionEffect::Allow,
                        lifetime: PermissionGrantLifetime::Permanent,
                        state: PermissionGrantState::Active,
                        expires_at: None,
                        review_required_after: None,
                        last_reviewed_at: None,
                        last_used_at: None,
                        created_by: TEST_DISCOVER_USER_URA.to_string(),
                        created_at: "2026-08-07T00:00:00Z".to_string(),
                        updated_at: None,
                        revoked_at: None,
                        reason: Some("explicit daemon invocation test authority".to_string()),
                    },
                    TEST_DISCOVER_USER_URA,
                )
            })
            .expect("open daemon invocation test authority store")
            .expect("create explicit daemon invocation test authority");
    }
}

fn install_test_user_authority_grants(
    stores: &crate::daemon::persistence::access_control::AccessControlStoreRegistry,
) {
    stores
        .with_store(TEST_DISCOVER_USER_URA, |store| {
            store.create_grant(
                PermissionGrant {
                    grant_id: "test-user-authority-invocation".to_string(),
                    owner_user_ura: TEST_DISCOVER_USER_URA.to_string(),
                    principal_kind: PrincipalKind::User,
                    principal_id: TEST_DISCOVER_USER_URA.to_string(),
                    token_id: None,
                    token_class: None,
                    session_id: None,
                    session_expires_at: None,
                    callee_ura: None,
                    subject_ura_pattern: None,
                    ability_ura_pattern: None,
                    actions: vec![
                        AccessAction::Read,
                        AccessAction::Invoke,
                        AccessAction::Stream,
                        AccessAction::Manage,
                        AccessAction::Grant,
                    ],
                    constraints: None,
                    effect: PermissionEffect::Allow,
                    lifetime: PermissionGrantLifetime::Permanent,
                    state: PermissionGrantState::Active,
                    expires_at: None,
                    review_required_after: None,
                    last_reviewed_at: None,
                    last_used_at: None,
                    created_by: TEST_DISCOVER_USER_URA.to_string(),
                    created_at: "2026-08-07T00:00:00Z".to_string(),
                    updated_at: None,
                    revoked_at: None,
                    reason: Some("explicit daemon invocation test user authority".to_string()),
                },
                TEST_DISCOVER_USER_URA,
            )
        })
        .expect("open daemon invocation test user authority store")
        .expect("create explicit daemon invocation test user authority");
}

fn test_trust_anchor() -> RealmTrustAnchor {
    use base64::{engine::general_purpose::STANDARD as BASE64_STANDARD, Engine as _};

    let devices = [
        (
            TEST_DAEMON_URA,
            test_device_signing_key().verifying_key().to_bytes(),
        ),
        (
            "easynet:///r/test-realm/device/client-1",
            test_device_signing_key().verifying_key().to_bytes(),
        ),
        (
            TEST_BOOTSTRAP_CALLER_URA,
            test_bootstrap_caller_signing_key()
                .verifying_key()
                .to_bytes(),
        ),
    ];
    let mut entries = devices
        .iter()
        .map(|(agent_ura, public_key)| TrustedAgent {
            agent_ura: (*agent_ura).to_string(),
            public_key_b64: BASE64_STANDARD.encode(public_key),
            role: TrustAnchorRole::Device,
            added_at_unix_ms: 1_700_000_000_000,
            origin_realm: None,
            hub_endpoint: None,
            tls_ca_pem_path: None,
        })
        .collect::<Vec<_>>();
    entries.push(TrustedAgent {
        agent_ura: TEST_DISCOVER_USER_URA.to_string(),
        public_key_b64: BASE64_STANDARD
            .encode(test_discover_user_signing_key().verifying_key().to_bytes()),
        role: TrustAnchorRole::User,
        added_at_unix_ms: 1_700_000_000_001,
        origin_realm: None,
        hub_endpoint: None,
        tls_ca_pem_path: None,
    });
    entries.push(TrustedAgent {
        agent_ura: TEST_HUB_URA.to_string(),
        public_key_b64: BASE64_STANDARD.encode(test_hub_signing_key().verifying_key().to_bytes()),
        role: TrustAnchorRole::Hub,
        added_at_unix_ms: 1_700_000_000_002,
        origin_realm: None,
        hub_endpoint: None,
        tls_ca_pem_path: None,
    });
    let mut principal_owners = devices
        .iter()
        .map(|(principal_ura, _)| TrustedPrincipalOwner {
            principal_ura: (*principal_ura).to_string(),
            owner_user_id: "test-user".to_string(),
            owner_ura: "easynet:///r/test-realm/user/test-user".to_string(),
            added_at_unix_ms: 1_700_000_000_000,
        })
        .collect::<Vec<_>>();
    principal_owners.push(TrustedPrincipalOwner {
        principal_ura: TEST_HUB_URA.to_string(),
        owner_user_id: "test-user".to_string(),
        owner_ura: TEST_DISCOVER_USER_URA.to_string(),
        added_at_unix_ms: 1_700_000_000_000,
    });
    RealmTrustAnchor::from_parts_with_principal_owners(entries, principal_owners, Vec::new())
        .expect("test daemon trust anchor")
}

fn test_trust_anchor_without_principal_owners() -> RealmTrustAnchor {
    RealmTrustAnchor::from_entries(test_trust_anchor().entries_sorted())
        .expect("test trust anchor without principal-owner projections")
}

fn test_trust_anchor_with_entries(entries: Vec<TrustedAgent>) -> RealmTrustAnchor {
    let mut anchor = test_trust_anchor();
    for entry in entries {
        anchor
            .append_agent(entry)
            .expect("additional test trust entry must be canonical and unique");
    }
    anchor
}

fn test_admission_daemon_ura_for_route_owner(route_owner_ura: &str) -> Option<String> {
    let parsed = crate::core::ura::parse_ura(route_owner_ura).ok()?;
    match parsed.kind {
        crate::core::ura::URAKind::Authority => Some(route_owner_ura.to_string()),
        _ => Some(TEST_DAEMON_URA.to_string()),
    }
}

fn test_catalog_for_route_owner(
    route_owner_ura: &str,
    runtime: Arc<axon_sdk::invocation::LocalRuntime>,
    access_control_stores: Arc<
        crate::daemon::persistence::access_control::AccessControlStoreRegistry,
    >,
) -> Arc<crate::daemon::ability::dispatch::AxonAbilityCatalog> {
    let authority_context = test_authority_context_for_route_owner(route_owner_ura);
    let agents = crate::daemon::persistence::agent_registry::AgentRegistry::default();
    let mut catalog_config =
        crate::daemon::ability::catalog::RegistryBuildConfig::new_with_authority_context(
            crate::daemon::ability::catalog::RegistryBuildServices::fresh()
                .with_access_control_stores(access_control_stores),
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
        crate::core::ura::URAKind::Authority => {
            crate::daemon::ability::dispatch::AbilityAuthorityContext::for_combined_authority_roots(
                test_device_authority_root_for_realm(&parsed.realm),
            )
            .expect("test combined daemon authority context")
        }
        crate::core::ura::URAKind::Device => {
            crate::daemon::ability::dispatch::AbilityAuthorityContext::for_combined_authority_roots(
                route_owner_ura,
            )
            .expect("test Device+realm authority context")
        }
        _ => panic!("daemon route owner must be a Hub or Device URA: {route_owner_ura}"),
    }
}

fn test_runtime_assembly(
    cell: SharedTrustAnchor,
) -> crate::daemon::axon_bridge::runtime_factory::DaemonRuntimeAssembly {
    crate::daemon::axon_bridge::runtime_factory::build_test_daemon_runtime_assembly(
        Arc::new(TestRuntimeKeyResolver::new(cell)),
        crate::daemon::axon_bridge::runtime_factory::isolated_test_runtime_persistence(
            "daemon-invocation-service",
        ),
        None,
    )
}

trait DaemonInvocationServiceTestRuntimeExt {
    fn with_test_daemon_runtime(self, cell: SharedTrustAnchor) -> Self;
}

impl DaemonInvocationServiceTestRuntimeExt for DaemonInvocationService {
    fn with_test_daemon_runtime(self, cell: SharedTrustAnchor) -> Self {
        let runtime_assembly = test_runtime_assembly(cell);
        self.with_daemon_runtime(runtime_assembly)
    }
}

fn test_runtime_with_default_trust(
) -> crate::daemon::axon_bridge::runtime_factory::DaemonRuntimeAssembly {
    test_runtime_assembly(SharedTrustAnchor::new(Arc::new(test_trust_anchor())))
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

impl axon_sdk::invocation::KeyResolver for TestRuntimeKeyResolver {
    fn resolve(
        &self,
        agent_ura: &str,
    ) -> Result<ed25519_dalek::VerifyingKey, axon_sdk::invocation::AxonError> {
        if agent_ura == TEST_DAEMON_URA {
            return Ok(test_device_signing_key().verifying_key());
        }
        axon_sdk::invocation::KeyResolver::resolve(&self.trust, agent_ura)
    }

    fn resolve_all(
        &self,
        agent_ura: &str,
    ) -> Result<Vec<ed25519_dalek::VerifyingKey>, axon_sdk::invocation::AxonError> {
        if agent_ura == TEST_DAEMON_URA {
            return Ok(vec![test_device_signing_key().verifying_key()]);
        }
        axon_sdk::invocation::KeyResolver::resolve_all(&self.trust, agent_ura)
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
    publish_test_projected_route(svc, owner_ura, public_name, TEST_DAEMON_URA);
}

fn publish_test_route_hosted_by(
    svc: &DaemonInvocationService,
    owner_ura: &str,
    public_name: &str,
    hosted_agent_host_ura: &str,
) {
    publish_test_projected_route(svc, owner_ura, public_name, hosted_agent_host_ura);
}

fn publish_test_projected_route(
    svc: &DaemonInvocationService,
    callee_owner_ura: &str,
    public_name: &str,
    execution_host_ura: &str,
) {
    let public_name = crate::core::ura::owner_local_ability_name(callee_owner_ura, public_name);
    let ability_ura = crate::core::ura::owner_ability_ura(callee_owner_ura, &public_name)
        .unwrap_or_else(|| panic!("derive test ability URA for {callee_owner_ura} {public_name}"));
    let host_ura = match crate::core::ura::parse_ura(callee_owner_ura).map(|parsed| parsed.kind) {
        Ok(crate::core::ura::URAKind::Agent) => {
            let advertised_host_node_id = crate::core::ura::parse_ura(execution_host_ura)
                .ok()
                .and_then(|parsed| {
                    (parsed.kind == crate::core::ura::URAKind::Device)
                        .then(|| parsed.device_id().map(str::to_string))
                        .flatten()
                })
                .unwrap_or_else(|| execution_host_ura.to_string());
            svc.directory.advertised_agents.upsert(
                crate::daemon::federation::read_model::advertised_agents::AdvertisedAgentRecord {
                    agent_ura: callee_owner_ura.to_string(),
                    generation: 1,
                    public_key_hex: String::new(),
                    host_node_id: Some(advertised_host_node_id),
                    signing_authority:
                        crate::daemon::federation::read_model::advertised_agents::AdvertisedAgentSigningAuthority::HostedBy {
                            host_ura: execution_host_ura.to_string(),
                        },
                },
            );
            execution_host_ura.to_string()
        }
        _ => execution_host_ura.to_string(),
    };
    if !svc.directory.presence.contains(&host_ura) {
        let (tx, _rx) = tokio::sync::mpsc::channel(1);
        insert_test_dispatch_presence(&svc.directory.presence, host_ura.clone(), tx)
            .expect("canonical presence key");
    }
    let (namespace, local_name) = public_name
        .rsplit_once('.')
        .map_or(("", public_name.as_str()), |(namespace, local_name)| {
            (namespace, local_name)
        });
    svc.directory.ability_catalog.upsert_projection(
        crate::daemon::federation::read_model::ability_catalog::OwnerAbilityProjectionRow::new(
            callee_owner_ura.to_string(),
            host_ura,
            1,
            1,
            "sha256:test".to_string(),
            4_102_444_800_000,
            vec![crate::daemon::federation::read_model::owner_projection::AbilityProjectionSummary {
                ability_ura: ability_ura.clone(),
                owner_ura: callee_owner_ura.to_string(),
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

fn publish_test_remote_system_agent_route(
    svc: &DaemonInvocationService,
    host_device_ura: &str,
    public_name: &str,
) -> String {
    publish_test_remote_system_agent_route_with_mode(
        svc,
        host_device_ura,
        public_name,
        crate::daemon::ability::CallMode::Rpc,
    )
}

fn publish_test_remote_system_agent_route_with_mode(
    svc: &DaemonInvocationService,
    host_device_ura: &str,
    public_name: &str,
    call_mode: crate::daemon::ability::CallMode,
) -> String {
    let system_agent_id = match public_name {
        crate::daemon::ability::names::governance::OBSERVE_HEALTH => {
            crate::daemon::ability::names::governance::RUNTIME_HEALTH_SYSTEM_AGENT_ID
        }
        crate::daemon::ability::names::governance::INVOCATION_HISTORY_LIST => {
            crate::daemon::ability::names::governance::RUNTIME_GOVERNANCE_SYSTEM_AGENT_ID
        }
        crate::daemon::ability::names::governance::META_LIST_ABILITIES
        | crate::daemon::ability::names::resources::META_LIST_RESOURCES => {
            crate::daemon::ability::names::governance::RUNTIME_INTROSPECTION_SYSTEM_AGENT_ID
        }
        crate::daemon::ability::names::device_control::SHELL_RUN => {
            crate::daemon::ability::names::device_control::LOCOMOTION_SYSTEM_AGENT_ID
        }
        _ => TEST_DISPATCH_SYSTEM_AGENT_ID,
    };
    let callee_ura = test_device_system_agent_ura(host_device_ura, system_agent_id);
    let _ = call_mode;
    publish_test_projected_route(svc, &callee_ura, public_name, host_device_ura);
    callee_ura
}

fn register_test_catalog_route(
    svc: &DaemonInvocationService,
    owner_ura: &str,
    public_name: &str,
    call_mode: crate::daemon::ability::CallMode,
) {
    let parsed_owner = crate::core::ura::parse_ura(owner_ura)
        .unwrap_or_else(|error| panic!("test route owner URA must parse: {owner_ura}: {error}"));
    assert_ne!(
        parsed_owner.kind,
        crate::core::ura::URAKind::Device,
        "positive test routes must not make Device an ability owner/callee; use a SystemAgent, Agent, or Authority owner"
    );
    let Some(catalog) = svc.directory.local_ability_catalog.as_ref() else {
        return;
    };
    let Some(owner) = local_test_owner_kind(svc, owner_ura) else {
        return;
    };
    if matches!(
        owner,
        crate::daemon::ability::dispatch::OwnerKind::SystemAgent(_)
    ) && catalog.authority_set_label() == "realm-authority"
    {
        return;
    }
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
        crate::core::ura::URAKind::Device => None,
        crate::core::ura::URAKind::Authority => svc
            .identity
            .session_realm
            .as_deref()
            .is_some_and(|realm| crate::core::ura::hub_ura(realm) == owner_ura)
            .then_some(crate::daemon::ability::dispatch::OwnerKind::RealmAuthority),
        crate::core::ura::URAKind::Agent => {
            if let Some((device_id, agent_id)) = parsed.device_agent_ids() {
                let is_local_device_sponsored_system_agent =
                    crate::core::ura::parse_ura(TEST_DAEMON_URA)
                        .ok()
                        .and_then(|daemon| daemon.device_id().map(str::to_string))
                        .is_some_and(|local_device_id| local_device_id == device_id);
                return is_local_device_sponsored_system_agent.then_some(
                    crate::daemon::ability::dispatch::OwnerKind::SystemAgent(agent_id.to_string()),
                );
            }
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
                    == svc.admission_plane.verifier_ref().daemon_ura()
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
) -> crate::daemon::axon_bridge::runtime_factory::DaemonRuntimeAssembly {
    let runtime_assembly = test_runtime_with_default_trust();
    let _catalog = catalog_with_json_echo_on_runtime(
        owner_ura,
        ability,
        marker_key,
        marker_value,
        runtime_assembly.runtime(),
    );
    runtime_assembly
}

fn catalog_with_json_echo_on_runtime(
    owner_ura: &str,
    ability: &'static str,
    marker_key: &'static str,
    marker_value: &'static str,
    runtime: Arc<axon_sdk::invocation::LocalRuntime>,
) -> Arc<crate::daemon::ability::dispatch::AxonAbilityCatalog> {
    use crate::daemon::ability::dispatch::{
        AbilityAuthorityContext, AxonAbilityCatalog, LocalRpcHandler, OwnerKind,
    };
    use crate::daemon::ability::{descriptors::AdmissionAction, manifest::AbilityManifest};

    let parsed_owner = crate::core::ura::parse_ura(owner_ura)
        .unwrap_or_else(|error| panic!("echo fixture owner URA must parse: {owner_ura}: {error}"));
    let (authority_root_ura, owner_kind) = match parsed_owner.kind {
        crate::core::ura::URAKind::Agent => {
            let (device_id, agent_id) = parsed_owner.device_agent_ids().unwrap_or_else(|| {
                panic!(
                    "echo fixture Agent owner must be a device-sponsored SystemAgent: {owner_ura}"
                )
            });
            (
                crate::core::ura::device_ura(&parsed_owner.realm, device_id),
                OwnerKind::SystemAgent(agent_id.to_string()),
            )
        }
        other => panic!("echo fixture owner must be a SystemAgent, got {other:?}"),
    };
    let mut catalog = AxonAbilityCatalog::new_with_runtime_and_authority_context(
        runtime,
        AbilityAuthorityContext::for_device_authority_root(&authority_root_ura)
            .expect("echo fixture authority root is a Device"),
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
        owner_kind,
        AdmissionAction::Invoke,
        manifest,
        handler,
    );
    Arc::new(catalog)
}

fn test_envelope() -> Envelope {
    ProtoEnvelope::from_target(
        crate::daemon::identity::local_invocation::LOCAL_SYSTEM_AGENT_URA,
        TEST_HUB_URA,
        TEST_DAEMON_URA,
        InvocationDerivationPolicy::FreshRoot,
    )
    .expect("valid test envelope")
    .into_inner("test.envelope", b"")
    .expect("complete test tuple")
}

#[test]
fn daemon_exact_route_owner_rejects_device_ura() {
    let error = normalize_daemon_route_owners(&[TEST_DAEMON_URA.to_string()])
        .expect_err("Device must not own public daemon exact routes");
    assert!(
        error
            .to_string()
            .contains("canonical realm Authority owner"),
        "unexpected error: {error}"
    );

    for route_family in ["stream", "bidi"] {
        let error = validate_daemon_route_authority_owner(TEST_DAEMON_URA, route_family)
            .expect_err("Device must not own public daemon exact route families");
        assert!(
            error
                .to_string()
                .contains("canonical realm Authority owner"),
            "unexpected {route_family} error: {error}"
        );
    }
}

#[test]
fn route_table_match_projects_descriptor_ref_to_public_name() {
    let ability =
        crate::daemon::invocation::dispatch::federation_wrappers::ABILITY_FEDERATION_RESOLVE;
    let descriptor_ref = test_descriptor_ref(TEST_HUB_URA, ability);
    let envelope = test_envelope();

    assert_eq!(
        dispatch_function_name_for_route_table(&descriptor_ref, Some(&envelope))
            .expect("descriptor ref route table projection"),
        ability
    );
}

#[test]
fn route_table_projects_hub_bidi_descriptor_ref_to_session_open() {
    let hub_ura = crate::core::ura::hub_ura("test-realm");
    let descriptor_ref =
        crate::daemon::axon_bridge::descriptor_ref::system_protocol_descriptor_ref_for_wire(
            &hub_ura,
            ABILITY_SESSION_OPEN,
            crate::daemon::ability::CallMode::Bidi,
        )
        .expect("session.open bidi descriptor ref");
    let envelope = ProtoEnvelope::from_target(
        crate::daemon::identity::local_invocation::LOCAL_SYSTEM_AGENT_URA,
        &hub_ura,
        crate::daemon::identity::local_invocation::LOCAL_SYSTEM_AGENT_URA,
        InvocationDerivationPolicy::FreshRoot,
    )
    .expect("valid hub envelope")
    .into_inner("test.session.open", b"")
    .expect("complete hub tuple");

    assert_eq!(
        dispatch_function_name_for_route_table(&descriptor_ref, Some(&envelope))
            .expect("session.open descriptor ref route table projection"),
        ABILITY_SESSION_OPEN
    );
}

#[test]
fn route_table_rejects_malformed_descriptor_ref_before_name_fallback() {
    let envelope = test_envelope();
    let malformed_descriptor_ref =
        crate::core::ura::owner_ability_ura(TEST_HUB_URA, "federation.resolve")
            .expect("test malformed Authority ability URA");
    let error = dispatch_function_name_for_route_table(&malformed_descriptor_ref, Some(&envelope))
        .expect_err("malformed descriptor refs must not fall back to public-name lookup");

    assert_eq!(error.code(), tonic::Code::InvalidArgument);
    assert!(
        error
            .message()
            .contains("descriptor_ref selector projection failed"),
        "unexpected route-table descriptor error: {error}"
    );
}

#[test]
fn route_table_rejects_descriptor_ref_owner_mismatch_before_name_fallback() {
    let ability =
        crate::daemon::invocation::dispatch::federation_wrappers::ABILITY_FEDERATION_RESOLVE;
    let other_owner = crate::core::ura::authority_ura("other-realm");
    let descriptor_ref = test_descriptor_ref(&other_owner, ability);
    let envelope = test_envelope();

    let error = dispatch_function_name_for_route_table(&descriptor_ref, Some(&envelope))
        .expect_err("descriptor owner mismatch must not fall back to selected-route lookup");

    assert_eq!(error.code(), tonic::Code::InvalidArgument);
    assert!(
        error.message().contains("does not match envelope callee"),
        "unexpected route-table owner mismatch error: {error}"
    );
}

fn test_device_signing_key() -> ed25519_dalek::SigningKey {
    ed25519_dalek::SigningKey::from_bytes(&TEST_DEVICE_SIGNING_SEED)
}

fn test_hub_signing_key() -> ed25519_dalek::SigningKey {
    ed25519_dalek::SigningKey::from_bytes(&[0x11; 32])
}

fn test_bootstrap_caller_signing_key() -> ed25519_dalek::SigningKey {
    ed25519_dalek::SigningKey::from_bytes(&TEST_BOOTSTRAP_CALLER_SIGNING_SEED)
}

fn test_discover_user_signing_key() -> ed25519_dalek::SigningKey {
    ed25519_dalek::SigningKey::from_bytes(&TEST_DISCOVER_USER_SIGNING_SEED)
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
        crate::daemon::axon_bridge::descriptor_ref::system_protocol_descriptor_ref_for_wire(
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
        .find(|row| {
            row.descriptor.owner_ura == callee_ura
                && row.name == ability
                && row.descriptor.call_mode() == call_mode
        })
        .unwrap_or_else(|| {
            panic!("catalog descriptor row for {callee_ura}#{ability} in {call_mode:?}")
        });
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
    let function_name =
        crate::daemon::invocation::dispatch::invocation_wire::function_name_from_invocation_target(
            "test request",
            request.target.as_ref(),
        )
        .expect("test request typed target")
        .to_string();
    request.envelope = Some(signed_test_envelope_with_descriptor_ref(
        caller_ura,
        callee_ura,
        subject_ura,
        descriptor_ref.clone(),
        &request.arguments,
        signing_key,
    ));
    request.target = Some(
        wire_invocation_target(&descriptor_ref, &function_name).expect("test descriptor target"),
    );
}

fn test_invocation_target(function_name: &str) -> axon_sdk::pb::axon::v1::InvocationTarget {
    let callee_ura = test_dispatch_system_agent_ura();
    wire_invocation_target(
        test_descriptor_ref(&callee_ura, function_name),
        function_name,
    )
    .expect("test descriptor target")
}

fn invocation_function_name(request: &InvokeRequest) -> &str {
    crate::daemon::invocation::dispatch::invocation_wire::function_name_from_invocation_target(
        "test request",
        request.target.as_ref(),
    )
    .expect("test request typed target")
}

async fn sync_runtime_proof_from_catalog(
    svc: &DaemonInvocationService,
    owner_ura: &str,
    ability: &str,
    call_mode: crate::daemon::ability::CallMode,
) {
    use axon_sdk::invocation::{AbilityCallModes, AbilityOptions, CallMode as AxonCallMode};

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
        .local_runtime()
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
    let hub_ura = crate::core::ura::hub_ura(realm);
    let descriptor_ref = test_descriptor_ref(&hub_ura, ABILITY_FEDERATION_JOIN);
    let signer = TestCanonicalSigner::new(membership_ura.to_string(), test_seed);
    crate::daemon::invocation::ProtoEnvelope::federation_join_bootstrap(
        hub_ura,
        membership_ura,
        InvocationDerivationPolicy::FreshRoot,
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
    let mut envelope = ProtoEnvelope::from_target(
        caller_ura,
        callee_ura,
        subject_ura,
        InvocationDerivationPolicy::Explicit {
            invocation_nonce: nonce,
            causal_context: axon_sdk::invocation::CausalContext::None,
        },
    )
    .expect("valid signed test envelope")
    .into_inner(&descriptor_ref, arguments)
    .expect("complete signed test tuple");
    let descriptor_bound =
        crate::daemon::axon_bridge::wire_descriptor::descriptor_bound_from_wire_parts(
            envelope.clone(),
            descriptor_ref,
            arguments,
        )
        .expect("descriptor-bound signed test envelope");
    let signature = signing_key.sign(&descriptor_bound_canonical_bytes(
        &descriptor_bound.envelope,
    ));
    envelope.caller_signature = Some(CallerSignature {
        algorithm: "ed25519".to_string(),
        signature: signature.to_bytes().to_vec(),
        key_id_hint: String::new(),
    });
    envelope
}

fn invoke_request(function_name: &str, args_json: &str) -> Request<InvokeRequest> {
    invoke_request_for_callee(TEST_HUB_URA, function_name, args_json)
}

fn authority_invoke_request(function_name: &str, args_json: &str) -> Request<InvokeRequest> {
    signed_invoke_request(
        TEST_HUB_URA,
        TEST_HUB_URA,
        TEST_HUB_URA,
        function_name,
        args_json,
        &test_hub_signing_key(),
    )
}

fn backend_invoke_request(function_name: &str, args_json: &str) -> Request<InvokeRequest> {
    authority_invoke_request(function_name, args_json)
}

fn user_scoped_discover_request(args_json: &str) -> Request<InvokeRequest> {
    signed_invoke_request(
        TEST_DISCOVER_USER_URA,
        TEST_HUB_URA,
        &crate::core::ura::resource_dot_ura("test-realm", "user.test-user", "directory/devices"),
        ABILITY_FEDERATION_DISCOVER,
        args_json,
        &test_discover_user_signing_key(),
    )
}

fn invoke_request_for_callee(
    callee_ura: &str,
    function_name: &str,
    args_json: &str,
) -> Request<InvokeRequest> {
    signed_invoke_request(
        TEST_DISCOVER_USER_URA,
        callee_ura,
        callee_ura,
        function_name,
        args_json,
        &test_discover_user_signing_key(),
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
        target: Some(
            wire_invocation_target(&descriptor_ref, function_name).expect("test descriptor target"),
        ),
        arguments,
        ..InvokeRequest::default()
    })
}

fn parse_response_body<T: serde::de::DeserializeOwned>(resp: Response<InvokeResponse>) -> T {
    let body = resp.into_inner();
    assert_eq!(body.result_content_type, FEDERATION_RESULT_CONTENT_TYPE);
    serde_json::from_slice(&body.result).expect("response body deserialises")
}

fn expect_canonical_in_band_failure(
    result: Result<Response<InvokeResponse>, Status>,
    expected_code: axon_sdk::invocation::ErrorCode,
    expectation: &str,
) -> axon_sdk::pb::axon::v1::Error {
    let body = result
        .unwrap_or_else(|status| {
            panic!(
                "{expectation}: invocation failure must be in-band, got transport status: {status}"
            )
        })
        .into_inner();
    assert_eq!(
        body.state,
        axon_sdk::invocation::InvocationState::Failed.to_wire_i32(),
        "{expectation}: canonical failure must use the Failed terminal state"
    );
    let error = body
        .error
        .expect("canonical Failed outcome must carry a typed error");
    assert_eq!(
        error.code,
        expected_code.as_str(),
        "{expectation}: wrong canonical error code"
    );
    error
}

// Shared invoke_remote frame helpers used by stream and bidi tests.
// Canonical session dispatch helpers.

use axon_sdk::pb::axon::v1::invoke_bidi_up::Payload as UpPayload;
use axon_sdk::pb::axon::v1::{BidiControl, EnvelopeOpen, InvokeBidiUp};
fn make_envelope_open(ability: &str, initial_args: Vec<u8>) -> EnvelopeOpen {
    let signing_key = test_device_signing_key();
    let callee_ura = test_dispatch_system_agent_ura();
    let descriptor_ref = test_descriptor_ref(&callee_ura, ability);
    EnvelopeOpen {
        envelope: Some(signed_test_envelope(
            TEST_DAEMON_URA,
            &callee_ura,
            &callee_ura,
            ability,
            &initial_args,
            &signing_key,
        )),
        target: Some(
            wire_invocation_target(descriptor_ref, ability).expect("test descriptor target"),
        ),
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

fn make_envelope_open_with_callee(callee_ura: &str) -> EnvelopeOpen {
    let ability =
        crate::daemon::ability::builtins::device_control::terminal::attach::ABILITY_TERMINAL_ATTACH;
    let signing_key = test_device_signing_key();
    let descriptor_ref = test_descriptor_ref(callee_ura, ability);
    EnvelopeOpen {
        envelope: Some(signed_test_envelope(
            TEST_DAEMON_URA,
            callee_ura,
            callee_ura,
            ability,
            &[],
            &signing_key,
        )),
        target: Some(
            wire_invocation_target(descriptor_ref, ability).expect("test descriptor target"),
        ),
        ..EnvelopeOpen::default()
    }
}

fn test_owner_ability_ura(target_ura: &str, ability: &str) -> String {
    let public_ability = crate::core::ura::owner_local_ability_name(target_ura, ability);
    crate::core::ura::owner_ability_ura(target_ura, &public_ability)
        .unwrap_or_else(|| panic!("derive test ability URA for {target_ura} {public_ability}"))
}

struct ChildAccessGrantInput<'a> {
    owner_user_ura: &'a str,
    principal_kind: PrincipalKind,
    principal_ura: &'a str,
    token_class: Option<TokenClass>,
    callee_ura: &'a str,
    subject_ura: &'a str,
    ability_ura: &'a str,
    action: AccessAction,
}

fn grant_child_access_for_test(
    stores: &crate::daemon::persistence::access_control::AccessControlStoreRegistry,
    input: ChildAccessGrantInput<'_>,
) {
    use std::sync::atomic::{AtomicU64, Ordering};

    let ChildAccessGrantInput {
        owner_user_ura,
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
    stores
        .with_store(owner_user_ura, |store| {
            store.create_grant(
                PermissionGrant {
                    grant_id: format!("test-grant-{n}"),
                    owner_user_ura: owner_user_ura.to_string(),
                    principal_kind,
                    principal_id: principal_ura.to_string(),
                    token_id,
                    token_class,
                    session_id: None,
                    session_expires_at: None,
                    callee_ura: Some(callee_ura.to_string()),
                    subject_ura_pattern: Some(subject_ura.to_string()),
                    ability_ura_pattern: Some(ability_ura.to_string()),
                    actions: vec![action],
                    constraints: None,
                    effect: PermissionEffect::Allow,
                    lifetime: PermissionGrantLifetime::Permanent,
                    state: PermissionGrantState::Active,
                    expires_at: None,
                    review_required_after: None,
                    last_reviewed_at: None,
                    last_used_at: None,
                    created_by: owner_user_ura.to_string(),
                    created_at: "2026-07-09T00:00:00Z".to_string(),
                    updated_at: None,
                    revoked_at: None,
                    reason: Some("forward-invoke test fixture".to_string()),
                },
                owner_user_ura,
            )
        })
        .expect("open test access-control store")
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

#[path = "daemon_invocation_service_tests/admission.rs"]
mod admission;
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
