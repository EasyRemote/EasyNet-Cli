//! On-miss device trust sync for cross-device origin-caller claims.
//!
//! An executing device verifies a forwarded invocation's
//! `OriginCallerClaim` against its LOCAL realm trust anchor
//! (INV-1: admission is local-anchor-authoritative). But it cannot
//! pre-know every device that may address it — device keys are
//! registered at the realm hub (`register_device_pubkey` during
//! `device join`). This sync closes the gap exactly like the paired
//! user's key sync (`UserTrustSync`), with the same authority
//! direction: on an anchor MISS for a claim's device caller, PULL the
//! key from the hub (`federation.resolve_key`, routed through this
//! daemon's own Axon Invocation surface) and import it through the
//! same `register_device_pubkey` write policy. Admission itself never
//! consults the network — the anchor is just kept warm.
//!
//! Hygiene: syncs are serialized (single-flight — a burst of frames
//! from one unknown caller triggers one resolve), and a hub that does
//! not know the caller is remembered briefly so repeated probes from
//! an unregistered device cannot turn into a resolve storm. Every
//! failure leaves the dispatch path to fail closed with the precise
//! admission error.

use std::collections::HashMap;
use std::path::PathBuf;
use std::sync::Arc;
use std::time::{Duration, Instant};

use crate::services::invocation_transport::invoke_remote_initiator::RequestOutcome;
use crate::services::invocation_transport::session_escalation::SessionEscalationHandle;
use crate::services::trust_anchor_cell::SharedTrustAnchor;

/// How long a hub "unknown caller" answer suppresses re-resolving the
/// same URA. Long enough to bound storms, short enough that a freshly
/// joined device becomes admissible without operator action.
const NEGATIVE_CACHE_TTL: Duration = Duration::from_secs(30);

pub struct DeviceTrustSync {
    daemon_realm: String,
    trust_anchor_path: PathBuf,
    cell: SharedTrustAnchor,
    /// Single-flight + negative cache. Holding the lock across the
    /// resolve serializes concurrent misses; entries record the last
    /// failed resolve per caller URA.
    state: tokio::sync::Mutex<HashMap<String, Instant>>,
    /// Where hub-attested keys come from. Production uses the
    /// `<self>.session` escalation channel — the SAME authenticated
    /// hub channel the paired-user sync and hot-agent advertising
    /// use. A device-local `federation.resolve_key` invoke would be
    /// answered from THIS daemon's own anchor (the local ability
    /// shadows hub routing) and can never learn a new key.
    source: KeySource,
}

enum KeySource {
    Session(Arc<SessionEscalationHandle>),
    /// Test seam: a pure function standing in for the hub.
    #[allow(dead_code)]
    Static(fn(&str) -> anyhow::Result<Vec<String>>),
}

impl DeviceTrustSync {
    #[must_use]
    pub fn new(
        daemon_realm: String,
        trust_anchor_path: PathBuf,
        cell: SharedTrustAnchor,
        escalation: Arc<SessionEscalationHandle>,
    ) -> Self {
        Self::with_source(
            daemon_realm,
            trust_anchor_path,
            cell,
            KeySource::Session(escalation),
        )
    }

    fn with_source(
        daemon_realm: String,
        trust_anchor_path: PathBuf,
        cell: SharedTrustAnchor,
        source: KeySource,
    ) -> Self {
        Self {
            daemon_realm,
            trust_anchor_path,
            cell,
            state: tokio::sync::Mutex::new(HashMap::new()),
            source,
        }
    }

    /// Test seam for sibling-module tests: a sync whose hub
    /// round-trip is a pure function. The service's self-targeted
    /// dispatch arm pins its warm-on-miss contract against this
    /// (`daemon_invocation_service` tests); production construction
    /// stays escalation-only via `new`.
    #[cfg(test)]
    pub(crate) fn with_static_source_for_tests(
        daemon_realm: String,
        trust_anchor_path: PathBuf,
        cell: SharedTrustAnchor,
        resolver: fn(&str) -> anyhow::Result<Vec<String>>,
    ) -> Self {
        Self::with_source(
            daemon_realm,
            trust_anchor_path,
            cell,
            KeySource::Static(resolver),
        )
    }

