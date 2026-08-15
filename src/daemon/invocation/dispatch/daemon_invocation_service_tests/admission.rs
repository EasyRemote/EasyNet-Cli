use super::*;

use std::sync::atomic::{AtomicUsize, Ordering};

use axon_sdk::invocation::{
    make_ability, AbilityCallModes, AbilityOptions, AxonError, CallMode as AxonCallMode,
};

struct GeometryFixture {
    owner_ura: String,
    ability: &'static str,
    mode: crate::daemon::ability::CallMode,
    descriptor_ref: String,
    mutations: Arc<AtomicUsize>,
}

async fn generic_geometry_fixture() -> (DaemonInvocationService, Vec<GeometryFixture>) {
    let trust = SharedTrustAnchor::new(Arc::new(test_trust_anchor_without_principal_owners()));
    let runtime_assembly = test_runtime_assembly(trust.clone());
    let runtime = runtime_assembly.runtime();
    let owner_ura = test_dispatch_system_agent_ura();
    let geometries = [
        (
            "test.rf7.policy_unary",
            crate::daemon::ability::CallMode::Rpc,
        ),
        (
            "test.rf7.policy_stream",
            crate::daemon::ability::CallMode::Stream,
        ),
        (
            "test.rf7.policy_bidi",
            crate::daemon::ability::CallMode::Bidi,
        ),
    ];
    let mut counters = Vec::with_capacity(geometries.len());

    for (ability, mode) in geometries {
        let mutations = Arc::new(AtomicUsize::new(0));
        let handler_mutations = Arc::clone(&mutations);
        let options = match mode {
            crate::daemon::ability::CallMode::Rpc => AbilityOptions::default()
                .with_modes(AbilityCallModes::RPC)
                .with_descriptor_proof(
                    crate::daemon::ability::DEFAULT_ABILITY_DESCRIPTOR_VERSION,
                    "invoke",
                    [0x31; 32],
                    [0x32; 32],
                    [0x33; 32],
                ),
            crate::daemon::ability::CallMode::Stream => AbilityOptions::streaming()
                .with_mode_descriptor_proof(
                    AxonCallMode::Stream,
                    crate::daemon::ability::DEFAULT_ABILITY_DESCRIPTOR_VERSION,
                    "invoke",
                    [0x31; 32],
                    [0x32; 32],
                    [0x33; 32],
                ),
            crate::daemon::ability::CallMode::Bidi => AbilityOptions::bidi()
                .with_mode_descriptor_proof(
                    AxonCallMode::Bidi,
                    crate::daemon::ability::DEFAULT_ABILITY_DESCRIPTOR_VERSION,
                    "invoke",
                    [0x31; 32],
                    [0x32; 32],
                    [0x33; 32],
                ),
        };
        runtime
            .register_ability_with_options(
                crate::core::ura::owner_ability_ura(&owner_ura, ability)
                    .expect("generic RF7 test ability URA"),
                make_ability(move |_context| {
                    let handler_mutations = Arc::clone(&handler_mutations);
                    async move {
                        handler_mutations.fetch_add(1, Ordering::SeqCst);
                        Ok(Vec::new())
                    }
                }),
                options,
            )
            .await
            .expect("register generic RF7 test ability");
        counters.push((ability, mode, mutations));
    }

    let service = register_test_daemon_routes(
        make_unregistered_service_for_route_owner_and_runtime_trust(
            TEST_DAEMON_URA,
            runtime_assembly,
            trust,
        ),
        TEST_DAEMON_URA,
    );
    let mut fixtures = Vec::with_capacity(counters.len());
    for (ability, mode, mutations) in counters {
        publish_test_route_with_mode(&service, &owner_ura, ability, mode);
        sync_runtime_proof_from_catalog(&service, &owner_ura, ability, mode).await;
        fixtures.push(GeometryFixture {
            owner_ura: owner_ura.clone(),
            ability,
            mode,
            descriptor_ref: catalog_test_descriptor_ref(
                service
                    .directory
                    .local_ability_catalog
                    .as_ref()
                    .expect("RF7 test catalog"),
                &owner_ura,
                ability,
                mode,
            ),
            mutations,
        });
    }
    (service, fixtures)
}

