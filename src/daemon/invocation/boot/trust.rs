use std::path::Path;
use std::sync::Arc;

use base64::{engine::general_purpose::STANDARD as BASE64_STANDARD, Engine as _};
use ed25519_dalek::SigningKey;
use serde::Deserialize;

use crate::daemon::trust::anchor::{RealmTrustAnchor, TrustedAgent, TrustedAgentRole};
use crate::daemon::trust::cell::SharedTrustAnchor;
use crate::persistence::daemon_config::DaemonConfig;
use crate::services::usage_quota_store::SharedUsageQuotaGate;

pub(super) fn load_trust_anchor_from(path: &Path) -> RealmTrustAnchor {
    match RealmTrustAnchor::load_or_empty(path) {
        Ok(anchor) => {
            let path_display = format!("{}", path.display());
            if anchor.is_empty() {
                crate::op_event!(
                    component = daemon_invocation,
                    kind = realm_trust_anchor_empty,
                    path = path_display,
                    message = "admission gate will reject every external caller until PR-7 pairing flow populates it",
                );
            } else {
                let entry_count = anchor.len();
                crate::op_event!(
                    component = daemon_invocation,
                    kind = realm_trust_anchor_loaded,
                    path = path_display,
                    entries = entry_count,
                );
            }
            anchor
        }
        Err(err) => {
            let path_display = format!("{}", path.display());
            let err_msg = format!("{err}");
            crate::op_event!(
                component = daemon_invocation,
                kind = realm_trust_anchor_load_failed,
                path = path_display,
                error = err_msg,
                message = "proceeding with empty trust set",
            );
            RealmTrustAnchor::default()
        }
    }
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
pub(super) struct BackendIdentityRecord {
    pub private_key_seed_hex: String,
    #[serde(default)]
    pub agent_ura: String,
    #[serde(default, rename = "created_at_unix_ms")]
    pub _created_at_unix_ms: Option<u64>,
}

pub(super) fn upsert_backend_identity_from_disk(
    realm: &str,
    trust_anchor_path: &Path,
    mut anchor: RealmTrustAnchor,
) -> RealmTrustAnchor {
    let Some(record) = read_backend_identity_record(realm) else {
        return anchor;
    };
    let expected_ura = crate::ura::hub_ura(realm);
    if !record.agent_ura.trim().is_empty() && record.agent_ura != expected_ura {
        crate::op_event!(
            component = daemon_invocation,
            kind = backend_identity_trust_upsert_skipped,
            expected_ura = expected_ura,
            actual_ura = record.agent_ura,
            message = "backend identity file does not match daemon realm",
        );
        return anchor;
    }
    let seed = match decode_backend_identity_seed(&record.private_key_seed_hex) {
        Ok(seed) => seed,
        Err(err) => {
            crate::op_event!(
                component = daemon_invocation,
                kind = backend_identity_trust_upsert_failed,
                error = err,
                message = "backend identity seed is not usable",
            );
            return anchor;
        }
    };
    let signing_key = SigningKey::from_bytes(&seed);
    let entry = TrustedAgent {
        agent_ura: expected_ura.clone(),
        public_key_b64: BASE64_STANDARD.encode(signing_key.verifying_key().to_bytes()),
        role: TrustedAgentRole::Backend,
        added_at_unix_ms: now_unix_ms(),
        origin_realm: None,
        hub_endpoint: None,
        tls_ca_pem_path: None,
    };
    if let Err(err) = anchor.upsert_singleton_agent(entry) {
        crate::op_event!(
            component = daemon_invocation,
            kind = backend_identity_trust_upsert_failed,
            error = format!("{err}"),
            message = "failed to merge backend identity into trust anchor",
        );
        return anchor;
    }
    if let Err(err) = anchor.save(trust_anchor_path) {
        crate::op_event!(
            component = daemon_invocation,
            kind = backend_identity_trust_save_failed,
            path = format!("{}", trust_anchor_path.display()),
            error = format!("{err}"),
            message = "using backend identity in memory; disk trust anchor was not updated",
        );
    } else {
        crate::op_event!(
            component = daemon_invocation,
            kind = backend_identity_trust_upserted,
            path = format!("{}", trust_anchor_path.display()),
            agent_ura = expected_ura,
            message = "backend identity public key is present in trust anchor",
        );
    }
    anchor
}

pub(super) fn read_backend_identity_record(realm: &str) -> Option<BackendIdentityRecord> {
    let home = std::env::var_os("HOME")?;
    let path = Path::new(&home)
        .join(".easynet-hub")
        .join(realm)
        .join("identity.json");
    let raw = match std::fs::read_to_string(&path) {
        Ok(raw) => raw,
        Err(err) if err.kind() == std::io::ErrorKind::NotFound => return None,
        Err(err) => {
            crate::op_event!(
                component = daemon_invocation,
                kind = backend_identity_trust_upsert_failed,
                path = format!("{}", path.display()),
                error = format!("{err}"),
                message = "failed to read backend identity file",
            );
            return None;
        }
    };
    match serde_json::from_str(&raw) {
        Ok(record) => Some(record),
        Err(err) => {
            crate::op_event!(
                component = daemon_invocation,
                kind = backend_identity_trust_upsert_failed,
                path = format!("{}", path.display()),
                error = format!("{err}"),
                message = "failed to parse backend identity file",
            );
            None
        }
    }
}

fn decode_backend_identity_seed(raw: &str) -> Result<[u8; 32], String> {
    let bytes = hex::decode(raw.trim()).map_err(|err| format!("seed hex decode failed: {err}"))?;
    bytes
        .as_slice()
        .try_into()
        .map_err(|_| format!("seed must decode to 32 bytes, got {}", bytes.len()))
}

fn now_unix_ms() -> u64 {
    use std::time::{SystemTime, UNIX_EPOCH};
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_millis() as u64)
        .unwrap_or(0)
}

pub(super) fn reload_trust_anchor_cell_from(
    path: &Path,
    trust_anchor_cell: &SharedTrustAnchor,
) -> anyhow::Result<usize> {
    let next = RealmTrustAnchor::load_or_empty(path)
        .map_err(|err| anyhow::anyhow!("load trust anchor from {}: {err}", path.display()))?;
    let len = next.len();
    trust_anchor_cell.replace(Arc::new(next));
    Ok(len)
}

pub(super) struct ReloadedDaemonConfigCells {
    pub federated_peers_len: usize,
    pub quota_configured: bool,
}

/// Re-parse daemon-config TOML at `path` and republish all live cells
/// that are intentionally SIGHUP-managed from that file.
pub(super) fn reload_daemon_config_cells_from(
    path: &Path,
    federated_peers_cell: &crate::daemon::federation::peers::SharedFederatedPeers,
    quota_gate: &SharedUsageQuotaGate,
) -> anyhow::Result<ReloadedDaemonConfigCells> {
    let next_config = DaemonConfig::load(path)
        .map_err(|err| anyhow::anyhow!("reload daemon-config from {}: {err}", path.display()))?;
    let next_peers = next_config.federated_peers().clone();
    let len = next_peers.len();
    federated_peers_cell.replace(next_peers);

    let next_quota = next_config.quota().cloned();
    let quota_configured = next_quota.is_some();
    quota_gate.replace_policy(next_quota);

    Ok(ReloadedDaemonConfigCells {
        federated_peers_len: len,
        quota_configured,
    })
}
