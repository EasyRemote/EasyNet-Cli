//! Daemon-custodied runtime signing authorities.
//!
//! This module adapts owner-bound key-service capabilities to Axon's receipt
//! and child-Invocation authority contracts. Both projections share the same
//! owner inventory and sign-only capability; no private key enters the daemon.
//! Runtime assembly remains in `daemon::axon_bridge::runtime_factory`.

use std::collections::HashMap;
use std::fmt;
use std::sync::Arc;

use axon_sdk::invocation::{
    canonical_host_attestation_bytes, AgentIdentity, AxonError, CalleeSignature, CallerSignature,
    CanonicalReceiptProvider, DescriptorBoundEnvelope, InvocationSigningAuthority,
    InvocationSigningAuthorityProvider, ReceiptSigningAuthority, UraProfile,
    VerifiedAdmissionPolicy,
};
#[cfg(test)]
use ed25519_dalek::{Signature, SIGNATURE_LENGTH};
use ed25519_dalek::{Verifier as _, VerifyingKey};

use super::self_identity::{CanonicalSigner, RuntimeSigningIdentity};
use crate::daemon::ability::dispatch::{HostedAgentAuthorityInventory, HostedAgentAuthorityLease};
use crate::daemon::axon_bridge::proof_owner::descriptor_bound_canonical_bytes;

/// Owner inventory required to build a production receipt-signing runtime.
/// `_system.local` is always added by the factory and cannot be omitted.
#[derive(Clone, Default)]
pub struct ProductionReceiptAuthorityConfig {
    self_signed_owner_uras: Vec<String>,
    hosted_agent_device_ura: Option<String>,
    hosted_agent_inventory: Option<Arc<dyn HostedAgentAuthorityInventory>>,
}

impl fmt::Debug for ProductionReceiptAuthorityConfig {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("ProductionReceiptAuthorityConfig")
            .field("self_signed_owner_uras", &self.self_signed_owner_uras)
            .field("hosted_agent_device_ura", &self.hosted_agent_device_ura)
            .field(
                "has_hosted_agent_inventory",
                &self.hosted_agent_inventory.is_some(),
            )
            .finish()
    }
}

impl ProductionReceiptAuthorityConfig {
    pub fn new(self_signed_owner_uras: impl IntoIterator<Item = String>) -> Self {
        Self {
            self_signed_owner_uras: self_signed_owner_uras.into_iter().collect(),
            hosted_agent_device_ura: None,
            hosted_agent_inventory: None,
        }
    }

    pub fn with_hosted_agent_inventory(
        mut self,
        device_ura: impl Into<String>,
        inventory: Arc<dyn HostedAgentAuthorityInventory>,
    ) -> Self {
        self.hosted_agent_device_ura = Some(device_ura.into());
        self.hosted_agent_inventory = Some(inventory);
        self
    }
}

struct KeyServiceReceiptAuthority {
    callee: AgentIdentity,
    signer: AgentIdentity,
    signer_capability: Arc<dyn CanonicalSigner>,
    verifying_key: VerifyingKey,
    host_attestation: Vec<u8>,
    key_id_hint: String,
    hosted_lease: Option<HostedSigningLease>,
}

struct KeyServiceInvocationAuthority {
    caller: AgentIdentity,
    signer_capability: Arc<dyn CanonicalSigner>,
    key_id_hint: String,
    hosted_lease: Option<HostedSigningLease>,
}

#[derive(Clone)]
struct HostedSigningLease {
    agent_ura: String,
    lease: HostedAgentAuthorityLease,
    inventory: Arc<dyn HostedAgentAuthorityInventory>,
}

impl HostedSigningLease {
    fn validate(&self) -> Result<(), AxonError> {
        if self
            .inventory
            .validate_signing_lease(&self.agent_ura, self.lease)
        {
            Ok(())
        } else {
            Err(
                AxonError::permission_denied("daemon_signing_authority_lease_revoked")
                    .with_context("agent_ura", self.agent_ura.clone()),
            )
        }
    }
}

