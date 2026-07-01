// EasyNet CLI — Services Layer — PendingDispatchMap
// ===================================================
//
// File: src/services/pending_dispatch.rs
// Description: Cross-call correlation table for `runtime.invoke_remote`
//              dispatches awaiting their reply on a target device's
//              `session.open` stream.
//
// Why this module exists
// ----------------------
// `runtime.invoke_remote` (PR-3, this commit) is the per-call bidi the
// caller opens against the daemon. The daemon pushes a `Dispatch`
// frame down the target device's pre-existing `session.open`
// reverse channel (PR-2 lands the session accept side). The target
// device runs the requested ability locally and writes a `Result`
// frame back up its session stream. PR-2's session-receive task
// must then route that `Result` back to the *original*
// `runtime.invoke_remote` caller — by call_id.
//
// `PendingDispatchMap` is the shared correlation surface:
//
//   * `runtime.invoke_remote` handler (PR-3, this commit's caller):
//     `register_pending(call_id) -> oneshot::Receiver<DispatchResult>`.
//     The handler awaits the receiver while a target session task
//     races to fulfil it.
//
//   * PR-2 `session.open` receive task (planned):
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
// 1. **Unique call_id per unary dispatch**: callers get monotonic IDs
//    from `next_call_id()`, then encode them into the even-numbered
//    namespace (`seq << 1`). `PendingStreamDispatchMap` reserves the
//    odd-numbered namespace for streamed bidi calls, so the two
//    routing tables can never race on the same session-wide `call_id`.
//    Collisions still only happen modulo a 64-bit wrap (geological time).
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
use tokio::sync::{mpsc, oneshot};

use crate::services::session_failure::SessionFailure;

#[cfg(feature = "axon-pb")]
pub type DispatchReceipt = easynet_axon::pb::axon::v1::InvocationReceipt;

#[cfg(not(feature = "axon-pb"))]
#[derive(Debug, Clone, PartialEq)]
pub enum DispatchReceipt {}

/// Result the target device sent back for a cross-device dispatch.
/// Mirrors the shape `session.open`'s receive task will hand off
/// when it sees a `Result` frame on the session up stream.
#[derive(Debug, Clone, PartialEq)]
pub struct DispatchResult {
    /// Reply payload from the target ability (opaque bytes).
    pub payload: Vec<u8>,
    /// Callee-signed execution receipt when the target spoke
    /// carrier-v1 (DEC-F004 landing audit 3). Internal projection
    /// field — never serialized; the invoke_remote consumer side
    /// projects it into the hub ledger where the full call context
    /// (ability, route) lives. `None` on the JSON carrier.
    pub receipt: Option<DispatchReceipt>,
    /// `Some(message)` if the target reported an execution error;
    /// `None` for a clean reply.
    pub error: Option<String>,
    /// Canonical terminal failure projection when the target supplied one.
    pub failure: Option<SessionFailure>,
    /// Target-side Axon ledger request id when available.
    pub request_id: Option<String>,
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

/// Per-call-id pending entry. Carries the `target_ura` of the
/// device the caller is waiting on so a `PresenceEvent::Offline`
/// for that URA can fail-fast every outstanding waiter targeting
/// it (PR-N6 mid-flight cancellation: pre-fix the daemon's
/// `forward_invoke` `await_reply()` blocked indefinitely when the
/// target session dropped, surfacing on the operator side as a
/// 30s HTTP timeout instead of an immediate `target_offline`).
struct PendingEntry {
    sender: oneshot::Sender<DispatchResult>,
    target_ura: String,
}

#[derive(Debug, Default)]
struct PendingDispatchInner {
    entries: DashMap<u64, PendingEntry>,
    next_call_id: AtomicU64,
}

impl std::fmt::Debug for PendingEntry {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("PendingEntry")
            .field("target_ura", &self.target_ura)
            .field("sender", &"<oneshot::Sender>")
            .finish()
    }
}

