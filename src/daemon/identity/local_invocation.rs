//! Daemon-local Axon invocation identity.
//!
//! This module owns the synthetic caller used when the daemon invokes its
//! embedded `LocalRuntime` without an external caller signature. It is not a
//! user, device, or hub identity; it is the daemon's internal control-plane
//! subject for loopback calls that still need to pass through Axon's public
//! signed invocation API.

use axon_sdk::invocation::{
    AgentIdentity, AxonError, CanonicalReceiptProvider, ErrorCode, ErrorStage, KeyResolver,
    SecurityClass, UraProfile,
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
        }
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
            Err(Self::unknown_agent_key(agent_ura))
        } else {
            Ok(keys)
        }
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
