// EasyNet CLI — session.{list,attach} handlers
// =================================================================
//
// File: src/daemon/ability/builtins/device_control/session.rs
// Description: The two device-level abilities a Client uses to
//              discover and observe agent runs:
//
//   * `session.list`   (RPC)    — return every session known
//                                        to this daemon (active +
//                                        recently terminated).
//   * `session.attach` (Stream) — stream TimelineEvent frames
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
// `daemon::execution::mission::session::Session::resume_replay` + `subscribe`. This
// handler composes them: read the on-disk prefix from `since_seq`
// (P3 contract), then attach a broadcast subscriber for live
// tailing. Boundary handoff is sequence-numbered so a Client sees
// every event exactly once with no gap and no duplicate.
//
// Author: Silan Hu <silan.hu@u.nus.edu>
// Copyright (c) 2026 EasyNet. All rights reserved.

use std::sync::Arc;

use serde_json::{json, Map, Value};

use crate::core::domain::SessionId;
use crate::daemon::ability::dispatch::OwnerKind;
use crate::daemon::ability::dispatch::{AxonAbilityCatalog, StreamSource};
use crate::daemon::execution::session::SessionService;

pub const ABILITY_LIST: &str = crate::daemon::ability::names::device_control::SESSION_LIST;
pub const ABILITY_ATTACH: &str = crate::daemon::ability::names::device_control::SESSION_ATTACH;

/// Register the two session abilities on the registry. Called from
/// `daemon::ability::catalog::build_registry`.
///
/// `attach` is a Stream-mode ability — its handler is a stream
/// producer rather than a single response. v1 ships the RPC handler
/// for `list` and a stream handler for `attach`; the dispatcher
/// routes by call_mode.
pub fn register(reg: &mut AxonAbilityCatalog, sessions: Arc<SessionService>) {
    let s_for_list = Arc::clone(&sessions);
    reg.register_rpc_with_owner(
        ABILITY_LIST,
        OwnerKind::Device,
        Arc::new(move |args: Value| list_handler(&s_for_list, args)),
    );
    // attach is registered as a stream handler — see
    // daemon::ability::dispatch for the LocalStreamRegistry surface.
    reg.register_stream_with_owner(
        "session.attach",
        OwnerKind::Device,
        Arc::new(move |args: Value| attach_handler(&sessions, args)),
    );
}

/// `session.list` RPC handler.
///
/// Args: `{ "include_terminated": bool? = true }`
/// Returns: `{ "sessions": [Session, ...] }` where each Session
/// matches `core::domain::Session`.
fn list_handler(svc: &SessionService, args: Value) -> anyhow::Result<Value> {
    let args = session_args_object("session.list", &args, &["include_terminated"])?;
    let include_terminated =
        session_optional_bool_arg("session.list", args, "include_terminated")?.unwrap_or(true);
    let sessions = svc.list_active()?;
    let filtered: Vec<&_> = sessions
        .iter()
        .filter(|s| include_terminated || s.ended_unix_ms.is_none())
        .collect();
    let json_sessions: Vec<Value> = filtered
        .iter()
        .map(|session| runtime_admin_session_projection(session))
        .collect();
    Ok(json!({ "sessions": json_sessions }))
}

fn runtime_admin_session_projection(session: &crate::core::domain::Session) -> Value {
    let realm = session.tenant.as_str();
    let runtime_host_ura = crate::core::ura::device_ura(realm, session.node.as_str());
    let control_authority_ura = crate::core::ura::authority_ura(realm);
    let state = if session.ended_unix_ms.is_some() {
        "terminated"
    } else {
        "active"
    };
    json!({
        "kind": "runtime_session",
        "session_id": session.id.as_str(),
        "runtime_host_ura": runtime_host_ura,
        "control_authority_ura": control_authority_ura,
        "state": state,
        "session_kind": "agent",
        "created_unix_ms": session.started_unix_ms,
        "expires_unix_ms": session.ended_unix_ms.unwrap_or(0),
        "metadata": {
            "agent_id": session.agent.as_str(),
            "tenant": realm,
        },
    })
}

/// `session.attach` stream handler.
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
    let args = session_args_object("session.attach", &args, &["session_id", "since_seq"])?;
    let session_id = session_required_string_arg("session.attach", args, "session_id")?;
    let since_seq = session_optional_usize_arg("session.attach", args, "since_seq")?.unwrap_or(0);

    let id = SessionId::new(&session_id);
    if svc.get(&id)?.is_none() {
        return Ok(StreamSource::Snapshot(Vec::new()));
    }
    let (snapshot, rx) = svc.subscribe_session(&id, since_seq)?;
    Ok(StreamSource::SnapshotThenLive(snapshot, rx))
}

fn session_args_object<'a>(
    ability: &str,
    args: &'a Value,
    allowed_fields: &[&str],
) -> anyhow::Result<&'a Map<String, Value>> {
    let object = args
        .as_object()
        .ok_or_else(|| anyhow::anyhow!("{ability}: args must be a JSON object"))?;
    let mut unknown = object
        .keys()
        .filter(|key| !allowed_fields.contains(&key.as_str()))
        .map(String::as_str)
        .collect::<Vec<_>>();
    unknown.sort_unstable();
    if !unknown.is_empty() {
        anyhow::bail!("{ability}: unsupported field(s): {}", unknown.join(", "));
    }
    Ok(object)
}