fn axon_mode(mode: crate::daemon::ability::CallMode) -> AxonCallMode {
    match mode {
        crate::daemon::ability::CallMode::Rpc => AxonCallMode::Rpc,
        crate::daemon::ability::CallMode::Stream => AxonCallMode::Stream,
        crate::daemon::ability::CallMode::Bidi => AxonCallMode::Bidi,
    }
}

async fn invoke_staged_wire(
    service: &DaemonInvocationService,
    geometry: &GeometryFixture,
    wire: crate::daemon::axon_bridge::descriptor_bound_dispatch::WireDispatch,
) -> Result<(), AxonError> {
    let runtime = service
        .runtime
        .local_runtime()
        .expect("RF7 test service LocalRuntime");
    let lease = service
        .runtime
        .stage_runtime_admission(
            service.admission_plane.verifier_ref(),
            &wire,
            geometry.ability,
            axon_mode(geometry.mode),
        )
        .map_err(|status| AxonError::internal(status.to_string()))?;
    let result = match geometry.mode {
        crate::daemon::ability::CallMode::Rpc => {
            let outcome =
                crate::daemon::axon_bridge::descriptor_bound_dispatch::dispatch_rpc_admitted(
                    &runtime,
                    wire,
                    &service.runtime.cancellations,
                )
                .await;
            match outcome.error {
                Some(error) => Err(error),
                None => Ok(()),
            }
        }
        crate::daemon::ability::CallMode::Stream => {
            let handle =
                crate::daemon::axon_bridge::descriptor_bound_dispatch::open_stream_admitted(
                    &runtime, wire,
                )
                .await?;
            handle.finalized().await.map(|_| ())
        }
        crate::daemon::ability::CallMode::Bidi => {
            let handle =
                crate::daemon::axon_bridge::descriptor_bound_dispatch::open_bidi_external_signed(
                    &runtime, wire,
                )
                .await?;
            handle.finalized().await.map(|_| ())
        }
    };
    if result.is_ok() {
        lease
            .commit()
            .map_err(|status| AxonError::internal(status.to_string()))?;
    }
    result
}

fn external_wire(
    geometry: &GeometryFixture,
    corrupt_signature: bool,
) -> crate::daemon::axon_bridge::descriptor_bound_dispatch::WireDispatch {
    let payload = b"{}".to_vec();
    let mut envelope = signed_test_envelope_with_descriptor_ref(
        TEST_DISCOVER_USER_URA,
        &geometry.owner_ura,
        &geometry.owner_ura,
        geometry.descriptor_ref.clone(),
        &payload,
        &test_discover_user_signing_key(),
    );
    if corrupt_signature {
        envelope
            .caller_signature
            .as_mut()
            .expect("signed RF7 test envelope")
            .signature[0] ^= 0x80;
    }
    crate::daemon::axon_bridge::descriptor_bound_dispatch::external_signed_from_wire_parts(
        envelope,
        geometry.descriptor_ref.clone(),
        payload,
        HashMap::new(),
    )
    .expect("external RF7 wire dispatch")
}

fn local_system_wire(
    geometry: &GeometryFixture,
    envelope: Envelope,
) -> crate::daemon::axon_bridge::descriptor_bound_dispatch::WireDispatch {
    crate::daemon::axon_bridge::descriptor_bound_dispatch::local_system_from_wire_parts(
        envelope,
        geometry.descriptor_ref.clone(),
        b"{}".to_vec(),
        HashMap::new(),
    )
    .expect("local-system RF7 wire dispatch")
}

