use super::*;

#[test]
fn quota_meters_user_abilities_but_exempts_control_plane() {
    assert!(quota_meters_function("observe.health"));
    assert!(quota_meters_function("agent.todo.run"));

    assert!(!quota_meters_function(ABILITY_FEDERATION_HEARTBEAT));
    assert!(!quota_meters_function(ABILITY_FEDERATION_FORWARD_INVOKE));
    assert!(!quota_meters_function(ABILITY_NAMESPACE_RESOLVE));
    assert!(!quota_meters_function(ABILITY_SELF_REGISTER_DEVICE_PUBKEY));
    assert!(!quota_meters_function(ABILITY_SELF_SESSION));

    assert!(
        quota_meters_function("federation.user_owned_probe"),
        "quota exemptions must be exact system abilities, not namespace prefixes"
    );
    assert!(
        quota_meters_function("<self>.user_owned_probe"),
        "a user-registered reserved-prefix ability must not bypass quota by spelling alone"
    );
}

#[test]
fn quota_for_forward_invoke_meters_inner_user_ability_only() {
    let user_call = InvokeRequest {
        function_name: ABILITY_FEDERATION_FORWARD_INVOKE.to_string(),
        arguments: forward_invoke_args_for_ability(
            "easynet:///r/test-realm/device/target",
            "observe.health",
            serde_json::json!({}),
        ),
        ..InvokeRequest::default()
    };
    assert_eq!(
        quota_metered_ability_for_request(&user_call)
            .expect("forward invoke parses")
            .as_deref(),
        Some("observe.health")
    );

    let control_call = InvokeRequest {
        function_name: ABILITY_FEDERATION_FORWARD_INVOKE.to_string(),
        arguments: forward_invoke_args_for_ability(
            &crate::ura::hub_ura("test-realm"),
            ABILITY_FEDERATION_HEARTBEAT,
            serde_json::json!({}),
        ),
        ..InvokeRequest::default()
    };
    assert_eq!(
        quota_metered_ability_for_request(&control_call).expect("forward invoke parses"),
        None,
        "nested federation control-plane calls stay quota-exempt"
    );

    let reserved_prefix_user_call = InvokeRequest {
        function_name: ABILITY_FEDERATION_FORWARD_INVOKE.to_string(),
        arguments: forward_invoke_args_for_ability(
            "easynet:///r/test-realm/device/target",
            "federation.user_owned_probe",
            serde_json::json!({}),
        ),
        ..InvokeRequest::default()
    };
    assert_eq!(
        quota_metered_ability_for_request(&reserved_prefix_user_call)
            .expect("forward invoke parses")
            .as_deref(),
        Some("federation.user_owned_probe"),
        "forward_invoke must not give quota amnesty to non-system reserved-prefix names"
    );
}

#[tokio::test]
async fn forward_invoke_quota_throttles_by_inner_user_ability() {
    let caller_ura = "easynet:///r/test-realm/device/quota-caller";
    let rt = runtime_with_json_echo(
        TEST_DAEMON_URI,
        "observe.health",
        "handled_by",
        "quota-test",
    )
    .await;
    let svc = make_quota_service_for_device_caller(caller_ura, 1).with_local_runtime(rt);
    publish_test_route(&svc, TEST_DAEMON_URI, "observe.health");
    let args = forward_invoke_args_for_ability(
        TEST_DAEMON_URI,
        "observe.health",
        serde_json::json!({"probe": true}),
    );

    let first = svc
        .invoke(invoke_request_from_device(
            caller_ura,
            ABILITY_FEDERATION_FORWARD_INVOKE,
            args.clone(),
        ))
        .await
        .expect("first forwarded user ability is within quota");
    let info = first
        .get_ref()
        .rate_limit
        .as_ref()
        .expect("forward_invoke response carries inner ability quota status");
    assert_eq!(info.quota_limit, 1);
    assert_eq!(info.quota_remaining, 0);

    let second = svc
        .invoke(invoke_request_from_device(
            caller_ura,
            ABILITY_FEDERATION_FORWARD_INVOKE,
            args,
        ))
        .await
        .expect_err("second forwarded user ability exhausts quota");
    assert_eq!(second.code(), tonic::Code::ResourceExhausted);
    assert!(
        second.message().contains("ability=observe.health"),
        "quota error must name the inner user ability, got: {}",
        second.message()
    );
}

#[tokio::test]
async fn invoke_dispatches_federation_join_to_wrapper() {
    let svc = make_service();
    let resp = svc
        .invoke(invoke_request(
            ABILITY_FEDERATION_JOIN,
            r#"{"membership_ura":"easynet:///r/realm/device/n1","realm":"realm"}"#,
        ))
        .await
        .expect("dispatch returns Ok");
    let body: federation_wrappers::JoinResponse = parse_response_body(resp);
    assert_eq!(body.membership_ura, "easynet:///r/realm/device/n1");
    assert_eq!(body.realm, "realm");
    assert_eq!(body.join_receipt_hash.len(), 64);
}

#[tokio::test]
async fn invoke_dispatches_federation_advertise_agent() {
    let svc = make_service();
    let resp = svc
        .invoke(invoke_request(
            ABILITY_FEDERATION_ADVERTISE_AGENT,
            r#"{"agent_ura":"easynet:///r/realm/device/n1"}"#,
        ))
        .await
        .expect("dispatch returns Ok");
    let body: federation_wrappers::AdvertiseAgentResponse = parse_response_body(resp);
    assert!(body.ack);
    assert!(!body.replaced_prior);
}

#[tokio::test]
async fn invoke_dispatches_federation_heartbeat() {
    let svc = make_service();
    let resp = svc
        .invoke(invoke_request(
            ABILITY_FEDERATION_HEARTBEAT,
            r#"{"agent_ura":"easynet:///r/realm/device/n1"}"#,
        ))
        .await
        .expect("dispatch returns Ok");
    let body: federation_wrappers::HeartbeatResponse = parse_response_body(resp);
    assert_eq!(body.membership_status, "active");
    assert_eq!(body.realm_directory_size, 0);
}

#[test]
fn session_control_heartbeat_renews_caller_owner_projection_lease() {
    let svc = make_service();
    let owner_ura = TEST_DAEMON_URI;
    let public_name = "agent.list";
    let ability_ura = crate::ura::owner_ability_ura(owner_ura, public_name).expect("ability ura");
    svc.directory.ability_catalog.upsert_projection(
        crate::services::ability_catalog_store::OwnerAbilityProjectionRow::new(
            owner_ura.to_string(),
            owner_ura.to_string(),
            1,
            "sha256:test".to_string(),
            1,
            vec![crate::runtime::owner_projection::AbilityProjectionSummary {
                ability_ura: ability_ura.clone(),
                owner_ura: owner_ura.to_string(),
                namespace: "agent".to_string(),
                local_name: "list".to_string(),
                descriptor_revision: "sha256:descriptor".to_string(),
                schema_ref: None,
                schema_hash: None,
                policy_ref: "visibility:PUBLIC".to_string(),
                route_summary_ref: Some(format!("route-ref::{ability_ura}")),
                tags: vec!["class:unary".to_string()],
                callable_summary: crate::runtime::owner_projection::AbilityCallableSummary::minimal(
                    public_name.to_string(),
                ),
            }],
        ),
    );

    assert!(
        svc.directory.ability_catalog.get_at(owner_ura, 2).is_none(),
        "test starts from an expired projection"
    );
    assert!(refresh_session_owner_projection_lease_at(
        &svc.bidi_dispatcher(),
        owner_ura,
        2
    ));

    let row = svc
        .directory
        .ability_catalog
        .projection_for_owner(owner_ura)
        .expect("projection still stored");
    assert_eq!(row.projection_revision(), 1);
    assert_eq!(row.projection_digest(), "sha256:test");
    assert!(row.lease_expires_unix_ms() > 2);
    assert!(
        svc.directory.ability_catalog.get_at(owner_ura, 2).is_some(),
        "refreshed projection is visible to namespace.resolve again"
    );
}

#[tokio::test]
async fn invoke_dispatches_federation_resolve_with_no_filter() {
    let svc = make_service();
    let resp = svc
        .invoke(invoke_request(ABILITY_FEDERATION_RESOLVE, "{}"))
        .await
        .expect("dispatch returns Ok");
    let body: federation_wrappers::ResolveResponse = parse_response_body(resp);
    assert!(body.agents.is_empty());
}

