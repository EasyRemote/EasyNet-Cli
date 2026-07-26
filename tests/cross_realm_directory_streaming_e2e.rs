// EasyNet CLI — PR-N3 N3-streaming-5: in-process 2-daemon
// streaming directory federation e2e
// =================================================================
//
// File: tests/cross_realm_directory_streaming_e2e.rs
// Description: End-to-end test that drives the full streaming
//              chain — daemon A's PresenceRegistry events flow
//              through its `subscribe_directory_v2` server stream,
//              an in-process forwarder delivers the bytes to daemon
//              B's `run_per_peer_supervisor`, the §2.4 chokepoint
//              stamps `origin_realm`, daemon B's federated_directory
//              cell reflects the entry within the test's bounded
//              wait window.
//
// Why in-process rather than spawned-binary
// ----------------------------------------
// The data-plane chain (server emit → in-process forwarder →
// consumer + chokepoint + cell publish) is what this test
// validates. The transport (real tonic over TCP/TLS, the
// channel cache, the breaker) is exercised by
// `cross_hub_two_daemon_real_tls_e2e.rs` (PR-N1 commit 7/N) for
// the unary surface; the streaming surface inherits the same
// channel setup. An in-process forwarder isolates the streaming
// chain logic from the TLS plumbing so a regression in either
// half lands cleanly.
//
// What this validates from spec §八 acceptance:
// - (3) "machine B SIGTERM flips status to stale within ~30s" —
//   exercised by the Online → Upsert path here; the Remove path
//   (the inverse) is exercised by the streaming-1 unit test
//   already.
// - (4) "new peer hub appears in <agent>.discover within ~5s" —
//   exercised end-to-end: the supervisor opens a stream, the
//   server emits the initial Snapshot, the consumer applies
//   it, the cell publishes; all within the ~50ms in-process
//   round-trip budget.
//
// Author: Silan.Hu <silan.hu@u.nus.edu>
// Copyright (c) 2026 EasyNet. All rights reserved.

#![cfg(feature = "axon-pb")]

use std::path::PathBuf;
use std::sync::{Arc, Mutex, MutexGuard, OnceLock};
use std::time::Duration;

use async_trait::async_trait;
use base64::{engine::general_purpose::STANDARD as BASE64_STANDARD, Engine as _};
use futures::StreamExt;

#[path = "support/runtime_fixture.rs"]
mod runtime_fixture;

use axon_sdk::pb::axon::v1::invocation_server::Invocation;
use axon_sdk::pb::axon::v1::{InvokeRequest, InvokeResponse, InvokeServerStreamRequest};
use easynet_cli::daemon::ability::dispatch::AbilityAuthorityContext;
use easynet_cli::daemon::federation::client::{
    DirectoryEventStream, FederationClient, FederationClientError, HubEndpoint,
};
use easynet_cli::daemon::federation::directory::{
    run_per_peer_supervisor, DirectoryEvent, FederatedDirectorySubscriptionIssuer,
    SharedFederatedDirectoryView,
};
use easynet_cli::daemon::identity::self_identity::{CanonicalSigner, SelfIdentityError};
use easynet_cli::daemon::invocation::admission::admission_facade::AdmissionFacade;
use easynet_cli::daemon::invocation::admission::decision::{
    AccessAction, PrincipalKind, TokenClass,
};
use easynet_cli::daemon::invocation::admission::grant_matcher::{
    PermissionEffect, PermissionGrant, PermissionGrantLifetime, PermissionGrantState,
};
use easynet_cli::daemon::invocation::bidi::state::presence::PresenceRegistry;
use easynet_cli::daemon::invocation::dispatch::daemon_invocation_service::DaemonInvocationService;
use easynet_cli::daemon::invocation::dispatch::federation_wrappers::ABILITY_FEDERATION_SUBSCRIBE_DIRECTORY_V2;
use easynet_cli::daemon::persistence::access_control::AccessControlStoreRegistry;
use easynet_cli::daemon::trust::anchor::{
    RealmTrustAnchor, TrustedAgent, TrustedAgentRole, TrustedPrincipalOwner,
};
use easynet_cli::daemon::trust::cell::SharedTrustAnchor;
use easynet_cli::daemon::trust::key_resolver::RealmTrustAnchorKeyResolver;
use ed25519_dalek::{Signature, Signer as _, SigningKey, VerifyingKey};

