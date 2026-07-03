// EasyNet CLI — Agent Session
// ============================
//
// File: src/daemon/execution/mission/session.rs
// Description: A `Session` is the handle an in-process caller uses
//              to drive one agent invocation on the CLI: allocate
//              an id, own a `TimelineWriter`, and offer two thin
//              APIs for live subscription (`subscribe`, real-time
//              events) and past-event replay (`resume_replay`,
//              read the on-disk prefix).
//
// Why this module exists separately from `TimelineWriter`
// -------------------------------------------------------
// `TimelineWriter` is the lower layer: it knows about events,
// sequences, and the disk-plus-broadcast composition. `Session`
// adds the identity + resume concepts on top. Keeping them in
// separate files mirrors the division in
// `INVOCATION_LIFECYCLE_ACROSS_PROCESSES.md`: the timeline is the
// event log (P1–P6), the session is the in-process handle that
// binds that log to a specific invocation.
//
// What this module does NOT provide
// ---------------------------------
// - Remote observation. A process that did not create the Session
//   can still observe the same invocation by opening the
//   PersistentLog directly via its `invocation_id`. That cross-
//   process story is exactly the P1–P6 contract and does not
//   need a `Session` handle.
// - AXIOM first-class Invocation semantics (signed envelopes,
//   receipt chain). Tracked in
//   `docs/open-questions/cli-dispatch-as-first-class-invocation.md`;
//   explicitly out of PR-7 scope.
//
// Author: Silan Hu <silan.hu@u.nus.edu>
// Copyright (c) 2026 EasyNet. All rights reserved.

use std::path::PathBuf;
use std::sync::Arc;

#[cfg(test)]
use easynet_axon::invocation::persistence::PersistentLog;
#[cfg(test)]
use tokio::sync::broadcast;
use uuid::Uuid;

#[cfg(test)]
use crate::daemon::execution::mission::timeline::TimelineEvent;
use crate::daemon::execution::mission::timeline::TimelineWriter;

/// Allocate a fresh `invocation_id` for a CLI-local dispatch.
/// UUID-v4 per AXIOM P1 (ids are opaque to ordering — sequence
/// inside the log carries ordering; ids only identify the log
/// itself).
///
/// Prefixed with `cli-` so an auditor walking a mixed log
/// directory can distinguish CLI-allocated ids from Axon-
/// allocated ones at a glance. The prefix is not load-bearing
/// for any parser; it's a human-readable hint.
pub fn fresh_invocation_id() -> String {
    format!("cli-{}", Uuid::new_v4())
}

/// In-process session handle for one agent invocation.
///
/// Cheap to clone (Arc<TimelineWriter>). Callers that fan the
/// session out to multiple subscribers (one emit path + N
/// subscribers) clone the Session and hand each subscriber their
/// own `subscribe()` receiver.
#[derive(Clone)]
pub struct Session {
    writer: Arc<TimelineWriter>,
}

impl Session {
    /// Open a fresh session. `log_dir` defaults to
    /// `PersistentLog`'s env-var / tempdir policy when None.
    ///
    /// Does not emit any event by itself. The caller typically
    /// emits an `admitted` event immediately after construction
    /// with the invocation's caller/callee/prompt as the payload.
    pub fn new(log_dir: Option<PathBuf>) -> Self {
        let invocation_id = fresh_invocation_id();
        Self {
            writer: Arc::new(TimelineWriter::new(invocation_id, log_dir)),
        }
    }

    /// Open a session reusing a known `invocation_id`. Used when
    /// the id was already allocated upstream (e.g. stamped into
    /// `meta.json` before the session handle was constructed) and
    /// needs to be honoured rather than regenerated.
    #[cfg(test)]
    pub fn with_id(invocation_id: impl Into<String>, log_dir: Option<PathBuf>) -> Self {
        Self {
            writer: Arc::new(TimelineWriter::new(invocation_id, log_dir)),
        }
    }

    pub fn invocation_id(&self) -> &str {
        self.writer.invocation_id()
    }

    /// Access the underlying writer for emit operations.
    ///
    /// Exposed directly rather than wrapping every `emit` variant
    /// because the Session's identity does not mutate the event
    /// payload — a Session-scoped `emit` would be a trivial
    /// delegate. Callers that want the same `emit` signature use
    /// `session.writer().emit(...)`.
    pub fn writer(&self) -> &TimelineWriter {
        &self.writer
    }

