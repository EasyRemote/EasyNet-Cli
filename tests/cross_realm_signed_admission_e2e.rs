// EasyNet CLI — PR-N2 commit 3/N: cross-realm signed admission e2e
// =================================================================
//
// File: tests/cross_realm_signed_admission_e2e.rs
// Description: In-process two-daemon test where daemon B admits a
//              signed envelope from a caller in realm-A's trust set
//              by dialling A's `federation.resolve_key` ability via
//              the FederatedKeyResolver shared by B's Axon runtime
//              and product-policy facade.
//
// Why this test exists
// --------------------
// The runtime and product policy must share one caller-key provider.
// This test proves the resulting cross-realm strict admission end to end:
//
//   "daemon B receives an envelope signed by a key it has never seen,
//    fetches the key from A, runs the §5.2 4-step verify, and admits."
//
// This integration test is that proof. The chain:
//
//   1. Test mints `device-A`'s Ed25519 keypair.
//   2. Daemon A boots with device-A and daemon B's Hub signer in its
//      trust set, then serves `federation.resolve_key` through LocalRuntime.
//   3. Daemon B boots with realm-b trust set = empty; daemon B's
//      runtime and AdmissionFacade share a `FederatedKeyResolver` whose
//      federation client is an in-process forwarder into daemon A,
//      and whose `federated_peers = {realm-a → "in-process-A"}`.
//   4. Test signs the registered `federation.status` descriptor with
//      caller = device-A's URA, callee = daemon B's URA.
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
//   7. Daemon B reaches product policy for the real registered route;
//      the policy denial carries the verified signature key id.
//
// What this test does NOT exercise
// --------------------------------
// - Real TCP/TLS. The federation client is an in-process forwarder
//   that bypasses the network. The 2-daemon spawned-binary version
//   (real TLS handshake) is a follow-up; this test verifies the
//   admission + resolve_key correlation logic deterministically.
// - The PR-N1 invoke unwrap path. The signed envelope is
//   delivered straight to daemon B's invoke endpoint, not through
//   `federation.invoke`. Composing the two is PR-N3
//   territory (full discover + forward + signed admit chain).
//
// Author: Silan.Hu <silan.hu@u.nus.edu>
// Copyright (c) 2026 EasyNet. All rights reserved.

#![cfg(feature = "axon-pb")]

use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::Arc;

#[path = "support/runtime_fixture.rs"]
mod runtime_fixture;

use async_trait::async_trait;
use base64::{engine::general_purpose::STANDARD as BASE64_STANDARD, Engine as _};
use ed25519_dalek::{Signature, Signer as _, SigningKey, VerifyingKey};
use sha2::Digest as _;

use axon_sdk::invocation::axiom::{
    AgentIdentity as AxiomAgentIdentity, CausalContext, DescriptorBoundEnvelope,
    InvocationEnvelope, SubjectIdentity, UraProfile,
};
use axon_sdk::invocation::{DescriptorBoundInvocationDraft, ErrorCode, KeyResolver};
use axon_sdk::pb::axon::v1::invocation_server::Invocation;
use axon_sdk::pb::axon::v1::{
    invocation_target, AbilityTarget, AgentIdentity as PbAgentIdentity,
    CallerSignature as PbCallerSignature, Envelope, Error as PbError, ErrorStage as PbErrorStage,
    InvocationState, InvocationTarget, InvokeRequest, InvokeResponse,
    SecurityClass as PbSecurityClass, SubjectIdentity as PbSubjectIdentity,
};
use easynet_cli::daemon::ability::dispatch::AbilityAuthorityContext;
use easynet_cli::daemon::federation::client::{
    FederationClient, FederationClientError, HubEndpoint,
};
use easynet_cli::daemon::federation::peers::SharedFederatedPeers;
use easynet_cli::daemon::identity::self_identity::{CanonicalSigner, SelfIdentityError};
use easynet_cli::daemon::invocation::admission::admission_facade::AdmissionFacade;
use easynet_cli::daemon::invocation::admission::federated_key_resolver::FederatedKeyResolver;
use easynet_cli::daemon::invocation::bidi::state::presence::PresenceRegistry;
use easynet_cli::daemon::invocation::dispatch::daemon_invocation_service::DaemonInvocationService;
use easynet_cli::daemon::invocation::dispatch::federation_wrappers::ABILITY_FEDERATION_STATUS;
use easynet_cli::daemon::trust::anchor::{RealmTrustAnchor, TrustAnchorRole, TrustedAgent};
use easynet_cli::daemon::trust::cell::SharedTrustAnchor;

