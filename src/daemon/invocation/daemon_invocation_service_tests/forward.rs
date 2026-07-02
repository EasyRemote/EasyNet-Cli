use super::*;

#[test]
fn with_federation_client_attaches_client_field() {
    use crate::daemon::federation::client::CrossHubDialer;

    let svc = make_service();
    assert!(svc.federation.client.is_none());

    let dialer = Arc::new(CrossHubDialer::new(Arc::new(RealmTrustAnchor::default())));
    let svc = svc.with_federation_client(dialer.clone() as Arc<dyn FederationClient>);
    assert!(svc.federation.client.is_some());
}

#[test]
fn with_federated_peers_attaches_map_field() {
    let svc = make_service();
    assert!(svc.federation.peers.snapshot().is_empty());

    let mut peers = BTreeMap::new();
    peers.insert(
        "peer-realm".to_string(),
        "https://peer-hub.example:50443".to_string(),
    );
    let svc = svc.with_federated_peers(peers);
    let snap = svc.federation.peers.snapshot();
    assert_eq!(snap.len(), 1);
    assert_eq!(
        snap.get("peer-realm").map(String::as_str),
        Some("https://peer-hub.example:50443")
    );
}

#[test]
fn federated_peers_cell_picks_up_replace_without_service_rebuild() {
    // PR-N1 commit 10/N: the SIGHUP reload task calls
    // `cell.replace(new_map)` on TOML re-parse success. The
    // dispatcher's per-call `snapshot()` must see the new
    // map without a `DaemonInvocationService` rebuild.
    use crate::daemon::federation::peers::SharedFederatedPeers;

    let cell = SharedFederatedPeers::default();
    let svc = make_service().with_federated_peers_cell(cell.clone());
    assert!(svc.federation.peers.snapshot().is_empty());

    let mut next = BTreeMap::new();
    next.insert(
        "hot-reloaded-realm".to_string(),
        "https://hot:50443".to_string(),
    );
    cell.replace(next);

    // Same `svc` instance, but the cell snapshot now has
    // the new entry — no rebuild required.
    let snap = svc.federation.peers.snapshot();
    assert_eq!(snap.len(), 1);
    assert!(snap.contains_key("hot-reloaded-realm"));
}

// ── PR-N1 commit 3b/N: realm-aware forward_invoke tests ──

// ── PR-1 commit 7/9 (LB-56) — self-targeted local dispatch ─────────

#[tokio::test]
async fn forward_invoke_self_target_runs_locally_via_axon_runtime() {
    // PR-1 commit 7/9 acceptance: when an inbound
    // `federation.forward_invoke` call's `target_ura` matches
    // THIS daemon's own canonical URA AND a local
    // Axon LocalRuntime is wired, the runtime MUST execute the
    // inner ability locally (no session push, no peer delegation)
    // and return the JSON result bytes inline
    // in `ForwardInvokeResponse.result_bytes`.
    //
    // This is the LB-56 §〇 production flow: hub-A → hub-B
    // peer delegation -> hub-B receives forward_invoke with
    // target_ura = hub-B's own URA (peer hub IS the target,
    // not a device on its bidi). Without this fall-through
    // the call surfaces target_offline because hub-B does
    // not register its own URA in its PresenceRegistry.
    // Build a minimal runtime with one ability that returns
    // a sentinel object so we can prove the bytes came from
    // the local runtime and not a daemon-internal stub.
    //
    // Register under the BARE registry key (`demo.echo`, not
    // `device.demo.echo`). Device-owned abilities enter
    // `AxonAbilityCatalog` un-prefixed (`fs.read`, `observe.health`,
    // …) and `sync_runtime_ability` mirrors that bare key into the
    // LocalRuntime verbatim, so the selected route's device-local
    // dispatch key is also bare. This mirrors the production
    // convention and the sibling `observe.health` quota test.
    let rt = runtime_with_json_echo(
        TEST_DAEMON_URI,
        "demo.echo",
        "MARKER-C9-1",
        "self-target-fallthrough-fired",
    )
    .await;

    let svc = make_service()
        .with_session_realm("test-realm")
        .with_local_runtime(Arc::clone(&rt));
    publish_test_route(&svc, TEST_DAEMON_URI, "demo.echo");

    let response = svc
        .unary_dispatcher()
        .dispatch_federation_forward_invoke(
            None,
            &forward_invoke_args_for_ability(
                TEST_DAEMON_URI,
                "demo.echo",
                serde_json::json!({"k": "v"}),
            ),
        )
        .await
        .expect("self-target dispatch returns Ok with result_bytes inline");

    let body = response.into_inner();
    let parsed: federation_wrappers::ForwardInvokeResponse =
        serde_json::from_slice(&body.result).expect("body decodes");
    assert_eq!(
        parsed.correlation_call_id, "test-call-id-1",
        "correlation_call_id must round-trip through self-target arm"
    );
    assert!(
        !parsed.result_bytes.is_empty(),
        "self-target dispatch fills result_bytes (no async reverse-channel reply needed)"
    );

    let result_value: serde_json::Value =
        serde_json::from_slice(&parsed.result_bytes).expect("result_bytes is JSON");
    assert_eq!(
        result_value.get("MARKER-C9-1").and_then(|v| v.as_str()),
        Some("self-target-fallthrough-fired"),
        "result_bytes must come from the AxonAbilityCatalog handler, \
         not a daemon-internal fallback"
    );
    assert_eq!(
        result_value
            .get("echoed_args")
            .and_then(|v| v.get("k"))
            .and_then(|v| v.as_str()),
        Some("v"),
        "inner args must round-trip through the dispatcher's normalized_args path"
    );
}

