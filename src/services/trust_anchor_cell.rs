// EasyNet CLI — shared trust-anchor cell (PR-7 commit 5/N)
// =========================================================
//
// File: src/services/trust_anchor_cell.rs
// Description: A reload-friendly cell holding the daemon's current
//              `RealmTrustAnchor`. Built once at boot, shared by
//              the admission facade and the
//              `<self>.register_device_pubkey` ability handler.
//              Replaces an `Arc<RealmTrustAnchor>` so the pairing
//              flow can publish a fresh anchor (after
//              `append_agent` + atomic `save`) without rebuilding
//              the gRPC service.
//
// Why an RwLock<Arc<…>> rather than ArcSwap
// -----------------------------------------
// `ArcSwap` would be a tiny bit more elegant for this read-heavy
// pattern (admission reads on every RPC, writes are rare — pairing
// + SIGHUP-reload), but it's a new crate dependency for one usage.
// `std::sync::RwLock<Arc<…>>` is one-line equivalent: the lock is
// held only long enough to clone the inner `Arc`, then released
// before the per-call lookup runs. The clone is O(1) ref-count
// bump; admission then performs its lookup against a private
// snapshot Arc that no concurrent writer can disturb.
//
// On-write invariants
// -------------------
// Writers MUST construct the new `RealmTrustAnchor` *before*
// taking the write lock — building it from scratch keeps lock
// hold-time at a single pointer swap. The `replace` method below
// embodies this: caller passes a ready-to-publish anchor, lock is
// held for one assignment.
//
// Lock poisoning
// --------------
// `RwLock::read()` / `write()` return a poison error if a previous
// holder panicked while holding the lock. We recover via
// `into_inner()` so a single buggy panic in the writer doesn't
// wedge the daemon's admission gate. The `RealmTrustAnchor`'s
// invariants are upheld by construction (URI uniqueness checked
// in `from_entries` / `append_agent` before insert), so post-
// poison reads see a structurally-valid anchor.
//
// Author: Silan.Hu <silan.hu@u.nus.edu>
// Copyright (c) 2026 EasyNet. All rights reserved.

use std::sync::{
    atomic::{AtomicU64, Ordering},
    Arc, RwLock,
};

use crate::services::realm_trust_anchor::RealmTrustAnchor;

/// Shared, reload-friendly cell over the daemon's current
/// `RealmTrustAnchor`. Cheap to clone (one outer `Arc` bump).
///
/// DEC-N5 §5: every `replace` bumps a monotonic
/// `cert_anchor_generation` counter so the federation client's
/// channel pool can key cached channels by `(hub_uri, generation)`.
/// A SIGHUP reload that swaps the anchor (and therefore the per-
/// peer `tls_ca_pem_path`) bumps the counter, the next dial misses
/// the cache, and a fresh channel is built against the new CA.
/// In-flight calls on the old channel complete naturally — DEC-N5
/// §5 explicit contract.
#[derive(Debug, Clone)]
pub struct SharedTrustAnchor {
    inner: Arc<RwLock<Arc<RealmTrustAnchor>>>,
    /// Monotonic counter bumped on every successful `replace`.
    /// Boot-time anchors start at generation 0; the first SIGHUP
    /// reload that publishes a new anchor produces generation 1.
    /// `Acquire` reads pair with the `Release` write inside
    /// `replace` so a reader observes a generation no older than
    /// the anchor it just snapshotted.
    cert_anchor_generation: Arc<AtomicU64>,
}

impl SharedTrustAnchor {
    /// Build a cell wrapping `anchor` at generation 0.
    #[must_use]
    pub fn new(anchor: Arc<RealmTrustAnchor>) -> Self {
        Self {
            inner: Arc::new(RwLock::new(anchor)),
            cert_anchor_generation: Arc::new(AtomicU64::new(0)),
        }
    }

    /// Snapshot the current anchor as an `Arc`. The caller holds a
    /// stable view even if a concurrent writer publishes a new
    /// anchor mid-operation; later admissions will see the new one
    /// on their next `snapshot()` call.
    pub fn snapshot(&self) -> Arc<RealmTrustAnchor> {
        let guard = match self.inner.read() {
            Ok(g) => g,
            Err(poisoned) => poisoned.into_inner(),
        };
        Arc::clone(&guard)
    }

    /// DEC-N5 §5 generation counter. Read with `Acquire` so it
    /// pairs with `replace`'s `Release` write. The federation
    /// client uses `(hub_uri, generation)` as its channel cache
    /// key — a bump invalidates cached channels for the next
    /// dial without disturbing in-flight calls on the old channel.
    pub fn cert_anchor_generation(&self) -> u64 {
        self.cert_anchor_generation.load(Ordering::Acquire)
    }

    /// Atomically replace the held anchor with `next` AND bump
    /// `cert_anchor_generation` by 1. The bump happens *after* the
    /// anchor swap so a concurrent reader that observes the new
    /// generation is guaranteed to see the new anchor on its next
    /// `snapshot()` (Acquire/Release pairing).
    pub fn replace(&self, next: Arc<RealmTrustAnchor>) {
        let mut guard = match self.inner.write() {
            Ok(g) => g,
            Err(poisoned) => poisoned.into_inner(),
        };
        *guard = next;
        self.cert_anchor_generation.fetch_add(1, Ordering::Release);
    }
}

