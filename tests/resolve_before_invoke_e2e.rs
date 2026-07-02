//! Resolve-before-invoke end-to-end integration test (RFC-005 Phase C)
//! ===================================================================
//!
//! File: tests/resolve_before_invoke_e2e.rs
//! Purpose: X1 acceptance test for the CLI half of RFC-005. Proves
//!          that the daemon's unary `Invoke` product path runs
//!          `namespace.resolve` FIRST, dispatches only via the
//!          resolver-selected route, and surfaces a typed
//!          `ROUTE_NEGATIVE` (with the canonical `NegativeReason`)
//!          when no executable route exists — over a real tonic
//!          gRPC server on a tempfile UDS, with no mocks on the
//!          dispatch path.
//!
//! What this test exercises (the real bytes, not mocks)
//! ----------------------------------------------------
//! 1. A real `tonic::transport::Server` hosts a
//!    `DaemonInvocationService` wired with a real Axon
//!    `LocalRuntime` and a real `PresenceRegistry`, on a UDS.
//! 2. The device may publish its ability projection through the
//!    real `federation.advertise_abilities` wire path — the same
//!    RFC-005 owner-projection publication a production device
//!    uses. No catalog seam is poked directly.
//! 3. A unary `Invoke` of the device-local ability drives
//!    `resolve_local_rpc_route` → `DaemonRouteResolver` →
//!    `LocalDeviceAbility` FINAL_ROUTE → runtime dispatch, and the
//!    echoed payload round-trips back. The route is proven by the
//!    live LocalRuntime binding, not by DeviceAgent projection.
//! 4. A unary `Invoke` of an ability not bound in the same device's
//!    LocalRuntime surfaces `FailedPrecondition` carrying
//!    `ROUTE_NEGATIVE` + `NEGATIVE_REASON_NODATA`.
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
use easynet_cli::daemon::identity::self_identity::{SelfIdentity, SelfIdentityError};
use easynet_cli::daemon::invocation::admission_facade::AdmissionFacade;
use easynet_cli::daemon::invocation::daemon_invocation_service::DaemonInvocationService;
use easynet_cli::daemon::invocation::invocation_wire::ProtoEnvelope;
use easynet_cli::daemon::invocation::state::presence::PresenceRegistry;
use easynet_cli::daemon::trust::anchor::RealmTrustAnchor;
use easynet_cli::runtime::ability_dispatch::{
    AbilityAuthorityContext, AxonAbilityCatalog, LocalRpcHandler, OwnerKind,
};
use ed25519_dalek::{Signature, Signer as _, SigningKey, VerifyingKey};
use serde_json::json;
use tokio::net::UnixListener;
use tokio::sync::{mpsc, oneshot};
use tokio::task::JoinHandle;
use tokio_stream::wrappers::UnixListenerStream;
use tonic::transport::{Channel, Endpoint, Server, Uri};
use tonic::Request;

const REALM: &str = "test-realm";
const DEVICE_URI: &str = "easynet:///r/test-realm/device/device-a";
const REMOTE_DEVICE_URI: &str = "easynet:///r/test-realm/device/device-b";
const ABILITY_PUBLIC_NAME: &str = "test.echo";
const UNBOUND_ABILITY_PUBLIC_NAME: &str = "test.missing";
const ABILITY_URA: &str = "easynet:///r/test-realm/ability/device.device-a.test.echo";
/// The runtime registry key MUST equal the resolver's dispatch name
/// (the ability's public name), because the daemon dispatches strictly
/// via `SelectedInvokeRoute::dispatch_key()` — never via a tool-name or
/// owner-prefixed alias.
const ABILITY_REGISTRY_NAME: &str = ABILITY_PUBLIC_NAME;
const ADVERTISE_ABILITIES: &str = "federation.advertise_abilities";
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

