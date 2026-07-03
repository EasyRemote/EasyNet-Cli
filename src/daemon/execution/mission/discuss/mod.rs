// EasyNet CLI — Execution / Discuss sub-service
// ==============================================
//
// File: src/daemon/execution/discuss/mod.rs
// Description: Multi-agent room registry + per-room turn broadcast.
//              PR-DISCUSS surfaces this as the
//              `discuss.{create,post,subscribe}` ability
//              triple so a Client can host an asynchronous chat
//              between any mix of agents (local + remote).
//
// What v1 provides
// ----------------
// - In-memory room registry keyed by RoomId; metadata (origin
//   node, tenant, participants, topic).
// - Per-room broadcast channel of `DiscussTurn` events. Each
//   `discuss.post` call appends a turn (the speaker,
//   message, timestamp); subscribers see live turns; the
//   broadcast channel is the v1 "stream".
//
// What this is NOT
// ----------------
// Not the synchronous "run N rounds of these agents and return
// the log" orchestration path. This sub-service is a long-lived
// room a Client can attach to and post into at any time. They
// serve different products.
//
// Room persistence
// ----------------
// v1 keeps rooms in memory only. Daemon restart drops every
// room. v2 will write to ~/.easynet/tenants/<tenant>/discuss-rooms/.
//
// Author: Silan Hu <silan.hu@u.nus.edu>
// Copyright (c) 2026 EasyNet. All rights reserved.

use std::collections::BTreeMap;
use std::sync::RwLock;

use serde::{Deserialize, Serialize};
use serde_json::Value;
use tokio::sync::broadcast;
use uuid::Uuid;

use crate::core::domain::{AgentId, DiscussRoom, NodeId, RoomId, TenantId};

/// One turn posted into a discuss room. Mirrors the wire shape
/// the IPC layer fans out for `discuss.subscribe`.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct DiscussTurn {
    pub sequence: i64,
    pub timestamp_unix_ms: i64,
    pub speaker: AgentId,
    pub message: String,
    /// Optional structured payload (e.g. tool-call result). Free-
    /// form to avoid coupling the room shape to one product
    /// surface; downstream consumers that need typed payloads
    /// keep their own narrower schema on top.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub payload: Option<Value>,
}

struct RoomState {
    meta: DiscussRoom,
    next_sequence: i64,
    turns: Vec<DiscussTurn>,
    broadcast: broadcast::Sender<DiscussTurn>,
}

impl RoomState {
    fn new(meta: DiscussRoom) -> Self {
        let (tx, _) = broadcast::channel(256);
        Self {
            meta,
            next_sequence: 0,
            turns: Vec::new(),
            broadcast: tx,
        }
    }
}

#[derive(Default)]
pub struct DiscussService {
    rooms: RwLock<BTreeMap<RoomId, RoomState>>,
}

impl DiscussService {
    pub fn new() -> Self {
        Self::default()
    }

    /// Create a new room. Generates a fresh `RoomId`, indexes
    /// metadata + state. Returns the new id.
    pub fn create(
        &self,
        participants: Vec<String>,
        topic: Option<String>,
    ) -> anyhow::Result<RoomId> {
        if participants.is_empty() {
            anyhow::bail!("discuss.create: participants list must not be empty");
        }
        let id = RoomId::new(format!("room-{}", Uuid::new_v4()));
        let meta = DiscussRoom {
            id: id.clone(),
            origin_node: NodeId::new("self"),
            tenant: TenantId::default_v1(),
            participants: participants.into_iter().map(AgentId::new).collect(),
            topic,
            created_unix_ms: chrono::Utc::now().timestamp_millis(),
        };
        let mut g = self
            .rooms
            .write()
            .map_err(|_| anyhow::anyhow!("DiscussService lock poisoned"))?;
        g.insert(id.clone(), RoomState::new(meta));
        Ok(id)
    }

    /// Post a turn into a room. Appends to the in-memory log,
    /// notifies subscribers. Returns the assigned sequence.
    pub fn post(
        &self,
        room: &RoomId,
        speaker: AgentId,
        message: impl Into<String>,
        payload: Option<Value>,
    ) -> anyhow::Result<i64> {
        let mut g = self
            .rooms
            .write()
            .map_err(|_| anyhow::anyhow!("DiscussService lock poisoned"))?;
        let state = g
            .get_mut(room)
            .ok_or_else(|| anyhow::anyhow!("room {room} does not exist"))?;
        let seq = state.next_sequence;
        let turn = DiscussTurn {
            sequence: seq,
            timestamp_unix_ms: chrono::Utc::now().timestamp_millis(),
            speaker,
            message: message.into(),
            payload,
        };
        state.next_sequence += 1;
        state.turns.push(turn.clone());
        // send returns Err only when no subscribers — fine; turns
        // stay in the in-memory log for late joiners.
        let _ = state.broadcast.send(turn);
        Ok(seq)
    }

