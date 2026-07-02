//! On-miss trust sync for origin-caller claims.
//!
//! An executing device verifies a forwarded invocation's
//! `OriginCallerClaim` against its LOCAL realm trust anchor
//! (INV-1: admission is local-anchor-authoritative). But it cannot
//! pre-know every caller that may address it. Device keys are
//! registered at the realm hub (`register_device_pubkey` during
//! `device join`), and same-realm browser user keys are registered
//! there as `role = "user"`. This sync closes the gap with the same
//! authority direction as the session prelude: on an anchor MISS for a
//! syncable origin caller, PULL the key from the hub
//! (`federation.resolve_key`, routed through this daemon's
//! authenticated session channel) and import it through the same
//! `register_device_pubkey` write policy. Admission itself never
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

use crate::daemon::invocation::invoke_remote_initiator::RequestOutcome;
use crate::daemon::invocation::session_escalation::SessionEscalationHandle;
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
    /// `session.open` escalation channel — the SAME authenticated
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

#[derive(Clone, Debug, PartialEq, Eq)]
enum SyncableCaller {
    Device,
    User {
        presented_pubkey_b64: Option<String>,
    },
}

impl SyncableCaller {
    fn register_role(&self) -> &'static str {
        match self {
            Self::Device => "device",
            Self::User { .. } => "user",
        }
    }

    fn cache_key(&self, caller_ura: &str) -> String {
        match self {
            Self::Device => format!("device:{caller_ura}"),
            Self::User {
                presented_pubkey_b64,
            } => match presented_pubkey_b64 {
                Some(pk) => format!("user:{caller_ura}:{pk}"),
                None => format!("user:{caller_ura}:*"),
            },
        }
    }

    fn presented_pubkey_b64(&self) -> Option<&str> {
        match self {
            Self::Device => None,
            Self::User {
                presented_pubkey_b64,
            } => presented_pubkey_b64.as_deref(),
        }
    }
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

    async fn resolve_from_hub(
        &self,
        agent_ura: &str,
        presented_pubkey_b64: Option<&str>,
    ) -> anyhow::Result<Vec<String>> {
        match &self.source {
            KeySource::Session(handle) => {
                let mut args_value = serde_json::json!({ "agent_ura": agent_ura });
                if let Some(pk) = presented_pubkey_b64.filter(|pk| !pk.is_empty()) {
                    args_value["presented_pubkey_b64"] = serde_json::Value::String(pk.to_string());
                }
                let args = serde_json::to_vec(&args_value)?;
                let ability =
                    crate::daemon::invocation::federation_wrappers::ABILITY_FEDERATION_RESOLVE_KEY;
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

    /// Make a single-key device caller admissible if the realm hub attests it.
    pub async fn ensure_caller_key(&self, caller_ura: &str) -> bool {
        self.ensure_caller_key_with_presented_pubkey(caller_ura, None)
            .await
    }

    /// Make `caller_ura`'s key admissible if the realm hub attests it.
    /// Returns whether the anchor holds the required entry afterwards;
    /// `false` simply lets the claim dispatch fail closed downstream.
    ///
    /// For user callers, `presented_pubkey_b64` pins the check to the
    /// exact browser key carried by the signed origin-caller claim.
    pub async fn ensure_caller_key_with_presented_pubkey(
        &self,
        caller_ura: &str,
        presented_pubkey_b64: Option<&str>,
    ) -> bool {
        let Some(role) = self.syncable_caller(caller_ura, presented_pubkey_b64) else {
            return false;
        };
        if self.anchor_has_caller_key(caller_ura, &role) {
            return true;
        }
        let cache_key = role.cache_key(caller_ura);

        let mut state = self.state.lock().await;
        // Double-check under the lock: a concurrent miss may have
        // synced while this one waited.
        if self.anchor_has_caller_key(caller_ura, &role) {
            return true;
        }
        if let Some(failed_at) = state.get(&cache_key) {
            if failed_at.elapsed() < NEGATIVE_CACHE_TTL {
                return false;
            }
        }

        let keys = match self
            .resolve_from_hub(caller_ura, role.presented_pubkey_b64())
            .await
        {
            Ok(keys) if !keys.is_empty() => keys,
            Ok(_) => {
                state.insert(cache_key, Instant::now());
                return false;
            }
            Err(err) => {
                crate::op_event!(
                    component = device_trust_sync,
                    kind = resolve_failed,
                    caller_ura = caller_ura,
                    error = err.to_string(),
                );
                state.insert(cache_key, Instant::now());
                return false;
            }
        };

        let imported = self.import_caller_keys(caller_ura, &keys, &role);
        if imported {
            state.remove(&cache_key);
            crate::op_event!(
                component = device_trust_sync,
                kind = caller_key_synced,
                caller_ura = caller_ura,
                role = role.register_role(),
            );
        } else {
            state.insert(cache_key, Instant::now());
        }
        imported
    }

    fn syncable_caller(
        &self,
        caller_ura: &str,
        presented_pubkey_b64: Option<&str>,
    ) -> Option<SyncableCaller> {
        let parsed = crate::ura::parse_ura(caller_ura).ok()?;
        match parsed.kind {
            crate::ura::URAKind::Device => Some(SyncableCaller::Device),
            crate::ura::URAKind::User if parsed.realm == self.daemon_realm => {
                Some(SyncableCaller::User {
                    presented_pubkey_b64: presented_pubkey_b64
                        .filter(|pk| !pk.is_empty())
                        .map(str::to_string),
                })
            }
            _ => None,
        }
    }

    fn anchor_has_caller_key(&self, caller_ura: &str, role: &SyncableCaller) -> bool {
        let anchor = self.cell.snapshot();
        match role {
            SyncableCaller::Device => anchor.lookup(caller_ura).is_some(),
            SyncableCaller::User {
                presented_pubkey_b64: Some(pk),
            } => anchor.lookup_user_by_pubkey(caller_ura, pk).is_some(),
            SyncableCaller::User {
                presented_pubkey_b64: None,
            } => anchor.lookup(caller_ura).is_some(),
        }
    }

    /// Import hub-attested keys through the SAME write policy the gRPC
    /// surface and the prelude sync use, then report whether the anchor
    /// now resolves the caller.
    fn import_caller_keys(&self, caller_ura: &str, keys: &[String], role: &SyncableCaller) -> bool {
        for public_key_b64 in keys {
            let register_args = match serde_json::to_vec(&serde_json::json!({
                "agent_ura": caller_ura,
                "public_key_b64": public_key_b64,
                "role": role.register_role(),
            })) {
                Ok(v) => v,
                Err(_) => continue,
            };
            match crate::daemon::invocation::register_device_pubkey::handle(
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
                        role = role.register_role(),
                        error = status.message(),
                    );
                }
            }
        }
        self.anchor_has_caller_key(caller_ura, role)
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
    use crate::services::realm_trust_anchor::{RealmTrustAnchor, TrustedAgent, TrustedAgentRole};

    fn empty_cell() -> SharedTrustAnchor {
        SharedTrustAnchor::new(Arc::new(
            RealmTrustAnchor::from_entries(vec![]).expect("empty anchor"),
        ))
    }

    fn cell_with_user_key(user_ura: &str, public_key_b64: &str) -> SharedTrustAnchor {
        SharedTrustAnchor::new(Arc::new(
            RealmTrustAnchor::from_entries(vec![TrustedAgent {
                agent_ura: user_ura.to_string(),
                public_key_b64: public_key_b64.to_string(),
                role: TrustedAgentRole::User,
                added_at_unix_ms: 1_700_000_000_000,
                origin_realm: None,
                hub_endpoint: None,
                tls_ca_pem_path: None,
            }])
            .expect("user anchor"),
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
            sync.state
                .lock()
                .await
                .contains_key(&format!("device:{ura}")),
            "unknown caller must be negative-cached"
        );
        assert!(!sync.ensure_caller_key(ura).await);
    }

    #[tokio::test]
    async fn same_realm_user_miss_resolves_imports_presented_key() {
        fn resolver(_ura: &str) -> anyhow::Result<Vec<String>> {
            Ok(vec![B64.encode(
                SigningKey::from_bytes(&[0x51; 32])
                    .verifying_key()
                    .to_bytes(),
            )])
        }
        let dir = tempfile::tempdir().expect("tmp");
        let sync = sync_with(resolver, &dir);
        let user_ura = "easynet:///r/test-realm/user/alice";
        let presented = B64.encode(
            SigningKey::from_bytes(&[0x51; 32])
                .verifying_key()
                .to_bytes(),
        );

        assert!(
            sync.ensure_caller_key_with_presented_pubkey(user_ura, Some(&presented))
                .await,
            "hub-attested user key must be imported before admission"
        );
        assert!(sync
            .cell
            .snapshot()
            .lookup_user_by_pubkey(user_ura, &presented)
            .is_some());
    }

    #[tokio::test]
    async fn same_realm_user_existing_different_key_still_syncs_presented_key() {
        fn resolver(_ura: &str) -> anyhow::Result<Vec<String>> {
            Ok(vec![B64.encode(
                SigningKey::from_bytes(&[0x62; 32])
                    .verifying_key()
                    .to_bytes(),
            )])
        }
        let dir = tempfile::tempdir().expect("tmp");
        let user_ura = "easynet:///r/test-realm/user/alice";
        let old_key = B64.encode(
            SigningKey::from_bytes(&[0x61; 32])
                .verifying_key()
                .to_bytes(),
        );
        let presented = B64.encode(
            SigningKey::from_bytes(&[0x62; 32])
                .verifying_key()
                .to_bytes(),
        );
        let sync = DeviceTrustSync::with_source(
            "test-realm".into(),
            dir.path().join("realm-trust.toml"),
            cell_with_user_key(user_ura, &old_key),
            KeySource::Static(resolver),
        );

        assert!(
            sync.ensure_caller_key_with_presented_pubkey(user_ura, Some(&presented))
                .await,
            "presented browser key must not be hidden by another key under the same user URA"
        );
        let anchor = sync.cell.snapshot();
        assert_eq!(anchor.lookup_user_all(user_ura).len(), 2);
        assert!(anchor.lookup_user_by_pubkey(user_ura, &presented).is_some());
    }

    #[tokio::test]
    async fn non_syncable_callers_never_sync() {
        fn resolver(_ura: &str) -> anyhow::Result<Vec<String>> {
            panic!("resolver must not run for non-device callers");
        }
        let dir = tempfile::tempdir().expect("tmp");
        let sync = sync_with(resolver, &dir);
        assert!(!sync.ensure_caller_key("easynet:///r/test-realm/hub").await);
        assert!(
            !sync
                .ensure_caller_key_with_presented_pubkey(
                    "easynet:///r/other-realm/user/alice",
                    Some(&test_key_b64()),
                )
                .await
        );
    }
}