/// Boot a daemon whose own identity is `DEVICE_URI`, with the echo
/// ability registered in a real `LocalRuntime`. Presence is left
/// empty so individual tests choose whether the owner is online.
async fn start_daemon() -> TestDaemon {
    let tempdir = tempfile::tempdir().expect("tempdir");
    let socket_path = tempdir.path().join("daemon.sock");

    let trust_path = tempdir.path().join("realm-trust.toml");
    let device_public_key_b64 = device_public_key_b64();
    let trust_toml = format!(
        r#"
[[trusted_agent]]
agent_ura = "{DEVICE_URI}"
public_key_b64 = "{device_public_key_b64}"
role = "device"
added_at_unix_ms = 0

[[trusted_agent]]
agent_ura = "{REMOTE_DEVICE_URI}"
public_key_b64 = "AAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAA="
role = "device"
added_at_unix_ms = 0
"#,
    );
    let mut f = std::fs::File::create(&trust_path).expect("create trust toml");
    f.write_all(trust_toml.as_bytes())
        .expect("write trust toml");
    drop(f);

    let trust_anchor = RealmTrustAnchor::try_load_strict(&trust_path).expect("load trust anchor");
    let presence = Arc::new(PresenceRegistry::new());
    let admission = AdmissionFacade::new(Arc::new(trust_anchor), Some(DEVICE_URI.to_string()));

    let runtime = LocalRuntime::new();
    let authority_context =
        AbilityAuthorityContext::for_device_authority_root(DEVICE_URI).expect("device authority");
    let mut catalog = AxonAbilityCatalog::new_with_runtime_and_authority_context(
        Arc::clone(&runtime),
        authority_context,
    );
    let echo_handler: LocalRpcHandler = Arc::new(Ok);
    catalog.register_rpc_with_owner(ABILITY_REGISTRY_NAME, OwnerKind::Device, echo_handler);

    let service = DaemonInvocationService::new(Arc::clone(&presence), admission)
        .with_session_realm(REALM)
        .with_local_runtime(runtime)
        .with_loopback_trusted(true);

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

/// Mark `DEVICE_URI` online in the presence registry. The dispatch
/// sender is never read on these paths (resolve only consults the
/// registry for liveness), so a throwaway channel is sufficient.
fn mark_owner_online(presence: &PresenceRegistry) {
    let (tx, _rx) = mpsc::channel(1);
    presence.insert(DEVICE_URI.to_string(), tx);
}

/// Build a unary `Invoke` of `function_name` against `callee_ura`,
/// with `args` as the JSON argument payload.
fn invoke(
    callee_ura: &str,
    function_name: &str,
    args: serde_json::Value,
) -> Request<InvokeRequest> {
    let arguments = args.to_string().into_bytes();
    let signer = device_signer();
    let descriptor_ref = format!(
        "{}@{}",
        easynet_cli::ura::owner_ability_ura(callee_ura, function_name)
            .expect("fixture ability URA"),
        easynet_cli::runtime::ability::DEFAULT_ABILITY_DESCRIPTOR_VERSION
    );
    Request::new(
        ProtoEnvelope::targeted(DEVICE_URI, callee_ura, callee_ura)
            .expect("valid invoke envelope")
            .signed_descriptor_ref_invoke_request(function_name, descriptor_ref, arguments, &signer)
            .expect("valid signed invoke request"),
    )
}

/// Publish the echo ability's owner projection through the real
/// `federation.advertise_abilities` wire path.
async fn publish_echo_projection(client: &mut InvocationClient<Channel>) {
    let request = invoke(
        DEVICE_URI,
        ADVERTISE_ABILITIES,
        json!({
            "owner_ura": DEVICE_URI,
            "host_device_ura": DEVICE_URI,
            "projection_revision": 1,
            "projection_digest": "sha256:test",
            "lease_expires_unix_ms": 4_102_444_800_000_i64,
            "ability_summaries": [{
                "ability_ura": ABILITY_URA,
                "owner_ura": DEVICE_URI,
                "namespace": "test",
                "local_name": "echo",
                "descriptor_revision": "sha256:descriptor",
                "policy_ref": "visibility:PUBLIC",
                "route_summary_ref": format!("route-ref::{ABILITY_URA}"),
                "tags": ["class:unary"],
                "callable_summary": {
                    "public_name": ABILITY_PUBLIC_NAME,
                    "description": "echo back the request payload",
                    "ability_class": "unary",
                    "input_fields": [],
                    "flags": {
                        "read_only": true,
                        "destructive": false,
                        "idempotent": true,
                        "streaming_only": false,
                        "bidi_only": false
                    }
                }
            }]
        }),
    );
    let resp = tokio::time::timeout(STEP_TIMEOUT, client.invoke(request))
        .await
        .expect("advertise_abilities did not time out")
        .expect("advertise_abilities returns Ok")
        .into_inner();
    let body: serde_json::Value =
        serde_json::from_slice(&resp.result).expect("advertise response is JSON");
    assert_eq!(body["ack"], true, "projection publication must be acked");
    assert_eq!(body["count"], 1, "exactly one ability published");
}

#[tokio::test]
async fn invoke_resolves_then_dispatches_published_device_ability() {
    let daemon = start_daemon().await;
    mark_owner_online(&daemon.presence);
    let mut client = InvocationClient::new(connect(&daemon.socket_path).await);

    publish_echo_projection(&mut client).await;

    let payload = json!({ "marker": "resolve-before-invoke" });
    let resp = tokio::time::timeout(
        STEP_TIMEOUT,
        client.invoke(invoke(DEVICE_URI, ABILITY_PUBLIC_NAME, payload.clone())),
    )
    .await
    .expect("invoke did not time out")
    .expect("resolver-selected dispatch returns Ok")
    .into_inner();

    let echoed: serde_json::Value =
        serde_json::from_slice(&resp.result).expect("echo result is JSON");
    assert_eq!(
        echoed, payload,
        "the published device ability must run and echo its payload"
    );
}

#[tokio::test]
async fn invoke_surfaces_typed_nodata_when_owner_online_but_ability_unpublished() {
    let daemon = start_daemon().await;
    mark_owner_online(&daemon.presence);
    let mut client = InvocationClient::new(connect(&daemon.socket_path).await);

    // Owner is online, but this ability is not bound in the live
    // LocalRuntime: resolve must return NODATA.
    let status = tokio::time::timeout(
        STEP_TIMEOUT,
        client.invoke(invoke(DEVICE_URI, UNBOUND_ABILITY_PUBLIC_NAME, json!({}))),
    )
    .await
    .expect("invoke did not time out")
    .expect_err("unbound ability must surface a typed resolver negative");

    assert_eq!(status.code(), tonic::Code::FailedPrecondition);
    assert!(
        status.message().contains("ROUTE_NEGATIVE")
            && status.message().contains("NEGATIVE_REASON_NODATA"),
        "online owner without the ability must surface NODATA, got: {status}"
    );
}

#[tokio::test]
async fn invoke_surfaces_typed_nxdomain_when_owner_offline() {
    let daemon = start_daemon().await;
    // Remote owner deliberately left offline (no presence insert).
    let mut client = InvocationClient::new(connect(&daemon.socket_path).await);

    let status = tokio::time::timeout(
        STEP_TIMEOUT,
        client.invoke(invoke(REMOTE_DEVICE_URI, ABILITY_PUBLIC_NAME, json!({}))),
    )
    .await
    .expect("invoke did not time out")
    .expect_err("offline owner must surface a typed resolver negative");

    assert_eq!(status.code(), tonic::Code::FailedPrecondition);
    assert!(
        status.message().contains("ROUTE_NEGATIVE")
            && status.message().contains("NEGATIVE_REASON_NXDOMAIN"),
        "offline owner must surface NXDOMAIN, got: {status}"
    );
}
