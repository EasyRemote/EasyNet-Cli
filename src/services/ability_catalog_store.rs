// EasyNet CLI — Services Layer — AbilityCatalogStore
// =====================================================================
//
// File: src/services/ability_catalog_store.rs
//
// Why this exists
// ---------------
// `federation.advertise_abilities` lands on the hub-mode daemon when
// a device publishes its ability catalog (one call per agent at
// daemon boot, see EasyNet-Cli/src/runtime/publish.rs). Before this
// store the wrapper was a no-op stub that ack'd the call but
// dropped the descriptors on the floor — so when the backend later
// asked `federation.resolve(IncludeAbilities=true)` to drive the
// `/api/v1/abilities` catalog, the response carried URIs but no
// abilities and the Frontend showed an empty catalog despite every
// device having advertised one.
//
// Wire contract
// -------------
// `AdvertiseAbilitiesRequest { agent_ura, abilities: Vec<Value> }`
// arrives at `dispatch_federation_advertise_abilities`. This store
// upserts `(agent_ura, abilities)` into a DashMap; the resolve
// dispatch reads it back and merges into `ResolveAgentSummary`'s
// optional `abilities` field when the caller set
// `include_abilities = true`.
//
// Lifecycle
// ---------
// Re-advertise overwrites prior abilities by the same agent URI —
// idempotent, and the daemon-side advertise loop in
// `runtime/publish.rs` re-emits on every boot anyway. Eviction
// happens implicitly when an agent's `<self>.session` drops: the
// PresenceRegistry removes the URI; the catalog entry stays
// orphaned but the resolve filter prefix never visits it again
// (the resolve filter walks `presence.snapshot()` not the catalog
// store), so "list abilities for online devices" surfaces only
// live entries. A future GC step can sweep abandoned entries on
// `presence.remove_if_session` callbacks; v1 keeps the surface
// small.
//
// Trust boundary
// --------------
// The advertise call IS admitted by the same `<self>.session`
// trust gate as every other federation.* ability — the wrapper
// runs after admission, so a malicious caller cannot stuff the
// catalog with someone else's URI without first owning that URI's
// signing key.
//
// Author: Silan.Hu <silan.hu@u.nus.edu>
// Copyright (c) 2026 EasyNet. All rights reserved.

use std::sync::Arc;

use dashmap::DashMap;
use serde_json::Value;

/// Stores the most recent `abilities[]` advertised by each
/// agent URI. Cheap clone (`Arc` wrapper); shared between the
/// advertise dispatch handler and the resolve dispatch handler
/// inside `DaemonInvocationService`.
#[derive(Debug, Clone, Default)]
pub struct AbilityCatalogStore {
    inner: Arc<DashMap<String, Vec<Value>>>,
}

impl AbilityCatalogStore {
    pub fn new() -> Self {
        Self::default()
    }

    /// Upsert the abilities[] descriptor list for an agent. Re-call
    /// overwrites — last writer wins.
    pub fn upsert(&self, agent_ura: String, abilities: Vec<Value>) {
        if abilities.is_empty() {
            // Empty list is a legitimate advertise (an agent with
            // zero abilities). Store the empty Vec so resolve
            // returns `[]` instead of falling back to "no entry"
            // semantics — the difference matters for the
            // Frontend's empty-state vs unknown-state UI.
            self.inner.insert(agent_ura, Vec::new());
            return;
        }
        self.inner.insert(agent_ura, abilities);
    }

    /// Return the abilities[] for an agent, or `None` when no
    /// advertise has landed yet for that URI.
    pub fn get(&self, agent_ura: &str) -> Option<Vec<Value>> {
        self.inner.get(agent_ura).map(|entry| entry.clone())
    }

    /// Total number of agent URIs with stored abilities. Used by
    /// `daemon outstanding`-style smoke tests.
    pub fn len(&self) -> usize {
        self.inner.len()
    }

    pub fn is_empty(&self) -> bool {
        self.inner.is_empty()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn empty_store_returns_none() {
        let store = AbilityCatalogStore::new();
        assert!(store.is_empty());
        assert!(store.get("easynet:///r/easynet.run/device/abc").is_none());
    }

    #[test]
    fn upsert_then_get_round_trips() {
        let store = AbilityCatalogStore::new();
        let abilities = vec![json!({"name": "fs.read"}), json!({"name": "ping"})];
        store.upsert(
            "easynet:///r/easynet.run/device/abc".into(),
            abilities.clone(),
        );
        let got = store
            .get("easynet:///r/easynet.run/device/abc")
            .expect("get matches upsert");
        assert_eq!(got, abilities);
        assert_eq!(store.len(), 1);
    }

    #[test]
    fn upsert_overwrites_prior_entry() {
        let store = AbilityCatalogStore::new();
        store.upsert("uri".into(), vec![json!({"name": "v1"})]);
        store.upsert("uri".into(), vec![json!({"name": "v2"})]);
        let got = store.get("uri").unwrap();
        assert_eq!(got.len(), 1);
        assert_eq!(got[0]["name"], "v2");
    }

    #[test]
    fn upsert_empty_list_stores_empty_not_none() {
        let store = AbilityCatalogStore::new();
        store.upsert("uri".into(), Vec::new());
        let got = store
            .get("uri")
            .expect("empty advertise still surfaces a row");
        assert!(got.is_empty());
    }
}
