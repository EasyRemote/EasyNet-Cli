// EasyNet CLI — system.session.{list,attach} handlers (PR-ATTACH)
// =================================================================
//
// File: src/runtime/system/session_ability.rs
// Description: The two device-level abilities a Client uses to
//              discover and observe agent runs:
//
//   * `system.session.list`   (RPC)    — return every session known
//                                        to this daemon (active +
//                                        recently terminated).
//   * `system.session.attach` (Stream) — stream TimelineEvent frames
//                                        from one session, optionally
//                                        replaying from a `since_seq`
//                                        offset before tailing live.
//
// Streaming surface
// -----------------
// PR-SYS shipped only the RPC dispatch path. PR-ATTACH introduces
// the v1 streaming surface as a `LocalStreamHandler` registry on
// the dispatcher. Each subscribe-mode invocation produces a
// TimelineEvent stream that the IPC server forwards over its
// frame-of-frames protocol.
//
// Resume + tail composition
// -------------------------
// The "replay then subscribe" pattern is already in
// `runtime::session::Session::resume_replay` + `subscribe`. This
// handler composes them: read the on-disk prefix from `since_seq`
// (P3 contract), then attach a broadcast subscriber for live
// tailing. Boundary handoff is sequence-numbered so a Client sees
// every event exactly once with no gap and no duplicate.
//
// Author: Silan Hu <silan.hu@u.nus.edu>
// Copyright (c) 2026 EasyNet. All rights reserved.

use std::sync::Arc;

use serde_json::{json, Value};

use crate::runtime::ability_dispatch::{LocalAbilityRegistry, StreamSource};
use crate::runtime::domain::SessionId;
use crate::runtime::execution::session::SessionService;

pub const ABILITY_LIST: &str = "fleet.list_sessions";
pub const ABILITY_ATTACH: &str = "fleet.attach_session";

/// Register the two session abilities on the registry. Called from
/// `runtime::agents::build_registry`.
///
/// `attach` is a Stream-mode ability — its handler is a stream
/// producer rather than a single response. v1 ships the RPC handler
/// for `list` and a stream handler for `attach`; the dispatcher
/// routes by call_mode.
pub fn register(reg: &mut LocalAbilityRegistry, sessions: Arc<SessionService>) {
    let s_for_list = Arc::clone(&sessions);
    reg.register_rpc(
        ABILITY_LIST,
        Arc::new(move |args: Value| list_handler(&s_for_list, args)),
    );
    // attach is registered as a stream handler — see
    // runtime::ability_dispatch for the LocalStreamRegistry surface.
    reg.register_stream(
        ABILITY_ATTACH,
        Arc::new(move |args: Value| attach_handler(&sessions, args)),
    );
}

/// `system.session.list` RPC handler.
///
/// Args: `{ "include_terminated": bool? = true }`
/// Returns: `{ "sessions": [Session, ...] }` where each Session
/// matches `runtime::domain::Session`.
fn list_handler(svc: &SessionService, args: Value) -> anyhow::Result<Value> {
    let include_terminated = args
        .get("include_terminated")
        .and_then(Value::as_bool)
        .unwrap_or(true);
    let sessions = svc.list_active();
    let filtered: Vec<&_> = sessions
        .iter()
        .filter(|s| include_terminated || s.ended_unix_ms.is_none())
        .collect();
    let json_sessions: Vec<Value> = filtered
        .iter()
        .map(|s| serde_json::to_value(s).unwrap_or(Value::Null))
        .collect();
    Ok(json!({ "sessions": json_sessions }))
}

/// `system.session.attach` stream handler.
///
/// Args: `{ "session_id": string, "since_seq": int? = 0 }`
/// Returns a SnapshotThenLive stream:
///   * snapshot half = `history[since_seq..]` from SessionService
///   * live half     = a fresh broadcast::Receiver tailing every
///                     event emitted via `SessionService::emit_event`
///                     after the call point.
///
/// When the session_id is unknown (no admission has fired yet) the
/// handler returns an empty Snapshot. The Client interprets this
/// as "session not found / not yet" and may retry; emitting an
/// error frame instead would force every stale-id case to surface
/// as a hard fault, which is too coarse for the timeline view.
fn attach_handler(svc: &SessionService, args: Value) -> anyhow::Result<StreamSource> {
    let session_id = args
        .get("session_id")
        .and_then(Value::as_str)
        .ok_or_else(|| anyhow::anyhow!("fleet.attach_session: `session_id` required"))?
        .to_string();
    let since_seq = args
        .get("since_seq")
        .and_then(Value::as_i64)
        .unwrap_or(0)
        .max(0) as usize;

    let id = SessionId::new(&session_id);
    if svc.get(&id).is_none() {
        return Ok(StreamSource::Snapshot(Vec::new()));
    }
    let (snapshot, rx) = svc.subscribe_session(&id, since_seq)?;
    Ok(StreamSource::SnapshotThenLive(snapshot, rx))
}

