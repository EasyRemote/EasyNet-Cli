// EasyNet CLI — remote desktop session event log
// ===============================================
//
// File: plugins/remote-desktop/src/event_log.rs
// Description: Bounded event log and live event broadcast for sessions.

use std::collections::VecDeque;

use super::contract::RemoteDesktopSessionState;
use serde_json::{Value, json};
use tokio::sync::broadcast;

use crate::daemon::plugins::remote_desktop::session::now_ms;

pub(in crate::daemon::plugins::remote_desktop) const MAX_EVENTS_PER_SESSION: usize = 256;

/// Stored remote desktop session event.
///
/// What this is NOT: a second event schema. The public stream remains JSON;
/// this wrapper labels the stored value as an event record so the session log
/// no longer stores arbitrary `Value` rows.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(in crate::daemon::plugins::remote_desktop) struct RemoteDesktopEventRecord {
    value: Value,
}

impl RemoteDesktopEventRecord {
    fn new(value: Value) -> Self {
        Self { value }
    }

    pub(in crate::daemon::plugins::remote_desktop) fn to_value(&self) -> Value {
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
pub(in crate::daemon::plugins::remote_desktop) struct RemoteDesktopEventLog {
    events: VecDeque<RemoteDesktopEventRecord>,
    event_tx: Option<broadcast::Sender<Value>>,
    next_sequence: u64,
}

impl RemoteDesktopEventLog {
    pub(in crate::daemon::plugins::remote_desktop) fn new() -> Self {
        let (event_tx, _) = broadcast::channel(MAX_EVENTS_PER_SESSION);
        Self {
            events: VecDeque::with_capacity(MAX_EVENTS_PER_SESSION),
            event_tx: Some(event_tx),
            next_sequence: 1,
        }
    }

    pub(in crate::daemon::plugins::remote_desktop) fn events(&self) -> Vec<Value> {
        self.events
            .iter()
            .map(RemoteDesktopEventRecord::to_value)
            .collect()
    }

    pub(in crate::daemon::plugins::remote_desktop) fn subscribe(
        &self,
    ) -> Option<broadcast::Receiver<Value>> {
        self.event_tx.as_ref().map(broadcast::Sender::subscribe)
    }

    pub(in crate::daemon::plugins::remote_desktop) fn close(&mut self) {
        self.event_tx.take();
    }

    pub(in crate::daemon::plugins::remote_desktop) fn push(
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
            "state": state.json_name(),
            "state_proto": state.wire_name(),
            "terminal": state.is_terminal(),
            "at_ms": now_ms(),
            "payload": payload,
        });
        if self.events.len() == MAX_EVENTS_PER_SESSION {
            self.events.pop_front();
        }
        self.events
            .push_back(RemoteDesktopEventRecord::new(event.clone()));
        if let Some(event_tx) = self.event_tx.as_ref() {
            let _ = event_tx.send(event);
        }
    }
}

pub(in crate::daemon::plugins::remote_desktop) fn event_type_proto_name(
    event_type: &str,
) -> &'static str {
    match event_type {
        "SESSION_CREATED" => "REMOTE_DESKTOP_EVENT_SESSION_CREATED",
        "CAPTURE_TARGET_RESOLVED" => "REMOTE_DESKTOP_EVENT_CAPTURE_TARGET_RESOLVED",
        "TARGET_BOUND" => "REMOTE_DESKTOP_EVENT_TARGET_BOUND",
        "TARGET_MOVED"
        | "TARGET_RESIZED"
        | "TARGET_VISIBLE"
        | "TARGET_HIDDEN"
        | "TARGET_MINIMIZED"
        | "TARGET_LOST"
        | "TARGET_REBIND_FAILED" => "REMOTE_DESKTOP_EVENT_TARGET_CHANGED",
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

#[cfg(test)]
mod tests {
    use serde_json::json;

    use super::{MAX_EVENTS_PER_SESSION, RemoteDesktopEventLog, event_type_proto_name};
    use crate::daemon::plugins::remote_desktop::contract::RemoteDesktopSessionState;

    #[test]
    fn event_log_retains_fixed_ring_and_monotonic_sequences_under_large_storm() {
        const EVENT_STORM: usize = 100_000;
        let mut log = RemoteDesktopEventLog::new();

        for index in 0..EVENT_STORM {
            log.push(
                "rd-event-ring",
                RemoteDesktopSessionState::Negotiating,
                "INPUT_FRAME_REJECTED",
                json!({ "index": index }),
            );
        }

        let events = log.events();
        assert_eq!(
            events.len(),
            MAX_EVENTS_PER_SESSION,
            "retained event projection must stay fixed at the session cap"
        );

        let first_sequence = events
            .first()
            .and_then(|event| event.get("sequence"))
            .and_then(serde_json::Value::as_u64)
            .expect("first retained event has sequence");
        let last_sequence = events
            .last()
            .and_then(|event| event.get("sequence"))
            .and_then(serde_json::Value::as_u64)
            .expect("last retained event has sequence");
        assert_eq!(
            first_sequence,
            (EVENT_STORM - MAX_EVENTS_PER_SESSION + 1) as u64
        );
        assert_eq!(last_sequence, EVENT_STORM as u64);

        for pair in events.windows(2) {
            let previous = pair[0]["sequence"].as_u64().unwrap();
            let next = pair[1]["sequence"].as_u64().unwrap();
            assert_eq!(
                next,
                previous + 1,
                "retained event sequences must remain contiguous and monotonic"
            );
        }
    }

    #[test]
    fn target_rebind_failed_projects_as_target_change_event() {
        assert_eq!(
            event_type_proto_name("TARGET_REBIND_FAILED"),
            "REMOTE_DESKTOP_EVENT_TARGET_CHANGED"
        );
    }
}
