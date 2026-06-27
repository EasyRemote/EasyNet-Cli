use super::*;

/// Descriptor proof a raw-runtime test ability must carry so Axon's
/// receipt-proof normalizer admits its dispatch (production stamps the
/// equivalent from the control-plane record). Non-zero stub hashes;
/// version is the default these owner-local test abilities dispatch under.
fn test_rpc_options() -> easynet_axon::invocation::AbilityOptions {
    use easynet_axon::invocation::{AbilityCallModes, AbilityOptions};
    AbilityOptions::default()
        .with_modes(AbilityCallModes::RPC)
        .with_descriptor_proof(
            crate::runtime::ability::DEFAULT_ABILITY_DESCRIPTOR_VERSION,
            [0x11; 32],
            [0x22; 32],
        )
}

fn test_stream_options() -> easynet_axon::invocation::AbilityOptions {
    use easynet_axon::invocation::{AbilityOptions, CallMode};
    AbilityOptions::streaming().with_mode_descriptor_proof(
        CallMode::Stream,
        crate::runtime::ability::DEFAULT_ABILITY_DESCRIPTOR_VERSION,
        [0x11; 32],
        [0x22; 32],
    )
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn matches_self_target_ura_accepts_hot_added_agent_only_for_local_identity() {
    // Hot-added agents can be dispatchable through `agents.json`
    // before publish persists them to `local-agents.json`. The
    // fallback must still be bound to the daemon's exact realm/user
    // identity so a peer realm or peer user cannot be collapsed into
    // this process by sharing the same bare agent name.
    use crate::persistence::config::{save_credentials, Credentials};
    use crate::registry::agents::{save_agents, AgentEntry, AgentRegistry, AgentType};
    let _hg = crate::facade::cli::test_support::HomeGuard::new();
    save_credentials(&Credentials {
        node_id: "dev-1".to_string(),
        credential_token: "token".to_string(),
        hub_endpoint: "axon://hub.test:50051".to_string(),
        realm: "test-realm".to_string(),
        username: Some("dev".to_string()),
        ..Default::default()
    })
    .expect("seed credentials");
    let svc = make_service().with_session_realm("test-realm");

    let agent_target = "easynet:///r/test-realm/agent/dev.liangbing";

    // Pre-write: no agents.json row → slow tier must miss too.
    assert!(
        !svc.target_gate()
            .matches_self_target_ura(agent_target)
            .await,
        "agent absent from agents.json must not be treated as self-target"
    );

    // Stage the hot-added row.
    let mut registry = AgentRegistry::default();
    registry.agents.insert(
        "liangbing".to_string(),
        AgentEntry::new(AgentType::ClaudeCode, None),
    );
    save_agents(&registry).expect("stage agents.json under HomeGuard");

    assert!(
        svc.target_gate()
            .matches_self_target_ura(agent_target)
            .await,
        "agent present in agents.json must be recognised as self-target \
         when the target realm/user match local credentials"
    );
    assert!(
        !svc.target_gate()
            .matches_self_target_ura("easynet:///r/other-realm/agent/dev.liangbing")
            .await,
        "same bare agent name in another realm must not be treated as local"
    );
    assert!(
        !svc.target_gate()
            .matches_self_target_ura("easynet:///r/test-realm/agent/peer.liangbing")
            .await,
        "same bare agent name under another user must not be treated as local"
    );

    // Sibling agent URA whose <agentID> is NOT in agents.json
    // must still be rejected — guards against the slow-tier
    // turning into a blanket "any agent URA is self-target".
    assert!(
        !svc.target_gate()
            .matches_self_target_ura("easynet:///r/test-realm/agent/dev.unknown")
            .await,
        "slow tier must only accept agents present in agents.json"
    );
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn matches_self_target_ura_uses_exact_local_agents_identity() {
    use crate::persistence::local_agents::{save, upsert_hosted_agent, LocalAgentsFile};

    let _hg = crate::facade::cli::test_support::HomeGuard::new();
    let mut local = LocalAgentsFile {
        host_device_agent_ura: "easynet:///r/test-realm/device/dev-1".to_string(),
        hosted_agents: Vec::new(),
    };
    upsert_hosted_agent(
        &mut local,
        "llm",
        "liangbing",
        "easynet:///r/test-realm/agent/dev.liangbing",
    );
    save(&local).expect("seed local-agents.json");

    let svc = make_service().with_session_realm("test-realm");
    assert!(
        svc.target_gate()
            .matches_self_target_ura("easynet:///r/test-realm/agent/dev.liangbing")
            .await,
        "exact hosted Agent identity from local-agents.json must be local"
    );
    assert!(
        !svc.target_gate()
            .matches_self_target_ura("easynet:///r/other-realm/agent/dev.liangbing")
            .await,
        "local-agents identity must include the realm"
    );
    assert!(
        !svc.target_gate()
            .matches_self_target_ura("easynet:///r/test-realm/agent/peer.liangbing")
            .await,
        "local-agents identity must include the user id"
    );
}

#[tokio::test]
async fn dispatch_invoke_remote_routes_through_axon_runtime_when_ability_registered() {
    // RFC-005 acceptance: `<self>.invoke_remote` self-target
    // execution is selected by `namespace.resolve`, then
    // dispatched through Axon LocalRuntime using the selected
    // route's callee + dispatch key.
    use easynet_axon::invocation::{make_ability, LocalRuntime};
    use futures::StreamExt;

    let _hg = crate::facade::cli::test_support::HomeGuard::new();

    let owner_ura = "easynet:///r/test-realm/agent/dev.liangbing";
    let ability_ura = crate::ura::owner_ability_ura(owner_ura, "chat").expect("agent ability URA");
    let rt = LocalRuntime::new();
    rt.register_ability_with_options(
        ability_ura.clone(),
        make_ability(|ctx| async move {
            // Echo: terminal payload is the inbound `args`.
            Ok(ctx.payload.clone())
        }),
        test_rpc_options(),
    )
    .await
    .unwrap();

    // `LocalRuntime::new()` already returns `Arc<LocalRuntime>`;
    // pass through verbatim.
    let svc = make_service()
        .with_session_realm("test-realm")
        .with_local_runtime(Arc::clone(&rt));
    publish_test_route(&svc, owner_ura, "chat");

    let selected_route = svc
        .target_gate()
        .route_resolver()
        .await
        .resolve_route(&ability_ura, "")
        .expect("resolver selects agent route");
    assert_eq!(selected_route.owner_ura, owner_ura);
    assert_eq!(selected_route.callee_ura, owner_ura);
    assert_eq!(selected_route.execution_host_ura, TEST_DAEMON_URI);
    assert_eq!(selected_route.ability_ura, ability_ura);

    let response = svc
        .unary_dispatcher()
        .dispatch_self_targeted_invoke_remote(
            &selected_route,
            None,
            b"hello-axon-routed".as_slice(),
            &std::collections::HashMap::new(),
            None,
        )
        .await
        .expect("self-target selected route dispatches");
    let mut stream = response.into_inner();
    let frame = stream
        .next()
        .await
        .expect("one terminal frame")
        .expect("terminal frame is in-band");
    let chunk = match frame.payload.expect("frame payload") {
        DownPayload::BinaryChunk(chunk) => chunk,
        other => panic!("expected BinaryChunk, got {other:?}"),
    };
    let down: InvokeRemoteDown =
        serde_json::from_slice(&chunk.data).expect("decode InvokeRemoteDown");
    match down {
        InvokeRemoteDown::Result { payload, error, .. } => {
            assert!(error.is_none(), "handler should complete: {error:?}");
            assert_eq!(payload, b"hello-axon-routed");
        }
        other => panic!("expected terminal Result, got {other:?}"),
    }
    assert!(
        stream.next().await.is_none(),
        "self-target stream is one-shot"
    );
}

#[tokio::test]
async fn self_targeted_origin_claim_warms_device_trust_on_miss() {
    // Honest-report 2026-06-11 item 15: the self-targeted
    // `<self>.invoke_remote` arm must consult the daemon's
    // DeviceTrustSync before verifying a device-signed origin
    // claim, exactly like the `<self>.session` dispatcher arm —
    // first-contact cross-device callers warm the anchor instead
    // of failing closed on a cold one. Admission itself must STAY
    // fail-closed: the fabricated signature below cannot admit.
    use easynet_axon::invocation::{make_ability, LocalRuntime};
    use std::sync::atomic::{AtomicBool, Ordering};

    use base64::{engine::general_purpose::STANDARD as B64, Engine as _};
    use futures::StreamExt;

    static RESOLVER_CONSULTED: AtomicBool = AtomicBool::new(false);
    fn recording_resolver(_ura: &str) -> anyhow::Result<Vec<String>> {
        RESOLVER_CONSULTED.store(true, Ordering::SeqCst);
        Ok(vec![])
    }

    let _hg = crate::facade::cli::test_support::HomeGuard::new();
    let owner_ura = "easynet:///r/test-realm/agent/dev.liangbing";
    let ability_ura = crate::ura::owner_ability_ura(owner_ura, "chat").expect("agent ability URA");
    let rt = LocalRuntime::new();
    rt.register_ability_with_options(
        ability_ura.clone(),
        make_ability(|ctx| async move { Ok(ctx.payload.clone()) }),
        test_rpc_options(),
    )
    .await
    .unwrap();

    let anchor_dir = tempfile::tempdir().expect("tmp anchor dir");
    let cell = crate::services::trust_anchor_cell::SharedTrustAnchor::new(Arc::new(
        crate::services::realm_trust_anchor::RealmTrustAnchor::from_entries(vec![])
            .expect("empty anchor"),
    ));
    let sync = Arc::new(
        crate::services::invocation_transport::device_trust_sync::DeviceTrustSync::with_static_source_for_tests(
            "test-realm".into(),
            anchor_dir.path().join("realm-trust.toml"),
            cell,
            recording_resolver,
        ),
    );

    let svc = make_service()
        .with_session_realm("test-realm")
        .with_local_runtime(Arc::clone(&rt))
        .with_device_trust_sync(sync);
    publish_test_route(&svc, owner_ura, "chat");
    let selected_route = svc
        .target_gate()
        .route_resolver()
        .await
        .resolve_route(&ability_ura, "")
        .expect("resolver selects agent route");

    let claim = crate::services::invocation_transport::origin_caller::OriginCallerClaim {
        caller_ura: "easynet:///r/test-realm/device/first-contact".into(),
        ability: "chat".into(),
        descriptor_version: crate::runtime::ability::DEFAULT_ABILITY_DESCRIPTOR_VERSION.into(),
        signature_b64: B64.encode([0_u8; 64]),
        signer_pubkey_b64: B64.encode([0_u8; 32]),
        nonce_b64: B64.encode([0_u8; 16]),
    };

    let response = svc
        .unary_dispatcher()
        .dispatch_self_targeted_invoke_remote(
            &selected_route,
            None,
            b"payload".as_slice(),
            &std::collections::HashMap::new(),
            Some(&claim),
        )
        .await
        .expect("claim dispatch completes in-band");

    assert!(
        RESOLVER_CONSULTED.load(Ordering::SeqCst),
        "device-signed origin claim must warm DeviceTrustSync before verification"
    );

    let mut stream = response.into_inner();
    let frame = stream
        .next()
        .await
        .expect("one terminal frame")
        .expect("terminal frame is in-band");
    let chunk = match frame.payload.expect("frame payload") {
        DownPayload::BinaryChunk(chunk) => chunk,
        other => panic!("expected BinaryChunk, got {other:?}"),
    };
    let down: InvokeRemoteDown =
        serde_json::from_slice(&chunk.data).expect("decode InvokeRemoteDown");
    match down {
        InvokeRemoteDown::Result { error, .. } => {
            assert!(
                error.is_some(),
                "fabricated signature must fail closed, not admit"
            );
        }
        other => panic!("expected terminal Result, got {other:?}"),
    }
}

#[tokio::test]
async fn axon_arm_must_not_intercept_calls_targeting_a_peer_device() {
    // **Phase 4 regression pin.**
    //
    // Without the `matches_self_target_ura` guard the Axon
    // arm intercepts every call whose ability is registered
    // locally, regardless of `subject_device`. That caused
    // the Web UI's `<self>.invoke_remote(subject_device=peer,
    // ability=agent.list)` to return THIS daemon's
    // agents instead of the peer's — the agent-list page
    // lit up with the wrong rows.
    //
    // The guard restricts the arm to self-target. This test
    // pins it: a call against a non-self peer URA must SKIP
    // the Axon arm so the selected remote-session path can
    // forward dispatch to the peer's session.
    //
    // We assert by reading the predicate directly:
    // `matches_self_target_ura` MUST return `false` for a
    // peer device URA even when the local runtime hosts the
    // requested ability. The dispatch arm checks this
    // predicate first; a `false` here is the only thing
    // standing between "Axon-local execution" and "peer
    // forward". This pin guards the regression at the
    // predicate layer; the full bidi exercise lives in
    // integration tests where a real grpc Streaming can be
    // constructed.
    use easynet_axon::invocation::{make_ability, LocalRuntime};

    let _hg = crate::facade::cli::test_support::HomeGuard::new();
    let rt = LocalRuntime::new();
    // Register an ability under a name that exists everywhere
    // (every daemon mirrors `agent.list` into its
    // LocalRuntime via the Phase-3 boot sweep). The bug it's
    // pinning: pre-guard, this presence would have hijacked
    // peer-target calls.
    rt.register_ability_with_options(
        "agent.list",
        make_ability(|_| async move { Ok(Vec::new()) }),
        test_rpc_options(),
    )
    .await
    .unwrap();

    let svc = make_service()
        .with_session_realm("test-realm")
        .with_local_runtime(Arc::clone(&rt));

    // 1. THIS daemon's URA → self target.
    assert!(
        svc.target_gate()
            .matches_self_target_ura(TEST_DAEMON_URI)
            .await,
        "own daemon URA must be self-target"
    );

    // 2. A peer device URA in the same realm → NOT self target.
    //    The dispatch arm must skip Axon and let selected
    //    remote-session dispatch forward to the peer.
    let peer_ura = "easynet:///r/test-realm/device/some-peer";
    assert!(
        !svc.target_gate().matches_self_target_ura(peer_ura).await,
        "peer device URA must NOT be self-target — the Axon arm \
         must skip and let selected remote-session dispatch forward"
    );

    // 3. A peer-realm hub URA → NOT self target.
    let peer_realm_hub = crate::ura::hub_ura("other-realm");
    assert!(
        !svc.target_gate()
            .matches_self_target_ura(&peer_realm_hub)
            .await,
        "peer realm hub must NOT be self-target"
    );
}

#[tokio::test]
async fn dispatch_local_rpc_selected_route_runs_runtime_when_registered() {
    // Catch-all unary `invoke` must resolve through namespace.resolve,
    // then route through Axon
    // (`invoke_async` → `LedgerSink`) when the runtime hosts the
    // ability — that's the path that gets the canonical record
    // into `invocations.redb` for CLI→daemon notify hops like
    // `easynet agent add` → `agent.start`.
    //
    // Returns `(response, axon_took_it=true)` so the caller in
    // `invoke()` skips the manual `record_unary_invocation`
    // write (avoiding the duplicate row keyed by `request_id`).
    use easynet_axon::invocation::{make_ability, InvocationLedger, LedgerSink, LocalRuntime};

    let _hg = crate::facade::cli::test_support::HomeGuard::new();
    let temp = tempfile::tempdir().unwrap();
    let ledger = Arc::new(InvocationLedger::open(temp.path().join("inv.redb")).unwrap());
    let rt = LocalRuntime::new();
    rt.set_ledger_sink(LedgerSink::new(Arc::clone(&ledger)));
    let runtime_ability =
        crate::ura::owner_ability_ura(TEST_DAEMON_URI, "demo.unary_via_axon").unwrap();
    rt.register_ability_with_options(
        runtime_ability.clone(),
        make_ability(|ctx| async move {
            let subject = ctx
                .runtime
                .axiom_envelope_of(&ctx.invocation_id)
                .await
                .map(|signed| signed.envelope.subject.ura);
            serde_json::to_vec(&serde_json::json!({
                "payload": serde_json::from_slice::<serde_json::Value>(&ctx.payload)
                    .unwrap_or(serde_json::Value::Null),
                "subject": subject,
            }))
            .map_err(|err| easynet_axon::invocation::AxonError::internal(err.to_string()))
        }),
        test_rpc_options(),
    )
    .await
    .unwrap();

    let svc = make_service()
        .with_session_realm("test-realm")
        .with_local_runtime(Arc::clone(&rt));
    publish_test_route(&svc, TEST_DAEMON_URI, "demo.unary_via_axon");

    let mut request = invoke_request("demo.unary_via_axon", r#"{"k":"v"}"#).into_inner();
    let external_caller = "easynet:///r/test-realm/device/client-1";
    let signing_key = test_device_signing_key();
    request.envelope = Some(signed_test_envelope(
        external_caller,
        TEST_DAEMON_URI,
        "easynet:///r/test-realm/resource/camera-1",
        &request.function_name,
        &request.arguments,
        &signing_key,
    ));
    let (result, axon_took_it) = svc
        .unary_dispatcher()
        .dispatch_local_rpc_selected_route(&request)
        .await;

    assert!(
        axon_took_it,
        "runtime hosts the ability ⇒ Axon path must take it; result={:?}",
        result.as_ref().err()
    );
    let response = result.expect("axon dispatch returns Ok");
    let body = response.into_inner();
    let decoded: serde_json::Value =
        serde_json::from_slice(&body.result).expect("decode handler payload");
    assert_eq!(decoded["payload"], serde_json::json!({"k": "v"}));
    assert_eq!(
        decoded["subject"], "easynet:///r/test-realm/resource/camera-1",
        "external-signed Axon dispatch must preserve the wire envelope subject"
    );
    let header_request_id = body
        .header
        .as_ref()
        .map(|header| header.request_id.as_str());
    assert!(
        header_request_id.is_some(),
        "Axon-routed unary response must expose the ledger request_id"
    );

    // LedgerSink writes on the spawn task; pacing matches Axon's
    // own ledger_sink_persists_completed_invocation pattern.
    tokio::task::yield_now().await;
    tokio::time::sleep(std::time::Duration::from_millis(50)).await;
    let records = ledger.list_all().expect("list ledger");
    assert_eq!(
        records.len(),
        1,
        "Axon-routed unary call must land exactly one ledger row"
    );
    assert_eq!(
        records[0].ability_name,
        format!(
            "{runtime_ability}@{}",
            crate::runtime::ability::DEFAULT_ABILITY_DESCRIPTOR_VERSION
        )
    );
    assert_eq!(records[0].state, "completed");
    assert_eq!(
        records[0].caller_ura, external_caller,
        "Axon-routed unary ledger row must preserve the external-signed wire caller"
    );
    assert_eq!(
        records[0].callee_ura, TEST_DAEMON_URI,
        "Axon-routed unary ledger row must preserve the external-signed wire callee"
    );
    assert_eq!(
        records[0].subject_ura, "easynet:///r/test-realm/resource/camera-1",
        "Axon-routed unary ledger row must preserve the external-signed wire subject"
    );
    assert_eq!(header_request_id, Some(records[0].request_id.as_str()));
}

#[tokio::test]
async fn dispatch_local_rpc_selected_route_accepts_unsigned_loopback_request() {
    use easynet_axon::invocation::{make_ability, InvocationLedger, LedgerSink, LocalRuntime};

    let _hg = crate::facade::cli::test_support::HomeGuard::new();
    let temp = tempfile::tempdir().unwrap();
    let ledger = Arc::new(InvocationLedger::open(temp.path().join("inv.redb")).unwrap());
    let rt = LocalRuntime::new();
    rt.set_ledger_sink(LedgerSink::new(Arc::clone(&ledger)));
    let runtime_ability =
        crate::ura::owner_ability_ura(TEST_DAEMON_URI, "demo.loopback_unsigned").unwrap();
    rt.register_ability_with_options(
        runtime_ability.clone(),
        make_ability(|ctx| async move {
            let envelope = ctx
                .runtime
                .axiom_envelope_of(&ctx.invocation_id)
                .await
                .expect("runtime stores descriptor-bound envelope")
                .envelope;
            serde_json::to_vec(&serde_json::json!({
                "caller": envelope.caller.ura,
                "callee": envelope.callee.ura,
                "subject": envelope.subject.ura,
                "payload": serde_json::from_slice::<serde_json::Value>(&ctx.payload)
                    .unwrap_or(serde_json::Value::Null),
            }))
            .map_err(|err| easynet_axon::invocation::AxonError::internal(err.to_string()))
        }),
        test_rpc_options(),
    )
    .await
    .unwrap();

    let svc = make_service()
        .with_session_realm("test-realm")
        .with_local_runtime(Arc::clone(&rt));
    publish_test_route(&svc, TEST_DAEMON_URI, "demo.loopback_unsigned");

    let arguments = br#"{"k":"v"}"#.to_vec();
    let request = InvokeRequest {
        envelope: Some(
            ProtoEnvelope::targeted(
                crate::runtime::local_invocation_identity::LOCAL_SYSTEM_AGENT_URA,
                TEST_DAEMON_URI,
                "easynet:///r/test-realm/resource/camera-1",
            )
            .expect("valid loopback envelope")
            .into_inner(),
        ),
        function_name: "demo.loopback_unsigned".to_string(),
        arguments,
        ..InvokeRequest::default()
    };

    let (result, axon_took_it) = svc
        .unary_dispatcher()
        .dispatch_local_rpc_selected_route(&request)
        .await;

    assert!(
        axon_took_it,
        "trusted loopback should enter Axon without external signature; result={:?}",
        result.as_ref().err()
    );
    let body = result.expect("loopback dispatch returns Ok").into_inner();
    let decoded: serde_json::Value =
        serde_json::from_slice(&body.result).expect("decode handler payload");
    assert_eq!(
        decoded["caller"],
        crate::runtime::local_invocation_identity::LOCAL_SYSTEM_AGENT_URA
    );
    assert_eq!(decoded["callee"], TEST_DAEMON_URI);
    assert_eq!(
        decoded["subject"],
        "easynet:///r/test-realm/resource/camera-1"
    );
    assert_eq!(decoded["payload"], serde_json::json!({"k": "v"}));

    tokio::task::yield_now().await;
    tokio::time::sleep(std::time::Duration::from_millis(50)).await;
    let records = ledger.list_all().expect("list ledger");
    assert_eq!(
        records.len(),
        1,
        "loopback dispatch must still be a single Axon-owned ledger row"
    );
    assert_eq!(
        records[0].ability_name,
        format!(
            "{runtime_ability}@{}",
            crate::runtime::ability::DEFAULT_ABILITY_DESCRIPTOR_VERSION
        )
    );
    assert_eq!(
        records[0].caller_ura,
        crate::runtime::local_invocation_identity::LOCAL_SYSTEM_AGENT_URA,
        "loopback dispatch must be signed by daemon-local system identity"
    );
    assert_eq!(records[0].callee_ura, TEST_DAEMON_URI);
    assert_eq!(
        records[0].subject_ura,
        "easynet:///r/test-realm/resource/camera-1"
    );
}

#[tokio::test]
async fn dispatch_local_rpc_selected_route_rejects_when_runtime_misses() {
    // A device-owned ability is the device's own runtime authority
    // (RFC-005 D105): when the runtime does not host the dispatch key,
    // the resolver itself rejects with a typed NODATA negative — the
    // catalog row alone cannot manufacture a route. There is no
    // select-then-fail-at-executor window for self-owned abilities.
    use easynet_axon::invocation::LocalRuntime;

    let _hg = crate::facade::cli::test_support::HomeGuard::new();
    let rt = LocalRuntime::new();
    let svc = make_service()
        .with_session_realm("test-realm")
        .with_local_runtime(Arc::clone(&rt));
    publish_test_route(&svc, TEST_DAEMON_URI, "missing.ability");

    let request = invoke_request("missing.ability", "{}").into_inner();
    let (result, axon_took_it) = svc
        .unary_dispatcher()
        .dispatch_local_rpc_selected_route(&request)
        .await;
    assert!(
        !axon_took_it,
        "runtime miss means no Axon invocation was started"
    );
    let err = result.expect_err("runtime miss rejects without alternate dispatch");
    assert_eq!(err.code(), tonic::Code::FailedPrecondition);
    assert!(
        err.message().contains(ROUTE_NEGATIVE_CODE)
            && err
                .message()
                .contains("does not register a dispatchable route"),
        "error must be a typed resolver negative naming the missing dispatch binding, got: {err}"
    );
}

#[tokio::test]
async fn dispatch_local_rpc_selected_route_returns_false_for_non_rpc_runtime_row() {
    // A registered stream/bidi-only ability is known to
    // LocalRuntime, but unary Invoke cannot start an invocation
    // for it. `axon_took_it` must stay false so `invoke()` records
    // the failed unary attempt through the manual ledger path
    // instead of assuming Axon's LedgerSink persisted a row.
    use easynet_axon::invocation::{make_ability, LocalRuntime};

    let _hg = crate::facade::cli::test_support::HomeGuard::new();
    let rt = LocalRuntime::new();
    let runtime_ability =
        crate::ura::owner_ability_ura(TEST_DAEMON_URI, "demo.stream_only").unwrap();
    rt.register_ability_with_options(
        runtime_ability,
        make_ability(|_ctx| async { Ok(Vec::new()) }),
        test_stream_options(),
    )
    .await
    .unwrap();

    let svc = make_service()
        .with_session_realm("test-realm")
        .with_local_runtime(Arc::clone(&rt));
    publish_test_route(&svc, TEST_DAEMON_URI, "demo.stream_only");

    let request = invoke_request("demo.stream_only", "{}").into_inner();
    let (result, axon_took_it) = svc
        .unary_dispatcher()
        .dispatch_local_rpc_selected_route(&request)
        .await;
    assert!(
        !axon_took_it,
        "mode mismatch happens before Axon starts an invocation"
    );
    let err = result.expect_err("stream-only ability rejects unary Invoke");
    assert_eq!(err.code(), tonic::Code::InvalidArgument);
    assert!(
        err.message().contains("does not support RPC Invoke"),
        "error must explain the call-shape mismatch, got: {err}"
    );
}
