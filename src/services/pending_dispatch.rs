// EasyNet CLI — Services Layer — PendingDispatchMap
// ===================================================
//
// File: src/services/pending_dispatch.rs
// Description: Cross-call correlation table for `<self>.invoke_remote`
//              dispatches awaiting their reply on a target device's
//              `<self>.session` stream.
//
// Why this module exists
// ----------------------
// `<self>.invoke_remote` (PR-3, this commit) is the per-call bidi the
// caller opens against the daemon. The daemon pushes a `Dispatch`
// frame down the target device's pre-existing `<self>.session`
// reverse channel (PR-2 lands the session accept side). The target
// device runs the requested ability locally and writes a `Result`
// frame back up its session stream. PR-2's session-receive task
// must then route that `Result` back to the *original*
// `<self>.invoke_remote` caller — by call_id.
//
// `PendingDispatchMap` is the shared correlation surface:
//
//   * `<self>.invoke_remote` handler (PR-3, this commit's caller):
//     `register_pending(call_id) -> oneshot::Receiver<DispatchResult>`.
//     The handler awaits the receiver while a target session task
//     races to fulfil it.
//
//   * PR-2 `<self>.session` receive task (planned):
//     `complete(call_id, DispatchResult)` invoked when the device's
//     `Result { call_id, ... }` frame arrives. The matching
//     oneshot is fulfilled and the invoke_remote handler wakes.
//
// Lifetime + bounded growth
// -------------------------
// Each `register_pending` adds one entry; either `complete` removes
// it on reply, or the handler drops the receiver on caller cancel
// and the entry's oneshot sender becomes a no-op on next `complete`
// — but the entry itself stays. To bound growth, `register_pending`
// returns a `PendingHandle` that auto-removes the entry on Drop, so
// caller cancellation reclaims the slot without explicit cleanup.
//
// Invariants
// ----------
// 1. **Unique call_id per dispatch**: callers get monotonic IDs from
//    `next_call_id()`. The map is keyed by this ID; collisions never
//    happen modulo a 64-bit wrap (geological time).
// 2. **At-most-once completion**: a `complete(id, ...)` after the
//    matching pending entry has been removed (caller cancelled or
//    a prior `complete` already fired) is a silent no-op rather
//    than an error — PR-2's session task does not need to know
//    whether the caller is still waiting.
// 3. **No ambient state besides the table + counter**: the map
//    holds only the DashMap of pending entries and the AtomicU64
//    counter. No timers, no background tasks. Caller-side timeout
//    is the caller's responsibility (e.g. `tokio::time::timeout`
//    around the receiver).
//
// Author: Silan Hu <silan.hu@u.nus.edu>
// Copyright (c) 2026 EasyNet. All rights reserved.

use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::Arc;

use dashmap::DashMap;
use tokio::sync::oneshot;

/// Result the target device sent back for a cross-device dispatch.
/// Mirrors the shape `<self>.session`'s receive task will hand off
/// when it sees a `Result` frame on the session up stream.
#[derive(Debug, Clone)]
pub struct DispatchResult {
    /// Reply payload from the target ability (opaque bytes).
    pub payload: Vec<u8>,
    /// `Some(message)` if the target reported an execution error;
    /// `None` for a clean reply.
    pub error: Option<String>,
}

/// Handle to one pending dispatch entry. Drop the handle to remove
/// the entry from the map (caller cancellation path); call
/// `await_reply` to consume the oneshot receiver.
///
/// The handle is the only public way to wait on a reply — callers
/// cannot await the bare oneshot because the cleanup-on-drop only
/// fires through this struct.
pub struct PendingHandle {
    call_id: u64,
    map: Arc<PendingDispatchInner>,
    rx: Option<oneshot::Receiver<DispatchResult>>,
}

impl PendingHandle {
    /// Block until the target's reply arrives or the sender is
    /// dropped. `Err(Cancelled)` means the matching session task
    /// dropped the sender without completing — surfaced as caller-
    /// visible "remote disconnected mid-call".
    pub async fn await_reply(mut self) -> Result<DispatchResult, oneshot::error::RecvError> {
        let rx = self.rx.take().expect("await_reply called twice");
        rx.await
    }

    /// Numeric ID assigned to this dispatch. The PR-3 handler
    /// embeds this in the `Dispatch` frame it pushes to the target
    /// session; the target echoes it back in its `Result` frame so
    /// PR-2 session task can route it back via `complete(call_id, ...)`.
    pub fn call_id(&self) -> u64 {
        self.call_id
    }
}

impl Drop for PendingHandle {
    fn drop(&mut self) {
        // Cleanup-on-drop: caller cancelled before the reply arrived,
        // or `await_reply` already consumed the receiver and is now
        // dropping the handle. Either way, evict the map entry. A
        // late `complete` after eviction is a silent no-op (Invariant 2).
        self.map.entries.remove(&self.call_id);
    }
}

#[derive(Debug, Default)]
struct PendingDispatchInner {
    entries: DashMap<u64, oneshot::Sender<DispatchResult>>,
    next_call_id: AtomicU64,
}

/// Shared correlation table between `<self>.invoke_remote` (writer)
/// and `<self>.session` (completer). Constructed once per daemon
/// process; cloned by `Arc` into each handler that needs it.
#[derive(Debug, Clone, Default)]
pub struct PendingDispatchMap {
    inner: Arc<PendingDispatchInner>,
}

