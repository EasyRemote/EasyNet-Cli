//! Resolve-before-invoke end-to-end integration test (RFC-005 Phase C)
//! ===================================================================
//!
//! File: tests/resolve_before_invoke_e2e.rs
//! Purpose: X1 acceptance test for the CLI half of RFC-005. Proves
//!          that the daemon's unary `Invoke` product path runs
//!          `namespace.resolve` FIRST, dispatches only via the
//!          resolver-selected route — over a real tonic gRPC server
//!          on a tempfile UDS, with no mocks on the dispatch path.
//!
//! What this test exercises (the real bytes, not mocks)
//! ----------------------------------------------------
//! 1. A real `tonic::transport::Server` hosts a
//!    `DaemonInvocationService` wired with a real Axon
//!    `LocalRuntime` and a real `PresenceRegistry`, on a UDS.
//! 2. The hub read model is seeded with the same admitted
//!    owner-projection shape that `federation.advertise_abilities`
//!    persists after publication authority admission.
//! 3. A unary `Invoke` of the device-local ability drives
//!    `resolve_local_rpc_route` → `DaemonRouteResolver` →
//!    remote device FINAL_ROUTE → pending-dispatch precondition.
//!    The route is proven by descriptor-bound admission plus the
//!    projection read model, not by a DeviceAgent shortcut.
//! 4. A unary `Invoke` of a known online device ability without a
//!    pending dispatcher reaches the same dispatch precondition.
//! 5. A unary `Invoke` against a non-local owner that is *not online*
//!    surfaces `ROUTE_NEGATIVE` + `NEGATIVE_REASON_NXDOMAIN`.
//!
//! Per `team-work/conventions/cargo-discipline.md`: single tokio
//! runtime, every blocking await bounded by a timeout so a hang
//! regression surfaces as a failure, never a CI hang.
//!
//! Author: Silan Hu <silan.hu@u.nus.edu>
//! Copyright (c) 2026 EasyNet. All rights reserved.

#![cfg(feature = "axon-pb")]

use std::io::Write;
use std::sync::Arc;
use std::time::Duration;

use base64::{engine::general_purpose::STANDARD as BASE64_STANDARD, Engine as _};
use easynet_axon::invocation::LocalRuntime;
use easynet_axon::pb::axon::v1::invocation_client::InvocationClient;
use easynet_axon::pb::axon::v1::invocation_server::InvocationServer;
use easynet_axon::pb::axon::v1::InvokeRequest;
use easynet_cli::daemon::ability::descriptors::{AbilityDescriptor, CallMode};
use easynet_cli::daemon::ability::dispatch::{
    AbilityAuthorityContext, AxonAbilityCatalog, OwnerKind,
};
use easynet_cli::daemon::identity::self_identity::{SelfIdentity, SelfIdentityError};
use easynet_cli::daemon::invocation::admission::admission_facade::{
    AdmissionFacade, AdmissionTransportBoundary,
};
use easynet_cli::daemon::invocation::bidi::state::presence::{PresenceRegistry, SessionContract};
use easynet_cli::daemon::invocation::dispatch::daemon_invocation_service::DaemonInvocationService;
use easynet_cli::daemon::invocation::dispatch::invocation_wire::ProtoEnvelope;
use easynet_cli::daemon::trust::anchor::RealmTrustAnchor;
use ed25519_dalek::{Signature, Signer as _, SigningKey, VerifyingKey};
use serde_json::json;
use tokio::net::UnixListener;
use tokio::sync::{mpsc, oneshot};
use tokio::task::JoinHandle;
use tokio_stream::wrappers::UnixListenerStream;
use tonic::transport::{Channel, Endpoint, Server, Uri};
use tonic::Request;

const REALM: &str = "test-realm";
const DEVICE_URA: &str = "easynet:///r/test-realm/device/device-a";
const REMOTE_DEVICE_URA: &str = "easynet:///r/test-realm/device/device-b";
const HUB_URA: &str = "easynet:///r/test-realm/hub";
const ABILITY_PUBLIC_NAME: &str = "observe.health";
const UNBOUND_ABILITY_PUBLIC_NAME: &str = "observe.network_health";
const ABILITY_URA: &str = "easynet:///r/test-realm/ability/device.device-a.observe.health";
const DEVICE_SIGNING_SEED: [u8; 32] = [0xA1; 32];

