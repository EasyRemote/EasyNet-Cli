// EasyNet CLI — PR-N2 commit 3/N: cross-realm signed admission e2e
// =================================================================
//
// File: tests/cross_realm_signed_admission_e2e.rs
// Description: In-process two-daemon test where daemon B admits a
//              signed envelope from a caller in realm-A's trust set
//              by dialling A's `federation.resolve_key` ability via
//              the FederatedKeyResolver wired into B's
//              AdmissionFacade.
//
// Why this test exists
// --------------------
// PR-N2 commit 1/N landed the FederatedKeyResolver + AdmissionFacade
// swap; commit 2/N landed the peer-side `federation.resolve_key`
// handler. Together they unblock cross-realm strict admission, but
// neither commit proves end-to-end:
//
//   "daemon B receives an envelope signed by a key it has never seen,
//    fetches the key from A, runs the §5.2 4-step verify, and admits."
//
// This integration test is that proof. The chain:
//
//   1. Test mints `device-A`'s Ed25519 keypair.
//   2. Daemon A boots with realm-a trust set = [device-A]; daemon A
//      serves `federation.resolve_key` from this anchor.
//   3. Daemon B boots with realm-b trust set = empty; daemon B's
//      AdmissionFacade is wired with a `FederatedKeyResolver` whose
//      federation client is an in-process forwarder into daemon A,
//      and whose `federated_peers = {realm-a → "in-process-A"}`.
//   4. Test signs an envelope for ability `self.echo` with caller =
//      device-A's URA, callee = daemon B's URA.
//   5. Test calls `daemon_b.invoke(signed_request)`.
//   6. Daemon B's admission gate runs strict path → FederatedKey-
//      Resolver:
//        a. local trust miss (device-A not in B's anchor)
//        b. caller_tenant `realm-a` ≠ self_realm `realm-b`,
//           federated_peers has `realm-a` → in-process-A → dial
//        c. cross-hub `federation.resolve_key` against in-process-A
//        d. A returns `{public_key_b64: <device-A's pubkey>}`
//        e. B builds VerifyingKey, admission's 4-step verify
//           accepts.
//   7. Daemon B dispatches `self.echo` (which is unimplemented in
//      this minimal test, but admission already passed — that's
//      what the test asserts).
//
// What this test does NOT exercise
// --------------------------------
// - Real TCP/TLS. The federation client is an in-process forwarder
//   that bypasses the network. The 2-daemon spawned-binary version
//   (real TLS handshake) is a follow-up; this test verifies the
//   admission + resolve_key correlation logic deterministically.
// - The PR-N1 forward_invoke unwrap path. The signed envelope is
//   delivered straight to daemon B's invoke endpoint, not through
//   `federation.forward_invoke`. Composing the two is PR-N3
//   territory (full discover + forward + signed admit chain).
//
// Author: Silan.Hu <silan.hu@u.nus.edu>
// Copyright (c) 2026 EasyNet. All rights reserved.

#![cfg(feature = "axon-pb")]

use std::sync::Arc;

use async_trait::async_trait;
use base64::{engine::general_purpose::STANDARD as BASE64_STANDARD, Engine as _};
use ed25519_dalek::SigningKey;
use sha2::Digest as _;

use easynet_axon::invocation::axiom::{
    sign_descriptor_bound_invocation, AgentIdentity as AxiomAgentIdentity, CausalContext,
    DescriptorBoundEnvelope, InvocationEnvelope, SubjectIdentity, UraProfile,
};
use easynet_axon::invocation::LocalRuntime;
use easynet_axon::pb::axon::v1::invocation_server::Invocation;
use easynet_axon::pb::axon::v1::{
    AgentIdentity as PbAgentIdentity, CallerSignature as PbCallerSignature, Envelope,
    InvokeRequest, InvokeResponse, SubjectIdentity as PbSubjectIdentity,
};
use easynet_cli::daemon::ability::DEFAULT_ABILITY_DESCRIPTOR_VERSION;
use easynet_cli::daemon::federation::client::{
    FederationClient, FederationClientError, HubEndpoint,
};
use easynet_cli::daemon::federation::peers::SharedFederatedPeers;
use easynet_cli::daemon::invocation::admission::admission_facade::AdmissionFacade;
use easynet_cli::daemon::invocation::bidi::state::presence::PresenceRegistry;
use easynet_cli::daemon::invocation::dispatch::daemon_invocation_service::DaemonInvocationService;
use easynet_cli::daemon::trust::anchor::{RealmTrustAnchor, TrustedAgent, TrustedAgentRole};

