//! Daemon-local Axon invocation identity.
//!
//! This module owns the synthetic caller used when the daemon invokes its
//! embedded `LocalRuntime` without an external caller signature. It is not a
//! user, device, or hub identity; it is the daemon's internal control-plane
//! subject for loopback calls that still need to pass through Axon's public
//! signed invocation API.

use axon_sdk::invocation::{
    AgentIdentity, AxonError, CanonicalReceiptProvider, ErrorCode, ErrorStage, KeyResolver,
    SecurityClass, UraProfile, MAX_KEYS_PER_AGENT_URA,
};
use ed25519_dalek::{Signature, VerifyingKey};
use std::sync::{Arc, OnceLock};

pub(crate) use crate::core::ura::LOCAL_SYSTEM_AGENT_URA;

/// Build an Axon identity for a daemon-local agent URA.
pub(crate) fn agent_identity(ura: impl Into<String>) -> AgentIdentity {
    AgentIdentity::new(ura, UraProfile::StrictV2)
}

/// Build the daemon's synthetic caller identity.
pub(crate) fn system_agent_identity() -> AgentIdentity {
    agent_identity(LOCAL_SYSTEM_AGENT_URA)
}

/// Daemon-key-service-backed capability for daemon-internal LocalRuntime
/// calls. `_system.local` is an ordinary runtime owner: it has a public
/// projection and can request signatures, but the daemon process never owns
/// its private key.
pub(crate) struct LocalSystemIdentity {
    public_key: VerifyingKey,
    provider: Arc<dyn super::self_identity::SelfIdentity>,
}

impl LocalSystemIdentity {
    #[cfg(not(test))]
    fn load_from_key_service() -> Result<Self, super::self_identity::SelfIdentityError> {
        ensure_key_service_ready_for_local_system_identity()?;
        let client = Arc::new(super::self_identity::KeyringClient::default_path());
        let public_key = client.ensure(LOCAL_SYSTEM_AGENT_URA)?;
        Ok(Self {
            public_key,
            provider: client,
        })
    }

    fn sign_canonical(
        &self,
        canonical_bytes: &[u8],
    ) -> Result<Signature, super::self_identity::SelfIdentityError> {
        self.provider
            .sign_bound(LOCAL_SYSTEM_AGENT_URA, &self.public_key, canonical_bytes)
    }

    pub(crate) fn verifying_key(&self) -> VerifyingKey {
        self.public_key
    }
}

#[cfg(not(test))]
static PROCESS_LOCAL_SYSTEM_IDENTITY: OnceLock<Arc<LocalSystemIdentity>> = OnceLock::new();

/// Return the daemon-key-service-backed `_system.local` capability.
///
/// The daemon lifecycle provisions this owner before building LocalRuntime. A
/// failed early lookup is deliberately not cached so a supervised key-service
/// restart can recover the next request without recreating an in-process key.
#[cfg(not(test))]
fn process_local_system_identity(
) -> Result<Arc<LocalSystemIdentity>, super::self_identity::SelfIdentityError> {
    if let Some(identity) = PROCESS_LOCAL_SYSTEM_IDENTITY.get() {
        return Ok(Arc::clone(identity));
    }
    let candidate = Arc::new(LocalSystemIdentity::load_from_key_service()?);
    let _ = PROCESS_LOCAL_SYSTEM_IDENTITY.set(candidate);
    Ok(Arc::clone(PROCESS_LOCAL_SYSTEM_IDENTITY.get().expect(
        "local system identity must be installed after successful key-service load",
    )))
}

