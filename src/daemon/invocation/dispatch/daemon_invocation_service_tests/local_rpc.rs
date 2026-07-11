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
            crate::daemon::ability::DEFAULT_ABILITY_DESCRIPTOR_VERSION,
            [0x11; 32],
            [0x22; 32],
        )
}

fn test_stream_options() -> easynet_axon::invocation::AbilityOptions {
    use easynet_axon::invocation::{AbilityOptions, CallMode};
    AbilityOptions::streaming().with_mode_descriptor_proof(
        CallMode::Stream,
        crate::daemon::ability::DEFAULT_ABILITY_DESCRIPTOR_VERSION,
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
    use easynet_axon::invocation::{make_ability, LocalRuntime};

    let _hg = crate::cli::commands::test_support::HomeGuard::new();
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
    // Returns `(response, axon_took_it=true)` so the caller in
    // `invoke()` skips the manual `record_unary_invocation`
    // write (avoiding the duplicate row keyed by `request_id`).
    use easynet_axon::invocation::{make_ability, InvocationLedger, LedgerSink, LocalRuntime};

    let _hg = crate::cli::commands::test_support::HomeGuard::new();
    let temp = tempfile::tempdir().unwrap();
    let ledger = Arc::new(InvocationLedger::open(temp.path().join("inv.redb")).unwrap());
    let rt = LocalRuntime::new();
    rt.set_ledger_sink(LedgerSink::new(Arc::clone(&ledger)));
    let runtime_ability =
        crate::core::ura::owner_ability_ura(TEST_DAEMON_URA, "demo.unary_via_axon").unwrap();
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
    publish_test_route(&svc, TEST_DAEMON_URA, "demo.unary_via_axon");

    let mut request = invoke_request("demo.unary_via_axon", r#"{"k":"v"}"#).into_inner();
    let external_caller = "easynet:///r/test-realm/device/client-1";
    let signing_key = test_device_signing_key();
    request.envelope = Some(signed_test_envelope(
        external_caller,
        TEST_DAEMON_URA,
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
            crate::daemon::ability::DEFAULT_ABILITY_DESCRIPTOR_VERSION
        )
    );
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
async fn dispatch_local_rpc_selected_route_accepts_descriptor_ref_function_name() {
    use easynet_axon::invocation::{make_ability, LocalRuntime};

    let _hg = crate::cli::commands::test_support::HomeGuard::new();
    let rt = LocalRuntime::new();
    let ability = "demo.descriptor_bound_unary";
    let runtime_ability = crate::core::ura::owner_ability_ura(TEST_DAEMON_URA, ability).unwrap();
    rt.register_ability_with_options(
        runtime_ability,
        make_ability(|ctx| async move { Ok(ctx.payload.clone()) }),
        test_rpc_options(),
    )
    .await
    .unwrap();

    let svc = make_service()
        .with_session_realm("test-realm")
        .with_local_runtime(Arc::clone(&rt));
    publish_test_route(&svc, TEST_DAEMON_URA, ability);

    let descriptor_ref = test_descriptor_ref(TEST_DAEMON_URA, ability);
    let request = invoke_request(&descriptor_ref, r#"{"descriptor":"function-name"}"#).into_inner();
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
    use easynet_axon::invocation::{make_ability, InvocationLedger, LedgerSink, LocalRuntime};

    let _hg = crate::cli::commands::test_support::HomeGuard::new();
    let temp = tempfile::tempdir().unwrap();
    let ledger = Arc::new(InvocationLedger::open(temp.path().join("inv.redb")).unwrap());
    let rt = LocalRuntime::new();
    rt.set_ledger_sink(LedgerSink::new(Arc::clone(&ledger)));
    let runtime_ability =
        crate::core::ura::owner_ability_ura(TEST_DAEMON_URA, "demo.loopback_unsigned").unwrap();
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
    publish_test_route(&svc, TEST_DAEMON_URA, "demo.loopback_unsigned");

    let arguments = br#"{"k":"v"}"#.to_vec();
    let request = InvokeRequest {
        envelope: Some(
            ProtoEnvelope::targeted(
                crate::daemon::identity::local_invocation::LOCAL_SYSTEM_AGENT_URA,
                TEST_DAEMON_URA,
                "easynet:///r/test-realm/resource/camera-1",
            )
            .expect("valid loopback envelope")
            .into_inner(),
        ),
        function_name: test_descriptor_ref(TEST_DAEMON_URA, "demo.loopback_unsigned"),
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
        format!(
            "{runtime_ability}@{}",
            crate::daemon::ability::DEFAULT_ABILITY_DESCRIPTOR_VERSION
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
    let svc = std::sync::Arc::new(
        make_service()
            .with_session_realm("test-realm")
            .with_local_runtime(rt),
    );
    publish_test_route(svc.as_ref(), TEST_DAEMON_URA, ability);

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
                        ProtoEnvelope::targeted(
                            crate::daemon::identity::local_invocation::LOCAL_SYSTEM_AGENT_URA,
                            TEST_DAEMON_URA,
                            TEST_DAEMON_URA,
                        )
                        .expect("valid concurrency probe envelope")
                        .into_inner(),
                    ),
                    function_name: ability.to_string(),
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
    use easynet_axon::pb::axon::v1::invocation_client::InvocationClient;
    use easynet_axon::pb::axon::v1::invocation_server::InvocationServer;
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
    let service = make_service()
        .with_session_realm("test-realm")
        .with_local_runtime(rt);
    publish_test_route(&service, TEST_DAEMON_URA, ability);

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
                        ProtoEnvelope::targeted(
                            crate::daemon::identity::local_invocation::LOCAL_SYSTEM_AGENT_URA,
                            TEST_DAEMON_URA,
                            TEST_DAEMON_URA,
                        )
                        .expect("valid UDS concurrency probe envelope")
                        .into_inner(),
                    ),
                    function_name: ability.to_string(),
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
    // A device-owned ability is the device's own runtime authority
    // (RFC-005 D105): when the runtime does not host the dispatch key,
    // the resolver itself rejects with a typed NODATA negative — the
    // catalog row alone cannot manufacture a route. There is no
    // select-then-fail-at-executor window for self-owned abilities.
    use easynet_axon::invocation::LocalRuntime;

    let _hg = crate::cli::commands::test_support::HomeGuard::new();
    let rt = LocalRuntime::new();
    let svc = make_service()
        .with_session_realm("test-realm")
        .with_local_runtime(Arc::clone(&rt));
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

    let _hg = crate::cli::commands::test_support::HomeGuard::new();
    let rt = LocalRuntime::new();
    let runtime_ability =
        crate::core::ura::owner_ability_ura(TEST_DAEMON_URA, "demo.stream_only").unwrap();
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
