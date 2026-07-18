//! Boot-time factory for the shared `LocalRuntime` instance.
//!
//! Centralises the "build Axon's runtime + install KeyResolver +
//! install LedgerSink" recipe so that:
//!
//!   * production boot (`daemon::invocation::start_daemon_invocation_transport`)
//!     gets the runtime wired the same way every time, and
//!   * integration tests can call the same factory with a tempdir
//!     ledger + a stub trust anchor without duplicating the
//!     plumbing.
//!
//! The runtime returned here owns no `AbilityFn` yet — Phase 3
//! registers those. Phase 2 only ensures the runtime exists and is
//! reachable from the dispatch service.

use std::sync::Arc;
#[cfg(test)]
use std::{collections::HashMap, sync::Mutex};

#[cfg(any(test, feature = "axon-pb"))]
use axon_sdk::invocation::AxonError;
#[cfg(test)]
use axon_sdk::invocation::{
    authority_proof_expected_hash, sha256, AgentIdentity, AuthorityBinding, CalleeSignature,
    DescriptorBoundEnvelope, InvocationAuthorityProof, ReceiptSigningAuthority,
    VerifiedAdmissionPolicy,
};
use axon_sdk::invocation::{
    AxiomBinding, CanonicalReceiptProvider, InvocationLedger, KeyResolver, LedgerSink, LocalRuntime,
};
#[cfg(any(test, feature = "axon-pb"))]
use ed25519_dalek::VerifyingKey;
#[cfg(test)]
use ed25519_dalek::{Signature, Signer, SigningKey, Verifier};

use crate::daemon::identity::local_invocation::CanonicalAdmissionKeyResolver;
#[cfg(feature = "axon-pb")]
use crate::daemon::identity::receipt_signing::load_runtime_signing_authority_providers;
#[cfg(feature = "axon-pb")]
pub use crate::daemon::identity::receipt_signing::ProductionReceiptAuthorityConfig;

/// Immutable runtime admission composition shared by transport dispatch and
/// Axon's signature verifier.
#[cfg(feature = "axon-pb")]
pub struct DaemonRuntimeAdmissionGraph {
    key_resolver: Arc<CanonicalAdmissionKeyResolver>,
    product_policy: Arc<
        crate::daemon::invocation::admission::admission_facade::DaemonProductAdmissionCoordinator,
    >,
}

#[cfg(feature = "axon-pb")]
impl DaemonRuntimeAdmissionGraph {
    fn new(
        key_resolver: Arc<CanonicalAdmissionKeyResolver>,
        product_policy: Arc<
            crate::daemon::invocation::admission::admission_facade::DaemonProductAdmissionCoordinator,
        >,
    ) -> Self {
        Self {
            key_resolver,
            product_policy,
        }
    }

    pub(crate) fn bootstrap_identity_provider(
        &self,
    ) -> Arc<crate::daemon::axon_bridge::runtime_admin::RuntimeBootstrapIdentityProvider> {
        self.key_resolver.bootstrap_identity_provider()
    }

    pub(crate) fn provisional_bootstrap_provider(
        &self,
    ) -> Arc<crate::daemon::axon_bridge::runtime_admin::ProvisionalBootstrapKeyProvider> {
        self.key_resolver.provisional_bootstrap_provider()
    }

    pub(crate) fn product_policy(
        &self,
    ) -> Arc<
        crate::daemon::invocation::admission::admission_facade::DaemonProductAdmissionCoordinator,
    > {
        Arc::clone(&self.product_policy)
    }
}

#[cfg(feature = "axon-pb")]
impl KeyResolver for DaemonRuntimeAdmissionGraph {
    fn resolve(&self, agent_ura: &str) -> Result<VerifyingKey, AxonError> {
        self.key_resolver.resolve(agent_ura)
    }

    fn resolve_all(&self, agent_ura: &str) -> Result<Vec<VerifyingKey>, AxonError> {
        self.key_resolver.resolve_all(agent_ura)
    }
}

#[cfg(feature = "axon-pb")]
struct ProductPolicyCanonicalReceiptProvider {
    receipt_authority: Arc<dyn CanonicalReceiptProvider>,
    product_policy: Arc<
        crate::daemon::invocation::admission::admission_facade::DaemonProductAdmissionCoordinator,
    >,
}

