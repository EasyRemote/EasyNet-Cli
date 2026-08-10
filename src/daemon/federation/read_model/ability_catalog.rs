// EasyNet CLI — daemon federation read model — AbilityCatalogStore
// ================================================================
//
// File: src/daemon/federation/read_model/ability_catalog.rs
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

use std::collections::BTreeSet;
use std::sync::Arc;

use dashmap::mapref::entry::Entry;
use dashmap::DashMap;
use serde_json::Value;

use crate::daemon::federation::read_model::owner_projection::AbilityProjectionSummary;

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
    generation: u64,
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
        generation: u64,
        projection_revision: u64,
        projection_digest: String,
        lease_expires_unix_ms: i64,
        ability_summaries: Vec<AbilityProjectionSummary>,
    ) -> Self {
        Self {
            owner_ura,
            host_device_ura,
            generation,
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

    #[must_use]
    pub(crate) fn is_live_at(&self, now_unix_ms: i64) -> bool {
        // C4: lease cancelled (ISS-002) — a non-positive lease means
        // "never expires", mirroring Axon's `expire_leases` guard
        // (`lease > 0`). Owner projections now publish lease=0, so the
        // hub read model must keep serving them indefinitely.
        self.lease_expires_unix_ms <= 0 || self.lease_expires_unix_ms > now_unix_ms
    }

    pub(crate) fn owner_ura(&self) -> &str {
        &self.owner_ura
    }

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

/// Outcome of applying a projection publication to the hub read model.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum ProjectionUpsertOutcome {
    Inserted,
    Updated,
    Idempotent,
    IgnoredStale,
    RejectedConflict,
}

impl ProjectionUpsertOutcome {
    #[must_use]
    pub(crate) fn is_stored(self) -> bool {
        matches!(self, Self::Inserted | Self::Updated | Self::Idempotent)
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

    /// Upsert a projection that has already passed transport/publication
    /// admission. This keeps external integration harnesses on the same
    /// integrity and revision-fence path as `federation.advertise_abilities`
    /// without exposing the internal row type.
    pub fn upsert_admitted_projection_json(
        &self,
        publication: serde_json::Value,
    ) -> Result<bool, String> {
        let mut publication: crate::daemon::federation::read_model::owner_projection::OwnerProjectionPublication =
            serde_json::from_value(publication)
                .map_err(|error| format!("decode owner projection publication: {error}"))?;
        if publication.projection_digest.is_empty() {
            publication.projection_digest = publication.canonical_digest();
        }
        publication.validate_integrity()?;
        Ok(self
            .upsert_projection(OwnerAbilityProjectionRow::new(
                publication.owner_ura,
                publication.host_device_ura,
                publication.generation,
                publication.projection_revision,
                publication.projection_digest,
                publication.lease_expires_unix_ms,
                publication.ability_summaries,
            ))
            .is_stored())
    }

    /// Upsert the owner projection row behind a per-owner revision fence.
    ///
    /// This is read-model protection, not namespace authority: signature,
    /// caller, and host authorization checks happen before this store. The
    /// fence prevents stale or conflicting projections that have already
    /// reached the admitted handler from corrupting resolver summaries.
    pub(crate) fn upsert_projection(
        &self,
        row: OwnerAbilityProjectionRow,
    ) -> ProjectionUpsertOutcome {
        match self.inner.entry(row.owner_ura.clone()) {
            Entry::Vacant(entry) => {
                entry.insert(row);
                ProjectionUpsertOutcome::Inserted
            }
            Entry::Occupied(mut entry) => {
                let current = entry.get();
                if row.generation < current.generation {
                    return ProjectionUpsertOutcome::IgnoredStale;
                }
                if row.generation == current.generation
                    && row.projection_revision < current.projection_revision
                {
                    return ProjectionUpsertOutcome::IgnoredStale;
                }
                if row.generation > current.generation {
                    entry.insert(row);
                    return ProjectionUpsertOutcome::Updated;
                }
                if row.projection_revision == current.projection_revision {
                    if row.projection_digest == current.projection_digest {
                        return ProjectionUpsertOutcome::Idempotent;
                    }
                    if row.host_device_ura == current.host_device_ura
                        && row.ability_summaries == current.ability_summaries
                        && row.lease_expires_unix_ms > current.lease_expires_unix_ms
                    {
                        entry.insert(row);
                        return ProjectionUpsertOutcome::Updated;
                    }
                    return ProjectionUpsertOutcome::RejectedConflict;
                }
                entry.insert(row);
                ProjectionUpsertOutcome::Updated
            }
        }
    }

    /// Remove only the projection belonging to the revoked incarnation.
    pub(crate) fn remove_generation(&self, owner_ura: &str, generation: u64) -> bool {
        self.inner
            .remove_if(owner_ura, |_owner, row| row.generation == generation)
            .is_some()
    }

    /// Remove the current projection for an owner when the revoke command does
    /// not carry an incarnation fence.
    pub(crate) fn remove_owner(&self, owner_ura: &str) -> bool {
        self.inner.remove(owner_ura).is_some()
    }

    /// Return namespace-safe ability summaries for an owner, or `None`
    /// when no projection has landed yet for that owner URA.
    pub fn get(&self, owner_ura: &str) -> Option<Vec<Value>> {
        self.get_at(
            owner_ura,
            crate::daemon::federation::directory::now_unix_ms(),
        )
    }

    /// Return namespace-safe ability summaries if the owner's projection
    /// exists and its lease has not expired at `now_unix_ms`.
    pub(crate) fn get_at(&self, owner_ura: &str, now_unix_ms: i64) -> Option<Vec<Value>> {
        self.inner
            .get(owner_ura)
            .filter(|entry| entry.is_live_at(now_unix_ms))
            .map(|entry| entry.summaries_as_json())
    }

    /// Return live projection rows whose execution host Device is present.
    ///
    /// This is the resolver read-model join for device-sponsored SystemAgents:
    /// the SystemAgent is the ability owner/callee, while the Device remains
    /// the session-bearing host. Presence belongs to the host Device; the
    /// SystemAgent must not be inserted into `PresenceRegistry`; it is not a
    /// second online principal just to make its published abilities discoverable.
    pub(crate) fn projection_rows_for_live_hosts_at(
        &self,
        live_host_device_uras: &BTreeSet<String>,
        now_unix_ms: i64,
    ) -> Vec<OwnerAbilityProjectionRow> {
        let mut rows: Vec<_> = self
            .inner
            .iter()
            .filter(|entry| entry.is_live_at(now_unix_ms))
            .filter(|entry| live_host_device_uras.contains(entry.host_device_ura()))
            .map(|entry| entry.clone())
            .collect();
        rows.sort_by(|left, right| left.owner_ura().cmp(right.owner_ura()));
        rows
    }

    /// Extend an owner projection's lease to `new_expires_unix_ms` in
    /// response to a `federation.heartbeat` refresh.
    ///
    /// RFC-005: heartbeat refreshes the lease only — it MUST NOT mutate
    /// projection contents, revision, or digest. This therefore touches
    /// `lease_expires_unix_ms` and nothing else, and never shrinks an
    /// existing lease (a late/duplicate heartbeat cannot pull the
    /// expiry backwards). Returns `true` when a live-or-revivable row was
    /// extended, `false` when no projection exists for the owner (the
    /// device must re-publish via `advertise_abilities` first).
    pub(crate) fn refresh_lease(&self, owner_ura: &str, new_expires_unix_ms: i64) -> bool {
        match self.inner.get_mut(owner_ura) {
            Some(mut row) => {
                if row.lease_expires_unix_ms <= 0 {
                    return true;
                }
                if new_expires_unix_ms > row.lease_expires_unix_ms {
                    row.lease_expires_unix_ms = new_expires_unix_ms;
                }
                true
            }
            None => false,
        }
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

        let outcome = store.upsert_projection(row.clone());
        assert_eq!(outcome, ProjectionUpsertOutcome::Inserted);
        assert!(outcome.is_stored());

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
        let first = projection_row_with_revision("ura", 1, "sha256:first", vec![summary("read")]);
        let second =
            projection_row_with_revision("ura", 2, "sha256:second", vec![summary("write")]);

        assert_eq!(
            store.upsert_projection(first),
            ProjectionUpsertOutcome::Inserted
        );
        assert_eq!(
            store.upsert_projection(second),
            ProjectionUpsertOutcome::Updated
        );

        let got = store.get("ura").unwrap();
        assert_eq!(got.len(), 1);
        assert_eq!(got[0]["local_name"], "write");
    }

    #[test]
    fn stale_revision_does_not_replace_newer_projection() {
        let store = AbilityCatalogStore::new();
        let newer = projection_row_with_revision("ura", 7, "sha256:new", vec![summary("write")]);
        let stale = projection_row_with_revision("ura", 6, "sha256:old", vec![summary("read")]);

        assert_eq!(
            store.upsert_projection(newer),
            ProjectionUpsertOutcome::Inserted
        );
        assert_eq!(
            store.upsert_projection(stale),
            ProjectionUpsertOutcome::IgnoredStale
        );

        let got = store.get("ura").unwrap();
        assert_eq!(got.len(), 1);
        assert_eq!(got[0]["local_name"], "write");
    }

    #[test]
    fn equal_revision_digest_conflict_does_not_replace_projection() {
        let store = AbilityCatalogStore::new();
        let first = projection_row_with_revision("ura", 7, "sha256:first", vec![summary("read")]);
        let conflict =
            projection_row_with_revision("ura", 7, "sha256:conflict", vec![summary("write")]);

        assert_eq!(
            store.upsert_projection(first.clone()),
            ProjectionUpsertOutcome::Inserted
        );
        assert_eq!(
            store.upsert_projection(first),
            ProjectionUpsertOutcome::Idempotent
        );
        assert_eq!(
            store.upsert_projection(conflict),
            ProjectionUpsertOutcome::RejectedConflict
        );

        let got = store.get("ura").unwrap();
        assert_eq!(got.len(), 1);
        assert_eq!(got[0]["local_name"], "read");
    }

    #[test]
    fn equal_revision_same_content_new_lease_refreshes_projection() {
        let store = AbilityCatalogStore::new();
        let first = projection_row_with_revision_and_lease(
            "ura",
            7,
            "sha256:first",
            1_000,
            vec![summary("read")],
        );
        let refresh = projection_row_with_revision_and_lease(
            "ura",
            7,
            "sha256:refreshed-lease",
            2_000,
            vec![summary("read")],
        );

        assert_eq!(
            store.upsert_projection(first),
            ProjectionUpsertOutcome::Inserted
        );
        assert_eq!(
            store.upsert_projection(refresh),
            ProjectionUpsertOutcome::Updated
        );

        let row = store.projection_for_owner("ura").expect("projection");
        assert_eq!(row.projection_digest(), "sha256:refreshed-lease");
        assert_eq!(row.lease_expires_unix_ms(), 2_000);
        assert!(store.get_at("ura", 1_500).is_some());
    }

    #[test]
    fn upsert_empty_projection_stores_empty_not_none() {
        let store = AbilityCatalogStore::new();

        assert_eq!(
            store.upsert_projection(projection_row("ura", Vec::new())),
            ProjectionUpsertOutcome::Inserted
        );

        let got = store
            .get("ura")
            .expect("empty advertise still surfaces a row");
        assert!(got.is_empty());
    }

    #[test]
    fn expired_projection_is_not_returned_to_resolve_readers() {
        let store = AbilityCatalogStore::new();

        assert_eq!(
            store.upsert_projection(OwnerAbilityProjectionRow::new(
                "ura".to_string(),
                crate::core::ura::device_ura("easynet.run", "abc"),
                1,
                1,
                "sha256:digest".to_string(),
                1_000,
                vec![summary("read")],
            )),
            ProjectionUpsertOutcome::Inserted
        );

        assert!(store.get_at("ura", 999).is_some());
        assert!(store.get_at("ura", 1_000).is_none());
        assert!(store.get_at("ura", 1_001).is_none());
    }

    #[test]
    fn heartbeat_refresh_extends_only_a_finite_projection_lease() {
        let store = AbilityCatalogStore::new();
        assert_eq!(
            store.upsert_projection(projection_row_with_revision_and_lease(
                "ura",
                1,
                "sha256:digest",
                1_000,
                vec![summary("read")],
            )),
            ProjectionUpsertOutcome::Inserted
        );

        assert!(store.refresh_lease("ura", 2_000));
        let row = store.projection_for_owner("ura").expect("projection");
        assert_eq!(row.lease_expires_unix_ms(), 2_000);
        assert_eq!(row.projection_revision(), 1);
        assert_eq!(row.projection_digest(), "sha256:digest");
    }

    #[test]
    fn heartbeat_refresh_preserves_non_expiring_owner_projection() {
        let store = AbilityCatalogStore::new();
        assert_eq!(
            store.upsert_projection(projection_row_with_revision_and_lease(
                "ura",
                1,
                "sha256:digest",
                0,
                vec![summary("read")],
            )),
            ProjectionUpsertOutcome::Inserted
        );

        assert!(store.refresh_lease("ura", 2_000));
        let row = store.projection_for_owner("ura").expect("projection");
        assert_eq!(row.lease_expires_unix_ms(), 0);
    }

    fn projection_row(
        owner_ura: &str,
        ability_summaries: Vec<AbilityProjectionSummary>,
    ) -> OwnerAbilityProjectionRow {
        projection_row_with_revision(owner_ura, 1, "sha256:digest", ability_summaries)
    }

    fn projection_row_with_revision(
        owner_ura: &str,
        revision: u64,
        digest: &str,
        ability_summaries: Vec<AbilityProjectionSummary>,
    ) -> OwnerAbilityProjectionRow {
        projection_row_with_revision_and_lease(
            owner_ura,
            revision,
            digest,
            4_102_444_800_000,
            ability_summaries,
        )
    }

    fn projection_row_with_revision_and_lease(
        owner_ura: &str,
        revision: u64,
        digest: &str,
        lease_expires_unix_ms: i64,
        ability_summaries: Vec<AbilityProjectionSummary>,
    ) -> OwnerAbilityProjectionRow {
        OwnerAbilityProjectionRow::new(
            owner_ura.to_string(),
            crate::core::ura::device_ura("easynet.run", "abc"),
            1,
            revision,
            digest.to_string(),
            lease_expires_unix_ms,
            ability_summaries,
        )
    }

    fn summary(local_name: &str) -> AbilityProjectionSummary {
        let ability_id = format!("fs.{local_name}");
        let owner_ura = crate::core::ura::device_ura("easynet.run", "abc");
        AbilityProjectionSummary {
            ability_ura: crate::core::ura::device_ability_ura("easynet.run", "abc", &ability_id),
            owner_ura,
            namespace: "fs".to_string(),
            local_name: local_name.to_string(),
            descriptor_revision: "sha256:descriptor".to_string(),
            schema_ref: None,
            schema_hash: None,
            policy_ref: "visibility:PUBLIC".to_string(),
            route_summary_ref: None,
            tags: vec!["class:unary".to_string()],
            callable_summary: crate::daemon::federation::read_model::owner_projection::AbilityCallableSummary::minimal(
                ability_id,
            ),
        }
    }
}