#[tokio::test]
async fn invoke_dispatches_namespace_resolve_to_typed_answer() {
    let svc = make_service();
    let owner_ura = TEST_DAEMON_URI;
    let ability_ura =
        crate::ura::owner_ability_ura(owner_ura, "agent.list").expect("device ability ura");
    svc.directory.presence.insert(owner_ura.to_string(), {
        let (tx, _rx) = mpsc::channel(1);
        tx
    });
    svc.directory.ability_catalog.upsert_projection(
        crate::services::ability_catalog_store::OwnerAbilityProjectionRow::new(
            owner_ura.to_string(),
            owner_ura.to_string(),
            1,
            "sha256:test".to_string(),
            4_102_444_800_000,
            vec![crate::runtime::owner_projection::AbilityProjectionSummary {
                ability_ura: ability_ura.clone(),
                owner_ura: owner_ura.to_string(),
                namespace: "agent".to_string(),
                local_name: "list".to_string(),
                descriptor_revision: "sha256:descriptor".to_string(),
                schema_ref: None,
                schema_hash: None,
                policy_ref: "visibility:PUBLIC".to_string(),
                route_summary_ref: Some(format!("route-ref::{ability_ura}")),
                tags: vec!["class:unary".to_string()],
                callable_summary: crate::runtime::owner_projection::AbilityCallableSummary::minimal(
                    "agent.list",
                ),
            }],
        ),
    );

    let resp = svc
        .invoke(invoke_request(
            ABILITY_NAMESPACE_RESOLVE,
            &serde_json::json!({
                "queryName": owner_ura,
                "qtype": "RESOLVE_TYPE_ROUTE",
                "abilityName": "agent.list",
            })
            .to_string(),
        ))
        .await
        .expect("namespace.resolve dispatch returns Ok");
    let body: serde_json::Value = parse_response_body(resp);

    assert_eq!(
        body["answerKind"],
        easynet_axon::pb::axon::v1::ResolveAnswerKind::FinalRoute.as_str_name()
    );
    assert_eq!(body["abilityUra"], ability_ura);
    assert_eq!(
        body["nextHop"]["localDeviceAbility"]["deviceUra"],
        TEST_DAEMON_URI
    );
}

#[tokio::test]
async fn namespace_resolve_cross_realm_route_returns_peer_hub_delegation() {
    let remote_owner = crate::ura::device_ura("remote-realm", "remote-device");
    let ability_ura =
        crate::ura::owner_ability_ura(&remote_owner, "observe.health").expect("ability ura");
    let svc = make_service()
        .with_session_realm("local-realm")
        .with_federated_peers(BTreeMap::from([(
            "remote-realm".to_string(),
            "https://remote-hub.example".to_string(),
        )]));

    let resp = svc
        .invoke(invoke_request(
            ABILITY_NAMESPACE_RESOLVE,
            &serde_json::json!({
                "queryName": ability_ura,
                "qtype": "RESOLVE_TYPE_ROUTE",
            })
            .to_string(),
        ))
        .await
        .expect("namespace.resolve dispatch returns Ok");
    let body: serde_json::Value = parse_response_body(resp);

    assert_eq!(
        body["answerKind"],
        easynet_axon::pb::axon::v1::ResolveAnswerKind::Delegation.as_str_name()
    );
    assert_eq!(body["ownerUra"], remote_owner);
    assert_eq!(body["nextHop"]["peerHub"]["realm"], "remote-realm");
    assert_eq!(
        body["nextHop"]["peerHub"]["hubUra"],
        crate::ura::hub_ura("remote-realm")
    );
    assert_eq!(
        body["nextHop"]["peerHub"]["endpoints"][0]["endpoint"],
        "https://remote-hub.example"
    );
    assert_eq!(
        body["nextHop"]["peerHub"]["endpoints"][0]["metadata"]["source"],
        "federated_peers"
    );
    assert_eq!(
        body["selectedRoute"]["reason"],
        easynet_axon::pb::axon::v1::RouteReason::PeerDelegation.as_str_name()
    );
}

#[tokio::test]
async fn invoke_writes_success_record_to_invocation_ledger() {
    let temp = tempfile::tempdir().expect("tempdir");
    let ledger = Arc::new(
        easynet_axon::invocation::InvocationLedger::open(
            temp.path().join("billing").join("invocations.redb"),
        )
        .expect("ledger"),
    );
    let svc = make_service().with_invocation_ledger(Arc::clone(&ledger));

    svc.invoke(invoke_request(ABILITY_FEDERATION_RESOLVE, "{}"))
        .await
        .expect("dispatch returns Ok");

    let records = ledger.list_all().expect("ledger list");
    assert_eq!(records.len(), 1);
    let record = &records[0];
    assert_eq!(record.caller_ura, TEST_DAEMON_URI);
    let expected_prefix =
        crate::ura::resource_dot_ura("test-realm", "device.test-daemon", "invocations/");
    assert!(record.invocation_ura.starts_with(&expected_prefix));
    assert!(!record.invocation_ura.contains("/resource/invocation."));
    assert_eq!(record.ability_name, ABILITY_FEDERATION_RESOLVE);
    // The ledger ability_ura now projects from the callee DEVICE binding
    // (where the self-target call actually executed), not the abstract hub
    // form — the truthful provenance of a locally-run invocation.
    assert_eq!(
        record.ability_ura,
        "easynet:///r/test-realm/ability/device.test-daemon.federation.resolve"
    );
    assert_eq!(record.state, "completed");
    assert_eq!(record.authority_form, "self");
    assert!(matches!(
        record.args,
        easynet_axon::invocation::LedgerEventPayload::Digest { .. }
    ));
    assert!(record.result.is_some());
}

#[tokio::test]
async fn invoke_writes_error_record_to_invocation_ledger() {
    let temp = tempfile::tempdir().expect("tempdir");
    let ledger = Arc::new(
        easynet_axon::invocation::InvocationLedger::open(
            temp.path().join("billing").join("invocations.redb"),
        )
        .expect("ledger"),
    );
    let svc = make_service().with_invocation_ledger(Arc::clone(&ledger));

    let err = svc
        .invoke(invoke_request("unknown.ability", "{}"))
        .await
        .expect_err("unknown ability returns status");
    assert_eq!(err.code(), tonic::Code::FailedPrecondition);

    let records = ledger.list_all().expect("ledger list");
    assert_eq!(records.len(), 1);
    let record = &records[0];
    let expected_prefix =
        crate::ura::resource_dot_ura("test-realm", "device.test-daemon", "invocations/");
    assert!(record.invocation_ura.starts_with(&expected_prefix));
    assert!(!record.invocation_ura.contains("/resource/invocation."));
    assert_eq!(record.state, "failed");
    assert_eq!(record.ability_name, "unknown.ability");
    assert_eq!(
        record.error.as_ref().map(|err| err.code.as_str()),
        Some(ROUTE_NEGATIVE_CODE)
    );
    assert_eq!(
        record
            .error
            .as_ref()
            .and_then(|err| err.context.get("transport_status"))
            .map(String::as_str),
        Some("failedprecondition")
    );
    assert_eq!(record.diagnostics[0].code, ROUTE_NEGATIVE_CODE);
}

#[test]
fn unary_ledger_projects_failed_invoke_response_error() {
    let request = invoke_request("terminal.fs.read", "{}").into_inner();
    let response = InvokeResponse {
        state: easynet_axon::invocation::InvocationState::Failed.to_wire_i32(),
        scheduling_reason: "handler failed".to_string(),
        error: Some(Error {
            code: "TARGET_NOT_IN_PRESENCE_REGISTRY".to_string(),
            message: "target device is not in PresenceRegistry".to_string(),
            retryable: true,
            stage: ErrorStage::Transport as i32,
            security_class: SecurityClass::Transport as i32,
            ..Error::default()
        }),
        ..InvokeResponse::default()
    };
    let result = Ok(Response::new(response));
    let record = build_unary_ledger_record(&request, 10, 15, &result).expect("ledger record");

    assert_eq!(record.state, "failed");
    assert!(record.result.is_none());
    let error = record.error.as_ref().expect("ledger error");
    assert_eq!(error.code, "TARGET_NOT_IN_PRESENCE_REGISTRY");
    assert_eq!(error.message, "target device is not in PresenceRegistry");
    assert!(error.retryable);
    assert_eq!(
        error.context.get("error_stage").map(String::as_str),
        Some("Transport")
    );
    assert_eq!(record.diagnostics.len(), 1);
    assert_eq!(
        record.diagnostics[0].code,
        "TARGET_NOT_IN_PRESENCE_REGISTRY"
    );
}

