// EasyNet CLI — Federated user binding store (PR-N4 commit 3/N)
// =================================================================
//
// File: src/daemon/keyring/federated_bindings.rs
// Description: In-memory + on-disk store for cross-realm user
//              identity bindings. The consumer-side counterpart to
//              `user_binding_chain.rs` (commit 1/N): when a realm B
//              user successfully consumes a `UserBindingToken`
//              issued by realm A, an entry is written here mapping
//              `(source_realm, source_user_ura, source_user_pubkey)
//              → realm_B_user_id`. Future cross-realm
//              `<agent>.discover` calls filter their results
//              through the binding to surface "the user's own
//              devices, both realms".
//
// Wire shape vs. RFC-002 keyring entries
// --------------------------------------
// RFC-002 `Entry` rows are device-key material. Bindings are
// user-identity assertions imported from peer realms — different
// schema, different audit boundary. Mixing them in one
// `KeyRing.entries` table would conflate "this device's keys" with
// "this hub's view of who-is-who in other realms"; a binding is
// not a key the local daemon owns.
//
// Persistence story
// -----------------
// v1 stores bindings in `~/.easynet/federated_bindings.json` —
// plain JSON, atomic write via temp-file-rename. RFC-N v2 may
// move to the keyring's encrypted store once the schema
// stabilises; for now the binding is non-secret (it IS public-
// key material), so plaintext is acceptable.
//
// Replay defence
// --------------
// The store tracks consumed nonces per `(source_realm, nonce)`
// tuple. A second consume of the same token surfaces
// `UserBindingError::ReplayDetected`. Nonce retention is
// per-source-realm to bound state growth; production daemons
// can prune entries older than `2 * USER_BINDING_FRESHNESS_MS`.
//
// Author: Silan.Hu <silan.hu@u.nus.edu>
// Copyright (c) 2026 EasyNet. All rights reserved.

use std::collections::{BTreeMap, BTreeSet};
use std::path::PathBuf;
use std::sync::{Arc, RwLock};

use anyhow::{anyhow, Context, Result};
use serde::{Deserialize, Serialize};

/// One federated-user-binding record. Written by realm B's
/// `device.keyring.consume_federate_user_token` after the four-
/// check verify chain passes.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct FederatedUserBinding {
    /// Realm of the issuing daemon (the source_realm field of
    /// the consumed `UserBindingToken`).
    pub source_realm: String,
    /// User URA on the source realm.
    pub source_user_ura: String,
    /// Source user's Ed25519 verifying key, base64-encoded for
    /// stable JSON. The structural length contract (32 bytes)
    /// is enforced on the upstream consume path.
    pub source_user_pubkey_b64: String,
    /// Realm B's local user id this binding maps to. Comes from
    /// the consumer's authenticated session at consume time.
    pub local_user_id: String,
    /// When the binding was written. Useful for operator audit;
    /// PR-N5 will fold this into a federated audit chain.
    pub bound_at_unix_ms: i64,
}

/// On-disk JSON shape for the binding store.
#[derive(Debug, Default, Serialize, Deserialize)]
struct StoreFile {
    bindings: Vec<FederatedUserBinding>,
    /// Per-source-realm consumed-nonce set, base64-encoded.
    /// `BTreeMap<source_realm, BTreeSet<nonce_b64>>`.
    #[serde(default)]
    consumed_nonces: BTreeMap<String, BTreeSet<String>>,
}

/// Reload-friendly handle on the federated-bindings store.
/// Cloning is cheap (Arc-shaped); production callers wire one
/// instance per daemon process and clone it into the consume
/// ability handler.
#[derive(Clone, Debug)]
pub struct FederatedBindingsStore {
    inner: Arc<RwLock<StoreFile>>,
    path: Arc<PathBuf>,
}

impl FederatedBindingsStore {
    /// Open or create the store file at `path`. Missing file ⇒
    /// fresh empty store (the daemon's first cross-realm
    /// consume creates the file).
    pub fn open_or_create(path: PathBuf) -> Result<Self> {
        let file: StoreFile = if path.exists() {
            let bytes = std::fs::read(&path).with_context(|| format!("read {}", path.display()))?;
            if bytes.is_empty() {
                StoreFile::default()
            } else {
                serde_json::from_slice(&bytes)
                    .with_context(|| format!("parse {}", path.display()))?
            }
        } else {
            StoreFile::default()
        };
        Ok(Self {
            inner: Arc::new(RwLock::new(file)),
            path: Arc::new(path),
        })
    }

    /// In-memory empty store with no on-disk persistence. Used
    /// by tests + smoke runs that don't want a tempdir.
    #[must_use]
    pub fn in_memory() -> Self {
        Self {
            inner: Arc::new(RwLock::new(StoreFile::default())),
            path: Arc::new(PathBuf::new()),
        }
    }

    /// Snapshot all current bindings. Each call clones the
    /// underlying Vec for isolation; the cell is the source of
    /// truth and concurrent writers don't disturb in-flight
    /// readers.
    #[must_use]
    pub fn list(&self) -> Vec<FederatedUserBinding> {
        self.inner.read().expect("rwlock poisoned").bindings.clone()
    }