#[async_trait::async_trait]
#[cfg(feature = "axon-pb")]
impl CanonicalReceiptProvider for ProductPolicyCanonicalReceiptProvider {
    fn verify_admission_policy(
        &self,
        envelope: &axon_sdk::invocation::DescriptorBoundEnvelope,
    ) -> Result<axon_sdk::invocation::VerifiedAdmissionPolicy, AxonError> {
        self.product_policy.verify_provider_policy(envelope)
    }

    async fn resolve_signing_authority(
        &self,
        callee: &axon_sdk::invocation::AgentIdentity,
    ) -> Result<Arc<dyn axon_sdk::invocation::ReceiptSigningAuthority>, AxonError> {
        self.receipt_authority
            .resolve_signing_authority(callee)
            .await
    }

    fn resolve_signer_key(&self, signer_ura: &str) -> Result<Option<VerifyingKey>, AxonError> {
        self.receipt_authority.resolve_signer_key(signer_ura)
    }
}

/// One immutable daemon runtime assembly.
///
/// The canonical runtime and the CLI-owned admission graph are constructed
/// together and cannot be replaced independently. Any transport that exposes
/// daemon-owned routes must receive this value rather than a bare
/// [`LocalRuntime`].
#[derive(Clone)]
#[cfg(feature = "axon-pb")]
pub struct DaemonRuntimeAssembly {
    runtime: Arc<LocalRuntime>,
    admission_graph: Arc<DaemonRuntimeAdmissionGraph>,
}

#[cfg(feature = "axon-pb")]
impl DaemonRuntimeAssembly {
    #[must_use]
    pub fn runtime(&self) -> Arc<LocalRuntime> {
        Arc::clone(&self.runtime)
    }

    #[must_use]
    pub(crate) fn admission_graph(&self) -> Arc<DaemonRuntimeAdmissionGraph> {
        Arc::clone(&self.admission_graph)
    }

    /// Bind the daemon's completed product policy facade to the handlers
    /// already installed in this exact runtime.
    ///
    /// Catalog assembly must precede facade assembly because admission
    /// validates descriptors against the live catalog. This explicit
    /// one-writer operation closes that construction cycle before listeners
    /// are published, without introducing a process-global runtime lookup.
    pub fn bind_derived_invocation_admission(
        &self,
        catalog: &crate::daemon::ability::dispatch::AxonAbilityCatalog,
        facade: crate::daemon::invocation::admission::admission_facade::AdmissionFacade,
    ) -> anyhow::Result<()> {
        catalog.bind_derived_invocation_admission(Arc::new(
            crate::daemon::invocation::admission::admission_facade::DaemonDerivedInvocationAdmission::new(
                facade,
                self.admission_graph.product_policy(),
            ),
        ))
    }
}

/// Build the daemon production runtime from persistent owner-bound key-service
/// capabilities and one stable admission graph. No private key enters the CLI
/// process.
#[cfg(feature = "axon-pb")]
pub fn build_production_local_runtime(
    config: ProductionReceiptAuthorityConfig,
    trusted_identities: Arc<dyn KeyResolver>,
) -> Result<DaemonRuntimeAssembly, AxonError> {
    let providers = load_runtime_signing_authority_providers(config)?;
    Ok(assemble_daemon_runtime(
        trusted_identities,
        providers.receipt,
        Some(providers.invocation),
        None,
    ))
}

/// Build the complete daemon runtime assembly from explicit canonical
/// providers.
///
/// Host adapters and integration tests use this constructor when they expose
/// daemon routes but own their trust and receipt providers. The optional
/// ledger is installed during assembly so the returned value is ready before
/// publication.
#[must_use]
#[cfg(feature = "axon-pb")]
pub fn build_daemon_runtime_with_receipt_provider(
    trusted_identities: Arc<dyn KeyResolver>,
    canonical_receipt_provider: Arc<dyn CanonicalReceiptProvider>,
    ledger: Option<Arc<InvocationLedger>>,
) -> DaemonRuntimeAssembly {
    assemble_daemon_runtime(trusted_identities, canonical_receipt_provider, None, ledger)
}

