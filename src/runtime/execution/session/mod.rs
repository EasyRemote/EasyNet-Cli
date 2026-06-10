// EasyNet CLI — Execution / Session sub-service
// ==============================================
//
// File: src/runtime/execution/session/mod.rs
// Description: Session sub-service. Tracks the live-agent-run
//              registry that `session.list` /
//              `session.attach` abilities query and
//              subscribe to.
//
// What this owns
// --------------
// - `live_sessions`: an in-memory index keyed by SessionId. Each
//   entry holds the metadata (agent / node / tenant / start time /
//   end time) the discovery/attach abilities serve.
// - Insert when a Session is admitted; mark `ended_unix_ms` when
//   it terminates. Entries persist after termination so a late-
//   joining attach can still see "yes, run X completed at T".
//
// Isolation rule: must NOT import from sibling execution sub-
// services. Cross-service talk goes through the Kernel.
//
// PR-INVOCATION-EXEC-UNITY interaction
// ------------------------------------
// Every Session admission will be triggered by `Kernel::invoke`
// admission for an agent-chat-like Invocation; the session
// sub-service is the bookkeeper, not the dispatcher. v1 ships the
// bookkeeping; the wiring from `Kernel::invoke` lands in the
// later commit.
//
// Author: Silan Hu <silan.hu@u.nus.edu>
// Copyright (c) 2026 EasyNet. All rights reserved.

use std::collections::BTreeMap;
use std::sync::RwLock;

use serde_json::Value;
use tokio::sync::broadcast;

use crate::runtime::domain::{AgentId, NodeId, Session, SessionId, TenantId};

/// One indexed session, plus its per-session timeline broadcast.
struct SessionEntry {
    meta: Session,
    /// Past timeline frames in admission order. Late subscribers
    /// receive these as the snapshot half of SnapshotThenLive.
    history: Vec<Value>,
    /// Live broadcast — a fresh receiver gets every frame emitted
    /// AFTER the receiver was created.
    broadcast: broadcast::Sender<Value>,
}

/// Session sub-service handle. Holds a `BTreeMap` for deterministic
/// iteration; `RwLock` is sufficient because admit/terminate are
/// rare (per agent run) and reads are bounded by the number of
/// active sessions.
#[derive(Default)]
pub struct SessionService {
    sessions: RwLock<BTreeMap<SessionId, SessionEntry>>,
}

impl SessionService {
    pub fn new() -> Self {
        Self::default()
    }

    /// Admit a new session. The caller (Kernel::invoke admission
    /// path) supplies the assembled Session handle; this service
    /// indexes it. Returns the inserted handle's id for chaining.
    pub fn admit(&self, session: Session) -> anyhow::Result<SessionId> {
        let id = session.id.clone();
        let mut g = self
            .sessions
            .write()
            .map_err(|_| anyhow::anyhow!("SessionService lock poisoned"))?;
        if g.contains_key(&id) {
            anyhow::bail!("session id {id} already admitted");
        }
        let (tx, _) = broadcast::channel(256);
        let admitted_event = serde_json::json!({
            "kind": "admitted",
            "session_id": id.as_str(),
            "agent": session.agent.as_str(),
            "node": session.node.as_str(),
            "started_unix_ms": session.started_unix_ms,
        });
        let entry = SessionEntry {
            meta: session,
            history: vec![admitted_event],
            broadcast: tx,
        };
        g.insert(id.clone(), entry);
        Ok(id)
    }

    /// Mark a session terminated. Writes `ended_unix_ms` to the
    /// indexed entry, emits a `terminated` timeline frame, and
    /// drops the broadcast sender so live subscribers see the
    /// channel close (their next recv returns Closed → IPC server
    /// emits Terminal{done}).
    pub fn terminate(&self, id: &SessionId, ended_unix_ms: i64) -> anyhow::Result<()> {
        let mut g = self
            .sessions
            .write()
            .map_err(|_| anyhow::anyhow!("SessionService lock poisoned"))?;
        match g.get_mut(id) {
            Some(entry) => {
                entry.meta.ended_unix_ms = Some(ended_unix_ms);
                let event = serde_json::json!({
                    "kind": "terminated",
                    "session_id": id.as_str(),
                    "ended_unix_ms": ended_unix_ms,
                });
                entry.history.push(event.clone());
                let _ = entry.broadcast.send(event);
                // Note: we keep the entry around so late attaches
                // can still replay history. The broadcast::Sender
                // stays alive too — a "done" terminal is the
                // Client's signal that no more frames will come,
                // not a hard channel close.
                Ok(())
            }
            None => anyhow::bail!("session {id} not found"),
        }
    }