#[tokio::test]
async fn canonical_signature_and_authority_admission_precede_handler_mutation_for_all_geometries() {
    let (service, geometries) = generic_geometry_fixture().await;

    for geometry in &geometries {
        let signature_error = invoke_staged_wire(&service, geometry, external_wire(geometry, true))
            .await
            .expect_err("malformed signature must fail in canonical runtime admission");
        assert!(
            signature_error.reason.contains("CALLER_SIGNATURE_INVALID"),
            "{} malformed signature must retain Axon's reason: {signature_error}",
            geometry.ability
        );
        assert_eq!(
            geometry.mutations.load(Ordering::SeqCst),
            0,
            "{} handler must not run before signature admission",
            geometry.ability
        );

        let authority_error =
            invoke_staged_wire(&service, geometry, external_wire(geometry, false))
                .await
                .expect_err("cross-subject User invocation must carry explicit authority");
        assert_eq!(
            authority_error.reason, "AUTHORITY_REQUIRED",
            "{} authority rejection must preserve the canonical outer taxonomy",
            geometry.ability
        );
        assert!(
            authority_error.message.contains("AUTHORITY_DENIED")
                && authority_error.message.contains("AUTHORITY_REQUIRED"),
            "{} authority detail must come from the fail-closed admission seam: {authority_error}",
            geometry.ability
        );
        assert_eq!(
            geometry.mutations.load(Ordering::SeqCst),
            0,
            "{} authority admission must run before handler mutation",
            geometry.ability
        );
    }
}

#[tokio::test]
async fn canonical_replay_admission_executes_each_geometry_exactly_once() {
    let (service, geometries) = generic_geometry_fixture().await;

    for (index, geometry) in geometries.iter().enumerate() {
        let envelope = ProtoEnvelope::from_target(
            crate::daemon::identity::local_invocation::LOCAL_SYSTEM_AGENT_URA,
            &geometry.owner_ura,
            &geometry.owner_ura,
            InvocationDerivationPolicy::Explicit {
                invocation_nonce: [0xD0 + index as u8; 16],
                causal_context: axon_sdk::invocation::CausalContext::None,
            },
        )
        .expect("valid local-system RF7 envelope")
        .into_inner(&geometry.descriptor_ref, b"{}")
        .expect("complete local-system RF7 tuple");

        invoke_staged_wire(
            &service,
            geometry,
            local_system_wire(geometry, envelope.clone()),
        )
        .await
        .expect("first canonical invocation is admitted");
        assert_eq!(
            geometry.mutations.load(Ordering::SeqCst),
            1,
            "{} first invocation must execute exactly once",
            geometry.ability
        );

        let replay_error =
            invoke_staged_wire(&service, geometry, local_system_wire(geometry, envelope))
                .await
                .expect_err("replayed canonical nonce must be rejected");
        assert!(
            replay_error.reason.contains("NONCE_REPLAY"),
            "{} replay must retain Axon's canonical reason: {replay_error}",
            geometry.ability
        );
        assert_eq!(
            geometry.mutations.load(Ordering::SeqCst),
            1,
            "{} replay must not execute the handler twice",
            geometry.ability
        );
    }
}

struct HostedAgentCarrierKeyResolver {
    hosted_agent_ura: String,
    hosted_agent_key: ed25519_dalek::VerifyingKey,
    fallback: TestRuntimeKeyResolver,
}

impl axon_sdk::invocation::KeyResolver for HostedAgentCarrierKeyResolver {
    fn resolve(
        &self,
        agent_ura: &str,
    ) -> Result<ed25519_dalek::VerifyingKey, axon_sdk::invocation::AxonError> {
        if agent_ura == self.hosted_agent_ura {
            return Ok(self.hosted_agent_key);
        }
        self.fallback.resolve(agent_ura)
    }

    fn resolve_all(
        &self,
        agent_ura: &str,
    ) -> Result<Vec<ed25519_dalek::VerifyingKey>, axon_sdk::invocation::AxonError> {
        if agent_ura == self.hosted_agent_ura {
            return Ok(vec![self.hosted_agent_key]);
        }
        self.fallback.resolve_all(agent_ura)
    }
}

struct HostedAgentCarrierVerificationKey {
    hosted_agent_ura: String,
    hosted_agent_key: ed25519_dalek::VerifyingKey,
}

impl crate::daemon::identity::receipt_signing::InvocationVerificationKeyProvider
    for HostedAgentCarrierVerificationKey
{
    fn resolve_invocation_verifying_key(
        &self,
        caller_ura: &str,
    ) -> Result<Option<ed25519_dalek::VerifyingKey>, axon_sdk::invocation::AxonError> {
        Ok((caller_ura == self.hosted_agent_ura).then_some(self.hosted_agent_key))
    }
}