    /// `Arc` clone of the writer. Used when passing the writer
    /// into an opaque consumer (e.g. `adapter::InvokeOpts::timeline`)
    /// that needs a `'static` handle for a background thread or
    /// closure capture. The Arc shares the same sequence counter
    /// and broadcast channel as `.writer()` — emits from either
    /// handle contribute to the same timeline.
    pub fn writer_arc(&self) -> Arc<TimelineWriter> {
        Arc::clone(&self.writer)
    }

    /// Subscribe to live events from this invocation.
    ///
    /// Composes with `resume_replay` for the "replay + tail"
    /// pattern: a client that joined late calls
    /// `resume_replay(from_offset)` to read the disk prefix, then
    /// `subscribe()` to tail new events. The boundary between the
    /// two is the sequence the client last saw on disk; events at
    /// that sequence or higher arrive on the subscription.
    #[cfg(test)]
    pub fn subscribe(&self) -> broadcast::Receiver<TimelineEvent> {
        self.writer.subscribe()
    }

    /// Read past events from disk starting at `from_offset`.
    ///
    /// Thin wrapper over `PersistentLog::read_events` that
    /// deserialises back into `TimelineEvent`. Returns only
    /// events with `sequence >= from_offset` (P3 contract);
    /// empty vec when the log is missing or the offset is beyond
    /// the current tail.
    ///
    /// Deserialisation errors on a single event are silently
    /// skipped — a corrupted line (impossible under P6 crash
    /// consistency, but possible if an operator hand-edits the
    /// file) should not mask the readable prefix. The counter
    /// of skipped events is exposed as the return value's
    /// `skipped` field when that visibility matters; this path
    /// returns only the parsed events because resume consumers
    /// care about events, not about operator mistakes.
    #[cfg(test)]
    pub fn resume_replay(&self, from_offset: i64) -> Vec<TimelineEvent> {
        let log = PersistentLog::new(Some(
            self.writer
                .log_path()
                .parent()
                .map(|p| p.to_path_buf())
                .unwrap_or_else(std::env::temp_dir),
        ));
        log.read_events(self.writer.invocation_id(), from_offset)
            .into_iter()
            .filter_map(|v| serde_json::from_value(v).ok())
            .collect()
    }
}

