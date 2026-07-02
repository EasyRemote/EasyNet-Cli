// EasyNet CLI — PR-N3 commit N3-3.1 / N3-validation: cross-realm
// directory poll end-to-end
// =================================================================
//
// File: tests/cross_realm_directory_poll_e2e.rs
// Description: In-process integration test where daemon B's
//              `poll_once` dials daemon A's `federation.discover`
//              ability and B's federated_directory cell ends up
//              with A's view stamped with origin_realm=Some(realm-a).
//
// Why this test
// -------------
// The unit tests in `daemon::federation::directory` mock the
// `FederationClient`; they prove the data plane (rewrite chokepoint,
// per-peer view, cell semantics) but not the wire path. This test
// uses a real `DaemonInvocationService` for daemon A and an in-
// process forwarder for daemon B's outbound dial — exercising the
// federation.discover dispatch arm + admission gate + presence
// projection chain end-to-end without spawning binaries.
//
// What it validates from spec §八 acceptance:
// - (4) "new peer hub added via federated_peers SIGHUP appears in
//   <agent>.discover results within ~5s" — by triggering the poll
//   directly we verify the chain works; the 5s cadence is a
//   property of the spawned task wrapper, not the data flow.
//
// What it does NOT exercise:
// - Real TCP/TLS (the in-process forwarder bypasses the network).
// - Backend Go listDevices aggregation (N3-6).
// - 30s heartbeat / staleness detection (N3-2 FSM driven; would
//   need a streaming variant).
//
// Author: Silan.Hu <silan.hu@u.nus.edu>
// Copyright (c) 2026 EasyNet. All rights reserved.

#![cfg(feature = "axon-pb")]

use std::sync::Arc;

use async_trait::async_trait;

use easynet_axon::pb::axon::v1::invocation_server::Invocation;
use easynet_axon::pb::axon::v1::{InvokeRequest, InvokeResponse};
use easynet_cli::daemon::federation::client::{FederationClient, FederationClientError, HubUri};
use easynet_cli::daemon::federation::directory::{
    poll_once, DirectoryEntry, DirectoryView, SharedFederatedDirectoryView,
};
use easynet_cli::daemon::invocation::admission_facade::AdmissionFacade;
use easynet_cli::daemon::invocation::daemon_invocation_service::DaemonInvocationService;
use easynet_cli::services::presence_registry::PresenceRegistry;
use easynet_cli::services::realm_trust_anchor::RealmTrustAnchor;

/// In-process forwarder that delivers `forward_invoke` calls
/// straight to a target `DaemonInvocationService`. Stamps daemon
/// A's loopback URI as caller so admission's loopback bypass
/// admits without a signed envelope.
struct InProcessForwarder {
    peer: Arc<DaemonInvocationService>,
    peer_loopback_uri: String,
}