#[tokio::test]
async fn forward_invoke_self_target_scopes_agent_target_ability() {
    use crate::persistence::local_agents::{save, upsert_hosted_agent, LocalAgentsFile};

    let _hg = crate::cli::test_support::HomeGuard::new();
    let target_ura = "easynet:///r/test-realm/agent/user.alice";
    let mut local = LocalAgentsFile {
        host_device_agent_ura: "easynet:///r/test-realm/device/dev-1".to_string(),
        hosted_agents: Vec::new(),
    };
    upsert_hosted_agent(&mut local, "llm", "alice", target_ura);
    save(&local).expect("seed local-agents.json");

    let rt = runtime_with_json_echo(
        target_ura,
        "alice.chat",
        "MARKER-AGENT-SCOPE",
        "agent-scope-fired",
    )
    .await;

    let svc = make_service()
        .with_session_realm("test-realm")
        .with_local_runtime(Arc::clone(&rt));
    publish_test_route(&svc, target_ura, "alice.chat");

    let response = svc
        .unary_dispatcher()
        .dispatch_federation_forward_invoke(
            None,
            &forward_invoke_args_for_ability_ura(
                target_ura,
                "easynet:///r/test-realm/ability/user.alice.chat",
                serde_json::json!({"prompt": "hi"}),
            ),
        )
        .await
        .expect("self-target agent dispatch must scope and run locally");

    let body = response.into_inner();
    let parsed: federation_wrappers::ForwardInvokeResponse =
        serde_json::from_slice(&body.result).expect("body decodes");
    let result_value: serde_json::Value =
        serde_json::from_slice(&parsed.result_bytes).expect("result_bytes is JSON");
    assert_eq!(
        result_value
            .get("MARKER-AGENT-SCOPE")
            .and_then(|v| v.as_str()),
        Some("agent-scope-fired"),
        "bare `chat` must dispatch as `alice.chat` for agent URA self-targets"
    );
}

/// Contract update (hosted-agent addressing, 2026-06-11): an
/// agent-owned ability forwarded at a device target is no longer
/// vetoed by local string equality — whether the device hosts the
/// agent is the RESOLVER's call. An unhosted agent therefore fails
/// at resolution with a precise route negative instead of a local
/// InvalidArgument.
#[tokio::test]
async fn forward_invoke_agent_ability_unhosted_by_target_fails_at_resolution() {
    let target_ura = TEST_DAEMON_URI;
    let rt = runtime_with_json_echo(
        target_ura,
        "observe.health",
        "MARKER-DEVICE-SCOPE",
        "device-scope-fired",
    )
    .await;
    let svc = make_service()
        .with_session_realm("test-realm")
        .with_local_runtime(Arc::clone(&rt));

    let err = svc
        .unary_dispatcher()
        .dispatch_federation_forward_invoke(
            None,
            &forward_invoke_args_for_ability_ura(
                target_ura,
                "easynet:///r/test-realm/ability/user.alice.chat",
                serde_json::json!({"prompt": "hi"}),
            ),
        )
        .await
        .expect_err("unhosted agent ability must fail at resolution");
    assert_eq!(err.code(), tonic::Code::FailedPrecondition);
    assert!(
        err.message().contains(ROUTE_NEGATIVE_CODE),
        "error must be a route negative, got: {}",
        err.message()
    );
}

#[tokio::test]
async fn forward_invoke_rejects_bare_device_agent_alias() {
    let alias_target = "easynet:///r/test-realm/agent/dev-B";
    let canonical_target = "easynet:///r/test-realm/device/dev-B";
    let canonical_ability = "easynet:///r/test-realm/ability/device.dev-B.observe.health";
    let presence = Arc::new(PresenceRegistry::new());

    let (alias_tx, alias_rx) = tokio::sync::mpsc::channel(1);
    drop(alias_rx);
    presence.insert(alias_target.to_string(), alias_tx);
    let (canonical_tx, canonical_rx) = tokio::sync::mpsc::channel(1);
    drop(canonical_rx);
    presence.insert(canonical_target.to_string(), canonical_tx);

    let admission = AdmissionFacade::new(
        Arc::new(RealmTrustAnchor::default()),
        Some(TEST_DAEMON_URI.to_string()),
    );
    let svc = DaemonInvocationService::new(presence, admission)
        .with_session_realm("test-realm")
        .with_pending(Arc::new(PendingDispatchMap::new()));

    let err = svc
        .unary_dispatcher()
        .dispatch_federation_forward_invoke(
            None,
            &forward_invoke_args_for_ability_ura(
                alias_target,
                canonical_ability,
                serde_json::json!({}),
            ),
        )
        .await
        .expect_err("legacy device-as-agent target alias must not be repaired");

    assert_eq!(err.code(), tonic::Code::InvalidArgument);
    assert!(
        err.message().contains("does not belong to target"),
        "error must cite owner mismatch, got: {}",
        err.message()
    );
}

#[tokio::test]
async fn forward_invoke_local_hub_ura_runs_locally_via_axon_runtime() {
    // Device-mode escalation targets the local realm's hub URA,
    // not the hub host's device URA. The hub daemon must treat
    // `easynet:///r/<realm>/hub` as self-targeted even though
    // `AdmissionFacade.daemon_ura()` still carries the host
    // device URA from credentials.json.
    let rt = runtime_with_json_echo(
        &crate::ura::hub_ura("test-realm"),
        "demo.echo",
        "MARKER-C9-HUB",
        "local-hub-self-target-fired",
    )
    .await;

    let svc = make_service()
        .with_session_realm("test-realm")
        .with_local_runtime(Arc::clone(&rt));
    publish_test_route(&svc, &crate::ura::hub_ura("test-realm"), "demo.echo");

    let response = svc
        .unary_dispatcher()
        .dispatch_federation_forward_invoke(
            None,
            &forward_invoke_args_for_ability(
                &crate::ura::hub_ura("test-realm"),
                "demo.echo",
                serde_json::json!({"k": "hub"}),
            ),
        )
        .await
        .expect("local hub URA must hit the self-target dispatcher");

    let body = response.into_inner();
    let parsed: federation_wrappers::ForwardInvokeResponse =
        serde_json::from_slice(&body.result).expect("body decodes");
    let result_value: serde_json::Value =
        serde_json::from_slice(&parsed.result_bytes).expect("result_bytes is JSON");
    assert_eq!(
        result_value.get("MARKER-C9-HUB").and_then(|v| v.as_str()),
        Some("local-hub-self-target-fired"),
    );
    assert_eq!(
        result_value
            .get("echoed_args")
            .and_then(|v| v.get("k"))
            .and_then(|v| v.as_str()),
        Some("hub"),
    );
}

#[tokio::test]
async fn forward_invoke_self_target_without_local_runtime_rejects_explicitly() {
    // Guard: when Axon LocalRuntime is not wired, self-targeted
    // dispatch must fail explicitly instead of falling through to
    // PresenceRegistry and reporting a misleading target_offline.
    let svc = make_service().with_session_realm("test-realm");
    publish_test_route(&svc, TEST_DAEMON_URI, "observe.health");

    let err = svc
        .unary_dispatcher()
        .dispatch_federation_forward_invoke(None, &forward_invoke_args(TEST_DAEMON_URI))
        .await
        .expect_err("no LocalRuntime => explicit wiring error");
    assert_eq!(err.code(), tonic::Code::FailedPrecondition);
    assert!(
        err.message().contains("Axon LocalRuntime is not wired"),
        "expected LocalRuntime wiring error, got: {err}"
    );
}

