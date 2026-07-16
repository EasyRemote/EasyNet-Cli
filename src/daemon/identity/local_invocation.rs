//! Daemon-local Axon invocation identity.
//!
//! This module owns the synthetic caller used when the daemon invokes its
//! embedded `LocalRuntime` without an external caller signature. It is not a
//! user, device, or hub identity; it is the daemon's internal control-plane
//! subject for loopback calls that still need to pass through Axon's public
//! signed invocation API.

use easynet_axon::invocation::{
    AgentIdentity, AxonError, ErrorCode, ErrorStage, KeyResolver, SecurityClass, UraProfile,
};
use ed25519_dalek::{Signature, VerifyingKey};
use std::sync::{Arc, OnceLock};

pub(crate) use crate::core::ura::LOCAL_SYSTEM_AGENT_URA;

pub(crate) const UNPAIRED_LOCAL_REALM: &str = "default";
pub(crate) const UNPAIRED_LOCAL_DEVICE_ID: &str = "local";

/// Build an Axon identity for a daemon-local agent URA.
pub(crate) fn agent_identity(ura: impl Into<String>) -> AgentIdentity {
    AgentIdentity::new(ura, UraProfile::EasynetStrictV2)
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
    process_local_system_identity()?.sign_canonical(canonical_bytes)
}

/// Return the verifying key for daemon-internal loopback signatures.
pub(crate) fn system_verifying_key() -> Result<VerifyingKey, super::self_identity::SelfIdentityError>
{
    Ok(process_local_system_identity()?.verifying_key())
}

/// Device URA used by local daemon clients when no more specific loopback
/// caller has been supplied.
pub(crate) fn local_device_ura() -> String {
    if let Some(ura) = persisted_local_device_ura() {
        return ura;
    }
    crate::daemon::persistence::config::load_credentials()
        .ok()
        .map(|creds| crate::core::ura::device_ura(&creds.realm, &creds.node_id))
        .unwrap_or_else(|| {
            crate::core::ura::device_ura(UNPAIRED_LOCAL_REALM, UNPAIRED_LOCAL_DEVICE_ID)
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

/// KeyResolver overlay for EasyNet-Cli's synthetic system caller.
///
/// What this is: a bounded single-key resolver branch for
/// `easynet:///r/_system/agent/_system.local`.
///
/// What this is not: a trust shortcut for ordinary device, hub, backend, or
/// user URAs. Every non-system lookup is delegated unchanged to the upstream
/// resolver.
pub(crate) struct LocalSystemKeyResolver {
    upstream: Option<Arc<dyn KeyResolver>>,
    receipt_signing_runtime: Option<std::sync::Weak<easynet_axon::invocation::LocalRuntime>>,
}

impl LocalSystemKeyResolver {
    pub(crate) fn new(upstream: Option<Arc<dyn KeyResolver>>) -> Self {
        Self {
            upstream,
            receipt_signing_runtime: None,
        }
    }

    pub(crate) fn with_receipt_signing_runtime(
        mut self,
        runtime: std::sync::Weak<easynet_axon::invocation::LocalRuntime>,
    ) -> Self {
        self.receipt_signing_runtime = Some(runtime);
        self
    }

    fn receipt_signer_key(&self, signer_ura: &str) -> Result<Option<VerifyingKey>, AxonError> {
        let Some(runtime) = self
            .receipt_signing_runtime
            .as_ref()
            .and_then(std::sync::Weak::upgrade)
        else {
            return Ok(None);
        };
        runtime.resolve_receipt_signer_key(signer_ura)
    }

    fn unknown_agent_key(agent_ura: &str) -> AxonError {
        AxonError::invalid_argument(ErrorCode::CallerKeyNotFound.as_str())
            .with_code(ErrorCode::CallerKeyNotFound)
            .with_stage(ErrorStage::CallerAuthentication)
            .with_security_class(SecurityClass::Identity)
            .with_message(format!("unknown_agent_key:{agent_ura}"))
    }
}

impl KeyResolver for LocalSystemKeyResolver {
    fn resolve(&self, agent_ura: &str) -> Result<VerifyingKey, AxonError> {
        if agent_ura == LOCAL_SYSTEM_AGENT_URA {
            return system_verifying_key().map_err(|error| {
                AxonError::internal(format!(
                    "daemon-local system identity unavailable from key service: {error}"
                ))
            });
        }
        if let Some(key) = self.receipt_signer_key(agent_ura)? {
            return Ok(key);
        }
        self.upstream
            .as_ref()
            .ok_or_else(|| Self::unknown_agent_key(agent_ura))?
            .resolve(agent_ura)
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
        if let Some(key) = self.receipt_signer_key(agent_ura)? {
            return Ok(vec![key]);
        }
        self.upstream
            .as_ref()
            .ok_or_else(|| Self::unknown_agent_key(agent_ura))?
            .resolve_all(agent_ura)
    }
}

#[cfg(test)]
mod tests {
    use ed25519_dalek::Verifier as _;

    use super::{sign_system_canonical, system_verifying_key};

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
}