/// Unit tests exercise the same vault state machine in memory so they do not
/// depend on a process-global UDS endpoint. This is deliberately compiled
/// only in the library test target; production and integration consumers have
/// no in-process signing fallback.
#[cfg(test)]
fn process_local_system_identity(
) -> Result<Arc<LocalSystemIdentity>, super::self_identity::SelfIdentityError> {
    use super::self_identity::InMemoryVault;
    use crate::daemon::keyring::{MasterKeySource, Vault};

    static TEST_SYSTEM_IDENTITY: OnceLock<Arc<LocalSystemIdentity>> = OnceLock::new();
    let identity = TEST_SYSTEM_IDENTITY.get_or_init(|| {
        let directory =
            tempfile::tempdir().expect("create daemon-local system identity test vault directory");
        let path = directory.path().join("key-service.enc");
        let mut vault = Vault::open_or_init(
            &path,
            &MasterKeySource::Explicit("test-daemon-local-system-identity".into()),
        )
        .expect("open daemon-local system identity test vault");
        vault
            .ensure(LOCAL_SYSTEM_AGENT_URA)
            .expect("provision daemon-local system identity in test vault");
        std::mem::forget(directory);

        let provider: Arc<dyn super::self_identity::SelfIdentity> =
            Arc::new(InMemoryVault::new(vault));
        let public_key = provider
            .public_key(LOCAL_SYSTEM_AGENT_URA)
            .expect("project daemon-local system identity from test vault");
        Arc::new(LocalSystemIdentity {
            public_key,
            provider,
        })
    });
    Ok(Arc::clone(identity))
}

/// Sign daemon-internal canonical bytes through the key-service custody
/// boundary. This is the only signing capability for `_system.local`.
pub(crate) fn sign_system_canonical(
    canonical_bytes: &[u8],
) -> Result<Signature, super::self_identity::SelfIdentityError> {
    #[cfg(not(test))]
    ensure_key_service_ready_for_local_system_identity()?;
    process_local_system_identity()?.sign_canonical(canonical_bytes)
}

/// Return the verifying key for daemon-internal loopback signatures.
pub(crate) fn system_verifying_key() -> Result<VerifyingKey, super::self_identity::SelfIdentityError>
{
    #[cfg(not(test))]
    ensure_key_service_ready_for_local_system_identity()?;
    Ok(process_local_system_identity()?.verifying_key())
}

#[cfg(not(test))]
fn ensure_key_service_ready_for_local_system_identity(
) -> Result<(), super::self_identity::SelfIdentityError> {
    super::self_identity::KeyringClient::default_path().health()
}

/// Device URA used by local daemon clients when a real local device identity
/// has been provisioned.
pub(crate) fn local_device_ura() -> anyhow::Result<String> {
    if let Some(ura) = persisted_local_device_ura() {
        return Ok(ura);
    }
    let creds = crate::daemon::persistence::config::load_credentials().map_err(|error| {
        anyhow::anyhow!(
            "local device identity unavailable: no hosted device projection and credentials are unavailable: {error}"
        )
    })?;
    Ok(crate::core::ura::device_ura(&creds.realm, &creds.node_id))
}

/// Runtime-published URA owned by the local daemon process advertised in
/// control.json.
///
/// CLI loopback calls that target the running daemon itself must be bound to
/// that daemon's published identity. Falling back to `local_device_ura()` would
/// let absent or stale control discovery synthesize a device/default owner and
/// reintroduce a second daemon-identity authority outside daemon boot.
pub(crate) fn local_daemon_ura() -> anyhow::Result<String> {
    control_discovery_daemon_ura()?.ok_or_else(|| {
        anyhow::anyhow!(
            "local daemon identity unavailable: control discovery does not publish a daemon \
             identity; start or restart the daemon before constructing daemon-local invocation"
        )
    })
}

fn persisted_local_device_ura() -> Option<String> {
    let hosted_identity =
        crate::daemon::persistence::agent_aggregate::AgentAggregateRepository::load_hosted_identity_status()
            .ok()?;
    let ura = hosted_identity.host_device_agent_ura()?;
    let parsed = crate::core::ura::parse_ura(ura).ok()?;
    if parsed.kind == crate::core::ura::URAKind::Device {
        Some(ura.to_string())
    } else {
        None
    }
}

fn control_discovery_daemon_ura() -> anyhow::Result<Option<String>> {
    let path = crate::daemon::control::discovery::try_default_path().map_err(|error| {
        anyhow::anyhow!("resolve local daemon identity discovery path: {error}")
    })?;
    let Some(discovery) = crate::daemon::control::discovery::read(&path).map_err(|error| {
        anyhow::anyhow!(
            "read local daemon identity from {}: {error}",
            path.display()
        )
    })?
    else {
        return Ok(None);
    };
    let Some(identity) = discovery.daemon_identity else {
        return Ok(None);
    };
    match identity.mode.as_str() {
        "hub" => Ok(Some(crate::core::ura::hub_ura(&identity.realm))),
        "device" | "both" => Ok(identity
            .node_id
            .as_deref()
            .map(|node_id| crate::core::ura::device_ura(&identity.realm, node_id))),
        other => anyhow::bail!(
            "local daemon identity unavailable: control discovery daemon_identity.mode {other:?} \
             is not hub, device, or both"
        ),
    }
}