#[tokio::test]
async fn forward_invoke_self_target_unknown_ability_returns_route_negative() {
    let rt = easynet_axon::invocation::LocalRuntime::new();
    let svc = make_service()
        .with_session_realm("test-realm")
        .with_local_runtime(Arc::clone(&rt));
    publish_test_route(&svc, TEST_DAEMON_URI, "demo.missing");

    let err = svc
        .unary_dispatcher()
        .dispatch_federation_forward_invoke(
            None,
            &forward_invoke_args_for_ability(
                TEST_DAEMON_URI,
                "demo.missing",
                serde_json::json!({}),
            ),
        )
        .await
        .expect_err("known self target with unknown ability must be rejected");
    // RFC-005 D105: the device's own runtime is the authority, so an
    // ability the runtime does not host is a resolver NODATA negative.
    assert_eq!(err.code(), tonic::Code::FailedPrecondition);
    assert!(
        err.message().contains(ROUTE_NEGATIVE_CODE)
            && err
                .message()
                .contains("does not register a dispatchable route"),
        "expected a typed resolver negative, got: {err}"
    );
}

#[tokio::test]
async fn forward_invoke_self_target_does_not_intercept_other_target_uras() {
    // Guard: the self-target arm must ONLY fire when
    // `target_ura == admission.daemon_ura()`. A different
    // target_ura (a real device URA in the same realm) goes
    // through the existing presence-push path and surfaces
    // target_offline when the device is not subscribed —
    // unchanged by the fall-through.
    let rt = runtime_with_json_echo(
        TEST_DAEMON_URI,
        "demo.echo",
        "MARKER-OTHER",
        "must-not-fire",
    )
    .await;
    let svc = make_service()
        .with_session_realm("test-realm")
        .with_local_runtime(Arc::clone(&rt));

    let err = svc
        .unary_dispatcher()
        .dispatch_federation_forward_invoke(
            None,
            &forward_invoke_args("easynet:///r/test-realm/device/some-other-device"),
        )
        .await
        .expect_err("non-self target ⇒ presence-push path ⇒ target_offline");
    assert_eq!(err.code(), tonic::Code::FailedPrecondition);
    assert!(
        err.message().contains(ROUTE_NEGATIVE_CODE),
        "non-self route miss must surface resolver negative, got: {err}"
    );
}

#[tokio::test]
async fn forward_invoke_local_realm_requires_selected_route_before_peer_delegation() {
    // C1a / DEC-N4 §2.1: when `target_ura` realm matches
    // the daemon's own realm, the local presence-registry
    // path runs. With no presence entry inserted, the
    // dispatcher surfaces `Status::failed_precondition`
    // with the wire-stable `target_offline` reason. Critical:
    // the federation client is NEVER called even though one
    // is wired.
    let canned = InvokeResponse {
        result: br#"{"result_bytes":[]}"#.to_vec(),
        result_content_type: FEDERATION_RESULT_CONTENT_TYPE.to_string(),
        state: easynet_axon::invocation::InvocationState::Completed.to_wire_i32(),
        ..InvokeResponse::default()
    };
    let recorder = Arc::new(RecordingFederationClient::new(canned));

    let svc = make_service()
        .with_session_realm("test-realm")
        .with_federation_client(recorder.clone() as Arc<dyn FederationClient>);

    let err = svc
        .unary_dispatcher()
        .dispatch_federation_forward_invoke(
            None,
            &forward_invoke_args("easynet:///r/test-realm/device/local-target"),
        )
        .await
        .expect_err("local-realm resolver miss surfaces route negative");
    assert_eq!(err.code(), tonic::Code::FailedPrecondition);
    assert!(
        err.message().contains(ROUTE_NEGATIVE_CODE),
        "expected resolver negative reason, got: {err}"
    );
    assert!(
        recorder.calls().is_empty(),
        "federation client must NOT be called for local-realm resolver negative"
    );
}

#[tokio::test]
async fn forward_invoke_same_realm_route_negative_does_not_peer_fanout_when_configured() {
    let canned = InvokeResponse {
        result: serde_json::to_vec(&federation_wrappers::ForwardInvokeResponse {
            result_bytes: br#"{"hello":"from-same-realm-peer"}"#.to_vec(),
            correlation_call_id: "peer-call-id".to_string(),
        })
        .expect("encode peer ForwardInvokeResponse"),
        result_content_type: FEDERATION_RESULT_CONTENT_TYPE.to_string(),
        state: easynet_axon::invocation::InvocationState::Completed.to_wire_i32(),
        ..InvokeResponse::default()
    };
    let recorder = Arc::new(RecordingFederationClient::new(canned));
    let mut peers = BTreeMap::new();
    peers.insert(
        "same-realm-peer-hub".to_string(),
        "https://same-realm-peer.example:50443".to_string(),
    );

    let svc = make_service()
        .with_session_realm("test-realm")
        .with_federation_client(recorder.clone() as Arc<dyn FederationClient>)
        .with_federated_peers(peers);

    let target_ura = "easynet:///r/test-realm/device/paired-on-peer";
    let err = svc
        .unary_dispatcher()
        .dispatch_federation_forward_invoke(None, &forward_invoke_args(target_ura))
        .await
        .expect_err("local resolver negative stays terminal even with peers configured");
    assert_eq!(err.code(), tonic::Code::FailedPrecondition);
    assert!(
        err.message().contains(ROUTE_NEGATIVE_CODE),
        "expected resolver negative reason, got: {err}"
    );
    assert!(
        recorder.calls().is_empty(),
        "RFC-005 forbids same-realm peer fanout after local resolver negative"
    );
}

#[tokio::test]
async fn forward_invoke_cross_realm_without_peer_route_surfaces_resolver_noroute_before_client() {
    // Route-first invariant: the resolver must select a peer
    // delegation before the dispatcher checks whether a federation
    // client is wired. An unmapped realm is therefore a typed
    // `NEGATIVE_REASON_NOROUTE`, not an opaque transport
    // `target_offline`.
    let svc = make_service().with_session_realm("test-realm");

    let err = svc
        .unary_dispatcher()
        .dispatch_federation_forward_invoke(
            None,
            &forward_invoke_args("easynet:///r/peer-realm/device/peer-target"),
        )
        .await
        .expect_err("cross-realm without peer route surfaces resolver NOROUTE");
    assert_eq!(err.code(), tonic::Code::FailedPrecondition);
    assert_route_negative_noroute(err.message());
}