/// Shared correlation table between `runtime.invoke_remote` (writer)
/// and `session.open` (completer). Constructed once per daemon
/// process; cloned by `Arc` into each handler that needs it.
#[derive(Debug, Clone, Default)]
pub struct PendingDispatchMap {
    inner: Arc<PendingDispatchInner>,
}

impl PendingDispatchMap {
    pub fn new() -> Self {
        Self::default()
    }

    /// Register a new pending dispatch with no target URA. Kept for
    /// callers that haven't been wired to the cancel-on-offline path
    /// yet — they simply won't be auto-cancelled when the target
    /// goes offline (the legacy "wait until oneshot drops" behaviour).
    pub fn register_pending(&self) -> PendingHandle {
        self.register_pending_for("")
    }

    /// Register a new pending dispatch keyed to a specific
    /// `target_ura`. When that URA's session goes offline the
    /// daemon's presence-event watcher calls `cancel_for(ura,
    /// "target_offline")` to release every outstanding waiter
    /// immediately, instead of letting the caller block on the
    /// HTTP / gRPC request timeout (30s) for a session that's
    /// already known-dead.
    pub fn register_pending_for(&self, target_ura: &str) -> PendingHandle {
        let sequence = self.inner.next_call_id.fetch_add(1, Ordering::Relaxed);
        let call_id = sequence << 1;
        let (tx, rx) = oneshot::channel();
        self.inner.entries.insert(
            call_id,
            PendingEntry {
                sender: tx,
                target_ura: target_ura.to_string(),
            },
        );
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
            Some((_, entry)) => entry.sender.send(result).is_ok(),
            None => false,
        }
    }

    /// Cancel every outstanding pending dispatch whose `target_ura`
    /// matches. Called from the daemon's presence-event watcher
    /// when a `session.open` reverse channel drops — without this,
    /// `forward_invoke` callers would block on `oneshot::Receiver`
    /// until their HTTP request timeout fired (typically 30s) for a
    /// target whose offline state is already known. Returns the
    /// number of entries cancelled.
    pub fn cancel_for(&self, target_ura: &str, error_reason: &str) -> usize {
        let to_cancel: Vec<u64> = self
            .inner
            .entries
            .iter()
            .filter(|e| e.value().target_ura == target_ura)
            .map(|e| *e.key())
            .collect();
        let mut count = 0;
        for call_id in to_cancel {
            if let Some((_, entry)) = self.inner.entries.remove(&call_id) {
                let _ = entry.sender.send(DispatchResult {
                    payload: Vec::new(),
                    error: Some(error_reason.to_string()),
                    failure: Some(SessionFailure::from_reason(
                        error_reason,
                        "TARGET_NOT_IN_PRESENCE_REGISTRY",
                        true,
                    )),
                    request_id: None,
                    receipt: None,
                });
                count += 1;
            }
        }
        count
    }

    /// Number of currently-outstanding pending dispatches. Used by
    /// the daemon boot log + PR-10 canary verification + tests.
    pub fn outstanding(&self) -> usize {
        self.inner.entries.len()
    }
}

/// One streamed event flowing back from a target device's remote
/// bidi session. Same-hub remote `fs.transfer` uses this:
/// zero or more `Chunk`s followed by exactly one `Terminal`.
#[derive(Debug, Clone, PartialEq)]
pub enum DispatchStreamEvent {
    Chunk(Vec<u8>),
    Terminal(Box<DispatchResult>),
}

/// Outcome of a non-blocking delivery into a pending stream entry.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum StreamDeliver {
    Delivered,
    /// No live entry — the caller cancelled or already finished.
    NoMatch,
    /// The consumer's per-call channel was full: it stopped
    /// draining. The entry has been evicted so the stalled call is
    /// cancelled instead of blocking the shared session drain.
    ConsumerStalled,
}

pub struct PendingStreamHandle {
    call_id: u64,
    map: Arc<PendingStreamDispatchInner>,
    rx: Option<mpsc::Receiver<DispatchStreamEvent>>,
}

