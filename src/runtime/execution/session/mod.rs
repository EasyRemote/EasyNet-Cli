// EasyNet CLI — Execution / Session sub-service
// ==============================================
//
// File: src/runtime/execution/session/mod.rs
// Description: Session sub-service. Tracks the live-agent-run
//              registry that PR-ATTACH's `system.session.list` /
//              `system.session.attach` abilities query and
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

use crate::runtime::domain::{AgentId, NodeId, Session, SessionId, TenantId};

/// Session sub-service handle. Holds a `BTreeMap` for deterministic
/// iteration; `RwLock` is sufficient because admit/terminate are
/// rare (per agent run) and reads are bounded by the number of
/// active sessions.
#[derive(Default)]
pub struct SessionService {
    sessions: RwLock<BTreeMap<SessionId, Session>>,
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
        g.insert(id.clone(), session);
        Ok(id)
    }

    /// Mark a session terminated. Writes `ended_unix_ms` to the
    /// indexed entry; the entry stays in the index so late-joining
    /// readers still observe the run.
    pub fn terminate(&self, id: &SessionId, ended_unix_ms: i64) -> anyhow::Result<()> {
        let mut g = self
            .sessions
            .write()
            .map_err(|_| anyhow::anyhow!("SessionService lock poisoned"))?;
        match g.get_mut(id) {
            Some(s) => {
                s.ended_unix_ms = Some(ended_unix_ms);
                Ok(())
            }
            None => anyhow::bail!("session {id} not found"),
        }
    }

    /// Snapshot of every session currently indexed (active or
    /// terminated). v1 returns Vec; v2 will paginate when the
    /// index grows large.
    pub fn list_active(&self) -> Vec<Session> {
        match self.sessions.read() {
            Ok(g) => g.values().cloned().collect(),
            Err(_) => Vec::new(),
        }
    }

    /// Lookup by id.
    pub fn get(&self, id: &SessionId) -> Option<Session> {
        self.sessions.read().ok().and_then(|g| g.get(id).cloned())
    }

    /// Convenience constructor for admission code paths that have
    /// the four input fields but not a full Session struct yet.
    pub fn make_session(
        id: SessionId,
        agent: AgentId,
        node: NodeId,
        tenant: TenantId,
    ) -> Session {
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
        let n = self
            .sessions
            .read()
            .ok()
            .map(|g| g.len())
            .unwrap_or(0);
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

    #[test]
    fn terminate_unknown_session_errors() {
        let svc = SessionService::new();
        let err = svc
            .terminate(&SessionId::new("ghost"), 0)
            .unwrap_err();
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
