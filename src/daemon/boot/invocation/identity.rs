use std::sync::Arc;

use crate::daemon::identity::self_identity::{load_runtime_caller_signer, CanonicalSigner};
use crate::daemon::persistence::config::{load_credentials_optional, Credentials};
use crate::daemon::persistence::daemon_config::DaemonMode;

#[derive(Clone)]
pub(super) struct DaemonIdentity {
    pub caller_ura: String,
    pub signer: Arc<dyn CanonicalSigner>,
}

impl std::fmt::Debug for DaemonIdentity {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter
            .debug_struct("DaemonIdentity")
            .field("caller_ura", &self.caller_ura)
            .field("signer_owner_ura", &self.signer.owner_ura())
            .finish()
    }
}

impl DaemonIdentity {
    pub(super) fn bind(
        caller_ura: String,
        signer: Arc<dyn CanonicalSigner>,
    ) -> anyhow::Result<Self> {
        if signer.owner_ura() != caller_ura {
            anyhow::bail!(
                "daemon identity signer owner mismatch: expected `{caller_ura}`, got `{}`",
                signer.owner_ura()
            );
        }
        Ok(Self { caller_ura, signer })
    }
}

/// Resolve the daemon's caller URA and bind it to the canonical local key
/// service. A missing credentials file is the only `None` state. Present but
/// invalid credentials or an unavailable signing identity fail boot instead
/// of silently producing an unsigned runtime.
pub(super) fn load_daemon_identity() -> anyhow::Result<Option<DaemonIdentity>> {
    let Some(credentials) = load_credentials_optional()? else {
        return Ok(None);
    };
    let caller_ura = canonical_caller_ura_from_credentials(&credentials)?;
    let signer = load_runtime_signer(&caller_ura)?;
    Ok(Some(DaemonIdentity::bind(caller_ura, signer)?))
}

/// Resolve Device credentials only for modes that actually own a Device
/// runtime. Hub-only boot has no dependency on credentials.json and must not
/// fail because a stale device pairing file exists on the same host.
pub(super) fn load_daemon_identity_for_mode(
    mode: DaemonMode,
) -> anyhow::Result<Option<DaemonIdentity>> {
    match mode {
        DaemonMode::Hub => Ok(None),
        DaemonMode::Device | DaemonMode::Both => load_daemon_identity(),
    }
}

pub(super) fn load_runtime_signer(owner_ura: &str) -> anyhow::Result<Arc<dyn CanonicalSigner>> {
    load_runtime_caller_signer(owner_ura.to_string())
        .map_err(|error| anyhow::anyhow!("bind runtime signer for `{owner_ura}`: {error}"))
}

pub(super) fn canonical_caller_ura_from_credentials(
    credentials: &Credentials,
) -> anyhow::Result<String> {
    let realm = credentials.realm.trim();
    let node_id = credentials.node_id.trim();
    if realm.is_empty() || node_id.is_empty() {
        anyhow::bail!("daemon credentials do not contain a consistent realm/node identity");
    }
    Ok(crate::core::ura::device_ura(realm, node_id))
}
