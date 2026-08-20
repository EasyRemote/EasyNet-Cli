// EasyNet CLI — daemon federation read model — AuthorityPublishedAbilityStore
// =========================================================================
//
// File: src/daemon/federation/read_model/authority_published_abilities.rs
//
// Why this exists
// ---------------
// AXON-RFC-001 v4.1.7 Authority broadcast contract. Each realm Authority
// publishes an authoritative list of abilities and pushes that list down to
// every member device through `federation.join` (full snapshot) and
// `federation.heartbeat` (incremental diff). The device-side catalogue
// (`meta.list_abilities scope=realm`, `<agent>.discover` scope=realm) merges
// the device-local registry with this store so users see everything the realm
// offers without an extra round-trip per query.
//
// This store is the schema gate between federation wire JSON and the canonical
// runtime read model. Federation broadcasts enter as `AuthorityAbilityEntry { name,
// descriptor: Value }` for wire compatibility, but only parsed, validated
// `AbilityDescriptor` values may be cached or returned. That keeps
// `meta.list_abilities` from exposing names without `ability_ura` /
// `descriptor_ref` and prevents products from discovering routes that cannot
// be resolved canonically.
//
// What this store does NOT do:
//   * It is not a registration target. The device never invokes
//     Authority-published abilities through this store; calls go through the
//     canonical `Invocation::Invoke` RPC to the publishing runtime. The store
//     is read-mostly metadata.
//   * It is not persistent. Restart drops the cache; the next
//     `federation.join` reseeds it. Simpler than disk
//     synchronisation, and join is cheap enough to amortise.
//
// Concurrency: a single `RwLock<Inner>` because mutations come
// only from the federation client (one task per realm) and reads
// from `meta.list_abilities` are infrequent. Per-key sharding
// would be premature optimisation here.
//
// Author: Silan.Hu <silan.hu@u.nus.edu>
// Copyright (c) 2026 EasyNet. All rights reserved.

use std::collections::BTreeMap;
use std::sync::{Arc, RwLock};

use crate::daemon::ability::descriptors::AbilityDescriptor;
use crate::daemon::ability::{insert_catalog_descriptor, CatalogDescriptorKey};
use crate::daemon::federation::client::ability_contract::{
    AuthorityAbilitiesDiff, AuthorityAbilityEntry,
};

/// In-memory cache of Authority-published ability descriptors, scoped
/// to one realm session. The store lives behind an `Arc` so the
/// federation client + the meta-ability synth path can share it
/// without ownership gymnastics.
#[derive(Debug, Default)]
pub struct AuthorityPublishedAbilityStore {
    inner: RwLock<Inner>,
}

#[derive(Debug, Default)]
struct Inner {
    /// Full canonical descriptor identity → descriptor. Public name alone is
    /// insufficient because one ability may publish RPC/Stream/Bidi variants
    /// and version is a receipt-bound invocation fact.
    entries: BTreeMap<CatalogDescriptorKey, AbilityDescriptor>,
    /// Last Authority broadcast revision the cache reflects. Surfaced to the
    /// federation client as `since_abilities_revision` on the
    /// next heartbeat.
    revision: u64,
}

#[derive(Debug, Clone)]
pub(crate) struct AuthorityPublishedAbilitySnapshot {
    pub(crate) revision: u64,
    pub(crate) descriptors: Vec<AbilityDescriptor>,
}

impl AuthorityPublishedAbilityStore {
    pub fn new() -> Arc<Self> {
        Arc::new(Self::default())
    }

    /// Replace the whole cache with a join-time snapshot. Called
    /// on `federation.join` (initial seed) and on rejoin after a
    /// realm session drop — the device must drop any stale entries
    /// since the Authority's revision space resets only with explicit
    /// removes, not session restarts.
    pub fn seed_from_snapshot(
        &self,
        revision: u64,
        abilities: Vec<AuthorityAbilityEntry>,
    ) -> Result<(), String> {
        let descriptors = validate_authority_ability_entries(abilities)?;
        let mut entries = BTreeMap::new();
        for descriptor in descriptors {
            insert_catalog_descriptor(&mut entries, descriptor, "Authority snapshot")?;
        }
        let mut inner = self.inner.write().unwrap_or_else(|e| e.into_inner());
        inner.entries = entries;
        inner.revision = revision;
        Ok(())
    }