    /// Snapshot every room. Deterministic-ordered (BTreeMap).
    pub fn list(&self) -> Vec<DiscussRoom> {
        match self.rooms.read() {
            Ok(g) => g.values().map(|s| s.meta.clone()).collect(),
            Err(_) => Vec::new(),
        }
    }

    /// Subscribe to live turns posted to a room. Each call returns
    /// a fresh `broadcast::Receiver`; subscribers obtained AFTER a
    /// turn was posted do not see that turn — combine with
    /// `turns_from(room, since_seq)` for the "replay then tail"
    /// pattern. The discuss.subscribe ability handler does
    /// this composition via `StreamSource::SnapshotThenLive`.
    pub fn subscribe_room(
        &self,
        room: &RoomId,
    ) -> anyhow::Result<broadcast::Receiver<DiscussTurn>> {
        let g = self
            .rooms
            .read()
            .map_err(|_| anyhow::anyhow!("DiscussService lock poisoned"))?;
        let state = g
            .get(room)
            .ok_or_else(|| anyhow::anyhow!("room {room} does not exist"))?;
        Ok(state.broadcast.subscribe())
    }

    /// Read every turn in a room from `since_seq` onwards. Used as
    /// the "snapshot" half of the SnapshotThenLive stream.
    pub fn turns_from(&self, room: &RoomId, since_seq: i64) -> anyhow::Result<Vec<DiscussTurn>> {
        let g = self
            .rooms
            .read()
            .map_err(|_| anyhow::anyhow!("DiscussService lock poisoned"))?;
        let state = g
            .get(room)
            .ok_or_else(|| anyhow::anyhow!("room {room} does not exist"))?;
        Ok(state
            .turns
            .iter()
            .filter(|t| t.sequence >= since_seq)
            .cloned()
            .collect())
    }
}

impl std::fmt::Debug for DiscussService {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        let n = self.rooms.read().ok().map(|g| g.len()).unwrap_or(0);
        write!(f, "DiscussService {{ rooms: {n} }}")
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn create_then_list_returns_the_room() {
        let s = DiscussService::new();
        let id = s
            .create(vec!["alice".into(), "bob".into()], Some("topic".into()))
            .unwrap();
        let listed = s.list();
        assert_eq!(listed.len(), 1);
        assert_eq!(listed[0].id, id);
        assert_eq!(listed[0].participants.len(), 2);
        assert_eq!(listed[0].topic.as_deref(), Some("topic"));
    }

    #[test]
    fn create_with_empty_participants_errors() {
        // A room without anyone in it has no semantics. Refusing
        // up front catches the misuse at the call site.
        let s = DiscussService::new();
        let err = s.create(vec![], None).unwrap_err();
        assert!(format!("{err}").contains("participants"));
    }

    #[test]
    fn post_assigns_monotonic_sequence_per_room() {
        let s = DiscussService::new();
        let r = s.create(vec!["alice".into(), "bob".into()], None).unwrap();
        let s0 = s.post(&r, AgentId::new("alice"), "hi", None).unwrap();
        let s1 = s.post(&r, AgentId::new("bob"), "hello", None).unwrap();
        let s2 = s.post(&r, AgentId::new("alice"), "next", None).unwrap();
        assert_eq!(s0, 0);
        assert_eq!(s1, 1);
        assert_eq!(s2, 2);
    }

    #[test]
    fn turns_from_honours_since_seq() {
        let s = DiscussService::new();
        let r = s.create(vec!["alice".into()], None).unwrap();
        for i in 0..5 {
            s.post(&r, AgentId::new("alice"), format!("msg{i}"), None)
                .unwrap();
        }
        let tail = s.turns_from(&r, 3).unwrap();
        assert_eq!(tail.len(), 2);
        assert_eq!(tail[0].sequence, 3);
        assert_eq!(tail[1].sequence, 4);
    }

    #[test]
    fn post_unknown_room_errors() {
        let s = DiscussService::new();
        let err = s
            .post(&RoomId::new("never"), AgentId::new("alice"), "hi", None)
            .unwrap_err();
        assert!(format!("{err}").contains("does not exist"));
    }
}
