use crate::daemon::invocation::session_initiator::SessionSigningSeed;
use crate::runtime::publish::derive_subject_keypair;

use super::paths::expand_home;

#[derive(Debug, Clone)]
pub(super) struct DaemonIdentity {
    pub caller_ura: String,
    pub signing_seed: Option<SessionSigningSeed>,
}

/// Narrow read-projection of `~/.easynet/credentials.json` carrying
/// only the three fields the daemon needs to derive its caller URA +
/// signing seed.
///
/// MUST NOT use `#[serde(deny_unknown_fields)]`. The writer
/// (`persistence::config::Credentials`) owns the file and its field
/// set grows over time — `credential_token`, `hub_endpoint`,
/// `hub_api_base`, `username`, `hub_pubkey_b64`, `hub_tls_ca_pem_b64`
/// were all added after this projection. A strict reader would reject
/// the whole file the moment any such field appears, silently
/// collapsing `load_daemon_identity()` to `None` (the `.ok()?` at the
/// call site). That drops the daemon's device identity, so the
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

/// Resolve the daemon's caller URA plus the optional deterministic
/// signing seed from `~/.easynet/credentials.json`.
///
/// Contract:
/// - credentials must carry `(realm, node_id)`.
/// - `tenant_id` is a retired field and is rejected by serde.
/// - `agent_ura`, when present, is only a consistency checksum; it is
///   never a fallback identity.
/// - once we have the canonical `(realm, node_id)` pair, derive the same
///   deterministic Ed25519 seed the SDK uses for
///   `easynet:prv:reg:agent.<node>`.
pub(super) fn load_daemon_identity() -> Option<DaemonIdentity> {
    let path = expand_home("~/.easynet/credentials.json");
    let raw = std::fs::read_to_string(&path).ok()?;
    let stored: StoredDeviceIdentity = serde_json::from_str(&raw).ok()?;
    daemon_identity_from_stored(&stored)
}

pub(super) fn daemon_identity_from_stored(stored: &StoredDeviceIdentity) -> Option<DaemonIdentity> {
    let caller_ura = canonical_caller_ura_from_stored_identity(stored)?;

    let realm = stored
        .realm
        .as_deref()
        .map(str::trim)
        .filter(|realm| !realm.is_empty())
        .map(str::to_string);
    let node_id = stored
        .node_id
        .as_deref()
        .map(str::trim)
        .filter(|node| !node.is_empty())
        .map(str::to_string)
        .or_else(|| device_id_from_caller_ura(&caller_ura));

    // Phase 3D: prefer the keyring vault's seed when the operator
    // has opted in via EASYNET_KEYRING_PASSPHRASE. The vault's
    // primary_self for this device is `caller_ura`; the role
    // overlay also matches HubURI(realm) on the same host, so
    // backend (Go side, Phase 3D's Go reader) and daemon (Rust
    // side here) end up signing with the **same** Ed25519 seed.
    //
    // Misses (env unset, vault file missing, this URA not in
    // vault) silently fall through to the v4.1.4 deterministic
    // derive — operators who have not yet rolled their daemons
    // onto the keyring stay unaffected.
    let signing_seed = if let Some(seed) = try_load_daemon_seed_from_keyring(&caller_ura) {
        Some(seed)
    } else {
        match (realm.as_deref(), node_id.as_deref()) {
            (Some(realm), Some(node_id)) => {
                let subject_id = easynet_axon::invocation::private_agent_subject_id(node_id);
                Some(derive_subject_keypair(realm, &subject_id).0)
            }
            _ => None,
        }
    };

    Some(DaemonIdentity {
        caller_ura,
        signing_seed,
    })
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

    let state = match crate::persistence::config::load() {
        Ok(state) => state,
        Err(_) => return,
    };
    if matches!(
        state.runtime_kind,
        crate::persistence::config::RuntimeKind::DaemonOnly
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
    let invoker = crate::runtime::advertise::BridgeAbilityInvoker::with_caller_ura(
        &bridge,
        identity.caller_ura.clone(),
    );
    match crate::runtime::publish::bootstrap_self_identity_via_runtime(
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

fn try_load_daemon_seed_from_keyring(self_ura: &str) -> Option<[u8; 32]> {
    use crate::daemon::keyring::{MasterKeySource, Vault, VaultError};

    std::env::var("EASYNET_KEYRING_PASSPHRASE")
        .ok()
        .filter(|v| !v.is_empty())?;
    let path = if let Ok(p) = std::env::var("EASYNET_KEYRING_VAULT_PATH") {
        std::path::PathBuf::from(p)
    } else {
        expand_home(&format!("~/{}", crate::daemon::keyring::DEFAULT_VAULT_REL))
    };
    if !path.exists() {
        return None;
    }
    let source = match MasterKeySource::from_env() {
        Ok(s) => s,
        Err(err) => {
            let err_msg = format!("{err}");
            crate::op_event!(
                component = daemon_invocation,
                kind = keyring_master_key_source_failed,
                error = err_msg,
            );
            return None;
        }
    };
    let vault = match Vault::open(&path, &source) {
        Ok(v) => v,
        Err(VaultError::NotFound(_)) => return None,
        Err(err) => {
            let err_msg = format!("{err}");
            crate::op_event!(
                component = daemon_invocation,
                kind = keyring_open_failed,
                error = err_msg,
            );
            return None;
        }
    };
    match vault.export_seed(self_ura) {
        Ok(seed) => {
            crate::op_event!(
                component = daemon_invocation,
                kind = keyring_daemon_seed_resolved,
                self_ura = self_ura,
            );
            Some(seed)
        }
        Err(VaultError::NotFound(_)) => None,
        Err(err) => {
            let err_msg = format!("{err}");
            crate::op_event!(
                component = daemon_invocation,
                kind = keyring_export_seed_failed,
                self_ura = self_ura,
                error = err_msg,
            );
            None
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

    let expected = crate::ura::device_ura(realm, node_id);
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

// URA v4.1.5: strict parsing via crate::ura::parse_ura per memory
// `feedback_no_legacy_ura.md`. The daemon's stored caller URA in
// v4.1.5 is always `easynet:///r/<realm>/device/<device-uuid>`
// (device-mode CLI's self-identity URA), so we only need to match
// that one shape.
//
// Legacy `r/{prv,org}/reg/agent.<id>?tenant_id=<t>` (URA v1) and
// `agent/<bare-id>` (URA v2 transitional) shapes are rejected —
// pre-v4.1.5 credential files cannot bootstrap signing seeds; users
// must `easynet device join` again to mint a v4.1.5 credential.
// Returning `None` triggers the parent code's "skip signing seed"
// branch (CLI starts unsigned, harmless in dev).
fn realm_from_agent_ura(ura: &str) -> Option<String> {
    let parsed = crate::ura::parse_ura(ura).ok()?;
    if parsed.realm.is_empty() {
        None
    } else {
        Some(parsed.realm)
    }
}

fn device_id_from_caller_ura(ura: &str) -> Option<String> {
    let parsed = crate::ura::parse_ura(ura).ok()?;
    // Only Device-kind URAs carry a device_id field; other kinds
    // leave it empty. Empty == not a device URA.
    parsed.device_id().map(str::to_string)
}