/// Stable CLI-owned resolver graph for canonical runtime admission.
///
/// The graph is complete before `LocalRuntime` is constructed. It combines
/// four explicit key authorities without mutating the SDK runtime:
///
/// - daemon trust-anchor identities;
/// - product bootstrap identities;
/// - request-scoped bootstrap candidate keys;
/// - receipt-signer public projections.
///
/// `_system.local` remains a bounded exact-match branch backed by the daemon
/// key service. No branch can resolve an alias owned by another branch.
pub struct CanonicalAdmissionKeyResolver {
    trusted_identities: Arc<dyn KeyResolver>,
    bootstrap_identities:
        Arc<crate::daemon::axon_bridge::runtime_admin::RuntimeBootstrapIdentityProvider>,
    bootstrap_candidate:
        Arc<crate::daemon::axon_bridge::runtime_admin::BootstrapCandidateKeyProvider>,
    receipt_signers: Arc<dyn CanonicalReceiptProvider>,
    invocation_verification_keys: Option<
        Arc<dyn crate::daemon::identity::receipt_signing::InvocationVerificationKeyProvider>,
    >,
}

impl CanonicalAdmissionKeyResolver {
    pub(crate) fn new(
        trusted_identities: Arc<dyn KeyResolver>,
        bootstrap_identities: Arc<
            crate::daemon::axon_bridge::runtime_admin::RuntimeBootstrapIdentityProvider,
        >,
        bootstrap_candidate: Arc<
            crate::daemon::axon_bridge::runtime_admin::BootstrapCandidateKeyProvider,
        >,
        receipt_signers: Arc<dyn CanonicalReceiptProvider>,
    ) -> Self {
        Self {
            trusted_identities,
            bootstrap_identities,
            bootstrap_candidate,
            receipt_signers,
            invocation_verification_keys: None,
        }
    }

    pub(crate) fn with_invocation_verification_keys(
        mut self,
        provider: Arc<
            dyn crate::daemon::identity::receipt_signing::InvocationVerificationKeyProvider,
        >,
    ) -> Self {
        self.invocation_verification_keys = Some(provider);
        self
    }

    pub(crate) fn bootstrap_identity_provider(
        &self,
    ) -> Arc<crate::daemon::axon_bridge::runtime_admin::RuntimeBootstrapIdentityProvider> {
        Arc::clone(&self.bootstrap_identities)
    }

    pub(crate) fn bootstrap_candidate_provider(
        &self,
    ) -> Arc<crate::daemon::axon_bridge::runtime_admin::BootstrapCandidateKeyProvider> {
        Arc::clone(&self.bootstrap_candidate)
    }

    fn unknown_agent_key(agent_ura: &str) -> AxonError {
        AxonError::invalid_argument(ErrorCode::CallerKeyNotFound.as_str())
            .with_code(ErrorCode::CallerKeyNotFound)
            .with_stage(ErrorStage::CallerAuthentication)
            .with_security_class(SecurityClass::Identity)
            .with_message(format!("unknown_agent_key:{agent_ura}"))
    }

    fn append_unique(keys: &mut Vec<VerifyingKey>, candidate: VerifyingKey) {
        if !keys.contains(&candidate) {
            keys.push(candidate);
        }
    }
}

impl KeyResolver for CanonicalAdmissionKeyResolver {
    fn resolve(&self, agent_ura: &str) -> Result<VerifyingKey, AxonError> {
        self.resolve_all(agent_ura)?
            .into_iter()
            .next()
            .ok_or_else(|| Self::unknown_agent_key(agent_ura))
    }