const REALM_B_HUB_SIGNING_SEED: [u8; 32] = [0xB0; 32];
const SIGNED_DESCRIPTOR_REF_METADATA_KEY: &str = "x-easynet-signed-descriptor-ref";

/// In-process federation client that forwards every `forward_invoke`
/// to a target `DaemonInvocationService`. Used so daemon B's
/// FederatedKeyResolver can dial daemon A without a real TLS
/// channel.
///
/// The test wires daemon A as the forwarding target and registers
/// the resulting client under `federated_peers["realm-a"] =
/// "in-process-A"`. The hub URA string is opaque to the resolver
/// (it just looks for the key); the forwarder ignores it because
/// there's only one peer in the test fixture.
///
/// The forwarder also stamps daemon A's loopback URA as the request
/// envelope's caller. Production cross-hub dialling sends a signed
/// envelope from one hub's identity to another (PR-N5 territory);
/// in-process here we use loopback bypass — daemon A's admission
/// recognises its own URA as `caller.ura` and skips the membership
/// gate, exactly the same fast-path operators see when a daemon
/// dispatches an ability against itself.
struct InProcessForwarder {
    peer: Arc<DaemonInvocationService>,
    peer_loopback_uri: String,
}

#[async_trait]
impl FederationClient for InProcessForwarder {
    async fn forward_invoke(
        &self,
        _target_hub_endpoint: &HubEndpoint,
        mut request: InvokeRequest,
    ) -> Result<InvokeResponse, FederationClientError> {
        // Stamp loopback: daemon A admits its own URA without
        // running the strict pipeline, so the resolve_key lookup
        // proceeds straight to the trust-anchor read.
        let loopback_envelope = Envelope {
            caller: Some(PbAgentIdentity {
                ura: self.peer_loopback_uri.clone(),
                profile: "easynet-strict-v2".to_string(),
            }),
            callee: Some(PbAgentIdentity {
                ura: self.peer_loopback_uri.clone(),
                profile: "easynet-strict-v2".to_string(),
            }),
            subject: Some(PbSubjectIdentity {
                ura: self.peer_loopback_uri.clone(),
                profile: "easynet-strict-v2".to_string(),
            }),
            ..Envelope::default()
        };
        request.envelope = Some(loopback_envelope);
        let response = self
            .peer
            .invoke(tonic::Request::new(request))
            .await
            .map_err(|status| FederationClientError::InnerInvokeFailed {
                endpoint: "in-process-A".to_string(),
                status: format!("code={:?} message={}", status.code(), status.message()),
            })?;
        Ok(response.into_inner())
    }
}