    async fn resolve_from_hub(&self, agent_ura: &str) -> anyhow::Result<Vec<String>> {
        match &self.source {
            KeySource::Session(handle) => {
                let args = serde_json::to_vec(&serde_json::json!({ "agent_ura": agent_ura }))?;
                let ability = crate::services::invocation_transport::federation_wrappers::ABILITY_FEDERATION_RESOLVE_KEY;
                match handle.escalate(ability.to_string(), args).await {
                    RequestOutcome::Ok { result_bytes } => parse_resolved_keys(&result_bytes),
                    RequestOutcome::Err { error } => {
                        anyhow::bail!("hub resolve_key failed: {error:?}")
                    }
                }
            }
            KeySource::Static(resolver) => resolver(agent_ura),
        }
    }

    /// Make `caller_ura`'s key admissible if the realm hub attests it.
    /// Returns whether the anchor holds an entry afterwards; `false`
    /// simply lets the claim dispatch fail closed downstream.
    pub async fn ensure_caller_key(&self, caller_ura: &str) -> bool {
        if self.cell.snapshot().lookup(caller_ura).is_some() {
            return true;
        }
        // Only DEVICE callers sync here: user keys have their own
        // session-lifetime sync, and anything else is not a key the
        // hub registers.
        let is_device = crate::ura::parse_ura(caller_ura)
            .map(|parsed| parsed.kind == crate::ura::URAKind::Device)
            .unwrap_or(false);
        if !is_device {
            return false;
        }

        let mut state = self.state.lock().await;
        // Double-check under the lock: a concurrent miss may have
        // synced while this one waited.
        if self.cell.snapshot().lookup(caller_ura).is_some() {
            return true;
        }
        if let Some(failed_at) = state.get(caller_ura) {
            if failed_at.elapsed() < NEGATIVE_CACHE_TTL {
                return false;
            }
        }

        let keys = match self.resolve_from_hub(caller_ura).await {
            Ok(keys) if !keys.is_empty() => keys,
            Ok(_) => {
                state.insert(caller_ura.to_string(), Instant::now());
                return false;
            }
            Err(err) => {
                crate::op_event!(
                    component = device_trust_sync,
                    kind = resolve_failed,
                    caller_ura = caller_ura,
                    error = err.to_string(),
                );
                state.insert(caller_ura.to_string(), Instant::now());
                return false;
            }
        };

        let imported = self.import_device_keys(caller_ura, &keys);
        if imported {
            state.remove(caller_ura);
            crate::op_event!(
                component = device_trust_sync,
                kind = device_key_synced,
                caller_ura = caller_ura,
            );
        } else {
            state.insert(caller_ura.to_string(), Instant::now());
        }
        imported
    }

    /// Import hub-attested device keys through the SAME write policy
    /// the gRPC surface and the user sync use, then report whether
    /// the anchor now resolves the caller.
    fn import_device_keys(&self, caller_ura: &str, keys: &[String]) -> bool {
        for public_key_b64 in keys {
            let register_args = match serde_json::to_vec(&serde_json::json!({
                "agent_ura": caller_ura,
                "public_key_b64": public_key_b64,
                "role": "device",
            })) {
                Ok(v) => v,
                Err(_) => continue,
            };
            match crate::services::invocation_transport::register_device_pubkey::handle(
                &register_args,
                &self.daemon_realm,
                &self.trust_anchor_path,
                &self.cell,
            ) {
                Ok(_) => {}
                Err(status) if status.code() == tonic::Code::AlreadyExists => {}
                Err(status) => {
                    crate::op_event!(
                        component = device_trust_sync,
                        kind = import_rejected,
                        caller_ura = caller_ura,
                        error = status.message(),
                    );
                }
            }
        }
        self.cell.snapshot().lookup(caller_ura).is_some()
    }
}