    fn resolve_all(&self, agent_ura: &str) -> Result<Vec<VerifyingKey>, AxonError> {
        if agent_ura == LOCAL_SYSTEM_AGENT_URA {
            return system_verifying_key()
                .map(|key| vec![key])
                .map_err(|error| {
                    AxonError::internal(format!(
                        "daemon-local system identity unavailable from key service: {error}"
                    ))
                });
        }

        let mut keys = Vec::new();
        if let Some(key) = self.receipt_signers.resolve_signer_key(agent_ura)? {
            Self::append_unique(&mut keys, key);
        }
        if let Some(provider) = self.invocation_verification_keys.as_ref() {
            if let Some(key) = provider.resolve_invocation_verifying_key(agent_ura)? {
                Self::append_unique(&mut keys, key);
            }
        }
        if let Some(bootstrap_keys) = self.bootstrap_identities.keys_for(agent_ura)? {
            for key in bootstrap_keys {
                Self::append_unique(&mut keys, key);
            }
        }
        if let Some(key) = self.bootstrap_candidate.key_for(agent_ura)? {
            Self::append_unique(&mut keys, key);
        }
        match self.trusted_identities.resolve_all(agent_ura) {
            Ok(trusted_keys) => {
                for key in trusted_keys {
                    Self::append_unique(&mut keys, key);
                }
            }
            Err(error) if error.code == ErrorCode::CallerKeyNotFound && !keys.is_empty() => {}
            Err(error) => return Err(error),
        }
        if keys.is_empty() {
            return Err(Self::unknown_agent_key(agent_ura));
        }
        keys.truncate(MAX_KEYS_PER_AGENT_URA);
        Ok(keys)
    }
}

#[cfg(test)]
mod tests {
    use std::sync::Arc;

    use axon_sdk::invocation::{
        AgentIdentity, AxonError, CanonicalReceiptProvider, DescriptorBoundEnvelope, ErrorCode,
        KeyResolver, ReceiptSigningAuthority, VerifiedAdmissionPolicy,
    };
    use ed25519_dalek::{Verifier as _, VerifyingKey};

    use super::{
        local_daemon_ura, local_device_ura, sign_system_canonical, system_verifying_key,
        CanonicalAdmissionKeyResolver,
    };
    use crate::daemon::axon_bridge::runtime_admin::{
        BootstrapCandidateKeyProvider, RuntimeBootstrapIdentityProvider,
    };
    use crate::daemon::control::discovery::{
        self, ControlDiscovery, DaemonIdentity, IpcVersionRange, IPC_VERSION_V1,
    };

    struct FixedReceiptKeyProjection {
        signer_ura: &'static str,
        key: VerifyingKey,
    }

    #[async_trait::async_trait]
    impl CanonicalReceiptProvider for FixedReceiptKeyProjection {
        fn verify_admission_policy(
            &self,
            _envelope: &DescriptorBoundEnvelope,
        ) -> Result<VerifiedAdmissionPolicy, AxonError> {
            Err(AxonError::internal("unused_test_admission_policy"))
        }

        async fn resolve_signing_authority(
            &self,
            _callee: &AgentIdentity,
        ) -> Result<Arc<dyn ReceiptSigningAuthority>, AxonError> {
            Err(AxonError::internal("unused_test_signing_authority"))
        }

        fn resolve_signer_key(&self, signer_ura: &str) -> Result<Option<VerifyingKey>, AxonError> {
            Ok((signer_ura == self.signer_ura).then_some(self.key))
        }
    }

    struct FailingTrustedIdentityResolver {
        error: AxonError,
    }

    impl KeyResolver for FailingTrustedIdentityResolver {
        fn resolve(&self, _agent_ura: &str) -> Result<VerifyingKey, AxonError> {
            Err(self.error.clone())
        }
    }

    struct FixedTrustedIdentityResolver {
        keys: Vec<VerifyingKey>,
    }

    impl KeyResolver for FixedTrustedIdentityResolver {
        fn resolve(&self, _agent_ura: &str) -> Result<VerifyingKey, AxonError> {
            self.keys
                .first()
                .copied()
                .ok_or_else(|| AxonError::invalid_argument(ErrorCode::CallerKeyNotFound.as_str()))
        }

        fn resolve_all(&self, _agent_ura: &str) -> Result<Vec<VerifyingKey>, AxonError> {
            Ok(self.keys.clone())
        }
    }