#[cfg(feature = "axon-pb")]
fn assemble_daemon_runtime(
    trusted_identities: Arc<dyn KeyResolver>,
    canonical_receipt_provider: Arc<dyn CanonicalReceiptProvider>,
    invocation_authority_provider: Option<
        Arc<dyn axon_sdk::invocation::InvocationSigningAuthorityProvider>,
    >,
    ledger: Option<Arc<InvocationLedger>>,
) -> DaemonRuntimeAssembly {
    let product_policy = Arc::new(
        crate::daemon::invocation::admission::admission_facade::DaemonProductAdmissionCoordinator::default(),
    );
    let receipt_provider: Arc<dyn CanonicalReceiptProvider> =
        Arc::new(ProductPolicyCanonicalReceiptProvider {
            receipt_authority: canonical_receipt_provider,
            product_policy: Arc::clone(&product_policy),
        });
    let bootstrap_identities = Arc::new(
        crate::daemon::axon_bridge::runtime_admin::RuntimeBootstrapIdentityProvider::default(),
    );
    let provisional_bootstrap = Arc::new(
        crate::daemon::axon_bridge::runtime_admin::ProvisionalBootstrapKeyProvider::default(),
    );
    let admission_key_resolver = Arc::new(CanonicalAdmissionKeyResolver::new(
        trusted_identities,
        bootstrap_identities,
        provisional_bootstrap,
        Arc::clone(&receipt_provider),
    ));
    let admission_graph = Arc::new(DaemonRuntimeAdmissionGraph::new(
        admission_key_resolver,
        product_policy,
    ));
    let runtime_resolver: Arc<dyn KeyResolver> = admission_graph.clone();
    let runtime = LocalRuntime::new_with_authority_providers(
        runtime_resolver,
        invocation_authority_provider,
        receipt_provider,
    );
    install_ledger_sink(&runtime, ledger);
    DaemonRuntimeAssembly {
        runtime,
        admission_graph,
    }
}

/// Build a runtime from an explicit trust resolver and receipt provider.
///
/// Bounded probes use this canonical-only seam when they do not expose daemon
/// routes. Both dependencies are mandatory; feature-enabled daemon transports
/// must use the immutable daemon assembly constructor instead.
pub fn build_local_runtime_with_receipt_provider(
    trusted_identities: Arc<dyn KeyResolver>,
    receipt_provider: Arc<dyn CanonicalReceiptProvider>,
) -> Arc<LocalRuntime> {
    let bootstrap_identities = Arc::new(
        crate::daemon::axon_bridge::runtime_admin::RuntimeBootstrapIdentityProvider::default(),
    );
    let provisional_bootstrap = Arc::new(
        crate::daemon::axon_bridge::runtime_admin::ProvisionalBootstrapKeyProvider::default(),
    );
    let resolver: Arc<dyn KeyResolver> = Arc::new(CanonicalAdmissionKeyResolver::new(
        trusted_identities,
        bootstrap_identities,
        provisional_bootstrap,
        Arc::clone(&receipt_provider),
    ));
    LocalRuntime::new_with_canonical_receipt_provider(resolver, receipt_provider)
}

/// Construct an explicit canonical-only `Arc<LocalRuntime>` for unit tests.
///
/// Test wiring includes:
///
/// - the required caller-key resolver supplied by the test;
/// - an explicit test receipt provider;
/// - the ledger sink backed by `InvocationLedger` (so every
///   terminal invocation persists into `<ledger_dir>/invocations.redb`
///   without the dispatch arm needing to manually build a record).
///
/// This fixture intentionally has no daemon product-policy coordinator. Tests
/// for daemon transport admission must use
/// [`build_test_daemon_runtime_assembly`]. Production boot must use
/// [`build_production_local_runtime`].
#[cfg(test)]
#[must_use]
pub fn build_local_runtime(
    key_resolver: Arc<dyn KeyResolver>,
    ledger: Option<Arc<InvocationLedger>>,
) -> Arc<LocalRuntime> {
    let receipt_provider = ephemeral_test_canonical_receipt_provider();
    let runtime = build_local_runtime_with_receipt_provider(key_resolver, receipt_provider);
    install_ledger_sink(&runtime, ledger);
    runtime
}