struct HostedAgentCarrierGeometry {
    public_name: &'static str,
    mode: crate::daemon::ability::CallMode,
    descriptor_ref: String,
    starts: Arc<AtomicUsize>,
}

struct HostedAgentCarrierFixture {
    service: DaemonInvocationService,
    dispatcher:
        crate::daemon::invocation::dispatch::local_session_dispatcher::LocalAxonSessionDispatcher,
    agent_ura: String,
    subject_ura: String,
    signing_key: ed25519_dalek::SigningKey,
    assignment:
        crate::daemon::federation::hosted_agent_publication::HostedAgentGenerationAssignment,
    desired_catalog_epoch: u64,
    geometries: Vec<HostedAgentCarrierGeometry>,
}

fn hosted_agent_carrier_manifest(
    public_name: &str,
) -> crate::daemon::ability::manifest::AbilityManifest {
    crate::daemon::ability::manifest::AbilityManifest::new(
        public_name.rsplit('.').next().unwrap_or(public_name),
        "Hosted Agent carrier readiness regression",
        serde_json::json!({"type": "object"}),
    )
    .and_then(|manifest| manifest.with_admission_action("invoke"))
    .expect("hosted Agent carrier manifest")
}

async fn hosted_agent_carrier_fixture() -> HostedAgentCarrierFixture {
    use crate::daemon::ability::dispatch::{
        AbilityAuthorityContext, AxonAbilityCatalog, BidiSource, OwnerKind, StreamSource,
    };

    let agent_id = "readiness";
    let agent_ura = crate::core::ura::agent_ura("test-realm", "test-user", agent_id);
    let subject_ura =
        crate::core::ura::resource_dot_ura("test-realm", "user.test-user", "agent/readiness");
    let signing_key = ed25519_dalek::SigningKey::from_bytes(&[0x6a; 32]);
    let mut anchor = test_trust_anchor();
    anchor
        .upsert_principal_owner(TrustedPrincipalOwner {
            principal_ura: agent_ura.clone(),
            owner_user_id: "test-user".to_string(),
            owner_ura: TEST_DISCOVER_USER_URA.to_string(),
            added_at_unix_ms: 1_700_000_000_010,
        })
        .expect("hosted Agent owner projection");
    let trust = SharedTrustAnchor::new(Arc::new(anchor));
    let runtime_assembly =
        crate::daemon::axon_bridge::runtime_factory::build_test_daemon_runtime_assembly(
            Arc::new(HostedAgentCarrierKeyResolver {
                hosted_agent_ura: agent_ura.clone(),
                hosted_agent_key: signing_key.verifying_key(),
                fallback: TestRuntimeKeyResolver::new(trust.clone()),
            }),
            crate::daemon::axon_bridge::runtime_factory::isolated_test_runtime_persistence(
                "hosted-agent-carrier-readiness",
            ),
            None,
        );
    let mut catalog = AxonAbilityCatalog::new_with_runtime_and_authority_context(
        runtime_assembly.runtime(),
        AbilityAuthorityContext::for_device_authority_root_with_hosted_agents(
            TEST_DAEMON_URA,
            [agent_ura.clone()],
        )
        .expect("hosted Agent authority context"),
    );
    let owner = OwnerKind::Agent(agent_id.to_string());
    let rpc_starts = Arc::new(AtomicUsize::new(0));
    let stream_starts = Arc::new(AtomicUsize::new(0));
    let bidi_starts = Arc::new(AtomicUsize::new(0));

    let handler_starts = Arc::clone(&rpc_starts);
    catalog.register_rpc_with_spec(
        "readiness.rpc",
        owner.clone(),
        hosted_agent_carrier_manifest("rpc"),
        Arc::new(move |_| {
            handler_starts.fetch_add(1, Ordering::SeqCst);
            Ok(serde_json::json!({"ok": true}))
        }),
    );
    let handler_starts = Arc::clone(&stream_starts);
    catalog.register_stream_with_spec(
        "readiness.stream",
        owner.clone(),
        hosted_agent_carrier_manifest("stream"),
        Arc::new(move |_| {
            handler_starts.fetch_add(1, Ordering::SeqCst);
            Ok(StreamSource::Snapshot(Vec::new()))
        }),
    );
    let handler_starts = Arc::clone(&bidi_starts);
    catalog.register_bidi_with_spec(
        "readiness.fs.transfer",
        owner,
        hosted_agent_carrier_manifest("fs.transfer"),
        Arc::new(move |_| {
            handler_starts.fetch_add(1, Ordering::SeqCst);
            let (to_client, _handler_input) = tokio::sync::mpsc::channel(1);
            let (_handler_output, from_client) = tokio::sync::mpsc::channel(1);
            Ok(BidiSource {
                to_client,
                from_client,
            })
        }),
    );
    let catalog = Arc::new(catalog);
    let geometries = [
        (
            "rpc",
            "readiness.rpc",
            crate::daemon::ability::CallMode::Rpc,
            rpc_starts,
        ),
        (
            "stream",
            "readiness.stream",
            crate::daemon::ability::CallMode::Stream,
            stream_starts,
        ),
        (
            "fs.transfer",
            "readiness.fs.transfer",
            crate::daemon::ability::CallMode::Bidi,
            bidi_starts,
        ),
    ]
    .into_iter()
    .map(
        |(public_name, registry_name, mode, starts)| HostedAgentCarrierGeometry {
            public_name,
            mode,
            descriptor_ref: catalog_test_descriptor_ref(
                catalog.as_ref(),
                &agent_ura,
                registry_name,
                mode,
            ),
            starts,
        },
    )
    .collect::<Vec<_>>();
    let stores = Arc::new(
        crate::daemon::persistence::access_control::AccessControlStoreRegistry::ephemeral(),
    );
    for geometry in &geometries {
        grant_child_access_for_test(
            stores.as_ref(),
            ChildAccessGrantInput {
                owner_user_ura: TEST_DISCOVER_USER_URA,
                principal_kind: PrincipalKind::Agent,
                principal_ura: &agent_ura,
                token_class: None,
                callee_ura: &agent_ura,
                subject_ura: &subject_ura,
                ability_ura: &crate::core::ura::owner_ability_ura(&agent_ura, geometry.public_name)
                    .expect("hosted Agent public ability URA"),
                action: AccessAction::Invoke,
            },
        );
    }
    let verification_keys = Arc::new(HostedAgentCarrierVerificationKey {
        hosted_agent_ura: agent_ura.clone(),
        hosted_agent_key: signing_key.verifying_key(),
    });
    let admission =
        AdmissionFacade::with_trust_anchor_cell(trust, Some(TEST_DAEMON_URA.to_string()))
            .with_transport_boundary(AdmissionTransportBoundary::LocalOnlyIpc)
            .with_invocation_verification_keys(verification_keys)
            .with_access_control_stores(stores)
            .with_ability_catalog(Arc::clone(&catalog));
    let service =
        DaemonInvocationService::new(Arc::new(PresenceRegistry::new()), admission.clone())
            .with_local_ability_catalog(Arc::clone(&catalog))
            .with_daemon_runtime(runtime_assembly.clone())
            .with_invocation_attempt_ledger(test_attempt_ledger());
    for geometry in &geometries {
        publish_test_route_hosted_by(&service, &agent_ura, geometry.public_name, TEST_DAEMON_URA);
    }
    let dispatcher = crate::daemon::invocation::dispatch::local_session_dispatcher::LocalAxonSessionDispatcher::new(
        Default::default(),
    )
    .with_runtime_admission(runtime_assembly, admission);
    let pending = crate::daemon::persistence::hosted_agent_publications::begin_registration(
        &agent_ura,
        TEST_DAEMON_URA,
        1,
    )
    .expect("begin hosted Agent carrier publication");
    let assignment =
        crate::daemon::federation::hosted_agent_publication::HostedAgentGenerationAssignment {
            agent_ura: agent_ura.clone(),
            host_device_ura: TEST_DAEMON_URA.to_string(),
            incarnation_id: pending.incarnation_id().clone(),
            generation: 1,
        };
    crate::daemon::persistence::hosted_agent_publications::bind_assignment(&assignment, 2)
        .expect("enter Assigned readiness state");

    HostedAgentCarrierFixture {
        service,
        dispatcher,
        agent_ura,
        subject_ura,
        signing_key,
        assignment,
        desired_catalog_epoch: pending.desired_catalog_epoch,
        geometries,
    }
}