    fn admission_resolver_with_trusted_error(
        signer_ura: &'static str,
        trusted_error: AxonError,
    ) -> (CanonicalAdmissionKeyResolver, VerifyingKey) {
        let mut compressed_basepoint = [0x66; 32];
        compressed_basepoint[0] = 0x58;
        let key =
            VerifyingKey::from_bytes(&compressed_basepoint).expect("Ed25519 basepoint public key");
        let trusted_identities: Arc<dyn KeyResolver> = Arc::new(FailingTrustedIdentityResolver {
            error: trusted_error,
        });
        let receipt_signers: Arc<dyn CanonicalReceiptProvider> =
            Arc::new(FixedReceiptKeyProjection { signer_ura, key });
        (
            CanonicalAdmissionKeyResolver::new(
                trusted_identities,
                Arc::new(RuntimeBootstrapIdentityProvider::default()),
                Arc::new(BootstrapCandidateKeyProvider::default()),
                receipt_signers,
            ),
            key,
        )
    }

    #[test]
    fn admission_resolver_ignores_only_an_absent_trusted_identity() {
        const SIGNER_URA: &str = "easynet:///r/acme/device/receipt-signer";
        let missing = AxonError::invalid_argument(ErrorCode::CallerKeyNotFound.as_str())
            .with_code(ErrorCode::CallerKeyNotFound);
        let (resolver, receipt_key) = admission_resolver_with_trusted_error(SIGNER_URA, missing);

        assert_eq!(resolver.resolve_all(SIGNER_URA).unwrap(), vec![receipt_key]);
    }

    fn verifying_key(bytes: [u8; 32]) -> VerifyingKey {
        VerifyingKey::from_bytes(&bytes).expect("valid verifying key fixture")
    }