    /// Emit one timeline frame on a session. Stored in history
    /// (so late subscribers replay it) AND broadcast (so live
    /// subscribers see it). Used by the future Kernel::invoke
    /// admission path to push agent-driver progress into the
    /// session timeline.
    pub fn emit_event(&self, id: &SessionId, event: Value) -> anyhow::Result<()> {
        let mut g = self
            .sessions
            .write()
            .map_err(|_| anyhow::anyhow!("SessionService lock poisoned"))?;
        match g.get_mut(id) {
            Some(entry) => {
                entry.history.push(event.clone());
                let _ = entry.broadcast.send(event);
                Ok(())
            }
            None => anyhow::bail!("session {id} not found"),
        }
    }

    /// Subscribe to live timeline frames on a session. Returns the
    /// past `history[since_seq..]` as the snapshot half plus a
    /// fresh broadcast receiver for the live half. The session.attach
    /// ability handler hands the pair to StreamSource::SnapshotThenLive.
    pub fn subscribe_session(
        &self,
        id: &SessionId,
        since_seq: usize,
    ) -> anyhow::Result<(Vec<Value>, broadcast::Receiver<Value>)> {
        let g = self
            .sessions
            .read()
            .map_err(|_| anyhow::anyhow!("SessionService lock poisoned"))?;
        let entry = g
            .get(id)
            .ok_or_else(|| anyhow::anyhow!("session {id} not found"))?;
        let history = entry.history.iter().skip(since_seq).cloned().collect();
        let rx = entry.broadcast.subscribe();
        Ok((history, rx))
    }

    /// Snapshot of every session currently indexed (active or
    /// terminated). v1 returns Vec; v2 will paginate when the
    /// index grows large.
    pub fn list_active(&self) -> Vec<Session> {
        match self.sessions.read() {
            Ok(g) => g.values().map(|e| e.meta.clone()).collect(),
            Err(_) => Vec::new(),
        }
    }

    /// Lookup by id.
    pub fn get(&self, id: &SessionId) -> Option<Session> {
        self.sessions
            .read()
            .ok()
            .and_then(|g| g.get(id).map(|e| e.meta.clone()))
    }

    /// Convenience constructor for admission code paths that have
    /// the four input fields but not a full Session struct yet.
    pub fn make_session(id: SessionId, agent: AgentId, node: NodeId, tenant: TenantId) -> Session {
        Session {
            id,
            agent,
            node,
            tenant,
            started_unix_ms: chrono::Utc::now().timestamp_millis(),
            ended_unix_ms: None,
        }
    }
}

impl std::fmt::Debug for SessionService {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        let n = self.sessions.read().ok().map(|g| g.len()).unwrap_or(0);
        write!(f, "SessionService {{ sessions: {n} }}")
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn s(id: &str, agent: &str) -> Session {
        SessionService::make_session(
            SessionId::new(id),
            AgentId::new(agent),
            NodeId::new("self"),
            TenantId::default_v1(),
        )
    }

    #[test]
    fn admit_then_list_returns_the_session() {
        let svc = SessionService::new();
        svc.admit(s("run-1", "alice")).unwrap();
        let listed = svc.list_active();
        assert_eq!(listed.len(), 1);
        assert_eq!(listed[0].id, SessionId::new("run-1"));
        assert_eq!(listed[0].agent, AgentId::new("alice"));
        assert!(listed[0].ended_unix_ms.is_none());
    }