impl From<Arc<RealmTrustAnchor>> for SharedTrustAnchor {
    fn from(anchor: Arc<RealmTrustAnchor>) -> Self {
        Self::new(anchor)
    }
}

impl Default for SharedTrustAnchor {
    fn default() -> Self {
        Self::new(Arc::new(RealmTrustAnchor::default()))
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::services::realm_trust_anchor::{TrustedAgent, TrustedAgentRole};

    fn agent(uri: &str) -> TrustedAgent {
        TrustedAgent {
            agent_uri: uri.to_string(),
            public_key_b64: "AAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAA=".to_string(),
            role: TrustedAgentRole::Backend,
            added_at_unix_ms: 1_714_492_800_000,
            origin_tenant_id: None,
            hub_uri: None,
            tls_ca_pem_path: None,
        }
    }

    #[test]
    fn snapshot_returns_current_anchor() {
        let initial = Arc::new(
            RealmTrustAnchor::from_entries(vec![agent("easynet:///r/realm/agent/a")])
                .expect("anchor"),
        );
        let cell = SharedTrustAnchor::new(initial);
        assert_eq!(cell.snapshot().len(), 1);
    }

    #[test]
    fn replace_publishes_new_anchor_atomically() {
        let initial = Arc::new(RealmTrustAnchor::default());
        let cell = SharedTrustAnchor::new(initial);
        assert!(cell.snapshot().is_empty());

        let next = Arc::new(
            RealmTrustAnchor::from_entries(vec![
                agent("easynet:///r/realm/agent/a"),
                agent("easynet:///r/realm/agent/b"),
            ])
            .expect("anchor"),
        );
        cell.replace(next);
        assert_eq!(cell.snapshot().len(), 2);
    }

    #[test]
    fn snapshot_held_across_replace_remains_valid() {
        // A reader takes a snapshot, a writer replaces the anchor.
        // The reader's snapshot continues to point at the old
        // anchor — Arc semantics. This is the contract admission
        // depends on: a single RPC's view is consistent for its
        // duration even if a concurrent register flow publishes
        // mid-RPC.
        let initial = Arc::new(
            RealmTrustAnchor::from_entries(vec![agent("easynet:///r/realm/agent/initial")])
                .expect("anchor"),
        );
        let cell = SharedTrustAnchor::new(Arc::clone(&initial));

        let snapshot = cell.snapshot();
        assert!(snapshot
            .lookup("easynet:///r/realm/agent/initial")
            .is_some());

        let next = Arc::new(
            RealmTrustAnchor::from_entries(vec![agent("easynet:///r/realm/agent/next")])
                .expect("anchor"),
        );
        cell.replace(next);

        // Old snapshot still reflects the original anchor.
        assert!(snapshot
            .lookup("easynet:///r/realm/agent/initial")
            .is_some());
        assert!(snapshot.lookup("easynet:///r/realm/agent/next").is_none());
        // Fresh snapshot reflects the replacement.
        let fresh = cell.snapshot();
        assert!(fresh.lookup("easynet:///r/realm/agent/initial").is_none());
        assert!(fresh.lookup("easynet:///r/realm/agent/next").is_some());
    }

    #[test]
    fn cert_anchor_generation_bumps_on_each_replace() {
        // DEC-N5 §5: every successful `replace` bumps the
        // monotonic generation counter by exactly 1. Boot starts
        // at generation 0; the first reload publishes generation 1.
        // CrossHubDialer keys its channel pool by `(hub_uri,
        // generation)` so a bump invalidates cached channels for
        // the next dial — old channels stay valid for in-flight
        // calls per DEC-N5 §5 explicit contract.
        let cell = SharedTrustAnchor::new(Arc::new(RealmTrustAnchor::default()));
        assert_eq!(cell.cert_anchor_generation(), 0);
        cell.replace(Arc::new(RealmTrustAnchor::default()));
        assert_eq!(cell.cert_anchor_generation(), 1);
        cell.replace(Arc::new(RealmTrustAnchor::default()));
        assert_eq!(cell.cert_anchor_generation(), 2);

        // Cloned cells observe the same atomic counter.
        let cell_clone = cell.clone();
        assert_eq!(cell_clone.cert_anchor_generation(), 2);
        cell.replace(Arc::new(RealmTrustAnchor::default()));
        assert_eq!(cell_clone.cert_anchor_generation(), 3);
    }

    #[test]
    fn cloned_cell_observes_replacements_from_origin() {
        // Two clones of the same cell observe each other's writes —
        // the boot wiring hands one clone to the admission facade
        // and another to the register-pubkey ability handler.
        let cell = SharedTrustAnchor::default();
        let cell2 = cell.clone();
        let next = Arc::new(
            RealmTrustAnchor::from_entries(vec![agent("easynet:///r/realm/agent/x")])
                .expect("anchor"),
        );
        cell.replace(next);
        assert_eq!(cell2.snapshot().len(), 1);
    }
}