#[tokio::test]
async fn forward_invoke_cross_realm_with_peer_route_but_no_client_returns_target_offline() {
    // Once the resolver has selected a concrete peer hub route, missing
    // federation transport is a dispatch-plane offline condition.
    let mut peers = BTreeMap::new();
    peers.insert(
        "peer-realm".to_string(),
        "https://peer-hub.example:50443".to_string(),
    );
    let svc = make_service()
        .with_session_realm("test-realm")
        .with_federated_peers(peers);

    let err = svc
        .unary_dispatcher()
        .dispatch_federation_forward_invoke(
            None,
            &forward_invoke_args("easynet:///r/peer-realm/device/peer-target"),
        )
        .await
        .expect_err("selected peer route without client surfaces target_offline");
    assert_eq!(err.code(), tonic::Code::FailedPrecondition);
    assert_eq!(
        err.message(),
        federation_wrappers::FORWARD_INVOKE_TARGET_OFFLINE_REASON,
    );
}

#[tokio::test]
async fn forward_invoke_cross_realm_with_no_peer_entry_surfaces_resolver_noroute() {
    // C1a / DEC-N4 §2.1: federation client wired but the
    // operator-curated `federated_peers` map has no entry
    // for the target's realm. Under RFC-005 the cross-realm
    // delegation runs `namespace.resolve` first, so an
    // unmapped realm surfaces a typed `FailedPrecondition`
    // carrying `NEGATIVE_REASON_NOROUTE` instead of the old
    // opaque `target_offline` string. The map is still the
    // operator's explicit statement of "these are the peer
    // realms I federate with"; an unmapped realm is not
    // dialable and the federation client is never called.
    let canned = InvokeResponse {
        result: br#"{"result_bytes":[]}"#.to_vec(),
        result_content_type: FEDERATION_RESULT_CONTENT_TYPE.to_string(),
        state: easynet_axon::invocation::InvocationState::Completed.to_wire_i32(),
        ..InvokeResponse::default()
    };
    let recorder = Arc::new(RecordingFederationClient::new(canned));

    let svc = make_service()
        .with_session_realm("test-realm")
        .with_federation_client(recorder.clone() as Arc<dyn FederationClient>);

    let err = svc
        .unary_dispatcher()
        .dispatch_federation_forward_invoke(
            None,
            &forward_invoke_args("easynet:///r/unmapped-realm/device/peer-target"),
        )
        .await
        .expect_err("unmapped realm surfaces resolver NOROUTE");
    assert_eq!(err.code(), tonic::Code::FailedPrecondition);
    assert_route_negative_noroute(err.message());
    assert!(
        recorder.calls().is_empty(),
        "federation client must NOT be called when peer entry is missing"
    );
}

#[tokio::test]
async fn forward_invoke_cross_realm_auto_routes_via_federated_directory_when_opted_in() {
    // **Cross-hub auto-route, operator opt-in path**.
    // `federated_peers` is empty so the operator did NOT
    // statically declare `peer-realm → hub_endpoint`. But (a) the
    // hub-to-hub directory sync has previously observed the
    // target device on `https://hub-auto.example:50443`, and
    // (b) the operator opted into directory-driven auto-route
    // via `[daemon] allow_directory_auto_route = true`. The
    // dispatcher must then look the device up in
    // `federated_directory`, lift its `hub_endpoint`, and dial
    // there — lifting the requirement that operators
    // pre-declare every reachable realm in daemon-config.toml.
    //
    // The default-off counterpart lives in
    // `forward_invoke_cross_realm_directory_fallback_surfaces_resolver_noroute_by_default`.
    use crate::daemon::federation::directory::{
        DirectoryEntry, DirectoryView, SharedFederatedDirectoryView,
    };
    use std::collections::BTreeMap;

    let peer_reply_bytes = br#"{"hello":"from-auto-routed-peer"}"#.to_vec();
    let canned = InvokeResponse {
        result: serde_json::to_vec(&federation_wrappers::ForwardInvokeResponse {
            result_bytes: peer_reply_bytes.clone(),
            correlation_call_id: "test-call-id-1".to_string(),
        })
        .expect("encode peer ForwardInvokeResponse"),
        result_content_type: FEDERATION_RESULT_CONTENT_TYPE.to_string(),
        state: easynet_axon::invocation::InvocationState::Completed.to_wire_i32(),
        ..InvokeResponse::default()
    };
    let recorder = Arc::new(RecordingFederationClient::new(canned));

    let target_ura = "easynet:///r/unmapped-realm/device/peer-target";
    let cell = SharedFederatedDirectoryView::default();
    let mut peer_view = DirectoryView::new("unmapped-realm".to_string());
    peer_view.replace_entries(vec![DirectoryEntry {
        agent_ura: target_ura.to_string(),
        node_id: "peer-target".to_string(),
        display_name: None,
        status: "active".to_string(),
        origin_realm: None,
        hub_endpoint: Some("https://hub-auto.example:50443".to_string()),
        last_seen_unix_ms: Some(1_714_500_000_000),
    }]);
    let mut peers = BTreeMap::new();
    peers.insert("unmapped-realm".to_string(), Arc::new(peer_view));
    cell.replace(peers);

    // Crucially: NO `with_federated_peers(...)`. The static
    // operator-curated map is empty — only the directory cell
    // knows where the target lives. The opt-in is set
    // explicitly to mirror the production wiring from
    // `boot.rs`'s `config.allow_directory_auto_route()`.
    let svc = make_service()
        .with_session_realm("test-realm")
        .with_federation_client(recorder.clone() as Arc<dyn FederationClient>)
        .with_federated_directory_cell(cell)
        .with_allow_directory_auto_route(true);

    let resp = svc
        .unary_dispatcher()
        .dispatch_federation_forward_invoke(None, &forward_invoke_args(target_ura))
        .await
        .expect("directory-fallback path dials the auto-discovered hub");

    let body: federation_wrappers::ForwardInvokeResponse = parse_response_body(resp);
    assert_eq!(body.result_bytes, peer_reply_bytes);
    assert_eq!(body.correlation_call_id, "test-call-id-1");

    let calls = recorder.calls();
    assert_eq!(
        calls.len(),
        1,
        "exactly one peer dial — at the directory-derived hub_endpoint"
    );
    assert_eq!(
        calls[0].0, "https://hub-auto.example:50443",
        "dial target must come from federated_directory.hub_endpoint, \
         not from the (empty) federated_peers map"
    );
}