/// Any single in-process step that takes longer than this is a
/// pipeline regression, not legitimate work.
const STEP_TIMEOUT: Duration = Duration::from_secs(5);

struct TestSigner(SigningKey);

impl SelfIdentity for TestSigner {
    fn sign(
        &self,
        _self_ura: &str,
        canonical_bytes: &[u8],
    ) -> Result<Signature, SelfIdentityError> {
        Ok(self.0.sign(canonical_bytes))
    }

    fn public_key(&self, _self_ura: &str) -> Result<VerifyingKey, SelfIdentityError> {
        Ok(self.0.verifying_key())
    }
}

fn device_signer() -> TestSigner {
    TestSigner(SigningKey::from_bytes(&DEVICE_SIGNING_SEED))
}

fn device_public_key_b64() -> String {
    BASE64_STANDARD.encode(device_signer().0.verifying_key().to_bytes())
}

/// An in-process daemon hosting a real `DaemonInvocationService`
/// over a tempfile UDS. On `Drop` it signals shutdown and aborts
/// the server task so no orphan listener survives the test.
struct TestDaemon {
    socket_path: std::path::PathBuf,
    presence: Arc<PresenceRegistry>,
    catalog: Arc<AxonAbilityCatalog>,
    ability_catalog_store:
        Arc<easynet_cli::daemon::federation::read_model::ability_catalog::AbilityCatalogStore>,
    _tempdir: tempfile::TempDir,
    shutdown: Option<oneshot::Sender<()>>,
    server: Option<JoinHandle<()>>,
}

impl Drop for TestDaemon {
    fn drop(&mut self) {
        if let Some(tx) = self.shutdown.take() {
            let _ = tx.send(());
        }
        if let Some(handle) = self.server.take() {
            handle.abort();
        }
    }
}