const DIRECTORY_SUBSCRIPTION_SIGNING_SEED: [u8; 32] = [0x5e; 32];
const UPSTREAM_OWNER_USER_ID: &str = "realm-a-operator";

struct TestHomeGuard {
    _lock: MutexGuard<'static, ()>,
    root: PathBuf,
    previous_home: Option<String>,
}

impl TestHomeGuard {
    fn new() -> Self {
        static HOME_LOCK: OnceLock<Mutex<()>> = OnceLock::new();
        static HOME_SEQUENCE: std::sync::atomic::AtomicU64 = std::sync::atomic::AtomicU64::new(0);

        let lock = HOME_LOCK
            .get_or_init(|| Mutex::new(()))
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner());
        let sequence = HOME_SEQUENCE.fetch_add(1, std::sync::atomic::Ordering::Relaxed);
        let root = std::env::temp_dir().join(format!(
            "easynet-cross-realm-directory-{}-{sequence}",
            std::process::id()
        ));
        std::fs::create_dir_all(&root).expect("create isolated integration-test HOME");
        let previous_home = std::env::var("HOME").ok();
        std::env::set_var("HOME", &root);
        Self {
            _lock: lock,
            root,
            previous_home,
        }
    }
}

impl Drop for TestHomeGuard {
    fn drop(&mut self) {
        match self.previous_home.take() {
            Some(previous) => std::env::set_var("HOME", previous),
            None => std::env::remove_var("HOME"),
        }
        let _ = std::fs::remove_dir_all(&self.root);
    }
}

struct TestCanonicalSigner {
    owner_ura: String,
    signing_key: SigningKey,
}

#[async_trait]
impl CanonicalSigner for TestCanonicalSigner {
    fn owner_ura(&self) -> &str {
        &self.owner_ura
    }

    async fn sign_canonical(&self, canonical_bytes: &[u8]) -> Result<Signature, SelfIdentityError> {
        Ok(self.signing_key.sign(canonical_bytes))
    }

    fn signing_public_key(&self) -> Result<VerifyingKey, SelfIdentityError> {
        Ok(self.signing_key.verifying_key())
    }
}

fn test_request_issuer(realm: &str) -> FederatedDirectorySubscriptionIssuer {
    let signer: Arc<dyn CanonicalSigner> = Arc::new(TestCanonicalSigner {
        owner_ura: easynet_cli::core::ura::hub_ura(realm),
        signing_key: SigningKey::from_bytes(&DIRECTORY_SUBSCRIPTION_SIGNING_SEED),
    });
    FederatedDirectorySubscriptionIssuer::new(signer).expect("test request issuer")
}

fn upstream_invocation_attempt_ledger_path(realm: &str, trusted_peer_realm: &str) -> PathBuf {
    let home = PathBuf::from(std::env::var_os("HOME").expect("isolated test HOME"));
    let dir = home.join(".easynet").join("invocation-attempt-ledgers");
    std::fs::create_dir_all(&dir).expect("create upstream invocation attempt ledger directory");
    dir.join(format!(
        "cross-realm-directory-stream-{realm}-{trusted_peer_realm}.jsonl"
    ))
}