    #[test]
    fn duplicate_admission_rejected_with_clear_error() {
        // Spirit: admission is keyed by session_id (which is the
        // invocation_id at the system level). Two admissions for
        // the same id is an upstream bug — refusing loudly here
        // catches it before the bookkeeping diverges.
        let svc = SessionService::new();
        svc.admit(s("dup", "x")).unwrap();
        let err = svc.admit(s("dup", "x")).unwrap_err();
        assert!(format!("{err}").contains("already admitted"));
    }

    #[test]
    fn terminate_marks_end_timestamp_and_keeps_entry() {
        // After terminate, the entry is still in the index but has
        // an `ended_unix_ms`. Late-joining readers still see the
        // run, and a "list active only" filter would gate on the
        // None-vs-Some test of this field.
        let svc = SessionService::new();
        svc.admit(s("done", "alice")).unwrap();
        svc.terminate(&SessionId::new("done"), 1_700_000_000_000)
            .unwrap();
        let s = svc.get(&SessionId::new("done")).unwrap();
        assert_eq!(s.ended_unix_ms, Some(1_700_000_000_000));
        // Entry kept in the index — list_active surfaces it.
        assert_eq!(svc.list_active().len(), 1);
    }

    #[tokio::test]
    async fn subscribe_replays_history_and_tails_live_emits() {
        // The "replay then tail" property: a subscriber joining
        // mid-flight sees every past event since `since_seq` (in
        // the snapshot) AND every event emitted after subscribe
        // (on the broadcast). This is what makes the session
        // attach view in the GUI not lose state when the user
        // opens the panel late.
        let svc = SessionService::new();
        svc.admit(s("live", "alice")).unwrap();
        // history at this point = [admitted].
        svc.emit_event(
            &SessionId::new("live"),
            serde_json::json!({"kind": "progress", "n": 1}),
        )
        .unwrap();

        let (snap, mut rx) = svc.subscribe_session(&SessionId::new("live"), 0).unwrap();
        // 2 frames already in history.
        assert_eq!(snap.len(), 2);
        assert_eq!(snap[0]["kind"], "admitted");
        assert_eq!(snap[1]["kind"], "progress");

        // Emit one more after subscribe; live tail receives it.
        svc.emit_event(
            &SessionId::new("live"),
            serde_json::json!({"kind": "progress", "n": 2}),
        )
        .unwrap();

        let live = rx.recv().await.expect("live frame");
        assert_eq!(live["kind"], "progress");
        assert_eq!(live["n"], 2);
    }

    #[test]
    fn since_seq_skips_history_prefix() {
        // since_seq=1 means "I already have frame 0". Replay must
        // begin at frame 1.
        let svc = SessionService::new();
        svc.admit(s("late", "alice")).unwrap();
        svc.emit_event(&SessionId::new("late"), serde_json::json!({"x": 1}))
            .unwrap();
        svc.emit_event(&SessionId::new("late"), serde_json::json!({"x": 2}))
            .unwrap();
        let (snap, _) = svc.subscribe_session(&SessionId::new("late"), 1).unwrap();
        assert_eq!(snap.len(), 2); // [{x:1}, {x:2}], skipped admitted
        assert_eq!(snap[0]["x"], 1);
    }

    #[test]
    fn terminate_unknown_session_errors() {
        let svc = SessionService::new();
        let err = svc.terminate(&SessionId::new("ghost"), 0).unwrap_err();
        assert!(format!("{err}").contains("not found"));
    }

    #[test]
    fn list_active_iteration_order_is_deterministic_by_id() {
        // BTreeMap-backed: lexical order on session_id.
        // PR-ATTACH's a2a discovery + golden fixtures rely on this.
        let svc = SessionService::new();
        svc.admit(s("c", "a")).unwrap();
        svc.admit(s("a", "a")).unwrap();
        svc.admit(s("b", "a")).unwrap();
        let ids: Vec<_> = svc.list_active().into_iter().map(|s| s.id).collect();
        assert_eq!(
            ids,
            vec![
                SessionId::new("a"),
                SessionId::new("b"),
                SessionId::new("c"),
            ]
        );
    }
}