/// Boot a hub-mode daemon with a production-shaped combined Device+Hub ability
/// catalog and real `LocalRuntime`. Presence is left empty so individual tests
/// choose whether the owner is online.
async fn start_daemon() -> TestDaemon {
    let tempdir = tempfile::tempdir().expect("tempdir");
    let socket_path = tempdir.path().join("daemon.sock");

    let trust_path = tempdir.path().join("realm-trust.toml");
    let device_public_key_b64 = device_public_key_b64();
    let trust_toml = format!(
        r#"
[[trusted_agent]]
agent_ura = "{DEVICE_URA}"
public_key_b64 = "{device_public_key_b64}"
role = "device"
added_at_unix_ms = 0

[[trusted_agent]]
agent_ura = "{REMOTE_DEVICE_URA}"
public_key_b64 = "AAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAA="
role = "device"
added_at_unix_ms = 0

[[trusted_principal_owner]]
principal_ura = "{DEVICE_URA}"
owner_user_id = "owner-a"
owner_ura = "easynet:///r/test-realm/user/owner-a"
added_at_unix_ms = 0

[[trusted_principal_owner]]
principal_ura = "{REMOTE_DEVICE_URA}"
owner_user_id = "owner-b"
owner_ura = "easynet:///r/test-realm/user/owner-b"
added_at_unix_ms = 0
"#,
    );
    let mut f = std::fs::File::create(&trust_path).expect("create trust toml");
    f.write_all(trust_toml.as_bytes())
        .expect("write trust toml");
    drop(f);

    let runtime = LocalRuntime::new();
    let authority_context = AbilityAuthorityContext::for_combined_authority_roots(DEVICE_URA)
        .expect("combined Device+Hub authority");
    let agents = easynet_cli::daemon::persistence::agent_registry::AgentRegistry::default();
    let mut catalog_config =
        easynet_cli::daemon::ability::catalog::RegistryBuildConfig::new_with_authority_context(
            easynet_cli::daemon::ability::catalog::RegistryBuildServices::fresh(),
            &agents,
            authority_context,
        );
    catalog_config.local_runtime = Some(Arc::clone(&runtime));
    let catalog =
        easynet_cli::daemon::ability::catalog::build_registry_with_services_result(catalog_config)
            .expect("assemble production-shaped test ability catalog")
            .catalog;
    let trust_anchor = RealmTrustAnchor::try_load_strict(&trust_path).expect("load trust anchor");
    let presence = Arc::new(PresenceRegistry::new());
    let advertised_agents = Arc::new(
        easynet_cli::daemon::federation::read_model::advertised_agents::AdvertisedAgentStore::new(),
    );
    let ability_catalog_store = Arc::new(
        easynet_cli::daemon::federation::read_model::ability_catalog::AbilityCatalogStore::new(),
    );
    let admission = AdmissionFacade::new(Arc::new(trust_anchor), Some(HUB_URA.to_string()))
        .with_ability_catalog(Arc::clone(&catalog));

    let service = DaemonInvocationService::new(Arc::clone(&presence), admission)
        .with_session_realm(REALM)
        .with_local_runtime(runtime)
        .with_transport_boundary(AdmissionTransportBoundary::LocalOnlyIpc)
        .with_directory_read_models(advertised_agents, Arc::clone(&ability_catalog_store))
        .with_local_ability_catalog(Arc::clone(&catalog));
    service
        .register_daemon_unary_routes(HUB_URA)
        .await
        .expect("register daemon exact routes before exposing test server");

    let listener = UnixListener::bind(&socket_path).expect("bind UDS");
    let incoming = UnixListenerStream::new(listener);
    let (shutdown_tx, shutdown_rx) = oneshot::channel::<()>();
    let server = tokio::spawn(async move {
        let _ = Server::builder()
            .add_service(InvocationServer::new(service))
            .serve_with_incoming_shutdown(incoming, async {
                let _ = shutdown_rx.await;
            })
            .await;
    });
    tokio::time::sleep(Duration::from_millis(20)).await;

    TestDaemon {
        socket_path,
        presence,
        catalog,
        ability_catalog_store,
        _tempdir: tempdir,
        shutdown: Some(shutdown_tx),
        server: Some(server),
    }
}

async fn connect(socket_path: &std::path::Path) -> Channel {
    let socket_path = socket_path.to_path_buf();
    Endpoint::try_from("http://[::]:50051")
        .expect("dummy endpoint")
        .connect_with_connector(tower::service_fn(move |_: Uri| {
            let path = socket_path.clone();
            async move {
                let stream = tokio::net::UnixStream::connect(path).await?;
                Ok::<_, std::io::Error>(hyper_util::rt::TokioIo::new(stream))
            }
        }))
        .await
        .expect("connect to daemon")
}

/// Mark `DEVICE_URA` online in the presence registry. The dispatch
/// sender is never read on these paths (resolve only consults the
/// registry for liveness), so a throwaway channel is sufficient.
fn mark_owner_online(presence: &PresenceRegistry) {
    let (tx, _rx) = mpsc::channel(1);
    presence.insert_negotiated(
        DEVICE_URA.to_string(),
        tx,
        SessionContract {
            version: 1,
            claimant_boot_nonce: vec![0xA1; 16],
        },
    );
}

/// Build a unary `Invoke` of `function_name` against `callee_ura`,
/// with `args` as the JSON argument payload.
fn invoke(
    catalog: &AxonAbilityCatalog,
    callee_ura: &str,
    function_name: &str,
    args: serde_json::Value,
) -> Request<InvokeRequest> {
    invoke_with_subject(catalog, callee_ura, DEVICE_URA, function_name, args)
}

