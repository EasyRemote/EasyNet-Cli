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
use ed25519_dalek::{SigningKey, VerifyingKey};
use rand::RngCore as _;
use std::sync::{Arc, OnceLock};

/// Synthetic system-agent URA for daemon-internal LocalRuntime calls.
pub(crate) const LOCAL_SYSTEM_AGENT_URA: &str = "easynet:///r/_system/agent/_system.local";
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

/// Process-local capability for daemon-internal LocalRuntime calls.
///
/// It is deliberately generated at process boot instead of derived from a
/// source-code constant. The private key never leaves this process; external
/// wire callers cannot reproduce a `_system.local` signature.
pub(crate) struct LocalSystemIdentity {
    signing_key: SigningKey,
}

impl LocalSystemIdentity {
    fn generate() -> Self {
        let mut seed = [0u8; 32];
        rand::rngs::OsRng.fill_bytes(&mut seed);
        Self {
            signing_key: SigningKey::from_bytes(&seed),
        }
    }

    pub(crate) fn signing_key(&self) -> &SigningKey {
        &self.signing_key
    }

    pub(crate) fn verifying_key(&self) -> VerifyingKey {
        self.signing_key.verifying_key()
    }
}

static PROCESS_LOCAL_SYSTEM_IDENTITY: OnceLock<Arc<LocalSystemIdentity>> = OnceLock::new();

/// Return the process-local `_system.local` capability.
pub(crate) fn process_local_system_identity() -> Arc<LocalSystemIdentity> {
    Arc::clone(
        PROCESS_LOCAL_SYSTEM_IDENTITY.get_or_init(|| Arc::new(LocalSystemIdentity::generate())),
    )
}

/// Return the verifying key for daemon-internal loopback signatures.
pub(crate) fn system_verifying_key() -> VerifyingKey {
    process_local_system_identity().verifying_key()
}

/// Device URA used by local daemon clients when no more specific loopback
/// caller has been supplied.
pub(crate) fn local_device_ura() -> String {
    if let Some(ura) = persisted_local_device_ura() {
        return ura;
    }
    crate::persistence::config::load_credentials()
        .ok()
        .map(|creds| crate::ura::device_ura(&creds.realm, &creds.node_id))
        .unwrap_or_else(|| crate::ura::device_ura(UNPAIRED_LOCAL_REALM, UNPAIRED_LOCAL_DEVICE_ID))
}

fn persisted_local_device_ura() -> Option<String> {
    let local = crate::persistence::local_agents::load().ok()?;
    let ura = local.host_device_agent_ura.trim();
    if ura.is_empty() {
        return None;
    }
    let parsed = crate::ura::parse_ura(ura).ok()?;
    if parsed.kind == crate::ura::URAKind::Device {
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
    system_verifying_key: VerifyingKey,
}

impl LocalSystemKeyResolver {
    pub(crate) fn new(upstream: Option<Arc<dyn KeyResolver>>) -> Self {
        Self {
            upstream,
            system_verifying_key: system_verifying_key(),
        }
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
            return Ok(self.system_verifying_key);
        }
        self.upstream
            .as_ref()
            .ok_or_else(|| Self::unknown_agent_key(agent_ura))?
            .resolve(agent_ura)
    }

    fn resolve_all(&self, agent_ura: &str) -> Result<Vec<VerifyingKey>, AxonError> {
        if agent_ura == LOCAL_SYSTEM_AGENT_URA {
            return Ok(vec![self.system_verifying_key]);
        }
        self.upstream
            .as_ref()
            .ok_or_else(|| Self::unknown_agent_key(agent_ura))?
            .resolve_all(agent_ura)
    }
}
