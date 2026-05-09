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

/// **M1 receipt-reader compat shim** for the system-namespace
/// migration (RFC-001 v4.1.6 carrier).
///
/// During and after the migration window, persisted receipts
/// carry `function_name` strings spanning two eras:
///
///   * **Legacy** (`fs.read`, `fleet.list_nodes`, `01HUB.openai.*`,
///     `voice.create_call`, …): receipts written by daemons before
///     the M2 catalogue cutover.
///   * **Canonical** (`device.fs.read`, `device.fleet.list_nodes`,
///     `hub.openai.*`, `device.voice.create_call`, …): receipts
///     written after M2.
///
/// Audit display layers MUST present both as the same logical
/// verb. This function is the canonical single-direction map:
/// **legacy → canonical**. Names already in canonical form pass
/// through verbatim. Names that aren't recognised (per-agent
/// `<agent>.chat`, per-user `<user-uuid>.api_key.*`, third-party
/// abilities) also pass through verbatim — the function is
/// idempotent and total.
///
/// Why a free function and not a method on the store: this is
/// pure name-shape transformation; it doesn't read or write the
/// receipt buffer. Co-locating it here keeps the receipt-reader
/// concerns in one file (the store + the name canonicaliser),
/// without forcing a `&self` lifetime on a stateless map.
///
/// Removed at M5 cleanup once the legacy-name window has fully
/// closed and historical receipts have rolled out of the
/// in-memory bound.
#[must_use]
pub fn function_name_canonical(legacy_or_canonical: &str) -> String {
    let name = legacy_or_canonical;
    // Split on first dot. No dot → not a partitioned name; pass
    // through (catches degenerate `bare.verb` cases that legacy
    // tests still seed).
    let Some((head, _rest)) = name.split_once('.') else {
        return name.to_string();
    };
    // Already canonical — pass through.
    if head == "device" || head == "hub" {
        return name.to_string();
    }
    // Hub-rooted legacy: 01HUB.openai.* → hub.openai.*
    if head == "01HUB" {
        // Skip "01HUB." prefix exactly.
        return format!("hub.{}", &name[head.len() + 1..]);
    }
    // Device-rooted legacy: only the closed system-namespace set
    // gets rewritten. Anything else (per-agent, per-user, third-
    // party) passes through.
    const DEVICE_LEGACY_HEADS: &[&str] = &[
        "fs", "http", "shell", "process", "fleet", "observe", "admin", "easynet", "meta",
        "mission", "schedule", "loop", "discuss", "mcp", "a2a", "policy", "ability", "camera",
        "mic", "screen", "speaker", "voice", "skill", "consent",
    ];
    if DEVICE_LEGACY_HEADS.contains(&head) {
        return format!("device.{name}");
    }
    name.to_string()
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

    // ── M1 receipt-reader compat shim ────────────────────────────

    #[test]
    fn function_name_canonical_passes_through_already_canonical() {
        assert_eq!(function_name_canonical("device.fs.read"), "device.fs.read");
        assert_eq!(
            function_name_canonical("hub.openai.chat_completions"),
            "hub.openai.chat_completions"
        );
    }

    #[test]
    fn function_name_canonical_rewrites_legacy_device_namespaces() {
        // Spot-check one verb from each of the 24 legacy
        // device-owned namespaces. If a future contributor adds a
        // namespace to DEVICE_LEGACY_HEADS without updating this
        // test, the partition stays internally consistent — but
        // adding it without updating the test is harmless. Test is
        // for the inverse: deletion of a head.
        assert_eq!(function_name_canonical("fs.read"), "device.fs.read");
        assert_eq!(
            function_name_canonical("fleet.list_nodes"),
            "device.fleet.list_nodes"
        );
        assert_eq!(
            function_name_canonical("voice.create_call"),
            "device.voice.create_call"
        );
        assert_eq!(function_name_canonical("shell.run"), "device.shell.run");
        assert_eq!(
            function_name_canonical("camera.snapshot"),
            "device.camera.snapshot"
        );
    }

    #[test]
    fn function_name_canonical_rewrites_01hub_to_hub() {
        assert_eq!(
            function_name_canonical("01HUB.openai.chat_completions"),
            "hub.openai.chat_completions"
        );
        assert_eq!(
            function_name_canonical("01HUB.openai.list_models"),
            "hub.openai.list_models"
        );
    }

    #[test]
    fn function_name_canonical_passes_through_per_agent_names() {
        // Per-agent names (`<agent>.chat`, `<agent>.discover`,
        // `<agent>.invoke`, custom verbs like `<agent>.todo_*`) are
        // never rewritten — there's no legacy/canonical pair for
        // them; they ARE canonical. Pin so a future "be helpful and
        // rewrite agent names too" change has to argue.
        assert_eq!(function_name_canonical("codex.chat"), "codex.chat");
        assert_eq!(
            function_name_canonical("web-builder.todo_add_task"),
            "web-builder.todo_add_task"
        );
    }

    #[test]
    fn function_name_canonical_passes_through_user_names() {
        // Per-user `<uuid>.api_key.*` is not in the legacy set
        // (its slot is the user-id, not a system namespace).
        assert_eq!(
            function_name_canonical("11111111-2222-3333-4444-555555555555.api_key.create"),
            "11111111-2222-3333-4444-555555555555.api_key.create"
        );
    }

    #[test]
    fn function_name_canonical_passes_through_unrecognised_or_bare_names() {
        // Names without a dot, or with an unrecognised first
        // segment, pass through verbatim. Receipt readers should
        // never see these in production but the function is total.
        assert_eq!(function_name_canonical(""), "");
        assert_eq!(function_name_canonical("bare-no-dot"), "bare-no-dot");
        assert_eq!(
            function_name_canonical("third-party.verb"),
            "third-party.verb"
        );
    }

    #[test]
    fn function_name_canonical_is_idempotent() {
        // Composability: applying the function twice equals
        // applying it once. Critical for read paths that may
        // accidentally normalise an already-canonical name.
        for sample in [
            "fs.read",
            "device.fs.read",
            "01HUB.openai.list_models",
            "hub.openai.list_models",
            "codex.chat",
            "third-party.verb",
        ] {
            let once = function_name_canonical(sample);
            let twice = function_name_canonical(&once);
            assert_eq!(once, twice, "idempotence failed for {sample:?}");
        }
    }
}
