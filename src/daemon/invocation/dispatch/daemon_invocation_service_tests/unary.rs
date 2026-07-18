use super::*;
use crate::daemon::federation::resolver_contract::{ResolveAnswerKind, RouteReason};

#[test]
fn quota_meters_user_abilities_but_exempts_control_plane() {
    assert!(quota_meters_function("observe.health"));
    assert!(quota_meters_function("agent.todo.run"));

    assert!(!quota_meters_function(ABILITY_FEDERATION_HEARTBEAT));
    assert!(!quota_meters_function(ABILITY_FEDERATION_STATUS));
    assert!(!quota_meters_function(ABILITY_NAMESPACE_RESOLVE));
    assert!(!quota_meters_function(ABILITY_IDENTITY_REGISTER_PUBKEY));
    assert!(!quota_meters_function(ABILITY_SESSION_OPEN));

    assert!(
        quota_meters_function("federation.user_owned_probe"),
        "quota exemptions must be exact system abilities, not namespace prefixes"
    );
    assert!(
        quota_meters_function("agent.user_owned_probe"),
        "a user-registered reserved-prefix ability must not bypass quota by spelling alone"
    );
}

#[tokio::test]
async fn signed_invocation_cancel_command_replay_is_rejected() {
    let svc = make_service();
    let arguments = serde_json::to_vec(
        &crate::daemon::invocation::dispatch::cancellation::InvocationCancelCommand::new(
            "ab".repeat(32),
            None,
            "canonical replay probe",
        )
        .expect("valid cancellation command"),
    )
    .expect("encode cancellation command");
    let envelope = ProtoEnvelope::from_target(
        crate::daemon::identity::local_invocation::LOCAL_SYSTEM_AGENT_URA,
        TEST_DAEMON_URA,
        TEST_DAEMON_URA,
        InvocationDerivationPolicy::Explicit {
            invocation_nonce: [0xA7; 16],
            causal_context: axon_sdk::invocation::CausalContext::None,
        },
    )
    .expect("valid local-system cancellation envelope")
    .into_inner(
        crate::daemon::invocation::dispatch::cancellation::ABILITY_INVOCATION_CANCEL,
        &arguments,
    )
    .expect("complete cancellation tuple");
    let request = InvokeRequest {
        envelope: Some(envelope),
        target: Some(test_invocation_target(
            crate::daemon::invocation::dispatch::cancellation::ABILITY_INVOCATION_CANCEL,
        )),
        arguments,
        ..InvokeRequest::default()
    };

    svc.invoke(Request::new(request.clone()))
        .await
        .expect("first signed local-system cancel reaches canonical runtime");
    let replay = expect_canonical_in_band_failure(
        svc.invoke(Request::new(request)).await,
        axon_sdk::invocation::ErrorCode::CallerNonceReplayed,
        "same signed cancellation command nonce must be rejected",
    );
    assert!(
        replay.message.contains("dedup_window_hit"),
        "replay denial must come from Axon canonical admission: {}",
        replay.message
    );
}

#[test]
fn hub_daemon_invocation_surface_satisfies_baseline_contract() {
    use crate::daemon::ability::conformance::{DaemonInvocationSurface, HubBaseline};

    let report = DaemonInvocationSurface::from_daemon_surface()
        .check("hub", HubBaseline::required_abilities());

    assert!(report.is_conformant(), "{}", report.panic_message());
}

#[tokio::test]
async fn daemon_exact_route_family_registers_all_31_owner_bound_abilities() {
    let service = make_service();
    service
        .register_daemon_unary_routes(TEST_DAEMON_URA)
        .await
        .expect("register complete daemon exact-route family");
    let runtime = service
        .runtime
        .local_runtime()
        .expect("test service has shared LocalRuntime");

    assert_eq!(DaemonUnaryRoute::ALL.len(), 31);
    for route in DaemonUnaryRoute::ALL.iter().copied() {
        let binding = crate::daemon::invocation::dispatch::descriptor_binding::RuntimeBoundAbility::from_wire_target(
            "daemon exact-route registration test",
            runtime.as_ref(),
            TEST_DAEMON_URA,
            route.name(),
        )
        .await
        .unwrap_or_else(|error| panic!("{} must be runtime-registered: {error}", route.name()));
        assert!(binding.supports_mode(CallMode::Rpc));
        binding
            .descriptor_ref_for_mode(
                "daemon exact-route registration test",
                TEST_DAEMON_URA,
                CallMode::Rpc,
                None,
            )
            .unwrap_or_else(|error| {
                panic!(
                    "{} must have a governed RPC descriptor: {error}",
                    route.name()
                )
            });
    }
}

#[tokio::test]
async fn daemon_exact_route_family_registers_one_atomic_multi_authority_set() {
    let service = make_unregistered_service_for_route_owner(TEST_DAEMON_URA);
    let hub_ura = crate::core::ura::hub_ura("test-realm");
    let owners = vec![TEST_DAEMON_URA.to_string(), hub_ura];

    service
        .register_daemon_unary_routes_for_owners(&owners)
        .await
        .expect("register complete daemon exact-route family for both authority roots");
    let runtime = service
        .runtime
        .local_runtime()
        .expect("test service has shared LocalRuntime");

    for owner_ura in owners {
        for route in DaemonUnaryRoute::ALL.iter().copied() {
            let binding = crate::daemon::invocation::dispatch::descriptor_binding::RuntimeBoundAbility::from_wire_target(
                "daemon multi-authority exact-route registration test",
                runtime.as_ref(),
                &owner_ura,
                route.name(),
            )
            .await
            .unwrap_or_else(|error| {
                panic!(
                    "{} under {owner_ura} must be runtime-registered: {error}",
                    route.name()
                )
            });
            assert!(binding.supports_mode(CallMode::Rpc));
        }
    }
}

#[tokio::test]
async fn invoke_dispatches_federation_join_to_wrapper() {
    let cell = SharedTrustAnchor::new(Arc::new(RealmTrustAnchor::default()));
    let trust_dir = tempfile::tempdir().expect("trust tempdir");
    let trust_path = trust_dir.path().join("realm-trust.toml");
    let svc = make_service_with_runtime_trust_route_owner(
        &crate::core::ura::hub_ura("realm"),
        "realm",
        trust_path,
        cell.clone(),
    );
    let test_seed = [0x42; 32];
    let public_key_hex = hex::encode(
        ed25519_dalek::SigningKey::from_bytes(&test_seed)
            .verifying_key()
            .to_bytes(),
    );
    let membership_ura = "easynet:///r/realm/device/n1";
    let args = format!(
        r#"{{"membership_ura":"{membership_ura}","realm":"realm","public_key_hex":"{public_key_hex}"}}"#
    );
    let request =
        signed_federation_join_request("realm", membership_ura, args.into_bytes(), test_seed).await;
    let resp = svc
        .invoke(Request::new(request))
        .await
        .expect("dispatch returns Ok");
    let body: federation_wrappers::JoinResponse = parse_response_body(resp);
    assert_eq!(body.membership_ura, membership_ura);
    assert_eq!(body.realm, "realm");
    assert_eq!(body.join_receipt_hash.len(), 64);
}

#[tokio::test]
async fn principal_mutation_commits_once_and_returns_one_finalized_receipt_chain() {
    let cell = SharedTrustAnchor::new(Arc::new(RealmTrustAnchor::default()));
    let trust_dir = tempfile::tempdir().expect("principal mutation tempdir");
    let trust_path = trust_dir.path().join("realm-trust.toml");
    let receipt_key_resolver =
        crate::daemon::axon_bridge::runtime_factory::ephemeral_test_receipt_key_resolver();
    let principal_ura = "easynet:///r/test-realm/user/mutation-once";
    let arguments = serde_json::to_string(&serde_json::json!({
        "request": {
            "command": {
                "actor_ura": principal_ura,
                "idempotency_key": "a21-single-commit",
                "proof": {
                    "kind": "bootstrap",
                    "reference": "proof:a21-single-commit"
                }
            },
            "principal_ura": principal_ura
        }
    }))
    .expect("principal.create arguments");
    let service = register_test_daemon_routes(
        make_unregistered_service_for_route_owner(TEST_DAEMON_URA).with_register_pubkey(
            "test-realm",
            trust_path.clone(),
            cell,
        ),
        TEST_DAEMON_URA,
    );

    let response = service
        .invoke(invoke_request(ABILITY_PRINCIPAL_CREATE, &arguments))
        .await
        .expect("principal mutation reaches canonical finalization")
        .into_inner();

    assert_eq!(
        response.state,
        axon_sdk::invocation::InvocationState::Completed.to_wire_i32()
    );
    assert!(response.error.is_none());
    let admission = response
        .admission_receipt
        .expect("one finalized chain has an admission receipt");
    let terminal = response
        .terminal_receipt
        .expect("one finalized chain has a terminal receipt");
    assert_eq!(admission.invocation_id, terminal.invocation_id);
    assert_eq!(
        response
            .header
            .as_ref()
            .map(|header| header.request_id.as_str()),
        Some(terminal.invocation_id.as_str())
    );
    assert_eq!(
        terminal.state,
        axon_sdk::invocation::InvocationState::Completed.to_wire_i32()
    );

    let signed_receipts = [admission, terminal].map(|wire_receipt| {
        let receipt = axon_sdk::invocation::wire::try_receipt_from_wire(wire_receipt)
            .expect("canonical runtime returns a structurally valid signed receipt")
            .verify(receipt_key_resolver.as_ref())
            .expect("canonical runtime receipt signature verifies against the daemon authority");
        receipt
            .verify_proof_facts()
            .expect("canonical runtime receipt carries descriptor-bound proof facts");
        receipt
    });
    let verified_checkpoints =
        axon_sdk::invocation::FinalizationCheckpointVerifier::new(receipt_key_resolver.as_ref())
            .verify(&signed_receipts[0], &signed_receipts[1])
            .expect("admission and terminal are signed checkpoints from one Axon invocation");
    assert_eq!(
        verified_checkpoints.admission().state(),
        axon_sdk::invocation::InvocationState::Admitted
    );
    assert_eq!(
        verified_checkpoints.terminal().state(),
        axon_sdk::invocation::InvocationState::Completed
    );

    let store_path = crate::daemon::invocation::admission::principal_lifecycle::principal_lifecycle_store_path_for_trust_anchor(&trust_path);
    let store: serde_json::Value = serde_json::from_slice(
        &std::fs::read(&store_path).expect("principal mutation persisted exactly once"),
    )
    .expect("principal lifecycle store JSON");
    let record = &store["principals"][principal_ura];
    assert_eq!(record["version"], 1, "one request must bump version once");
    assert_eq!(
        record["command_log"].as_object().map(serde_json::Map::len),
        Some(1),
        "one request must record one committed command"
    );
}

