// EasyNet CLI — daemon federation read model — HubPublishedAbilityStore
// ====================================================================
//
// File: src/daemon/federation/read_model/hub_published_abilities.rs
//
// Why this exists
// ---------------
// AXON-RFC-001 v4.1.7 hub-broadcast contract. Each realm hub keeps
// an authoritative list of abilities IT implements and pushes that list down
// to every member device through `federation.join` (full snapshot) +
// `federation.heartbeat` (incremental diff). The device-side catalogue
// (`meta.list_abilities scope=realm`, `<agent>.discover` scope=realm)
// merges the device-local registry with this store so users see everything
// the realm offers without an extra round-trip per query.
//
// This store is the schema gate between federation wire JSON and the canonical
// runtime read-model. Hub broadcasts enter as `HubAbilityEntry { name,
// descriptor: Value }`, but only parsed, validated `AbilityDescriptor` values
// may be cached or returned. That keeps `meta.list_abilities` from exposing
// names without `ability_ura` / `descriptor_ref` and prevents products from
// discovering routes that cannot be resolved canonically.
//
// What this store does NOT do:
//   * It is not a registration target. The device never invokes
//     hub-owned abilities through this store; calls to
//     `hub.openai.*` go through the canonical `Invocation::Invoke` RPC to
//     the hub. The store is read-mostly metadata.
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
use crate::daemon::federation::client::ability_contract::{HubAbilitiesDiff, HubAbilityEntry};

/// In-memory cache of hub-published ability descriptors, scoped
/// to one realm session. The store lives behind an `Arc` so the
/// federation client + the meta-ability synth path can share it
/// without ownership gymnastics.
#[derive(Debug, Default)]
pub struct HubPublishedAbilityStore {
    inner: RwLock<Inner>,
}

#[derive(Debug, Default)]
struct Inner {
    /// Canonical name → descriptor. BTreeMap so iteration order is
    /// stable across heartbeats — the meta-ability rendering is
    /// easier to audit when entries don't shuffle on each tick.
    entries: BTreeMap<String, AbilityDescriptor>,
    /// Last hub revision the cache reflects. Surfaced to the
    /// federation client as `since_abilities_revision` on the
    /// next heartbeat.
    revision: u64,
}

impl HubPublishedAbilityStore {
    pub fn new() -> Arc<Self> {
        Arc::new(Self::default())
    }

    /// Replace the whole cache with a join-time snapshot. Called
    /// on `federation.join` (initial seed) and on rejoin after a
    /// hub session drop — the device must drop any stale entries
    /// since the hub's revision space resets only with explicit
    /// removes, not session restarts.
    pub fn seed_from_snapshot(
        &self,
        revision: u64,
        abilities: Vec<HubAbilityEntry>,
    ) -> Result<(), String> {
        let entries = validate_hub_ability_entries(abilities)?;
        let mut inner = self.inner.write().unwrap_or_else(|e| e.into_inner());
        inner.entries.clear();
        for descriptor in entries {
            inner
                .entries
                .insert(descriptor.public_name().to_string(), descriptor);
        }
        inner.revision = revision;
        Ok(())
    }

    /// Apply a heartbeat diff. `added` upserts; `removed` deletes;
    /// `revision` advances the cache's tracked rev so the next
    /// heartbeat asks for "what's changed since the new value".
    /// Idempotent: replaying the same diff is a no-op.
    pub fn apply_diff(&self, diff: HubAbilitiesDiff) -> Result<(), String> {
        let added = validate_hub_ability_entries(diff.added)?;
        let mut inner = self.inner.write().unwrap_or_else(|e| e.into_inner());
        for descriptor in added {
            inner
                .entries
                .insert(descriptor.public_name().to_string(), descriptor);
        }
        for name in diff.removed {
            inner.entries.remove(&name);
        }
        if diff.revision >= inner.revision {
            inner.revision = diff.revision;
        }
        Ok(())
    }

    /// Last-seen hub revision. Federation client passes this
    /// verbatim as `since_abilities_revision` on the next
    /// heartbeat.
    pub fn revision(&self) -> u64 {
        self.inner
            .read()
            .unwrap_or_else(|e| e.into_inner())
            .revision
    }