fn invoke_with_subject(
    catalog: &AxonAbilityCatalog,
    callee_ura: &str,
    subject_ura: &str,
    function_name: &str,
    args: serde_json::Value,
) -> Request<InvokeRequest> {
    let arguments = args.to_string().into_bytes();
    let signer = device_signer();
    let descriptor_ref = fixture_descriptor_ref(catalog, callee_ura, function_name);
    Request::new(
        ProtoEnvelope::targeted(DEVICE_URA, callee_ura, subject_ura)
            .expect("valid invoke envelope")
            .signed_descriptor_ref_invoke_request(function_name, descriptor_ref, arguments, &signer)
            .expect("valid signed invoke request"),
    )
}

fn fixture_descriptor_ref(
    catalog: &AxonAbilityCatalog,
    callee_ura: &str,
    function_name: &str,
) -> String {
    let descriptor = fixture_descriptor(catalog, callee_ura, function_name);
    let ability_ura = descriptor
        .canonical_ability_ura()
        .expect("fixture descriptor has canonical ability URA");
    easynet_axon::invocation::canonical_ability_descriptor_ref(&format!(
        "{}@{}#{}!{}",
        ability_ura,
        descriptor.version,
        hex::encode(descriptor.descriptor_hash_bytes()),
        descriptor.admission_action().as_str()
    ))
    .expect("fixture descriptor ref is canonical")
}

fn fixture_descriptor(
    catalog: &AxonAbilityCatalog,
    callee_ura: &str,
    function_name: &str,
) -> AbilityDescriptor {
    let owner = catalog_owner_kind_for(callee_ura);
    let mut matches = catalog
        .authority_ability_catalog_snapshot()
        .into_iter()
        .filter(|row| row.owner == owner)
        .filter(|row| row.name == function_name)
        .filter(|row| row.descriptor.call_mode() == CallMode::Rpc);
    let descriptor = matches
        .next()
        .unwrap_or_else(|| {
            panic!("fixture catalog missing {owner:?} RPC descriptor for {function_name:?}")
        })
        .descriptor
        .rebind_owner_ura(callee_ura)
        .expect("fixture descriptor can rebind to callee");
    assert!(
        matches.next().is_none(),
        "fixture catalog has ambiguous {owner:?} RPC descriptor for {function_name:?}"
    );
    descriptor
}

fn catalog_owner_kind_for(callee_ura: &str) -> OwnerKind {
    let parsed = easynet_cli::core::ura::parse_ura(callee_ura)
        .unwrap_or_else(|err| panic!("fixture callee URA must parse: {callee_ura}: {err}"));
    match parsed.kind {
        easynet_cli::core::ura::URAKind::Device => OwnerKind::Device,
        easynet_cli::core::ura::URAKind::Hub => OwnerKind::Hub,
        other => panic!("fixture callee owner kind must be Device or Hub, got {other:?}"),
    }
}

/// Seed the hub read model with the same admitted projection shape that
/// `federation.advertise_abilities` persists after authority admission.
fn seed_health_projection(daemon: &TestDaemon) {
    let descriptor = fixture_descriptor(&daemon.catalog, DEVICE_URA, ABILITY_PUBLIC_NAME);
    let descriptor_version = descriptor.version.clone();
    let descriptor_revision = descriptor.descriptor_hash_prefixed();
    let schema_hash = descriptor.schema_hash_prefixed();
    let policy_hash = descriptor.access_policy_hash_prefixed();
    let description = descriptor.description.clone();
    let admission_action = descriptor.admission_action().as_str();
    let flags = json!({
        "read_only": true,
        "destructive": false,
        "idempotent": true,
        "streaming_only": false,
        "bidi_only": false
    });
    let receipt_semantics = json!({
        "kind": "operational"
    });
    let mode_geometry = json!([{
        "call_mode": "rpc",
        "descriptor_version": descriptor_version,
        "descriptor_revision": descriptor_revision,
        "admission_action": admission_action,
        "schema_hash": schema_hash,
        "policy_ref": policy_hash,
        "policy_hash": policy_hash,
        "description": description,
        "receipt_semantics": receipt_semantics,
        "input_fields": [],
        "flags": flags,
        "tags": ["class:unary", "mode:rpc"]
    }]);
    let callable_summary = json!({
        "public_name": ABILITY_PUBLIC_NAME,
        "description": description,
        "call_mode": "rpc",
        "receipt_semantics": receipt_semantics,
        "input_fields": [],
        "flags": flags,
        "mode_geometry": mode_geometry
    });
    let ability_summaries = json!([{
        "ability_ura": ABILITY_URA,
        "owner_ura": DEVICE_URA,
        "namespace": "observe",
        "local_name": "health",
        "descriptor_revision": descriptor_revision,
        "schema_hash": schema_hash,
        "policy_ref": policy_hash,
        "route_summary_ref": format!("route-ref::{ABILITY_URA}"),
        "tags": ["class:unary", "mode:rpc"],
        "callable_summary": callable_summary
    }]);
    let stored = daemon
        .ability_catalog_store
        .upsert_admitted_projection_json(json!({
            "owner_ura": DEVICE_URA,
            "host_device_ura": DEVICE_URA,
            "generation": 1,
            "projection_revision": 1,
            "projection_digest": "",
            "lease_expires_unix_ms": 4_102_444_800_000_i64,
            "ability_summaries": ability_summaries
        }))
        .expect("seed admitted owner projection");
    assert!(stored, "projection publication must be stored");
}