    #[test]
    fn admission_resolver_caps_combined_user_keys_and_keeps_bootstrap_candidate_first() {
        const USER_URA: &str = "easynet:///r/acme/user/alice";
        let trusted_keys = vec![
            verifying_key([
                0x43, 0xa7, 0x2e, 0x71, 0x44, 0x01, 0x76, 0x2d, 0xf6, 0x6b, 0x68, 0xc2, 0x6d, 0xfb,
                0xdf, 0x26, 0x82, 0xaa, 0xec, 0x9f, 0x24, 0x74, 0xec, 0xa4, 0x61, 0x3e, 0x42, 0x4a,
                0x0f, 0xba, 0xfd, 0x3c,
            ]),
            verifying_key([
                0x66, 0xbe, 0x7e, 0x33, 0x2c, 0x7a, 0x45, 0x33, 0x32, 0xbd, 0x9d, 0x0a, 0x7f, 0x7d,
                0xb0, 0x55, 0xf5, 0xc5, 0xef, 0x1a, 0x06, 0xad, 0xa6, 0x6d, 0x98, 0xb3, 0x9f, 0xb6,
                0x81, 0x0c, 0x47, 0x3a,
            ]),
            verifying_key([
                0x0b, 0x51, 0x3a, 0xd9, 0xb4, 0x92, 0x40, 0x15, 0xca, 0x09, 0x02, 0xed, 0x07, 0x90,
                0x44, 0xd3, 0xac, 0x5d, 0xbe, 0xc2, 0x30, 0x6f, 0x06, 0x94, 0x8c, 0x10, 0xda, 0x8e,
                0xb6, 0xe3, 0x9f, 0x2d,
            ]),
            verifying_key([
                0x91, 0xa2, 0x8a, 0x0b, 0x74, 0x38, 0x15, 0x93, 0xa4, 0xd9, 0x46, 0x95, 0x79, 0x20,
                0x89, 0x26, 0xaf, 0xc8, 0xad, 0x82, 0xc8, 0x83, 0x9b, 0x76, 0x44, 0x35, 0x9b, 0x9e,
                0xba, 0x9a, 0x4b, 0x3a,
            ]),
            verifying_key([
                0x0b, 0xee, 0xf5, 0xa9, 0xe6, 0x79, 0xe6, 0xa3, 0xe1, 0x34, 0xfe, 0x27, 0x83, 0x7b,
                0xff, 0x32, 0xc7, 0xcb, 0x5f, 0x5d, 0x44, 0xea, 0x09, 0xbc, 0xb0, 0xe5, 0x42, 0xba,
                0xd6, 0xa4, 0xc0, 0xcc,
            ]),
            verifying_key([
                0xd9, 0xbf, 0x21, 0x48, 0x74, 0x8a, 0x85, 0xc8, 0x9d, 0xa5, 0xaa, 0xd8, 0xee, 0x0b,
                0x0f, 0xc2, 0xd1, 0x05, 0xfd, 0x39, 0xd4, 0x1a, 0x4c, 0x79, 0x65, 0x36, 0x35, 0x4f,
                0x0a, 0xe2, 0x90, 0x0c,
            ]),
            verifying_key([
                0x5c, 0x9c, 0x6d, 0xf2, 0x61, 0xc9, 0xcb, 0x84, 0x04, 0x75, 0x77, 0x6a, 0xae, 0xfc,
                0xd9, 0x44, 0xb4, 0x05, 0x32, 0x8f, 0xab, 0x28, 0xf9, 0xb3, 0xa9, 0x5e, 0xf4, 0x04,
                0x90, 0xd3, 0xde, 0x84,
            ]),
            verifying_key([
                0xd0, 0x4a, 0xb2, 0x32, 0x74, 0x2b, 0xb4, 0xab, 0x3a, 0x13, 0x68, 0xbd, 0x46, 0x15,
                0xe4, 0xe6, 0xd0, 0x22, 0x4a, 0xb7, 0x1a, 0x01, 0x6b, 0xaf, 0x85, 0x20, 0xa3, 0x32,
                0xc9, 0x77, 0x87, 0x37,
            ]),
        ];
        let candidate_key = verifying_key([
            0x8a, 0x88, 0xe3, 0xdd, 0x74, 0x09, 0xf1, 0x95, 0xfd, 0x52, 0xdb, 0x2d, 0x3c, 0xba,
            0x5d, 0x72, 0xca, 0x67, 0x09, 0xbf, 0x1d, 0x94, 0x12, 0x1b, 0xf3, 0x74, 0x88, 0x01,
            0xb4, 0x0f, 0x6f, 0x5c,
        ]);
        let candidate_provider = Arc::new(BootstrapCandidateKeyProvider::default());
        let _lease = candidate_provider
            .lease_candidate(USER_URA, candidate_key)
            .expect("lease bootstrap candidate");
        let receipt_signers: Arc<dyn CanonicalReceiptProvider> =
            Arc::new(FixedReceiptKeyProjection {
                signer_ura: "easynet:///r/acme/device/receipt-signer",
                key: verifying_key([
                    0x81, 0x39, 0x77, 0x0e, 0xa8, 0x7d, 0x17, 0x5f, 0x56, 0xa3, 0x54, 0x66, 0xc3,
                    0x4c, 0x7e, 0xcc, 0xcb, 0x8d, 0x8a, 0x91, 0xb4, 0xee, 0x37, 0xa2, 0x5d, 0xf6,
                    0x0f, 0x5b, 0x8f, 0xc9, 0xb3, 0x94,
                ]),
            });
        let resolver = CanonicalAdmissionKeyResolver::new(
            Arc::new(FixedTrustedIdentityResolver { keys: trusted_keys }),
            Arc::new(RuntimeBootstrapIdentityProvider::default()),
            candidate_provider,
            receipt_signers,
        );

        let keys = resolver.resolve_all(USER_URA).expect("resolve user keys");

        assert_eq!(keys.len(), axon_sdk::invocation::MAX_KEYS_PER_AGENT_URA);
        assert_eq!(keys[0], candidate_key);
    }

    #[test]
    fn admission_resolver_preserves_trusted_identity_provider_failures() {
        const SIGNER_URA: &str = "easynet:///r/acme/device/receipt-signer";
        let (resolver, _) = admission_resolver_with_trusted_error(
            SIGNER_URA,
            AxonError::internal("trusted_identity_backend_unavailable"),
        );

        let error = resolver
            .resolve_all(SIGNER_URA)
            .expect_err("provider failure must not be hidden by another candidate key");
        assert_eq!(error.code, ErrorCode::InternalError);
        assert_eq!(error.reason, "trusted_identity_backend_unavailable");
    }