#[tokio::test]
async fn local_source_path_reaches_authoritative_hosted_agent_readiness_error() {
    use axon_sdk::pb::axon::v1::{ErrorStage, SecurityClass};

    let _home = crate::cli::commands::test_support::HomeGuard::new();
    let fixture = hosted_agent_carrier_fixture().await;
    let geometry = fixture
        .geometries
        .iter()
        .find(|geometry| geometry.mode == crate::daemon::ability::CallMode::Rpc)
        .expect("hosted Agent RPC geometry");
    let arguments = b"{}".to_vec();
    let request = InvokeRequest {
        envelope: Some(signed_test_envelope_with_descriptor_ref(
            &fixture.agent_ura,
            &fixture.agent_ura,
            &fixture.subject_ura,
            geometry.descriptor_ref.clone(),
            &arguments,
            &fixture.signing_key,
        )),
        target: Some(
            wire_invocation_target(&geometry.descriptor_ref, geometry.public_name)
                .expect("hosted Agent source target"),
        ),
        arguments,
        ..InvokeRequest::default()
    };

    let response = fixture
        .service
        .invoke(Request::new(request))
        .await
        .expect("source transport returns an in-band canonical failure")
        .into_inner();
    let failure = response
        .error
        .expect("Assigned hosted Agent source call is rejected");

    assert_eq!(
        failure.code,
        axon_sdk::invocation::ErrorCode::AbilityDisabled.as_str()
    );
    assert_eq!(failure.stage, ErrorStage::AbilityPolicy as i32);
    assert_eq!(failure.security_class, SecurityClass::Authorization as i32);
    assert!(failure.message.contains("HOSTED_AGENT_NOT_PUBLISHED"));
    assert_eq!(geometry.starts.load(Ordering::SeqCst), 0);
}