const REALM_B_HUB_SIGNING_SEED: [u8; 32] = [0xB0; 32];

fn test_attempt_ledger_path(realm: &str) -> std::path::PathBuf {
    static SEQ: AtomicU64 = AtomicU64::new(1);
    std::env::temp_dir().join(format!(
        "easynet-cross-realm-signed-admission-{realm}-{}-{}.jsonl",
        std::process::id(),
        SEQ.fetch_add(1, Ordering::Relaxed)
    ))
}

/// In-process federation client that forwards every `invoke`
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
/// The forwarder preserves the signed Hub-to-Hub request generated by
/// `PeerInvokeRequest`; only the network transport is replaced.
struct InProcessForwarder {
    peer: Arc<DaemonInvocationService>,
}

#[async_trait]
impl FederationClient for InProcessForwarder {
    async fn invoke(
        &self,
        _target_hub_endpoint: &HubEndpoint,
        request: InvokeRequest,
    ) -> Result<InvokeResponse, FederationClientError> {
        let response = self
            .peer
            .invoke(tonic::Request::new(request))
            .await
            .map_err(|status| FederationClientError::InnerInvokeFailed {
                endpoint: "in-process-A".to_string(),
                status_code: status.code(),
                status_message: status.message().to_string(),
            })?;
        Ok(response.into_inner())
    }
}

struct IntegrationSigner {
    owner_ura: String,
    signing_key: SigningKey,
}

#[async_trait]
impl CanonicalSigner for IntegrationSigner {
    fn owner_ura(&self) -> &str {
        &self.owner_ura
    }

    async fn sign_canonical(&self, canonical_bytes: &[u8]) -> Result<Signature, SelfIdentityError> {
        Ok(self.signing_key.sign(canonical_bytes))
    }

    fn signing_public_key(&self) -> Result<VerifyingKey, SelfIdentityError> {
        Ok(self.signing_key.verifying_key())
    }
}

fn test_hub_signer(realm: &str) -> Arc<dyn CanonicalSigner> {
    Arc::new(IntegrationSigner {
        owner_ura: easynet_cli::core::ura::hub_ura(realm),
        signing_key: SigningKey::from_bytes(&REALM_B_HUB_SIGNING_SEED),
    })
}

async fn hub_service(
    realm: &str,
    trust: SharedTrustAnchor,
    federated_keys: Arc<FederatedKeyResolver>,
) -> (
    DaemonInvocationService,
    Arc<easynet_cli::daemon::ability::dispatch::AxonAbilityCatalog>,
) {
    let daemon_ura = easynet_cli::core::ura::hub_ura(realm);
    let runtime_keys: Arc<dyn KeyResolver> = federated_keys.clone();
    let daemon_runtime = runtime_fixture::daemon_runtime_with_key_resolver(runtime_keys);
    let runtime = daemon_runtime.runtime();
    let agents = easynet_cli::daemon::persistence::agent_registry::AgentRegistry::default();
    let authority = AbilityAuthorityContext::for_realm_authority_root(daemon_ura.clone())
        .expect("hub authority context");
    let mut config =
        easynet_cli::daemon::ability::catalog::RegistryBuildConfig::new_with_authority_context(
            easynet_cli::daemon::ability::catalog::RegistryBuildServices::fresh(),
            &agents,
            authority,
        );
    config.local_runtime = Some(runtime);
    let catalog =
        easynet_cli::daemon::ability::catalog::build_registry_with_services_result(config)
            .expect("build descriptor-bound hub catalog")
            .catalog;
    let admission = AdmissionFacade::with_trust_anchor_cell(trust, Some(daemon_ura.clone()))
        .with_ability_catalog(Arc::clone(&catalog))
        .with_federated_key_resolver(federated_keys);
    let service = DaemonInvocationService::new(Arc::new(PresenceRegistry::new()), admission)
        .with_session_realm(realm)
        .with_daemon_runtime(daemon_runtime)
        .with_local_ability_catalog(Arc::clone(&catalog))
        .with_invocation_attempt_ledger_path(test_attempt_ledger_path(realm))
        .expect("open cross-realm signed-admission attempt audit ledger");
    service
        .register_daemon_unary_routes(&daemon_ura)
        .await
        .expect("register descriptor-bound daemon routes");
    (service, catalog)
}

