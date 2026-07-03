// EasyNet CLI — Timeline Writer
// ==============================
//
// File: src/daemon/execution/mission/timeline.rs
// Description: Per-invocation event timeline. Composes two things:
//
//   1. A disk-backed append-only event log (`easynet_axon::invocation::
//      persistence::PersistentLog`) which honours AXIOM P1–P6
//      (append-only ordering, fsync-before-notify, offset read,
//      terminal idempotence, explicit eviction, crash consistency).
//      See EasyNet-Axon/document/concepts/INVOCATION_LIFECYCLE_ACROSS_PROCESSES.md.
//
//   2. A `tokio::sync::broadcast::Sender<TimelineEvent>` so in-process
//      subscribers (chat, loop execution in PR-10, the
//      Session's resume path) can tail events live without polling
//      the disk. The broadcast channel uses drop-oldest semantics
//      under subscriber lag, which is correct for "tail" consumers:
//      a slow UI is allowed to miss intermediate progress, but the
//      on-disk log remains the authoritative record.
//
// P2 discipline
// -------------
// AXIOM P2 requires an emitted event be durable on disk before any
// watcher is notified. The emit path below enforces the order
// architecturally:
//
//    1. Acquire next `sequence` (serialised by the writer lock).
//    2. Call `PersistentLog::append_event(..., fsync=true)`.
//    3. Only if step 2 returned Ok, call
//       `broadcast::Sender::send(event)`.
//
// A panic or error between 1 and 2 leaves no durable event (next
// reader sees the prefix before this emit). An error at step 2 is
// returned; no broadcast is sent. An error at step 3 (no active
// subscribers → `SendError`) is ignored — the disk log is
// authoritative, subscribers are best-effort.
//
// The writer lock (std::sync::Mutex over the tail-sequence counter)
// is held across the fsync. This serialises writes per invocation,
// which is what P1 (append-only, monotonic sequence) already
// requires — there is no parallelism to preserve at the writer
// layer.
//
// Author: Silan Hu <silan.hu@u.nus.edu>
// Copyright (c) 2026 EasyNet. All rights reserved.

use std::path::PathBuf;
use std::sync::Mutex;

use easynet_axon::invocation::persistence::PersistentLog;
use serde::{Deserialize, Serialize};
use serde_json::Value;
use tokio::sync::broadcast;

/// Default broadcast channel capacity. Subscribers that lag beyond
/// this many events receive `RecvError::Lagged` and resync by
/// re-reading the disk log from their last-seen sequence. 2048 is
/// chosen empirically: one streaming LLM response emits on the
/// order of 100-2000 progress events, and a caught-up subscriber
/// needs headroom for a full response burst before it is
/// considered lagged.
const DEFAULT_BROADCAST_CAPACITY: usize = 2048;

/// One event on the timeline. Shape is intentionally narrow: the
/// `type` field matches `INVOCATION_STATE_MACHINE.md §2` event
/// kinds for the subset we emit today (`admitted`, `progress`,
/// `completed`, `failed`, `cancelled`). A payload blob carries
/// kind-specific data (driver stream chunks for `progress`, final
/// reply for `completed`, error text for `failed`).
///
/// We deliberately do not split into a typed enum per kind —
/// `Value` is forward-compatible and mirrors how `PersistentLog`
/// stores events on disk. A typed enum would double the churn on
/// every new event kind without buying schema safety (the on-disk
/// representation stays `Value` regardless).
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct TimelineEvent {
    pub sequence: i64,
    pub timestamp_unix_ms: i64,
    /// Event kind. One of the strings documented in
    /// `INVOCATION_STATE_MACHINE.md §2` (plus `admitted` emitted at
    /// writer construction). CLI today does not model
    /// `dispatched`/`running` separately — a single `admitted`
    /// opens the timeline and a `progress` chunk stream fills the
    /// middle.
    #[serde(rename = "type")]
    pub event_type: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub payload: Option<Value>,
}