/// Parse the hub's resolve_key reply: prefer the multi-key field
/// (DEC-EU), fall back to the single-key field of older hubs — the
/// same tolerance the paired-user sync applies.
fn parse_resolved_keys(result_bytes: &[u8]) -> anyhow::Result<Vec<String>> {
    let response: serde_json::Value = serde_json::from_slice(result_bytes)?;
    let mut keys: Vec<String> = response
        .get("public_keys_b64")
        .and_then(|v| v.as_array())
        .map(|arr| {
            arr.iter()
                .filter_map(|k| k.as_str().map(str::to_string))
                .collect()
        })
        .unwrap_or_default();
    if keys.is_empty() {
        if let Some(pk) = response.get("public_key_b64").and_then(|v| v.as_str()) {
            keys.push(pk.to_string());
        }
    }
    Ok(keys)
}

#[cfg(test)]
mod tests {
    use std::sync::Arc;

    use base64::{engine::general_purpose::STANDARD as B64, Engine as _};
    use ed25519_dalek::SigningKey;

    use super::*;
    use crate::services::realm_trust_anchor::RealmTrustAnchor;

    fn empty_cell() -> SharedTrustAnchor {
        SharedTrustAnchor::new(Arc::new(
            RealmTrustAnchor::from_entries(vec![]).expect("empty anchor"),
        ))
    }

    fn sync_with(
        resolver: fn(&str) -> anyhow::Result<Vec<String>>,
        dir: &tempfile::TempDir,
    ) -> DeviceTrustSync {
        DeviceTrustSync::with_source(
            "test-realm".into(),
            dir.path().join("realm-trust.toml"),
            empty_cell(),
            KeySource::Static(resolver),
        )
    }

    fn test_key_b64() -> String {
        B64.encode(
            SigningKey::from_bytes(&[0x42; 32])
                .verifying_key()
                .to_bytes(),
        )
    }

    #[tokio::test]
    async fn miss_resolves_imports_and_admits() {
        fn resolver(_ura: &str) -> anyhow::Result<Vec<String>> {
            Ok(vec![B64.encode(
                SigningKey::from_bytes(&[0x42; 32])
                    .verifying_key()
                    .to_bytes(),
            )])
        }
        let dir = tempfile::tempdir().expect("tmp");
        let sync = sync_with(resolver, &dir);
        let ura = "easynet:///r/test-realm/device/node-a";

        assert!(sync.ensure_caller_key(ura).await, "synced key must admit");
        assert!(sync.cell.snapshot().lookup(ura).is_some());
        // Second call is an anchor hit (no resolve needed to prove —
        // returns true immediately).
        assert!(sync.ensure_caller_key(ura).await);
        let _ = test_key_b64();
    }

    #[tokio::test]
    async fn hub_unknown_fails_closed_and_is_negative_cached() {
        fn resolver(_ura: &str) -> anyhow::Result<Vec<String>> {
            Ok(vec![])
        }
        let dir = tempfile::tempdir().expect("tmp");
        let sync = sync_with(resolver, &dir);
        let ura = "easynet:///r/test-realm/device/unknown";

        assert!(!sync.ensure_caller_key(ura).await);
        assert!(
            sync.state.lock().await.contains_key(ura),
            "unknown caller must be negative-cached"
        );
        assert!(!sync.ensure_caller_key(ura).await);
    }

    #[tokio::test]
    async fn non_device_callers_never_sync() {
        fn resolver(_ura: &str) -> anyhow::Result<Vec<String>> {
            panic!("resolver must not run for non-device callers");
        }
        let dir = tempfile::tempdir().expect("tmp");
        let sync = sync_with(resolver, &dir);
        assert!(
            !sync
                .ensure_caller_key("easynet:///r/test-realm/user/alice")
                .await
        );
    }
}