fn descriptor_ref(
    catalog: &easynet_cli::daemon::ability::dispatch::AxonAbilityCatalog,
    owner_ura: &str,
    ability: &str,
) -> String {
    let mut matches = catalog
        .authority_ability_catalog_snapshot()
        .into_iter()
        .filter(|row| row.name == ability)
        .filter(|row| row.descriptor.call_mode() == easynet_cli::daemon::ability::CallMode::Rpc);
    let descriptor = matches
        .next()
        .unwrap_or_else(|| panic!("hub catalog missing RPC descriptor for {ability}"))
        .descriptor
        .rebind_owner_ura(owner_ura)
        .expect("hub descriptor owner binding");
    assert!(
        matches.next().is_none(),
        "hub catalog has ambiguous RPC descriptor for {ability}"
    );
    format!(
        "{}@{}#{}!{}",
        descriptor
            .canonical_ability_ura()
            .expect("hub descriptor ability URA"),
        descriptor.version,
        hex::encode(descriptor.descriptor_hash_bytes()),
        descriptor.admission_action().as_str(),
    )
}

/// Mirrors the production descriptor-bound signing path so admission
/// verifies exactly the bytes the test signed.
fn signed_request(
    caller_ura: &str,
    callee_ura: &str,
    ability: &str,
    ability_ref: &str,
    args: &[u8],
    signing_key: &SigningKey,
    nonce: [u8; 16],
) -> InvokeRequest {
    let subject_ura = ability_ref
        .split_once('@')
        .map(|(ability_ura, _)| ability_ura.to_string())
        .expect("descriptor ref carries version");
    let subject = SubjectIdentity::new(&subject_ura, UraProfile::StrictV2);
    let axiom_env = InvocationEnvelope {
        caller: AxiomAgentIdentity::new(caller_ura, UraProfile::StrictV2),
        callee: AxiomAgentIdentity::new(callee_ura, UraProfile::StrictV2),
        subject: subject.clone(),
        ability: ability_ref.to_string(),
        args_digest: sha2::Sha256::digest(args).into(),
        invocation_nonce: nonce,
        causal_context: CausalContext::None,
    };
    let descriptor_bound =
        DescriptorBoundEnvelope::new(axiom_env).expect("descriptor-bound test envelope");
    let key_id_hint = BASE64_STANDARD.encode(signing_key.verifying_key().to_bytes());
    let sig = DescriptorBoundInvocationDraft::from_envelope(descriptor_bound)
        .sign_caller_signature(signing_key, key_id_hint.as_str());

    let envelope = Envelope {
        caller: Some(PbAgentIdentity {
            ura: caller_ura.to_string(),
            profile: "axon-strict-v2".to_string(),
        }),
        callee: Some(PbAgentIdentity {
            ura: callee_ura.to_string(),
            profile: "axon-strict-v2".to_string(),
        }),
        subject: Some(PbSubjectIdentity {
            ura: subject_ura.clone(),
            profile: "axon-strict-v2".to_string(),
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
        target: Some(InvocationTarget {
            typed_target: Some(invocation_target::TypedTarget::Ability(AbilityTarget {
                ability_name: ability_ref.to_string(),
                function_name: ability.to_string(),
            })),
        }),
        arguments: args.to_vec(),
        ..InvokeRequest::default()
    }
}

fn expect_canonical_in_band_failure(
    result: Result<tonic::Response<InvokeResponse>, tonic::Status>,
    expected_code: ErrorCode,
    expectation: &str,
) -> PbError {
    let response = result
        .unwrap_or_else(|status| panic!("{expectation}: unexpected transport error: {status}"))
        .into_inner();
    assert_eq!(
        response.state,
        InvocationState::Failed as i32,
        "{expectation}: invocation must reach the canonical failed state"
    );
    let error = response
        .error
        .unwrap_or_else(|| panic!("{expectation}: failed response must carry a typed error"));
    assert_eq!(
        error.code,
        expected_code.as_str(),
        "{expectation}: wrong canonical error: {error:?}"
    );
    error
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn cross_realm_signed_caller_resolves_key_before_policy_or_dispatch() {
    const REALM_A: &str = "realm-a";
    const REALM_B: &str = "realm-b";
    const DEVICE_A_URA: &str = "easynet:///r/realm-a/device/device-A";
    const PEER_HUB_URA: &str = "in-process-A";
    let daemon_b_ura = easynet_cli::core::ura::hub_ura(REALM_B);

    // ── Mint device-A's signing key ─────────────────────────────
    let device_a_key = SigningKey::from_bytes(&[0xA1u8; 32]);
    let device_a_pubkey_b64 = BASE64_STANDARD.encode(device_a_key.verifying_key().to_bytes());

    // Daemon A trusts both device-A and daemon B's Hub signing authority.
    // The in-process federation hop preserves B's real signed request.
    let mut daemon_a_anchor_inner = RealmTrustAnchor::default();
    daemon_a_anchor_inner
        .append_agent(TrustedAgent {
            agent_ura: DEVICE_A_URA.to_string(),
            public_key_b64: device_a_pubkey_b64.clone(),
            role: TrustAnchorRole::Device,
            added_at_unix_ms: 1_714_492_800_000,
            origin_realm: None,
            hub_endpoint: None,
            tls_ca_pem_path: None,
        })
        .expect("append device-A");
    daemon_a_anchor_inner
        .append_agent(TrustedAgent {
            agent_ura: daemon_b_ura.clone(),
            public_key_b64: BASE64_STANDARD.encode(
                SigningKey::from_bytes(&REALM_B_HUB_SIGNING_SEED)
                    .verifying_key()
                    .to_bytes(),
            ),
            role: TrustAnchorRole::Hub,
            added_at_unix_ms: 1_714_492_800_000,
            origin_realm: Some(REALM_B.to_string()),
            hub_endpoint: Some("in-process-B".to_string()),
            tls_ca_pem_path: None,
        })
        .expect("append daemon B hub");
    let daemon_a_trust = SharedTrustAnchor::new(Arc::new(daemon_a_anchor_inner));
    let daemon_a_keys = Arc::new(FederatedKeyResolver::new(
        daemon_a_trust.clone(),
        None,
        SharedFederatedPeers::default(),
        Some(REALM_A.to_string()),
    ));
    let (daemon_a, _) = hub_service(REALM_A, daemon_a_trust, daemon_a_keys).await;
    let daemon_a = Arc::new(daemon_a);

    // ── Daemon B: empty trust + federated_peers → daemon A ──
    // Build the in-process forwarder, register it as B's
    // FederationClient, and stamp `federated_peers[realm-a] →
    // PEER_HUB_URA` so the FederatedKeyResolver routes there for
    // any caller in realm-a.
    let federation_client: Arc<dyn FederationClient> = Arc::new(InProcessForwarder {
        peer: Arc::clone(&daemon_a),
    });
    let mut peers = std::collections::BTreeMap::new();
    peers.insert(REALM_A.to_string(), PEER_HUB_URA.to_string());
    let peers_cell = SharedFederatedPeers::new(peers);

    let daemon_b_trust = SharedTrustAnchor::new(Arc::new(RealmTrustAnchor::default()));
    let daemon_b_keys = Arc::new(
        FederatedKeyResolver::new(
            daemon_b_trust.clone(),
            Some(Arc::clone(&federation_client)),
            peers_cell,
            Some(REALM_B.to_string()),
        )
        .with_hub_signer(test_hub_signer(REALM_B)),
    );
    let (daemon_b, catalog_b) = hub_service(REALM_B, daemon_b_trust, daemon_b_keys).await;
    let signed_descriptor = descriptor_ref(&catalog_b, &daemon_b_ura, ABILITY_FEDERATION_STATUS);
    let signed = signed_request(
        DEVICE_A_URA,
        &daemon_b_ura,
        ABILITY_FEDERATION_STATUS,
        &signed_descriptor,
        b"{}",
        &device_a_key,
        [0x33u8; 16],
    );

    let policy_denial = expect_canonical_in_band_failure(
        daemon_b.invoke(tonic::Request::new(signed)).await,
        ErrorCode::AbilityForbidden,
        "unowned cross-realm device must reach product policy after authentication",
    );
    assert!(
        policy_denial.message.contains("POLICY_DENIED")
            && policy_denial.message.contains("signature_key_id"),
        "valid federated signature must be verified before product policy, got {}",
        policy_denial.message,
    );

    // The request intentionally stops at product policy. No handler starts and
    // no terminal ledger row is expected; the policy evidence above proves the
    // descriptor-bound caller authentication stage completed.
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn cross_realm_caller_with_no_federated_peer_entry_rejected() {
    // Counter-test: same shape as above but `federated_peers` is
    // empty. The FederatedKeyResolver must NOT dial (operator did
    // not opt into resolving realm-a). Canonical admission rejects with
    // `CALLER_KEY_NOT_FOUND`.
    const REALM_B: &str = "realm-b";
    const DEVICE_A_URA: &str = "easynet:///r/realm-a/device/device-A";
    let daemon_b_ura = easynet_cli::core::ura::hub_ura(REALM_B);

    let device_a_key = SigningKey::from_bytes(&[0xB2u8; 32]);

    // Daemon A is irrelevant — we don't dial anywhere. Build a
    // dial-failed client to prove the resolver doesn't even try.
    struct DialFailedClient;
    #[async_trait]
    impl FederationClient for DialFailedClient {
        async fn invoke(
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

    let daemon_b_trust = SharedTrustAnchor::new(Arc::new(RealmTrustAnchor::default()));
    let daemon_b_keys = Arc::new(
        FederatedKeyResolver::new(
            daemon_b_trust.clone(),
            Some(federation_client),
            peers_cell,
            Some(REALM_B.to_string()),
        )
        .with_hub_signer(test_hub_signer(REALM_B)),
    );
    let (daemon_b, catalog_b) = hub_service(REALM_B, daemon_b_trust, daemon_b_keys).await;
    let signed_descriptor = descriptor_ref(&catalog_b, &daemon_b_ura, ABILITY_FEDERATION_STATUS);

    let signed = signed_request(
        DEVICE_A_URA,
        &daemon_b_ura,
        ABILITY_FEDERATION_STATUS,
        &signed_descriptor,
        b"{}",
        &device_a_key,
        [0x44u8; 16],
    );

    let error = expect_canonical_in_band_failure(
        daemon_b.invoke(tonic::Request::new(signed)).await,
        ErrorCode::CallerKeyNotFound,
        "missing peer authority must reject before route dispatch",
    );
    assert_eq!(error.stage, PbErrorStage::CallerAuthentication as i32);
    assert_eq!(error.security_class, PbSecurityClass::Identity as i32);
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
            role: TrustAnchorRole::Device,
            added_at_unix_ms: 1_714_492_800_000,
            origin_realm: None,
            hub_endpoint: None,
            tls_ca_pem_path: None,
        })
        .expect("append device-A");
    daemon_a_anchor_inner
        .append_agent(TrustedAgent {
            agent_ura: daemon_b_ura.clone(),
            public_key_b64: BASE64_STANDARD.encode(
                SigningKey::from_bytes(&REALM_B_HUB_SIGNING_SEED)
                    .verifying_key()
                    .to_bytes(),
            ),
            role: TrustAnchorRole::Hub,
            added_at_unix_ms: 1_714_492_800_000,
            origin_realm: Some(REALM_B.to_string()),
            hub_endpoint: Some("in-process-B".to_string()),
            tls_ca_pem_path: None,
        })
        .expect("append daemon B hub");
    let daemon_a_trust = SharedTrustAnchor::new(Arc::new(daemon_a_anchor_inner));
    let daemon_a_keys = Arc::new(FederatedKeyResolver::new(
        daemon_a_trust.clone(),
        None,
        SharedFederatedPeers::default(),
        Some(REALM_A.to_string()),
    ));
    let (daemon_a, _) = hub_service(REALM_A, daemon_a_trust, daemon_a_keys).await;
    let daemon_a = Arc::new(daemon_a);

    // ── Daemon B: empty trust + federated_peers → daemon A ──
    let federation_client: Arc<dyn FederationClient> = Arc::new(InProcessForwarder {
        peer: Arc::clone(&daemon_a),
    });
    let mut peers = std::collections::BTreeMap::new();
    peers.insert(REALM_A.to_string(), PEER_HUB_URA.to_string());
    let peers_cell = SharedFederatedPeers::new(peers);

    let daemon_b_trust = SharedTrustAnchor::new(Arc::new(RealmTrustAnchor::default()));
    let daemon_b_keys = Arc::new(
        FederatedKeyResolver::new(
            daemon_b_trust.clone(),
            Some(Arc::clone(&federation_client)),
            peers_cell,
            Some(REALM_B.to_string()),
        )
        .with_hub_signer(test_hub_signer(REALM_B)),
    );
    let (daemon_b, catalog_b) = hub_service(REALM_B, daemon_b_trust, daemon_b_keys).await;
    let signed_descriptor = descriptor_ref(&catalog_b, &daemon_b_ura, ABILITY_FEDERATION_STATUS);

    // Forge: caller = device-A, but signed with the attacker's key.
    let forged = signed_request(
        DEVICE_A_URA,
        &daemon_b_ura,
        ABILITY_FEDERATION_STATUS,
        &signed_descriptor,
        b"{}",
        &attacker_key,
        [0x55u8; 16],
    );

    let error = expect_canonical_in_band_failure(
        daemon_b.invoke(tonic::Request::new(forged)).await,
        ErrorCode::CallerSignatureInvalid,
        "forged signature must fail canonical caller authentication",
    );
    assert_eq!(error.stage, PbErrorStage::CallerAuthentication as i32);
    assert_eq!(error.security_class, PbSecurityClass::Authentication as i32);
}