#[tokio::test]
async fn invoke_resolves_published_device_ability_to_session_dispatch() {
    let daemon = start_daemon().await;
    mark_owner_online(&daemon.presence);
    let mut client = InvocationClient::new(connect(&daemon.socket_path).await);

    seed_health_projection(&daemon);

    let payload = json!({ "marker": "resolve-before-invoke" });
    let status = tokio::time::timeout(
        STEP_TIMEOUT,
        client.invoke(invoke(
            &daemon.catalog,
            DEVICE_URA,
            ABILITY_PUBLIC_NAME,
            payload.clone(),
        )),
    )
    .await
    .expect("invoke did not time out")
    .expect_err("hub-mode route must select the device session dispatcher");
    assert_eq!(
        status.code(),
        tonic::Code::FailedPrecondition,
        "a resolved remote route without a pending dispatcher must fail at dispatch, not resolution"
    );
    assert!(
        status.message().contains("PendingDispatchMap"),
        "published route must reach the remote session dispatcher: {status}"
    );
}

#[tokio::test]
async fn invoke_reaches_dispatch_precondition_when_owner_online_for_known_ability() {
    let daemon = start_daemon().await;
    mark_owner_online(&daemon.presence);
    let mut client = InvocationClient::new(connect(&daemon.socket_path).await);

    // Owner is online and the descriptor is known; without a pending remote
    // dispatcher, resolve must get far enough to fail at dispatch precondition.
    let status = tokio::time::timeout(
        STEP_TIMEOUT,
        client.invoke(invoke(
            &daemon.catalog,
            DEVICE_URA,
            UNBOUND_ABILITY_PUBLIC_NAME,
            json!({}),
        )),
    )
    .await
    .expect("invoke did not time out")
    .expect_err("known online ability must reach remote dispatch precondition");

    assert_eq!(status.code(), tonic::Code::FailedPrecondition);
    assert!(
        status.message().contains("PendingDispatchMap"),
        "online known ability must fail at remote dispatch precondition, got: {status}"
    );
}

#[tokio::test]
async fn invoke_surfaces_typed_nxdomain_when_owner_offline() {
    let daemon = start_daemon().await;
    // Remote owner deliberately left offline (no presence insert).
    let mut client = InvocationClient::new(connect(&daemon.socket_path).await);

    let status = tokio::time::timeout(
        STEP_TIMEOUT,
        client.invoke(invoke(
            &daemon.catalog,
            REMOTE_DEVICE_URA,
            ABILITY_PUBLIC_NAME,
            json!({}),
        )),
    )
    .await
    .expect("invoke did not time out")
    .expect_err("offline owner must surface a typed resolver negative");

    assert_eq!(status.code(), tonic::Code::NotFound);
    assert!(
        status.message().contains("ROUTE_NEGATIVE")
            && status.message().contains("NEGATIVE_REASON_NXDOMAIN"),
        "offline owner must surface NXDOMAIN, got: {status}"
    );
}
