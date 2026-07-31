use super::*;

use std::sync::atomic::{AtomicUsize, Ordering};

use axon_sdk::invocation::{
    make_ability, AbilityCallModes, AbilityOptions, AxonError, CallMode as AxonCallMode,
};

struct GeometryFixture {
    ability: &'static str,
    mode: crate::daemon::ability::CallMode,
    descriptor_ref: String,
    mutations: Arc<AtomicUsize>,
}

async fn generic_geometry_fixture() -> (DaemonInvocationService, Vec<GeometryFixture>) {
    let trust = SharedTrustAnchor::new(Arc::new(test_trust_anchor_without_principal_owners()));
    let runtime_assembly = test_runtime_assembly(trust.clone());
    let runtime = runtime_assembly.runtime();
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
                crate::core::ura::owner_ability_ura(TEST_DAEMON_URA, ability)
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
        publish_test_route_with_mode(&service, TEST_DAEMON_URA, ability, mode);
        sync_runtime_proof_from_catalog(&service, TEST_DAEMON_URA, ability, mode).await;
        fixtures.push(GeometryFixture {
            ability,
            mode,
            descriptor_ref: catalog_test_descriptor_ref(
                service
                    .directory
                    .local_ability_catalog
                    .as_ref()
                    .expect("RF7 test catalog"),
                TEST_DAEMON_URA,
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
        TEST_DAEMON_URA,
        TEST_DAEMON_URA,
        TEST_DAEMON_URA,
        geometry.descriptor_ref.clone(),
        &payload,
        &test_device_signing_key(),
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
async fn canonical_signature_and_runtime_admission_precede_handler_mutation_for_all_geometries() {
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

        let policy_error = invoke_staged_wire(&service, geometry, external_wire(geometry, false))
            .await
            .expect_err("daemon runtime admission must reject unresolved owner authority");
        assert_eq!(
            policy_error.reason, "ABILITY_FORBIDDEN",
            "{} policy rejection must preserve the canonical outer taxonomy",
            geometry.ability
        );
        assert!(
            policy_error.message.contains("POLICY_DENIED")
                && policy_error.message.contains("OWNER_UNRESOLVED"),
            "{} policy detail must come from the provider-backed admission seam: {policy_error}",
            geometry.ability
        );
        assert_eq!(
            geometry.mutations.load(Ordering::SeqCst),
            0,
            "{} runtime admission must run before handler mutation",
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
            TEST_DAEMON_URA,
            TEST_DAEMON_URA,
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