#[tokio::test]
async fn forward_invoke_cross_realm_directory_fallback_surfaces_resolver_noroute_by_default() {
    // **P0 default-off pin**. Same setup as
    // `forward_invoke_cross_realm_auto_routes_via_federated_directory_when_opted_in`
    // but the operator has NOT opted in. The directory has the
    // entry, but the dispatcher must refuse to dial — it would
    // be handing an outbound federation request to a peer-hub-
    // controllable URL. The contract is: with the secure
    // default, an unmapped realm always resolves to typed
    // `NEGATIVE_REASON_NOROUTE`, regardless of what the
    // directory sync observed.
    use crate::daemon::federation::directory::{
        DirectoryEntry, DirectoryView, SharedFederatedDirectoryView,
    };
    use std::collections::BTreeMap;

    let canned = InvokeResponse {
        result: br#"{"result_bytes":[]}"#.to_vec(),
        result_content_type: FEDERATION_RESULT_CONTENT_TYPE.to_string(),
        state: easynet_axon::invocation::InvocationState::Completed.to_wire_i32(),
        ..InvokeResponse::default()
    };
    let recorder = Arc::new(RecordingFederationClient::new(canned));

    let target_ura = "easynet:///r/unmapped-realm/device/peer-target";
    let cell = SharedFederatedDirectoryView::default();
    let mut peer_view = DirectoryView::new("unmapped-realm".to_string());
    peer_view.replace_entries(vec![DirectoryEntry {
        agent_ura: target_ura.to_string(),
        node_id: "peer-target".to_string(),
        display_name: None,
        status: "active".to_string(),
        origin_realm: None,
        hub_endpoint: Some("https://attacker.example:50443".to_string()),
        last_seen_unix_ms: Some(1_714_500_000_000),
    }]);
    let mut peers = BTreeMap::new();
    peers.insert("unmapped-realm".to_string(), Arc::new(peer_view));
    cell.replace(peers);

    // No `with_allow_directory_auto_route(true)` — service
    // inherits the secure default (false).
    let svc = make_service()
        .with_session_realm("test-realm")
        .with_federation_client(recorder.clone() as Arc<dyn FederationClient>)
        .with_federated_directory_cell(cell);

    let err = svc
        .unary_dispatcher()
        .dispatch_federation_forward_invoke(None, &forward_invoke_args(target_ura))
        .await
        .expect_err("default-off must refuse the directory-derived endpoint");
    assert_eq!(err.code(), tonic::Code::FailedPrecondition);
    assert_route_negative_noroute(err.message());
    assert!(
        recorder.calls().is_empty(),
        "federation client must NOT be called when directory fallback is disabled"
    );
}

#[tokio::test]
async fn forward_invoke_cross_realm_directory_entry_without_hub_endpoint_surfaces_resolver_noroute()
{
    // Edge case: the directory has the target URA but the peer's
    // snapshot omitted `hub_endpoint`. Auto-route has nowhere to
    // dial; the resolver must surface a typed `NEGATIVE_REASON_NOROUTE`
    // rather than dialing some default. Operators relying on auto-route
    // need to know their directory sync is missing the endpoint
    // field, not get a misleading "delivered" outcome.
    use crate::daemon::federation::directory::{
        DirectoryEntry, DirectoryView, SharedFederatedDirectoryView,
    };
    use std::collections::BTreeMap;

    let canned = InvokeResponse {
        result: br#"{"result_bytes":[]}"#.to_vec(),
        result_content_type: FEDERATION_RESULT_CONTENT_TYPE.to_string(),
        state: easynet_axon::invocation::InvocationState::Completed.to_wire_i32(),
        ..InvokeResponse::default()
    };
    let recorder = Arc::new(RecordingFederationClient::new(canned));

    let target_ura = "easynet:///r/unmapped-realm/device/peer-target";
    let cell = SharedFederatedDirectoryView::default();
    let mut peer_view = DirectoryView::new("unmapped-realm".to_string());
    peer_view.replace_entries(vec![DirectoryEntry {
        agent_ura: target_ura.to_string(),
        node_id: "peer-target".to_string(),
        display_name: None,
        status: "active".to_string(),
        origin_realm: None,
        hub_endpoint: None, // <- the gap under test
        last_seen_unix_ms: Some(1_714_500_000_000),
    }]);
    let mut peers = BTreeMap::new();
    peers.insert("unmapped-realm".to_string(), Arc::new(peer_view));
    cell.replace(peers);

    // The opt-in is ON in this test so we exercise the
    // "missing hub_endpoint" branch of the resolver, not the
    // "fallback disabled" branch (which is its own pin in
    // `forward_invoke_cross_realm_directory_fallback_surfaces_resolver_noroute_by_default`).
    let svc = make_service()
        .with_session_realm("test-realm")
        .with_federation_client(recorder.clone() as Arc<dyn FederationClient>)
        .with_federated_directory_cell(cell)
        .with_allow_directory_auto_route(true);

    let err = svc
        .unary_dispatcher()
        .dispatch_federation_forward_invoke(None, &forward_invoke_args(target_ura))
        .await
        .expect_err("missing hub_endpoint cannot be auto-routed");
    assert_eq!(err.code(), tonic::Code::FailedPrecondition);
    assert_route_negative_noroute(err.message());
    assert!(
        recorder.calls().is_empty(),
        "no dial when directory entry carries no hub_endpoint"
    );
}

