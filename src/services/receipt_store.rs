// EasyNet CLI — admission receipt store (RFC 001 §5.3 + DEC-012)
// ===============================================================
//
// File: src/services/receipt_store.rs
//
// PR-10 commit 2/N. The bounded in-memory store the admission gate
// records signed `InvocationReceipt`s into. Mirrors the
// `SharedNonceReplayStore` shape (Arc<Mutex<…>> wrapper around a
// single-threaded inner type) so the daemon can share one store
// across every concurrent admission RPC without coordination.
//
// Why bounded in-memory
// ---------------------
// PR-10 spec INV-2: receipt emission unconditional on the strict
// path. INV-5: receipt failure does NOT fail admission. Together
// these mean the store must be cheap (no disk I/O on the hot
// path) and bounded (otherwise long-running daemons accumulate
// receipts forever and OOM). FIFO eviction at 10 000 entries gives
// ~1h of audit history at 3 ops/s — enough for the canary's 24h
// soak window when paired with periodic out-of-band drain.
//
// What this store is NOT
// ----------------------
// - Not a WAL. Persistence is a future RFC concern; the v1 store
//   is in-memory only. A daemon restart loses every receipt.
// - Not an audit query surface. v1 exposes `record` + `len` +
//   `is_empty` + `snapshot_recent`. Federated audit query
//   (RFC-N PR-N5) introduces a richer subscription/tap API.
// - Not a chain root. Receipts carry `prev_receipt_hash =
//   [0u8; 32]` for v1; multi-receipt chains within one invocation
//   are RFC-N PR-N5 territory.
//
// Author: Silan.Hu <silan.hu@u.nus.edu>
// Copyright (c) 2026 EasyNet. All rights reserved.

use std::collections::VecDeque;
use std::sync::{Arc, Mutex};

use crate::pb::axon::v1::InvocationReceipt;

/// Default bounded capacity. ~1h at 3 ops/s; tunable via
/// [`SharedReceiptStore::with_capacity`] for tests / canaries.
pub const DEFAULT_RECEIPT_CAPACITY: usize = 10_000;

/// Daemon-side receipt store shared across every admission RPC.
/// Cheap to clone (`Arc<Mutex<…>>`); production threads one shared
/// instance through `AdmissionFacade` at boot.
#[derive(Clone, Debug)]
pub struct SharedReceiptStore {
    inner: Arc<Mutex<ReceiptStoreInner>>,
}

#[derive(Debug)]
struct ReceiptStoreInner {
    /// Fixed-capacity ring buffer. `push_back` evicts the front
    /// when at capacity — FIFO eviction so the most recent
    /// `capacity` receipts are retained.
    receipts: VecDeque<InvocationReceipt>,
    capacity: usize,
}

impl SharedReceiptStore {
    /// Build a store with the default capacity
    /// (`DEFAULT_RECEIPT_CAPACITY`).
    #[must_use]
    pub fn new() -> Self {
        Self::with_capacity(DEFAULT_RECEIPT_CAPACITY)
    }

    /// Build a store with a custom capacity. Tests use small caps
    /// to exercise the eviction path; production uses
    /// `DEFAULT_RECEIPT_CAPACITY`.
    #[must_use]
    pub fn with_capacity(capacity: usize) -> Self {
        let capacity = capacity.max(1);
        Self {
            inner: Arc::new(Mutex::new(ReceiptStoreInner {
                receipts: VecDeque::with_capacity(capacity),
                capacity,
            })),
        }
    }

    /// Append a receipt. If the store is at capacity, the oldest
    /// receipt is evicted before the new one is recorded. Lock
    /// contention is non-blocking on the admission path: the
    /// critical section is one `pop_front` + one `push_back`. If
    /// the lock is poisoned (a previous admission panicked while
    /// holding it), we recover via `into_inner` and continue —
    /// matching the `SharedNonceReplayStore` poison-handling
    /// pattern.
    pub fn record(&self, receipt: InvocationReceipt) {
        let mut guard = match self.inner.lock() {
            Ok(g) => g,
            Err(poisoned) => poisoned.into_inner(),
        };
        if guard.receipts.len() >= guard.capacity {
            guard.receipts.pop_front();
        }
        guard.receipts.push_back(receipt);
    }