    #[test]
    fn system_signing_capability_verifies_against_the_shared_runtime_projection() {
        let canonical = b"daemon-local canonical invocation";
        let signature = sign_system_canonical(canonical)
            .expect("test key-service state machine signs daemon-local canonical bytes");
        system_verifying_key()
            .expect("test key-service state machine projects daemon-local public key")
            .verify(canonical, &signature)
            .expect("daemon-local signature verifies against its shared public projection");
    }

    #[test]
    fn local_device_ura_rejects_missing_identity_instead_of_synthesizing_default_local() {
        let _home = crate::cli::commands::test_support::HomeGuard::new();

        let error = local_device_ura().expect_err("missing device identity must fail closed");

        let message = format!("{error}");
        assert!(
            message.contains("local device identity unavailable"),
            "unexpected error: {message}"
        );
        assert!(
            !message.contains("easynet:///r/default/device/local"),
            "error must not expose a synthetic device fallback: {message}"
        );
    }

    #[test]
    fn local_device_ura_projects_credentials_when_hosted_identity_is_absent() {
        let _home = crate::cli::commands::test_support::HomeGuard::new();
        crate::daemon::persistence::config::save_credentials(
            &crate::daemon::persistence::config::Credentials {
                node_id: "dev-a".to_string(),
                credential_token: "token".to_string(),
                hub_endpoint: "axon://hub.example:50051".to_string(),
                realm: "acme".to_string(),
                username: Some("alice".to_string()),
                user_id: Some("user-alice".to_string()),
                ..Default::default()
            },
        )
        .expect("write local device credentials");

        assert_eq!(
            local_device_ura().expect("credentials-backed local device URA"),
            crate::core::ura::device_ura("acme", "dev-a")
        );
    }

    #[test]
    fn local_daemon_ura_uses_control_discovery_identity() {
        let _home = crate::cli::commands::test_support::HomeGuard::new();
        write_discovery_identity("hub", "hub-realm", None);
        assert_eq!(
            local_daemon_ura().unwrap(),
            crate::core::ura::hub_ura("hub-realm")
        );

        write_discovery_identity("device", "device-realm", Some("node-a"));
        assert_eq!(
            local_daemon_ura().unwrap(),
            crate::core::ura::device_ura("device-realm", "node-a")
        );
    }

    #[test]
    fn local_daemon_ura_rejects_missing_control_discovery_identity() {
        let _home = crate::cli::commands::test_support::HomeGuard::new();

        let error = local_daemon_ura().expect_err("missing daemon identity must fail closed");

        let message = format!("{error}");
        assert!(
            message.contains("control discovery does not publish a daemon identity"),
            "unexpected error: {message}"
        );
    }

    #[test]
    fn local_daemon_ura_propagates_malformed_control_discovery() {
        let _home = crate::cli::commands::test_support::HomeGuard::new();
        let path = discovery::default_path();
        std::fs::create_dir_all(path.parent().expect("default control path parent"))
            .expect("create control discovery parent");
        std::fs::write(&path, b"not json").expect("write malformed control discovery");

        let error = local_daemon_ura()
            .expect_err("malformed control discovery must not collapse to missing identity");

        let message = format!("{error:#}");
        assert!(
            message.contains("read local daemon identity")
                && message.contains("control.json")
                && message.contains("malformed"),
            "unexpected error: {message}"
        );
        assert!(
            !message.contains("does not publish a daemon identity"),
            "malformed discovery must not be reported as absent identity: {message}"
        );
    }

    fn write_discovery_identity(mode: &str, realm: &str, node_id: Option<&str>) {
        let disc = ControlDiscovery {
            socket_path: None,
            pipe_name: None,
            invocation_endpoint: None,
            daemon_identity: Some(DaemonIdentity {
                mode: mode.to_string(),
                realm: realm.to_string(),
                node_id: node_id.map(str::to_string),
            }),
            pid: std::process::id(),
            daemon_version: "test".to_string(),
            supported_ipc_versions: IpcVersionRange::single(IPC_VERSION_V1),
            capability_flags: Vec::new(),
            pages_port: None,
        };
        discovery::write(&discovery::default_path(), &disc).expect("write control discovery");
    }
}
