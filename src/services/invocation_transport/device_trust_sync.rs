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
use std::time::{Duration, Instant};

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
    /// Resolver indirection so the import pipeline is unit-testable
    /// without a hub: production wires `resolve_key_via_local_daemon`.
    resolver: fn(&str) -> anyhow::Result<Vec<String>>,
}

impl DeviceTrustSync {
    #[must_use]
    pub fn new(daemon_realm: String, trust_anchor_path: PathBuf, cell: SharedTrustAnchor) -> Self {
        Self::with_resolver(
            daemon_realm,
            trust_anchor_path,
            cell,
            resolve_key_via_local_daemon,
        )
    }

    fn with_resolver(
        daemon_realm: String,
        trust_anchor_path: PathBuf,
        cell: SharedTrustAnchor,
        resolver: fn(&str) -> anyhow::Result<Vec<String>>,
    ) -> Self {
        Self {
            daemon_realm,
            trust_anchor_path,
            cell,
            state: tokio::sync::Mutex::new(HashMap::new()),
            resolver,
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

        let resolver = self.resolver;
        let ura = caller_ura.to_string();
        let resolved =
            tokio::task::spawn_blocking(move || resolver(&ura)).await;
        let keys = match resolved {
            Ok(Ok(keys)) if !keys.is_empty() => keys,
            Ok(Ok(_)) | Ok(Err(_)) | Err(_) => {
                if let Ok(Err(err)) = resolved {
                    crate::op_event!(
                        component = device_trust_sync,
                        kind = resolve_failed,
                        caller_ura = caller_ura,
                        error = err.to_string(),
                    );
                }
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

/// Resolve a device key from the realm hub through this daemon's own
/// Axon Invocation surface (the canonical client seam — same routing
/// the CLI uses, so hub escalation, TLS, and admission are uniform).
/// Blocking: call from `spawn_blocking`.
fn resolve_key_via_local_daemon(agent_ura: &str) -> anyhow::Result<Vec<String>> {
    let response = crate::support::local_invoke::invoke_local_ability(
        crate::services::invocation_transport::federation_wrappers::ABILITY_FEDERATION_RESOLVE_KEY,
        serde_json::json!({ "agent_ura": agent_ura }),
    )?;
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
        DeviceTrustSync::with_resolver(
            "test-realm".into(),
            dir.path().join("realm-trust.toml"),
            empty_cell(),
            resolver,
        )
    }

    fn test_key_b64() -> String {
        B64.encode(SigningKey::from_bytes(&[0x42; 32]).verifying_key().to_bytes())
    }

    #[tokio::test]
    async fn miss_resolves_imports_and_admits() {
        fn resolver(_ura: &str) -> anyhow::Result<Vec<String>> {
            Ok(vec![B64
                .encode(SigningKey::from_bytes(&[0x42; 32]).verifying_key().to_bytes())])
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
        assert!(!sync.ensure_caller_key("easynet:///r/test-realm/user/alice").await);
    }
}
