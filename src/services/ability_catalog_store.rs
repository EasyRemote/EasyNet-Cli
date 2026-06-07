// EasyNet CLI — Services Layer — AbilityCatalogStore
// ==================================================
//
// File: src/services/ability_catalog_store.rs
//
// Why this exists
// ---------------
// `federation.advertise_abilities` lands on the hub-mode daemon when
// a device publishes an RFC-005 owner projection. This store keeps the
// latest accepted projection row per owner so `federation.resolve`
// can expose bounded ability summaries to backend read models without
// treating raw implementation descriptors as hub authority.
//
// Wire contract
// -------------
// `AdvertiseAbilitiesRequest` carries `owner_ura`, `host_device_ura`,
// projection metadata, and `ability_summaries`. This store upserts one
// typed row keyed by owner URA. The resolve dispatch reads the row back
// and serializes only `ability_summaries` into
// `ResolveAgentSummary.abilities` when the caller sets
// `include_abilities = true`.
//
// Lifecycle
// ---------
// Re-advertise replaces the prior projection for the same owner URA.
// Empty projections are stored explicitly so callers can distinguish
// "known owner with zero visible abilities" from "no projection has
// arrived". Eviction remains tied to directory liveness: resolve only
// surfaces rows for online owners or hosted agents whose host is online.
//
// Trust boundary
// --------------
// The advertise call is admitted before this store runs. This store is
// still not authority: it does not validate signatures, does not host
// route internals, and does not store local resource references. It is
// a hub read model for namespace-safe summaries only.
//
// Author: Silan.Hu <silan.hu@u.nus.edu>
// Copyright (c) 2026 EasyNet. All rights reserved.

use std::sync::Arc;

use dashmap::DashMap;
use serde_json::Value;

use crate::runtime::owner_projection::AbilityProjectionSummary;

/// Latest accepted owner ability projection retained by the hub read model.
///
/// This is not the Axon namespace authority and it is not an implementation
/// descriptor cache. It preserves publication metadata for diagnostics and
/// invalidation, while exposing only bounded `AbilityProjectionSummary` values
/// to `federation.resolve(include_abilities=true)`.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct OwnerAbilityProjectionRow {
    owner_ura: String,
    host_device_ura: String,
    projection_revision: u64,
    projection_digest: String,
    lease_expires_unix_ms: i64,
    ability_summaries: Vec<AbilityProjectionSummary>,
}

impl OwnerAbilityProjectionRow {
    #[must_use]
    pub(crate) fn new(
        owner_ura: String,
        host_device_ura: String,
        projection_revision: u64,
        projection_digest: String,
        lease_expires_unix_ms: i64,
        ability_summaries: Vec<AbilityProjectionSummary>,
    ) -> Self {
        Self {
            owner_ura,
            host_device_ura,
            projection_revision,
            projection_digest,
            lease_expires_unix_ms,
            ability_summaries,
        }
    }

    #[must_use]
    pub(crate) fn summaries_as_json(&self) -> Vec<Value> {
        self.ability_summaries
            .iter()
            .map(|summary| {
                serde_json::to_value(summary)
                    .expect("AbilityProjectionSummary must serialize to JSON")
            })
            .collect()
    }

    #[cfg(test)]
    pub(crate) fn owner_ura(&self) -> &str {
        &self.owner_ura
    }

    #[cfg(test)]
    pub(crate) fn host_device_ura(&self) -> &str {
        &self.host_device_ura
    }

    #[cfg(test)]
    pub(crate) fn projection_revision(&self) -> u64 {
        self.projection_revision
    }

    #[cfg(test)]
    pub(crate) fn projection_digest(&self) -> &str {
        &self.projection_digest
    }

    #[cfg(test)]
    pub(crate) fn lease_expires_unix_ms(&self) -> i64 {
        self.lease_expires_unix_ms
    }

    #[cfg(test)]
    pub(crate) fn ability_count(&self) -> usize {
        self.ability_summaries.len()
    }
}

/// Stores the most recent owner projection row by owner URA. Cheap clone
/// (`Arc` wrapper); shared between the advertise dispatch handler and the
/// resolve dispatch handler inside `DaemonInvocationService`.
#[derive(Debug, Clone, Default)]
pub struct AbilityCatalogStore {
    inner: Arc<DashMap<String, OwnerAbilityProjectionRow>>,
}

