#![allow(dead_code)]

use std::collections::HashMap;
use std::sync::{Arc, Mutex};

use axon_sdk::invocation::axiom::authority_proof_expected_hash;
use axon_sdk::invocation::{
    sha256, AgentIdentity, AuthorityBinding, AxonError, CalleeSignature, CanonicalReceiptProvider,
    DescriptorBoundEnvelope, InvocationAuthorityProof, KeyResolver, LocalRuntime,
    ReceiptSigningAuthority, VerifiedAdmissionPolicy,
};
use ed25519_dalek::{Signer, SigningKey, Verifier, VerifyingKey};

pub struct RejectingKeyResolver;

impl KeyResolver for RejectingKeyResolver {
    fn resolve(&self, agent_ura: &str) -> Result<VerifyingKey, AxonError> {
        Err(AxonError::permission_denied(format!(
            "integration_test_key_not_configured:{agent_ura}"
        )))
    }
}

pub fn rejecting_key_resolver() -> Arc<dyn KeyResolver> {
    Arc::new(RejectingKeyResolver)
}

pub fn rejecting_runtime() -> Arc<LocalRuntime> {
    runtime_with_key_resolver(rejecting_key_resolver())
}

pub fn runtime_with_key_resolver(resolver: Arc<dyn KeyResolver>) -> Arc<LocalRuntime> {
    easynet_cli::daemon::axon_bridge::runtime_factory::build_local_runtime_with_receipt_provider(
        resolver,
        Arc::new(DeterministicReceiptProvider::default()),
    )
}

pub fn daemon_runtime_with_key_resolver(
    resolver: Arc<dyn KeyResolver>,
) -> easynet_cli::daemon::axon_bridge::runtime_factory::DaemonRuntimeAssembly {
    daemon_runtime_with_key_resolver_and_ledger(resolver, None)
}

pub fn daemon_runtime_with_key_resolver_and_ledger(
    resolver: Arc<dyn KeyResolver>,
    ledger: Option<Arc<axon_sdk::invocation::InvocationLedger>>,
) -> easynet_cli::daemon::axon_bridge::runtime_factory::DaemonRuntimeAssembly {
    easynet_cli::daemon::axon_bridge::runtime_factory::build_daemon_runtime_with_receipt_provider(
        resolver,
        Arc::new(DeterministicReceiptProvider::default()),
        ledger,
    )
}

#[derive(Default)]
struct DeterministicReceiptProvider {
    signer_keys: Mutex<HashMap<String, VerifyingKey>>,
}

#[async_trait::async_trait]
impl CanonicalReceiptProvider for DeterministicReceiptProvider {
    fn verify_admission_policy(
        &self,
        envelope: &DescriptorBoundEnvelope,
    ) -> Result<VerifiedAdmissionPolicy, AxonError> {
        let binding = AuthorityBinding::Self_ {
            principal_ura: envelope.envelope().caller.ura.clone(),
        };
        let mut proof = InvocationAuthorityProof::new(
            "integration-test-verified-admission",
            Some(binding.clone()),
            Vec::new(),
            [0u8; 32],
            Some(envelope.envelope().callee.clone()),
            None,
            "easynet-cli.integration-test.canonical_receipt_provider.admission.v1",
        );
        proof.proof_hash = authority_proof_expected_hash(&proof);
        VerifiedAdmissionPolicy::new(envelope, binding, proof)
    }

    async fn resolve_signing_authority(
        &self,
        callee: &AgentIdentity,
    ) -> Result<Arc<dyn ReceiptSigningAuthority>, AxonError> {
        let authority = DeterministicReceiptAuthority {
            callee: callee.clone(),
            signing_key: SigningKey::from_bytes(&sha256(callee.ura.as_bytes())),
        };
        self.signer_keys
            .lock()
            .map_err(|_| AxonError::internal("test_receipt_signer_registry_lock_poisoned"))?
            .insert(callee.ura.clone(), authority.signing_key.verifying_key());
        Ok(Arc::new(authority))
    }

    fn resolve_signer_key(&self, signer_ura: &str) -> Result<Option<VerifyingKey>, AxonError> {
        Ok(self
            .signer_keys
            .lock()
            .map_err(|_| AxonError::internal("test_receipt_signer_registry_lock_poisoned"))?
            .get(signer_ura)
            .copied())
    }
}

struct DeterministicReceiptAuthority {
    callee: AgentIdentity,
    signing_key: SigningKey,
}

#[async_trait::async_trait]
impl ReceiptSigningAuthority for DeterministicReceiptAuthority {
    fn callee_identity(&self) -> &AgentIdentity {
        &self.callee
    }

    fn signer_identity(&self) -> &AgentIdentity {
        &self.callee
    }

    fn host_attestation(&self) -> &[u8] {
        &[]
    }

    fn verifying_key(&self) -> VerifyingKey {
        self.signing_key.verifying_key()
    }

    async fn sign_and_verify(
        &self,
        canonical_receipt: &[u8],
    ) -> Result<CalleeSignature, AxonError> {
        let signature = self.signing_key.sign(canonical_receipt);
        self.signing_key
            .verifying_key()
            .verify(canonical_receipt, &signature)
            .map_err(|_| AxonError::internal("test_receipt_signature_self_verify_failed"))?;
        Ok(CalleeSignature {
            algorithm: "ed25519".to_string(),
            signature: signature.to_bytes().to_vec(),
            key_id_hint: "integration-test-receipt".to_string(),
        })
    }
}