    /// Apply a heartbeat diff. `added` upserts; `removed` deletes;
    /// `revision` advances the cache's tracked rev so the next
    /// heartbeat asks for "what's changed since the new value".
    /// Idempotent: replaying the same diff is a no-op.
    pub fn apply_diff(&self, diff: AuthorityAbilitiesDiff) -> Result<(), String> {
        let mut inner = self.inner.write().unwrap_or_else(|e| e.into_inner());
        if diff.revision <= inner.revision {
            return Ok(());
        }
        let added = validate_authority_ability_entries(diff.added)?;
        let mut next_entries = inner.entries.clone();
        for name in diff.removed {
            next_entries.retain(|_, descriptor| descriptor.public_name() != name);
        }
        for descriptor in added {
            insert_catalog_descriptor(&mut next_entries, descriptor, "Authority heartbeat diff")?;
        }
        inner.entries = next_entries;
        inner.revision = diff.revision;
        Ok(())
    }

    /// Last-seen Authority broadcast revision. Federation client passes this
    /// verbatim as `since_abilities_revision` on the next
    /// heartbeat.
    pub fn revision(&self) -> u64 {
        self.inner
            .read()
            .unwrap_or_else(|e| e.into_inner())
            .revision
    }

    /// Snapshot of every Authority-published ability descriptor currently
    /// cached. `meta.list_abilities scope=realm` consumes this and
    /// merges with the device-local published set.
    #[must_use]
    pub fn snapshot(&self) -> Vec<AbilityDescriptor> {
        self.inner
            .read()
            .unwrap_or_else(|e| e.into_inner())
            .entries
            .values()
            .cloned()
            .collect()
    }

    /// Atomically capture the revision and all rows it names. Callers that
    /// expose a catalog snapshot must not pair two independent lock reads,
    /// because a heartbeat could advance between them.
    #[must_use]
    pub(crate) fn snapshot_with_revision(&self) -> AuthorityPublishedAbilitySnapshot {
        let inner = self.inner.read().unwrap_or_else(|e| e.into_inner());
        AuthorityPublishedAbilitySnapshot {
            revision: inner.revision,
            descriptors: inner.entries.values().cloned().collect(),
        }
    }

    /// Cardinality — used by tests and diagnostics. Matches
    /// `snapshot().len()` without the clone.
    #[must_use]
    pub fn len(&self) -> usize {
        self.inner
            .read()
            .unwrap_or_else(|e| e.into_inner())
            .entries
            .len()
    }

    #[must_use]
    pub fn is_empty(&self) -> bool {
        self.len() == 0
    }
}

fn validate_authority_ability_entries(
    entries: Vec<AuthorityAbilityEntry>,
) -> Result<Vec<AbilityDescriptor>, String> {
    entries
        .into_iter()
        .enumerate()
        .map(|(index, entry)| validate_authority_ability_entry(index, entry))
        .collect()
}