#[async_trait::async_trait]
impl InvocationSigningAuthority for KeyServiceInvocationAuthority {
    fn owner_identity(&self) -> &AgentIdentity {
        &self.caller
    }

    async fn sign_descriptor_bound_invocation(
        &self,
        envelope: &DescriptorBoundEnvelope,
    ) -> Result<CallerSignature, AxonError> {
        if let Some(lease) = &self.hosted_lease {
            lease.validate()?;
        }
        if envelope.envelope().caller != self.caller {
            return Err(AxonError::permission_denied(
                "daemon_invocation_signer_caller_mismatch",
            ));
        }
        let signature = self
            .signer_capability
            .sign_canonical(&descriptor_bound_canonical_bytes(envelope))
            .await
            .map_err(receipt_identity_error)?;
        if let Some(lease) = &self.hosted_lease {
            lease.validate()?;
        }
        Ok(CallerSignature {
            algorithm: "ed25519".to_string(),
            signature: signature.to_bytes().to_vec(),
            key_id_hint: self.key_id_hint.clone(),
        })
    }
}

#[async_trait::async_trait]
impl ReceiptSigningAuthority for KeyServiceReceiptAuthority {
    fn callee_identity(&self) -> &AgentIdentity {
        &self.callee
    }

    fn signer_identity(&self) -> &AgentIdentity {
        &self.signer
    }

    fn host_attestation(&self) -> &[u8] {
        &self.host_attestation
    }

    fn verifying_key(&self) -> VerifyingKey {
        self.verifying_key
    }

    async fn sign_and_verify(
        &self,
        canonical_receipt: &[u8],
    ) -> Result<CalleeSignature, AxonError> {
        if let Some(lease) = &self.hosted_lease {
            lease.validate()?;
        }
        let signature = self
            .signer_capability
            .sign_canonical(canonical_receipt)
            .await
            .map_err(receipt_identity_error)?;
        if let Some(lease) = &self.hosted_lease {
            lease.validate()?;
        }
        self.verifying_key
            .verify(canonical_receipt, &signature)
            .map_err(|_| AxonError::internal("daemon_receipt_signature_self_verify_failed"))?;
        Ok(CalleeSignature {
            algorithm: "ed25519".to_string(),
            signature: signature.to_bytes().to_vec(),
            key_id_hint: self.key_id_hint.clone(),
        })
    }
}

struct KeyServiceReceiptAuthorityProvider {
    self_signed: HashMap<String, Arc<dyn ReceiptSigningAuthority>>,
    signer_capabilities: HashMap<String, Arc<dyn CanonicalSigner>>,
    hosted_agent_device: Option<AgentIdentity>,
    hosted_agent_inventory: Option<Arc<dyn HostedAgentAuthorityInventory>>,
}

impl KeyServiceReceiptAuthorityProvider {
    fn load(config: ProductionReceiptAuthorityConfig) -> Result<Self, AxonError> {
        let mut owner_uras = config.self_signed_owner_uras;
        owner_uras.push(crate::core::ura::LOCAL_SYSTEM_AGENT_URA.to_string());
        owner_uras.sort();
        owner_uras.dedup();

        let mut self_signed = HashMap::new();
        let mut signer_capabilities = HashMap::new();
        for owner_ura in owner_uras {
            let identity = strict_self_signed_identity(&owner_ura)?;
            let signer: Arc<dyn CanonicalSigner> = Arc::new(
                RuntimeSigningIdentity::load_default(owner_ura.clone())
                    .map_err(receipt_identity_error)?,
            );
            let authority = Arc::new(self_signed_authority(identity, Arc::clone(&signer))?)
                as Arc<dyn ReceiptSigningAuthority>;
            signer_capabilities.insert(owner_ura.clone(), signer);
            self_signed.insert(owner_ura, authority);
        }

        let hosted_agent_device = config
            .hosted_agent_device_ura
            .map(|device_ura| {
                let parsed = crate::core::ura::parse_ura(&device_ura).map_err(|error| {
                    AxonError::invalid_argument(format!(
                        "daemon_receipt_host_device_invalid:{error}"
                    ))
                })?;
                if parsed.kind != crate::core::ura::URAKind::Device {
                    return Err(AxonError::invalid_argument(
                        "daemon_receipt_host_must_be_device",
                    ));
                }
                if !signer_capabilities.contains_key(&device_ura) {
                    return Err(AxonError::invalid_argument(
                        "daemon_receipt_host_signer_not_owner_bound",
                    ));
                }
                strict_identity(&device_ura)
            })
            .transpose()?;
        if config.hosted_agent_inventory.is_some() && hosted_agent_device.is_none() {
            return Err(AxonError::invalid_argument(
                "daemon_receipt_hosted_agents_require_device",
            ));
        }
        if config.hosted_agent_inventory.is_none() && hosted_agent_device.is_some() {
            return Err(AxonError::invalid_argument(
                "daemon_receipt_hosted_inventory_required",
            ));
        }
        Ok(Self {
            self_signed,
            signer_capabilities,
            hosted_agent_device,
            hosted_agent_inventory: config.hosted_agent_inventory,
        })
    }