fn hosted_agent_carrier_call(
    fixture: &HostedAgentCarrierFixture,
    geometry: &HostedAgentCarrierGeometry,
    call_id: u64,
) -> InvokeBidiDown {
    let arguments = b"{}".to_vec();
    InvokeBidiDown {
        payload: Some(DownPayload::DispatchCall(
            axon_sdk::pb::axon::v1::DispatchCall {
                call_id,
                request: Some(InvokeRequest {
                    envelope: Some(signed_test_envelope_with_descriptor_ref(
                        &fixture.agent_ura,
                        &fixture.agent_ura,
                        &fixture.subject_ura,
                        geometry.descriptor_ref.clone(),
                        &arguments,
                        &fixture.signing_key,
                    )),
                    target: Some(
                        wire_invocation_target(&geometry.descriptor_ref, geometry.public_name)
                            .expect("hosted Agent carrier target"),
                    ),
                    arguments,
                    ..InvokeRequest::default()
                }),
                call_mode: crate::daemon::invocation::bidi::session_wire::canonical_call_mode_wire(
                    axon_mode(geometry.mode),
                ),
            },
        )),
        ..InvokeBidiDown::default()
    }
}

async fn dispatch_hosted_agent_carrier_call(
    fixture: &HostedAgentCarrierFixture,
    geometry: &HostedAgentCarrierGeometry,
    call_id: u64,
) -> axon_sdk::pb::axon::v1::DispatchResult {
    use crate::daemon::invocation::bidi::session_initiator::{
        SessionFrameDispatcher, SessionUpSender,
    };

    let (tx, mut rx) = tokio::sync::mpsc::channel(8);
    let outbound = SessionUpSender::new(tx);
    outbound.set_negotiated_contract(
        crate::daemon::invocation::bidi::state::presence::CANONICAL_SESSION_CARRIER_VERSION,
    );
    fixture.dispatcher.session_started(outbound.scope_id());
    fixture
        .dispatcher
        .handle_down(
            hosted_agent_carrier_call(fixture, geometry, call_id),
            &outbound,
        )
        .await
        .expect("carrier ingress accepts canonical dispatch");
    let reply = tokio::time::timeout(std::time::Duration::from_secs(3), rx.recv())
        .await
        .expect("carrier reply does not time out")
        .expect("carrier reply exists");
    fixture.dispatcher.session_ended(outbound.scope_id());
    match reply.payload {
        Some(UpPayload::DispatchResult(result)) => result,
        other => panic!("expected carrier DispatchResult, got {other:?}"),
    }
}