async fn upstream_daemon(
    realm: &str,
    trusted_peer_realm: &str,
    presence: Arc<PresenceRegistry>,
) -> Arc<DaemonInvocationService> {
    let owner_ura = easynet_cli::core::ura::hub_ura(realm);
    let trusted_peer_ura = easynet_cli::core::ura::hub_ura(trusted_peer_realm);
    let owner_user_ura = easynet_cli::core::ura::user_ura(realm, UPSTREAM_OWNER_USER_ID);
    let subject_ura = easynet_cli::core::ura::resource_dot_ura(
        trusted_peer_realm,
        "hub.federation",
        &format!("directory/{realm}"),
    );
    let ability_ura = easynet_cli::core::ura::owner_ability_ura(
        &owner_ura,
        ABILITY_FEDERATION_SUBSCRIBE_DIRECTORY_V2,
    )
    .expect("directory subscription ability URA");
    let mut trust_anchor = RealmTrustAnchor::default();
    trust_anchor
        .append_agent(TrustedAgent {
            agent_ura: trusted_peer_ura.clone(),
            public_key_b64: BASE64_STANDARD.encode(
                SigningKey::from_bytes(&DIRECTORY_SUBSCRIPTION_SIGNING_SEED)
                    .verifying_key()
                    .to_bytes(),
            ),
            role: TrustedAgentRole::Hub,
            added_at_unix_ms: 1_700_000_000_000,
            origin_realm: Some(trusted_peer_realm.to_string()),
            hub_endpoint: Some(format!("https://{trusted_peer_realm}.example:50443")),
            tls_ca_pem_path: None,
        })
        .expect("append trusted peer Hub");
    trust_anchor
        .upsert_principal_owner(TrustedPrincipalOwner {
            principal_ura: owner_ura.clone(),
            owner_user_id: UPSTREAM_OWNER_USER_ID.to_string(),
            owner_ura: owner_user_ura.clone(),
            added_at_unix_ms: 1_700_000_000_000,
        })
        .expect("bind upstream Hub to an accountable owner");
    let access_control_stores = Arc::new(AccessControlStoreRegistry::default());
    access_control_stores
        .with_store(UPSTREAM_OWNER_USER_ID, |store| {
            store.create_grant(
                PermissionGrant {
                    grant_id: "cross-realm-directory-stream".to_string(),
                    owner_user_id: UPSTREAM_OWNER_USER_ID.to_string(),
                    principal_kind: PrincipalKind::Token,
                    principal_id: trusted_peer_ura.clone(),
                    token_id: Some(trusted_peer_ura),
                    token_class: Some(TokenClass::HubLink),
                    callee_ura: Some(owner_ura.clone()),
                    subject_ura_pattern: Some(subject_ura),
                    ability_ura_pattern: Some(ability_ura),
                    actions: vec![AccessAction::Stream],
                    constraints: None,
                    effect: PermissionEffect::Allow,
                    lifetime: PermissionGrantLifetime::Session,
                    state: PermissionGrantState::Active,
                    expires_at: None,
                    review_required_after: None,
                    last_reviewed_at: None,
                    last_used_at: None,
                    created_by: owner_user_ura.clone(),
                    created_at: "2026-07-18T00:00:00Z".to_string(),
                    updated_at: None,
                    revoked_at: None,
                    reason: Some("cross-realm directory integration fixture".to_string()),
                },
                &owner_user_ura,
            )
        })
        .expect("open isolated directory-subscription policy store")
        .expect("grant peer Hub directory stream access");
    let trust_anchor = SharedTrustAnchor::new(Arc::new(trust_anchor));
    let runtime_assembly = runtime_fixture::daemon_runtime_with_key_resolver(Arc::new(
        RealmTrustAnchorKeyResolver::new(trust_anchor.clone()),
    ));
    let authority_context = AbilityAuthorityContext::for_realm_authority_root(&owner_ura)
        .expect("upstream Hub authority context");
    let agents = easynet_cli::daemon::persistence::agent_registry::AgentRegistry::default();
    let mut catalog_config =
        easynet_cli::daemon::ability::catalog::RegistryBuildConfig::new_with_authority_context(
            easynet_cli::daemon::ability::catalog::RegistryBuildServices::fresh()
                .with_access_control_stores(Arc::clone(&access_control_stores)),
            &agents,
            authority_context,
        );
    catalog_config.local_runtime = Some(runtime_assembly.runtime());
    let catalog =
        easynet_cli::daemon::ability::catalog::build_registry_with_services_result(catalog_config)
            .expect("assemble production-shaped upstream ability catalog")
            .catalog;
    let admission = AdmissionFacade::with_trust_anchor_cell(trust_anchor, Some(owner_ura.clone()))
        .with_access_control_stores(access_control_stores)
        .with_ability_catalog(Arc::clone(&catalog));
    let service = DaemonInvocationService::new(presence, admission)
        .with_session_realm(realm)
        .with_local_ability_catalog(catalog)
        .with_daemon_runtime(runtime_assembly)
        .with_invocation_attempt_ledger_path(upstream_invocation_attempt_ledger_path(
            realm,
            trusted_peer_realm,
        ))
        .expect("open upstream invocation attempt audit ledger");
    service
        .register_daemon_stream_routes(&owner_ura)
        .await
        .expect("register upstream stream routes before exposing service");
    Arc::new(service)
}