impl AbilityCatalogStore {
    #[must_use]
    pub fn new() -> Self {
        Self::default()
    }

    /// Upsert the owner projection row. Re-call overwrites by owner URA.
    pub(crate) fn upsert_projection(&self, row: OwnerAbilityProjectionRow) {
        self.inner.insert(row.owner_ura.clone(), row);
    }

    /// Return namespace-safe ability summaries for an owner, or `None`
    /// when no projection has landed yet for that owner URA.
    pub fn get(&self, owner_ura: &str) -> Option<Vec<Value>> {
        self.inner
            .get(owner_ura)
            .map(|entry| entry.summaries_as_json())
    }

    /// Return the full stored row for tests.
    #[cfg(test)]
    pub(crate) fn projection_for_owner(
        &self,
        owner_ura: &str,
    ) -> Option<OwnerAbilityProjectionRow> {
        self.inner.get(owner_ura).map(|entry| entry.clone())
    }

    /// Total number of owner URAs with stored projections. Used by
    /// `daemon outstanding`-style smoke tests.
    #[must_use]
    pub fn len(&self) -> usize {
        self.inner.len()
    }

    #[must_use]
    pub fn is_empty(&self) -> bool {
        self.inner.is_empty()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn empty_store_returns_none() {
        let store = AbilityCatalogStore::new();
        assert!(store.is_empty());
        assert!(store.get("easynet:///r/easynet.run/device/abc").is_none());
    }

    #[test]
    fn upsert_then_get_round_trips_projection_summary() {
        let store = AbilityCatalogStore::new();
        let row = projection_row("easynet:///r/easynet.run/device/abc", vec![summary("read")]);

        store.upsert_projection(row.clone());

        let got = store
            .get("easynet:///r/easynet.run/device/abc")
            .expect("get matches upsert");
        assert_eq!(got.len(), 1);
        assert_eq!(got[0]["local_name"], "read");
        assert_eq!(got[0]["namespace"], "fs");
        assert_eq!(
            store
                .projection_for_owner("easynet:///r/easynet.run/device/abc")
                .unwrap(),
            row
        );
        assert_eq!(store.len(), 1);
    }

    #[test]
    fn upsert_overwrites_prior_projection_by_owner() {
        let store = AbilityCatalogStore::new();
        store.upsert_projection(projection_row("ura", vec![summary("read")]));
        store.upsert_projection(projection_row("ura", vec![summary("write")]));

        let got = store.get("ura").unwrap();
        assert_eq!(got.len(), 1);
        assert_eq!(got[0]["local_name"], "write");
    }

    #[test]
    fn upsert_empty_projection_stores_empty_not_none() {
        let store = AbilityCatalogStore::new();

        store.upsert_projection(projection_row("ura", Vec::new()));

        let got = store
            .get("ura")
            .expect("empty advertise still surfaces a row");
        assert!(got.is_empty());
    }

    fn projection_row(
        owner_ura: &str,
        ability_summaries: Vec<AbilityProjectionSummary>,
    ) -> OwnerAbilityProjectionRow {
        OwnerAbilityProjectionRow::new(
            owner_ura.to_string(),
            crate::ura::device_ura("easynet.run", "abc"),
            1,
            "sha256:digest".to_string(),
            1_714_492_800_000,
            ability_summaries,
        )
    }

    fn summary(local_name: &str) -> AbilityProjectionSummary {
        let ability_id = format!("fs.{local_name}");
        let owner_ura = crate::ura::device_ura("easynet.run", "abc");
        AbilityProjectionSummary {
            ability_ura: crate::ura::device_ability_ura("easynet.run", "abc", &ability_id),
            owner_ura,
            namespace: "fs".to_string(),
            local_name: local_name.to_string(),
            descriptor_revision: "sha256:descriptor".to_string(),
            schema_ref: None,
            schema_hash: None,
            policy_ref: "visibility:PUBLIC".to_string(),
            route_summary_ref: None,
            tags: vec!["class:unary".to_string()],
        }
    }
}