#[tokio::test]
async fn forward_invoke_cross_realm_with_peer_entry_dials_via_federation_client() {
    // C1a / DEC-N4 §2.1: cross-realm + federation client
    // wired + peer entry present ⇒ federation client called
    // with the peer's hub URA + the *inner* ability decoded
    // from `inner_envelope_b64`. Response carries peer's
    // `result` bytes through `result_bytes`, plus the
    // caller's `correlation_call_id` echoed back.
    let peer_reply_bytes = br#"{"hello":"from-peer"}"#.to_vec();
    let canned = InvokeResponse {
        result: peer_reply_bytes.clone(),
        result_content_type: FEDERATION_RESULT_CONTENT_TYPE.to_string(),
        state: easynet_axon::invocation::InvocationState::Completed.to_wire_i32(),
        ..InvokeResponse::default()
    };
    let recorder = Arc::new(RecordingFederationClient::new(canned));

    let mut peers = BTreeMap::new();
    peers.insert(
        "peer-realm".to_string(),
        "https://peer-hub.example:50443".to_string(),
    );

    let svc = make_service()
        .with_session_realm("test-realm")
        .with_federation_client(recorder.clone() as Arc<dyn FederationClient>)
        .with_federated_peers(peers);

    let target_ura = "easynet:///r/peer-realm/device/peer-target";
    let args = forward_invoke_args(target_ura);
    let resp = svc
        .unary_dispatcher()
        .dispatch_federation_forward_invoke(None, &args)
        .await
        .expect("cross-realm returns Ok");

    // Response carries the peer's `result` bytes verbatim
    // in `result_bytes`, and stamps back the caller's
    // `call_id` from the fixture as `correlation_call_id`.
    let body: federation_wrappers::ForwardInvokeResponse = parse_response_body(resp);
    assert_eq!(body.result_bytes, peer_reply_bytes);
    assert_eq!(body.correlation_call_id, "test-call-id-1");

    let calls = recorder.calls();
    assert_eq!(calls.len(), 1, "exactly one peer delegation call");
    assert_eq!(calls[0].0, "https://peer-hub.example:50443");
    // **LB-57 §一 Option A wire shape**. Peer delegation
    // re-wraps the call as another `federation.forward_invoke`
    // so the peer hub's top-level `Invoke::invoke` match routes
    // through `dispatch_federation_forward_invoke` (which owns
    // local-session dispatch + same-realm fan-out + cross-realm
    // delegation). The pre-LB-57 PR-N1 commit 11/N shape (sending the
    // bare inner ability name) landed at the peer's `other` arm
    // → Unimplemented → demo `target_offline`. This assertion
    // pins the new wire shape; flipping back to bare-inner-name
    // would re-introduce the LB-57 §〇 production bug.
    assert_eq!(
        calls[0].1.function_name, ABILITY_FEDERATION_FORWARD_INVOKE,
        "LB-57 Option A: peer dispatcher receives the federation.forward_invoke \
         wrapper, NOT the bare inner ability name"
    );
    // The peer_request body is a serialized
    // ForwardInvokeRequest carrying the SAME target_ura +
    // inner_envelope_b64 the caller hub received, so the
    // peer's `dispatch_federation_forward_invoke` re-runs
    // its own routing (local-presence / same-realm fan-out
    // / cross-realm dial) against the original payload.
    let nested: federation_wrappers::ForwardInvokeRequest =
        serde_json::from_slice(&calls[0].1.arguments)
            .expect("peer arguments decode as nested ForwardInvokeRequest");
    assert_eq!(nested.target_ura, target_ura);
    assert!(
        !nested.inner_envelope_b64.is_empty(),
        "nested wrapper carries the original inner_envelope_b64 verbatim"
    );
    // When the original request carries no caller envelope, the
    // caller hub must still present its own hub URA to the peer.
    // Using `target_ura` here makes the peer believe the target
    // device itself initiated the call, which fails trust-anchor
    // admission and opens the circuit breaker.
    let peer_envelope = calls[0].1.envelope.as_ref().expect("envelope present");
    let peer_caller = peer_envelope
        .caller
        .as_ref()
        .expect("caller identity present");
    assert_eq!(peer_caller.ura, crate::ura::hub_ura("test-realm"));
    let peer_callee = peer_envelope
        .callee
        .as_ref()
        .expect("callee identity present");
    assert_eq!(peer_callee.ura, crate::ura::hub_ura("peer-realm"));
    let caller_signature = peer_envelope
        .caller_signature
        .as_ref()
        .expect("caller signature present for peer admission");
    assert_eq!(caller_signature.algorithm, "ed25519");
    assert!(
        !caller_signature.signature.is_empty(),
        "peer envelope signature bytes must be populated"
    );
    assert_eq!(
        peer_envelope.invocation_nonce.len(),
        16,
        "peer envelope must carry a fresh 16-byte nonce for strict admission"
    );
    let peer_signature = peer_envelope
        .caller_signature
        .as_ref()
        .expect("peer envelope must be signed for cross-hub admission");
    assert_eq!(peer_signature.algorithm, "ed25519");
    assert_eq!(
        peer_signature.signature.len(),
        64,
        "peer envelope signature must be one Ed25519 signature"
    );
}

#[tokio::test]
async fn forward_invoke_cross_realm_peer_request_admits_against_hub_anchor() {
    // The cross-hub deep harness failure we care about is not
    // "signature field missing" anymore; it is "peer hub rejects
    // the rebuilt federation.forward_invoke wrapper with
    // CALLER_SIGNATURE_INVALID". Rebuild that exact wrapper via
    // the caller-hub dispatch path, then feed it into a fresh
    // AdmissionFacade that trusts the caller hub's public key.
    //
    // If this test fails, the signer/canonicalization path is
    // wrong. If it passes while docker deep e2e still fails, the
    // remaining bug lives in boot/runtime wiring rather than in
    // the envelope bytes themselves.
    use base64::{engine::general_purpose::STANDARD as BASE64_STANDARD, Engine as _};
    use ed25519_dalek::SigningKey;

    let canned = InvokeResponse {
        result: br#"{"result_bytes":[]}"#.to_vec(),
        result_content_type: FEDERATION_RESULT_CONTENT_TYPE.to_string(),
        state: easynet_axon::invocation::InvocationState::Completed.to_wire_i32(),
        ..InvokeResponse::default()
    };
    let recorder = Arc::new(RecordingFederationClient::new(canned));

    let mut peers = BTreeMap::new();
    peers.insert(
        "peer-realm".to_string(),
        "https://peer-hub.example:50443".to_string(),
    );

    let svc = make_service()
        .with_session_realm("test-realm")
        .with_federation_client(recorder.clone() as Arc<dyn FederationClient>)
        .with_federated_peers(peers);

    let target_ura = "easynet:///r/peer-realm/device/peer-target";
    svc.unary_dispatcher()
        .dispatch_federation_forward_invoke(None, &forward_invoke_args(target_ura))
        .await
        .expect("cross-realm wrapper build succeeds");

    let calls = recorder.calls();
    assert_eq!(calls.len(), 1, "exactly one peer request captured");
    let peer_request = calls[0].1.clone();
    let peer_envelope = peer_request
        .envelope
        .as_ref()
        .expect("peer request envelope present");
    let caller_ura = peer_envelope
        .caller
        .as_ref()
        .expect("caller present")
        .ura
        .clone();

    let caller_signing_key = SigningKey::from_bytes(&[0x11; 32]);
    let caller_pubkey_b64 = BASE64_STANDARD.encode(caller_signing_key.verifying_key().to_bytes());
    let peer_anchor = Arc::new(
        RealmTrustAnchor::from_entries(vec![crate::daemon::trust::anchor::TrustedAgent {
            agent_ura: caller_ura,
            public_key_b64: caller_pubkey_b64,
            role: crate::daemon::trust::anchor::TrustedAgentRole::Hub,
            added_at_unix_ms: 1_714_492_800_000,
            origin_realm: Some("test-realm".to_string()),
            hub_endpoint: Some("https://peer-hub.example:50443".to_string()),
            tls_ca_pem_path: None,
        }])
        .expect("peer hub trust anchor"),
    );
    let peer_admission = AdmissionFacade::new(peer_anchor, Some(crate::ura::hub_ura("peer-realm")));

    peer_admission
        .verify_invoke(&peer_request)
        .expect("peer hub must admit the rebuilt signed wrapper");
}