/// Discovery JSON for `system.session.list`. Mirrors the shape
/// used inside `a2a.system_skills_json`.
pub fn list_input_schema() -> Value {
    json!({
        "type": "object",
        "properties": {
            "include_terminated": {"type": "boolean"}
        },
        "additionalProperties": false,
    })
}

/// Discovery JSON for `system.session.attach`.
pub fn attach_input_schema() -> Value {
    json!({
        "type": "object",
        "properties": {
            "session_id": {"type": "string"},
            "since_seq": {"type": "integer", "minimum": 0}
        },
        "required": ["session_id"],
        "additionalProperties": false,
    })
}

pub fn list_description() -> &'static str {
    "List sessions known to this daemon. Default returns active and recently-terminated runs; \
     pass `include_terminated: false` to filter to active only."
}

pub fn attach_description() -> &'static str {
    "Subscribe to a session's TimelineEvent stream. Replays from `since_seq` (default 0) on disk, \
     then tails live events. Terminates when the session reaches a terminal state."
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::runtime::domain::{AgentId, NodeId, TenantId};

    fn svc_with(ids: &[&str]) -> Arc<SessionService> {
        let svc = Arc::new(SessionService::new());
        for id in ids {
            svc.admit(SessionService::make_session(
                SessionId::new(*id),
                AgentId::new("a"),
                NodeId::new("self"),
                TenantId::default_v1(),
            ))
            .unwrap();
        }
        svc
    }

    #[test]
    fn list_returns_every_admitted_session() {
        let svc = svc_with(&["a", "b"]);
        let resp = list_handler(&svc, json!({})).unwrap();
        let arr = resp["sessions"].as_array().unwrap();
        assert_eq!(arr.len(), 2);
    }

    #[test]
    fn list_include_terminated_false_filters_ended_sessions() {
        let svc = svc_with(&["live", "done"]);
        svc.terminate(&SessionId::new("done"), 1_000_000).unwrap();
        let resp = list_handler(&svc, json!({"include_terminated": false})).unwrap();
        let arr = resp["sessions"].as_array().unwrap();
        assert_eq!(arr.len(), 1);
        assert_eq!(arr[0]["id"], "live");
    }

    #[test]
    fn attach_unknown_session_returns_empty_stream() {
        // v1 contract: unknown session_id → empty stream (not
        // an error).
        let svc = svc_with(&[]);
        let frames = attach_handler(&svc, json!({"session_id": "nope"}))
            .unwrap()
            .into_snapshot();
        assert!(frames.is_empty());
    }

    #[tokio::test]
    async fn attach_known_session_returns_history_then_live_tail() {
        // The "看得见" contract: a Client attaching after a
        // session was admitted and progressed sees the history
        // (admitted + emitted progress) AND new emitted events
        // arriving after attach.
        let svc = svc_with(&["live-1"]);
        svc.emit_event(
            &SessionId::new("live-1"),
            json!({"kind": "progress", "n": 1}),
        )
        .unwrap();

        let stream = attach_handler(&svc, json!({"session_id": "live-1"})).unwrap();
        let (snap, mut rx) = match stream {
            crate::runtime::ability_dispatch::StreamSource::SnapshotThenLive(s, r) => (s, r),
            other => panic!("expected SnapshotThenLive, got {other:?}"),
        };
        assert_eq!(snap.len(), 2); // admitted + first progress
        assert_eq!(snap[0]["kind"], "admitted");
        assert_eq!(snap[1]["kind"], "progress");

        // After attach, emit a second progress; live tail receives.
        svc.emit_event(
            &SessionId::new("live-1"),
            json!({"kind": "progress", "n": 2}),
        )
        .unwrap();
        let live = rx.recv().await.expect("live frame");
        assert_eq!(live["n"], 2);
    }

    #[test]
    fn attach_missing_session_id_errors() {
        let svc = svc_with(&[]);
        let err = attach_handler(&svc, json!({})).unwrap_err();
        assert!(format!("{err}").contains("session_id"));
    }
}