#[test]
fn unary_ledger_rejects_missing_subject_identity() {
    let mut request = invoke_request("terminal.fs.read", "{}").into_inner();
    request
        .envelope
        .as_mut()
        .expect("test request carries envelope")
        .subject = None;
    let response = InvokeResponse {
        state: easynet_axon::invocation::InvocationState::Completed.to_wire_i32(),
        ..InvokeResponse::default()
    };
    let result = Ok(Response::new(response));
    let err = build_unary_ledger_record(&request, 10, 15, &result)
        .expect_err("ledger projection must reject incomplete invocation tuples");

    assert!(
        err.to_string().contains("envelope.subject.ura is required"),
        "{err}"
    );
}

#[tokio::test]
async fn malformed_forward_invoke_quota_parse_error_is_audited() {
    let temp = tempfile::tempdir().expect("tempdir");
    let ledger = Arc::new(
        easynet_axon::invocation::InvocationLedger::open(
            temp.path().join("ledger").join("invocations.redb"),
        )
        .expect("ledger"),
    );
    let svc = make_service().with_invocation_ledger(Arc::clone(&ledger));

    let err = svc
        .invoke(invoke_request(
            ABILITY_FEDERATION_FORWARD_INVOKE,
            "{not-json",
        ))
        .await
        .expect_err("malformed forward_invoke must reject");
    assert_eq!(err.code(), tonic::Code::InvalidArgument);

    let records = ledger.list_all().expect("ledger list");
    assert_eq!(
        records.len(),
        1,
        "quota pre-parse errors must still write one failed ledger row"
    );
    let record = &records[0];
    assert_eq!(record.state, "failed");
    assert_eq!(record.ability_name, ABILITY_FEDERATION_FORWARD_INVOKE);
    assert_eq!(
        record.error.as_ref().map(|err| err.code.as_str()),
        Some("INVALID_ARGUMENT")
    );
    assert_eq!(
        record
            .error
            .as_ref()
            .and_then(|err| err.context.get("transport_status"))
            .map(String::as_str),
        Some("invalidargument")
    );
}

#[test]
fn ledger_authority_form_classifies_bootstrap_delegated_session_and_self() {
    let bootstrap = invoke_request(ABILITY_SELF_REGISTER_DEVICE_PUBKEY, "{}").into_inner();
    assert_eq!(ledger_authority_form_for_request(&bootstrap), "bootstrap");

    let mut delegated = invoke_request("demo.delegated", "{}").into_inner();
    delegated.metadata.insert(
        DELEGATION_METADATA_KEY.to_string(),
        "serialized-proof".to_string(),
    );
    assert_eq!(ledger_authority_form_for_request(&delegated), "delegated");

    let mut hosted = invoke_request("demo.hosted_delegated", "{}").into_inner();
    hosted.metadata.insert(
        HOSTED_AGENT_DELEGATION_METADATA_KEY.to_string(),
        r#"{"kind":"hosted_agent"}"#.to_string(),
    );
    assert_eq!(ledger_authority_form_for_request(&hosted), "delegated");

    let mut session = invoke_request("demo.session", "{}").into_inner();
    session.metadata.insert(
        SESSION_AUTHORITY_METADATA_KEY.to_string(),
        "serialized-session-authority".to_string(),
    );
    assert_eq!(ledger_authority_form_for_request(&session), "session");

    let self_authority = invoke_request("demo.self", "{}").into_inner();
    assert_eq!(ledger_authority_form_for_request(&self_authority), "self");
}

#[test]
fn invocation_resource_ura_is_owned_by_subject_user_when_present() {
    let ura = invocation_resource_ura(
        "test-realm",
        "req-1",
        &crate::ura::user_ura("test-realm", "alice"),
        &crate::ura::device_ura("test-realm", "callee-device"),
        &crate::ura::device_ura("test-realm", "caller-device"),
    )
    .expect("resource ura");
    assert_eq!(
        ura,
        "easynet:///r/test-realm/resource/alice.invocations/req-1"
    );
}

#[test]
fn invocation_resource_ura_maps_agent_to_user_owned_namespace() {
    let ura = invocation_resource_ura(
        "test-realm",
        "req/with spaces",
        &crate::ura::agent_ura("test-realm", "alice", "frontend"),
        &crate::ura::device_ura("test-realm", "callee-device"),
        &crate::ura::device_ura("test-realm", "caller-device"),
    )
    .expect("resource ura");
    assert!(ura.starts_with(
        "easynet:///r/test-realm/resource/alice.invocations/agents/frontend/invocations/req-with-spaces-"
    ));
    assert!(!ura.contains("/resource/invocation."));
}

#[test]
fn remote_receipt_projection_preserves_ability_identity_authority_and_proof_facts() {
    let _home = crate::facade::cli::test_support::HomeGuard::new();
    let receipt = InvocationReceipt {
        invocation_id: "remote-req-1".to_string(),
        state: easynet_axon::invocation::InvocationState::Completed.to_wire_i32(),
        timestamp_unix_ms: 1_200,
        caller_binding: Some(AgentIdentity {
            ura: crate::ura::agent_ura("caller-realm", "alice", "frontend"),
            profile: "easynet-strict-v2".to_string(),
        }),
        callee_binding: Some(AgentIdentity {
            ura: crate::ura::device_ura("peer-realm", "device-b"),
            profile: "easynet-strict-v2".to_string(),
        }),
        subject_binding: Some(SubjectIdentity {
            ura: crate::ura::user_ura("peer-realm", "bob"),
            profile: "easynet-strict-v2".to_string(),
        }),
        usage: Some(InvocationUsage {
            tokens_in: 5,
            tokens_out: 7,
            duration_ms: 200,
            external_calls: 1,
        }),
        descriptor_version: "descriptor.remote.v1".to_string(),
        schema_hash: vec![0x11; 32],
        impl_hash: vec![0x22; 32],
        runtime_env: "remote-runtime".to_string(),
        ..InvocationReceipt::default()
    };

    let record = ledger_record_from_remote_receipt(&receipt, "demo.echo", 1_000)
        .expect("remote receipt projects");
    // The forwarded-receipt ability_ura projects from the callee DEVICE
    // binding (`device-b`), the truthful provenance of where the peer ran
    // it — not the abstract hub form.
    assert_eq!(
        record.ability_ura,
        crate::ura::owner_ability_ura(
            &crate::ura::device_ura("peer-realm", "device-b"),
            "demo.echo"
        )
        .expect("device-form ability URA"),
        "remote rows must not leave ability_ura empty"
    );
    // A forwarded receipt does not carry the caller's authority form
    // (the delegation/session metadata lived on the originating request,
    // which the hub never sees). The projection records "unknown" rather
    // than minting a classification the receipt never asserted — the
    // callee's own ledger row holds the authoritative form.
    assert_eq!(record.authority_form, "unknown");
    assert_eq!(record.usage.tokens_in, 5);
    assert_eq!(record.usage.tokens_out, 7);
    assert_eq!(record.elapsed_ms, Some(200));
    assert_eq!(record.diagnostics.len(), 1);
    let diagnostic = &record.diagnostics[0];
    assert_eq!(diagnostic.source, "remote_receipt");
    assert_eq!(diagnostic.code, "REMOTE_RECEIPT_PROOF_FACTS");
    assert!(
        diagnostic.message.contains("descriptor.remote.v1"),
        "{diagnostic:?}"
    );
    assert!(
        diagnostic.message.contains("remote-runtime"),
        "{diagnostic:?}"
    );
    assert!(
        diagnostic.payload.is_some(),
        "proof facts must be digest-addressable in the local ledger projection"
    );
}

