// EasyNet CLI — remote desktop session event log
// ===============================================
//
// File: src/plugins/builtin/remote_desktop/event_log.rs
// Description: Bounded event log and live event broadcast for sessions.

use easynet_axon::RemoteDesktopSessionState;
use serde_json::{json, Value};
use tokio::sync::broadcast;

use crate::plugins::remote_desktop::session::now_ms;

pub(in crate::plugins::builtin::remote_desktop) const MAX_EVENTS_PER_SESSION: usize = 256;

/// Stored remote desktop session event.
///
/// What this is NOT: a second event schema. The public stream remains JSON;
/// this wrapper labels the stored value as an event record so the session log
/// no longer stores arbitrary `Value` rows.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(in crate::plugins::builtin::remote_desktop) struct RemoteDesktopEventRecord {
    value: Value,
}

impl RemoteDesktopEventRecord {
    fn new(value: Value) -> Self {
        Self { value }
    }

    pub(in crate::plugins::builtin::remote_desktop) fn to_value(&self) -> Value {
        self.value.clone()
    }
}

/// Bounded event log owned by one remote desktop session.
///
/// Invariant 1: sequence numbers are monotonic within this log and only
/// advance through [`Self::push`].
/// Invariant 2: stored events are capped at [`MAX_EVENTS_PER_SESSION`], while
/// live subscribers receive every event that the broadcast channel can retain.
/// Invariant 3: event naming is projected here, so session state mutation does
/// not know Axon protobuf enum labels.
#[derive(Debug, Clone)]
pub(in crate::plugins::builtin::remote_desktop) struct RemoteDesktopEventLog {
    events: Vec<RemoteDesktopEventRecord>,
    event_tx: broadcast::Sender<Value>,
    next_sequence: u64,
}

impl RemoteDesktopEventLog {
    pub(in crate::plugins::builtin::remote_desktop) fn new() -> Self {
        let (event_tx, _) = broadcast::channel(MAX_EVENTS_PER_SESSION);
        Self {
            events: Vec::new(),
            event_tx,
            next_sequence: 1,
        }
    }

    pub(in crate::plugins::builtin::remote_desktop) fn events(&self) -> Vec<Value> {
        self.events
            .iter()
            .map(RemoteDesktopEventRecord::to_value)
            .collect()
    }

    pub(in crate::plugins::builtin::remote_desktop) fn subscribe(
        &self,
    ) -> broadcast::Receiver<Value> {
        self.event_tx.subscribe()
    }

    pub(in crate::plugins::builtin::remote_desktop) fn push(
        &mut self,
        session_id: &str,
        state: RemoteDesktopSessionState,
        event_type: &str,
        payload: Value,
    ) {
        let sequence = self.next_sequence;
        self.next_sequence = self.next_sequence.saturating_add(1);
        let event = json!({
            "event_id": format!("{session_id}:{sequence}"),
            "session_id": session_id,
            "sequence": sequence,
            "event_type": event_type,
            "event_type_proto": event_type_proto_name(event_type),
            "state": state.legacy_label(),
            "state_proto": state.as_proto_name(),
            "terminal": state.is_terminal(),
            "at_ms": now_ms(),
            "payload": payload,
        });
        if self.events.len() == MAX_EVENTS_PER_SESSION {
            self.events.remove(0);
        }
        self.events
            .push(RemoteDesktopEventRecord::new(event.clone()));
        let _ = self.event_tx.send(event);
    }
}

pub(in crate::plugins::builtin::remote_desktop) fn event_type_proto_name(
    event_type: &str,
) -> &'static str {
    match event_type {
        "SESSION_CREATED" => "REMOTE_DESKTOP_EVENT_SESSION_CREATED",
        "DESCRIPTION_SET" => "REMOTE_DESKTOP_EVENT_DESCRIPTION_SET",
        "ICE_CANDIDATE_ADDED" => "REMOTE_DESKTOP_EVENT_CANDIDATE_ADDED",
        "LOCAL_ICE_CANDIDATE" => "REMOTE_DESKTOP_EVENT_LOCAL_CANDIDATE",
        "ICE_CONNECTION_STATE_CHANGED" => "REMOTE_DESKTOP_EVENT_ICE_STATE_CHANGED",
        "ICE_CANDIDATE_ERROR" => "REMOTE_DESKTOP_EVENT_ICE_CANDIDATE_ERROR",
        "MEDIA_PIPELINE_STATS" => "REMOTE_DESKTOP_EVENT_MEDIA_PIPELINE_STATS",
        "MEDIA_PIPELINE_DOWNGRADED" => "REMOTE_DESKTOP_EVENT_QUALITY_CHANGED",
        "TRANSPORT_CONNECTED" => "REMOTE_DESKTOP_EVENT_TRANSPORT_CONNECTED",
        "TRANSPORT_BLOCKED" => "REMOTE_DESKTOP_EVENT_TRANSPORT_BLOCKED",
        "INPUT_CHANNEL_OPENING"
        | "INPUT_CHANNEL_OPENED"
        | "INPUT_CHANNEL_CLOSED"
        | "INPUT_CHANNEL_REJECTED"
        | "INPUT_CHANNEL_ERROR"
        | "INPUT_FRAME_APPLIED"
        | "INPUT_FRAME_REJECTED" => "REMOTE_DESKTOP_EVENT_INPUT",
        "LEASE_REFRESHED" => "REMOTE_DESKTOP_EVENT_LEASE_REFRESHED",
        "SESSION_CLOSING" => "REMOTE_DESKTOP_EVENT_SESSION_CLOSING",
        "SESSION_CLOSED" => "REMOTE_DESKTOP_EVENT_SESSION_CLOSED",
        "SESSION_FAILED" => "REMOTE_DESKTOP_EVENT_SESSION_FAILED",
        _ => "REMOTE_DESKTOP_EVENT_STATE_CHANGED",
    }
}