    /// Find the local user id bound to `(source_realm,
    /// source_user_ura)`. Returns `None` when no such binding
    /// has been consumed.
    #[must_use]
    pub fn find_local_user(&self, source_realm: &str, source_user_ura: &str) -> Option<String> {
        self.inner
            .read()
            .expect("rwlock poisoned")
            .bindings
            .iter()
            .find(|b| b.source_realm == source_realm && b.source_user_ura == source_user_ura)
            .map(|b| b.local_user_id.clone())
    }

    /// Has the given nonce already been consumed for
    /// `source_realm`? Used by `consume_federate_user_token` to
    /// implement INV-3 replay defence.
    #[must_use]
    pub fn nonce_seen(&self, source_realm: &str, nonce_b64: &str) -> bool {
        self.inner
            .read()
            .expect("rwlock poisoned")
            .consumed_nonces
            .get(source_realm)
            .is_some_and(|set| set.contains(nonce_b64))
    }

    /// Atomically write a new binding + remember its nonce.
    /// Persistence is best-effort: when constructed via
    /// `in_memory`, no on-disk write occurs (no path).
    pub fn record_binding(&self, binding: FederatedUserBinding, nonce_b64: String) -> Result<()> {
        let mut guard = self.inner.write().expect("rwlock poisoned");
        let nonces = guard
            .consumed_nonces
            .entry(binding.source_realm.clone())
            .or_default();
        if !nonces.insert(nonce_b64) {
            return Err(anyhow!(
                "nonce already consumed for source_realm `{}`",
                binding.source_realm
            ));
        }
        guard.bindings.push(binding);
        if !self.path.as_os_str().is_empty() {
            let bytes =
                serde_json::to_vec_pretty(&*guard).context("serialise federated bindings")?;
            if let Some(parent) = self.path.parent() {
                std::fs::create_dir_all(parent)
                    .with_context(|| format!("create parent dir for {}", self.path.display()))?;
            }
            crate::daemon::persistence::config::atomic_write(self.path.as_path(), &bytes)?;
        }
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn binding_fixture() -> FederatedUserBinding {
        FederatedUserBinding {
            source_realm: "realm-a".to_string(),
            source_user_ura: "easynet:///r/realm-a/user/user-c".to_string(),
            source_user_pubkey_b64: "AAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAA=".to_string(),
            local_user_id: "user-c-on-realm-b".to_string(),
            bound_at_unix_ms: 1_714_500_000_000,
        }
    }

    #[test]
    fn in_memory_store_starts_empty() {
        let store = FederatedBindingsStore::in_memory();
        assert!(store.list().is_empty());
    }

    #[test]
    fn record_binding_then_lookup_by_source() {
        let store = FederatedBindingsStore::in_memory();
        let b = binding_fixture();
        store
            .record_binding(b.clone(), "nonce-base64-a".to_string())
            .unwrap();
        assert_eq!(
            store
                .find_local_user(&b.source_realm, &b.source_user_ura)
                .as_deref(),
            Some("user-c-on-realm-b")
        );
    }

    #[test]
    fn nonce_seen_dedups_within_source_realm() {
        let store = FederatedBindingsStore::in_memory();
        assert!(!store.nonce_seen("realm-a", "n"));
        store
            .record_binding(binding_fixture(), "n".to_string())
            .unwrap();
        assert!(store.nonce_seen("realm-a", "n"));
        // Same nonce in a different realm is independent.
        assert!(!store.nonce_seen("realm-c", "n"));
    }

    #[test]
    fn record_binding_with_duplicate_nonce_errors() {
        let store = FederatedBindingsStore::in_memory();
        store
            .record_binding(binding_fixture(), "n".to_string())
            .unwrap();
        let err = store
            .record_binding(binding_fixture(), "n".to_string())
            .expect_err("duplicate nonce must reject");
        assert!(err.to_string().contains("already consumed"));
    }

    #[test]
    fn open_or_create_round_trips_through_disk() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("federated_bindings.json");
        let store = FederatedBindingsStore::open_or_create(path.clone()).unwrap();
        store
            .record_binding(binding_fixture(), "n".to_string())
            .unwrap();
        // Reopen — the binding should persist.
        let store2 = FederatedBindingsStore::open_or_create(path).unwrap();
        let bindings = store2.list();
        assert_eq!(bindings.len(), 1);
        assert_eq!(bindings[0].local_user_id, "user-c-on-realm-b");
        assert!(store2.nonce_seen("realm-a", "n"));
    }

    #[test]
    fn list_returns_clone_so_callers_dont_hold_lock() {
        let store = FederatedBindingsStore::in_memory();
        store
            .record_binding(binding_fixture(), "n".to_string())
            .unwrap();
        let snap_a = store.list();
        // A subsequent record_binding call must not deadlock —
        // proves list() does not hold the read lock.
        store
            .record_binding(
                FederatedUserBinding {
                    source_user_ura: "easynet:///r/realm-a/user/another".to_string(),
                    ..binding_fixture()
                },
                "n2".to_string(),
            )
            .unwrap();
        assert_eq!(snap_a.len(), 1, "snapshot is independent of later writes");
        assert_eq!(store.list().len(), 2);
    }
}