// ── C1b / DEC-N5 §1: ForwardReceipt dual-write tests ──

// Phase 5a removed the three ForwardReceipt-shape tests
// (`forward_invoke_cross_realm_happy_path_records_forward_receipt_with_digest`,
//  `forward_invoke_target_offline_records_forward_receipt_with_no_digest`,
//  `forward_invoke_local_realm_miss_records_forward_receipt_with_no_digest`).
// Their entire surface was asserting on the now-deleted
// `SharedReceiptStore`. The *behaviours* those tests pinned
// (target_offline returns FailedPrecondition / local-realm
// resolver miss returns FailedPrecondition / cross-realm
// happy path returns Ok) are still covered by the
// `forward_invoke_local_realm_requires_selected_route_before_peer_delegation`,
// `forward_invoke_*_target_offline` and
// `cross_hub_forward_invoke_e2e_in_process` tests further
// down — those check the wire-level Result, which is the
// contract that actually matters for downstream callers.

// ── PR-N1 commit 5/N: 2-daemon in-process cross-hub e2e ──

#[tokio::test]
async fn cross_hub_forward_invoke_e2e_in_process() {
    // ── Setup: two daemons in distinct realms ─────────
    // daemon_a: realm "realm-a", knows about daemon_b's
    //           realm via federated_peers + federation_client.
    // daemon_b: realm "realm-b", peer dispatches through to
    //           its own local presence registry.
    //
    const REALM_A: &str = "realm-a";
    const REALM_B: &str = "realm-b";
    const DAEMON_A_URI: &str = "easynet:///r/realm-a/device/daemon-a";
    const DAEMON_B_URI: &str = "easynet:///r/realm-b/device/daemon-b";
    const TARGET_DEVICE_URI: &str = "easynet:///r/realm-b/device/target-device";
    const PEER_HUB_URI: &str = "https://daemon-b.example:50443";
    const DAEMON_A_SIGNING_SEED: [u8; 32] = [0xA1; 32];

    let daemon_a_signing_key = ed25519_dalek::SigningKey::from_bytes(&DAEMON_A_SIGNING_SEED);
    let daemon_a_pubkey_b64 = {
        use base64::{engine::general_purpose::STANDARD as BASE64_STANDARD, Engine as _};
        BASE64_STANDARD.encode(daemon_a_signing_key.verifying_key().to_bytes())
    };

    // Daemon B's trust anchor contains daemon A's public key. The
    // in-process federation client below signs the rebuilt peer
    // request with the matching private key, so daemon B exercises
    // the same strict Device-caller admission path as production callers.
    let daemon_a_in_b_trust = vec![crate::daemon::trust::anchor::TrustedAgent {
        agent_ura: DAEMON_A_URI.to_string(),
        public_key_b64: daemon_a_pubkey_b64,
        role: crate::daemon::trust::anchor::TrustedAgentRole::Device,
        added_at_unix_ms: 1_714_492_800_000,
        origin_realm: None,
        hub_endpoint: None,
        tls_ca_pem_path: None,
    }];
    let daemon_b_anchor =
        Arc::new(RealmTrustAnchor::from_entries(daemon_a_in_b_trust).expect("anchor"));

    // Daemon B: presence registry contains the target device,
    // and a `PendingDispatchMap` is wired so the new LB-57
    // local-presence dispatch path can register a pending
    // entry, push a SessionDispatch::Dispatch frame, and
    // await the matching Result. A fake device task spawned
    // below drains the reverse-channel push, decodes the
    // dispatch frame, and completes the pending entry with
    // canned bytes (mirrors what `drain_session_up_stream`
    // does in production when the target device sends
    // SessionDispatch::Result up).
    let daemon_b_presence = Arc::new(PresenceRegistry::new());
    let (target_tx, mut target_rx) = tokio::sync::mpsc::channel(8);
    daemon_b_presence.insert(TARGET_DEVICE_URI.to_string(), target_tx);

    let daemon_b_pending = Arc::new(PendingDispatchMap::new());
    let daemon_b_admission = AdmissionFacade::new(daemon_b_anchor, Some(DAEMON_B_URI.to_string()));
    let daemon_b = Arc::new(
        DaemonInvocationService::new(daemon_b_presence, daemon_b_admission)
            .with_session_realm(REALM_B)
            .with_pending(Arc::clone(&daemon_b_pending)),
    );
    publish_test_route(&daemon_b, TARGET_DEVICE_URI, "federation.heartbeat");

    // Fake device-B task: drain the dispatch frame, decode it,
    // and feed back a canned ability response via
    // PendingDispatchMap::complete. The canned bytes here are
    // the JSON shape `federation.heartbeat`'s real handler
    // would have produced if it ran on a real device — kept
    // structurally lean (one field) so the test asserts only
    // round-trip integrity, not full handler semantics.
    let pending_for_fake = Arc::clone(&daemon_b_pending);
    tokio::spawn(async move {
        while let Some(frame_result) = target_rx.recv().await {
            let frame = match frame_result {
                Ok(f) => f,
                Err(_) => continue,
            };
            use easynet_axon::pb::axon::v1::invoke_bidi_down::Payload as DownPayload;
            let chunk = match frame.frame.payload {
                Some(DownPayload::BinaryChunk(c)) => c,
                _ => continue,
            };
            let dispatch: SessionDispatch = match serde_json::from_slice(&chunk.data) {
                Ok(d) => d,
                Err(_) => continue,
            };
            let SessionDispatch::Dispatch { call_id, .. } = dispatch else {
                continue;
            };
            let canned = br#"{"echo":"e2e-canned"}"#.to_vec();
            pending_for_fake.complete(
                call_id,
                DispatchResult {
                    payload: canned,
                    error: None,
                    failure: None,
                    request_id: None,
                    receipt: None,
                },
            );
        }
    });

    // Daemon A: empty presence registry; cross-realm target
    // routes via the InProcessPeerClient → daemon B. The fixture
    // signs the peer request after daemon A has rebuilt the target
    // ability and argument bytes, matching the strict admission
    // bytes daemon B verifies.
    let daemon_a_admission = AdmissionFacade::new(
        Arc::new(RealmTrustAnchor::default()),
        Some(DAEMON_A_URI.to_string()),
    );
    let federation_client: Arc<dyn FederationClient> = Arc::new(ForwardingPeerClient {
        peer: daemon_b,
        caller_ura: DAEMON_A_URI.to_string(),
        callee_ura: crate::ura::hub_ura(REALM_B),
        subject_ura: DAEMON_B_URI.to_string(),
        signing_seed: DAEMON_A_SIGNING_SEED,
    });
    let mut peers = BTreeMap::new();
    peers.insert(REALM_B.to_string(), PEER_HUB_URI.to_string());

    let daemon_a =
        DaemonInvocationService::new(Arc::new(PresenceRegistry::new()), daemon_a_admission)
            .with_session_realm(REALM_A)
            .with_federation_client(federation_client)
            .with_federated_peers(peers);

    // ── Drive: daemon_a receives a federation.forward_invoke ──
    // PR-N1 commit 11/N rewrote the dispatch path: daemon A
    // now decodes the CLI bridge's `inner_envelope_b64`
    // (base64 of `{ability_ura, args}`) and sends the inner
    // ability URA to the peer instead of re-wrapping in another
    // `federation.forward_invoke`.
    //
    // base64({"ability_ura":".../ability/device.target-device-b.federation.heartbeat","args":{
    //   "membership_ura":"easynet:///r/realm-b/device/target-device-b",
    //   "ts_ms":0
    // }})
    let public_ability = "federation.heartbeat";
    let ability_ura = crate::ura::owner_ability_ura(TARGET_DEVICE_URI, public_ability)
        .expect("target device ability URA");
    let inner_payload = serde_json::json!({
        "ability_ura": ability_ura,
        "args": {
            "agent_ura": TARGET_DEVICE_URI,
        },
        "call_id": "e2e-call-id-1",
    });
    let inner_b64 = {
        use base64::{engine::general_purpose::STANDARD, Engine as _};
        STANDARD.encode(serde_json::to_vec(&inner_payload).unwrap())
    };
    let forward_args = format!(
        r#"{{"target_ura":"{}","inner_envelope_b64":"{}"}}"#,
        TARGET_DEVICE_URI, inner_b64
    );
    let req = Request::new(InvokeRequest {
        envelope: Some(test_envelope_with_uri(DAEMON_A_URI)),
        function_name: ABILITY_FEDERATION_FORWARD_INVOKE.to_string(),
        arguments: forward_args.into_bytes(),
        ..InvokeRequest::default()
    });

    let response = daemon_a
        .invoke(req)
        .await
        .expect("e2e forward_invoke returns Ok");
    let body = response.into_inner();

    // ── Assert: cross-realm chain returned the device's ──
    // canned bytes intact.
    // LB-57 Option A wire shape: the outer InvokeResponse
    // body carries a `ForwardInvokeResponse {result_bytes,
    // correlation_call_id}`, where `result_bytes` is the
    // canned bytes the fake device-B task fed back via
    // `PendingDispatchMap::complete`. The pre-LB-57 path
    // returned an empty `result_bytes` and the assertion
    // accidentally passed because the layered wrapper JSON
    // happened to parse as an object — that masked a real
    // wire-shape gap (raw inner-envelope BinaryChunk push
    // with no SessionDispatch::Dispatch wrapper, no
    // PendingDispatchMap correlation). The new contract
    // closes both halves.
    let outer: federation_wrappers::ForwardInvokeResponse =
        serde_json::from_slice(&body.result).expect("outer ForwardInvokeResponse is JSON");
    assert_eq!(outer.correlation_call_id, "e2e-call-id-1");
    assert_eq!(
        outer.result_bytes,
        br#"{"echo":"e2e-canned"}"#.to_vec(),
        "result_bytes must carry the fake device-B canned reply verbatim"
    );
}

