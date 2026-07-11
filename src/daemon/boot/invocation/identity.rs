use std::sync::Arc;

use anyhow::Context as _;

use crate::daemon::identity::self_identity::{CanonicalSigner, RuntimeSigningIdentity};
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
/// One field IS still rejected: `tenant_id`. It is the retired alias
/// for `realm` (URA v4.1.4) — a credentials.json carrying it predates
/// the rename and would derive a daemon URA under the wrong namespace.
/// We reject it explicitly via a typed sentinel field rather than a
/// blanket `deny_unknown_fields`, so retirement enforcement survives
/// without re-introducing the field-drift regression above.
#[derive(Debug, serde::Deserialize)]
pub(super) struct StoredDeviceIdentity {
    #[serde(default)]
    pub(super) agent_ura: Option<String>,
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
    Ok(Arc::new(
        RuntimeSigningIdentity::load_default(owner_ura.to_string())
            .map_err(|error| anyhow::anyhow!("bind runtime signer for `{owner_ura}`: {error}"))?,
    ))
}

/// Best-effort runtime-side self-identity bootstrap for daemon boots
/// that already have a live local runtime.
///
/// Why this exists:
/// - `easynet start` already bootstraps runtime key material before
///   republishing abilities.
/// - The heartbeat daemon also bootstraps before its first tick.
/// - `easynet-daemon` can, however, boot in shapes where neither of
///   those has fired yet while local CLI surfaces already route
///   through `BridgeAbilityInvoker` (`node.describe` ->
///   `federation.resolve`, `node.list`, etc.).
///
/// In that window the runtime rejects signed federation reads with
/// `AXON_EASYNET_SUBJECT_KEY_UNREGISTERED`. Bootstrapping here closes
/// the gap for any daemon boot that can already see a live runtime.
///
/// Best-effort by contract:
/// - no runtime state file -> silent skip (standalone daemon harnesses)
/// - runtime down / bridge connect fail -> log + continue
/// - bootstrap reject -> log + continue
///
/// The call is idempotent. If `easynet start` or the heartbeat daemon
/// already registered the keys, the runtime simply keeps the prior
/// entries and startup proceeds unchanged.
pub(super) fn maybe_bootstrap_runtime_self_identity(identity: &DaemonIdentity) {
    let Some(realm) = realm_from_agent_ura(&identity.caller_ura) else {
        return;
    };
    let Some(node_id) = device_id_from_caller_ura(&identity.caller_ura) else {
        return;
    };

    let state = match crate::daemon::persistence::config::load() {
        Ok(state) => state,
        Err(_) => return,
    };
    if matches!(
        state.runtime_kind,
        crate::daemon::persistence::config::RuntimeKind::DaemonOnly
    ) {
        return;
    }
    let bridge = match state.connect_bridge() {
        Ok(bridge) => bridge,
        Err(err) => {
            let err_msg = format!("{err}");
            crate::op_event!(
                component = daemon_invocation,
                kind = runtime_self_bootstrap_skipped,
                node_id = node_id,
                reason = "connect_local_runtime_bridge_failed",
                error = err_msg,
            );
            return;
        }
    };
    let invoker = crate::daemon::federation::advertise::BridgeAbilityInvoker::with_caller_ura(
        &bridge,
        identity.caller_ura.clone(),
    );
    match crate::daemon::federation::publish::bootstrap_self_identity_via_runtime(
        &invoker, &realm, &realm, &node_id,
    )
    .result
    {
        Ok(()) => {
            crate::op_event!(
                component = daemon_invocation,
                kind = runtime_self_bootstrap_registered,
                node_id = node_id,
            );
        }
        Err(msg) => {
            crate::op_event!(
                component = daemon_invocation,
                kind = runtime_self_bootstrap_failed,
                node_id = node_id,
                error = msg,
            );
        }
    }
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

    let expected = crate::core::ura::device_ura(realm, node_id);
    if let Some(agent_ura) = stored
        .agent_ura
        .as_deref()
        .map(str::trim)
        .filter(|ura| !ura.is_empty())
    {
        if agent_ura != expected {
            return None;
        }
    }

    Some(expected)
}

// URA v4.1.5: strict parsing via crate::core::ura::parse_ura per memory
// `feedback_no_legacy_ura.md`. The daemon's stored caller URA in
// v4.1.5 is always `easynet:///r/<realm>/device/<device-uuid>`
// (device-mode CLI's self-identity URA), so we only need to match
// that one shape.
//
// Legacy `r/{prv,org}/reg/agent.<id>?tenant_id=<t>` (URA v1) and
// `agent/<bare-id>` (URA v2 transitional) shapes are rejected —
// pre-v4.1.5 credential files cannot bind a canonical runtime owner; users
// must `easynet device join` again to mint a v4.1.5 credential. Boot fails
// closed when a Device-mode daemon has no valid canonical owner.
fn realm_from_agent_ura(ura: &str) -> Option<String> {
    let parsed = crate::core::ura::parse_ura(ura).ok()?;
    if parsed.realm.is_empty() {
        None
    } else {
        Some(parsed.realm)
    }
}

fn device_id_from_caller_ura(ura: &str) -> Option<String> {
    let parsed = crate::core::ura::parse_ura(ura).ok()?;
    // Only Device-kind URAs carry a device_id field; other kinds
    // leave it empty. Empty == not a device URA.
    parsed.device_id().map(str::to_string)
}