    async fn hosted_authority(
        &self,
        callee: &AgentIdentity,
        device: &AgentIdentity,
        lease: HostedAgentAuthorityLease,
        inventory: Arc<dyn HostedAgentAuthorityInventory>,
    ) -> Result<Arc<dyn ReceiptSigningAuthority>, AxonError> {
        axon_sdk::invocation::validate_hosted_attestation_authority(&callee.ura, &device.ura)?;
        let signer = self
            .signer_capabilities
            .get(&device.ura)
            .cloned()
            .ok_or_else(|| AxonError::permission_denied("daemon_receipt_host_signer_missing"))?;
        let verifying_key = signer
            .signing_public_key()
            .map_err(receipt_identity_error)?;
        let attestation_bytes = canonical_host_attestation_bytes(&callee.ura, &device.ura);
        let host_attestation = signer
            .sign_canonical(&attestation_bytes)
            .await
            .map_err(receipt_identity_error)?
            .to_bytes()
            .to_vec();
        let authority: Arc<dyn ReceiptSigningAuthority> = Arc::new(KeyServiceReceiptAuthority {
            callee: callee.clone(),
            signer: device.clone(),
            signer_capability: signer,
            verifying_key,
            host_attestation,
            key_id_hint: key_id_hint(&device.ura, &verifying_key),
            hosted_lease: Some(HostedSigningLease {
                agent_ura: callee.ura.clone(),
                lease,
                inventory,
            }),
        });
        Ok(authority)
    }
}

#[async_trait::async_trait]
impl CanonicalReceiptProvider for KeyServiceReceiptAuthorityProvider {
    fn verify_admission_policy(
        &self,
        _envelope: &DescriptorBoundEnvelope,
    ) -> Result<VerifiedAdmissionPolicy, AxonError> {
        // This provider owns key custody only. Daemon runtime assembly wraps
        // it with the CLI product-admission coordinator, which is the sole
        // producer of receipt-bound admission policy.
        Err(AxonError::internal(
            "daemon_product_admission_coordinator_required",
        ))
    }

    async fn resolve_signing_authority(
        &self,
        callee: &AgentIdentity,
    ) -> Result<Arc<dyn ReceiptSigningAuthority>, AxonError> {
        if let Some(authority) = self.self_signed.get(&callee.ura) {
            if authority.callee_identity() != callee {
                return Err(AxonError::permission_denied(
                    "daemon_receipt_callee_profile_mismatch",
                ));
            }
            return Ok(Arc::clone(authority));
        }
        let inventory = self.hosted_agent_inventory.as_ref().ok_or_else(|| {
            AxonError::permission_denied("daemon_receipt_callee_not_owned")
                .with_context("callee_ura", callee.ura.clone())
        })?;
        if callee.profile != UraProfile::StrictV2 {
            return Err(AxonError::permission_denied(
                "daemon_receipt_callee_profile_mismatch",
            ));
        }
        let Some(lease) = inventory.resolve_signing_lease(&callee.ura) else {
            return Err(
                AxonError::permission_denied("daemon_receipt_callee_not_owned")
                    .with_context("callee_ura", callee.ura.clone()),
            );
        };
        let device = self
            .hosted_agent_device
            .as_ref()
            .ok_or_else(|| AxonError::internal("daemon_receipt_hosted_inventory_missing_device"))?;
        self.hosted_authority(callee, device, lease, Arc::clone(inventory))
            .await
    }