#[async_trait]
impl FederationClient for InProcessForwarder {
    async fn forward_invoke(
        &self,
        _target_hub: &HubUri,
        mut request: InvokeRequest,
    ) -> Result<InvokeResponse, FederationClientError> {
        request.envelope = Some(easynet_axon::pb::axon::v1::Envelope {
            caller: Some(easynet_axon::pb::axon::v1::AgentIdentity {
                ura: self.peer_loopback_uri.clone(),
                profile: "easynet-strict-v2".to_string(),
            }),
            ..Default::default()
        });
        let response = self
            .peer
            .invoke(tonic::Request::new(request))
            .await
            .map_err(|status| FederationClientError::InnerInvokeFailed {
                hub: "in-process-A".to_string(),
                status: format!("code={:?} message={}", status.code(), status.message()),
            })?;
        Ok(response.into_inner())
    }
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn poll_once_against_real_daemon_populates_cell_with_peer_directory() {
    // ── Daemon A: realm-a, federated_directory pre-populated ──
    // We pre-populate A's federated_directory with one peer
    // entry (representing what A would have learned from
    // polling some other peer hub). Daemon B then polls A's
    // `federation.discover`, gets that entry, and stamps A's
    // realm onto it via the §2.4 chokepoint.
    let daemon_a_loopback = easynet_cli::ura::hub_ura("realm-a");
    let daemon_a_admission = AdmissionFacade::new(
        Arc::new(RealmTrustAnchor::default()),
        Some(daemon_a_loopback.clone()),
    );
    let daemon_a_directory = SharedFederatedDirectoryView::default();
    // Pre-populate: pretend A already knows about a peer in
    // realm-c (a transitive view).
    let mut realm_c_view = DirectoryView::new("realm-c".to_string());
    realm_c_view.replace_entries(vec![DirectoryEntry {
        agent_ura: "easynet:///r/realm-c/device/device-X".to_string(),
        node_id: "device-X".to_string(),
        display_name: Some("third-party-device".to_string()),
        status: "active".to_string(),
        origin_realm: None, // chokepoint will stamp realm-c
        hub_endpoint: Some("https://hub-c.example:50443".to_string()),
        last_seen_unix_ms: Some(1_714_500_000_000),
    }]);
    let mut a_map = std::collections::BTreeMap::new();
    a_map.insert("realm-c".to_string(), Arc::new(realm_c_view));
    daemon_a_directory.replace(a_map);

    let daemon_a = Arc::new(
        DaemonInvocationService::new(Arc::new(PresenceRegistry::new()), daemon_a_admission)
            .with_session_realm("realm-a")
            .with_federated_directory_cell(daemon_a_directory.clone()),
    );

    // ── Daemon B: realm-b, empty directory + the forwarder ──
    let daemon_b_loopback = easynet_cli::ura::hub_ura("realm-b");
    let federation_client: Arc<dyn FederationClient> = Arc::new(InProcessForwarder {
        peer: Arc::clone(&daemon_a),
        peer_loopback_uri: daemon_a_loopback.clone(),
    });
    let daemon_b_directory = SharedFederatedDirectoryView::default();
    assert!(
        daemon_b_directory.snapshot().is_empty(),
        "daemon B starts with empty federated directory"
    );

    // Configure peers map: realm-a → in-process-A.
    let mut peers = std::collections::BTreeMap::new();
    peers.insert(
        "realm-a".to_string(),
        "https://hub-a.example:50443".to_string(),
    );

    // ── Drive: B polls A. Should pull A's federated view.
    let outcome = poll_once(
        federation_client.as_ref(),
        &peers,
        Some(daemon_b_loopback.as_str()),
        &daemon_b_directory,
    )
    .await;

    // ── Assert: poll succeeded; B's cell now has realm-a's
    // entry; the entry that came from A (which originally lived
    // in A's realm-c view, transitively) is stored under realm-a
    // in B's cell with origin_realm rewritten to "realm-a".
    //
    // This is INV-4 boundary behavior: the §2.4 chokepoint runs
    // on B's side and stamps "this came from A" regardless of
    // what A claimed. A future smarter polling target (per spec
    // §3.4 the backend uses `list_user_devices` not `discover`,
    // which avoids transitive re-broadcast) lands in N3-6.
    assert_eq!(outcome.successful_peers, vec!["realm-a".to_string()]);
    assert!(outcome.failed_peers.is_empty());

    let snap = daemon_b_directory.snapshot();
    let realm_a_view = snap.get("realm-a").expect("realm-a in B's cell");
    let entry = realm_a_view
        .lookup("easynet:///r/realm-c/device/device-X")
        .expect("device-X projected through");
    assert_eq!(
        entry.origin_realm.as_deref(),
        Some("realm-a"),
        "§2.4 chokepoint: B stamps `realm-a` on entries it received from A"
    );
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn discover_dispatch_returns_what_poll_populated() {
    // End-to-end happy path: daemon B polls daemon A, populates
    // its directory, then a CLI call to B's federation.discover
    // surfaces the entries.
    let daemon_a_loopback = easynet_cli::ura::hub_ura("realm-a");
    let daemon_a_admission = AdmissionFacade::new(
        Arc::new(RealmTrustAnchor::default()),
        Some(daemon_a_loopback.clone()),
    );
    let daemon_a_directory = SharedFederatedDirectoryView::default();
    let mut realm_c = DirectoryView::new("realm-c".to_string());
    realm_c.replace_entries(vec![DirectoryEntry {
        agent_ura: "easynet:///r/realm-c/device/device-Y".to_string(),
        node_id: "device-Y".to_string(),
        display_name: None,
        status: "active".to_string(),
        origin_realm: None,
        hub_endpoint: None,
        last_seen_unix_ms: None,
    }]);
    let mut a_map = std::collections::BTreeMap::new();
    a_map.insert("realm-c".to_string(), Arc::new(realm_c));
    daemon_a_directory.replace(a_map);

    let daemon_a = Arc::new(
        DaemonInvocationService::new(Arc::new(PresenceRegistry::new()), daemon_a_admission)
            .with_session_realm("realm-a")
            .with_federated_directory_cell(daemon_a_directory),
    );

    let daemon_b_loopback = easynet_cli::ura::hub_ura("realm-b");
    let federation_client: Arc<dyn FederationClient> = Arc::new(InProcessForwarder {
        peer: Arc::clone(&daemon_a),
        peer_loopback_uri: daemon_a_loopback.clone(),
    });
    let daemon_b_directory = SharedFederatedDirectoryView::default();
    let daemon_b = DaemonInvocationService::new(
        Arc::new(PresenceRegistry::new()),
        AdmissionFacade::new(
            Arc::new(RealmTrustAnchor::default()),
            Some(daemon_b_loopback.clone()),
        ),
    )
    .with_session_realm("realm-b")
    .with_federated_directory_cell(daemon_b_directory.clone());

    // Step 1: B polls A.
    let mut peers = std::collections::BTreeMap::new();
    peers.insert(
        "realm-a".to_string(),
        "https://hub-a.example:50443".to_string(),
    );
    poll_once(
        federation_client.as_ref(),
        &peers,
        Some(daemon_b_loopback.as_str()),
        &daemon_b_directory,
    )
    .await;

    // Step 2: A CLI-style call to B's federation.discover
    // surfaces the populated cell.
    let resp = daemon_b
        .invoke(tonic::Request::new(InvokeRequest {
            envelope: Some(easynet_axon::pb::axon::v1::Envelope {
                caller: Some(easynet_axon::pb::axon::v1::AgentIdentity {
                    ura: daemon_b_loopback.clone(),
                    profile: "easynet-strict-v2".to_string(),
                }),
                ..Default::default()
            }),
            function_name: "federation.discover".to_string(),
            arguments: br#"{}"#.to_vec(),
            ..Default::default()
        }))
        .await
        .expect("federation.discover ok");
    let body: easynet_cli::daemon::invocation::federation_wrappers::DiscoverResponse =
        serde_json::from_slice(&resp.into_inner().result).expect("DiscoverResponse");

    // The CLI should see at least one entry — device-Y from
    // realm-c, projected through A, stamped by B.
    assert!(
        !body.entries.is_empty(),
        "federation.discover must surface populated entries"
    );
    let device_y = body
        .entries
        .iter()
        .find(|e| e.agent_ura == "easynet:///r/realm-c/device/device-Y")
        .expect("device-Y in B's discover response");
    assert_eq!(
        device_y.origin_realm.as_deref(),
        Some("realm-a"),
        "B's view stamps realm-a as origin (received from A); transitive via §2.4"
    );
}