#[tokio::test]
async fn federation_join_with_principal_proof_binds_device_owner_in_runtime_trust() {
    use base64::{engine::general_purpose::STANDARD as BASE64_STANDARD, Engine as _};

    let cell = SharedTrustAnchor::new(Arc::new(RealmTrustAnchor::default()));
    let trust_dir = tempfile::tempdir().expect("trust tempdir");
    let trust_path = trust_dir.path().join("realm-trust.toml");
    let runtime_trust = crate::daemon::invocation::admission::runtime_trust::RuntimeTrustContext {
        daemon_realm: "realm".to_string(),
        trust_anchor_path: trust_path.clone(),
        cell: cell.clone(),
    };
    let lifecycle =
        crate::daemon::invocation::admission::principal_lifecycle::PrincipalLifecycleContext::from_runtime_trust(runtime_trust);
    let user_ura = "easynet:///r/realm/user/alice";
    lifecycle
        .handle(
            crate::daemon::invocation::admission::principal_lifecycle::ABILITY_PRINCIPAL_CREATE,
            serde_json::to_vec(&serde_json::json!({
                "request": {
                    "command": {
                        "actor_ura": user_ura,
                        "idempotency_key": "create-alice",
                        "proof": {"kind": "bootstrap", "reference": "proof:create-alice"}
                    },
                    "principal_ura": user_ura
                }
            }))
            .expect("create args")
            .as_slice(),
        )
        .expect("create principal");
    let user_pubkey = BASE64_STANDARD.encode(
        ed25519_dalek::SigningKey::from_bytes(&[0x51; 32])
            .verifying_key()
            .to_bytes(),
    );
    let bound = lifecycle
        .handle(
            crate::daemon::invocation::admission::principal_lifecycle::ABILITY_PRINCIPAL_BIND_FIRST_KEY,
            serde_json::to_vec(&serde_json::json!({
                "request": {
                    "command": {
                        "actor_ura": user_ura,
                        "idempotency_key": "bind-alice",
                        "expected_version": 1,
                        "proof": {"kind": "bootstrap", "reference": "proof:create-alice"}
                    },
                    "principal_ura": user_ura,
                    "public_key_b64": user_pubkey
                }
            }))
            .expect("bind args")
            .as_slice(),
        )
        .expect("bind first key");
    let bound: serde_json::Value = serde_json::from_slice(&bound).expect("bound json");
    let binding_id = bound["principal"]["bindings"][0]["binding_id"]
        .as_str()
        .expect("binding id");

    let svc = make_service_with_runtime_trust_route_owner(
        &crate::core::ura::hub_ura("realm"),
        "realm",
        trust_path,
        cell.clone(),
    );
    let test_seed = [0x42; 32];
    let public_key_hex = hex::encode(
        ed25519_dalek::SigningKey::from_bytes(&test_seed)
            .verifying_key()
            .to_bytes(),
    );
    let membership_ura = "easynet:///r/realm/device/n1";
    let args = serde_json::to_vec(&serde_json::json!({
        "membership_ura": membership_ura,
        "realm": "realm",
        "public_key_hex": public_key_hex,
        "principal_enrollment": {
            "principal_ura": user_ura,
            "proof": {"kind": "active_key", "reference": binding_id}
        }
    }))
    .expect("join args");
    let request = signed_federation_join_request("realm", membership_ura, args, test_seed).await;

    svc.invoke(Request::new(request))
        .await
        .expect("principal-bound join succeeds");

    let anchor = cell.snapshot();
    let owner = anchor
        .lookup_principal_owner(membership_ura)
        .expect("device owner binding");
    assert_eq!(owner.owner_ura, user_ura);
    assert_eq!(owner.owner_user_id, "alice");
    assert_eq!(owner.owner_username.as_deref(), Some("alice"));
}