    /// Snapshot of every hub-owned ability descriptor currently
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

fn validate_hub_ability_entries(
    entries: Vec<HubAbilityEntry>,
) -> Result<Vec<AbilityDescriptor>, String> {
    entries
        .into_iter()
        .enumerate()
        .map(|(index, entry)| validate_hub_ability_entry(index, entry))
        .collect()
}

fn validate_hub_ability_entry(
    index: usize,
    entry: HubAbilityEntry,
) -> Result<AbilityDescriptor, String> {
    let descriptor: AbilityDescriptor =
        serde_json::from_value(entry.descriptor).map_err(|error| {
            format!(
                "hub-published ability row {index} named {:?} is not a canonical descriptor: {error}",
                entry.name
            )
        })?;
    let public_name = descriptor.public_name();
    if entry.name != public_name {
        return Err(format!(
            "hub-published ability row {index} outer name {:?} does not match canonical descriptor name {:?}",
            entry.name, public_name
        ));
    }
    if descriptor.descriptor_ref().is_none() {
        return Err(format!(
            "hub-published ability row {index} named {:?} has no canonical descriptor_ref",
            entry.name
        ));
    }
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
            &crate::core::ura::hub_ura("test"),
            Visibility::Public,
            AdmissionAction::Read,
        )
        .expect("canonical hub descriptor")
    }

    fn entry(name: &str) -> HubAbilityEntry {
        let descriptor = descriptor(name);
        HubAbilityEntry {
            name: name.to_string(),
            descriptor: serde_json::to_value(descriptor).expect("descriptor wire json"),
        }
    }

    #[test]
    fn empty_store_starts_at_revision_zero() {
        let store = HubPublishedAbilityStore::new();
        assert_eq!(store.revision(), 0);
        assert!(store.is_empty());
    }

    #[test]
    fn seed_replaces_cache_and_records_revision() {
        let store = HubPublishedAbilityStore::new();
        store
            .seed_from_snapshot(7, vec![entry("test.a"), entry("test.b")])
            .expect("canonical snapshot");
        assert_eq!(store.revision(), 7);
        assert_eq!(store.len(), 2);
    }

    #[test]
    fn apply_diff_adds_new_entries() {
        let store = HubPublishedAbilityStore::new();
        store
            .apply_diff(HubAbilitiesDiff {
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
        let store = HubPublishedAbilityStore::new();
        store
            .seed_from_snapshot(1, vec![entry("test.a"), entry("test.b")])
            .expect("canonical snapshot");
        store
            .apply_diff(HubAbilitiesDiff {
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
        let store = HubPublishedAbilityStore::new();
        let diff = HubAbilitiesDiff {
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
    fn apply_diff_does_not_rewind_revision() {
        // A late heartbeat reply with a stale revision must not
        // rewind the cache's tracked revision — the federation
        // client uses this value to ask for "what's new since
        // here", and rewinding would re-fetch already-applied
        // diffs.
        let store = HubPublishedAbilityStore::new();
        store
            .seed_from_snapshot(10, vec![])
            .expect("canonical snapshot");
        store
            .apply_diff(HubAbilitiesDiff {
                revision: 5,
                added: vec![entry("test.late")],
                removed: vec![],
            })
            .expect("canonical diff");
        assert_eq!(store.revision(), 10);
        // The added entry still applies — late doesn't mean wrong,
        // just that the rev counter shouldn't move backwards.
        assert_eq!(store.len(), 1);
    }

    #[test]
    fn seed_rejects_noncanonical_descriptor_rows() {
        let store = HubPublishedAbilityStore::new();
        let error = store
            .seed_from_snapshot(
                7,
                vec![HubAbilityEntry {
                    name: "hub.bad".to_string(),
                    descriptor: json!({"name": "hub.bad"}),
                }],
            )
            .expect_err("opaque hub rows must not enter the runtime catalog");

        assert!(
            error.contains("is not a canonical descriptor"),
            "unexpected validation error: {error}"
        );
        assert_eq!(store.revision(), 0);
        assert!(store.is_empty());
    }

    #[test]
    fn apply_diff_is_atomic_when_added_row_is_noncanonical() {
        let store = HubPublishedAbilityStore::new();
        store
            .seed_from_snapshot(10, vec![entry("test.good")])
            .expect("canonical seed");

        let error = store
            .apply_diff(HubAbilitiesDiff {
                revision: 11,
                added: vec![HubAbilityEntry {
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