#[cfg(all(test, feature = "axon-pb"))]
#[must_use]
/// Construct the complete downstream daemon admission assembly for transport
/// tests that stage product policy before entering the canonical runtime.
pub(crate) fn build_test_daemon_runtime_assembly(
    key_resolver: Arc<dyn KeyResolver>,
    ledger: Option<Arc<InvocationLedger>>,
) -> DaemonRuntimeAssembly {
    build_daemon_runtime_with_receipt_provider(
        key_resolver,
        ephemeral_test_canonical_receipt_provider(),
        ledger,
    )
}

#[cfg(test)]
#[must_use]
pub(crate) fn rejecting_test_key_resolver() -> Arc<dyn KeyResolver> {
    Arc::new(RejectingTestKeyResolver)
}

#[cfg(test)]
struct RejectingTestKeyResolver;

#[cfg(test)]
impl KeyResolver for RejectingTestKeyResolver {
    fn resolve(&self, agent_ura: &str) -> Result<VerifyingKey, AxonError> {
        Err(AxonError::permission_denied(format!(
            "test_key_not_configured:{agent_ura}"
        )))
    }
}

#[cfg(test)]
pub(crate) fn ephemeral_test_canonical_receipt_provider() -> Arc<dyn CanonicalReceiptProvider> {
    Arc::new(EphemeralTestCanonicalReceiptProvider::default())
}

#[cfg(test)]
#[must_use]
pub(crate) fn ephemeral_test_receipt_key_resolver() -> Arc<dyn KeyResolver> {
    Arc::new(EphemeralTestReceiptKeyResolver)
}

#[cfg(test)]
struct EphemeralTestReceiptKeyResolver;

#[cfg(test)]
impl KeyResolver for EphemeralTestReceiptKeyResolver {
    fn resolve(&self, signer_ura: &str) -> Result<VerifyingKey, AxonError> {
        Ok(SigningKey::from_bytes(&sha256(signer_ura.as_bytes())).verifying_key())
    }
}

#[cfg(test)]
#[derive(Default)]
struct EphemeralTestCanonicalReceiptProvider {
    authorities: Mutex<HashMap<AgentIdentity, Arc<dyn ReceiptSigningAuthority>>>,
}

#[cfg(test)]
#[async_trait::async_trait]
impl CanonicalReceiptProvider for EphemeralTestCanonicalReceiptProvider {
    fn verify_admission_policy(
        &self,
        envelope: &DescriptorBoundEnvelope,
    ) -> Result<VerifiedAdmissionPolicy, AxonError> {
        let binding = AuthorityBinding::Self_ {
            principal_ura: envelope.envelope().caller.ura.clone(),
        };
        let mut proof = InvocationAuthorityProof::new(
            "test-self-admission",
            Some(binding.clone()),
            Vec::new(),
            [0u8; 32],
            Some(envelope.envelope().callee.clone()),
            None,
            "easynet-cli.test_self_admission.v1",
        );
        proof.proof_hash = authority_proof_expected_hash(&proof);
        VerifiedAdmissionPolicy::new(envelope, binding, proof)
    }

    async fn resolve_signing_authority(
        &self,
        callee: &AgentIdentity,
    ) -> Result<Arc<dyn ReceiptSigningAuthority>, AxonError> {
        let mut authorities = self
            .authorities
            .lock()
            .map_err(|_| AxonError::internal("ephemeral_test_receipt_authority_lock_poisoned"))?;
        if let Some(authority) = authorities.get(callee) {
            return Ok(Arc::clone(authority));
        }
        let authority: Arc<dyn ReceiptSigningAuthority> = Arc::new(
            EphemeralTestReceiptSigningAuthority::self_signed(callee.clone()),
        );
        authorities.insert(callee.clone(), Arc::clone(&authority));
        Ok(authority)
    }

    fn resolve_signer_key(
        &self,
        signer_ura: &str,
    ) -> Result<Option<ed25519_dalek::VerifyingKey>, AxonError> {
        let authorities = self
            .authorities
            .lock()
            .map_err(|_| AxonError::internal("ephemeral_test_receipt_authority_lock_poisoned"))?;
        Ok(authorities
            .values()
            .find(|authority| authority.signer_identity().ura == signer_ura)
            .map(|authority| authority.verifying_key()))
    }
}