impl PendingStreamHandle {
    pub fn call_id(&self) -> u64 {
        self.call_id
    }

    pub async fn recv(&mut self) -> Option<DispatchStreamEvent> {
        let rx = self.rx.as_mut().expect("recv called after stream taken");
        rx.recv().await
    }
}

impl Drop for PendingStreamHandle {
    fn drop(&mut self) {
        self.map.entries.remove(&self.call_id);
    }
}

#[derive(Debug, Default)]
struct PendingStreamDispatchInner {
    entries: DashMap<u64, PendingStreamEntry>,
    next_call_id: AtomicU64,
}

struct PendingStreamEntry {
    sender: mpsc::Sender<DispatchStreamEvent>,
    target_ura: String,
}

impl std::fmt::Debug for PendingStreamEntry {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("PendingStreamEntry")
            .field("target_ura", &self.target_ura)
            .field("sender", &"<mpsc::Sender>")
            .finish()
    }
}

#[derive(Debug, Clone, Default)]
pub struct PendingStreamDispatchMap {
    inner: Arc<PendingStreamDispatchInner>,
}

impl PendingStreamDispatchMap {
    const CHANNEL_CAPACITY: usize = 32;

    pub fn new() -> Self {
        Self::default()
    }

    pub fn register_pending(&self) -> PendingStreamHandle {
        self.register_pending_for("")
    }

    pub fn register_pending_for(&self, target_ura: &str) -> PendingStreamHandle {
        // SessionDispatch::Result frames share one session-wide
        // keyspace across unary and streaming paths. Reserve odd
        // call_ids for streaming so a late terminal/chunk frame cannot
        // accidentally complete a unary waiter (or vice versa).
        let sequence = self.inner.next_call_id.fetch_add(1, Ordering::Relaxed);
        let call_id = (sequence << 1) | 1;
        let (tx, rx) = mpsc::channel(Self::CHANNEL_CAPACITY);
        self.inner.entries.insert(
            call_id,
            PendingStreamEntry {
                sender: tx,
                target_ura: target_ura.to_string(),
            },
        );
        PendingStreamHandle {
            call_id,
            map: Arc::clone(&self.inner),
            rx: Some(rx),
        }
    }

    pub async fn push_chunk(&self, call_id: u64, payload: Vec<u8>) -> bool {
        let Some(sender) = self
            .inner
            .entries
            .get(&call_id)
            .map(|entry| entry.sender.clone())
        else {
            return false;
        };
        sender
            .send(DispatchStreamEvent::Chunk(payload))
            .await
            .is_ok()
    }

    pub async fn finish(&self, call_id: u64, result: DispatchResult) -> bool {
        match self.inner.entries.remove(&call_id) {
            Some((_, entry)) => entry
                .sender
                .send(DispatchStreamEvent::Terminal(Box::new(result)))
                .await
                .is_ok(),
            None => false,
        }
    }

    /// Non-blocking `push_chunk` for the session drain. The drain is
    /// the ONE reader of a device's whole `session.open`; awaiting
    /// a full per-call channel there lets a single stalled consumer
    /// block every other invocation on that device (measured
    /// 2026-06-12: one unread streaming caller froze all calls to
    /// the device until it went away). On `Full` the entry is
    /// evicted: the stalled call alone is cut — its consumer sees
    /// end-of-stream after the buffered chunks — and the drain moves
    /// on.
    pub fn try_push_chunk(&self, call_id: u64, payload: Vec<u8>) -> StreamDeliver {
        let Some(sender) = self
            .inner
            .entries
            .get(&call_id)
            .map(|entry| entry.sender.clone())
        else {
            return StreamDeliver::NoMatch;
        };
        match sender.try_send(DispatchStreamEvent::Chunk(payload)) {
            Ok(()) => StreamDeliver::Delivered,
            Err(mpsc::error::TrySendError::Full(_)) => {
                self.inner.entries.remove(&call_id);
                StreamDeliver::ConsumerStalled
            }
            Err(mpsc::error::TrySendError::Closed(_)) => {
                self.inner.entries.remove(&call_id);
                StreamDeliver::NoMatch
            }
        }
    }