    fn resolve_signer_key(&self, signer_ura: &str) -> Result<Option<VerifyingKey>, AxonError> {
        self.signer_capabilities
            .get(signer_ura)
            .map(|signer| signer.signing_public_key().map_err(receipt_identity_error))
            .transpose()
    }
}

#[async_trait::async_trait]
impl InvocationSigningAuthorityProvider for KeyServiceReceiptAuthorityProvider {
    async fn resolve(
        &self,
        caller_ura: &str,
    ) -> Result<Option<Arc<dyn InvocationSigningAuthority>>, AxonError> {
        let signer = if let Some(signer) = self.signer_capabilities.get(caller_ura) {
            Arc::clone(signer)
        } else {
            let Some(inventory) = self.hosted_agent_inventory.as_ref() else {
                return Ok(None);
            };
            let Some(lease) = inventory.resolve_signing_lease(caller_ura) else {
                return Ok(None);
            };
            let Some(device) = self.hosted_agent_device.as_ref() else {
                return Ok(None);
            };
            axon_sdk::invocation::validate_hosted_attestation_authority(caller_ura, &device.ura)?;
            let signer = self
                .signer_capabilities
                .get(&device.ura)
                .cloned()
                .ok_or_else(|| {
                    AxonError::permission_denied("daemon_invocation_host_signer_missing")
                })?;
            let caller = strict_identity(caller_ura)?;
            let verifying_key = signer
                .signing_public_key()
                .map_err(receipt_identity_error)?;
            return Ok(Some(Arc::new(KeyServiceInvocationAuthority {
                caller,
                key_id_hint: key_id_hint(signer.owner_ura(), &verifying_key),
                signer_capability: signer,
                hosted_lease: Some(HostedSigningLease {
                    agent_ura: caller_ura.to_string(),
                    lease,
                    inventory: Arc::clone(inventory),
                }),
            })));
        };
        let caller = self
            .self_signed
            .get(caller_ura)
            .ok_or_else(|| AxonError::permission_denied("daemon_invocation_caller_not_owned"))?
            .callee_identity()
            .clone();
        let verifying_key = signer
            .signing_public_key()
            .map_err(receipt_identity_error)?;
        Ok(Some(Arc::new(KeyServiceInvocationAuthority {
            caller,
            key_id_hint: key_id_hint(signer.owner_ura(), &verifying_key),
            signer_capability: signer,
            hosted_lease: None,
        })))
    }
}

fn self_signed_authority(
    identity: AgentIdentity,
    signer: Arc<dyn CanonicalSigner>,
) -> Result<KeyServiceReceiptAuthority, AxonError> {
    if signer.owner_ura() != identity.ura {
        return Err(AxonError::permission_denied(
            "daemon_receipt_signer_owner_mismatch",
        ));
    }
    let verifying_key = signer
        .signing_public_key()
        .map_err(receipt_identity_error)?;
    Ok(KeyServiceReceiptAuthority {
        callee: identity.clone(),
        signer: identity,
        signer_capability: signer,
        verifying_key,
        host_attestation: Vec::new(),
        key_id_hint: key_id_hint("self", &verifying_key),
        hosted_lease: None,
    })
}

fn strict_self_signed_identity(ura: &str) -> Result<AgentIdentity, AxonError> {
    if ura == crate::core::ura::LOCAL_SYSTEM_AGENT_URA {
        return strict_identity(ura);
    }
    let parsed = crate::core::ura::parse_ura(ura).map_err(|error| {
        AxonError::invalid_argument(format!("daemon_receipt_owner_invalid:{error}"))
    })?;
    if !matches!(
        parsed.kind,
        crate::core::ura::URAKind::Device | crate::core::ura::URAKind::Authority
    ) {
        return Err(AxonError::permission_denied(
            "daemon_receipt_self_signed_owner_kind_invalid",
        ));
    }
    Ok(AgentIdentity::new(ura, UraProfile::StrictV2))
}