/// Like `InProcessPeerClient` but signs a fresh envelope over the
/// rebuilt peer request so daemon B verifies the same ability and
/// argument bytes it will dispatch.
struct ForwardingPeerClient {
    peer: Arc<DaemonInvocationService>,
    caller_ura: String,
    callee_ura: String,
    subject_ura: String,
    signing_seed: [u8; 32],
}

#[async_trait::async_trait]
impl FederationClient for ForwardingPeerClient {
    async fn forward_invoke(
        &self,
        _target_hub: &crate::daemon::federation::client::HubUri,
        mut request: InvokeRequest,
    ) -> Result<InvokeResponse, crate::daemon::federation::client::FederationClientError> {
        let signing_key = ed25519_dalek::SigningKey::from_bytes(&self.signing_seed);
        request.envelope = Some(signed_test_envelope(
            &self.caller_ura,
            &self.callee_ura,
            &self.subject_ura,
            &request.function_name,
            &request.arguments,
            &signing_key,
        ));
        let response = self
            .peer
            .invoke(Request::new(request))
            .await
            .map_err(|status| {
                crate::daemon::federation::client::FederationClientError::InnerInvokeFailed {
                    hub: "in-process-peer".to_string(),
                    status: format!("code={:?} message={}", status.code(), status.message()),
                }
            })?;
        Ok(response.into_inner())
    }
}

fn test_envelope_with_uri(ura: &str) -> Envelope {
    Envelope {
        caller: Some(AgentIdentity {
            ura: ura.to_string(),
            ..AgentIdentity::default()
        }),
        ..Envelope::default()
    }
}