#[tokio::test]
async fn invoke_dispatches_federation_discover_with_no_filter_returns_empty_when_no_peers() {
    // PR-N3 N3-4: single-realm daemon (no federated peers)
    // returns the empty discover list. Graceful degradation —
    // the ability is callable on every daemon, just empty
    // when nothing has been federated yet.
    let svc = make_service();
    let resp = svc
        .invoke(invoke_request(ABILITY_FEDERATION_DISCOVER, "{}"))
        .await
        .expect("dispatch returns Ok");
    let body: federation_wrappers::DiscoverResponse = parse_response_body(resp);
    assert!(body.entries.is_empty());
}

#[tokio::test]
async fn invoke_dispatches_federation_discover_returns_peer_entries_when_view_populated() {
    // PR-N3 N3-4: when the federated_directory cell holds
    // entries (write side is the per-peer
    // RemoteDirectoryClient task in N3-3.1 — for this unit
    // test we manually `replace` the cell with a populated
    // map), discover surfaces them with origin_realm
    // stamped per §2.4.
    use crate::services::federation_directory::{
        DirectoryEntry, DirectoryView, SharedFederatedDirectoryView,
    };
    use std::collections::BTreeMap;

    let cell = SharedFederatedDirectoryView::default();
    let mut peer_view = DirectoryView::new("realm-b".to_string());
    peer_view.replace_entries(vec![DirectoryEntry {
        agent_ura: "easynet:///r/realm-b/device/peer-device".to_string(),
        node_id: "peer-1".to_string(),
        display_name: Some("silan-phone".to_string()),
        status: "active".to_string(),
        origin_realm: None, // peer omitted; rewrite stamps realm-b
        hub_endpoint: Some("https://hub-b.example:50443".to_string()),
        last_seen_unix_ms: Some(1_714_500_000_000),
    }]);
    let mut peers = BTreeMap::new();
    peers.insert("realm-b".to_string(), Arc::new(peer_view));
    cell.replace(peers);

    let svc = make_service().with_federated_directory_cell(cell);
    let resp = svc
        .invoke(invoke_request(ABILITY_FEDERATION_DISCOVER, "{}"))
        .await
        .expect("dispatch returns Ok");
    let body: federation_wrappers::DiscoverResponse = parse_response_body(resp);
    assert_eq!(body.entries.len(), 1);
    assert_eq!(
        body.entries[0].agent_ura,
        "easynet:///r/realm-b/device/peer-device"
    );
    assert_eq!(
        body.entries[0].origin_realm.as_deref(),
        Some("realm-b"),
        "§2.4 origin_realm rewrite must show through to the discover response"
    );
}

#[tokio::test]
async fn invoke_dispatches_federation_discover_with_ura_filter_returns_single_hit() {
    use crate::services::federation_directory::{
        DirectoryEntry, DirectoryView, SharedFederatedDirectoryView,
    };
    use std::collections::BTreeMap;

    let cell = SharedFederatedDirectoryView::default();
    let mut peer_view = DirectoryView::new("realm-b".to_string());
    peer_view.replace_entries(vec![
        DirectoryEntry {
            agent_ura: "easynet:///r/realm-b/device/match".to_string(),
            node_id: "n1".to_string(),
            display_name: None,
            status: "active".to_string(),
            origin_realm: None,
            hub_endpoint: None,
            last_seen_unix_ms: None,
        },
        DirectoryEntry {
            agent_ura: "easynet:///r/realm-b/device/other".to_string(),
            node_id: "n2".to_string(),
            display_name: None,
            status: "active".to_string(),
            origin_realm: None,
            hub_endpoint: None,
            last_seen_unix_ms: None,
        },
    ]);
    let mut peers = BTreeMap::new();
    peers.insert("realm-b".to_string(), Arc::new(peer_view));
    cell.replace(peers);

    let svc = make_service().with_federated_directory_cell(cell);
    let resp = svc
        .invoke(invoke_request(
            ABILITY_FEDERATION_DISCOVER,
            r#"{"agent_ura":"easynet:///r/realm-b/device/match"}"#,
        ))
        .await
        .expect("dispatch returns Ok");
    let body: federation_wrappers::DiscoverResponse = parse_response_body(resp);
    assert_eq!(body.entries.len(), 1);
    assert_eq!(
        body.entries[0].agent_ura,
        "easynet:///r/realm-b/device/match"
    );
}

// ── N3-N4 dispatch wire — discover with user filter ─────

#[tokio::test]
async fn invoke_discover_with_user_id_filters_unbound_cross_realm_entries() {
    // Daemon's session_realm = realm-b. View has realm-c
    // entry (unbound for the calling user). Bindings store
    // is empty, so the cross-realm entry is filtered out.
    use crate::runtime::keyring::federated_bindings::FederatedBindingsStore;
    use crate::services::federation_directory::{
        DirectoryEntry, DirectoryView, SharedFederatedDirectoryView,
    };
    use std::collections::BTreeMap;

    let cell = SharedFederatedDirectoryView::default();
    let mut realm_c = DirectoryView::new("realm-c".to_string());
    realm_c.replace_entries(vec![DirectoryEntry {
        agent_ura: "easynet:///r/realm-c/user/unbound".to_string(),
        node_id: "n".to_string(),
        display_name: None,
        status: "active".to_string(),
        origin_realm: None,
        hub_endpoint: None,
        last_seen_unix_ms: None,
    }]);
    let mut peers = BTreeMap::new();
    peers.insert("realm-c".to_string(), Arc::new(realm_c));
    cell.replace(peers);

    let bindings = Arc::new(FederatedBindingsStore::in_memory());
    let svc = make_service()
        .with_session_realm("realm-b")
        .with_federated_directory_cell(cell)
        .with_federated_bindings_store(bindings);

    let resp = svc
        .invoke(invoke_request(
            ABILITY_FEDERATION_DISCOVER,
            r#"{"local_user_id":"user-on-b"}"#,
        ))
        .await
        .expect("dispatch returns Ok");
    let body: federation_wrappers::DiscoverResponse = parse_response_body(resp);
    assert!(
        body.entries.is_empty(),
        "unbound cross-realm entry must be filtered when local_user_id is set"
    );
}