/// Mirrors the production descriptor-bound signing path so admission
/// verifies exactly the bytes the test signed.
fn signed_request(
    caller_ura: &str,
    callee_ura: &str,
    ability: &str,
    args: &[u8],
    signing_key: &SigningKey,
    nonce: [u8; 16],
) -> InvokeRequest {
    let subject_ura = easynet_cli::core::ura::owner_ability_ura(callee_ura, ability)
        .expect("callee-owned descriptor ability subject");
    let subject = SubjectIdentity::new(&subject_ura, UraProfile::EasynetStrictV2);
    let ability_ref = format!(
        "{}@{}",
        easynet_cli::core::ura::owner_ability_ura(callee_ura, ability)
            .expect("callee-owned descriptor ability"),
        DEFAULT_ABILITY_DESCRIPTOR_VERSION
    );
    let axiom_env = InvocationEnvelope {
        caller: AxiomAgentIdentity::new(caller_ura, UraProfile::EasynetStrictV2),
        callee: AxiomAgentIdentity::new(callee_ura, UraProfile::EasynetStrictV2),
        subject: subject.clone(),
        ability: ability_ref.clone(),
        args_digest: sha2::Sha256::digest(args).into(),
        invocation_nonce: nonce,
        causal_context: CausalContext::None,
    };
    let descriptor_bound =
        DescriptorBoundEnvelope::new(axiom_env).expect("descriptor-bound test envelope");
    let key_id_hint = BASE64_STANDARD.encode(signing_key.verifying_key().to_bytes());
    let sig =
        sign_descriptor_bound_invocation(signing_key, &descriptor_bound, key_id_hint.as_str());

    let envelope = Envelope {
        caller: Some(PbAgentIdentity {
            ura: caller_ura.to_string(),
            profile: "easynet-strict-v2".to_string(),
        }),
        callee: Some(PbAgentIdentity {
            ura: callee_ura.to_string(),
            profile: "easynet-strict-v2".to_string(),
        }),
        subject: Some(PbSubjectIdentity {
            ura: subject_ura.clone(),
            profile: "easynet-strict-v2".to_string(),
        }),
        invocation_nonce: nonce.to_vec(),
        caller_signature: Some(PbCallerSignature {
            algorithm: sig.algorithm,
            signature: sig.signature,
            key_id_hint: sig.key_id_hint,
        }),
        ..Envelope::default()
    };

    InvokeRequest {
        envelope: Some(envelope),
        function_name: ability.to_string(),
        arguments: args.to_vec(),
        metadata: std::collections::HashMap::from([(
            SIGNED_DESCRIPTOR_REF_METADATA_KEY.to_string(),
            ability_ref,
        )]),
        ..InvokeRequest::default()
    }
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn cross_realm_signed_caller_accepted_via_federated_resolve_key() {
    const REALM_A: &str = "realm-a";
    const REALM_B: &str = "realm-b";
    const DEVICE_A_URA: &str = "easynet:///r/realm-a/device/device-A";
    const PEER_HUB_URA: &str = "in-process-A";
    let daemon_b_ura = easynet_cli::core::ura::hub_ura(REALM_B);

    // ── Mint device-A's signing key ─────────────────────────────
    let device_a_key = SigningKey::from_bytes(&[0xA1u8; 32]);
    let device_a_pubkey_b64 = BASE64_STANDARD.encode(device_a_key.verifying_key().to_bytes());

    // ── Daemon A: trust set = [device-A]; serves resolve_key ──
    let mut daemon_a_anchor_inner = RealmTrustAnchor::default();
    daemon_a_anchor_inner
        .append_agent(TrustedAgent {
            agent_ura: DEVICE_A_URA.to_string(),
            public_key_b64: device_a_pubkey_b64.clone(),
            role: TrustedAgentRole::Device,
            added_at_unix_ms: 1_714_492_800_000,
            origin_realm: None,
            hub_endpoint: None,
            tls_ca_pem_path: None,
        })
        .expect("append device-A");
    let daemon_a_anchor = Arc::new(daemon_a_anchor_inner);
    let daemon_a_admission = AdmissionFacade::new(
        daemon_a_anchor,
        Some(easynet_cli::core::ura::hub_ura(REALM_A)),
    );
    let daemon_a = Arc::new(
        DaemonInvocationService::new(Arc::new(PresenceRegistry::new()), daemon_a_admission)
            .with_session_realm(REALM_A),
    );

    // ── Daemon B: empty trust + federated_peers → daemon A ──
    // Build the in-process forwarder, register it as B's
    // FederationClient, and stamp `federated_peers[realm-a] →
    // PEER_HUB_URA` so the FederatedKeyResolver routes there for
    // any caller in realm-a.
    let federation_client: Arc<dyn FederationClient> = Arc::new(InProcessForwarder {
        peer: Arc::clone(&daemon_a),
        peer_loopback_uri: easynet_cli::core::ura::hub_ura(REALM_A),
    });
    let mut peers = std::collections::BTreeMap::new();
    peers.insert(REALM_A.to_string(), PEER_HUB_URA.to_string());
    let peers_cell = SharedFederatedPeers::new(peers);

    let daemon_b_admission = AdmissionFacade::new(
        Arc::new(RealmTrustAnchor::default()),
        Some(daemon_b_ura.clone()),
    )
    .with_federation(Arc::clone(&federation_client), peers_cell.clone())
    .with_hub_signing_seed(REALM_B_HUB_SIGNING_SEED);
    let daemon_b = DaemonInvocationService::new(
        Arc::new(PresenceRegistry::new()),
        daemon_b_admission.clone(),
    )
    .with_session_realm(REALM_B)
    .with_local_runtime(LocalRuntime::new());

    // ── Build a signed `self.echo` invocation from device-A ──
    // device-A signs over the canonical bytes; daemon B's
    // admission fetches the signing pubkey from daemon A and
    // verifies the signature.
    //
    // `self.echo` does not need to actually be implemented for the
    // test — admission acceptance is the assertion target. Daemon B
    // has an empty LocalRuntime wired, so resolve-first dispatch now
    // rejects the unbound ability with ROUTE_NEGATIVE/NODATA AFTER
    // admission has already passed; the test catches that as the
    // success signal.
    let signed = signed_request(
        DEVICE_A_URA,
        &daemon_b_ura,
        "self.echo",
        b"{}",
        &device_a_key,
        [0x33u8; 16],
    );

    // ── Drive: daemon B's admission must accept ──
    let outcome = daemon_b.invoke(tonic::Request::new(signed)).await;
    match outcome {
        Ok(_) => {
            // Daemon B happens to admit + dispatch + complete. The
            // test only requires admission acceptance, but if a
            // working dispatcher returns Ok we accept that too.
        }
        Err(status) => {
            // Admission acceptance is proven by reaching the
            // resolve-first route gate: this is a post-admission
            // unbound-ability miss, not a §5.2 signature/nonce
            // rejection (`PermissionDenied` / `InvalidArgument`).
            assert_eq!(
                status.code(),
                tonic::Code::FailedPrecondition,
                "expected post-admission unbound-ability route miss, but got code={:?} message={}",
                status.code(),
                status.message()
            );
            assert!(
                status.message().contains("ROUTE_NEGATIVE")
                    && status.message().contains("NEGATIVE_REASON_NODATA"),
                "expected route NODATA after admission, got message={}",
                status.message()
            );
        }
    }

    // Phase 5a (SharedReceiptStore deletion): the original
    // PR-10 commit 5/N assertion was that B's admission recorded
    // exactly one `"admitted"` receipt naming device-A as caller.
    // That ring-buffer is gone — admission success is now observable
    // through the dispatch outcome assertions above (the call passed
    // admission and reached the resolve-first NODATA arm, which proves
    // the caller's signature verified and the nonce was accepted).
    // Audit-trail-level persistence of "who admitted what"
    // moved to the `InvocationLedger` rows the `LedgerSink` writes
    // at terminal time; this test scenario doesn't reach terminal
    // (intentionally — it stops at admission), so no ledger row is
    // expected here either.
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn cross_realm_caller_with_no_federated_peer_entry_rejected() {
    // Counter-test: same shape as above but `federated_peers` is
    // empty. The FederatedKeyResolver must NOT dial (operator did
    // not opt into resolving realm-a). Admission rejects with
    // `caller_signature_invalid` (the wire surface for "URA not
    // trusted").
    const REALM_B: &str = "realm-b";
    const DEVICE_A_URA: &str = "easynet:///r/realm-a/device/device-A";
    let daemon_b_ura = easynet_cli::core::ura::hub_ura(REALM_B);

    let device_a_key = SigningKey::from_bytes(&[0xB2u8; 32]);

    // Daemon A is irrelevant — we don't dial anywhere. Build a
    // dial-failed client to prove the resolver doesn't even try.
    struct DialFailedClient;
    #[async_trait]
    impl FederationClient for DialFailedClient {
        async fn forward_invoke(
            &self,
            target_hub_endpoint: &HubEndpoint,
            _request: InvokeRequest,
        ) -> Result<InvokeResponse, FederationClientError> {
            Err(FederationClientError::DialFailed {
                endpoint: target_hub_endpoint.clone(),
                detail: "test must not dial".to_string(),
            })
        }
    }
    let federation_client: Arc<dyn FederationClient> = Arc::new(DialFailedClient);
    let peers_cell = SharedFederatedPeers::new(std::collections::BTreeMap::new());

    let daemon_b_admission = AdmissionFacade::new(
        Arc::new(RealmTrustAnchor::default()),
        Some(daemon_b_ura.clone()),
    )
    .with_federation(federation_client, peers_cell);
    let daemon_b =
        DaemonInvocationService::new(Arc::new(PresenceRegistry::new()), daemon_b_admission)
            .with_session_realm(REALM_B);

    let signed = signed_request(
        DEVICE_A_URA,
        &daemon_b_ura,
        "self.echo",
        b"{}",
        &device_a_key,
        [0x44u8; 16],
    );

    let err = daemon_b
        .invoke(tonic::Request::new(signed))
        .await
        .expect_err("must reject — no federated peer entry");
    // The wire-stable reject reason for "URA not trusted" rolls up
    // to `PermissionDenied` (the trust anchor membership gate)
    // when the URA is missing locally and the resolver collapses to
    // local-only. INV-4 fail-closed by construction.
    assert_eq!(err.code(), tonic::Code::PermissionDenied);
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn cross_realm_forged_signature_rejected_after_key_resolves() {
    // The crypto-gate counter-test. Unlike the membership-gate test
    // above, here EVERYTHING resolves: daemon A's trust set holds
    // device-A's REAL public key, daemon B's federated_peers routes
    // realm-a to daemon A, and the FederatedKeyResolver successfully
    // fetches device-A's pubkey. The ONLY thing wrong is the
    // signature: it was produced by an attacker's key, not device-A's.
    //
    // This isolates the Ed25519 verification step. A positive
    // "valid signature admitted" test cannot prove the verifier
    // works — a stub that always admits would pass it too. Proving
    // the verifier REJECTS a forged signature (with the key present
    // and resolvable) is what shows the crypto gate is load-bearing.
    const REALM_A: &str = "realm-a";
    const REALM_B: &str = "realm-b";
    const DEVICE_A_URA: &str = "easynet:///r/realm-a/device/device-A";
    const PEER_HUB_URA: &str = "in-process-A";
    let daemon_b_ura = easynet_cli::core::ura::hub_ura(REALM_B);

    // device-A's REAL keypair — its public half goes into daemon A's
    // trust set so the resolver returns a valid, parseable pubkey.
    let device_a_key = SigningKey::from_bytes(&[0xA1u8; 32]);
    let device_a_pubkey_b64 = BASE64_STANDARD.encode(device_a_key.verifying_key().to_bytes());

    // The attacker's key. It is NEVER registered anywhere. The forged
    // envelope claims caller = device-A but is signed with this key,
    // so verification against device-A's real pubkey must fail.
    let attacker_key = SigningKey::from_bytes(&[0xEEu8; 32]);

    // ── Daemon A: trust set = [device-A real pubkey]; serves resolve_key ──
    let mut daemon_a_anchor_inner = RealmTrustAnchor::default();
    daemon_a_anchor_inner
        .append_agent(TrustedAgent {
            agent_ura: DEVICE_A_URA.to_string(),
            public_key_b64: device_a_pubkey_b64.clone(),
            role: TrustedAgentRole::Device,
            added_at_unix_ms: 1_714_492_800_000,
            origin_realm: None,
            hub_endpoint: None,
            tls_ca_pem_path: None,
        })
        .expect("append device-A");
    let daemon_a = Arc::new(
        DaemonInvocationService::new(
            Arc::new(PresenceRegistry::new()),
            AdmissionFacade::new(
                Arc::new(daemon_a_anchor_inner),
                Some(easynet_cli::core::ura::hub_ura(REALM_A)),
            ),
        )
        .with_session_realm(REALM_A),
    );

    // ── Daemon B: empty trust + federated_peers → daemon A ──
    let federation_client: Arc<dyn FederationClient> = Arc::new(InProcessForwarder {
        peer: Arc::clone(&daemon_a),
        peer_loopback_uri: easynet_cli::core::ura::hub_ura(REALM_A),
    });
    let mut peers = std::collections::BTreeMap::new();
    peers.insert(REALM_A.to_string(), PEER_HUB_URA.to_string());
    let peers_cell = SharedFederatedPeers::new(peers);

    let daemon_b_admission = AdmissionFacade::new(
        Arc::new(RealmTrustAnchor::default()),
        Some(daemon_b_ura.clone()),
    )
    .with_federation(Arc::clone(&federation_client), peers_cell)
    .with_hub_signing_seed(REALM_B_HUB_SIGNING_SEED);
    let daemon_b =
        DaemonInvocationService::new(Arc::new(PresenceRegistry::new()), daemon_b_admission)
            .with_session_realm(REALM_B)
            .with_local_runtime(LocalRuntime::new());

    // Forge: caller = device-A, but signed with the attacker's key.
    let forged = signed_request(
        DEVICE_A_URA,
        &daemon_b_ura,
        "self.echo",
        b"{}",
        &attacker_key,
        [0x55u8; 16],
    );

    let err = daemon_b
        .invoke(tonic::Request::new(forged))
        .await
        .expect_err("a forged signature must be rejected, not admitted");
    // It must fail at the §5.2 signature-verification step, NOT at
    // dispatch. A `NotFound` here would mean admission wrongly passed
    // a forged signature and fell through to the (unimplemented)
    // ability — the exact failure this test guards against.
    assert_ne!(
        err.code(),
        tonic::Code::NotFound,
        "forged signature reached dispatch — the crypto gate did not verify it",
    );
    assert!(
        matches!(
            err.code(),
            tonic::Code::PermissionDenied | tonic::Code::InvalidArgument
        ),
        "forged signature must reject with a §5.2 admission code, got code={:?} message={}",
        err.code(),
        err.message(),
    );
}
