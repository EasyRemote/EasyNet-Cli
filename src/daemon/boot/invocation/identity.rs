use std::sync::Arc;

use anyhow::Context as _;

use crate::daemon::identity::self_identity::{load_runtime_caller_signer, CanonicalSigner};
use crate::daemon::persistence::daemon_config::DaemonMode;

use super::paths::expand_home;

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

/// Narrow read-projection of `~/.easynet/credentials.json` carrying
/// only the three fields the daemon needs to resolve its Device owner URA.
///
/// MUST NOT use `#[serde(deny_unknown_fields)]`. The writer
/// (`persistence::config::Credentials`) owns the file and its field
/// set grows over time — `credential_token`, `hub_endpoint`,
/// `hub_api_base`, `username`, `hub_pubkey_b64`, `hub_tls_ca_pem_b64`
/// were all added after this projection. A strict reader would reject
/// the whole file the moment any such field appears. That drops the
/// daemon's device identity, so the
/// device-mode `session.open` supervisor never starts, the hub
/// never sees the device's presence, and the backend renders it
/// REMOVED. This is a projection, not a schema gate: tolerate unknown
/// fields and read only what we own.
///
/// Two fields ARE still rejected:
///
/// * `tenant_id` is the retired alias for `realm` (URA v4.1.4);
/// * `agent_ura` is the retired pre-canonical daemon identity fact.
///
/// A credentials.json carrying either field predates the canonical
/// device identity model. Reject them explicitly via typed sentinel
/// fields rather than a blanket `deny_unknown_fields`, so retirement
/// enforcement survives without re-introducing the field-drift
/// regression above.
#[derive(Debug, serde::Deserialize)]
pub(super) struct StoredDeviceIdentity {
    /// Retired pre-canonical daemon identity fact. Present only in old
    /// files; its presence is a hard parse error.
    #[serde(default, rename = "agent_ura")]
    pub(super) _retired_agent_ura: Option<RejectedAgentUra>,
    #[serde(default)]
    pub(super) realm: Option<String>,
    #[serde(default)]
    pub(super) node_id: Option<String>,
    /// Retired `realm` alias. Present only in pre-v4.1.4 files; its
    /// presence is a hard parse error (see `deserialize` below).
    #[serde(default, rename = "tenant_id")]
    pub(super) _retired_tenant_id: Option<RejectedTenantId>,
}

/// Zero-sized marker whose `Deserialize` always errors, naming the
/// retired field. Used as the type of `StoredDeviceIdentity::tenant_id`
/// so any credentials.json still carrying `tenant_id` fails the parse
/// with a clear message, while every other unknown field is tolerated.
#[derive(Debug)]
pub(super) struct RejectedTenantId;

impl<'de> serde::Deserialize<'de> for RejectedTenantId {
    fn deserialize<D>(_deserializer: D) -> Result<Self, D::Error>
    where
        D: serde::Deserializer<'de>,
    {
        Err(serde::de::Error::custom(
            "credentials.json carries retired `tenant_id`; it was renamed to `realm` in URA \
             v4.1.4 — re-pair with `easynet join <token>` to rewrite the file",
        ))
    }
}

/// Zero-sized marker whose `Deserialize` always errors, naming the
/// retired field. Used as the type of `StoredDeviceIdentity::agent_ura`
/// so old credentials stop before daemon identity projection instead
/// of being treated as a checksum or fallback identity source.
#[derive(Debug)]
pub(super) struct RejectedAgentUra;

impl<'de> serde::Deserialize<'de> for RejectedAgentUra {
    fn deserialize<D>(_deserializer: D) -> Result<Self, D::Error>
    where
        D: serde::Deserializer<'de>,
    {
        Err(serde::de::Error::custom(
            "credentials.json carries retired `agent_ura`; daemon identity is now derived from \
             canonical `realm` + `node_id` — re-pair with `easynet join <token>` to rewrite the file",
        ))
    }
}

/// Resolve the daemon's caller URA and bind it to the canonical local key
/// service. A missing credentials file is the only `None` state. Present but
/// invalid credentials or an unavailable signing identity fail boot instead
/// of silently producing an unsigned runtime.
pub(super) fn load_daemon_identity() -> anyhow::Result<Option<DaemonIdentity>> {
    let path = expand_home("~/.easynet/credentials.json");
    let raw = match std::fs::read_to_string(&path) {
        Ok(raw) => raw,
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => return Ok(None),
        Err(error) => {
            return Err(error)
                .with_context(|| format!("read daemon credentials {}", path.display()));
        }
    };
    let stored: StoredDeviceIdentity = serde_json::from_str(&raw)
        .with_context(|| format!("parse daemon credentials {}", path.display()))?;
    let caller_ura = canonical_caller_ura_from_stored_identity(&stored).ok_or_else(|| {
        anyhow::anyhow!(
            "daemon credentials {} do not contain a consistent realm/node identity",
            path.display()
        )
    })?;
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

pub(super) fn canonical_caller_ura_from_stored_identity(
    stored: &StoredDeviceIdentity,
) -> Option<String> {
    let realm = stored
        .realm
        .as_deref()
        .map(str::trim)
        .filter(|realm| !realm.is_empty());
    let node_id = stored
        .node_id
        .as_deref()
        .map(str::trim)
        .filter(|node| !node.is_empty());

    let (Some(realm), Some(node_id)) = (realm, node_id) else {
        return None;
    };

    Some(crate::core::ura::device_ura(realm, node_id))
}