    /// Non-blocking `finish`, same drain-protection rationale as
    /// `try_push_chunk`. Unary calls only ever see the terminal
    /// event, so their channel can never be full — `ConsumerStalled`
    /// is reachable only for a streaming consumer that already
    /// stopped draining its chunks.
    pub fn try_finish(&self, call_id: u64, result: DispatchResult) -> StreamDeliver {
        match self.inner.entries.remove(&call_id) {
            Some((_, entry)) => {
                match entry
                    .sender
                    .try_send(DispatchStreamEvent::Terminal(Box::new(result)))
                {
                    Ok(()) => StreamDeliver::Delivered,
                    Err(mpsc::error::TrySendError::Full(_)) => StreamDeliver::ConsumerStalled,
                    Err(mpsc::error::TrySendError::Closed(_)) => StreamDeliver::NoMatch,
                }
            }
            None => StreamDeliver::NoMatch,
        }
    }

    pub fn cancel_for(&self, target_ura: &str, error_reason: &str) -> usize {
        let to_cancel: Vec<u64> = self
            .inner
            .entries
            .iter()
            .filter(|e| e.value().target_ura == target_ura)
            .map(|e| *e.key())
            .collect();
        let mut count = 0;
        for call_id in to_cancel {
            if let Some((_, entry)) = self.inner.entries.remove(&call_id) {
                let _ = entry
                    .sender
                    .try_send(DispatchStreamEvent::Terminal(Box::new(DispatchResult {
                        payload: Vec::new(),
                        error: Some(error_reason.to_string()),
                        failure: Some(SessionFailure::from_reason(
                            error_reason,
                            "TARGET_NOT_IN_PRESENCE_REGISTRY",
                            true,
                        )),
                        request_id: None,
                        receipt: None,
                    })));
                count += 1;
            }
        }
        count
    }

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
                    failure: None,
                    request_id: None,
                    receipt: None,
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
                receipt: None,
                payload: b"too late".to_vec(),
                error: None,
                failure: None,
                request_id: None,
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
            let failure =
                SessionFailure::from_reason("target ability raised", "INVOCATION_FAILED", false);
            map_clone.complete(
                id,
                DispatchResult {
                    payload: Vec::new(),
                    error: Some("target ability raised".into()),
                    failure: Some(failure),
                    request_id: None,
                    receipt: None,
                },
            )
        });

        let result = handle.await_reply().await.expect("reply received");
        assert_eq!(result.error.as_deref(), Some("target ability raised"));
        assert_eq!(
            result.failure.as_ref().map(|failure| failure.code.as_str()),
            Some("INVOCATION_FAILED")
        );
    }

    #[tokio::test]
    async fn call_ids_are_monotonic() {
        let map = PendingDispatchMap::new();
        let h1 = map.register_pending();
        let h2 = map.register_pending();
        let h3 = map.register_pending();
        assert_eq!(h1.call_id() + 2, h2.call_id());
        assert_eq!(h2.call_id() + 2, h3.call_id());
        assert_eq!(
            h1.call_id() & 1,
            0,
            "unary call_ids live in the even namespace"
        );
        assert_eq!(
            h2.call_id() & 1,
            0,
            "unary call_ids live in the even namespace"
        );
        assert_eq!(
            h3.call_id() & 1,
            0,
            "unary call_ids live in the even namespace"
        );
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
                    failure: None,
                    request_id: None,
                    receipt: None,
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
        drop(removed);

        let result = handle.await_reply().await;
        assert!(result.is_err(), "dropped sender surfaces as RecvError");
    }

    #[tokio::test]
    async fn pending_stream_map_yields_chunk_then_terminal() {
        let map = PendingStreamDispatchMap::new();
        let mut handle = map.register_pending();
        let id = handle.call_id();

        let writer = {
            let map = map.clone();
            tokio::spawn(async move {
                assert!(map.push_chunk(id, b"part-1".to_vec()).await);
                assert!(
                    map.finish(
                        id,
                        DispatchResult {
                            receipt: None,
                            payload: br#"{"sha256":"abc"}"#.to_vec(),
                            error: None,
                            failure: None,
                            request_id: None,
                        },
                    )
                    .await
                );
            })
        };

        assert_eq!(
            handle.recv().await,
            Some(DispatchStreamEvent::Chunk(b"part-1".to_vec()))
        );
        assert_eq!(
            handle.recv().await,
            Some(DispatchStreamEvent::Terminal(Box::new(DispatchResult {
                receipt: None,
                payload: br#"{"sha256":"abc"}"#.to_vec(),
                error: None,
                failure: None,
                request_id: None,
            })))
        );
        writer.await.expect("writer joined");
    }

    #[tokio::test]
    async fn pending_stream_map_cancel_for_target_yields_terminal_failure_after_chunks() {
        let map = PendingStreamDispatchMap::new();
        let mut handle = map.register_pending_for("easynet:///r/realm/device/target");
        let id = handle.call_id();

        assert_eq!(map.outstanding(), 1);
        assert_eq!(
            map.try_push_chunk(id, b"partial-frame".to_vec()),
            StreamDeliver::Delivered
        );
        assert_eq!(
            map.cancel_for("easynet:///r/realm/device/other", "target_offline"),
            0,
            "non-matching target URA must not cancel this stream"
        );
        assert_eq!(map.outstanding(), 1);
        assert_eq!(
            map.cancel_for("easynet:///r/realm/device/target", "target_offline"),
            1,
            "matching target URA must cancel the pending stream"
        );
        assert_eq!(map.outstanding(), 0);

        assert_eq!(
            handle.recv().await,
            Some(DispatchStreamEvent::Chunk(b"partial-frame".to_vec()))
        );
        let Some(DispatchStreamEvent::Terminal(result)) = handle.recv().await else {
            panic!("cancel_for must deliver one terminal failure event");
        };
        assert!(result.payload.is_empty());
        assert_eq!(result.error.as_deref(), Some("target_offline"));
        let failure = result.failure.expect("typed terminal failure");
        assert_eq!(failure.code, "TARGET_OFFLINE");
        assert!(failure.retryable);
        assert_eq!(
            handle.recv().await,
            None,
            "stream handle must close after terminal failure"
        );
    }

    #[tokio::test]
    async fn pending_stream_handle_drop_removes_entry() {
        let map = PendingStreamDispatchMap::new();
        let handle = map.register_pending();
        let id = handle.call_id();
        assert_eq!(map.outstanding(), 1);
        drop(handle);
        assert_eq!(map.outstanding(), 0);
        assert!(
            !map.finish(
                id,
                DispatchResult {
                    payload: Vec::new(),
                    error: None,
                    failure: None,
                    request_id: None,
                    receipt: None,
                },
            )
            .await
        );
    }

    #[tokio::test]
    async fn unary_and_stream_call_id_namespaces_do_not_overlap() {
        let unary = PendingDispatchMap::new();
        let stream = PendingStreamDispatchMap::new();

        let unary_one = unary.register_pending();
        let unary_two = unary.register_pending();
        let stream_one = stream.register_pending();
        let stream_two = stream.register_pending();

        assert_eq!(unary_one.call_id() & 1, 0);
        assert_eq!(unary_two.call_id() & 1, 0);
        assert_eq!(stream_one.call_id() & 1, 1);
        assert_eq!(stream_two.call_id() & 1, 1);

        assert_ne!(unary_one.call_id(), stream_one.call_id());
        assert_ne!(unary_one.call_id(), stream_two.call_id());
        assert_ne!(unary_two.call_id(), stream_one.call_id());
        assert_ne!(unary_two.call_id(), stream_two.call_id());
    }
}