#[cfg(test)]
struct EphemeralTestReceiptSigningAuthority {
    callee_identity: AgentIdentity,
    signing_key: SigningKey,
}

#[cfg(test)]
impl EphemeralTestReceiptSigningAuthority {
    fn self_signed(callee_identity: AgentIdentity) -> Self {
        Self {
            signing_key: SigningKey::from_bytes(&sha256(callee_identity.ura.as_bytes())),
            callee_identity,
        }
    }

    fn verifying_key(&self) -> VerifyingKey {
        self.signing_key.verifying_key()
    }
}

#[cfg(test)]
#[async_trait::async_trait]
impl ReceiptSigningAuthority for EphemeralTestReceiptSigningAuthority {
    fn callee_identity(&self) -> &AgentIdentity {
        &self.callee_identity
    }

    fn signer_identity(&self) -> &AgentIdentity {
        &self.callee_identity
    }

    fn host_attestation(&self) -> &[u8] {
        &[]
    }

    fn verifying_key(&self) -> VerifyingKey {
        self.verifying_key()
    }

    async fn sign_and_verify(
        &self,
        canonical_receipt: &[u8],
    ) -> Result<CalleeSignature, AxonError> {
        let signature: Signature = self.signing_key.sign(canonical_receipt);
        self.verifying_key()
            .verify(canonical_receipt, &signature)
            .map_err(|_| AxonError::internal("cli_test_receipt_signature_self_verify_failed"))?;
        Ok(CalleeSignature {
            algorithm: "ed25519".to_string(),
            signature: signature.to_bytes().to_vec(),
            key_id_hint: "cli-test-runtime".to_string(),
        })
    }
}

/// Install the optional persistence sink. Admission and signing providers are
/// immutable constructor dependencies and are deliberately absent here.
pub fn install_ledger_sink(runtime: &Arc<LocalRuntime>, ledger: Option<Arc<InvocationLedger>>) {
    if let Some(ledger) = ledger {
        runtime.set_ledger_sink(
            LedgerSink::new(ledger)
                .with_invocation_ura(ledger_invocation_ura)
                .with_ability_ura(ledger_route_ura),
        );
    }
}

fn ledger_invocation_ura(invocation_id: &str, binding: &AxiomBinding) -> String {
    crate::core::ura::invocation_record_ura_for_binding(
        &binding.subject.ura,
        &binding.callee.ura,
        &binding.caller.ura,
        invocation_id,
    )
    .unwrap_or_else(|| {
        crate::core::ura::invocation_history_resource_ura(
            "_system",
            "authority.invocations",
            invocation_id,
        )
    })
}

fn ledger_route_ura(ability_name: &str, binding: &AxiomBinding) -> String {
    if let Some(ability_ura) = canonical_ledger_ability_ura(ability_name, binding) {
        return ability_ura;
    }

    // RFC-005: a route names the same canonical `/ability/` URA the owner
    // publishes. Axon's ledger sink passes the daemon registry key here
    // (`liangbing.chat`, `fs.read`, ...), while public Ability URAs
    // store the owner-local name (`chat`, `fs.read`, ...). Project through
    // the CLI URA boundary object before calling Axon's canonical builder;
    // do not duplicate URA grammar in this adapter.
    let callee_public_name =
        crate::core::ura::owner_local_ability_name(&binding.callee.ura, ability_name);
    let caller_public_name =
        crate::core::ura::owner_local_ability_name(&binding.caller.ura, ability_name);

    crate::core::ura::published_route_ura(&binding.callee.ura, &callee_public_name)
        .or_else(|| crate::core::ura::published_route_ura(&binding.caller.ura, &caller_public_name))
        .unwrap_or_else(|| {
            crate::core::ura::hub_ability_ura("_system", &format!("system.{ability_name}"))
        })
}