fn validate_authority_ability_entry(
    index: usize,
    entry: AuthorityAbilityEntry,
) -> Result<AbilityDescriptor, String> {
    let descriptor: AbilityDescriptor =
        serde_json::from_value(entry.descriptor).map_err(|error| {
            format!(
                "Authority-published ability row {index} named {:?} is not a canonical descriptor: {error}",
                entry.name
            )
        })?;
    let public_name = descriptor.public_name();
    if entry.name != public_name {
        return Err(format!(
            "Authority-published ability row {index} outer name {:?} does not match canonical descriptor name {:?}",
            entry.name, public_name
        ));
    }
    descriptor.descriptor_ref().map_err(|error| {
        format!(
            "Authority-published ability row {index} named {:?} has invalid canonical descriptor_ref: {error}",
            entry.name
        )
    })?;
    Ok(descriptor)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::daemon::ability::descriptors::{AdmissionAction, Visibility};
    use serde_json::json;

    fn descriptor(name: &str) -> AbilityDescriptor {
        AbilityDescriptor::new(
            name,
            crate::core::ura::hub_ura("test"),
            Visibility::Public,
            AdmissionAction::Read,
        )
        .expect("canonical realm Authority descriptor")
    }

    fn entry(name: &str) -> AuthorityAbilityEntry {
        let descriptor = descriptor(name);
        AuthorityAbilityEntry {
            name: name.to_string(),
            descriptor: serde_json::to_value(descriptor).expect("descriptor wire json"),
        }
    }

    #[test]
    fn empty_store_starts_at_revision_zero() {
        let store = AuthorityPublishedAbilityStore::new();
        assert_eq!(store.revision(), 0);
        assert!(store.is_empty());
    }

    #[test]
    fn seed_replaces_cache_and_records_revision() {
        let store = AuthorityPublishedAbilityStore::new();
        store
            .seed_from_snapshot(7, vec![entry("test.a"), entry("test.b")])
            .expect("canonical snapshot");
        assert_eq!(store.revision(), 7);
        assert_eq!(store.len(), 2);
    }

    #[test]
    fn apply_diff_adds_new_entries() {
        let store = AuthorityPublishedAbilityStore::new();
        store
            .apply_diff(AuthorityAbilitiesDiff {
                revision: 1,
                added: vec![entry("test.a")],
                removed: vec![],
            })
            .expect("canonical diff");
        assert_eq!(store.revision(), 1);
        assert_eq!(store.snapshot()[0].public_name(), "test.a");
    }

    #[test]
    fn apply_diff_removes_entries() {
        let store = AuthorityPublishedAbilityStore::new();
        store
            .seed_from_snapshot(1, vec![entry("test.a"), entry("test.b")])
            .expect("canonical snapshot");
        store
            .apply_diff(AuthorityAbilitiesDiff {
                revision: 2,
                added: vec![],
                removed: vec!["test.a".to_string()],
            })
            .expect("canonical diff");
        assert_eq!(store.len(), 1);
        assert_eq!(store.snapshot()[0].public_name(), "test.b");
        assert_eq!(store.revision(), 2);
    }

    #[test]
    fn apply_diff_idempotent_when_replayed() {
        let store = AuthorityPublishedAbilityStore::new();
        let diff = AuthorityAbilitiesDiff {
            revision: 3,
            added: vec![entry("test.a")],
            removed: vec![],
        };
        store.apply_diff(diff.clone()).expect("canonical diff");
        store.apply_diff(diff).expect("canonical replay");
        assert_eq!(store.len(), 1);
        assert_eq!(store.revision(), 3);
    }

    #[test]
    fn apply_diff_ignores_complete_stale_transition() {
        let store = AuthorityPublishedAbilityStore::new();
        store
            .seed_from_snapshot(10, vec![])
            .expect("canonical snapshot");
        store
            .apply_diff(AuthorityAbilitiesDiff {
                revision: 5,
                added: vec![entry("test.late")],
                removed: vec![],
            })
            .expect("canonical diff");
        assert_eq!(store.revision(), 10);
        assert!(store.is_empty());
    }

    #[test]
    fn snapshot_preserves_same_ability_rpc_and_stream_variants() {
        let store = AuthorityPublishedAbilityStore::new();
        let owner = crate::core::ura::hub_ura("test");
        let rpc = AbilityDescriptor::new(
            "test.chat",
            &owner,
            Visibility::Public,
            AdmissionAction::Invoke,
        )
        .unwrap()
        .with_call_mode(crate::daemon::ability::CallMode::Rpc);
        let stream = rpc
            .clone()
            .with_call_mode(crate::daemon::ability::CallMode::Stream);
        store
            .seed_from_snapshot(
                4,
                vec![
                    AuthorityAbilityEntry {
                        name: "test.chat".to_string(),
                        descriptor: serde_json::to_value(rpc).unwrap(),
                    },
                    AuthorityAbilityEntry {
                        name: "test.chat".to_string(),
                        descriptor: serde_json::to_value(stream).unwrap(),
                    },
                ],
            )
            .expect("multi-mode snapshot");

        let snapshot = store.snapshot_with_revision();
        assert_eq!(snapshot.revision, 4);
        assert_eq!(snapshot.descriptors.len(), 2);
    }

    #[test]
    fn seed_rejects_noncanonical_descriptor_rows() {
        let store = AuthorityPublishedAbilityStore::new();
        let error = store
            .seed_from_snapshot(
                7,
                vec![AuthorityAbilityEntry {
                    name: "hub.bad".to_string(),
                    descriptor: json!({"name": "hub.bad"}),
                }],
            )
            .expect_err("opaque broadcast rows must not enter the runtime catalog");

        assert!(
            error.contains("is not a canonical descriptor"),
            "unexpected validation error: {error}"
        );
        assert_eq!(store.revision(), 0);
        assert!(store.is_empty());
    }

    #[test]
    fn apply_diff_is_atomic_when_added_row_is_noncanonical() {
        let store = AuthorityPublishedAbilityStore::new();
        store
            .seed_from_snapshot(10, vec![entry("test.good")])
            .expect("canonical seed");

        let error = store
            .apply_diff(AuthorityAbilitiesDiff {
                revision: 11,
                added: vec![AuthorityAbilityEntry {
                    name: "hub.bad".to_string(),
                    descriptor: json!({"name": "hub.bad"}),
                }],
                removed: vec!["test.good".to_string()],
            })
            .expect_err("bad added row must reject the whole diff");

        assert!(
            error.contains("is not a canonical descriptor"),
            "unexpected validation error: {error}"
        );
        assert_eq!(store.revision(), 10);
        assert_eq!(store.len(), 1);
        assert_eq!(store.snapshot()[0].public_name(), "test.good");
    }
}
