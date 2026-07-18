use super::*;

/// Descriptor proof a raw-runtime test ability must carry so Axon's
/// receipt-proof normalizer admits its dispatch (production stamps the
/// equivalent from the control-plane record). Non-zero stub hashes;
/// version is the default these owner-local test abilities dispatch under.
fn test_rpc_options() -> axon_sdk::invocation::AbilityOptions {
    use axon_sdk::invocation::{AbilityCallModes, AbilityOptions};
    AbilityOptions::default()
        .with_modes(AbilityCallModes::RPC)
        .with_descriptor_proof(
            crate::daemon::ability::DEFAULT_ABILITY_DESCRIPTOR_VERSION,
            "invoke",
            [0x33; 32],
            [0x11; 32],
            [0x22; 32],
        )
}

fn test_stream_options() -> axon_sdk::invocation::AbilityOptions {
    use axon_sdk::invocation::{AbilityOptions, CallMode};
    AbilityOptions::streaming().with_mode_descriptor_proof(
        CallMode::Stream,
        crate::daemon::ability::DEFAULT_ABILITY_DESCRIPTOR_VERSION,
        "invoke",
        [0x33; 32],
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
    use crate::daemon::persistence::agent_registry::{
        save_agents, AgentEntry, AgentRegistry, AgentType,
    };
    use crate::daemon::persistence::config::{save_credentials, Credentials};
    let _hg = crate::cli::commands::test_support::HomeGuard::new();
    save_credentials(&Credentials {
        node_id: "dev-1".to_string(),
        credential_token: "token".to_string(),
        hub_endpoint: "axon://hub.test:50051".to_string(),
        realm: "test-realm".to_string(),
        username: Some("dev".to_string()),
        user_id: Some("user-dev".to_string()),
        ..Default::default()
    })
    .expect("seed credentials");
    let svc = make_service().with_session_realm("test-realm");

    let agent_target = "easynet:///r/test-realm/agent/user-dev.liangbing";

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
            .matches_self_target_ura("easynet:///r/other-realm/agent/user-dev.liangbing")
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
            .matches_self_target_ura("easynet:///r/test-realm/agent/user-dev.unknown")
            .await,
        "slow tier must only accept agents present in agents.json"
    );
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn matches_self_target_ura_uses_exact_local_agents_identity() {
    use crate::daemon::persistence::local_agents::{save, upsert_hosted_agent, LocalAgentsFile};

    let _hg = crate::cli::commands::test_support::HomeGuard::new();
    let mut local = LocalAgentsFile {
        host_device_agent_ura: "easynet:///r/test-realm/device/dev-1".to_string(),
        hosted_agents: Vec::new(),
    };
    upsert_hosted_agent(
        &mut local,
        "llm",
        "liangbing",
        "easynet:///r/test-realm/agent/user-dev.liangbing",
    );
    save(&local).expect("seed local-agents.json");

    let svc = make_service().with_session_realm("test-realm");
    assert!(
        svc.target_gate()
            .matches_self_target_ura("easynet:///r/test-realm/agent/user-dev.liangbing")
            .await,
        "exact hosted Agent identity from local-agents.json must be local"
    );
    assert!(
        !svc.target_gate()
            .matches_self_target_ura("easynet:///r/other-realm/agent/user-dev.liangbing")
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
async fn axon_arm_must_not_intercept_calls_targeting_a_peer_device() {
    // **Phase 4 regression pin.**
    //
    // Without the `matches_self_target_ura` guard the Axon
    // arm intercepts every call whose ability is registered
    // locally, regardless of `subject_device`. That caused
    // a canonical Invoke targeting a peer device to return THIS daemon's
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
    use axon_sdk::invocation::make_ability;

    let _hg = crate::cli::commands::test_support::HomeGuard::new();
    let runtime_assembly = test_runtime_with_default_trust();
    let rt = runtime_assembly.runtime();
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

    let svc = make_service_with_test_runtime(runtime_assembly).with_session_realm("test-realm");

    // 1. THIS daemon's URA → self target.
    assert!(
        svc.target_gate()
            .matches_self_target_ura(TEST_DAEMON_URA)
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
    let peer_realm_hub = crate::core::ura::hub_ura("other-realm");
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
    // Returns `(response, runtime_started=true)` after the runtime has
    // produced the canonical terminal receipt. The daemon only projects
    // that outcome; LedgerSink owns the single durable row.
    use axon_sdk::invocation::{make_ability, InvocationLedger, LedgerSink};

    let _hg = crate::cli::commands::test_support::HomeGuard::new();
    let temp = tempfile::tempdir().unwrap();
    let ledger = Arc::new(InvocationLedger::open(temp.path().join("inv.redb")).unwrap());
    let runtime_assembly = test_runtime_with_default_trust();
    let rt = runtime_assembly.runtime();
    rt.set_ledger_sink(LedgerSink::new(Arc::clone(&ledger)));
    let runtime_ability =
        crate::core::ura::owner_ability_ura(TEST_DAEMON_URA, "demo.unary_via_axon").unwrap();
    rt.register_ability_with_options(
        runtime_ability.clone(),
        make_ability(|ctx| async move {
            let subject = ctx
                .signed_envelope()
                .map(|signed| signed.envelope.subject.ura.clone());
            serde_json::to_vec(&serde_json::json!({
                "payload": serde_json::from_slice::<serde_json::Value>(&ctx.payload)
                    .unwrap_or(serde_json::Value::Null),
                "subject": subject,
            }))
            .map_err(|err| axon_sdk::invocation::AxonError::internal(err.to_string()))
        }),
        test_rpc_options(),
    )
    .await
    .unwrap();

    let svc = make_service_with_test_runtime(runtime_assembly).with_session_realm("test-realm");
    publish_test_route(&svc, TEST_DAEMON_URA, "demo.unary_via_axon");
    sync_runtime_proof_from_catalog(
        &svc,
        TEST_DAEMON_URA,
        "demo.unary_via_axon",
        crate::daemon::ability::CallMode::Rpc,
    )
    .await;

    let mut request = invoke_request("demo.unary_via_axon", r#"{"k":"v"}"#).into_inner();
    let external_caller = "easynet:///r/test-realm/device/client-1";
    let signing_key = test_device_signing_key();
    let descriptor_ref = catalog_test_descriptor_ref(
        svc.directory.local_ability_catalog.as_ref().unwrap(),
        TEST_DAEMON_URA,
        "demo.unary_via_axon",
        crate::daemon::ability::CallMode::Rpc,
    );
    bind_invoke_request_to_descriptor_ref(
        &mut request,
        external_caller,
        TEST_DAEMON_URA,
        "easynet:///r/test-realm/resource/camera-1",
        descriptor_ref.clone(),
        &signing_key,
    );
    let (result, runtime_started) = tokio::time::timeout(
        std::time::Duration::from_secs(2),
        svc.unary_dispatcher()
            .dispatch_local_rpc_selected_route(&request),
    )
    .await
    .expect("local runtime dispatch must reach a terminal outcome");

    assert!(
        runtime_started,
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

    // LedgerSink writes on the runtime task after terminal receipt emission.
    // Poll within a hard deadline instead of assuming scheduler pacing.
    let records = tokio::time::timeout(std::time::Duration::from_secs(2), async {
        loop {
            let records = ledger.list_all().expect("list ledger");
            if !records.is_empty() {
                break records;
            }
            tokio::time::sleep(std::time::Duration::from_millis(10)).await;
        }
    })
    .await
    .expect("Axon LedgerSink must persist the terminal record");
    assert_eq!(
        records.len(),
        1,
        "Axon-routed unary call must land exactly one ledger row"
    );
    assert_eq!(records[0].ability_name, descriptor_ref);
    assert_eq!(records[0].state, "completed");
    assert_eq!(
        records[0].caller_ura, external_caller,
        "Axon-routed unary ledger row must preserve the external-signed wire caller"
    );
    assert_eq!(
        records[0].callee_ura, TEST_DAEMON_URA,
        "Axon-routed unary ledger row must preserve the external-signed wire callee"
    );
    assert_eq!(
        records[0].subject_ura, "easynet:///r/test-realm/resource/camera-1",
        "Axon-routed unary ledger row must preserve the external-signed wire subject"
    );
    assert_eq!(header_request_id, Some(records[0].request_id.as_str()));
}

#[tokio::test]
async fn dispatch_local_rpc_terminal_failure_stays_in_band_with_receipts() {
    use axon_sdk::invocation::{make_ability, AxonError, InvocationState};

    let _hg = crate::cli::commands::test_support::HomeGuard::new();
    let runtime_assembly = test_runtime_with_default_trust();
    let rt = runtime_assembly.runtime();
    let ability = "demo.terminal_failure";
    let runtime_ability = crate::core::ura::owner_ability_ura(TEST_DAEMON_URA, ability).unwrap();
    rt.register_ability_with_options(
        runtime_ability,
        make_ability(|_| async move {
            Err::<Vec<u8>, _>(AxonError::invalid_argument("expected handler failure"))
        }),
        test_rpc_options(),
    )
    .await
    .unwrap();

    let svc = make_service_with_test_runtime(runtime_assembly).with_session_realm("test-realm");
    publish_test_route(&svc, TEST_DAEMON_URA, ability);
    sync_runtime_proof_from_catalog(
        &svc,
        TEST_DAEMON_URA,
        ability,
        crate::daemon::ability::CallMode::Rpc,
    )
    .await;
    let mut request = invoke_request(ability, r#"{}"#).into_inner();
    let descriptor_ref = catalog_test_descriptor_ref(
        svc.directory.local_ability_catalog.as_ref().unwrap(),
        TEST_DAEMON_URA,
        ability,
        crate::daemon::ability::CallMode::Rpc,
    );
    bind_invoke_request_to_descriptor_ref(
        &mut request,
        "easynet:///r/test-realm/device/client-1",
        TEST_DAEMON_URA,
        "easynet:///r/test-realm/resource/failure-probe",
        descriptor_ref,
        &test_device_signing_key(),
    );

    let (result, runtime_started) = svc
        .unary_dispatcher()
        .dispatch_local_rpc_selected_route(&request)
        .await;

    assert!(runtime_started);
    let body = result
        .expect("accepted terminal failure must be an InvokeResponse")
        .into_inner();
    assert_eq!(body.state, InvocationState::Failed.to_wire_i32());
    let error = body.error.expect("failed terminal response error");
    assert_eq!(error.message, "expected handler failure");
    let admission = body
        .admission_receipt
        .expect("failed terminal response admission receipt");
    let terminal = body
        .terminal_receipt
        .expect("failed terminal response terminal receipt");
    assert_eq!(admission.invocation_id, terminal.invocation_id);
    assert_eq!(terminal.state, InvocationState::Failed.to_wire_i32());
    assert_eq!(
        body.header
            .as_ref()
            .map(|header| header.request_id.as_str()),
        Some(terminal.invocation_id.as_str())
    );
}

#[tokio::test]
async fn dispatch_local_rpc_selected_route_accepts_descriptor_ref_function_name() {
    use axon_sdk::invocation::make_ability;

    let _hg = crate::cli::commands::test_support::HomeGuard::new();
    let runtime_assembly = test_runtime_with_default_trust();
    let rt = runtime_assembly.runtime();
    let ability = "demo.descriptor_bound_unary";
    let runtime_ability = crate::core::ura::owner_ability_ura(TEST_DAEMON_URA, ability).unwrap();
    rt.register_ability_with_options(
        runtime_ability,
        make_ability(|ctx| async move { Ok(ctx.payload.clone()) }),
        test_rpc_options(),
    )
    .await
    .unwrap();

    let svc = make_service_with_test_runtime(runtime_assembly).with_session_realm("test-realm");
    publish_test_route(&svc, TEST_DAEMON_URA, ability);
    sync_runtime_proof_from_catalog(
        &svc,
        TEST_DAEMON_URA,
        ability,
        crate::daemon::ability::CallMode::Rpc,
    )
    .await;

    let descriptor_ref = catalog_test_descriptor_ref(
        svc.directory.local_ability_catalog.as_ref().unwrap(),
        TEST_DAEMON_URA,
        ability,
        crate::daemon::ability::CallMode::Rpc,
    );
    let mut request = invoke_request(ability, r#"{"descriptor":"function-name"}"#).into_inner();
    bind_invoke_request_to_descriptor_ref(
        &mut request,
        TEST_DAEMON_URA,
        TEST_DAEMON_URA,
        TEST_DAEMON_URA,
        descriptor_ref,
        &test_device_signing_key(),
    );
    let (result, axon_took_it) = svc
        .unary_dispatcher()
        .dispatch_local_rpc_selected_route(&request)
        .await;

    assert!(axon_took_it, "descriptor-ref route must reach Axon");
    let response = result.expect("descriptor-ref function_name dispatches");
    assert_eq!(response.into_inner().result, request.arguments);
}

#[tokio::test]
async fn dispatch_local_rpc_selected_route_accepts_unsigned_loopback_request() {
    use axon_sdk::invocation::{make_ability, InvocationLedger, LedgerSink};

    let _hg = crate::cli::commands::test_support::HomeGuard::new();
    let temp = tempfile::tempdir().unwrap();
    let ledger = Arc::new(InvocationLedger::open(temp.path().join("inv.redb")).unwrap());
    let runtime_assembly = test_runtime_with_default_trust();
    let rt = runtime_assembly.runtime();
    rt.set_ledger_sink(LedgerSink::new(Arc::clone(&ledger)));
    let runtime_ability =
        crate::core::ura::owner_ability_ura(TEST_DAEMON_URA, "demo.loopback_unsigned").unwrap();
    rt.register_ability_with_options(
        runtime_ability.clone(),
        make_ability(|ctx| async move {
            let envelope = ctx
                .signed_envelope()
                .expect("runtime stores descriptor-bound envelope")
                .clone()
                .envelope;
            serde_json::to_vec(&serde_json::json!({
                "caller": envelope.caller.ura,
                "callee": envelope.callee.ura,
                "subject": envelope.subject.ura,
                "payload": serde_json::from_slice::<serde_json::Value>(&ctx.payload)
                    .unwrap_or(serde_json::Value::Null),
            }))
            .map_err(|err| axon_sdk::invocation::AxonError::internal(err.to_string()))
        }),
        test_rpc_options(),
    )
    .await
    .unwrap();

    let svc = make_service_with_test_runtime(runtime_assembly).with_session_realm("test-realm");
    publish_test_route(&svc, TEST_DAEMON_URA, "demo.loopback_unsigned");
    sync_runtime_proof_from_catalog(
        &svc,
        TEST_DAEMON_URA,
        "demo.loopback_unsigned",
        crate::daemon::ability::CallMode::Rpc,
    )
    .await;

    let arguments = br#"{"k":"v"}"#.to_vec();
    let descriptor_ref = catalog_test_descriptor_ref(
        svc.directory.local_ability_catalog.as_ref().unwrap(),
        TEST_DAEMON_URA,
        "demo.loopback_unsigned",
        crate::daemon::ability::CallMode::Rpc,
    );
    let request = InvokeRequest {
        envelope: Some(
            ProtoEnvelope::from_target(
                crate::daemon::identity::local_invocation::LOCAL_SYSTEM_AGENT_URA,
                TEST_DAEMON_URA,
                "easynet:///r/test-realm/resource/camera-1",
                InvocationDerivationPolicy::FreshRoot,
            )
            .expect("valid loopback envelope")
            .into_inner("demo.loopback_unsigned", &arguments)
            .expect("complete loopback tuple"),
        ),
        target: Some(
            wire_invocation_target(&descriptor_ref, "demo.loopback_unsigned")
                .expect("typed descriptor target"),
        ),
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
        crate::daemon::identity::local_invocation::LOCAL_SYSTEM_AGENT_URA
    );
    assert_eq!(decoded["callee"], TEST_DAEMON_URA);
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
        catalog_test_descriptor_ref(
            svc.directory.local_ability_catalog.as_ref().unwrap(),
            TEST_DAEMON_URA,
            "demo.loopback_unsigned",
            crate::daemon::ability::CallMode::Rpc,
        )
    );
    assert_eq!(
        records[0].caller_ura,
        crate::daemon::identity::local_invocation::LOCAL_SYSTEM_AGENT_URA,
        "loopback dispatch must be signed by daemon-local system identity"
    );
    assert_eq!(records[0].callee_ura, TEST_DAEMON_URA);
    assert_eq!(
        records[0].subject_ura,
        "easynet:///r/test-realm/resource/camera-1"
    );
}

#[tokio::test(flavor = "multi_thread", worker_threads = 16)]
#[ignore = "load probe; set EASYNET_INVOCATION_CONCURRENCY_PROBE to override request count"]
async fn simple_local_rpc_invocation_concurrency_probe() {
    use tokio::task::JoinSet;

    fn percentile(sorted: &[u128], pct: usize) -> u128 {
        if sorted.is_empty() {
            return 0;
        }
        let idx = ((sorted.len() - 1) * pct) / 100;
        sorted[idx]
    }

    let _hg = crate::cli::commands::test_support::HomeGuard::new();
    let count = std::env::var("EASYNET_INVOCATION_CONCURRENCY_PROBE")
        .ok()
        .and_then(|raw| raw.parse::<usize>().ok())
        .filter(|value| *value > 0)
        .unwrap_or(10_000);
    let timeout = std::env::var("EASYNET_INVOCATION_CONCURRENCY_TIMEOUT_SECS")
        .ok()
        .and_then(|raw| raw.parse::<u64>().ok())
        .filter(|value| *value > 0)
        .map(std::time::Duration::from_secs)
        .unwrap_or_else(|| std::time::Duration::from_secs(60));

    let ability = "probe.concurrent_echo";
    let rt =
        runtime_with_json_echo(TEST_DAEMON_URA, ability, "handled_by", "concurrency-probe").await;
    let svc =
        std::sync::Arc::new(make_service_with_test_runtime(rt).with_session_realm("test-realm"));
    publish_test_route(svc.as_ref(), TEST_DAEMON_URA, ability);
    sync_runtime_proof_from_catalog(
        svc.as_ref(),
        TEST_DAEMON_URA,
        ability,
        crate::daemon::ability::CallMode::Rpc,
    )
    .await;

    let mut tasks = JoinSet::new();
    let started = std::time::Instant::now();
    for seq in 0..count {
        let svc = std::sync::Arc::clone(&svc);
        tasks.spawn(async move {
            let request_started = std::time::Instant::now();
            let arguments = format!(r#"{{"seq":{seq}}}"#).into_bytes();
            let response = svc
                .invoke(Request::new(InvokeRequest {
                    envelope: Some(
                        ProtoEnvelope::from_target(
                            crate::daemon::identity::local_invocation::LOCAL_SYSTEM_AGENT_URA,
                            TEST_DAEMON_URA,
                            TEST_DAEMON_URA,
                            InvocationDerivationPolicy::FreshRoot,
                        )
                        .expect("valid concurrency probe envelope")
                        .into_inner(ability, &arguments)
                        .expect("complete concurrency probe tuple"),
                    ),
                    target: Some(test_invocation_target(ability)),
                    arguments,
                    ..InvokeRequest::default()
                }))
                .await
                .map_err(|err| format!("seq={seq}: invoke failed: {err}"))?;
            let body = response.into_inner();
            let decoded: serde_json::Value = serde_json::from_slice(&body.result)
                .map_err(|err| format!("seq={seq}: decode response failed: {err}"))?;
            if decoded["handled_by"] != "concurrency-probe" {
                return Err(format!(
                    "seq={seq}: wrong handler marker in response: {decoded}"
                ));
            }
            if decoded["echoed_args"]["seq"].as_u64() != Some(seq as u64) {
                return Err(format!(
                    "seq={seq}: wrong echoed seq in response: {decoded}"
                ));
            }
            Ok::<u128, String>(request_started.elapsed().as_micros())
        });
    }

    let mut latencies_us = Vec::with_capacity(count);
    let mut errors = Vec::new();
    let deadline = tokio::time::Instant::now() + timeout;
    while let Some(joined) = match tokio::time::timeout_at(deadline, tasks.join_next()).await {
        Ok(joined) => joined,
        Err(_) => {
            tasks.abort_all();
            panic!(
                "concurrency probe timed out after {:?}: completed={} expected={} first_errors={:?}",
                timeout,
                latencies_us.len(),
                count,
                errors
            );
        }
    } {
        match joined {
            Ok(Ok(latency)) => latencies_us.push(latency),
            Ok(Err(err)) => {
                if errors.len() < 8 {
                    errors.push(err);
                }
            }
            Err(err) => {
                if errors.len() < 8 {
                    errors.push(format!("join failed: {err}"));
                }
            }
        }
    }

    latencies_us.sort_unstable();
    let elapsed = started.elapsed();
    let summary = format!(
        "simple_local_rpc_invocation_concurrency_probe count={} ok={} failed={} elapsed_ms={} throughput_per_s={:.1} p50_us={} p95_us={} p99_us={}",
        count,
        latencies_us.len(),
        count.saturating_sub(latencies_us.len()),
        elapsed.as_millis(),
        count as f64 / elapsed.as_secs_f64(),
        percentile(&latencies_us, 50),
        percentile(&latencies_us, 95),
        percentile(&latencies_us, 99),
    );
    if let Ok(path) = std::env::var("EASYNET_INVOCATION_CONCURRENCY_SUMMARY_PATH") {
        std::fs::write(path, format!("{summary}\n")).expect("write concurrency probe summary");
    }
    eprintln!("{summary}");

    assert!(errors.is_empty(), "first invocation errors: {errors:?}");
    assert_eq!(latencies_us.len(), count);
}

#[cfg(unix)]
#[tokio::test(flavor = "multi_thread", worker_threads = 16)]
#[ignore = "load probe; set EASYNET_INVOCATION_TRANSPORT_PROBE to override request count"]
async fn simple_uds_invocation_concurrency_probe() {
    use axon_sdk::pb::axon::v1::invocation_client::InvocationClient;
    use axon_sdk::pb::axon::v1::invocation_server::InvocationServer;
    use tokio::task::JoinSet;
    use tokio_stream::wrappers::UnixListenerStream;
    use tonic::transport::{Endpoint, Server, Uri};

    fn percentile(sorted: &[u128], pct: usize) -> u128 {
        if sorted.is_empty() {
            return 0;
        }
        let idx = ((sorted.len() - 1) * pct) / 100;
        sorted[idx]
    }

    let _hg = crate::cli::commands::test_support::HomeGuard::new();
    let count = std::env::var("EASYNET_INVOCATION_TRANSPORT_PROBE")
        .ok()
        .and_then(|raw| raw.parse::<usize>().ok())
        .filter(|value| *value > 0)
        .unwrap_or(10_000);
    let timeout = std::env::var("EASYNET_INVOCATION_TRANSPORT_TIMEOUT_SECS")
        .ok()
        .and_then(|raw| raw.parse::<u64>().ok())
        .filter(|value| *value > 0)
        .map(std::time::Duration::from_secs)
        .unwrap_or_else(|| std::time::Duration::from_secs(120));

    let ability = "probe.uds_concurrent_echo";
    let rt = runtime_with_json_echo(
        TEST_DAEMON_URA,
        ability,
        "handled_by",
        "uds-concurrency-probe",
    )
    .await;
    let service = make_service_with_test_runtime(rt).with_session_realm("test-realm");
    publish_test_route(&service, TEST_DAEMON_URA, ability);
    sync_runtime_proof_from_catalog(
        &service,
        TEST_DAEMON_URA,
        ability,
        crate::daemon::ability::CallMode::Rpc,
    )
    .await;

    let temp = tempfile::tempdir().expect("temp UDS dir");
    let socket_path = temp.path().join("invocation-probe.sock");
    let listener = tokio::net::UnixListener::bind(&socket_path).expect("bind probe UDS");
    let incoming = UnixListenerStream::new(listener);
    let (shutdown_tx, shutdown_rx) = tokio::sync::oneshot::channel::<()>();
    let max_streams = u32::try_from(count).unwrap_or(u32::MAX);
    let server = tokio::spawn(async move {
        Server::builder()
            .max_concurrent_streams(Some(max_streams))
            .concurrency_limit_per_connection(count)
            .add_service(InvocationServer::new(service))
            .serve_with_incoming_shutdown(incoming, async {
                let _ = shutdown_rx.await;
            })
            .await
            .expect("serve probe UDS")
    });

    let connector_path = socket_path.clone();
    let channel = Endpoint::try_from("http://[::]:50051")
        .expect("dummy endpoint")
        .connect_with_connector(tower::service_fn(move |_: Uri| {
            let path = connector_path.clone();
            async move {
                let stream = tokio::net::UnixStream::connect(path).await?;
                Ok::<_, std::io::Error>(hyper_util::rt::TokioIo::new(stream))
            }
        }))
        .await
        .expect("connect probe UDS");
    let client = InvocationClient::new(channel);

    let mut tasks = JoinSet::new();
    let started = std::time::Instant::now();
    for seq in 0..count {
        let mut client = client.clone();
        tasks.spawn(async move {
            let request_started = std::time::Instant::now();
            let arguments = format!(r#"{{"seq":{seq}}}"#).into_bytes();
            let response = client
                .invoke(Request::new(InvokeRequest {
                    envelope: Some(
                        ProtoEnvelope::from_target(
                            crate::daemon::identity::local_invocation::LOCAL_SYSTEM_AGENT_URA,
                            TEST_DAEMON_URA,
                            TEST_DAEMON_URA,
                            InvocationDerivationPolicy::FreshRoot,
                        )
                        .expect("valid UDS concurrency probe envelope")
                        .into_inner(ability, &arguments)
                        .expect("complete UDS concurrency probe tuple"),
                    ),
                    target: Some(test_invocation_target(ability)),
                    arguments,
                    ..InvokeRequest::default()
                }))
                .await
                .map_err(|err| format!("seq={seq}: UDS invoke failed: {err}"))?;
            let body = response.into_inner();
            let decoded: serde_json::Value = serde_json::from_slice(&body.result)
                .map_err(|err| format!("seq={seq}: decode response failed: {err}"))?;
            if decoded["handled_by"] != "uds-concurrency-probe" {
                return Err(format!(
                    "seq={seq}: wrong handler marker in response: {decoded}"
                ));
            }
            if decoded["echoed_args"]["seq"].as_u64() != Some(seq as u64) {
                return Err(format!(
                    "seq={seq}: wrong echoed seq in response: {decoded}"
                ));
            }
            Ok::<u128, String>(request_started.elapsed().as_micros())
        });
    }

    let mut latencies_us = Vec::with_capacity(count);
    let mut errors = Vec::new();
    let deadline = tokio::time::Instant::now() + timeout;
    while let Some(joined) = match tokio::time::timeout_at(deadline, tasks.join_next()).await {
        Ok(joined) => joined,
        Err(_) => {
            tasks.abort_all();
            let _ = shutdown_tx.send(());
            server.abort();
            panic!(
                "UDS concurrency probe timed out after {:?}: completed={} expected={} first_errors={:?}",
                timeout,
                latencies_us.len(),
                count,
                errors
            );
        }
    } {
        match joined {
            Ok(Ok(latency)) => latencies_us.push(latency),
            Ok(Err(err)) => {
                if errors.len() < 8 {
                    errors.push(err);
                }
            }
            Err(err) => {
                if errors.len() < 8 {
                    errors.push(format!("join failed: {err}"));
                }
            }
        }
    }

    let _ = shutdown_tx.send(());
    server.await.expect("probe UDS server task joins");

    latencies_us.sort_unstable();
    let elapsed = started.elapsed();
    let summary = format!(
        "simple_uds_invocation_concurrency_probe count={} ok={} failed={} elapsed_ms={} throughput_per_s={:.1} p50_us={} p95_us={} p99_us={}",
        count,
        latencies_us.len(),
        count.saturating_sub(latencies_us.len()),
        elapsed.as_millis(),
        count as f64 / elapsed.as_secs_f64(),
        percentile(&latencies_us, 50),
        percentile(&latencies_us, 95),
        percentile(&latencies_us, 99),
    );
    if let Ok(path) = std::env::var("EASYNET_INVOCATION_TRANSPORT_SUMMARY_PATH") {
        std::fs::write(path, format!("{summary}\n")).expect("write UDS concurrency probe summary");
    }
    eprintln!("{summary}");

    assert!(errors.is_empty(), "first UDS invocation errors: {errors:?}");
    assert_eq!(latencies_us.len(), count);
}

#[tokio::test]
async fn dispatch_local_rpc_selected_route_rejects_when_runtime_misses() {
    // This test deliberately injects a stale selected route without a
    // matching LocalRuntime handler. Dispatch must reject that inconsistent
    // projection as NotFound and must not start an Axon invocation.
    let _hg = crate::cli::commands::test_support::HomeGuard::new();
    let runtime_assembly = test_runtime_with_default_trust();
    let svc = make_service_with_test_runtime(runtime_assembly).with_session_realm("test-realm");
    publish_test_route(&svc, TEST_DAEMON_URA, "missing.ability");

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
    assert_eq!(err.code(), tonic::Code::NotFound);
    assert!(
        err.message().contains("selected route")
            && err.message().contains("missing.ability")
            && err
                .message()
                .contains("not registered in Axon LocalRuntime"),
        "error must name the stale route and missing runtime binding, got: {err}"
    );
}

#[tokio::test]
async fn selected_route_binding_rejects_removed_control_plane_record_even_with_runtime_row() {
    use axon_sdk::invocation::{make_ability, CallMode};

    let _hg = crate::cli::commands::test_support::HomeGuard::new();
    let runtime_assembly = test_runtime_with_default_trust();
    let rt = runtime_assembly.runtime();
    let ability = "demo.stale_catalog_proof";
    let runtime_ability =
        crate::core::ura::owner_ability_ura(TEST_DAEMON_URA, ability).expect("runtime ability URA");
    rt.register_ability_with_options(
        runtime_ability.clone(),
        make_ability(|ctx| async move { Ok(ctx.payload.clone()) }),
        test_rpc_options(),
    )
    .await
    .unwrap();

    let svc = make_service_with_test_runtime(runtime_assembly).with_session_realm("test-realm");
    publish_test_route(&svc, TEST_DAEMON_URA, ability);
    sync_runtime_proof_from_catalog(
        &svc,
        TEST_DAEMON_URA,
        ability,
        crate::daemon::ability::CallMode::Rpc,
    )
    .await;
    let selection = svc
        .target_gate()
        .route_resolver()
        .await
        .resolve_canonical_route(TEST_DAEMON_URA, &runtime_ability, CallMode::Rpc)
        .expect("selected route resolves before catalog removal");
    let selected_route = match selection.into_dispatch() {
        crate::daemon::invocation::routing::route_resolver::CanonicalRouteDispatch::Local(
            route,
        ) => route,
        crate::daemon::invocation::routing::route_resolver::CanonicalRouteDispatch::Peer(_) => {
            panic!("test route should dispatch locally")
        }
    };
    let catalog = svc
        .directory
        .local_ability_catalog
        .as_ref()
        .expect("test service has local ability catalog");
    assert!(catalog.remove_control_plane_record_for_authority_mode(
        TEST_DAEMON_URA,
        ability,
        crate::daemon::ability::CallMode::Rpc,
    ));
    assert!(
        rt.ability_options(&runtime_ability).await.is_some(),
        "runtime row remains installed after live catalog row removal"
    );

    let err =
        crate::daemon::invocation::dispatch::descriptor_binding::RuntimeBoundAbility::from_selected_route(
            "test selected route",
            rt.as_ref(),
            Some(catalog.as_ref()),
            &selected_route,
            CallMode::Rpc,
        )
        .await
        .expect_err("selected route binding must fail without live catalog proof");
    assert_eq!(err.code(), tonic::Code::FailedPrecondition);
    assert!(
        err.message().contains("selected route")
            && err
                .message()
                .contains("no live control-plane descriptor proof")
            && err.message().contains(ability),
        "error must name missing live control-plane proof, got: {err}"
    );
}

#[tokio::test]
async fn dispatch_local_rpc_selected_route_returns_false_for_non_rpc_runtime_row() {
    // A registered stream/bidi-only ability is known to
    // LocalRuntime, but unary Invoke cannot start an invocation
    // for it. `axon_took_it` must stay false so `invoke()` records
    // the failed unary attempt through the manual ledger path
    // instead of assuming Axon's LedgerSink persisted a row.
    use axon_sdk::invocation::make_ability;

    let _hg = crate::cli::commands::test_support::HomeGuard::new();
    let runtime_assembly = test_runtime_with_default_trust();
    let rt = runtime_assembly.runtime();
    let runtime_ability =
        crate::core::ura::owner_ability_ura(TEST_DAEMON_URA, "demo.stream_only").unwrap();
    rt.register_ability_with_options(
        runtime_ability,
        make_ability(|_ctx| async { Ok(Vec::new()) }),
        test_stream_options(),
    )
    .await
    .unwrap();

    let svc = make_service_with_test_runtime(runtime_assembly).with_session_realm("test-realm");
    publish_test_route(&svc, TEST_DAEMON_URA, "demo.stream_only");

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