fn canonical_ledger_ability_ura(ability_name: &str, binding: &AxiomBinding) -> Option<String> {
    let ability_name = ability_name.trim();
    if ability_name.is_empty() {
        return None;
    }
    if axon_sdk::invocation::canonical_ability_descriptor_ref(ability_name).is_ok() {
        if let Ok(ability_ura) = crate::daemon::axon_bridge::descriptor_ref::ability_ura_for_wire(
            &binding.callee.ura,
            ability_name,
        ) {
            return Some(ability_ura);
        }
        if let Ok(ability_ura) = crate::daemon::axon_bridge::descriptor_ref::ability_ura_for_wire(
            &binding.caller.ura,
            ability_name,
        ) {
            return Some(ability_ura);
        }
    }
    if let Ok(selector) = crate::core::ura::AbilitySelector::parse(ability_name) {
        return Some(selector.ability_ura().to_string());
    }
    None
}

#[cfg(test)]
mod tests {
    use super::*;
    use axon_sdk::invocation::axiom::AuthorityBinding;
    use axon_sdk::invocation::{AgentIdentity, CausalContext, SubjectIdentity, UraProfile};

    #[test]
    fn build_local_runtime_requires_explicit_admission_and_allows_no_sink() {
        let rt = build_local_runtime(rejecting_test_key_resolver(), None);
        // Arc strong count > 0 — proves the runtime was built.
        assert!(Arc::strong_count(&rt) >= 1);
    }

    #[test]
    fn ledger_resolvers_use_axon_canonical_ura_helpers() {
        let caller = AgentIdentity::new("easynet:///r/localhost/user/dev", UraProfile::StrictV2);
        let callee = AgentIdentity::new(
            "easynet:///r/localhost/agent/dev.liangbing",
            UraProfile::StrictV2,
        );
        let subject = SubjectIdentity::new("easynet:///r/localhost/user/dev", UraProfile::StrictV2);
        let binding = AxiomBinding {
            caller: caller.clone(),
            callee,
            subject,
            invocation_nonce: [0u8; 16],
            causal: CausalContext::None,
            payload_digest: [0u8; 32],
            callee_signature: None,
            signer_binding: None,
            host_attestation: Vec::new(),
            ability_binding: "liangbing.chat".to_string(),
            authority_binding: AuthorityBinding::Self_ {
                principal_ura: caller.ura.clone(),
            },
        };

        assert_eq!(
            ledger_route_ura("liangbing.chat", &binding),
            "easynet:///r/localhost/ability/dev.liangbing.chat"
        );
        assert_eq!(
            ledger_route_ura(
                "easynet:///r/localhost/ability/dev.liangbing.chat@1.2.3#0000000000000000000000000000000000000000000000000000000000000001!invoke",
                &binding,
            ),
            "easynet:///r/localhost/ability/dev.liangbing.chat"
        );
        assert_eq!(
            ledger_route_ura(
                "easynet:///r/localhost/ability/dev.liangbing.chat",
                &binding,
            ),
            "easynet:///r/localhost/ability/dev.liangbing.chat"
        );
        assert_eq!(
            ledger_invocation_ura("inv_123", &binding),
            "easynet:///r/localhost/resource/dev/invocation/inv_123/history"
        );

        let fallback_caller =
            AgentIdentity::new("easynet:///r/localhost/user/dev", UraProfile::StrictV2);
        let fallback_binding = AxiomBinding {
            caller: fallback_caller.clone(),
            callee: AgentIdentity::new(
                crate::core::ura::hub_ura("localhost"),
                UraProfile::StrictV2,
            ),
            subject: SubjectIdentity::new("easynet:///r/localhost/user/dev", UraProfile::StrictV2),
            invocation_nonce: [0u8; 16],
            causal: CausalContext::None,
            payload_digest: [0u8; 32],
            callee_signature: None,
            signer_binding: None,
            host_attestation: Vec::new(),
            ability_binding: "chat".to_string(),
            authority_binding: AuthorityBinding::Self_ {
                principal_ura: fallback_caller.ura.clone(),
            },
        };
        // RFC-005 removed the device ability *resource* route; the
        // last-resort fallback (neither binding URA publishes the route)
        // now names a hub-owned system ability URA.
        assert_eq!(
            ledger_route_ura("chat", &fallback_binding),
            "easynet:///r/_system/ability/authority.system.chat"
        );
    }
}