#[tokio::test]
async fn invoke_discover_without_user_id_does_not_filter() {
    // Same setup as above but no local_user_id ⇒ unfiltered
    // path. Cross-realm unbound entries surface (operator /
    // audit query path).
    use crate::services::federation_directory::{
        DirectoryEntry, DirectoryView, SharedFederatedDirectoryView,
    };
    use std::collections::BTreeMap;

    let cell = SharedFederatedDirectoryView::default();
    let mut realm_c = DirectoryView::new("realm-c".to_string());
    realm_c.replace_entries(vec![DirectoryEntry {
        agent_ura: "easynet:///r/realm-c/user/u".to_string(),
        node_id: "n".to_string(),
        display_name: None,
        status: "active".to_string(),
        origin_realm: None,
        hub_endpoint: None,
        last_seen_unix_ms: None,
    }]);
    let mut peers = BTreeMap::new();
    peers.insert("realm-c".to_string(), Arc::new(realm_c));
    cell.replace(peers);

    let svc = make_service()
        .with_session_realm("realm-b")
        .with_federated_directory_cell(cell);

    let resp = svc
        .invoke(invoke_request(ABILITY_FEDERATION_DISCOVER, r#"{}"#))
        .await
        .expect("dispatch returns Ok");
    let body: federation_wrappers::DiscoverResponse = parse_response_body(resp);
    assert_eq!(
        body.entries.len(),
        1,
        "unfiltered path must surface every entry regardless of binding state"
    );
}

#[tokio::test]
async fn invoke_discover_with_user_id_keeps_bound_entry() {
    use crate::runtime::keyring::federated_bindings::{
        FederatedBindingsStore, FederatedUserBinding,
    };
    use crate::services::federation_directory::{
        DirectoryEntry, DirectoryView, SharedFederatedDirectoryView,
    };
    use std::collections::BTreeMap;

    let cell = SharedFederatedDirectoryView::default();
    let mut realm_a = DirectoryView::new("realm-a".to_string());
    realm_a.replace_entries(vec![DirectoryEntry {
        agent_ura: "easynet:///r/realm-a/user/bound-user".to_string(),
        node_id: "n".to_string(),
        display_name: None,
        status: "active".to_string(),
        origin_realm: None,
        hub_endpoint: None,
        last_seen_unix_ms: None,
    }]);
    let mut peers = BTreeMap::new();
    peers.insert("realm-a".to_string(), Arc::new(realm_a));
    cell.replace(peers);

    let bindings = Arc::new(FederatedBindingsStore::in_memory());
    bindings
        .record_binding(
            FederatedUserBinding {
                source_realm: "realm-a".to_string(),
                source_user_ura: "easynet:///r/realm-a/user/bound-user".to_string(),
                source_user_pubkey_b64: "AAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAA=".to_string(),
                local_user_id: "user-on-b".to_string(),
                bound_at_unix_ms: 1_714_500_000_000,
            },
            "n".to_string(),
        )
        .unwrap();

    let svc = make_service()
        .with_session_realm("realm-b")
        .with_federated_directory_cell(cell)
        .with_federated_bindings_store(bindings);

    let resp = svc
        .invoke(invoke_request(
            ABILITY_FEDERATION_DISCOVER,
            r#"{"local_user_id":"user-on-b"}"#,
        ))
        .await
        .expect("dispatch returns Ok");
    let body: federation_wrappers::DiscoverResponse = parse_response_body(resp);
    assert_eq!(body.entries.len(), 1);
    assert_eq!(
        body.entries[0].agent_ura,
        "easynet:///r/realm-a/user/bound-user"
    );
}

#[tokio::test]
async fn invoke_dispatches_federation_list_user_devices_admits_loopback_caller() {
    // PR-N3 N3-5: a hub-mode daemon listing its own users
    // from a CLI on the same machine works without
    // configuring itself as a Hub trust entry — loopback
    // bypass admits at the general gate, the N3-5 filter
    // recognises `is_loopback = true` and accepts.
    let svc = make_service();
    // Two devices online for realm-x.
    svc.directory.presence.insert(
        "easynet:///r/realm-x/device/device-1".to_string(),
        tokio::sync::mpsc::channel(8).0,
    );
    svc.directory.presence.insert(
        "easynet:///r/realm-x/device/device-2".to_string(),
        tokio::sync::mpsc::channel(8).0,
    );
    // One device for an unrelated realm — must NOT show
    // through.
    svc.directory.presence.insert(
        "easynet:///r/realm-other/device/device-3".to_string(),
        tokio::sync::mpsc::channel(8).0,
    );

    let resp = svc
        .invoke(invoke_request(
            ABILITY_FEDERATION_LIST_USER_DEVICES,
            r#"{"realm":"realm-x"}"#,
        ))
        .await
        .expect("loopback caller admitted");
    let body: federation_wrappers::ListUserDevicesResponse = parse_response_body(resp);
    assert_eq!(body.devices.len(), 2);
    let expected_prefix = crate::ura::realm_device_prefix("realm-x");
    for entry in &body.devices {
        assert!(entry.agent_ura.starts_with(&expected_prefix));
    }
}

#[tokio::test]
async fn invoke_dispatches_federation_list_user_devices_rejects_non_hub_caller() {
    // PR-N3 N3-5: caller URA is in trust set but as Device
    // role → admission filter rejects. PermissionDenied is
    // the wire-stable rejection; the message mentions the
    // caller URA for operator audit grep.
    // The request is signed so the general admission gate passes;
    // the dispatch arm then reads the trust anchor again and finds
    // the role is Device, not Hub.
    use crate::services::realm_trust_anchor::{RealmTrustAnchor, TrustedAgent, TrustedAgentRole};
    use base64::{engine::general_purpose::STANDARD as BASE64_STANDARD, Engine as _};
    use ed25519_dalek::SigningKey;

    let device_caller_ura = "easynet:///r/realm-b/device/device-not-hub";
    let signing_key = SigningKey::from_bytes(&[0x77; 32]);
    let mut anchor_inner = RealmTrustAnchor::default();
    anchor_inner
        .append_agent(TrustedAgent {
            agent_ura: device_caller_ura.to_string(),
            public_key_b64: BASE64_STANDARD.encode(signing_key.verifying_key().to_bytes()),
            role: TrustedAgentRole::Device,
            added_at_unix_ms: 1_700_000_000_000,
            origin_realm: None,
            hub_endpoint: None,
            tls_ca_pem_path: None,
        })
        .expect("append device");
    let admission = AdmissionFacade::new(Arc::new(anchor_inner), Some(TEST_DAEMON_URI.to_string()));
    let svc = DaemonInvocationService::new(Arc::new(PresenceRegistry::new()), admission);

    let function_name = ABILITY_FEDERATION_LIST_USER_DEVICES;
    let arguments = br#"{"realm":"realm-x"}"#.to_vec();
    let envelope = signed_test_envelope(
        device_caller_ura,
        TEST_DAEMON_URI,
        TEST_DAEMON_URI,
        function_name,
        &arguments,
        &signing_key,
    );
    let req = Request::new(InvokeRequest {
        envelope: Some(envelope),
        function_name: function_name.to_string(),
        arguments,
        metadata: std::collections::HashMap::from([(
            crate::services::invocation_transport::invocation_wire::SIGNED_DESCRIPTOR_REF_METADATA_KEY
                .to_string(),
            test_descriptor_ref(TEST_DAEMON_URI, function_name),
        )]),
        ..InvokeRequest::default()
    });

    let err = svc
        .invoke(req)
        .await
        .expect_err("device-role caller must be rejected by N3-5 filter");
    assert_eq!(err.code(), tonic::Code::PermissionDenied);
    assert!(
        err.message().contains(device_caller_ura),
        "rejection message must surface the caller URA; got: {}",
        err.message()
    );
}

#[tokio::test]
async fn invoke_dispatches_federation_proxy_list_user_devices_fans_out_and_stamps_peer_metadata() {
    use crate::services::realm_trust_anchor::{RealmTrustAnchor, TrustedAgent, TrustedAgentRole};

    let peer_hub_url = "https://peer-hub.example:50443";
    let peer_hub_ura = crate::ura::hub_ura("peer-realm");
    let anchor = Arc::new(
        RealmTrustAnchor::from_entries(vec![TrustedAgent {
            agent_ura: peer_hub_ura.clone(),
            public_key_b64: "AAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAA=".to_string(),
            role: TrustedAgentRole::Hub,
            added_at_unix_ms: 1_700_000_000_000,
            origin_realm: Some("peer-realm".to_string()),
            hub_endpoint: Some(peer_hub_url.to_string()),
            tls_ca_pem_path: None,
        }])
        .expect("peer hub trust anchor"),
    );
    let admission = AdmissionFacade::new(anchor, Some(TEST_DAEMON_URI.to_string()));
    let canned = InvokeResponse {
        result: br#"{
            "devices":[{
                "agent_ura":"easynet:///r/user-realm/device/dev-peer",
                "node_id":"dev-peer",
                "status":"active"
            }]
        }"#
        .to_vec(),
        result_content_type: FEDERATION_RESULT_CONTENT_TYPE.to_string(),
        state: easynet_axon::invocation::InvocationState::Completed.to_wire_i32(),
        ..InvokeResponse::default()
    };
    let recorder = Arc::new(RecordingFederationClient::new(canned));
    let svc = DaemonInvocationService::new(Arc::new(PresenceRegistry::new()), admission)
        .with_hub_signing_seed([0x11; 32])
        .with_session_realm("local-realm")
        .with_federation_client(recorder.clone() as Arc<dyn FederationClient>);

    let resp = svc
        .invoke(invoke_request(
            ABILITY_FEDERATION_PROXY_LIST_USER_DEVICES,
            r#"{
                "realm":"user-realm",
                "peer_hub_urls":["https://peer-hub.example:50443"]
            }"#,
        ))
        .await
        .expect("proxy list user devices succeeds");
    let body: federation_wrappers::ProxyListUserDevicesResponse = parse_response_body(resp);
    assert_eq!(body.devices.len(), 1);
    let device = &body.devices[0];
    assert_eq!(device.agent_ura, "easynet:///r/user-realm/device/dev-peer");
    assert_eq!(device.origin_realm.as_deref(), Some("peer-realm"));
    assert_eq!(device.hub_endpoint.as_deref(), Some(peer_hub_url));

    let calls = recorder.calls();
    assert_eq!(calls.len(), 1, "exactly one peer request captured");
    assert_eq!(calls[0].0, peer_hub_url);
    assert_eq!(
        calls[0].1.function_name,
        ABILITY_FEDERATION_LIST_USER_DEVICES
    );
    let peer_args: federation_wrappers::ListUserDevicesRequest =
        serde_json::from_slice(&calls[0].1.arguments).expect("peer args decode");
    assert_eq!(peer_args.realm, "user-realm");
}