/// In-process forwarder. `subscribe_directory_v2` opens an
/// `invoke_stream` call against the target daemon, then JSON-
/// decodes each yielded `InvokeStreamChunk` payload as a
/// `DirectoryEvent` and forwards it to the consumer.
struct InProcessStreamingForwarder {
    peer_daemon: Arc<DaemonInvocationService>,
}

#[async_trait]
impl FederationClient for InProcessStreamingForwarder {
    async fn invoke(
        &self,
        _target_hub_endpoint: &HubEndpoint,
        _request: InvokeRequest,
    ) -> Result<InvokeResponse, FederationClientError> {
        Err(FederationClientError::Unimplemented(
            "not exercised in this test",
        ))
    }

    async fn subscribe_directory_v2(
        &self,
        _target_hub_endpoint: &HubEndpoint,
        request: InvokeServerStreamRequest,
    ) -> Result<DirectoryEventStream, FederationClientError> {
        let response = self
            .peer_daemon
            .invoke_stream(tonic::Request::new(request))
            .await
            .map_err(|status| FederationClientError::InnerInvokeFailed {
                endpoint: "in-process".to_string(),
                status: format!("code={:?} message={}", status.code(), status.message()),
            })?;
        let inner = response.into_inner();
        // Wrap the daemon's `Stream<Item = Result<InvokeStreamChunk, Status>>`
        // with a JSON-decode + filter step so the consumer sees
        // a clean `Stream<Item = DirectoryEvent>`.
        let events = inner.filter_map(|item| async move {
            match item {
                Ok(chunk) => serde_json::from_slice::<DirectoryEvent>(&chunk.payload).ok(),
                Err(_) => None,
            }
        });
        Ok(Box::pin(events))
    }
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn streaming_chain_propagates_presence_event_to_peer_cell() {
    let _home = TestHomeGuard::new();
    // ── Daemon A: realm-a, hosts the upstream PresenceRegistry ──
    let daemon_a_presence = Arc::new(PresenceRegistry::new());
    let daemon_a = upstream_daemon("realm-a", "realm-b", Arc::clone(&daemon_a_presence)).await;

    // ── Daemon B: realm-b, runs the per-peer supervisor pulling A's stream ──
    let daemon_b_directory = SharedFederatedDirectoryView::default();
    let federation_client: Arc<dyn FederationClient> = Arc::new(InProcessStreamingForwarder {
        peer_daemon: Arc::clone(&daemon_a),
    });

    let (cancel_tx, cancel_rx) = tokio::sync::oneshot::channel();
    let cell_for_task = daemon_b_directory.clone();
    let supervisor_task = tokio::spawn(async move {
        run_per_peer_supervisor(
            "realm-a".to_string(),
            "https://hub-a.example:50443".to_string(),
            test_request_issuer("realm-b"),
            federation_client,
            cell_for_task,
            cancel_rx,
        )
        .await;
    });

    // Give the supervisor a moment to open its stream and
    // consume daemon A's initial empty Snapshot.
    tokio::time::sleep(Duration::from_millis(50)).await;

    // ── Drive: insert a presence entry on daemon A ──
    // The registry's broadcast pump pushes a PresenceEvent::Online,
    // daemon A's subscribe_directory_v2 emits an Upsert frame,
    // the in-process forwarder delivers it to daemon B's
    // supervisor, the §2.4 chokepoint stamps origin_realm, the
    // cell publishes.
    let target_ura = "easynet:///r/realm-a/device/device-X";
    let (tx, _rx) = tokio::sync::mpsc::channel::<
        Result<
            easynet_cli::daemon::invocation::bidi::state::presence::DispatchFrame,
            tonic::Status,
        >,
    >(8);
    daemon_a_presence
        .insert(target_ura.to_string(), tx)
        .expect("canonical presence key");

    // ── Assert: daemon B's cell shows the entry within a
    // bounded window. The data-plane round-trip is in-process
    // and bounded by tokio scheduling; 1s is generous so a real
    // bug surfaces rather than a flaky timeout.
    let mut found = false;
    for _ in 0..40 {
        let snap = daemon_b_directory.snapshot();
        if let Some(view) = snap.get("realm-a") {
            if let Some(entry) = view.lookup(target_ura) {
                assert_eq!(
                    entry.origin_realm.as_deref(),
                    Some("realm-a"),
                    "§2.4 chokepoint must stamp the receiving-side realm"
                );
                assert_eq!(entry.status, "active");
                found = true;
                break;
            }
        }
        tokio::time::sleep(Duration::from_millis(25)).await;
    }
    assert!(
        found,
        "presence event did not propagate through the streaming chain to daemon B's cell"
    );

    // Cancel the supervisor and wait for shutdown.
    let _ = cancel_tx.send(());
    let _ = tokio::time::timeout(Duration::from_secs(5), supervisor_task).await;
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn streaming_chain_propagates_presence_remove() {
    let _home = TestHomeGuard::new();
    // Same harness; this time after the Online propagates, we
    // remove the entry on daemon A and assert the Remove frame
    // flows through to daemon B's cell.
    let daemon_a_presence = Arc::new(PresenceRegistry::new());
    let daemon_a = upstream_daemon("realm-a", "realm-b", Arc::clone(&daemon_a_presence)).await;

    let daemon_b_directory = SharedFederatedDirectoryView::default();
    let federation_client: Arc<dyn FederationClient> = Arc::new(InProcessStreamingForwarder {
        peer_daemon: Arc::clone(&daemon_a),
    });

    let (cancel_tx, cancel_rx) = tokio::sync::oneshot::channel();
    let cell_for_task = daemon_b_directory.clone();
    let supervisor_task = tokio::spawn(async move {
        run_per_peer_supervisor(
            "realm-a".to_string(),
            "https://hub-a.example:50443".to_string(),
            test_request_issuer("realm-b"),
            federation_client,
            cell_for_task,
            cancel_rx,
        )
        .await;
    });

    // Insert a device on A; wait for B's cell to reflect.
    let target_ura = "easynet:///r/realm-a/device/disappearing";
    let (tx, _rx) = tokio::sync::mpsc::channel::<
        Result<
            easynet_cli::daemon::invocation::bidi::state::presence::DispatchFrame,
            tonic::Status,
        >,
    >(8);
    daemon_a_presence
        .insert(target_ura.to_string(), tx)
        .expect("canonical presence key");
    let mut observed_online = false;
    for _ in 0..40 {
        let snap = daemon_b_directory.snapshot();
        if snap
            .get("realm-a")
            .and_then(|v| v.lookup(target_ura))
            .is_some()
        {
            observed_online = true;
            break;
        }
        tokio::time::sleep(Duration::from_millis(25)).await;
    }
    assert!(
        observed_online,
        "presence entry must propagate before the test can assert its removal"
    );

    // Remove on A → Remove frame propagates to B → entry
    // disappears from B's view.
    daemon_a_presence.remove(
        target_ura,
        easynet_cli::daemon::invocation::bidi::state::presence::OfflineReason::AdminRevoked,
    );
    let mut removed = false;
    for _ in 0..40 {
        let snap = daemon_b_directory.snapshot();
        if snap
            .get("realm-a")
            .and_then(|v| v.lookup(target_ura))
            .is_none()
        {
            removed = true;
            break;
        }
        tokio::time::sleep(Duration::from_millis(25)).await;
    }
    assert!(
        removed,
        "presence remove did not propagate; daemon B still sees the entry"
    );

    let _ = cancel_tx.send(());
    let _ = tokio::time::timeout(Duration::from_secs(5), supervisor_task).await;
}
