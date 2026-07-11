use std::path::Path;
use std::sync::Arc;

use base64::{engine::general_purpose::STANDARD as BASE64_STANDARD, Engine as _};

use crate::daemon::identity::self_identity::CanonicalSigner;
use crate::daemon::invocation::admission::usage_quota::SharedUsageQuotaGate;
use crate::daemon::persistence::daemon_config::DaemonConfig;
use crate::daemon::trust::anchor::{RealmTrustAnchor, TrustedAgent, TrustedAgentRole};
use crate::daemon::trust::cell::SharedTrustAnchor;

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

pub(super) fn upsert_hub_identity(
    realm: &str,
    signer: &dyn CanonicalSigner,
    trust_anchor_path: &Path,
    mut anchor: RealmTrustAnchor,
) -> RealmTrustAnchor {
    let expected_ura = crate::core::ura::hub_ura(realm);
    if signer.owner_ura() != expected_ura {
        crate::op_event!(
            component = daemon_invocation,
            kind = hub_identity_trust_upsert_failed,
            expected_ura = expected_ura,
            signer_owner_ura = signer.owner_ura(),
            message = "refusing to publish a public key from a differently bound signer",
        );
        return anchor;
    }
    let public_key = match signer.signing_public_key() {
        Ok(public_key) => public_key,
        Err(err) => {
            crate::op_event!(
                component = daemon_invocation,
                kind = hub_identity_trust_upsert_failed,
                agent_ura = expected_ura,
                error = format!("{err}"),
                message = "Hub runtime identity public projection is unavailable",
            );
            return anchor;
        }
    };
    let entry = TrustedAgent {
        agent_ura: expected_ura.clone(),
        public_key_b64: BASE64_STANDARD.encode(public_key.to_bytes()),
        role: TrustedAgentRole::Hub,
        added_at_unix_ms: now_unix_ms(),
        origin_realm: None,
        hub_endpoint: None,
        tls_ca_pem_path: None,
    };
    if let Err(err) = anchor.upsert_singleton_agent(entry) {
        crate::op_event!(
            component = daemon_invocation,
            kind = hub_identity_trust_upsert_failed,
            error = format!("{err}"),
            message = "failed to merge backend identity into trust anchor",
        );
        return anchor;
    }
    if let Err(err) = anchor.save(trust_anchor_path) {
        crate::op_event!(
            component = daemon_invocation,
            kind = hub_identity_trust_save_failed,
            path = format!("{}", trust_anchor_path.display()),
            error = format!("{err}"),
            message = "using backend identity in memory; disk trust anchor was not updated",
        );
    } else {
        crate::op_event!(
            component = daemon_invocation,
            kind = hub_identity_trust_upserted,
            path = format!("{}", trust_anchor_path.display()),
            agent_ura = expected_ura,
            message = "Hub runtime identity public key is present in trust anchor",
        );
    }
    anchor
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
