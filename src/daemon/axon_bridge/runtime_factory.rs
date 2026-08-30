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

#[cfg(test)]
use std::{collections::HashMap, sync::Mutex};
use std::{path::PathBuf, sync::Arc};

#[cfg(feature = "axon-pb")]
use axon_sdk::invocation::persistence::PersistentLog;
#[cfg(any(test, feature = "axon-pb"))]
use axon_sdk::invocation::AxonError;
#[cfg(test)]
use axon_sdk::invocation::{
    authority_proof_expected_hash, canonical_host_attestation_bytes, sha256, AgentIdentity,
    AuthorityBinding, AuthorityEvidence, AuthorityOrBootstrap, AuthorityRelation, CalleeSignature,
    DescriptorBoundEnvelope, InvocationAuthorityProof, ReceiptSigningAuthority, UraProfile,
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

#[derive(Debug, Clone, PartialEq, Eq)]
#[cfg(feature = "axon-pb")]
pub struct RuntimePersistenceConfig {
    log_dir: PathBuf,
}

#[cfg(feature = "axon-pb")]
impl RuntimePersistenceConfig {
    #[must_use]
    pub fn persistent(log_dir: impl Into<PathBuf>) -> Self {
        Self {
            log_dir: log_dir.into(),
        }
    }

    #[must_use]
    pub fn log_dir(&self) -> &std::path::Path {
        &self.log_dir
    }

    fn into_persistent_log(self) -> PersistentLog {
        PersistentLog::new(Some(self.log_dir))
    }
}

#[cfg(all(test, feature = "axon-pb"))]
pub(crate) fn isolated_test_runtime_persistence(label: &str) -> RuntimePersistenceConfig {
    static RUNTIME_PERSISTENCE_SEQ: std::sync::atomic::AtomicU64 =
        std::sync::atomic::AtomicU64::new(0);
    let sanitized_label: String = label
        .chars()
        .map(|ch| {
            if ch.is_ascii_alphanumeric() || ch == '-' || ch == '_' {
                ch
            } else {
                '-'
            }
        })
        .collect();
    let seq = RUNTIME_PERSISTENCE_SEQ.fetch_add(1, std::sync::atomic::Ordering::Relaxed);
    let dir = std::env::temp_dir().join(format!(
        "easynet-daemon-runtime-{}-{}-{}",
        std::process::id(),
        sanitized_label,
        seq
    ));
    let _ = std::fs::create_dir_all(&dir);
    RuntimePersistenceConfig::persistent(dir)
}

/// Immutable runtime admission composition shared by transport dispatch and
/// Axon's signature verifier.
#[cfg(feature = "axon-pb")]
pub struct DaemonRuntimeAdmissionGraph {
    key_resolver: Arc<CanonicalAdmissionKeyResolver>,
    runtime_admission: Arc<
        crate::daemon::invocation::admission::admission_facade::DaemonRuntimeAdmissionCoordinator,
    >,
}

#[cfg(feature = "axon-pb")]
impl DaemonRuntimeAdmissionGraph {
    fn new(
        key_resolver: Arc<CanonicalAdmissionKeyResolver>,
        runtime_admission: Arc<
            crate::daemon::invocation::admission::admission_facade::DaemonRuntimeAdmissionCoordinator,
        >,
    ) -> Self {
        Self {
            key_resolver,
            runtime_admission,
        }
    }

    pub(crate) fn bootstrap_identity_provider(
        &self,
    ) -> Arc<crate::daemon::axon_bridge::runtime_admin::RuntimeBootstrapIdentityProvider> {
        self.key_resolver.bootstrap_identity_provider()
    }

    pub(crate) fn bootstrap_candidate_provider(
        &self,
    ) -> Arc<crate::daemon::axon_bridge::runtime_admin::BootstrapCandidateKeyProvider> {
        self.key_resolver.bootstrap_candidate_provider()
    }

    pub(crate) fn runtime_admission(
        &self,
    ) -> Arc<
        crate::daemon::invocation::admission::admission_facade::DaemonRuntimeAdmissionCoordinator,
    > {
        Arc::clone(&self.runtime_admission)
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

    fn resolve_signature_keys(
        &self,
        agent_ura: &str,
        key_id_hint: &str,
    ) -> Result<Vec<VerifyingKey>, AxonError> {
        self.key_resolver
            .resolve_signature_keys(agent_ura, key_id_hint)
    }
}

#[cfg(feature = "axon-pb")]
struct RuntimeAdmissionCanonicalReceiptProvider {
    receipt_authority: Arc<dyn CanonicalReceiptProvider>,
    runtime_admission: Arc<
        crate::daemon::invocation::admission::admission_facade::DaemonRuntimeAdmissionCoordinator,
    >,
}

#[async_trait::async_trait]
#[cfg(feature = "axon-pb")]
impl CanonicalReceiptProvider for RuntimeAdmissionCanonicalReceiptProvider {
    fn verify_admission_policy(
        &self,
        envelope: &axon_sdk::invocation::DescriptorBoundEnvelope,
    ) -> Result<axon_sdk::invocation::VerifiedAdmissionPolicy, AxonError> {
        self.runtime_admission.verify_provider_policy(envelope)
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
    invocation_verification_keys: Option<
        Arc<dyn crate::daemon::identity::receipt_signing::InvocationVerificationKeyProvider>,
    >,
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

    pub(crate) fn invocation_verification_keys(
        &self,
    ) -> Option<Arc<dyn crate::daemon::identity::receipt_signing::InvocationVerificationKeyProvider>>
    {
        self.invocation_verification_keys.clone()
    }

    /// Bind the daemon's completed runtime admission facade to the handlers
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
                self.admission_graph.runtime_admission(),
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
    runtime_persistence: RuntimePersistenceConfig,
) -> Result<DaemonRuntimeAssembly, AxonError> {
    let providers = load_runtime_signing_authority_providers(config)?;
    Ok(assemble_daemon_runtime(
        trusted_identities,
        providers.receipt,
        Some(providers.invocation),
        Some(providers.invocation_verification),
        runtime_persistence,
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
    runtime_persistence: RuntimePersistenceConfig,
    ledger: Option<Arc<InvocationLedger>>,
) -> DaemonRuntimeAssembly {
    assemble_daemon_runtime(
        trusted_identities,
        canonical_receipt_provider,
        None,
        None,
        runtime_persistence,
        ledger,
    )
}

#[cfg(feature = "axon-pb")]
fn assemble_daemon_runtime(
    trusted_identities: Arc<dyn KeyResolver>,
    canonical_receipt_provider: Arc<dyn CanonicalReceiptProvider>,
    invocation_authority_provider: Option<
        Arc<dyn axon_sdk::invocation::InvocationSigningAuthorityProvider>,
    >,
    invocation_verification_keys: Option<
        Arc<dyn crate::daemon::identity::receipt_signing::InvocationVerificationKeyProvider>,
    >,
    runtime_persistence: RuntimePersistenceConfig,
    ledger: Option<Arc<InvocationLedger>>,
) -> DaemonRuntimeAssembly {
    let runtime_admission = Arc::new(
        crate::daemon::invocation::admission::admission_facade::DaemonRuntimeAdmissionCoordinator::default(),
    );
    let receipt_provider: Arc<dyn CanonicalReceiptProvider> =
        Arc::new(RuntimeAdmissionCanonicalReceiptProvider {
            receipt_authority: canonical_receipt_provider,
            runtime_admission: Arc::clone(&runtime_admission),
        });
    let bootstrap_identities = Arc::new(
        crate::daemon::axon_bridge::runtime_admin::RuntimeBootstrapIdentityProvider::default(),
    );
    let bootstrap_candidate = Arc::new(
        crate::daemon::axon_bridge::runtime_admin::BootstrapCandidateKeyProvider::default(),
    );
    let mut admission_key_resolver = CanonicalAdmissionKeyResolver::new(
        trusted_identities,
        bootstrap_identities,
        bootstrap_candidate,
        Arc::clone(&receipt_provider),
    );
    if let Some(provider) = invocation_verification_keys.as_ref() {
        admission_key_resolver =
            admission_key_resolver.with_invocation_verification_keys(Arc::clone(provider));
    }
    let admission_key_resolver = Arc::new(admission_key_resolver);
    let admission_graph = Arc::new(DaemonRuntimeAdmissionGraph::new(
        admission_key_resolver,
        runtime_admission,
    ));
    let runtime_resolver: Arc<dyn KeyResolver> = admission_graph.clone();
    let runtime_persistence = Arc::new(runtime_persistence.into_persistent_log());
    let runtime = LocalRuntime::new_with_authority_providers_and_persistence(
        runtime_resolver,
        invocation_authority_provider,
        receipt_provider,
        runtime_persistence,
    );
    install_ledger_sink(&runtime, ledger);
    DaemonRuntimeAssembly {
        runtime,
        admission_graph,
        invocation_verification_keys,
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
    let bootstrap_candidate = Arc::new(
        crate::daemon::axon_bridge::runtime_admin::BootstrapCandidateKeyProvider::default(),
    );
    let resolver: Arc<dyn KeyResolver> = Arc::new(CanonicalAdmissionKeyResolver::new(
        trusted_identities,
        bootstrap_identities,
        bootstrap_candidate,
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
/// This fixture intentionally has no daemon runtime-admission coordinator. Tests
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
/// tests that stage runtime admission before entering the canonical runtime.
pub(crate) fn build_test_daemon_runtime_assembly(
    key_resolver: Arc<dyn KeyResolver>,
    runtime_persistence: RuntimePersistenceConfig,
    ledger: Option<Arc<InvocationLedger>>,
) -> DaemonRuntimeAssembly {
    build_daemon_runtime_with_receipt_provider(
        key_resolver,
        ephemeral_test_canonical_receipt_provider(),
        runtime_persistence,
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
fn sponsor_device_identity_for_system_agent(agent_ura: &str) -> Option<AgentIdentity> {
    let parsed = crate::core::ura::parse_ura(agent_ura).ok()?;
    let (device_id, _agent_id) = parsed.device_agent_ids()?;
    let device_ura = crate::core::ura::device_ura(&parsed.realm, device_id);
    Some(AgentIdentity::new(device_ura, UraProfile::StrictV2))
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
        let binding = AuthorityOrBootstrap::Binding(AuthorityBinding {
            authority: AgentIdentity::new(
                envelope.envelope().caller.ura.clone(),
                UraProfile::StrictV2,
            ),
            relation: AuthorityRelation::Self_,
            evidence: AuthorityEvidence::Identity,
        });
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
            EphemeralTestReceiptSigningAuthority::for_callee(callee.clone()),
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
    signer_identity: AgentIdentity,
    signing_key: SigningKey,
    host_attestation: Vec<u8>,
}

#[cfg(test)]
impl EphemeralTestReceiptSigningAuthority {
    fn for_callee(callee_identity: AgentIdentity) -> Self {
        if let Some(signer_identity) =
            sponsor_device_identity_for_system_agent(&callee_identity.ura)
        {
            return Self::hosted(callee_identity, signer_identity);
        }
        Self::self_signed(callee_identity)
    }

    fn self_signed(callee_identity: AgentIdentity) -> Self {
        Self {
            signing_key: SigningKey::from_bytes(&sha256(callee_identity.ura.as_bytes())),
            signer_identity: callee_identity.clone(),
            callee_identity,
            host_attestation: Vec::new(),
        }
    }

    fn hosted(callee_identity: AgentIdentity, signer_identity: AgentIdentity) -> Self {
        let signing_key = SigningKey::from_bytes(&sha256(signer_identity.ura.as_bytes()));
        let attestation_bytes =
            canonical_host_attestation_bytes(&callee_identity.ura, &signer_identity.ura);
        let signature: Signature = signing_key.sign(&attestation_bytes);
        Self {
            callee_identity,
            signer_identity,
            signing_key,
            host_attestation: signature.to_bytes().to_vec(),
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
        &self.signer_identity
    }

    fn host_attestation(&self) -> &[u8] {
        &self.host_attestation
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
        panic!(
            "LedgerSink cannot derive invocation record URA from binding subject=`{}` callee=`{}` caller=`{}` invocation_id=`{}`",
            binding.subject.ura, binding.callee.ura, binding.caller.ura, invocation_id
        )
    })
}

fn ledger_route_ura(ability_name: &str, binding: &AxiomBinding) -> String {
    if let Some(ability_ura) = callee_ledger_ability_ura(ability_name, binding) {
        return ability_ura;
    }

    panic!(
        "LedgerSink cannot derive ability URA from binding callee=`{}` caller=`{}` ability=`{}`",
        binding.callee.ura, binding.caller.ura, ability_name
    )
}

fn callee_ledger_ability_ura(ability_name: &str, binding: &AxiomBinding) -> Option<String> {
    crate::daemon::axon_bridge::descriptor_ref::ability_ura_for_wire(
        &binding.callee.ura,
        ability_name,
    )
    .ok()
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

    #[tokio::test]
    async fn ephemeral_receipt_provider_uses_sponsor_device_for_system_agent() {
        let provider = ephemeral_test_canonical_receipt_provider();
        let callee = AgentIdentity::new(
            "easynet:///r/acme/agent/device.edge-01.runtime-governance",
            UraProfile::StrictV2,
        );

        let authority = provider
            .resolve_signing_authority(&callee)
            .await
            .expect("SystemAgent test receipt authority resolves");

        assert_eq!(authority.callee_identity().ura, callee.ura);
        assert_eq!(
            authority.signer_identity().ura,
            "easynet:///r/acme/device/edge-01"
        );
        let signer_key = provider
            .resolve_signer_key("easynet:///r/acme/device/edge-01")
            .expect("test receipt signer key resolves")
            .expect("sponsor Device signer key is visible");
        axon_sdk::invocation::verify_host_attestation(
            &callee.ura,
            "easynet:///r/acme/device/edge-01",
            authority.host_attestation(),
            &signer_key,
        )
        .expect("host attestation verifies against sponsor Device key");
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
            authority_binding: AuthorityOrBootstrap::Binding(AuthorityBinding {
                authority: caller.clone(),
                relation: AuthorityRelation::Self_,
                evidence: AuthorityEvidence::Identity,
            }),
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
            ability_binding: "system.chat".to_string(),
            authority_binding: AuthorityOrBootstrap::Binding(AuthorityBinding {
                authority: fallback_caller.clone(),
                relation: AuthorityRelation::Self_,
                evidence: AuthorityEvidence::Identity,
            }),
        };
        assert_eq!(
            ledger_route_ura("system.chat", &fallback_binding),
            "easynet:///r/localhost/ability/authority.system.chat"
        );
    }

    #[test]
    #[should_panic(expected = "LedgerSink cannot derive ability URA from binding")]
    fn ledger_route_resolver_rejects_authority_bare_ability_instead_of_system_fallback() {
        let caller = AgentIdentity::new("easynet:///r/localhost/user/dev", UraProfile::StrictV2);
        let binding = AxiomBinding {
            caller: caller.clone(),
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
            authority_binding: AuthorityOrBootstrap::Binding(AuthorityBinding {
                authority: caller.clone(),
                relation: AuthorityRelation::Self_,
                evidence: AuthorityEvidence::Identity,
            }),
        };

        let _ = ledger_route_ura("chat", &binding);
    }

    #[test]
    #[should_panic(expected = "LedgerSink cannot derive ability URA from binding")]
    fn ledger_route_resolver_rejects_caller_owned_explicit_ability() {
        let caller = AgentIdentity::new(
            "easynet:///r/localhost/agent/dev.caller",
            UraProfile::StrictV2,
        );
        let binding = AxiomBinding {
            caller: caller.clone(),
            callee: AgentIdentity::new(
                "easynet:///r/localhost/agent/dev.callee",
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
            authority_binding: AuthorityOrBootstrap::Binding(AuthorityBinding {
                authority: caller.clone(),
                relation: AuthorityRelation::Self_,
                evidence: AuthorityEvidence::Identity,
            }),
        };
        let caller_ability =
            crate::core::ura::owner_ability_ura(&binding.caller.ura, "chat").expect("ability URA");

        let _ = ledger_route_ura(&caller_ability, &binding);
    }

    #[test]
    #[should_panic(expected = "LedgerSink cannot derive ability URA from binding")]
    fn ledger_route_resolver_rejects_caller_owned_descriptor_ref() {
        let caller = AgentIdentity::new(
            "easynet:///r/localhost/agent/dev.caller",
            UraProfile::StrictV2,
        );
        let binding = AxiomBinding {
            caller: caller.clone(),
            callee: AgentIdentity::new(
                "easynet:///r/localhost/agent/dev.callee",
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
            authority_binding: AuthorityOrBootstrap::Binding(AuthorityBinding {
                authority: caller.clone(),
                relation: AuthorityRelation::Self_,
                evidence: AuthorityEvidence::Identity,
            }),
        };
        let caller_ability =
            crate::core::ura::owner_ability_ura(&binding.caller.ura, "chat").expect("ability URA");
        let caller_descriptor_ref = format!("{caller_ability}@1.0.0#{}!invoke", "11".repeat(32));

        let _ = ledger_route_ura(&caller_descriptor_ref, &binding);
    }

    #[test]
    #[should_panic(expected = "LedgerSink cannot derive ability URA from binding")]
    fn ledger_route_resolver_rejects_unowned_route_instead_of_system_fallback() {
        let caller = AgentIdentity::new("not-a-canonical-caller", UraProfile::StrictV2);
        let binding = AxiomBinding {
            caller: caller.clone(),
            callee: AgentIdentity::new("not-a-canonical-callee", UraProfile::StrictV2),
            subject: SubjectIdentity::new("not-a-canonical-subject", UraProfile::StrictV2),
            invocation_nonce: [0u8; 16],
            causal: CausalContext::None,
            payload_digest: [0u8; 32],
            callee_signature: None,
            signer_binding: None,
            host_attestation: Vec::new(),
            ability_binding: "chat".to_string(),
            authority_binding: AuthorityOrBootstrap::Binding(AuthorityBinding {
                authority: caller.clone(),
                relation: AuthorityRelation::Self_,
                evidence: AuthorityEvidence::Identity,
            }),
        };

        let _ = ledger_route_ura("chat", &binding);
    }

    #[test]
    #[should_panic(expected = "LedgerSink cannot derive invocation record URA from binding")]
    fn ledger_invocation_resolver_rejects_unowned_record_instead_of_system_fallback() {
        let caller = AgentIdentity::new("not-a-canonical-caller", UraProfile::StrictV2);
        let binding = AxiomBinding {
            caller: caller.clone(),
            callee: AgentIdentity::new("not-a-canonical-callee", UraProfile::StrictV2),
            subject: SubjectIdentity::new("not-a-canonical-subject", UraProfile::StrictV2),
            invocation_nonce: [0u8; 16],
            causal: CausalContext::None,
            payload_digest: [0u8; 32],
            callee_signature: None,
            signer_binding: None,
            host_attestation: Vec::new(),
            ability_binding: "chat".to_string(),
            authority_binding: AuthorityOrBootstrap::Binding(AuthorityBinding {
                authority: caller.clone(),
                relation: AuthorityRelation::Self_,
                evidence: AuthorityEvidence::Identity,
            }),
        };

        let _ = ledger_invocation_ura("inv_123", &binding);
    }
}