fn strict_identity(ura: &str) -> Result<AgentIdentity, AxonError> {
    crate::core::ura::parse_ura(ura).map_err(|error| {
        AxonError::invalid_argument(format!("daemon_receipt_owner_invalid:{error}"))
    })?;
    Ok(AgentIdentity::new(ura, UraProfile::StrictV2))
}

fn key_id_hint(owner_ura: &str, verifying_key: &VerifyingKey) -> String {
    format!(
        "keyring:{}:{}",
        owner_ura,
        &hex::encode(verifying_key.to_bytes())[..16]
    )
}

fn receipt_identity_error(error: impl std::fmt::Display) -> AxonError {
    AxonError::unavailable(format!("daemon_receipt_identity_unavailable:{error}"))
}

pub(crate) struct RuntimeSigningAuthorityProviders {
    pub invocation: Arc<dyn InvocationSigningAuthorityProvider>,
    pub receipt: Arc<dyn CanonicalReceiptProvider>,
}

pub(crate) fn load_runtime_signing_authority_providers(
    config: ProductionReceiptAuthorityConfig,
) -> Result<RuntimeSigningAuthorityProviders, AxonError> {
    let provider = Arc::new(KeyServiceReceiptAuthorityProvider::load(config)?);
    let invocation: Arc<dyn InvocationSigningAuthorityProvider> = provider.clone();
    let receipt: Arc<dyn CanonicalReceiptProvider> = provider;
    Ok(RuntimeSigningAuthorityProviders {
        invocation,
        receipt,
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::daemon::identity::self_identity::{
        InMemoryVault, SelfIdentity, TestCanonicalSigner,
    };
    use crate::daemon::keyring::{MasterKeySource, Vault};
    use std::collections::HashSet;
    use std::sync::Mutex;

    fn verify_authority_signature(
        authority: &dyn ReceiptSigningAuthority,
        canonical_receipt: &[u8],
        signature: &CalleeSignature,
    ) -> Result<(), AxonError> {
        if signature.algorithm != "ed25519" {
            return Err(AxonError::invalid_argument(
                "test_receipt_signature_algorithm_invalid",
            ));
        }
        let bytes: [u8; SIGNATURE_LENGTH] =
            signature.signature.as_slice().try_into().map_err(|_| {
                AxonError::invalid_argument("test_receipt_signature_length_invalid")
            })?;
        authority
            .verifying_key()
            .verify(canonical_receipt, &Signature::from_bytes(&bytes))
            .map_err(|_| AxonError::invalid_argument("test_receipt_signature_invalid"))
    }

    #[derive(Default)]
    struct TestHostedAgentInventory {
        state: Mutex<(HashSet<String>, u64)>,
    }

    impl HostedAgentAuthorityInventory for TestHostedAgentInventory {
        fn resolve_signing_lease(&self, agent_ura: &str) -> Option<HostedAgentAuthorityLease> {
            let state = self.state.lock().ok()?;
            state
                .0
                .contains(agent_ura)
                .then_some(HostedAgentAuthorityLease::for_generation(state.1))
        }

        fn validate_signing_lease(
            &self,
            agent_ura: &str,
            lease: HostedAgentAuthorityLease,
        ) -> bool {
            self.state
                .lock()
                .map(|state| state.1 == lease.generation() && state.0.contains(agent_ura))
                .unwrap_or(false)
        }
    }

    impl TestHostedAgentInventory {
        fn revoke(&self, agent_ura: &str) {
            let mut state = self.state.lock().unwrap();
            state.0.remove(agent_ura);
            state.1 = state.1.wrapping_add(1);
        }
    }

    fn test_device_provider() -> KeyServiceReceiptAuthorityProvider {
        test_device_provider_with_inventory().0
    }

    fn test_device_provider_with_inventory() -> (
        KeyServiceReceiptAuthorityProvider,
        Arc<TestHostedAgentInventory>,
    ) {
        let device_ura = "easynet:///r/acme/device/edge-01";
        let signer: Arc<dyn CanonicalSigner> =
            Arc::new(TestCanonicalSigner::new(device_ura, [0x73; 32]));
        let identity = strict_identity(device_ura).unwrap();
        let authority =
            Arc::new(self_signed_authority(identity.clone(), Arc::clone(&signer)).unwrap())
                as Arc<dyn ReceiptSigningAuthority>;
        let inventory = Arc::new(TestHostedAgentInventory::default());
        inventory
            .state
            .lock()
            .unwrap()
            .0
            .insert("easynet:///r/acme/agent/alice.worker".to_string());
        (
            KeyServiceReceiptAuthorityProvider {
                self_signed: HashMap::from([(device_ura.to_string(), authority)]),
                signer_capabilities: HashMap::from([(device_ura.to_string(), signer)]),
                hosted_agent_device: Some(identity),
                hosted_agent_inventory: Some(inventory.clone()),
            },
            inventory,
        )
    }

    #[tokio::test]
    async fn hosted_receipt_signer_is_owner_bound_and_resolver_visible() {
        let provider = test_device_provider();
        let callee = strict_identity("easynet:///r/acme/agent/alice.worker").unwrap();
        let authority = CanonicalReceiptProvider::resolve_signing_authority(&provider, &callee)
            .await
            .unwrap();
        let signer_ura = "easynet:///r/acme/device/edge-01";
        let resolver_key = provider
            .resolve_signer_key(signer_ura)
            .unwrap()
            .expect("host signer public projection");

        assert_eq!(authority.callee_identity(), &callee);
        assert_eq!(authority.signer_identity().ura, signer_ura);
        assert_eq!(authority.verifying_key(), resolver_key);
        axon_sdk::invocation::verify_host_attestation(
            &callee.ura,
            signer_ura,
            authority.host_attestation(),
            &resolver_key,
        )
        .unwrap();

        let canonical = b"daemon-production-receipt";
        let signature = authority.sign_and_verify(canonical).await.unwrap();
        verify_authority_signature(authority.as_ref(), canonical, &signature).unwrap();
    }

    #[tokio::test]
    async fn hosted_receipt_provider_rejects_non_agent_substitution() {
        let provider = test_device_provider();
        let unowned_hub = strict_identity("easynet:///r/acme/authority").unwrap();
        let error = match CanonicalReceiptProvider::resolve_signing_authority(
            &provider,
            &unowned_hub,
        )
        .await
        {
            Ok(_) => panic!("hosted Device authority must not sign for a Hub"),
            Err(error) => error,
        };
        assert_eq!(error.reason, "daemon_receipt_callee_not_owned");
    }

    #[tokio::test]
    async fn same_realm_unhosted_agent_has_no_receipt_or_invocation_authority() {
        let provider = test_device_provider();
        let unhosted = strict_identity("easynet:///r/acme/agent/bob.worker").unwrap();

        let receipt_error =
            match CanonicalReceiptProvider::resolve_signing_authority(&provider, &unhosted).await {
                Ok(_) => panic!("unhosted Agent must not receive receipt authority"),
                Err(error) => error,
            };
        assert_eq!(receipt_error.reason, "daemon_receipt_callee_not_owned");
        assert!(
            InvocationSigningAuthorityProvider::resolve(&provider, &unhosted.ura)
                .await
                .unwrap()
                .is_none()
        );
    }

    #[tokio::test]
    async fn invocation_signing_requires_owned_authority_not_just_a_signer_key() {
        let owner = "easynet:///r/acme/device/edge-01";
        let signer: Arc<dyn CanonicalSigner> =
            Arc::new(TestCanonicalSigner::new(owner, [0x44; 32]));
        let provider = KeyServiceReceiptAuthorityProvider {
            self_signed: HashMap::new(),
            signer_capabilities: HashMap::from([(owner.to_string(), signer)]),
            hosted_agent_device: None,
            hosted_agent_inventory: None,
        };

        let error = match InvocationSigningAuthorityProvider::resolve(&provider, owner).await {
            Ok(_) => panic!("a raw signer capability must not imply invocation authority"),
            Err(error) => error,
        };
        assert_eq!(error.reason, "daemon_invocation_caller_not_owned");
    }

    #[tokio::test]
    async fn resolved_hosted_lease_is_rejected_after_inventory_revoke() {
        let (provider, inventory) = test_device_provider_with_inventory();
        let callee_ura = "easynet:///r/acme/agent/alice.worker";
        let callee = strict_identity(callee_ura).unwrap();
        let authority = CanonicalReceiptProvider::resolve_signing_authority(&provider, &callee)
            .await
            .unwrap();

        inventory.revoke(callee_ura);

        let error = authority
            .sign_and_verify(b"must-not-sign-after-revoke")
            .await
            .expect_err("T1 lease must be invalid after T2 revoke");
        assert_eq!(error.reason, "daemon_signing_authority_lease_revoked");
    }

    #[tokio::test]
    async fn old_receipt_signature_remains_resolver_visible_after_key_service_restart() {
        let directory = tempfile::tempdir().unwrap();
        let vault_path = directory.path().join("runtime-signing.enc");
        let source = MasterKeySource::Explicit("receipt-restart-test-passphrase".to_string());
        let owner = "easynet:///r/acme/device/edge-01";
        let canonical_receipt = b"canonical-signed-receipt-before-daemon-restart";

        let mut first_vault = Vault::open_or_init(&vault_path, &source).unwrap();
        first_vault.put(owner, hex::encode([0x5a; 32])).unwrap();
        first_vault.seal().unwrap();
        let first_provider: Arc<dyn SelfIdentity> = Arc::new(InMemoryVault::new(first_vault));
        let first_signer: Arc<dyn CanonicalSigner> =
            Arc::new(RuntimeSigningIdentity::load(owner.to_string(), first_provider).unwrap());
        let first_authority =
            self_signed_authority(strict_self_signed_identity(owner).unwrap(), first_signer)
                .unwrap();
        let signature = first_authority
            .sign_and_verify(canonical_receipt)
            .await
            .unwrap();
        let published_key = first_authority.verifying_key();
        drop(first_authority);

        let restarted_vault = Vault::open(&vault_path, &source).unwrap();
        let restarted_provider: Arc<dyn SelfIdentity> =
            Arc::new(InMemoryVault::new(restarted_vault));
        let restarted_signer: Arc<dyn CanonicalSigner> =
            Arc::new(RuntimeSigningIdentity::load(owner.to_string(), restarted_provider).unwrap());
        let restarted_authority = self_signed_authority(
            strict_self_signed_identity(owner).unwrap(),
            restarted_signer,
        )
        .unwrap();

        assert_eq!(restarted_authority.verifying_key(), published_key);
        verify_authority_signature(&restarted_authority, canonical_receipt, &signature)
            .expect("resolver-visible persistent key verifies the pre-restart receipt signature");
    }

    #[test]
    fn self_signed_owner_rejects_agent_and_mismatched_signer() {
        let agent_ura = "easynet:///r/acme/agent/alice.worker";
        let error = strict_self_signed_identity(agent_ura).unwrap_err();
        assert_eq!(
            error.reason,
            "daemon_receipt_self_signed_owner_kind_invalid"
        );

        let device = strict_identity("easynet:///r/acme/device/edge-01").unwrap();
        let signer: Arc<dyn CanonicalSigner> = Arc::new(TestCanonicalSigner::new(
            "easynet:///r/acme/device/edge-02",
            [0x19; 32],
        ));
        let error = match self_signed_authority(device, signer) {
            Ok(_) => panic!("mismatched signer owner must be rejected"),
            Err(error) => error,
        };
        assert_eq!(error.reason, "daemon_receipt_signer_owner_mismatch");
    }
}
