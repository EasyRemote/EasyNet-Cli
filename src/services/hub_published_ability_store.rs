// EasyNet CLI — Services Layer — HubPublishedAbilityStore
// ========================================================
//
// File: src/services/hub_published_ability_store.rs
//
// Why this exists
// ---------------
// AXON-RFC-001 v4.1.7 hub-broadcast contract. Each realm hub keeps
// an authoritative list of abilities IT implements (e.g.
// `hub.openai.chat_completions` if a given deployment ships the
// OpenAI compat shim) and pushes that list down to every member
// device through `federation.join` (full snapshot) +
// `federation.heartbeat` (incremental diff). The device-side
// catalogue (`meta.list_abilities scope=realm`, `<self>.discover`
// scope=realm) merges the device-local registry with this store
// so users see everything the realm offers without an extra
// round-trip per query.
//
// What this store does NOT do:
//   * It is not a registration target. The device never invokes
//     hub-owned abilities through this store; calls to
//     `hub.openai.*` go through `federation.forward_invoke` to
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

use crate::runtime::federation_client::{HubAbilitiesDiff, HubAbilityEntry};

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
    entries: BTreeMap<String, HubAbilityEntry>,
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
    pub fn seed_from_snapshot(&self, revision: u64, abilities: Vec<HubAbilityEntry>) {
        let mut inner = self.inner.write().unwrap_or_else(|e| e.into_inner());
        inner.entries.clear();
        for entry in abilities {
            inner.entries.insert(entry.name.clone(), entry);
        }
        inner.revision = revision;
    }

    /// Apply a heartbeat diff. `added` upserts; `removed` deletes;
    /// `revision` advances the cache's tracked rev so the next
    /// heartbeat asks for "what's changed since the new value".
    /// Idempotent: replaying the same diff is a no-op.
    pub fn apply_diff(&self, diff: HubAbilitiesDiff) {
        let mut inner = self.inner.write().unwrap_or_else(|e| e.into_inner());
        for entry in diff.added {
            inner.entries.insert(entry.name.clone(), entry);
        }
        for name in diff.removed {
            inner.entries.remove(&name);
        }
        if diff.revision >= inner.revision {
            inner.revision = diff.revision;
        }
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
    pub fn snapshot(&self) -> Vec<HubAbilityEntry> {
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

/// Process-wide singleton. The session-prelude functions in
/// `services/axon_serve/session_initiator.rs` need to reach the
/// store without threading a new `Arc` through five layers of
/// supervisor / dial / handshake plumbing; the meta-ability synth
/// path (read-only) does too. We expose a `OnceLock` so both
/// sides see the same store without owning the lifecycle dance.
///
/// **Long-term**: this should be passed via `DaemonInvocationService`
/// the same way `AdvertisedAgentStore` is, removing the singleton.
/// Tracked as a follow-up cleanup; v0 keeps the smaller footprint.
static INSTANCE: std::sync::OnceLock<Arc<HubPublishedAbilityStore>> = std::sync::OnceLock::new();

pub fn global() -> &'static Arc<HubPublishedAbilityStore> {
    INSTANCE.get_or_init(HubPublishedAbilityStore::new)
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    fn entry(name: &str) -> HubAbilityEntry {
        HubAbilityEntry {
            name: name.to_string(),
            descriptor: json!({"name": name}),
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
        store.seed_from_snapshot(7, vec![entry("hub.a"), entry("hub.b")]);
        assert_eq!(store.revision(), 7);
        assert_eq!(store.len(), 2);
    }

    #[test]
    fn apply_diff_adds_new_entries() {
        let store = HubPublishedAbilityStore::new();
        store.apply_diff(HubAbilitiesDiff {
            revision: 1,
            added: vec![entry("hub.a")],
            removed: vec![],
        });
        assert_eq!(store.revision(), 1);
        assert_eq!(store.snapshot()[0].name, "hub.a");
    }

    #[test]
    fn apply_diff_removes_entries() {
        let store = HubPublishedAbilityStore::new();
        store.seed_from_snapshot(1, vec![entry("hub.a"), entry("hub.b")]);
        store.apply_diff(HubAbilitiesDiff {
            revision: 2,
            added: vec![],
            removed: vec!["hub.a".to_string()],
        });
        assert_eq!(store.len(), 1);
        assert_eq!(store.snapshot()[0].name, "hub.b");
        assert_eq!(store.revision(), 2);
    }

    #[test]
    fn apply_diff_idempotent_when_replayed() {
        let store = HubPublishedAbilityStore::new();
        let diff = HubAbilitiesDiff {
            revision: 3,
            added: vec![entry("hub.a")],
            removed: vec![],
        };
        store.apply_diff(diff.clone());
        store.apply_diff(diff);
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
        store.seed_from_snapshot(10, vec![]);
        store.apply_diff(HubAbilitiesDiff {
            revision: 5,
            added: vec![entry("hub.late")],
            removed: vec![],
        });
        assert_eq!(store.revision(), 10);
        // The added entry still applies — late doesn't mean wrong,
        // just that the rev counter shouldn't move backwards.
        assert_eq!(store.len(), 1);
    }
}