    /// Number of receipts currently retained. Test/observability
    /// only.
    #[must_use]
    pub fn len(&self) -> usize {
        match self.inner.lock() {
            Ok(g) => g.receipts.len(),
            Err(poisoned) => poisoned.into_inner().receipts.len(),
        }
    }

    /// Whether the store has zero receipts. Test-only convenience.
    #[must_use]
    pub fn is_empty(&self) -> bool {
        self.len() == 0
    }

    /// Snapshot of the most-recent up-to-`limit` receipts in the
    /// store, oldest-first. Used by tests and (eventually) by the
    /// audit query surface RFC-N PR-N5 will introduce. Cloning is
    /// O(n) in `limit`; callers picking a small `limit` get cheap
    /// snapshots.
    #[must_use]
    pub fn snapshot_recent(&self, limit: usize) -> Vec<InvocationReceipt> {
        let guard = match self.inner.lock() {
            Ok(g) => g,
            Err(poisoned) => poisoned.into_inner(),
        };
        let len = guard.receipts.len();
        let take = limit.min(len);
        guard.receipts.iter().skip(len - take).cloned().collect()
    }
}

impl Default for SharedReceiptStore {
    fn default() -> Self {
        Self::new()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn fixture_receipt(invocation_id: &str) -> InvocationReceipt {
        InvocationReceipt {
            invocation_id: invocation_id.to_string(),
            ..InvocationReceipt::default()
        }
    }

    #[test]
    fn record_increments_len() {
        let store = SharedReceiptStore::new();
        assert!(store.is_empty());
        store.record(fixture_receipt("inv-1"));
        assert_eq!(store.len(), 1);
        store.record(fixture_receipt("inv-2"));
        assert_eq!(store.len(), 2);
    }

    #[test]
    fn evicts_oldest_at_capacity() {
        let store = SharedReceiptStore::with_capacity(3);
        store.record(fixture_receipt("inv-1"));
        store.record(fixture_receipt("inv-2"));
        store.record(fixture_receipt("inv-3"));
        store.record(fixture_receipt("inv-4")); // evicts inv-1
        assert_eq!(store.len(), 3, "ring at capacity stays at capacity");
        let recent = store.snapshot_recent(10);
        let ids: Vec<_> = recent.iter().map(|r| r.invocation_id.clone()).collect();
        assert_eq!(ids, vec!["inv-2", "inv-3", "inv-4"]);
    }

    #[test]
    fn snapshot_recent_respects_limit() {
        let store = SharedReceiptStore::with_capacity(10);
        for n in 0..5 {
            store.record(fixture_receipt(&format!("inv-{n}")));
        }
        let recent = store.snapshot_recent(2);
        let ids: Vec<_> = recent.iter().map(|r| r.invocation_id.clone()).collect();
        assert_eq!(ids, vec!["inv-3", "inv-4"]);
    }

    #[test]
    fn shared_store_is_thread_safe() {
        // Simulate the daemon's tonic-worker concurrency: two
        // threads recording into the same store. Final len must
        // be the total of records issued.
        let store = SharedReceiptStore::new();
        let handles: Vec<_> = (0..4)
            .map(|t| {
                let store = store.clone();
                std::thread::spawn(move || {
                    for n in 0..25 {
                        store.record(fixture_receipt(&format!("t{t}-inv-{n}")));
                    }
                })
            })
            .collect();
        for h in handles {
            h.join().expect("thread join");
        }
        assert_eq!(store.len(), 100);
    }

    #[test]
    fn capacity_floor_at_one() {
        // Avoid a divide-by-zero or always-pop edge: capacity
        // requested as 0 promotes to 1.
        let store = SharedReceiptStore::with_capacity(0);
        store.record(fixture_receipt("inv-1"));
        assert_eq!(store.len(), 1);
        store.record(fixture_receipt("inv-2"));
        assert_eq!(store.len(), 1);
        let recent = store.snapshot_recent(10);
        assert_eq!(recent[0].invocation_id, "inv-2");
    }
}