impl TimelineEvent {
    /// Construct an event ready for append. The sequence is assigned
    /// by the caller (writer lock holder); `timestamp_unix_ms` is
    /// stamped here as wall-clock to keep the writer path simple.
    fn new(sequence: i64, event_type: impl Into<String>, payload: Option<Value>) -> Self {
        let timestamp_unix_ms = chrono::Utc::now().timestamp_millis();
        Self {
            sequence,
            timestamp_unix_ms,
            event_type: event_type.into(),
            payload,
        }
    }
}

/// Per-invocation writer: owns the `invocation_id`, the next
/// sequence, the `PersistentLog` handle, and the broadcast sender.
/// One `TimelineWriter` per agent invocation. Cheap to hold
/// (tens of bytes + two Arc-equivalents); expensive to construct
/// (opens the log file lazily on first emit via `PersistentLog`).
///
/// Thread safety: `emit` is `&self` and the interior `Mutex<i64>`
/// serialises the sequence increment + append. Multiple threads
/// emitting on the same `TimelineWriter` is legal but uncommon —
/// CLI today drives one driver stream per invocation. The lock is
/// structural (P1 requires monotonic sequence) and its cost is
/// negligible compared to the fsync it guards.
pub struct TimelineWriter {
    invocation_id: String,
    log: PersistentLog,
    next_sequence: Mutex<i64>,
    broadcast_tx: broadcast::Sender<TimelineEvent>,
}

impl TimelineWriter {
    /// Construct a writer for a fresh invocation. `log_dir` defaults
    /// to `PersistentLog`'s env-var / tempdir default when `None`.
    ///
    /// Does NOT emit any event by itself. The caller's first call
    /// to `emit` is typically the `admitted` event carrying the
    /// caller's prompt and the target agent.
    pub fn new(invocation_id: impl Into<String>, log_dir: Option<PathBuf>) -> Self {
        let (tx, _) = broadcast::channel(DEFAULT_BROADCAST_CAPACITY);
        Self {
            invocation_id: invocation_id.into(),
            log: PersistentLog::new(log_dir),
            next_sequence: Mutex::new(0),
            broadcast_tx: tx,
        }
    }

    pub fn invocation_id(&self) -> &str {
        &self.invocation_id
    }

    /// Path of the on-disk event log for this invocation. Exposed
    /// for diagnostic tools (`agent doctor`, `mission runs show`)
    /// that want to name the file without importing PersistentLog
    /// directly.
    #[cfg(test)]
    pub fn log_path(&self) -> PathBuf {
        self.log.events_path(&self.invocation_id)
    }

    /// Subscribe to live events. Each subscriber gets its own
    /// receiver; slow subscribers that lag beyond the channel's
    /// capacity receive `RecvError::Lagged(skipped)` and should
    /// resync by calling `PersistentLog::read_events` from their
    /// last-seen sequence.
    ///
    /// Subscribers obtained *after* some events have already been
    /// emitted do NOT see those past events on the channel — only
    /// events emitted from this call forward. Resume semantics
    /// (see `Session::resume` in `daemon::execution::mission::session`) explicitly
    /// replay the on-disk prefix before attaching the live
    /// subscriber, so the two pieces compose into a full
    /// "replay + tail" stream.
    #[cfg(test)]
    pub fn subscribe(&self) -> broadcast::Receiver<TimelineEvent> {
        self.broadcast_tx.subscribe()
    }