#[tokio::test]
async fn federation_proxy_caller_gate_accepts_local_hub_identity_with_hub_role() {
    use crate::services::realm_trust_anchor::{RealmTrustAnchor, TrustedAgent, TrustedAgentRole};

    let local_hub_ura = crate::ura::hub_ura("local-realm");
    let anchor = Arc::new(
        RealmTrustAnchor::from_entries(vec![TrustedAgent {
            agent_ura: local_hub_ura.clone(),
            public_key_b64: "AAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAA=".to_string(),
            role: TrustedAgentRole::Hub,
            added_at_unix_ms: 1_700_000_000_000,
            origin_realm: Some("local-realm".to_string()),
            hub_endpoint: Some("https://local-hub.example:50443".to_string()),
            tls_ca_pem_path: None,
        }])
        .expect("local hub trust anchor"),
    );
    let admission = AdmissionFacade::new(anchor, Some(TEST_DAEMON_URI.to_string()));
    let svc = DaemonInvocationService::new(Arc::new(PresenceRegistry::new()), admission)
        .with_session_realm("local-realm");
    let envelope = Envelope {
        caller: Some(AgentIdentity {
            ura: local_hub_ura,
            profile: "easynet-strict-v2".to_string(),
        }),
        ..Envelope::default()
    };

    svc.unary_dispatcher()
        .require_backend_or_loopback_proxy_caller(Some(&envelope), "namespace.proxy_resolve")
        .expect("local canonical hub identity is the backend proxy caller");
}

#[tokio::test]
async fn invoke_dispatches_federation_proxy_list_user_devices_rejects_hub_role_caller() {
    use base64::{engine::general_purpose::STANDARD as BASE64_STANDARD, Engine as _};
    use ed25519_dalek::SigningKey;

    use crate::services::realm_trust_anchor::{RealmTrustAnchor, TrustedAgent, TrustedAgentRole};

    let caller_signing_key = SigningKey::from_bytes(&[0x22; 32]);
    let caller_ura = crate::ura::hub_ura("peer-realm");
    let caller_pubkey_b64 = BASE64_STANDARD.encode(caller_signing_key.verifying_key().to_bytes());
    let anchor = Arc::new(
        RealmTrustAnchor::from_entries(vec![TrustedAgent {
            agent_ura: caller_ura.clone(),
            public_key_b64: caller_pubkey_b64,
            role: TrustedAgentRole::Hub,
            added_at_unix_ms: 1_700_000_000_000,
            origin_realm: Some("peer-realm".to_string()),
            hub_endpoint: Some("https://peer-hub.example:50443".to_string()),
            tls_ca_pem_path: None,
        }])
        .expect("hub caller trust anchor"),
    );
    let admission = AdmissionFacade::new(anchor, Some(TEST_DAEMON_URI.to_string()));
    let svc = DaemonInvocationService::new(Arc::new(PresenceRegistry::new()), admission)
        .with_session_realm("local-realm");

    let args = br#"{"realm":"user-realm","peer_hub_urls":["https://peer-hub.example:50443"]}"#;
    let descriptor_subject_ura = crate::ura::owner_ability_ura(
        &crate::ura::hub_ura("local-realm"),
        ABILITY_FEDERATION_PROXY_LIST_USER_DEVICES,
    )
    .expect("hub proxy ability subject");
    let mut envelope = Envelope {
        caller: Some(AgentIdentity {
            ura: caller_ura.clone(),
            profile: "easynet-strict-v2".to_string(),
        }),
        callee: Some(AgentIdentity {
            ura: crate::ura::hub_ura("local-realm"),
            profile: "easynet-strict-v2".to_string(),
        }),
        subject: Some(SubjectIdentity {
            ura: descriptor_subject_ura.clone(),
            profile: "easynet-strict-v2".to_string(),
        }),
        invocation_nonce: vec![7; 16],
        ..Envelope::default()
    };
    let descriptor_ref = format!(
        "{}@1.0.0",
        crate::ura::owner_ability_ura(
            &crate::ura::hub_ura("local-realm"),
            ABILITY_FEDERATION_PROXY_LIST_USER_DEVICES
        )
        .expect("proxy list descriptor ref")
    );
    let signed_descriptor_ref = sign_peer_request_envelope(
        &mut envelope,
        ABILITY_FEDERATION_PROXY_LIST_USER_DEVICES,
        &descriptor_ref,
        args,
        Some("local-realm"),
        Some(&[0x22; 32]),
    )
    .expect("sign test envelope");

    let mut request = InvokeRequest {
        envelope: Some(envelope),
        function_name: ABILITY_FEDERATION_PROXY_LIST_USER_DEVICES.to_string(),
        arguments: args.to_vec(),
        ..InvokeRequest::default()
    };
    request.metadata.insert(
        crate::services::invocation_transport::invocation_wire::SIGNED_DESCRIPTOR_REF_METADATA_KEY
            .to_string(),
        signed_descriptor_ref,
    );
    request.metadata.insert(
        "x-easynet-delegation".to_string(),
        signed_delegation_metadata_for_test(
            &caller_signing_key,
            &caller_ura,
            &descriptor_subject_ura,
            &caller_ura,
            &crate::ura::hub_ura("local-realm"),
            &[ABILITY_FEDERATION_PROXY_LIST_USER_DEVICES],
        ),
    );

    let err = svc
        .invoke(Request::new(request))
        .await
        .expect_err("hub-role caller must be rejected by proxy filter");
    assert_eq!(err.code(), tonic::Code::PermissionDenied);
    assert!(
        err.message().contains(&caller_ura),
        "rejection message must surface the caller URA; got: {}",
        err.message()
    );
}