// ── Tests ────────────────────────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    //! Session tests exercise the compose: identity allocation,
    //! resume_replay + subscribe continuity, cross-process open
    //! by id. The underlying TimelineWriter tests pin the P1–P6
    //! invariants at the layer below.

    use super::*;

    fn new_session(tag: &str) -> (Session, PathBuf) {
        let pid = std::process::id();
        let nanos = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .map(|d| d.subsec_nanos())
            .unwrap_or(0);
        let dir = std::env::temp_dir().join(format!("easynet-session-test-{tag}-{pid}-{nanos}"));
        std::fs::create_dir_all(&dir).unwrap();
        let s = Session::new(Some(dir.clone()));
        (s, dir)
    }

    // ── happy path ──────────────────────────────────────────────────────────

    #[test]
    fn fresh_invocation_id_is_unique_per_call() {
        // Two successive calls must produce distinct ids — uuid
        // v4 has collision probability << the birthday bound for
        // CLI dispatch rates, but we pin non-equality here so a
        // future "let me switch to a monotonic counter" refactor
        // that inadvertently duplicates trips the test.
        let a = fresh_invocation_id();
        let b = fresh_invocation_id();
        assert_ne!(a, b);
        assert!(a.starts_with("cli-"));
    }

    #[test]
    fn resume_replay_returns_all_events_from_offset_zero() {
        // P3: a reader reading from offset 0 sees every emitted
        // event in sequence order.
        let (s, _dir) = new_session("replay");
        s.writer().emit("admitted", None).unwrap();
        s.writer()
            .emit("progress", Some(serde_json::json!({"n": 1})))
            .unwrap();
        s.writer().emit("completed", None).unwrap();

        let events = s.resume_replay(0);
        assert_eq!(events.len(), 3);
        assert_eq!(events[0].event_type, "admitted");
        assert_eq!(events[1].payload.as_ref().unwrap()["n"], 1);
        assert_eq!(events[2].event_type, "completed");
    }

    #[test]
    fn resume_replay_honours_from_offset() {
        // P3: the offset contract. A resumer that last saw
        // sequence N calls `resume_replay(N + 1)` and receives
        // events with sequence >= N + 1 only.
        let (s, _dir) = new_session("offset");
        for _ in 0..5 {
            s.writer().emit("progress", None).unwrap();
        }
        let tail = s.resume_replay(3);
        assert_eq!(tail.len(), 2);
        assert_eq!(tail[0].sequence, 3);
        assert_eq!(tail[1].sequence, 4);
    }

    #[test]
    fn replay_then_subscribe_gives_continuous_stream() {
        // The composition pattern: resume the on-disk prefix, then
        // subscribe for live tail. A subscriber that executes
        // these two steps sees every event in sequence order with
        // no gap and no duplicate AT THE BOUNDARY.
        //
        // The test is single-threaded: emit 2, replay, subscribe,
        // emit 1 more. The boundary is sequence 2: the replay
        // returns 0..=1, subscribe delivers 2.
        let (s, _dir) = new_session("continuity");
        s.writer().emit("admitted", None).unwrap();
        s.writer().emit("progress", None).unwrap();
        let prefix = s.resume_replay(0);
        let mut rx = s.subscribe();
        s.writer().emit("completed", None).unwrap();
        assert_eq!(prefix.len(), 2);
        assert_eq!(prefix[1].sequence, 1);
        let live = rx.try_recv().unwrap();
        assert_eq!(live.sequence, 2, "subscriber picks up at prefix_end + 1");
    }

    #[test]
    fn with_id_reuses_the_provided_invocation_id() {
        let (_s, dir) = new_session("trash");
        let s = Session::with_id("preset-id", Some(dir.clone()));
        assert_eq!(s.invocation_id(), "preset-id");
        s.writer().emit("admitted", None).unwrap();
        // Log file is named by the invocation_id.
        assert!(dir.join("preset-id.jsonl").exists());
    }

    #[test]
    fn session_clone_shares_the_writer() {
        // Clone semantics: two Session handles pointing at the
        // same writer share sequence counter + broadcast channel
        // + disk log. This is how loop execution can hold a
        // Session and fan it out to N subscribers without
        // re-opening the log.
        let (s1, _dir) = new_session("clone");
        let s2 = s1.clone();
        assert_eq!(s1.invocation_id(), s2.invocation_id());
        let mut rx = s2.subscribe();
        s1.writer().emit("admitted", None).unwrap();
        assert_eq!(rx.try_recv().unwrap().sequence, 0);
    }

    // ── failure path ────────────────────────────────────────────────────────

    #[test]
    fn resume_replay_on_unknown_id_returns_empty() {
        // A session never emitted to. No disk log exists; replay
        // degrades to an empty vec rather than an IO error.
        let (_s, dir) = new_session("empty");
        let unknown = Session::with_id("never-emitted", Some(dir));
        assert!(unknown.resume_replay(0).is_empty());
    }

    #[test]
    fn resume_replay_from_offset_past_tail_returns_empty() {
        let (s, _dir) = new_session("past-tail");
        s.writer().emit("admitted", None).unwrap();
        // Tail is at sequence 0; ask for 100.
        assert!(s.resume_replay(100).is_empty());
    }

    // ── edge cases ──────────────────────────────────────────────────────────

    #[test]
    fn cross_process_open_sees_writer_commits() {
        // The spirit of P1–P6: a process that did not create the
        // invocation can still observe it by `invocation_id`. We
        // simulate by opening a PersistentLog on the same dir
        // with the same id and asserting parity with the
        // Session's own replay.
        //
        // This is the test that catches "I silently changed the
        // log filename convention" refactors — any divergence
        // between the Session's view and the bare-PersistentLog
        // view shows up here.
        let (s, dir) = new_session("cross-proc");
        for n in 0..5 {
            s.writer()
                .emit("progress", Some(serde_json::json!({"n": n})))
                .unwrap();
        }
        let bare = PersistentLog::new(Some(dir));
        let events = bare.read_events(s.invocation_id(), 0);
        assert_eq!(events.len(), 5);
        for (i, e) in events.iter().enumerate() {
            assert_eq!(e["sequence"], i as i64);
            assert_eq!(e["payload"]["n"], i as i64);
        }
    }
}