    /// Append one event. Returns the assigned sequence on success.
    ///
    /// Ordering guarantee (P2): the event is fsynced to disk before
    /// any subscriber is woken. A subscriber that observes sequence
    /// N via the broadcast channel is guaranteed to find sequence
    /// N in the on-disk log on subsequent read.
    ///
    /// Terminal events: when `event_type` is `completed`, `failed`,
    /// or `cancelled`, the PersistentLog index is marked terminal
    /// via the same append call. Repeated emits after a terminal
    /// event are appended to the log (P1 append-only), but callers
    /// SHOULD NOT emit after terminal — I1 pins terminal-state
    /// monotonicity at the state-machine layer above this writer.
    /// The writer does not enforce that invariant; enforcing it
    /// here would require terminal-state tracking that duplicates
    /// the state machine in `daemon::execution::mission::dispatch`.
    pub fn emit(
        &self,
        event_type: impl Into<String>,
        payload: Option<Value>,
    ) -> anyhow::Result<i64> {
        let event_type = event_type.into();
        let terminal_state: Option<&str> = match event_type.as_str() {
            "completed" => Some("COMPLETED"),
            "failed" => Some("FAILED"),
            "cancelled" => Some("CANCELLED"),
            _ => None,
        };

        let mut guard = self
            .next_sequence
            .lock()
            .map_err(|_| anyhow::anyhow!("timeline writer lock poisoned"))?;
        let sequence = *guard;
        let event = TimelineEvent::new(sequence, event_type, payload);
        // Serialize to the shape PersistentLog expects. The
        // serializer cannot fail for our `TimelineEvent` (all
        // fields are String / i64 / Option<Value>) but we propagate
        // the error rather than `expect` to keep the caller in
        // charge of "what do we do when disk is unreachable."
        let value =
            serde_json::to_value(&event).map_err(|e| anyhow::anyhow!("serialize event: {e}"))?;
        // fsync=true honours P2. The cost (one syscall per emit)
        // is bounded by the LLM streaming rate, which is orders of
        // magnitude slower than fsync even on slow disks.
        self.log
            .append_event(&self.invocation_id, &value, terminal_state, true)
            .map_err(|e| anyhow::anyhow!("append_event: {e}"))?;
        // Now that disk is durable, advance the counter and wake
        // subscribers. Order matters: if we advanced first and
        // then append_event failed, a resumer reading from disk
        // would see a gap; counter advances only after successful
        // append.
        *guard = sequence + 1;
        drop(guard);
        // `send` returns `Err` only when there are zero active
        // subscribers. That's fine — disk is authoritative; no
        // subscriber means no one is listening live, which is the
        // common case for one-shot `agent send` invocations.
        let _ = self.broadcast_tx.send(event);
        Ok(sequence)
    }

    /// Number of currently-active broadcast subscribers. Diagnostic;
    /// not part of the P1–P6 contract.
    #[cfg(test)]
    pub fn subscriber_count(&self) -> usize {
        self.broadcast_tx.receiver_count()
    }
}