#[tokio::test]
async fn invoke_dispatches_namespace_proxy_resolve_to_typed_peer_surface() {
    use crate::services::realm_trust_anchor::{RealmTrustAnchor, TrustedAgent, TrustedAgentRole};

    let peer_hub_url = "https://peer-hub.example:50443";
    let peer_hub_ura = crate::ura::hub_ura("peer-realm");
    let anchor = Arc::new(
        RealmTrustAnchor::from_entries(vec![TrustedAgent {
            agent_ura: peer_hub_ura,
            public_key_b64: "AAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAA=".to_string(),
            role: TrustedAgentRole::Hub,
            added_at_unix_ms: 1_700_000_000_000,
            origin_realm: Some("peer-realm".to_string()),
            hub_endpoint: Some(peer_hub_url.to_string()),
            tls_ca_pem_path: None,
        }])
        .expect("peer hub trust anchor"),
    );
    let admission = AdmissionFacade::new(anchor, Some(TEST_DAEMON_URI.to_string()));
    let owner_ura = "easynet:///r/peer-realm/device/dev-peer";
    let ability_ura = crate::ura::owner_ability_ura(owner_ura, "agent.list").expect("ability ura");
    let canned = InvokeResponse {
        result: serde_json::to_vec(&serde_json::json!({
            "answerKind": "RESOLVE_ANSWER_KIND_NON_DISPATCHABLE",
            "records": [
                {
                    "name": owner_ura,
                    "recordType": "RECORD_TYPE_ID",
                    "value": {
                        "id": {
                            "ura": owner_ura,
                            "kind": "URA_KIND_DEVICE"
                        }
                    }
                },
                {
                    "name": ability_ura,
                    "recordType": "RECORD_TYPE_ABILITY",
                    "value": {
                        "ability": {
                            "abilityUra": ability_ura,
                            "ownerUra": owner_ura,
                            "namespace": "agent",
                            "localName": "list"
                        }
                    }
                }
            ],
            "releaseProfile": "RESOLVER_RELEASE_PROFILE_AUTHORITATIVE_LOCAL",
            "cachePolicy": {
                "ttlMs": 0,
                "sharedCacheable": false,
                "retryAfterUnixMs": 0
            }
        }))
        .expect("typed resolve answer fixture"),
        result_content_type: FEDERATION_RESULT_CONTENT_TYPE.to_string(),
        state: easynet_axon::invocation::InvocationState::Completed.to_wire_i32(),
        ..InvokeResponse::default()
    };
    let recorder = Arc::new(RecordingFederationClient::new(canned));
    let svc = DaemonInvocationService::new(Arc::new(PresenceRegistry::new()), admission)
        .with_hub_signing_seed([0x11; 32])
        .with_session_realm("local-realm")
        .with_federation_client(recorder.clone() as Arc<dyn FederationClient>);

    let resp = svc
        .invoke(invoke_request(
            ABILITY_NAMESPACE_PROXY_RESOLVE,
            r#"{
                "peer_hub_urls":["https://peer-hub.example:50443"],
                "queryName":"easynet:///r/peer-realm/device/",
                "qtype":"RESOLVE_TYPE_DIRECTORY_LISTING",
                "callerUra":"easynet:///r/local-realm/hub",
                "subjectUra":"easynet:///r/local-realm/user/alice",
                "realmHint":"peer-realm"
            }"#,
        ))
        .await
        .expect("namespace proxy resolve succeeds");
    let body: serde_json::Value = parse_response_body(resp);
    assert_eq!(
        body["answerKind"], "RESOLVE_ANSWER_KIND_NON_DISPATCHABLE",
        "proxy returns typed ResolveAnswer shape"
    );
    assert_eq!(
        body["records"].as_array().map(Vec::len),
        Some(2),
        "proxy preserves peer namespace records"
    );

    let calls = recorder.calls();
    assert_eq!(calls.len(), 1, "exactly one peer request captured");
    assert_eq!(calls[0].0, peer_hub_url);
    assert_eq!(calls[0].1.function_name, ABILITY_NAMESPACE_RESOLVE);
    let peer_args: serde_json::Value =
        serde_json::from_slice(&calls[0].1.arguments).expect("peer args decode");
    assert_eq!(peer_args["queryName"], "easynet:///r/peer-realm/device/");
    assert_eq!(peer_args["qtype"], "RESOLVE_TYPE_DIRECTORY_LISTING");
}

#[tokio::test]
async fn invoke_dispatches_federation_resolve_key_returns_pubkey_when_present() {
    // PR-N2 commit 2/N: peer-side `federation.resolve_key`
    // surfaces the local trust anchor's `public_key_b64` for
    // a known URA. Cross-hub `FederatedKeyResolver` consumes
    // this exact wire shape.
    use crate::services::realm_trust_anchor::{RealmTrustAnchor, TrustedAgent, TrustedAgentRole};
    let entry = TrustedAgent {
        agent_ura: "easynet:///r/realm-a/device/n1".to_string(),
        public_key_b64: "AAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAA=".to_string(),
        role: TrustedAgentRole::Device,
        added_at_unix_ms: 1_700_000_000_000,
        origin_realm: None,
        hub_endpoint: None,
        tls_ca_pem_path: None,
    };
    let anchor = Arc::new(RealmTrustAnchor::from_entries(vec![entry]).expect("anchor"));
    let admission = AdmissionFacade::new(anchor, Some(TEST_DAEMON_URI.to_string()));
    let svc = DaemonInvocationService::new(Arc::new(PresenceRegistry::new()), admission);

    let resp = svc
        .invoke(invoke_request(
            ABILITY_FEDERATION_RESOLVE_KEY,
            r#"{"agent_ura":"easynet:///r/realm-a/device/n1"}"#,
        ))
        .await
        .expect("dispatch returns Ok");
    let body: federation_wrappers::ResolveKeyResponse = parse_response_body(resp);
    assert_eq!(
        body.public_key_b64,
        "AAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAA="
    );
}

#[tokio::test]
async fn invoke_dispatches_federation_resolve_key_returns_not_found_when_ura_unknown() {
    // PR-N2 commit 2/N: miss surfaces as Status::not_found
    // with the URA in the error message — operators can
    // grep the daemon log for the exact URA that failed.
    let svc = make_service();
    let err = svc
        .invoke(invoke_request(
            ABILITY_FEDERATION_RESOLVE_KEY,
            r#"{"agent_ura":"easynet:///r/realm-a/device/missing"}"#,
        ))
        .await
        .expect_err("miss must surface Status::not_found");
    assert_eq!(err.code(), tonic::Code::NotFound);
    assert!(
        err.message()
            .contains("easynet:///r/realm-a/device/missing"),
        "expected the missing URA in error message, got: {}",
        err.message()
    );
}

#[tokio::test]
async fn invoke_dispatches_federation_revoke() {
    let svc = make_service();
    let resp = svc
        .invoke(invoke_request(
            ABILITY_FEDERATION_REVOKE,
            r#"{"target_ura":"easynet:///r/realm/device/missing"}"#,
        ))
        .await
        .expect("dispatch returns Ok");
    let body: federation_wrappers::RevokeResponse = parse_response_body(resp);
    assert!(body.ack);
    assert!(!body.was_active);
}

#[tokio::test]
async fn invoke_dispatches_federation_forward_invoke() {
    // DEC-N4 §2.1: empty `inner_envelope_b64` is rejected
    // up front by `decode_inner_payload` because the
    // payload must carry a non-empty `call_id`. Earlier
    // staging code accepted the empty shape and replied
    // `target_online: false`; the final wire shape requires
    // a real correlation id, so the wrong shape surfaces as
    // `Status::invalid_argument`.
    let svc = make_service();
    let err = svc
        .invoke(invoke_request(
            ABILITY_FEDERATION_FORWARD_INVOKE,
            r#"{"target_ura":"easynet:///r/realm/device/missing","inner_envelope_b64":""}"#,
        ))
        .await
        .expect_err("empty inner_envelope_b64 must be rejected");
    assert_eq!(err.code(), tonic::Code::InvalidArgument);
    assert!(
        err.message().contains("inner_envelope_b64 is empty"),
        "expected empty-payload error, got: {}",
        err.message()
    );
}

#[tokio::test]
async fn invoke_rejects_subscribe_directory_via_unary_invoke() {
    let svc = make_service();
    match svc
        .invoke(invoke_request(ABILITY_FEDERATION_SUBSCRIBE_DIRECTORY, "{}"))
        .await
    {
        Err(err) => {
            assert_eq!(err.code(), tonic::Code::InvalidArgument);
            assert!(err.message().contains("server-stream"));
        }
        Ok(_) => panic!("subscribe_directory must be rejected on unary Invoke"),
    }
}

#[tokio::test]
async fn invoke_unknown_ability_without_projection_returns_resolver_negative() {
    // RFC-005 pin: when the federation-wrapper match misses,
    // namespace.resolve is the first gate. A missing owner
    // projection is reported before LocalRuntime wiring is
    // inspected.
    let svc = make_service();
    match svc.invoke(invoke_request("custom.ability.x", "{}")).await {
        Err(err) => {
            assert_eq!(err.code(), tonic::Code::FailedPrecondition);
            assert!(
                err.message().contains(ROUTE_NEGATIVE_CODE),
                "expected resolver negative; got: {}",
                err.message()
            );
        }
        Ok(_) => panic!("unknown ability must be rejected"),
    }
}