#[tokio::test]
async fn invoke_dispatches_federation_advertise_agent() {
    let _hg = crate::cli::commands::test_support::HomeGuard::new();
    use base64::{engine::general_purpose::STANDARD as BASE64_STANDARD, Engine as _};
    let caller_ura = "easynet:///r/realm/device/test-daemon";
    let trust_dir = tempfile::tempdir().expect("runtime trust tempdir");
    let trust_path = trust_dir.path().join("realm-trust.toml");
    let anchor = RealmTrustAnchor::from_parts_with_principal_owners(
        vec![TrustedAgent {
            agent_ura: caller_ura.to_string(),
            public_key_b64: BASE64_STANDARD
                .encode(test_device_signing_key().verifying_key().to_bytes()),
            role: TrustedAgentRole::Device,
            added_at_unix_ms: 1,
            origin_realm: None,
            hub_endpoint: None,
            tls_ca_pem_path: None,
        }],
        vec![crate::daemon::trust::anchor::TrustedPrincipalOwner {
            principal_ura: caller_ura.to_string(),
            owner_user_id: "user-dev".to_string(),
            owner_ura: "easynet:///r/realm/user/user-dev".to_string(),
            owner_username: Some("dev".to_string()),
            added_at_unix_ms: 1,
        }],
        Vec::new(),
    )
    .expect("host owner anchor");
    let cell = crate::daemon::trust::cell::SharedTrustAnchor::new(Arc::new(anchor));
    let callee_ura = crate::core::ura::hub_ura("realm");
    let svc = register_test_daemon_routes(
        DaemonInvocationService::new(
            Arc::new(PresenceRegistry::new()),
            AdmissionFacade::with_trust_anchor_cell(cell.clone(), Some(callee_ura.clone())),
        )
        .with_register_pubkey("realm", trust_path, cell.clone())
        .with_session_realm("realm")
        .with_hub_signer(test_hub_signer("realm"))
        .with_test_daemon_runtime(cell),
        &callee_ura,
    );
    let agent_ura = "easynet:///r/realm/agent/dev.anthropic";
    let arguments = br#"{"agent_ura":"easynet:///r/realm/agent/dev.anthropic","generation":1,"signing_authority":{"kind":"hosted_by","host_ura":"easynet:///r/realm/device/test-daemon"},"host_node_id":"test-daemon"}"#;
    let request = Request::new(InvokeRequest {
        envelope: Some(signed_test_envelope(
            caller_ura,
            &callee_ura,
            agent_ura,
            ABILITY_FEDERATION_ADVERTISE_AGENT,
            arguments,
            &test_device_signing_key(),
        )),
        target: Some(
            wire_invocation_target(
                test_descriptor_ref(&callee_ura, ABILITY_FEDERATION_ADVERTISE_AGENT),
                ABILITY_FEDERATION_ADVERTISE_AGENT,
            )
            .expect("typed descriptor target"),
        ),
        arguments: arguments.to_vec(),
        ..InvokeRequest::default()
    });
    let access_control_stores = svc.admission.access_control_stores();
    grant_child_access_for_test(
        access_control_stores.as_ref(),
        ChildAccessGrantInput {
            owner_user_id: "dev",
            principal_kind: PrincipalKind::Device,
            principal_ura: caller_ura,
            token_class: None,
            callee_ura: &callee_ura,
            subject_ura: agent_ura,
            ability_ura: &test_owner_ability_ura(&callee_ura, ABILITY_FEDERATION_ADVERTISE_AGENT),
            action: AccessAction::Manage,
        },
    );
    let resp = svc.invoke(request).await.expect("dispatch returns Ok");
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

#[tokio::test]
async fn invoke_dispatches_federation_status() {
    let svc = make_service();
    let resp = svc
        .invoke(invoke_request(ABILITY_FEDERATION_STATUS, "{}"))
        .await
        .expect("dispatch returns Ok");
    let body: serde_json::Value = parse_response_body(resp);
    assert!(
        body.get("ok")
            .and_then(serde_json::Value::as_bool)
            .is_some(),
        "status payload must expose a stable ok boolean: {body}"
    );
    assert!(
        body.get("code")
            .and_then(serde_json::Value::as_str)
            .is_some(),
        "status payload must expose a stable machine code: {body}"
    );
    assert!(
        body.get("outcome").is_some(),
        "status payload must include outcome even while boot is still in progress: {body}"
    );
}

#[test]
fn session_control_heartbeat_renews_caller_owner_projection_lease() {
    let svc = make_service();
    let owner_ura = TEST_DAEMON_URA;
    let public_name = "agent.list";
    let ability_ura =
        crate::core::ura::owner_ability_ura(owner_ura, public_name).expect("ability ura");
    svc.directory.ability_catalog.upsert_projection(
        crate::daemon::federation::read_model::ability_catalog::OwnerAbilityProjectionRow::new(
            owner_ura.to_string(),
            owner_ura.to_string(),
            1,
            1,
            "sha256:test".to_string(),
            1,
            vec![crate::daemon::federation::read_model::owner_projection::AbilityProjectionSummary {
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
                callable_summary: crate::daemon::federation::read_model::owner_projection::AbilityCallableSummary::minimal(
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
    let owner_ura = TEST_DAEMON_URA;
    let ability_ura =
        crate::core::ura::owner_ability_ura(owner_ura, "agent.list").expect("device ability ura");
    svc.directory.presence.insert(owner_ura.to_string(), {
        let (tx, _rx) = mpsc::channel(1);
        tx
    });
    svc.directory.ability_catalog.upsert_projection(
        crate::daemon::federation::read_model::ability_catalog::OwnerAbilityProjectionRow::new(
            owner_ura.to_string(),
            owner_ura.to_string(),
            1,
            1,
            "sha256:test".to_string(),
            4_102_444_800_000,
            vec![crate::daemon::federation::read_model::owner_projection::AbilityProjectionSummary {
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
                callable_summary: crate::daemon::federation::read_model::owner_projection::AbilityCallableSummary::minimal(
                    "agent.list",
                ),
            }],
        ),
    );

    let resp = svc
        .invoke(invoke_request(
            ABILITY_NAMESPACE_RESOLVE,
            &serde_json::json!({
                "query_name": owner_ura,
                "qtype": "RESOLVE_TYPE_ROUTE",
                "ability_name": "agent.list",
            })
            .to_string(),
        ))
        .await
        .expect("namespace.resolve dispatch returns Ok");
    let body: serde_json::Value = parse_response_body(resp);

    assert_eq!(
        body["answer_kind"],
        ResolveAnswerKind::FinalRoute.as_str_name()
    );
    assert_eq!(body["ability_ura"], ability_ura);
    assert_eq!(
        body["next_hop"]["local_device_ability"]["device_ura"],
        TEST_DAEMON_URA
    );
}

#[tokio::test]
async fn namespace_resolve_cross_realm_route_returns_peer_hub_delegation() {
    let remote_owner = crate::core::ura::device_ura("remote-realm", "remote-device");
    let ability_ura =
        crate::core::ura::owner_ability_ura(&remote_owner, "observe.health").expect("ability ura");
    let svc = register_test_daemon_routes(
        make_unregistered_service_for_route_owner(TEST_DAEMON_URA)
            .with_session_realm("local-realm")
            .with_federated_peers(BTreeMap::from([(
                "remote-realm".to_string(),
                "https://remote-hub.example".to_string(),
            )])),
        TEST_DAEMON_URA,
    );

    let resp = svc
        .invoke(invoke_request(
            ABILITY_NAMESPACE_RESOLVE,
            &serde_json::json!({
                "query_name": ability_ura,
                "qtype": "RESOLVE_TYPE_ROUTE",
            })
            .to_string(),
        ))
        .await
        .expect("namespace.resolve dispatch returns Ok");
    let body: serde_json::Value = parse_response_body(resp);

    assert_eq!(
        body["answer_kind"],
        ResolveAnswerKind::Delegation.as_str_name()
    );
    assert_eq!(body["owner_ura"], remote_owner);
    assert_eq!(body["next_hop"]["peer_hub"]["realm"], "remote-realm");
    assert_eq!(
        body["next_hop"]["peer_hub"]["hub_ura"],
        crate::core::ura::hub_ura("remote-realm")
    );
    assert_eq!(
        body["next_hop"]["peer_hub"]["endpoints"][0]["endpoint"],
        "https://remote-hub.example"
    );
    assert_eq!(
        body["next_hop"]["peer_hub"]["endpoints"][0]["metadata"]["source"],
        "federated_peers"
    );
    assert_eq!(
        body["selected_route"]["reason"],
        RouteReason::PeerDelegation.as_str_name()
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
async fn invoke_dispatches_federation_discover_includes_local_presence_devices() {
    let presence = Arc::new(PresenceRegistry::new());
    presence.insert(
        crate::core::ura::device_ura("local-realm", "device-a"),
        tokio::sync::mpsc::channel(8).0,
    );
    let mut svc = make_unregistered_service_for_route_owner(TEST_DAEMON_URA)
        .with_session_realm("local-realm");
    svc.directory.presence = presence;
    let svc = register_test_daemon_routes(svc, TEST_DAEMON_URA);

    let resp = svc
        .invoke(invoke_request(ABILITY_FEDERATION_DISCOVER, "{}"))
        .await
        .expect("dispatch returns Ok");
    let body: federation_wrappers::DiscoverResponse = parse_response_body(resp);
    assert_eq!(body.entries.len(), 1);
    assert_eq!(
        body.entries[0].agent_ura,
        crate::core::ura::device_ura("local-realm", "device-a")
    );
    assert_eq!(body.entries[0].node_id, "device-a");
    assert_eq!(body.entries[0].status, "active");
}

#[tokio::test]
async fn invoke_dispatches_federation_discover_returns_peer_entries_when_view_populated() {
    // PR-N3 N3-4: when the federated_directory cell holds
    // entries (write side is the per-peer
    // RemoteDirectoryClient task in N3-3.1 — for this unit
    // test we manually `replace` the cell with a populated
    // map), discover surfaces them with origin_realm
    // stamped per §2.4.
    use crate::daemon::federation::directory::{
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

    let svc = register_test_daemon_routes(
        make_unregistered_service_for_route_owner(TEST_DAEMON_URA)
            .with_federated_directory_cell(cell),
        TEST_DAEMON_URA,
    );
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
    use crate::daemon::federation::directory::{
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

    let svc = register_test_daemon_routes(
        make_unregistered_service_for_route_owner(TEST_DAEMON_URA)
            .with_federated_directory_cell(cell),
        TEST_DAEMON_URA,
    );
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
    use crate::daemon::federation::directory::{
        DirectoryEntry, DirectoryView, SharedFederatedDirectoryView,
    };
    use crate::daemon::keyring::federated_bindings::FederatedBindingsStore;
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
    use crate::daemon::federation::directory::{
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

    let svc = register_test_daemon_routes(
        make_unregistered_service_for_route_owner(TEST_DAEMON_URA)
            .with_session_realm("realm-b")
            .with_federated_directory_cell(cell),
        TEST_DAEMON_URA,
    );

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
    use crate::daemon::federation::directory::{
        DirectoryEntry, DirectoryView, SharedFederatedDirectoryView,
    };
    use crate::daemon::keyring::federated_bindings::{
        FederatedBindingsStore, FederatedUserBinding,
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

    let svc = register_test_daemon_routes(
        make_unregistered_service_for_route_owner(TEST_DAEMON_URA)
            .with_session_realm("realm-b")
            .with_federated_directory_cell(cell)
            .with_federated_bindings_store(bindings),
        TEST_DAEMON_URA,
    );

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
    let expected_prefix = crate::core::ura::realm_device_prefix("realm-x");
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
    use crate::daemon::trust::anchor::{RealmTrustAnchor, TrustedAgent, TrustedAgentRole};
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
    let trust = SharedTrustAnchor::new(Arc::new(anchor_inner));
    let runtime_assembly = test_runtime_assembly(trust.clone());
    let admission =
        AdmissionFacade::with_trust_anchor_cell(trust, Some(TEST_DAEMON_URA.to_string()));
    let svc = register_test_daemon_routes(
        DaemonInvocationService::new(Arc::new(PresenceRegistry::new()), admission)
            .with_daemon_runtime(runtime_assembly),
        TEST_DAEMON_URA,
    );

    let function_name = ABILITY_FEDERATION_LIST_USER_DEVICES;
    let arguments = br#"{"realm":"realm-x"}"#.to_vec();
    let envelope = signed_test_envelope(
        device_caller_ura,
        TEST_DAEMON_URA,
        TEST_DAEMON_URA,
        function_name,
        &arguments,
        &signing_key,
    );
    let req = Request::new(InvokeRequest {
        envelope: Some(envelope),
        target: Some(
            wire_invocation_target(
                test_descriptor_ref(TEST_DAEMON_URA, function_name),
                function_name,
            )
            .expect("typed descriptor target"),
        ),
        arguments,
        ..InvokeRequest::default()
    });

    let error = expect_canonical_in_band_failure(
        svc.invoke(req).await,
        axon_sdk::invocation::ErrorCode::AbilityForbidden,
        "device-role caller must be rejected by N3-5 filter",
    );
    assert!(
        error.message.contains(device_caller_ura),
        "rejection message must surface the caller URA; got: {}",
        error.message
    );
}

#[tokio::test]
async fn identity_register_pubkey_rejects_device_caller_for_user_role_before_write() {
    use base64::{engine::general_purpose::STANDARD as BASE64_STANDARD, Engine as _};

    let device_caller_ura = "easynet:///r/local/device/device-writer";
    let device_signing_key = test_device_signing_key();
    let device_pubkey_b64 = BASE64_STANDARD.encode(device_signing_key.verifying_key().to_bytes());
    let cell = SharedTrustAnchor::new(Arc::new(
        RealmTrustAnchor::from_entries(vec![TrustedAgent {
            agent_ura: device_caller_ura.to_string(),
            public_key_b64: device_pubkey_b64,
            role: TrustedAgentRole::Device,
            added_at_unix_ms: 1_700_000_000_000,
            origin_realm: None,
            hub_endpoint: None,
            tls_ca_pem_path: None,
        }])
        .expect("device caller trust anchor"),
    ));
    let admission =
        AdmissionFacade::with_trust_anchor_cell(cell.clone(), Some(TEST_DAEMON_URA.to_string()));
    let trust_dir = tempfile::tempdir().expect("trust tempdir");
    let trust_path = trust_dir.path().join("realm-trust.toml");
    let svc = register_test_daemon_routes(
        DaemonInvocationService::new(Arc::new(PresenceRegistry::new()), admission)
            .with_register_pubkey("local", trust_path.clone(), cell.clone())
            .with_session_realm("local")
            .with_test_daemon_runtime(cell.clone()),
        TEST_DAEMON_URA,
    );

    let user_key = ed25519_dalek::SigningKey::from_bytes(&[0x55; 32]);
    let user_ura = "easynet:///r/local/user/user-1";
    let arguments = serde_json::to_vec(&serde_json::json!({
        "agent_ura": user_ura,
        "public_key_b64": BASE64_STANDARD.encode(user_key.verifying_key().to_bytes()),
        "role": "user",
    }))
    .expect("register args");
    let function_name = ABILITY_IDENTITY_REGISTER_PUBKEY;
    let envelope = signed_test_envelope(
        device_caller_ura,
        TEST_DAEMON_URA,
        TEST_DAEMON_URA,
        function_name,
        &arguments,
        &device_signing_key,
    );
    let req = Request::new(InvokeRequest {
        envelope: Some(envelope),
        target: Some(
            wire_invocation_target(
                test_descriptor_ref(TEST_DAEMON_URA, function_name),
                function_name,
            )
            .expect("typed descriptor target"),
        ),
        arguments,
        ..InvokeRequest::default()
    });

    let error = expect_canonical_in_band_failure(
        svc.invoke(req).await,
        axon_sdk::invocation::ErrorCode::AbilityForbidden,
        "device caller must not write user trust row",
    );
    assert!(
        error.message.contains("role `device`") && error.message.contains("`user` trust row"),
        "rejection should identify caller role and target role; got: {}",
        error.message
    );
    assert!(
        cell.snapshot().lookup_user_all(user_ura).is_empty(),
        "denied user row must not be published into the shared trust anchor",
    );
    assert!(
        !trust_path.exists(),
        "denied user row must not persist realm-trust.toml",
    );
}

#[tokio::test]
async fn identity_revoke_user_pubkey_rejects_device_caller_before_write() {
    use base64::{engine::general_purpose::STANDARD as BASE64_STANDARD, Engine as _};

    let device_caller_ura = "easynet:///r/local/device/device-writer";
    let device_signing_key = test_device_signing_key();
    let device_pubkey_b64 = BASE64_STANDARD.encode(device_signing_key.verifying_key().to_bytes());
    let user_key = ed25519_dalek::SigningKey::from_bytes(&[0x56; 32]);
    let user_pubkey_b64 = BASE64_STANDARD.encode(user_key.verifying_key().to_bytes());
    let user_ura = "easynet:///r/local/user/user-1";
    let cell = SharedTrustAnchor::new(Arc::new(
        RealmTrustAnchor::from_entries(vec![
            TrustedAgent {
                agent_ura: device_caller_ura.to_string(),
                public_key_b64: device_pubkey_b64,
                role: TrustedAgentRole::Device,
                added_at_unix_ms: 1_700_000_000_000,
                origin_realm: None,
                hub_endpoint: None,
                tls_ca_pem_path: None,
            },
            TrustedAgent {
                agent_ura: user_ura.to_string(),
                public_key_b64: user_pubkey_b64.clone(),
                role: TrustedAgentRole::User,
                added_at_unix_ms: 1_700_000_000_001,
                origin_realm: None,
                hub_endpoint: None,
                tls_ca_pem_path: None,
            },
        ])
        .expect("device caller plus user trust anchor"),
    ));
    let admission =
        AdmissionFacade::with_trust_anchor_cell(cell.clone(), Some(TEST_DAEMON_URA.to_string()));
    let trust_dir = tempfile::tempdir().expect("trust tempdir");
    let trust_path = trust_dir.path().join("realm-trust.toml");
    let svc = register_test_daemon_routes(
        DaemonInvocationService::new(Arc::new(PresenceRegistry::new()), admission)
            .with_register_pubkey("local", trust_path.clone(), cell.clone())
            .with_session_realm("local")
            .with_test_daemon_runtime(cell.clone()),
        TEST_DAEMON_URA,
    );

    let arguments = serde_json::to_vec(&serde_json::json!({
        "agent_ura": user_ura,
        "public_key_b64": user_pubkey_b64,
    }))
    .expect("revoke args");
    let function_name = ABILITY_IDENTITY_REVOKE_USER_PUBKEY;
    let envelope = signed_test_envelope(
        device_caller_ura,
        TEST_DAEMON_URA,
        TEST_DAEMON_URA,
        function_name,
        &arguments,
        &device_signing_key,
    );
    let req = Request::new(InvokeRequest {
        envelope: Some(envelope),
        target: Some(
            wire_invocation_target(
                test_descriptor_ref(TEST_DAEMON_URA, function_name),
                function_name,
            )
            .expect("typed descriptor target"),
        ),
        arguments,
        ..InvokeRequest::default()
    });

    let error = expect_canonical_in_band_failure(
        svc.invoke(req).await,
        axon_sdk::invocation::ErrorCode::AbilityForbidden,
        "device caller must not revoke user trust row",
    );
    assert!(
        error.message.contains("role `device`") && error.message.contains("revoke user trust row"),
        "rejection should identify caller role and mutation; got: {}",
        error.message
    );
    assert!(
        cell.snapshot()
            .lookup_user_by_pubkey(user_ura, &user_pubkey_b64)
            .is_some(),
        "denied revoke must leave the shared trust anchor untouched",
    );
    assert!(
        !trust_path.exists(),
        "denied revoke must not persist realm-trust.toml",
    );
}

#[tokio::test]
async fn identity_revoke_user_pubkey_removes_matching_presence_after_write() {
    use base64::{engine::general_purpose::STANDARD as BASE64_STANDARD, Engine as _};

    let backend_ura = crate::core::ura::hub_ura("local");
    let backend_signing_key = test_device_signing_key();
    let backend_pubkey_b64 = BASE64_STANDARD.encode(backend_signing_key.verifying_key().to_bytes());
    let user_key = ed25519_dalek::SigningKey::from_bytes(&[0x57; 32]);
    let user_pubkey_b64 = BASE64_STANDARD.encode(user_key.verifying_key().to_bytes());
    let user_ura = "easynet:///r/local/user/user-1";
    let cell = SharedTrustAnchor::new(Arc::new(
        RealmTrustAnchor::from_entries(vec![
            TrustedAgent {
                agent_ura: backend_ura.clone(),
                public_key_b64: backend_pubkey_b64,
                role: TrustedAgentRole::Backend,
                added_at_unix_ms: 1_700_000_000_000,
                origin_realm: None,
                hub_endpoint: None,
                tls_ca_pem_path: None,
            },
            TrustedAgent {
                agent_ura: user_ura.to_string(),
                public_key_b64: user_pubkey_b64.clone(),
                role: TrustedAgentRole::User,
                added_at_unix_ms: 1_700_000_000_001,
                origin_realm: None,
                hub_endpoint: None,
                tls_ca_pem_path: None,
            },
        ])
        .expect("backend plus user trust anchor"),
    ));
    let admission =
        AdmissionFacade::with_trust_anchor_cell(cell.clone(), Some(TEST_DAEMON_URA.to_string()));
    let presence = Arc::new(PresenceRegistry::new());
    let (tx, _rx) = tokio::sync::mpsc::channel(1);
    presence.insert_negotiated_with_trust(
        user_ura.to_string(),
        tx,
        crate::daemon::invocation::bidi::state::presence::SessionContract::canonical(),
        crate::daemon::invocation::bidi::state::presence::SessionTrustContext::user_pubkey(
            user_pubkey_b64.clone(),
        ),
    );
    let mut events = presence.subscribe_events();
    let trust_dir = tempfile::tempdir().expect("trust tempdir");
    let trust_path = trust_dir.path().join("realm-trust.toml");
    let svc = register_test_daemon_routes(
        DaemonInvocationService::new(Arc::clone(&presence), admission)
            .with_register_pubkey("local", trust_path, cell.clone())
            .with_session_realm("local")
            .with_test_daemon_runtime(cell.clone()),
        TEST_DAEMON_URA,
    );

    let arguments = serde_json::to_vec(&serde_json::json!({
        "agent_ura": user_ura,
        "public_key_b64": user_pubkey_b64,
    }))
    .expect("revoke args");
    let function_name = ABILITY_IDENTITY_REVOKE_USER_PUBKEY;
    let envelope = signed_test_envelope(
        &backend_ura,
        TEST_DAEMON_URA,
        TEST_DAEMON_URA,
        function_name,
        &arguments,
        &backend_signing_key,
    );
    let resp = svc
        .invoke(Request::new(InvokeRequest {
            envelope: Some(envelope),
            target: Some(
                wire_invocation_target(
                    test_descriptor_ref(TEST_DAEMON_URA, function_name),
                    function_name,
                )
                .expect("typed descriptor target"),
            ),
            arguments,
            ..InvokeRequest::default()
        }))
        .await
        .expect("backend caller may revoke user key");

    let body: crate::daemon::invocation::admission::revoke_user_pubkey::RevokeResponse =
        parse_response_body(resp);
    assert!(body.ok);
    assert!(body.removed);
    assert!(
        cell.snapshot()
            .lookup_user_by_pubkey(user_ura, &user_pubkey_b64)
            .is_none(),
        "successful revoke removes the trust row before runtime invalidation",
    );
    assert!(
        !presence.contains(user_ura),
        "successful revoke must force-remove matching runtime presence",
    );
    let event = tokio::time::timeout(std::time::Duration::from_secs(1), events.recv())
        .await
        .expect("offline event emitted")
        .expect("presence event");
    match event {
        crate::daemon::invocation::bidi::state::presence::PresenceEvent::Offline {
            ura,
            reason,
        } => {
            assert_eq!(ura, user_ura);
            assert_eq!(
                reason,
                crate::daemon::invocation::bidi::state::presence::OfflineReason::AdminRevoked,
            );
        }
        other => panic!("expected AdminRevoked offline event, got {other:?}"),
    }
}

#[tokio::test]
async fn identity_revoke_user_pubkey_removes_user_hosted_agents_and_host_presence() {
    use base64::{engine::general_purpose::STANDARD as BASE64_STANDARD, Engine as _};

    let backend_ura = crate::core::ura::hub_ura("local");
    let backend_signing_key = test_device_signing_key();
    let backend_pubkey_b64 = BASE64_STANDARD.encode(backend_signing_key.verifying_key().to_bytes());
    let user_key = ed25519_dalek::SigningKey::from_bytes(&[0x59; 32]);
    let user_pubkey_b64 = BASE64_STANDARD.encode(user_key.verifying_key().to_bytes());
    let user_ura = "easynet:///r/local/user/alice";
    let host_ura = "easynet:///r/local/device/alice-laptop";
    let cell = SharedTrustAnchor::new(Arc::new(
        RealmTrustAnchor::from_entries(vec![
            TrustedAgent {
                agent_ura: backend_ura.clone(),
                public_key_b64: backend_pubkey_b64,
                role: TrustedAgentRole::Backend,
                added_at_unix_ms: 1_700_000_000_000,
                origin_realm: None,
                hub_endpoint: None,
                tls_ca_pem_path: None,
            },
            TrustedAgent {
                agent_ura: user_ura.to_string(),
                public_key_b64: user_pubkey_b64.clone(),
                role: TrustedAgentRole::User,
                added_at_unix_ms: 1_700_000_000_001,
                origin_realm: None,
                hub_endpoint: None,
                tls_ca_pem_path: None,
            },
        ])
        .expect("backend plus user trust anchor"),
    ));
    let admission =
        AdmissionFacade::with_trust_anchor_cell(cell.clone(), Some(TEST_DAEMON_URA.to_string()));
    let presence = Arc::new(PresenceRegistry::new());
    let (tx, _rx) = tokio::sync::mpsc::channel(1);
    presence.insert(host_ura.to_string(), tx);
    let mut events = presence.subscribe_events();
    let trust_dir = tempfile::tempdir().expect("trust tempdir");
    let trust_path = trust_dir.path().join("realm-trust.toml");
    let svc = register_test_daemon_routes(
        DaemonInvocationService::new(Arc::clone(&presence), admission)
            .with_register_pubkey("local", trust_path, cell.clone())
            .with_session_realm("local")
            .with_test_daemon_runtime(cell.clone()),
        TEST_DAEMON_URA,
    );

    for agent_ura in [
        "easynet:///r/local/agent/alice.helper",
        "easynet:///r/local/agent/alice.researcher",
    ] {
        svc.directory.advertised_agents.upsert(
            crate::daemon::federation::read_model::advertised_agents::AdvertisedAgentRecord {
                agent_ura: agent_ura.to_string(),
                generation: 1,
                public_key_hex: String::new(),
                host_node_id: Some("alice-laptop".to_string()),
                signing_authority:
                    crate::daemon::federation::read_model::advertised_agents::AdvertisedAgentSigningAuthority::HostedBy {
                        host_ura: host_ura.to_string(),
                    },
            },
        );
    }
    svc.directory.advertised_agents.upsert(
        crate::daemon::federation::read_model::advertised_agents::AdvertisedAgentRecord {
            agent_ura: "easynet:///r/local/agent/bob.helper".to_string(),
            generation: 1,
            public_key_hex: String::new(),
            host_node_id: Some("bob-laptop".to_string()),
            signing_authority:
                crate::daemon::federation::read_model::advertised_agents::AdvertisedAgentSigningAuthority::HostedBy {
                    host_ura: "easynet:///r/local/device/bob-laptop".to_string(),
                },
        },
    );

    let arguments = serde_json::to_vec(&serde_json::json!({
        "agent_ura": user_ura,
        "public_key_b64": user_pubkey_b64,
    }))
    .expect("revoke args");
    let function_name = ABILITY_IDENTITY_REVOKE_USER_PUBKEY;
    let envelope = signed_test_envelope(
        &backend_ura,
        TEST_DAEMON_URA,
        TEST_DAEMON_URA,
        function_name,
        &arguments,
        &backend_signing_key,
    );
    let resp = svc
        .invoke(Request::new(InvokeRequest {
            envelope: Some(envelope),
            target: Some(
                wire_invocation_target(
                    test_descriptor_ref(TEST_DAEMON_URA, function_name),
                    function_name,
                )
                .expect("typed descriptor target"),
            ),
            arguments,
            ..InvokeRequest::default()
        }))
        .await
        .expect("backend caller may revoke user key");

    let body: crate::daemon::invocation::admission::revoke_user_pubkey::RevokeResponse =
        parse_response_body(resp);
    assert!(body.ok);
    assert!(body.removed);
    assert!(
        !presence.contains(host_ura),
        "successful user revoke must force-remove the host device that made owned agents online",
    );
    assert!(svc
        .directory
        .advertised_agents
        .get("easynet:///r/local/agent/alice.helper")
        .is_none());
    assert!(svc
        .directory
        .advertised_agents
        .get("easynet:///r/local/agent/alice.researcher")
        .is_none());
    assert!(svc
        .directory
        .advertised_agents
        .get("easynet:///r/local/agent/bob.helper")
        .is_some());

    let event = tokio::time::timeout(std::time::Duration::from_secs(1), events.recv())
        .await
        .expect("offline event emitted")
        .expect("presence event");
    match event {
        crate::daemon::invocation::bidi::state::presence::PresenceEvent::Offline {
            ura,
            reason,
        } => {
            assert_eq!(ura, host_ura);
            assert_eq!(
                reason,
                crate::daemon::invocation::bidi::state::presence::OfflineReason::AdminRevoked,
            );
        }
        other => panic!("expected AdminRevoked host offline event, got {other:?}"),
    }
    match tokio::time::timeout(std::time::Duration::from_millis(50), events.recv()).await {
        Err(_elapsed) => {}
        Ok(other) => panic!("shared host must be revoked once, got extra event {other:?}"),
    }
}

#[tokio::test]
async fn identity_revoke_user_pubkey_idempotent_miss_keeps_presence() {
    use base64::{engine::general_purpose::STANDARD as BASE64_STANDARD, Engine as _};

    let backend_ura = crate::core::ura::hub_ura("local");
    let backend_signing_key = test_device_signing_key();
    let backend_pubkey_b64 = BASE64_STANDARD.encode(backend_signing_key.verifying_key().to_bytes());
    let missing_key = ed25519_dalek::SigningKey::from_bytes(&[0x58; 32]);
    let missing_pubkey_b64 = BASE64_STANDARD.encode(missing_key.verifying_key().to_bytes());
    let user_ura = "easynet:///r/local/user/user-1";
    let cell = SharedTrustAnchor::new(Arc::new(
        RealmTrustAnchor::from_entries(vec![TrustedAgent {
            agent_ura: backend_ura.clone(),
            public_key_b64: backend_pubkey_b64,
            role: TrustedAgentRole::Backend,
            added_at_unix_ms: 1_700_000_000_000,
            origin_realm: None,
            hub_endpoint: None,
            tls_ca_pem_path: None,
        }])
        .expect("backend trust anchor"),
    ));
    let admission =
        AdmissionFacade::with_trust_anchor_cell(cell.clone(), Some(TEST_DAEMON_URA.to_string()));
    let presence = Arc::new(PresenceRegistry::new());
    let (tx, _rx) = tokio::sync::mpsc::channel(1);
    presence.insert(user_ura.to_string(), tx);
    let mut events = presence.subscribe_events();
    let trust_dir = tempfile::tempdir().expect("trust tempdir");
    let trust_path = trust_dir.path().join("realm-trust.toml");
    let svc = register_test_daemon_routes(
        DaemonInvocationService::new(Arc::clone(&presence), admission)
            .with_register_pubkey("local", trust_path, cell.clone())
            .with_session_realm("local")
            .with_test_daemon_runtime(cell),
        TEST_DAEMON_URA,
    );

    let arguments = serde_json::to_vec(&serde_json::json!({
        "agent_ura": user_ura,
        "public_key_b64": missing_pubkey_b64,
    }))
    .expect("revoke args");
    let function_name = ABILITY_IDENTITY_REVOKE_USER_PUBKEY;
    let envelope = signed_test_envelope(
        &backend_ura,
        TEST_DAEMON_URA,
        TEST_DAEMON_URA,
        function_name,
        &arguments,
        &backend_signing_key,
    );
    let resp = svc
        .invoke(Request::new(InvokeRequest {
            envelope: Some(envelope),
            target: Some(
                wire_invocation_target(
                    test_descriptor_ref(TEST_DAEMON_URA, function_name),
                    function_name,
                )
                .expect("typed descriptor target"),
            ),
            arguments,
            ..InvokeRequest::default()
        }))
        .await
        .expect("idempotent revoke miss succeeds");

    let body: crate::daemon::invocation::admission::revoke_user_pubkey::RevokeResponse =
        parse_response_body(resp);
    assert!(body.ok);
    assert!(!body.removed);
    assert!(
        presence.contains(user_ura),
        "removed=false must not emit runtime offline side effects",
    );
    match tokio::time::timeout(std::time::Duration::from_millis(50), events.recv()).await {
        Err(_elapsed) => {}
        Ok(other) => panic!("idempotent revoke miss must not emit offline event, got {other:?}"),
    }
}

#[tokio::test]
async fn invoke_dispatches_federation_proxy_list_user_devices_fans_out_and_stamps_peer_metadata() {
    use crate::daemon::trust::anchor::{TrustedAgent, TrustedAgentRole};

    let peer_hub_url = "https://peer-hub.example:50443";
    let peer_hub_ura = crate::core::ura::hub_ura("peer-realm");
    let anchor = Arc::new(test_trust_anchor_with_entries(vec![TrustedAgent {
        agent_ura: peer_hub_ura.clone(),
        public_key_b64: "AAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAA=".to_string(),
        role: TrustedAgentRole::Hub,
        added_at_unix_ms: 1_700_000_000_000,
        origin_realm: Some("peer-realm".to_string()),
        hub_endpoint: Some(peer_hub_url.to_string()),
        tls_ca_pem_path: None,
    }]));
    let runtime_cell = SharedTrustAnchor::new(anchor.clone());
    let admission = AdmissionFacade::new(anchor, Some(TEST_DAEMON_URA.to_string()));
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
        state: axon_sdk::invocation::InvocationState::Completed.to_wire_i32(),
        ..InvokeResponse::default()
    };
    let recorder = Arc::new(RecordingFederationClient::new(canned));
    let svc = register_test_daemon_routes(
        DaemonInvocationService::new(Arc::new(PresenceRegistry::new()), admission)
            .with_hub_signer(test_hub_signer("local-realm"))
            .with_session_realm("local-realm")
            .with_federation_client(recorder.clone() as Arc<dyn FederationClient>)
            .with_test_daemon_runtime(runtime_cell),
        TEST_DAEMON_URA,
    );

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
        invocation_function_name(&calls[0].1),
        ABILITY_FEDERATION_LIST_USER_DEVICES
    );
    let peer_args: federation_wrappers::ListUserDevicesRequest =
        serde_json::from_slice(&calls[0].1.arguments).expect("peer args decode");
    assert_eq!(peer_args.realm, "user-realm");
}

#[tokio::test]
async fn federation_proxy_caller_gate_accepts_local_hub_identity_with_hub_role() {
    use crate::daemon::trust::anchor::{RealmTrustAnchor, TrustedAgent, TrustedAgentRole};

    let local_hub_ura = crate::core::ura::hub_ura("local-realm");
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
    let runtime_cell = SharedTrustAnchor::new(anchor.clone());
    let admission = AdmissionFacade::new(anchor, Some(TEST_DAEMON_URA.to_string()));
    let svc = register_test_daemon_routes(
        DaemonInvocationService::new(Arc::new(PresenceRegistry::new()), admission)
            .with_session_realm("local-realm")
            .with_test_daemon_runtime(runtime_cell),
        TEST_DAEMON_URA,
    );
    let envelope = Envelope {
        caller: Some(AgentIdentity {
            ura: local_hub_ura,
            profile: "easynet-strict-v2".to_string(),
        }),
        ..Envelope::default()
    };

    svc.unary_dispatcher()
        .require_backend_or_local_self_proxy_caller(Some(&envelope), "namespace.proxy_resolve")
        .expect("local canonical hub identity is the backend proxy caller");
}

#[tokio::test]
async fn invoke_dispatches_federation_proxy_list_user_devices_rejects_hub_role_caller() {
    use base64::{engine::general_purpose::STANDARD as BASE64_STANDARD, Engine as _};

    use crate::daemon::trust::anchor::{RealmTrustAnchor, TrustedAgent, TrustedAgentRole};

    let caller_ura = crate::core::ura::hub_ura("peer-realm");
    let hub_signer = test_hub_signer_with_seed("peer-realm", [0x22; 32]);
    let caller_pubkey_b64 = BASE64_STANDARD.encode(
        hub_signer
            .signing_public_key()
            .expect("peer hub signer public key")
            .to_bytes(),
    );
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
    let runtime_cell = SharedTrustAnchor::new(anchor.clone());
    let admission = AdmissionFacade::new(anchor, Some(TEST_DAEMON_URA.to_string()));
    let local_hub_ura = crate::core::ura::hub_ura("local-realm");
    let svc = register_test_daemon_routes(
        DaemonInvocationService::new(Arc::new(PresenceRegistry::new()), admission)
            .with_session_realm("local-realm")
            .with_test_daemon_runtime(runtime_cell),
        &local_hub_ura,
    );

    let args = br#"{"realm":"user-realm","peer_hub_urls":["https://peer-hub.example:50443"]}"#;
    let descriptor_subject_ura = crate::core::ura::owner_ability_ura(
        &crate::core::ura::hub_ura("local-realm"),
        ABILITY_FEDERATION_PROXY_LIST_USER_DEVICES,
    )
    .expect("hub proxy ability subject");
    let mut envelope = Envelope {
        caller: Some(AgentIdentity {
            ura: caller_ura.clone(),
            profile: "easynet-strict-v2".to_string(),
        }),
        callee: Some(AgentIdentity {
            ura: crate::core::ura::hub_ura("local-realm"),
            profile: "easynet-strict-v2".to_string(),
        }),
        subject: Some(SubjectIdentity {
            ura: descriptor_subject_ura.clone(),
            profile: "easynet-strict-v2".to_string(),
        }),
        invocation_nonce: vec![7; 16],
        ..Envelope::default()
    };
    let descriptor_ref =
        test_descriptor_ref(&local_hub_ura, ABILITY_FEDERATION_PROXY_LIST_USER_DEVICES);
    let signed_descriptor_ref = sign_peer_request_envelope(
        &mut envelope,
        ABILITY_FEDERATION_PROXY_LIST_USER_DEVICES,
        &descriptor_ref,
        args,
        Some("peer-realm"),
        Some(hub_signer.as_ref()),
    )
    .await
    .expect("sign test envelope");

    let mut request = InvokeRequest {
        envelope: Some(envelope),
        target: Some(
            wire_invocation_target(
                &signed_descriptor_ref,
                ABILITY_FEDERATION_PROXY_LIST_USER_DEVICES,
            )
            .expect("typed descriptor target"),
        ),
        arguments: args.to_vec(),
        ..InvokeRequest::default()
    };
    request.metadata.insert(
        "x-easynet-delegation".to_string(),
        signed_delegation_metadata_for_test(
            hub_signer.as_ref(),
            &caller_ura,
            &descriptor_subject_ura,
            &caller_ura,
            &local_hub_ura,
            &[ABILITY_FEDERATION_PROXY_LIST_USER_DEVICES],
        )
        .await,
    );

    let error = expect_canonical_in_band_failure(
        svc.invoke(Request::new(request)).await,
        axon_sdk::invocation::ErrorCode::AbilityForbidden,
        "hub-role caller must be rejected by proxy filter",
    );
    assert!(
        error.message.contains(&caller_ura),
        "rejection message must surface the caller URA; got: {}",
        error.message
    );
}

#[tokio::test]
async fn invoke_dispatches_namespace_proxy_resolve_to_typed_peer_surface() {
    use crate::daemon::trust::anchor::{TrustedAgent, TrustedAgentRole};

    let peer_hub_url = "https://peer-hub.example:50443";
    let peer_hub_ura = crate::core::ura::hub_ura("peer-realm");
    let anchor = Arc::new(test_trust_anchor_with_entries(vec![TrustedAgent {
        agent_ura: peer_hub_ura,
        public_key_b64: "AAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAA=".to_string(),
        role: TrustedAgentRole::Hub,
        added_at_unix_ms: 1_700_000_000_000,
        origin_realm: Some("peer-realm".to_string()),
        hub_endpoint: Some(peer_hub_url.to_string()),
        tls_ca_pem_path: None,
    }]));
    let runtime_cell = SharedTrustAnchor::new(anchor.clone());
    let admission = AdmissionFacade::new(anchor, Some(TEST_DAEMON_URA.to_string()));
    let owner_ura = "easynet:///r/peer-realm/device/dev-peer";
    let ability_ura =
        crate::core::ura::owner_ability_ura(owner_ura, "agent.list").expect("ability ura");
    let canned = InvokeResponse {
        result: serde_json::to_vec(&serde_json::json!({
            "answer_kind": "RESOLVE_ANSWER_KIND_NON_DISPATCHABLE",
            "records": [
                {
                    "name": owner_ura,
                    "record_type": "RECORD_TYPE_ID",
                    "value": {
                        "id": {
                            "ura": owner_ura,
                            "kind": "URA_KIND_DEVICE"
                        }
                    }
                },
                {
                    "name": ability_ura,
                    "record_type": "RECORD_TYPE_ABILITY",
                    "value": {
                        "ability": {
                            "ability_ura": ability_ura,
                            "owner_ura": owner_ura,
                            "namespace": "agent",
                            "local_name": "list"
                        }
                    }
                }
            ],
            "release_profile": "RESOLVER_RELEASE_PROFILE_AUTHORITATIVE_LOCAL",
            "cache_policy": {
                "ttl_ms": 0,
                "shared_cacheable": false,
                "retry_after_unix_ms": 0
            }
        }))
        .expect("typed resolve answer fixture"),
        result_content_type: FEDERATION_RESULT_CONTENT_TYPE.to_string(),
        state: axon_sdk::invocation::InvocationState::Completed.to_wire_i32(),
        ..InvokeResponse::default()
    };
    let recorder = Arc::new(RecordingFederationClient::new(canned));
    let svc = register_test_daemon_routes(
        DaemonInvocationService::new(Arc::new(PresenceRegistry::new()), admission)
            .with_hub_signer(test_hub_signer("local-realm"))
            .with_session_realm("local-realm")
            .with_federation_client(recorder.clone() as Arc<dyn FederationClient>)
            .with_test_daemon_runtime(runtime_cell),
        TEST_DAEMON_URA,
    );

    let resp = svc
        .invoke(invoke_request(
            ABILITY_NAMESPACE_PROXY_RESOLVE,
            r#"{
                "peer_hub_urls":["https://peer-hub.example:50443"],
                "query_name":"easynet:///r/peer-realm/device/",
                "qtype":"RESOLVE_TYPE_DIRECTORY_LISTING",
                "caller_ura":"easynet:///r/local-realm/hub",
                "subject_ura":"easynet:///r/local-realm/user/alice",
                "realm_hint":"peer-realm"
            }"#,
        ))
        .await
        .expect("namespace proxy resolve succeeds");
    let body: serde_json::Value = parse_response_body(resp);
    assert_eq!(
        body["answer_kind"], "RESOLVE_ANSWER_KIND_NON_DISPATCHABLE",
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
    assert_eq!(
        invocation_function_name(&calls[0].1),
        ABILITY_NAMESPACE_RESOLVE
    );
    let peer_args: serde_json::Value =
        serde_json::from_slice(&calls[0].1.arguments).expect("peer args decode");
    assert_eq!(peer_args["query_name"], "easynet:///r/peer-realm/device/");
    assert_eq!(peer_args["qtype"], "RESOLVE_TYPE_DIRECTORY_LISTING");
}

#[tokio::test]
async fn invoke_rejects_namespace_proxy_resolve_legacy_camel_case_input_aliases() {
    let svc = make_service();

    let error = expect_canonical_in_band_failure(
        svc.invoke(invoke_request(
            ABILITY_NAMESPACE_PROXY_RESOLVE,
            r#"{
                "peer_hub_urls":[],
                "queryName":"easynet:///r/peer-realm/device/",
                "qtype":"RESOLVE_TYPE_DIRECTORY_LISTING",
                "callerUra":"easynet:///r/local-realm/hub",
                "subjectUra":"easynet:///r/local-realm/user/alice",
                "realmHint":"peer-realm"
            }"#,
        ))
        .await,
        axon_sdk::invocation::ErrorCode::RequestPayloadInvalid,
        "legacy camel-case namespace proxy input must be rejected",
    );
    assert!(
        error.message.contains("unknown field `queryName`"),
        "rejection must name the retired input alias; got: {}",
        error.message
    );
}

#[tokio::test]
async fn invoke_dispatches_federation_resolve_key_returns_pubkey_when_present() {
    // PR-N2 commit 2/N: peer-side `federation.resolve_key`
    // surfaces the local trust anchor's `public_key_b64` for
    // a known URA. Cross-hub `FederatedKeyResolver` consumes
    // this exact wire shape.
    use crate::daemon::trust::anchor::{TrustedAgent, TrustedAgentRole};
    let entry = TrustedAgent {
        agent_ura: "easynet:///r/realm-a/device/n1".to_string(),
        public_key_b64: "AAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAA=".to_string(),
        role: TrustedAgentRole::Device,
        added_at_unix_ms: 1_700_000_000_000,
        origin_realm: None,
        hub_endpoint: None,
        tls_ca_pem_path: None,
    };
    let anchor = Arc::new(test_trust_anchor_with_entries(vec![entry]));
    let runtime_cell = SharedTrustAnchor::new(anchor.clone());
    let admission = AdmissionFacade::new(anchor, Some(TEST_DAEMON_URA.to_string()));
    let svc = register_test_daemon_routes(
        DaemonInvocationService::new(Arc::new(PresenceRegistry::new()), admission)
            .with_test_daemon_runtime(runtime_cell),
        TEST_DAEMON_URA,
    );

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

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn invoke_dispatches_federation_resolve_key_uses_federated_resolver_on_local_miss() {
    let peer_hub_url = "https://peer-hub.example:50443";
    let peer_caller_ura = "easynet:///r/peer-realm/device/n1";
    let peer_public_key_b64 = "AAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAA=";
    let canned = InvokeResponse {
        result: serde_json::to_vec(&federation_wrappers::resolve_key_response(
            peer_public_key_b64,
            Vec::new(),
        ))
        .expect("resolve_key response serializes"),
        result_content_type: FEDERATION_RESULT_CONTENT_TYPE.to_string(),
        state: axon_sdk::invocation::InvocationState::Completed.to_wire_i32(),
        ..InvokeResponse::default()
    };
    let recorder = Arc::new(RecordingFederationClient::new(canned));
    let peers = crate::daemon::federation::peers::SharedFederatedPeers::new(
        std::collections::BTreeMap::from([("peer-realm".to_string(), peer_hub_url.to_string())]),
    );
    let runtime_cell = SharedTrustAnchor::new(Arc::new(test_trust_anchor()));
    let resolver = Arc::new(
        crate::daemon::invocation::admission::federated_key_resolver::FederatedKeyResolver::new(
            runtime_cell.clone(),
            Some(recorder.clone() as Arc<dyn FederationClient>),
            peers,
            Some("test-realm".to_string()),
        )
        .with_hub_signer(test_hub_signer("test-realm")),
    );
    let admission = AdmissionFacade::with_trust_anchor_cell(
        runtime_cell.clone(),
        Some(TEST_DAEMON_URA.to_string()),
    )
    .with_federated_key_resolver(resolver);
    let svc = register_test_daemon_routes(
        DaemonInvocationService::new(Arc::new(PresenceRegistry::new()), admission)
            .with_test_daemon_runtime(runtime_cell),
        TEST_DAEMON_URA,
    );

    let resp = svc
        .invoke(invoke_request(
            ABILITY_FEDERATION_RESOLVE_KEY,
            r#"{"agent_ura":"easynet:///r/peer-realm/device/n1"}"#,
        ))
        .await
        .expect("federated resolve_key succeeds");
    let body: federation_wrappers::ResolveKeyResponse = parse_response_body(resp);
    assert_eq!(body.public_key_b64, peer_public_key_b64);

    let calls = recorder.calls();
    assert_eq!(calls.len(), 1, "local miss must dial the peer hub once");
    assert_eq!(calls[0].0, peer_hub_url);
    assert_eq!(
        invocation_function_name(&calls[0].1),
        ABILITY_FEDERATION_RESOLVE_KEY
    );
    let peer_args: federation_wrappers::ResolveKeyRequest =
        serde_json::from_slice(&calls[0].1.arguments).expect("peer args decode");
    assert_eq!(peer_args.agent_ura, peer_caller_ura);
}

#[tokio::test]
async fn invoke_dispatches_federation_resolve_key_returns_not_found_when_ura_unknown() {
    // The canonical terminal failure preserves both the resource condition
    // and the URA so operators can identify the unresolved principal.
    let svc = make_service();
    let error = expect_canonical_in_band_failure(
        svc.invoke(invoke_request(
            ABILITY_FEDERATION_RESOLVE_KEY,
            r#"{"agent_ura":"easynet:///r/realm-a/device/missing"}"#,
        ))
        .await,
        axon_sdk::invocation::ErrorCode::NotFound,
        "unknown URA must surface a canonical resource failure",
    );
    assert!(
        error
            .message
            .contains("easynet:///r/realm-a/device/missing"),
        "expected the missing URA in error message, got: {}",
        error.message
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
            assert_eq!(err.code(), tonic::Code::NotFound);
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
    use axon_sdk::invocation::{make_ability, AbilityCallModes, AbilityOptions};

    let runtime_assembly = test_runtime_with_default_trust();
    let rt = runtime_assembly.runtime();
    let ability = "test.fallback.echo";
    let ability_ura = crate::core::ura::owner_ability_ura(TEST_DAEMON_URA, ability).unwrap();
    rt.register_ability_with_options(
        ability_ura,
        make_ability(|ctx| async move { Ok(ctx.payload.clone()) }),
        AbilityOptions::default()
            .with_modes(AbilityCallModes::RPC)
            .with_descriptor_proof(
                crate::daemon::ability::DEFAULT_ABILITY_DESCRIPTOR_VERSION,
                "invoke",
                [0x33; 32],
                [0x11; 32],
                [0x22; 32],
            ),
    )
    .await
    .unwrap();

    let svc = make_service_with_test_runtime(runtime_assembly);
    publish_test_route(&svc, TEST_DAEMON_URA, ability);
    sync_runtime_proof_from_catalog(
        &svc,
        TEST_DAEMON_URA,
        ability,
        crate::daemon::ability::CallMode::Rpc,
    )
    .await;
    let mut request = invoke_request(ability, r#"{"hello":"world"}"#).into_inner();
    let descriptor_ref = catalog_test_descriptor_ref(
        svc.directory.local_ability_catalog.as_ref().unwrap(),
        TEST_DAEMON_URA,
        ability,
        crate::daemon::ability::CallMode::Rpc,
    );
    bind_invoke_request_to_descriptor_ref(
        &mut request,
        TEST_DAEMON_URA,
        TEST_DAEMON_URA,
        TEST_DAEMON_URA,
        descriptor_ref,
        &test_device_signing_key(),
    );
    let resp = svc
        .invoke(Request::new(request))
        .await
        .expect("selected-route dispatch succeeds");
    let body: serde_json::Value = parse_response_body(resp);
    assert_eq!(body["hello"], "world");
}

#[tokio::test]
async fn invoke_selected_route_unknown_runtime_handler_surfaces_not_found() {
    let rt = test_runtime_with_default_trust();
    let svc = make_service_with_test_runtime(rt.clone());
    publish_test_route(&svc, TEST_DAEMON_URA, "nope.nope");

    match svc.invoke(invoke_request("nope.nope", "{}")).await {
        Err(err) => {
            assert_eq!(err.code(), tonic::Code::NotFound);
            assert!(
                err.message().contains("selected route")
                    && err.message().contains("nope.nope")
                    && err
                        .message()
                        .contains("not registered in Axon LocalRuntime"),
                "expected the stale-route runtime-miss message; got: {}",
                err.message()
            );
        }
        Ok(_) => panic!("unregistered ability must be rejected"),
    }
}

#[tokio::test]
async fn invoke_runtime_bootstrap_self_identity_is_not_cli_shadow_acked() {
    // No SDK admin installed: descriptor-bound routing may resolve the
    // Hub-owned runtime-admin contract, but LocalRuntime must still report
    // the missing canonical runtime row. The daemon must not fabricate a
    // CLI-side ack or fall back to a bare `runtime.*` dispatch.
    let rt = test_runtime_with_default_trust();
    let svc =
        make_service_with_test_runtime(rt.clone()).with_session_realm("test-realm".to_string());
    let hub_ura = crate::core::ura::hub_ura("test-realm");
    let args = r#"{
        "tenant_id":"tenant-a",
        "node_id":"node-a",
        "owner_id":"node-a",
        "public_key_b64":"AAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAA="
    }"#;

    match svc
        .invoke(signed_invoke_request(
            TEST_BOOTSTRAP_CALLER_URA,
            &hub_ura,
            TEST_BOOTSTRAP_CALLER_URA,
            federation_wrappers::ABILITY_RUNTIME_BOOTSTRAP_SELF_IDENTITY,
            args,
            &test_bootstrap_caller_signing_key(),
        ))
        .await
    {
        Err(err) => {
            assert_eq!(err.code(), tonic::Code::NotFound);
            assert!(
                err.message()
                    .contains("not registered in Axon LocalRuntime as"),
                "expected descriptor-bound LocalRuntime missing-handler diagnostic; got: {}",
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

    // Runtime admin is installed as a Hub-owned descriptor-bound ability.
    // The bare `runtime.bootstrap_self_identity` key must disappear; dispatch
    // enters through the canonical Ability URA selected by namespace.resolve.
    let svc = make_service().with_session_realm("test-realm".to_string());
    let rt = svc
        .runtime
        .local_runtime()
        .expect("test service has LocalRuntime");
    let catalog = svc
        .directory
        .local_ability_catalog
        .as_ref()
        .expect("test service has local ability catalog")
        .clone();
    let hub_ura = crate::core::ura::hub_ura("test-realm");
    let bootstrap_identity_provider = svc
        .runtime
        .daemon_admission_graph()
        .expect("test service shares the LocalRuntime admission resolver")
        .bootstrap_identity_provider();
    let install =
        crate::daemon::axon_bridge::runtime_admin::register_runtime_bootstrap_identity_ability(
            &rt,
            catalog.as_ref(),
            &hub_ura,
            bootstrap_identity_provider,
        )
        .await
        .expect("descriptor-bound runtime admin installs");
    assert!(
        rt.ability_options(federation_wrappers::ABILITY_RUNTIME_BOOTSTRAP_SELF_IDENTITY)
            .await
            .is_none(),
        "runtime admin must not leave a bare LocalRuntime dispatch key"
    );
    let installed_options = rt
        .ability_options(&install.runtime_key)
        .await
        .expect("runtime admin canonical key is registered");
    assert!(
        installed_options
            .proof_for_mode(axon_sdk::invocation::CallMode::Rpc)
            .is_some_and(|proof| proof.is_bound()),
        "runtime admin registration must carry descriptor proof"
    );

    let key = SigningKey::from_bytes(&[0x55; 32]);
    let args = serde_json::json!({
        "tenant_id": "tenant-a",
        "node_id": "node-a",
        "owner_id": "node-a",
        "public_key_b64": BASE64_STANDARD.encode(key.verifying_key().to_bytes()),
    })
    .to_string();

    let resp = svc
        .invoke(signed_invoke_request(
            TEST_BOOTSTRAP_CALLER_URA,
            &hub_ura,
            TEST_BOOTSTRAP_CALLER_URA,
            federation_wrappers::ABILITY_RUNTIME_BOOTSTRAP_SELF_IDENTITY,
            &args,
            &test_bootstrap_caller_signing_key(),
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
    expect_canonical_in_band_failure(
        svc.invoke(invoke_request(ABILITY_FEDERATION_JOIN, "not-json"))
            .await,
        axon_sdk::invocation::ErrorCode::RequestPayloadInvalid,
        "malformed JSON must be rejected",
    );
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
        .insert(TEST_DAEMON_URA.to_string(), self_tx);
    publish_test_route(&svc, TEST_DAEMON_URA, "observe.health");

    let ability_ura = crate::core::ura::owner_ability_ura(TEST_DAEMON_URA, "observe.health")
        .expect("device ability URA");
    let selected_route = svc
        .target_gate()
        .route_resolver()
        .await
        .resolve_route(&ability_ura, "")
        .expect("resolver selects the self-hosted route");
    assert_eq!(selected_route.execution_host_ura, TEST_DAEMON_URA);

    let request = invoke_request("observe.health", "{}").into_inner();
    let err = svc
        .unary_dispatcher()
        .dispatch_remote_rpc_selected_route(&request, &selected_route, CallMode::Rpc)
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
    svc.directory.presence.insert_negotiated(
        WEDGED_DEVICE_URA.to_string(),
        wedged_tx,
        crate::daemon::invocation::bidi::state::presence::SessionContract {
            version: 1,
            claimant_boot_nonce: vec![7; 16],
        },
    );
    publish_test_route(&svc, WEDGED_DEVICE_URA, "observe.health");

    let ability_ura = crate::core::ura::owner_ability_ura(WEDGED_DEVICE_URA, "observe.health")
        .expect("device ability URA");
    let selected_route = svc
        .target_gate()
        .route_resolver()
        .await
        .resolve_route(&ability_ura, "")
        .expect("resolver selects the wedged-device route");
    assert_eq!(selected_route.execution_host_ura, WEDGED_DEVICE_URA);

    let request = invoke_request_for_callee(WEDGED_DEVICE_URA, "observe.health", "{}").into_inner();
    let err = svc
        .unary_dispatcher()
        .dispatch_remote_rpc_selected_route(&request, &selected_route, CallMode::Rpc)
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

#[tokio::test]
async fn dispatch_remote_rpc_rejects_missing_signed_descriptor_ref() {
    const REMOTE_DEVICE_URA: &str = "easynet:///r/test-realm/device/remote-device";

    let pending = Arc::new(PendingDispatchMap::new());
    let svc = make_service().with_pending(Arc::clone(&pending));
    let (remote_tx, mut remote_rx) = mpsc::channel(8);
    svc.directory
        .presence
        .insert(REMOTE_DEVICE_URA.to_string(), remote_tx);
    publish_test_route(&svc, REMOTE_DEVICE_URA, "observe.health");

    let ability_ura = crate::core::ura::owner_ability_ura(REMOTE_DEVICE_URA, "observe.health")
        .expect("remote device ability URA");
    let selected_route = svc
        .target_gate()
        .route_resolver()
        .await
        .resolve_route(&ability_ura, "")
        .expect("resolver selects the remote-device route");

    let mut request =
        invoke_request_for_callee(REMOTE_DEVICE_URA, "observe.health", "{}").into_inner();
    request.target = None;

    let err = svc
        .unary_dispatcher()
        .dispatch_remote_rpc_selected_route(&request, &selected_route, CallMode::Rpc)
        .await
        .expect_err("missing typed descriptor target must fail before carrier dispatch");
    assert_eq!(err.code(), tonic::Code::InvalidArgument);
    assert!(
        err.message()
            .contains("invocation requires InvocationTarget.typed_target"),
        "expected canonical typed-target denial, got: {}",
        err.message()
    );
    assert!(
        remote_rx.try_recv().is_err(),
        "missing descriptor-bound signed target must not be forwarded"
    );
    assert_eq!(pending.outstanding(), 0);
}

#[tokio::test]
async fn dispatch_remote_rpc_carrier_v1_preserves_signed_canonical_material() {
    use ed25519_dalek::Verifier as _;

    const REMOTE_DEVICE_URA: &str = "easynet:///r/test-realm/device/remote-device";

    let pending = Arc::new(PendingDispatchMap::new());
    let svc = make_service().with_pending(Arc::clone(&pending));
    let (remote_tx, mut remote_rx) = mpsc::channel(8);
    svc.directory.presence.insert_negotiated(
        REMOTE_DEVICE_URA.to_string(),
        remote_tx,
        crate::daemon::invocation::bidi::state::presence::SessionContract {
            version: 1,
            claimant_boot_nonce: vec![9; 16],
        },
    );
    publish_test_route(&svc, REMOTE_DEVICE_URA, "shell.run");

    let ability_ura = crate::core::ura::owner_ability_ura(REMOTE_DEVICE_URA, "shell.run")
        .expect("remote device ability URA");
    let selected_route = svc
        .target_gate()
        .route_resolver()
        .await
        .resolve_route(&ability_ura, "")
        .expect("resolver selects the remote-device route");
    let descriptor_ref = test_descriptor_ref(REMOTE_DEVICE_URA, "shell.run");
    let request =
        invoke_request_for_callee(REMOTE_DEVICE_URA, "shell.run", r#"{"command":"hostname"}"#)
            .into_inner();

    let dispatcher = svc.unary_dispatcher();
    let dispatch_task = tokio::spawn(async move {
        dispatcher
            .dispatch_remote_rpc_selected_route(&request, &selected_route, CallMode::Rpc)
            .await
    });
    let frame = remote_rx
        .recv()
        .await
        .expect("carrier frame delivered to v1 presence target")
        .expect("presence dispatch frame ok")
        .frame;
    dispatch_task.abort();

    let call = match frame.payload {
        Some(axon_sdk::pb::axon::v1::invoke_bidi_down::Payload::DispatchCall(call)) => call,
        other => panic!("expected carrier-v1 DispatchCall, got {other:?}"),
    };
    let request = call.request.expect("carrier-v1 request");
    assert_eq!(invocation_function_name(&request), "shell.run");
    assert_eq!(
        crate::daemon::invocation::dispatch::invocation_wire::descriptor_ref_from_invocation_target(
            "test forwarded unary",
            REMOTE_DEVICE_URA,
            request.target.as_ref(),
        )
        .unwrap(),
        descriptor_ref
    );
    let envelope = request.envelope.expect("carrier-v1 envelope");
    let signature = envelope
        .caller_signature
        .as_ref()
        .expect("caller signature preserved")
        .signature
        .clone();
    let descriptor_bound =
        crate::daemon::axon_bridge::wire_descriptor::descriptor_bound_from_wire_parts(
            envelope,
            descriptor_ref,
            &request.arguments,
        )
        .expect("descriptor-bound forwarded carrier");
    let signature = ed25519_dalek::Signature::from_slice(&signature).expect("ed25519 signature");
    test_device_signing_key()
        .verifying_key()
        .verify(&descriptor_bound.envelope.canonical_bytes(), &signature)
        .expect("forwarded carrier material must verify against original signature");
}

#[tokio::test]
async fn dispatch_remote_rpc_rejects_signed_callee_rewrite() {
    const REMOTE_DEVICE_URA: &str = "easynet:///r/test-realm/device/remote-device";

    let pending = Arc::new(PendingDispatchMap::new());
    let svc = make_service().with_pending(Arc::clone(&pending));
    let (remote_tx, mut remote_rx) = mpsc::channel(8);
    svc.directory
        .presence
        .insert(REMOTE_DEVICE_URA.to_string(), remote_tx);
    publish_test_route(&svc, REMOTE_DEVICE_URA, "observe.health");

    let ability_ura = crate::core::ura::owner_ability_ura(REMOTE_DEVICE_URA, "observe.health")
        .expect("remote device ability URA");
    let selected_route = svc
        .target_gate()
        .route_resolver()
        .await
        .resolve_route(&ability_ura, "")
        .expect("resolver selects the remote-device route");
    assert_eq!(selected_route.callee_ura, REMOTE_DEVICE_URA);

    // invoke_request signs TEST_DAEMON_URA as callee. The remote route selects
    // REMOTE_DEVICE_URA. Dispatch must reject locally instead of rewriting the
    // signed envelope and causing CALLER_SIGNATURE_INVALID on the device.
    let request = invoke_request("observe.health", "{}").into_inner();
    let err = svc
        .unary_dispatcher()
        .dispatch_remote_rpc_selected_route(&request, &selected_route, CallMode::Rpc)
        .await
        .expect_err("signed callee mismatch must fail before carrier dispatch");
    assert_eq!(err.code(), tonic::Code::InvalidArgument);
    assert!(
        err.message().contains("SIGNED_ENVELOPE_ROUTE_MUTATION"),
        "expected RFC-014 route mutation denial, got: {}",
        err.message()
    );
    assert!(
        remote_rx.try_recv().is_err(),
        "mismatched signed envelope must not be forwarded"
    );
    assert_eq!(pending.outstanding(), 0);
}