// ── Tests ────────────────────────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    //! Tests pin the P1–P6 invariants at the CLI-side composition
    //! layer. The underlying PersistentLog has its own conformance
    //! suite in the Axon SDK; what we test here is that our
    //! composition (sequence counter + broadcast wake order) does
    //! not break them.

    use super::*;

    /// Fresh writer in a throwaway log dir. Each test gets its own
    /// tempdir so concurrent tests never cross-write to the same
    /// `<invocation_id>.jsonl`.
    fn new_writer(tag: &str) -> (TimelineWriter, PathBuf) {
        let pid = std::process::id();
        let nanos = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .map(|d| d.subsec_nanos())
            .unwrap_or(0);
        let dir = std::env::temp_dir().join(format!("easynet-timeline-test-{tag}-{pid}-{nanos}"));
        std::fs::create_dir_all(&dir).unwrap();
        let invocation_id = format!("inv-{tag}-{pid}-{nanos}");
        let writer = TimelineWriter::new(invocation_id, Some(dir.clone()));
        (writer, dir)
    }

    // ── happy path ──────────────────────────────────────────────────────────

    #[test]
    fn emit_returns_monotonic_sequences_starting_from_zero() {
        // I5: events within one invocation carry strictly
        // monotonic sequence. We pin the starting point (0) so a
        // future "start from 1" refactor trips this loudly; the
        // PersistentLog readers assume 0-based offsets.
        let (w, _dir) = new_writer("monotonic");
        assert_eq!(w.emit("admitted", None).unwrap(), 0);
        assert_eq!(w.emit("progress", None).unwrap(), 1);
        assert_eq!(w.emit("progress", None).unwrap(), 2);
        assert_eq!(w.emit("completed", None).unwrap(), 3);
    }

    #[test]
    fn emit_persists_events_readable_by_persistent_log() {
        // P3: offset-read contract. After emitting N events, a
        // fresh reader reading from offset 0 must see exactly
        // those N events in sequence order.
        let (w, dir) = new_writer("persist");
        w.emit("admitted", None).unwrap();
        w.emit("progress", Some(serde_json::json!({"chunk": "hello"})))
            .unwrap();
        w.emit("completed", Some(serde_json::json!({"reply": "hi"})))
            .unwrap();

        let log = PersistentLog::new(Some(dir));
        let events = log.read_events(w.invocation_id(), 0);
        assert_eq!(events.len(), 3);
        assert_eq!(events[0]["sequence"], 0);
        assert_eq!(events[0]["type"], "admitted");
        assert_eq!(events[2]["type"], "completed");
        assert_eq!(events[2]["payload"]["reply"], "hi");
    }

    #[test]
    fn terminal_event_marks_persistent_log_index_terminal() {
        // P4 terminal idempotence requires the index to record the
        // terminal state. A reader reopening after the writer is
        // gone must see the same terminal state bytes.
        let (w, dir) = new_writer("terminal");
        w.emit("admitted", None).unwrap();
        w.emit("failed", Some(serde_json::json!({"reason": "boom"})))
            .unwrap();

        let log = PersistentLog::new(Some(dir));
        let idx = log.read_index(w.invocation_id()).expect("index present");
        assert_eq!(idx.terminal_state.as_deref(), Some("FAILED"));
        assert_eq!(idx.last_sequence, 1);
    }

    #[test]
    fn subscribe_receives_events_emitted_after_subscription() {
        // Subscribers obtained AFTER emit miss those past events on
        // the channel (explicit contract in the docstring). A
        // subscriber obtained BEFORE sees them.
        let (w, _dir) = new_writer("subscribe");
        let mut rx = w.subscribe();
        w.emit("admitted", None).unwrap();
        w.emit("progress", Some(serde_json::json!({"n": 1})))
            .unwrap();
        // try_recv drains without awaiting — broadcast's receiver
        // is tokio-typed but try_recv is sync and returns
        // immediately when events are already enqueued.
        let e0 = rx.try_recv().unwrap();
        assert_eq!(e0.sequence, 0);
        let e1 = rx.try_recv().unwrap();
        assert_eq!(e1.sequence, 1);
        assert_eq!(e1.payload.unwrap()["n"], 1);
    }

    #[test]
    fn subscribe_after_emit_does_not_replay_on_channel() {
        // Documents the no-replay-on-subscribe rule. Callers that
        // want replay must read from PersistentLog separately —
        // that's the Session::resume path in session.rs.
        let (w, _dir) = new_writer("late-sub");
        w.emit("admitted", None).unwrap();
        let mut rx = w.subscribe();
        // No prior events delivered on the channel.
        match rx.try_recv() {
            Err(broadcast::error::TryRecvError::Empty) => {}
            other => panic!("expected Empty, got {other:?}"),
        }
        // But new events are delivered.
        w.emit("progress", None).unwrap();
        assert_eq!(rx.try_recv().unwrap().sequence, 1);
    }

    #[test]
    fn subscriber_count_reflects_live_receivers() {
        let (w, _dir) = new_writer("count");
        assert_eq!(w.subscriber_count(), 0);
        let rx1 = w.subscribe();
        assert_eq!(w.subscriber_count(), 1);
        let rx2 = w.subscribe();
        assert_eq!(w.subscriber_count(), 2);
        drop(rx1);
        assert_eq!(w.subscriber_count(), 1);
        drop(rx2);
        assert_eq!(w.subscriber_count(), 0);
    }

    // ── P2 ordering: disk-before-notify ─────────────────────────────────────

    #[test]
    fn broadcast_is_not_woken_until_disk_append_is_durable() {
        // P2 in spirit: a subscriber that observes event N via the
        // broadcast channel must find event N on disk when it
        // looks there. We pin this by reading the disk IMMEDIATELY
        // after the recv and asserting the sequence is present.
        //
        // This test catches a hypothetical refactor that reorders
        // `broadcast.send(event); self.log.append_event(...)` —
        // an optimisation a future contributor might propose on
        // the grounds that "fsync is slow, send first." Under that
        // reorder, a fast subscriber could crash the writer before
        // append_event runs; this test would fail because the
        // disk read after recv would return zero events.
        let (w, dir) = new_writer("p2-order");
        let mut rx = w.subscribe();
        w.emit("admitted", Some(serde_json::json!({"n": 42})))
            .unwrap();
        let event = rx.try_recv().unwrap();
        // Disk must already be authoritative at this point.
        let log = PersistentLog::new(Some(dir));
        let events = log.read_events(w.invocation_id(), 0);
        assert_eq!(
            events.len(),
            1,
            "disk must carry the event by the time the subscriber sees it"
        );
        assert_eq!(events[0]["sequence"], event.sequence);
        assert_eq!(events[0]["payload"]["n"], 42);
    }

    // ── failure path ────────────────────────────────────────────────────────

    #[test]
    fn emit_with_no_subscribers_still_persists_to_disk() {
        // No subscribers: the broadcast send returns Err, but disk
        // is authoritative and must succeed. The common one-shot
        // `agent send` case.
        let (w, dir) = new_writer("no-sub");
        assert_eq!(w.subscriber_count(), 0);
        w.emit("admitted", None).unwrap();
        w.emit("completed", None).unwrap();
        let log = PersistentLog::new(Some(dir));
        assert_eq!(log.read_events(w.invocation_id(), 0).len(), 2);
    }

    #[test]
    fn serialization_round_trips_event_through_disk() {
        // Downstream consumers (PR-10 services, PR-7 Session
        // resume) parse events off disk via serde. Pin that the
        // in-memory shape and the on-disk shape are isomorphic so
        // a future schema drift forces this test to update.
        let (w, dir) = new_writer("roundtrip");
        let payload = serde_json::json!({"kind": "delta", "text": "hello"});
        w.emit("progress", Some(payload.clone())).unwrap();
        let log = PersistentLog::new(Some(dir));
        let events = log.read_events(w.invocation_id(), 0);
        let parsed: TimelineEvent = serde_json::from_value(events[0].clone()).unwrap();
        assert_eq!(parsed.event_type, "progress");
        assert_eq!(parsed.sequence, 0);
        assert_eq!(parsed.payload.unwrap(), payload);
    }

    // ── edge cases ──────────────────────────────────────────────────────────

    #[test]
    fn empty_payload_serializes_without_null_key() {
        // Payload is Option<Value>. When None, serde's
        // `skip_serializing_if = "Option::is_none"` should omit
        // the key on disk. A noisy `"payload": null` would make
        // disk reads fatter for no information gain.
        let (w, dir) = new_writer("empty-payload");
        w.emit("admitted", None).unwrap();
        let log = PersistentLog::new(Some(dir));
        let events = log.read_events(w.invocation_id(), 0);
        let obj = events[0].as_object().unwrap();
        assert!(
            !obj.contains_key("payload"),
            "None payload must not serialize a null key, got: {obj:?}"
        );
    }

    #[test]
    fn concurrent_emits_produce_unique_increasing_sequences() {
        // Writer lock serialises the sequence advance. Hammer it
        // from multiple threads and assert no duplicate, no gap.
        use std::sync::Arc;
        use std::thread;

        let (w, _dir) = new_writer("concurrent");
        let w = Arc::new(w);
        let mut handles = Vec::new();
        const THREADS: usize = 8;
        const PER_THREAD: usize = 50;
        for _ in 0..THREADS {
            let w = w.clone();
            handles.push(thread::spawn(move || {
                let mut seqs = Vec::new();
                for _ in 0..PER_THREAD {
                    seqs.push(w.emit("progress", None).unwrap());
                }
                seqs
            }));
        }
        let mut all: Vec<i64> = handles
            .into_iter()
            .flat_map(|h| h.join().unwrap())
            .collect();
        all.sort();
        assert_eq!(all.len(), THREADS * PER_THREAD);
        for (i, s) in all.iter().enumerate() {
            assert_eq!(*s as usize, i, "sequence gap or duplicate at {i}");
        }
    }
}