fn session_required_string_arg(
    ability: &str,
    args: &Map<String, Value>,
    field: &str,
) -> anyhow::Result<String> {
    let value = args
        .get(field)
        .and_then(Value::as_str)
        .map(str::trim)
        .ok_or_else(|| anyhow::anyhow!("{ability}: `{field}` required"))?;
    if value.is_empty() {
        anyhow::bail!("{ability}: {field} must be non-empty");
    }
    Ok(value.to_string())
}

fn session_optional_bool_arg(
    ability: &str,
    args: &Map<String, Value>,
    field: &str,
) -> anyhow::Result<Option<bool>> {
    let Some(value) = args.get(field) else {
        return Ok(None);
    };
    value
        .as_bool()
        .map(Some)
        .ok_or_else(|| anyhow::anyhow!("{ability}: `{field}` must be a boolean"))
}

fn session_optional_usize_arg(
    ability: &str,
    args: &Map<String, Value>,
    field: &str,
) -> anyhow::Result<Option<usize>> {
    let Some(value) = args.get(field) else {
        return Ok(None);
    };
    let number = value
        .as_u64()
        .ok_or_else(|| anyhow::anyhow!("{ability}: `{field}` must be an integer >= 0"))?;
    usize::try_from(number)
        .map(Some)
        .map_err(|_| anyhow::anyhow!("{ability}: `{field}` does not fit in usize"))
}

/// Discovery JSON for `session.list`. Mirrors the shape
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

/// Discovery JSON for `session.attach`.
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
    use crate::core::domain::{AgentId, NodeId, TenantId};

    fn svc_with(ids: &[&str]) -> Arc<SessionService> {
        let svc = Arc::new(SessionService::new());
        svc.bind_memory_for_test(NodeId::new("runtime-node"), TenantId::new("runtime-tenant"));
        for id in ids {
            svc.admit(SessionService::make_session(
                SessionId::new(*id),
                AgentId::new("a"),
                NodeId::new("caller-node"),
                TenantId::new("caller-tenant"),
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
        assert_eq!(arr[0]["session_id"], "live");
        assert_eq!(arr[0]["state"], "active");
    }

    #[test]
    fn list_rejects_non_object_unknown_and_wrong_typed_args_before_service_access() {
        let svc = svc_with(&["live"]);
        svc.poison_index_for_test();

        for (case, args, expected) in [
            ("non-object", json!(null), "JSON object"),
            (
                "unknown field",
                json!({"include_terminated": true, "legacy_mode": true}),
                "unsupported field(s)",
            ),
            (
                "wrong include_terminated type",
                json!({"include_terminated": "false"}),
                "include_terminated",
            ),
        ] {
            let err = list_handler(&svc, args).expect_err(case);
            let message = format!("{err:#}");
            assert!(
                message.contains(expected),
                "{case} should fail at parser boundary, got {message}"
            );
            assert!(
                !message.contains("session index lock poisoned"),
                "{case} must not dispatch into SessionService before parser rejection: {message}"
            );
        }
    }

    #[test]
    fn list_projects_generic_runtime_admin_session_rows() {
        let svc = svc_with(&["live"]);
        let resp = list_handler(&svc, json!({})).unwrap();
        let row = &resp["sessions"].as_array().unwrap()[0];

        assert_eq!(row["kind"], "runtime_session");
        assert_eq!(row["session_id"], "live");
        assert_eq!(
            row["runtime_host_ura"],
            "easynet:///r/runtime-tenant/device/runtime-node"
        );
        assert_eq!(
            row["control_authority_ura"],
            "easynet:///r/runtime-tenant/authority"
        );
        assert!(row.get("device_ura").is_none());
        assert!(row.get("authority_ura").is_none());
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
            crate::daemon::ability::dispatch::StreamSource::SnapshotThenLive(s, r) => (s, r),
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

    #[test]
    fn attach_rejects_non_object_unknown_and_wrong_typed_args_before_service_access() {
        let svc = svc_with(&["live"]);
        svc.poison_index_for_test();

        for (case, args, expected) in [
            ("non-object", json!(false), "JSON object"),
            (
                "unknown field",
                json!({"session_id": "live", "legacy_mode": true}),
                "unsupported field(s)",
            ),
            ("blank session_id", json!({"session_id": "  "}), "non-empty"),
            (
                "wrong since_seq type",
                json!({"session_id": "live", "since_seq": "1"}),
                "since_seq",
            ),
            (
                "negative since_seq",
                json!({"session_id": "live", "since_seq": -1}),
                "integer >= 0",
            ),
        ] {
            let err = attach_handler(&svc, args).expect_err(case);
            let message = format!("{err:#}");
            assert!(
                message.contains(expected),
                "{case} should fail at parser boundary, got {message}"
            );
            assert!(
                !message.contains("session index lock poisoned"),
                "{case} must not dispatch into SessionService before parser rejection: {message}"
            );
        }
    }

    #[test]
    fn list_rejects_poisoned_session_index_instead_of_empty_sessions() {
        let svc = svc_with(&["live"]);
        svc.poison_index_for_test();

        let err = list_handler(&svc, json!({})).expect_err("poisoned index must fail");
        assert!(format!("{err:#}").contains("SessionService session index lock poisoned"));
    }

    #[test]
    fn attach_rejects_poisoned_session_index_instead_of_empty_snapshot() {
        let svc = svc_with(&["live"]);
        svc.poison_index_for_test();

        let err = attach_handler(&svc, json!({"session_id": "live"}))
            .expect_err("poisoned index must fail");
        assert!(format!("{err:#}").contains("SessionService session index lock poisoned"));
    }
}
