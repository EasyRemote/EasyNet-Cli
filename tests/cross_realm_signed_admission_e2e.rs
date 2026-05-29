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
use ed25519_dalek::{Signer, SigningKey};
use sha2::{Digest, Sha256};

use easynet_axon::invocation::axiom::{
    canonical_invocation_bytes, AgentIdentity as AxiomAgentIdentity, CausalContext,
    InvocationEnvelope, SubjectIdentity, UraProfile,
};
use easynet_axon::invocation::LocalRuntime;
use easynet_cli::pb::axon::v1::invocation_server::Invocation;
use easynet_cli::pb::axon::v1::{
    AgentIdentity as PbAgentIdentity, CallerSignature as PbCallerSignature, Envelope,
    InvokeRequest, InvokeResponse, SubjectIdentity as PbSubjectIdentity,
};
use easynet_cli::services::axon_serve::admission_facade::AdmissionFacade;
use easynet_cli::services::axon_serve::daemon_invocation_service::DaemonInvocationService;
use easynet_cli::services::federated_peers_cell::SharedFederatedPeers;
use easynet_cli::services::federation_client::{FederationClient, FederationClientError, HubUri};
use easynet_cli::services::presence_registry::PresenceRegistry;
use easynet_cli::services::realm_trust_anchor::{RealmTrustAnchor, TrustedAgent, TrustedAgentRole};

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
        _target_hub: &HubUri,
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
                hub: "in-process-A".to_string(),
                status: format!("code={:?} message={}", status.code(), status.message()),
            })?;
        Ok(response.into_inner())
    }
}

/// Build a signed `InvokeRequest` where:
/// - caller URA is `caller_ura`
/// - callee/subject URA is `callee_ura`
/// - ability is `ability`, args is `args`
/// - the envelope's `caller_signature` is a real Ed25519 signature
///   over the canonical invocation bytes computed by axon's encoder
///
/// Mirrors the production CLI bridge's signing path so admission
/// verifies against bytes the test really signed.
fn signed_request(
    caller_ura: &str,
    callee_ura: &str,
    ability: &str,
    args: &[u8],
    signing_key: &SigningKey,
    nonce: [u8; 16],
) -> InvokeRequest {
    let mut hasher = Sha256::new();
    hasher.update(args);
    let args_digest: [u8; 32] = hasher.finalize().into();

    let axiom_env = InvocationEnvelope {
        caller: AxiomAgentIdentity::new(caller_ura, UraProfile::EasynetStrictV2),
        callee: AxiomAgentIdentity::new(callee_ura, UraProfile::EasynetStrictV2),
        subject: SubjectIdentity::new(callee_ura, UraProfile::EasynetStrictV2),
        ability: ability.to_string(),
        args_digest,
        invocation_nonce: nonce,
        causal_context: CausalContext::None,
    };
    let bytes = canonical_invocation_bytes(&axiom_env);
    let sig = signing_key.sign(&bytes);

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
            ura: callee_ura.to_string(),
            profile: "easynet-strict-v2".to_string(),
        }),
        invocation_nonce: nonce.to_vec(),
        caller_signature: Some(PbCallerSignature {
            algorithm: "ed25519".to_string(),
            signature: sig.to_bytes().to_vec(),
            key_id_hint: String::new(),
        }),
        ..Envelope::default()
    };

    InvokeRequest {
        envelope: Some(envelope),
        function_name: ability.to_string(),
        arguments: args.to_vec(),
        ..InvokeRequest::default()
    }
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn cross_realm_signed_caller_admitted_via_federated_resolve_key() {
    const REALM_A: &str = "realm-a";
    const REALM_B: &str = "realm-b";
    const DEVICE_A_URA: &str = "easynet:///r/realm-a/device/device-A";
    const DAEMON_B_URA: &str = "easynet:///r/realm-b/hub";
    const PEER_HUB_URA: &str = "in-process-A";

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
            origin_tenant_id: None,
            hub_uri: None,
            tls_ca_pem_path: None,
        })
        .expect("append device-A");
    let daemon_a_anchor = Arc::new(daemon_a_anchor_inner);
    let daemon_a_admission = AdmissionFacade::new(
        daemon_a_anchor,
        Some("easynet:///r/realm-a/hub".to_string()),
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
        peer_loopback_uri: "easynet:///r/realm-a/hub".to_string(),
    });
    let mut peers = std::collections::BTreeMap::new();
    peers.insert(REALM_A.to_string(), PEER_HUB_URA.to_string());
    let peers_cell = SharedFederatedPeers::new(peers);

    let daemon_b_admission = AdmissionFacade::new(
        Arc::new(RealmTrustAnchor::default()),
        Some(DAEMON_B_URA.to_string()),
    )
    .with_federation(Arc::clone(&federation_client), peers_cell.clone());
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
    // has an empty LocalRuntime wired, so it returns `Status::not_found`
    // for unknown abilities AFTER admission has already passed; the
    // test catches that as the success signal (admission succeeded;
    // dispatch then fails for an unrelated reason).
    let signed = signed_request(
        DEVICE_A_URA,
        DAEMON_B_URA,
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
            // Admission acceptance is proven by the failure code
            // being `NotFound` (post-admission dispatch miss)
            // rather than the §5.2 reject codes
            // (`PermissionDenied` / `InvalidArgument`).
            assert_eq!(
                status.code(),
                tonic::Code::NotFound,
                "expected post-admission unknown-ability dispatch miss, but got code={:?} message={}",
                status.code(),
                status.message()
            );
        }
    }

    // Phase 5a (SharedReceiptStore deletion): the original
    // PR-10 commit 5/N assertion was that B's admission recorded
    // exactly one `"admitted"` receipt naming device-A as caller.
    // That ring-buffer is gone — admission success is now observable
    // through the dispatch outcome assertions above (the call passed
    // admission and reached the `NotFound` dispatch arm, which
    // proves the caller's signature verified and the nonce was
    // accepted). Audit-trail-level persistence of "who admitted what"
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
    const DAEMON_B_URA: &str = "easynet:///r/realm-b/hub";

    let device_a_key = SigningKey::from_bytes(&[0xB2u8; 32]);

    // Daemon A is irrelevant — we don't dial anywhere. Build a
    // dial-failed client to prove the resolver doesn't even try.
    struct DialFailedClient;
    #[async_trait]
    impl FederationClient for DialFailedClient {
        async fn forward_invoke(
            &self,
            target_hub: &HubUri,
            _request: InvokeRequest,
        ) -> Result<InvokeResponse, FederationClientError> {
            Err(FederationClientError::DialFailed {
                hub: target_hub.clone(),
                detail: "test must not dial".to_string(),
            })
        }
    }
    let federation_client: Arc<dyn FederationClient> = Arc::new(DialFailedClient);
    let peers_cell = SharedFederatedPeers::new(std::collections::BTreeMap::new());

    let daemon_b_admission = AdmissionFacade::new(
        Arc::new(RealmTrustAnchor::default()),
        Some(DAEMON_B_URA.to_string()),
    )
    .with_federation(federation_client, peers_cell);
    let daemon_b =
        DaemonInvocationService::new(Arc::new(PresenceRegistry::new()), daemon_b_admission)
            .with_session_realm(REALM_B);

    let signed = signed_request(
        DEVICE_A_URA,
        DAEMON_B_URA,
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