/// When the Axon `LocalRuntime` is wired, the owner projection
/// publishes the ability, and namespace.resolve selects a route,
/// direct unary Invoke dispatches through `LocalRuntime::invoke_async`
/// and returns the handler's JSON output.
#[tokio::test]
async fn invoke_dispatches_selected_route_to_axon_runtime_when_wired() {
    use easynet_axon::invocation::{make_ability, AbilityCallModes, AbilityOptions, LocalRuntime};

    let rt = LocalRuntime::new();
    let ability = "test.fallback.echo";
    let ability_ura = crate::ura::owner_ability_ura(TEST_DAEMON_URI, ability).unwrap();
    rt.register_ability_with_options(
        ability_ura,
        make_ability(|ctx| async move { Ok(ctx.payload.clone()) }),
        AbilityOptions::default()
            .with_modes(AbilityCallModes::RPC)
            .with_descriptor_proof(
                crate::runtime::ability::DEFAULT_ABILITY_DESCRIPTOR_VERSION,
                [0x11; 32],
                [0x22; 32],
            ),
    )
    .await
    .unwrap();

    let svc = make_service().with_local_runtime(Arc::clone(&rt));
    publish_test_route(&svc, TEST_DAEMON_URI, ability);
    let resp = svc
        .invoke(invoke_request(ability, r#"{"hello":"world"}"#))
        .await
        .expect("selected-route dispatch succeeds");
    let body: serde_json::Value = parse_response_body(resp);
    assert_eq!(body["hello"], "world");
}

#[tokio::test]
async fn invoke_selected_route_unknown_runtime_handler_surfaces_not_found() {
    use easynet_axon::invocation::LocalRuntime;

    let rt = LocalRuntime::new();
    let svc = make_service().with_local_runtime(Arc::clone(&rt));
    publish_test_route(&svc, TEST_DAEMON_URI, "nope.nope");

    match svc.invoke(invoke_request("nope.nope", "{}")).await {
        Err(err) => {
            assert_eq!(err.code(), tonic::Code::FailedPrecondition);
            assert!(
                err.message()
                    .contains("does not register a dispatchable route"),
                "expected the not-registered message; got: {}",
                err.message()
            );
        }
        Ok(_) => panic!("unregistered ability must be rejected"),
    }
}

#[tokio::test]
async fn invoke_runtime_bootstrap_self_identity_is_not_cli_shadow_acked() {
    // No SDK admin installed: the runtime admin path must report the
    // missing handler, never fabricate a CLI-side ack. No catalog
    // route is published — `runtime.*` bypasses owner resolution.
    let rt = crate::runtime::axon_bridge::runtime_factory::build_local_runtime(None, None);
    let svc = make_service().with_local_runtime(Arc::clone(&rt));
    let args = r#"{
        "tenant_id":"tenant-a",
        "node_id":"node-a",
        "owner_id":"node-a",
        "public_key_b64":"AAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAA="
    }"#;

    match svc
        .invoke(invoke_request(
            federation_wrappers::ABILITY_RUNTIME_BOOTSTRAP_SELF_IDENTITY,
            args,
        ))
        .await
    {
        Err(err) => {
            assert_eq!(err.code(), tonic::Code::NotFound);
            assert!(
                err.message().contains("not installed in Axon LocalRuntime"),
                "expected SDK LocalRuntime missing-handler diagnostic; got: {}",
                err.message()
            );
        }
        Ok(resp) => {
            let body: serde_json::Value = parse_response_body(resp);
            panic!("bootstrap_self_identity must not be CLI-shadow-acked: {body}");
        }
    }
}

#[tokio::test]
async fn invoke_runtime_bootstrap_self_identity_succeeds_when_sdk_admin_installed() {
    use base64::{engine::general_purpose::STANDARD as BASE64_STANDARD, Engine as _};
    use ed25519_dalek::SigningKey;

    // Admin installed, NO catalog route published: `runtime.*`
    // dispatches directly on the LocalRuntime, proving it bypasses
    // owner-presence resolution (the production bug was a hub-owner
    // callee resolving to NXDOMAIN on the device daemon).
    let rt = crate::runtime::axon_bridge::runtime_factory::build_local_runtime(None, None);
    rt.install_bootstrap_self_identity_admin().await.unwrap();
    let svc = make_service().with_local_runtime(Arc::clone(&rt));
    let key = SigningKey::from_bytes(&[0x44; 32]);
    let args = serde_json::json!({
        "tenant_id": "tenant-a",
        "node_id": "node-a",
        "owner_id": "node-a",
        "public_key_b64": BASE64_STANDARD.encode(key.verifying_key().to_bytes()),
    })
    .to_string();

    let resp = svc
        .invoke(invoke_request(
            federation_wrappers::ABILITY_RUNTIME_BOOTSTRAP_SELF_IDENTITY,
            &args,
        ))
        .await
        .expect("SDK runtime admin bootstrap should be dispatched");
    let body: serde_json::Value = parse_response_body(resp);
    assert_eq!(body["ack"], true);
    assert_eq!(body["replaced_prior"], false);
}

#[tokio::test]
async fn invoke_returns_invalid_argument_on_bad_json() {
    let svc = make_service();
    match svc
        .invoke(invoke_request(ABILITY_FEDERATION_JOIN, "not-json"))
        .await
    {
        Err(err) => assert_eq!(err.code(), tonic::Code::InvalidArgument),
        Ok(_) => panic!("malformed JSON must be rejected"),
    }
}

#[tokio::test]
async fn dispatch_remote_rpc_refuses_self_execution_host() {
    // Device-mode boot seeds a resolve-only presence entry under the
    // daemon's own URA (boot/presence_seed.rs) whose drain task
    // accepts frames and never completes the pending entry. A route
    // whose selected execution host is this daemon must be refused
    // at the presence-dispatch core — never queued onto that entry.
    let (self_tx, mut self_rx) = mpsc::channel(8);
    let svc = make_service().with_pending(Arc::new(PendingDispatchMap::new()));
    svc.directory
        .presence
        .insert(TEST_DAEMON_URI.to_string(), self_tx);
    publish_test_route(&svc, TEST_DAEMON_URI, "observe.health");

    let ability_ura = crate::ura::owner_ability_ura(TEST_DAEMON_URI, "observe.health")
        .expect("device ability URA");
    let selected_route = svc
        .target_gate()
        .route_resolver()
        .await
        .resolve_route(&ability_ura, "")
        .expect("resolver selects the self-hosted route");
    assert_eq!(selected_route.execution_host_ura, TEST_DAEMON_URI);

    let request = invoke_request("observe.health", "{}").into_inner();
    let err = svc
        .unary_dispatcher()
        .dispatch_remote_rpc_selected_route(&request, &selected_route)
        .await
        .expect_err("self execution host must not presence-dispatch");
    assert_eq!(err.code(), tonic::Code::FailedPrecondition);
    assert!(
        err.message().contains("this daemon itself"),
        "expected the self-host refusal, got: {}",
        err.message()
    );
    assert!(
        self_rx.try_recv().is_err(),
        "no dispatch frame may reach the resolve-only self entry"
    );
}

#[tokio::test(start_paused = true)]
async fn dispatch_remote_rpc_times_out_when_target_never_replies() {
    // A presence entry that accepts the dispatch frame but never
    // sends a Result frame back (wedged device, drain-only channel)
    // must surface as structured DeadlineExceeded once
    // PRESENCE_DISPATCH_REPLY_TIMEOUT elapses instead of parking the
    // caller for the life of the connection. Paused clock: tokio
    // advances straight to the deadline once the waiter is idle.
    const WEDGED_DEVICE_URA: &str = "easynet:///r/test-realm/device/wedged-device";

    let pending = Arc::new(PendingDispatchMap::new());
    let svc = make_service().with_pending(Arc::clone(&pending));
    let (wedged_tx, mut wedged_rx) = mpsc::channel(8);
    svc.directory
        .presence
        .insert(WEDGED_DEVICE_URA.to_string(), wedged_tx);
    publish_test_route(&svc, WEDGED_DEVICE_URA, "observe.health");

    let ability_ura = crate::ura::owner_ability_ura(WEDGED_DEVICE_URA, "observe.health")
        .expect("device ability URA");
    let selected_route = svc
        .target_gate()
        .route_resolver()
        .await
        .resolve_route(&ability_ura, "")
        .expect("resolver selects the wedged-device route");
    assert_eq!(selected_route.execution_host_ura, WEDGED_DEVICE_URA);

    let request = invoke_request("observe.health", "{}").into_inner();
    let err = svc
        .unary_dispatcher()
        .dispatch_remote_rpc_selected_route(&request, &selected_route)
        .await
        .expect_err("no Result frame: the dispatch must deadline, not hang");
    assert_eq!(err.code(), tonic::Code::DeadlineExceeded);
    assert!(
        wedged_rx.try_recv().is_ok(),
        "the dispatch frame itself was delivered before the deadline"
    );
    assert_eq!(
        pending.outstanding(),
        0,
        "timeout must evict the pending entry so a late Result is a no-op"
    );
}