impl PendingDispatchMap {
    pub fn new() -> Self {
        Self::default()
    }

    /// Register a new pending dispatch and return both the assigned
    /// call_id (in `PendingHandle::call_id`) and an awaitable handle
    /// that fulfils when the matching `complete(call_id, ...)` fires.
    pub fn register_pending(&self) -> PendingHandle {
        let call_id = self.inner.next_call_id.fetch_add(1, Ordering::Relaxed);
        let (tx, rx) = oneshot::channel();
        self.inner.entries.insert(call_id, tx);
        PendingHandle {
            call_id,
            map: Arc::clone(&self.inner),
            rx: Some(rx),
        }
    }

    /// Fulfil the pending entry for `call_id` with `result`. Silent
    /// no-op if the entry is gone (caller cancelled, prior complete
    /// already fired). Returns `true` on completion, `false` on the
    /// no-op path so the caller can log if it cares.
    pub fn complete(&self, call_id: u64, result: DispatchResult) -> bool {
        match self.inner.entries.remove(&call_id) {
            Some((_, sender)) => sender.send(result).is_ok(),
            None => false,
        }
    }

    /// Number of currently-outstanding pending dispatches. Used by
    /// the daemon boot log + PR-10 canary verification + tests.
    pub fn outstanding(&self) -> usize {
        self.inner.entries.len()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn new_map_is_empty() {
        let map = PendingDispatchMap::new();
        assert_eq!(map.outstanding(), 0);
    }

    #[tokio::test]
    async fn register_then_complete_delivers_result() {
        let map = PendingDispatchMap::new();
        let handle = map.register_pending();
        let id = handle.call_id();
        assert_eq!(map.outstanding(), 1);

        let map_for_complete = map.clone();
        let completer = tokio::spawn(async move {
            map_for_complete.complete(
                id,
                DispatchResult {
                    payload: b"reply".to_vec(),
                    error: None,
                },
            )
        });

        let result = handle.await_reply().await.expect("clean reply");
        assert_eq!(result.payload, b"reply");
        assert!(result.error.is_none());

        let completed = completer.await.expect("completer joined");
        assert!(completed, "complete should report true");
    }

    #[tokio::test]
    async fn drop_handle_removes_entry() {
        let map = PendingDispatchMap::new();
        let handle = map.register_pending();
        assert_eq!(map.outstanding(), 1);
        drop(handle);
        assert_eq!(map.outstanding(), 0);
    }

    #[tokio::test]
    async fn complete_after_drop_is_silent_noop() {
        let map = PendingDispatchMap::new();
        let handle = map.register_pending();
        let id = handle.call_id();
        drop(handle);

        let still_completes = map.complete(
            id,
            DispatchResult {
                payload: b"too late".to_vec(),
                error: None,
            },
        );
        assert!(!still_completes, "complete on dropped entry returns false");
    }

    #[tokio::test]
    async fn complete_with_error_propagates_to_handle() {
        let map = PendingDispatchMap::new();
        let handle = map.register_pending();
        let id = handle.call_id();

        let map_clone = map.clone();
        tokio::spawn(async move {
            map_clone.complete(
                id,
                DispatchResult {
                    payload: Vec::new(),
                    error: Some("target ability raised".into()),
                },
            )
        });

        let result = handle.await_reply().await.expect("reply received");
        assert_eq!(result.error.as_deref(), Some("target ability raised"));
    }

    #[tokio::test]
    async fn call_ids_are_monotonic() {
        let map = PendingDispatchMap::new();
        let h1 = map.register_pending();
        let h2 = map.register_pending();
        let h3 = map.register_pending();
        assert_eq!(h1.call_id() + 1, h2.call_id());
        assert_eq!(h2.call_id() + 1, h3.call_id());
    }

    #[tokio::test]
    async fn many_pending_entries_isolate_their_completions() {
        let map = PendingDispatchMap::new();
        let h1 = map.register_pending();
        let h2 = map.register_pending();
        let id2 = h2.call_id();

        // Complete only h2 — h1 should still be pending.
        let map_clone = map.clone();
        tokio::spawn(async move {
            map_clone.complete(
                id2,
                DispatchResult {
                    payload: b"two".to_vec(),
                    error: None,
                },
            )
        });

        let r2 = h2.await_reply().await.unwrap();
        assert_eq!(r2.payload, b"two");
        assert_eq!(map.outstanding(), 1, "h1 still pending after h2 completes");

        // Drop h1 to clean up the test.
        drop(h1);
        assert_eq!(map.outstanding(), 0);
    }

    #[tokio::test]
    async fn dropped_completer_surfaces_to_handle_as_recv_error() {
        let map = PendingDispatchMap::new();
        let handle = map.register_pending();

        // Simulate the session task crashing: drop the inner
        // sender by removing the entry without completing.
        let id = handle.call_id();
        let removed = map.inner.entries.remove(&id);
        assert!(removed.is_some(), "entry was present pre-drop");
        // `removed` goes out of scope here, dropping the sender.

        let result = handle.await_reply().await;
        assert!(result.is_err(), "dropped sender surfaces as RecvError");
    }
}