async fn assert_hosted_agent_carrier_state(
    fixture: &HostedAgentCarrierFixture,
    state: &str,
    published: bool,
    next_call_id: &mut u64,
) {
    use axon_sdk::pb::axon::v1::{ErrorStage, SecurityClass};

    for geometry in &fixture.geometries {
        let starts_before = geometry.starts.load(Ordering::SeqCst);
        let result = dispatch_hosted_agent_carrier_call(fixture, geometry, *next_call_id).await;
        *next_call_id += 1;
        if published {
            assert!(
                result.failure.is_none(),
                "{state} {:?} carrier dispatch must be admitted: {:?}",
                geometry.mode,
                result.failure
            );
            assert_eq!(
                geometry.starts.load(Ordering::SeqCst),
                starts_before + 1,
                "{state} {:?} must start its handler exactly once",
                geometry.mode
            );
        } else {
            let failure = result.failure.expect("readiness rejection is typed");
            assert_eq!(
                failure.code,
                axon_sdk::invocation::ErrorCode::AbilityDisabled.as_str(),
                "{state} {:?} must retain canonical readiness code",
                geometry.mode
            );
            assert_eq!(
                failure.stage,
                ErrorStage::AbilityPolicy as i32,
                "{state} {:?} must retain canonical readiness stage",
                geometry.mode
            );
            assert_eq!(
                failure.security_class,
                SecurityClass::Authorization as i32,
                "{state} {:?} must retain authorization classification",
                geometry.mode
            );
            assert!(
                failure.message.contains("HOSTED_AGENT_NOT_PUBLISHED"),
                "{state} {:?} must retain readiness detail: {failure:?}",
                geometry.mode
            );
            assert_eq!(
                geometry.starts.load(Ordering::SeqCst),
                starts_before,
                "{state} {:?} must reject before handler start",
                geometry.mode
            );
        }
    }
}

#[tokio::test]
async fn destination_carrier_enforces_hosted_agent_publication_readiness_for_every_geometry() {
    let _home = crate::cli::commands::test_support::HomeGuard::new();
    let fixture = hosted_agent_carrier_fixture().await;
    let mut next_call_id = 1;

    assert_hosted_agent_carrier_state(&fixture, "Assigned", false, &mut next_call_id).await;
    crate::daemon::persistence::hosted_agent_publications::stage_projection(
        &fixture.assignment,
        fixture.desired_catalog_epoch,
        1,
        "sha256:carrier-readiness",
        3,
    )
    .expect("enter Publishing readiness state");
    assert_hosted_agent_carrier_state(&fixture, "Publishing", false, &mut next_call_id).await;
    crate::daemon::persistence::hosted_agent_publications::mark_published(
        &fixture.assignment,
        fixture.desired_catalog_epoch,
        1,
        "sha256:carrier-readiness",
        4,
    )
    .expect("enter Published readiness state");
    assert_hosted_agent_carrier_state(&fixture, "Published", true, &mut next_call_id).await;
    crate::daemon::persistence::hosted_agent_publications::retire(
        &fixture.agent_ura,
        &fixture.assignment.incarnation_id,
        fixture.assignment.generation,
        5,
    )
    .expect("enter Retired readiness state");
    assert_hosted_agent_carrier_state(&fixture, "Retired", false, &mut next_call_id).await;
}
