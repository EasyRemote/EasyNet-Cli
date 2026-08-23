// EasyNet CLI — remote desktop session event log
// ===============================================
//
// File: plugins/remote-desktop/src/event_log.rs
// Description: Bounded event log and live event broadcast for sessions.

use std::collections::VecDeque;

use super::contract::RemoteDesktopSessionState;
use serde_json::{json, Value};
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

/// Replay projection for `remote_desktop.watch_events`.
///
/// What this is NOT: an unbounded history cursor. The event log is a fixed
/// ring; when a caller asks for a sequence older than the retained window this
/// projection prepends a diagnostic frame so consumers can re-snapshot instead
/// of silently accepting a partial lifecycle history as complete.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(in crate::daemon::plugins::remote_desktop) struct RemoteDesktopEventReplay {
    events: Vec<Value>,
}

impl RemoteDesktopEventReplay {
    fn new(events: Vec<Value>) -> Self {
        Self { events }
    }

    pub(in crate::daemon::plugins::remote_desktop) fn into_events(self) -> Vec<Value> {
        self.events
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

    pub(in crate::daemon::plugins::remote_desktop) fn rehydrate(
        events: Vec<Value>,
        terminal: bool,
    ) -> anyhow::Result<Self> {
        let mut retained = VecDeque::with_capacity(MAX_EVENTS_PER_SESSION);
        let skip = events.len().saturating_sub(MAX_EVENTS_PER_SESSION);
        let mut max_sequence = 0;
        for event in events.into_iter().skip(skip) {
            let sequence = event
                .get("sequence")
                .and_then(Value::as_u64)
                .ok_or_else(|| anyhow::anyhow!("RemoteApp recovery event is missing sequence"))?;
            max_sequence = max_sequence.max(sequence);
            retained.push_back(RemoteDesktopEventRecord::new(event));
        }
        let event_tx = if terminal {
            None
        } else {
            Some(broadcast::channel(MAX_EVENTS_PER_SESSION).0)
        };
        Ok(Self {
            events: retained,
            event_tx,
            next_sequence: max_sequence.saturating_add(1).max(1),
        })
    }

    pub(in crate::daemon::plugins::remote_desktop) fn events(&self) -> Vec<Value> {
        self.events
            .iter()
            .map(RemoteDesktopEventRecord::to_value)
            .collect()
    }

    pub(in crate::daemon::plugins::remote_desktop) fn replay_from(
        &self,
        session_id: &str,
        state: RemoteDesktopSessionState,
        from_sequence: u64,
    ) -> RemoteDesktopEventReplay {
        let first_retained_sequence = self.events.front().and_then(event_sequence);
        let mut events = Vec::new();
        if let Some(first_sequence) = first_retained_sequence {
            if from_sequence.saturating_add(1) < first_sequence {
                events.push(self.compaction_diagnostic_event(
                    session_id,
                    state,
                    from_sequence,
                    first_sequence,
                ));
            }
        }
        events.extend(
            self.events
                .iter()
                .map(RemoteDesktopEventRecord::to_value)
                .filter(|event| {
                    event
                        .get("sequence")
                        .and_then(Value::as_u64)
                        .map(|sequence| sequence > from_sequence)
                        .unwrap_or(true)
                }),
        );
        RemoteDesktopEventReplay::new(events)
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
    ) -> Value {
        let sequence = self.next_sequence;
        self.next_sequence = self.next_sequence.saturating_add(1);
        let target_field = |name: &str| payload.get(name).cloned().unwrap_or(Value::Null);
        let subject_ura = target_field("subject_ura");
        let binding_id = target_field("binding_id");
        let binding_epoch = target_field("binding_epoch");
        let previous_target_identity_epoch = target_field("previous_target_identity_epoch");
        let target_identity_epoch = target_field("target_identity_epoch");
        let target_geometry_revision = target_field("target_geometry_revision");
        let media_source_epoch = target_field("media_source_epoch");
        let consent_epoch = target_field("consent_epoch");
        let transport_epoch = target_field("transport_epoch");
        let reason_code = target_field("reason_code");
        let recoverability = target_field("recoverability");
        let event = json!({
            "event_id": format!("{session_id}:{sequence}"),
            "session_id": session_id,
            "sequence": sequence,
            "subject_ura": subject_ura,
            "binding_id": binding_id,
            "binding_epoch": binding_epoch,
            "previous_target_identity_epoch": previous_target_identity_epoch,
            "target_identity_epoch": target_identity_epoch,
            "target_geometry_revision": target_geometry_revision,
            "media_source_epoch": media_source_epoch,
            "consent_epoch": consent_epoch,
            "transport_epoch": transport_epoch,
            "event_type": event_type,
            "event_type_proto": event_type_proto_name(event_type),
            "reason_code": reason_code,
            "recoverability": recoverability,
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
        self.events
            .back()
            .expect("event log push appends one record")
            .to_value()
    }

    fn compaction_diagnostic_event(
        &self,
        session_id: &str,
        state: RemoteDesktopSessionState,
        from_sequence: u64,
        first_retained_sequence: u64,
    ) -> Value {
        let last_retained_sequence = self.events.back().and_then(event_sequence);
        let marker_sequence = first_retained_sequence.saturating_sub(1);
        let payload = json!({
            "reason_code": "event_log_compacted",
            "recoverability": "resnapshot",
            "requested_from_sequence": from_sequence,
            "first_retained_sequence": first_retained_sequence,
            "last_retained_sequence": last_retained_sequence,
            "next_sequence": self.next_sequence,
            "retained_event_capacity": MAX_EVENTS_PER_SESSION,
        });
        json!({
            "event_id": format!(
                "{session_id}:event-log-compacted:{from_sequence}:{first_retained_sequence}"
            ),
            "session_id": session_id,
            "sequence": marker_sequence,
            "subject_ura": Value::Null,
            "binding_id": Value::Null,
            "binding_epoch": Value::Null,
            "previous_target_identity_epoch": Value::Null,
            "target_identity_epoch": Value::Null,
            "target_geometry_revision": Value::Null,
            "media_source_epoch": Value::Null,
            "consent_epoch": Value::Null,
            "transport_epoch": Value::Null,
            "event_type": "EVENT_LOG_COMPACTED",
            "event_type_proto": event_type_proto_name("EVENT_LOG_COMPACTED"),
            "reason_code": "event_log_compacted",
            "recoverability": "resnapshot",
            "state": state.json_name(),
            "state_proto": state.wire_name(),
            "terminal": state.is_terminal(),
            "at_ms": now_ms(),
            "payload": payload,
        })
    }
}

fn event_sequence(event: &RemoteDesktopEventRecord) -> Option<u64> {
    event.value.get("sequence").and_then(Value::as_u64)
}

const TARGET_CHANGED_EVENT_TYPES: &[&str] = &[
    "CAPTURE_TARGET_STALE",
    "CAPTURE_TARGET_IDENTITY_MISMATCH",
    "CAPTURE_TARGET_AMBIGUOUS",
    "DISPLAY_FALLBACK_FORBIDDEN",
    "SCREEN_CAPTURE_PERMISSION_DENIED",
    "TARGET_MOVED",
    "TARGET_RESIZED",
    "TARGET_TITLE_CHANGED",
    "TARGET_FOCUSED",
    "TARGET_BLURRED",
    "TARGET_HIDDEN",
    "TARGET_VISIBLE",
    "TARGET_MINIMIZED",
    "TARGET_RESTORED",
    "TARGET_LOST",
    "TARGET_REBIND_ATTEMPTED",
    "TARGET_REBOUND",
    "TARGET_REBIND_FAILED",
    "TARGET_BINDING_CHANGED",
    "TARGET_PERMISSION_REVOKED",
    "DISPLAY_TOPOLOGY_CHANGED",
];

pub(in crate::daemon::plugins::remote_desktop) fn event_type_proto_name(
    event_type: &str,
) -> &'static str {
    if TARGET_CHANGED_EVENT_TYPES.contains(&event_type) {
        return "REMOTE_DESKTOP_EVENT_TARGET_CHANGED";
    }

    match event_type {
        "SESSION_CREATED" => "REMOTE_DESKTOP_EVENT_SESSION_CREATED",
        "CAPTURE_TARGET_RESOLVED" => "REMOTE_DESKTOP_EVENT_CAPTURE_TARGET_RESOLVED",
        "TARGET_BOUND" => "REMOTE_DESKTOP_EVENT_TARGET_BOUND",
        "DESCRIPTION_SET" => "REMOTE_DESKTOP_EVENT_DESCRIPTION_SET",
        "ICE_CANDIDATE_ADDED" => "REMOTE_DESKTOP_EVENT_CANDIDATE_ADDED",
        "LOCAL_ICE_CANDIDATE" => "REMOTE_DESKTOP_EVENT_LOCAL_CANDIDATE",
        "ICE_CONNECTION_STATE_CHANGED" => "REMOTE_DESKTOP_EVENT_ICE_STATE_CHANGED",
        "ICE_CANDIDATE_ERROR" => "REMOTE_DESKTOP_EVENT_ICE_CANDIDATE_ERROR",
        "MEDIA_PIPELINE_STATS" => "REMOTE_DESKTOP_EVENT_MEDIA_PIPELINE_STATS",
        "MEDIA_PIPELINE_DOWNGRADED" => "REMOTE_DESKTOP_EVENT_QUALITY_CHANGED",
        "TRANSPORT_CONNECTED" => "REMOTE_DESKTOP_EVENT_TRANSPORT_CONNECTED",
        "MEDIA_SOURCE_LOST" | "TRANSPORT_FAILED" | "SESSION_DEGRADED" => {
            "REMOTE_DESKTOP_EVENT_STATE_CHANGED"
        }
        "EVENT_LOG_COMPACTED" => "REMOTE_DESKTOP_EVENT_STATE_CHANGED",
        "TRANSPORT_BLOCKED" => "REMOTE_DESKTOP_EVENT_TRANSPORT_BLOCKED",
        "INPUT_CHANNEL_OPENING"
        | "INPUT_CHANNEL_OPENED"
        | "INPUT_CHANNEL_CLOSED"
        | "INPUT_CHANNEL_REJECTED"
        | "INPUT_CHANNEL_ERROR"
        | "INPUT_FRAME_APPLIED"
        | "INPUT_FRAME_REJECTED"
        | "INPUT_PERMISSION_BLOCKED" => "REMOTE_DESKTOP_EVENT_INPUT",
        "LEASE_REFRESHED" => "REMOTE_DESKTOP_EVENT_LEASE_REFRESHED",
        "SESSION_CLOSING" => "REMOTE_DESKTOP_EVENT_SESSION_CLOSING",
        "SESSION_CLOSED" => "REMOTE_DESKTOP_EVENT_SESSION_CLOSED",
        _ => "REMOTE_DESKTOP_EVENT_STATE_CHANGED",
    }
}

#[cfg(test)]
mod tests {
    use serde_json::json;

    use super::{
        event_type_proto_name, RemoteDesktopEventLog, MAX_EVENTS_PER_SESSION,
        TARGET_CHANGED_EVENT_TYPES,
    };
    use crate::daemon::plugins::remote_desktop::contract::RemoteDesktopSessionState;

    #[test]
    fn event_log_push_returns_the_stored_event_record() {
        let mut log = RemoteDesktopEventLog::new();

        let event = log.push(
            "rd-event-return",
            RemoteDesktopSessionState::Closed,
            "SESSION_CLOSED",
            json!({ "reason_code": "caller_ended" }),
        );

        assert_eq!(event["event_id"], json!("rd-event-return:1"));
        assert_eq!(event["sequence"], json!(1));
        assert_eq!(event["event_type"], json!("SESSION_CLOSED"));
        assert_eq!(event["terminal"], json!(true));
        assert_eq!(log.events(), vec![event]);
    }

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
    fn event_replay_projects_compaction_before_retained_window() {
        let mut log = RemoteDesktopEventLog::new();

        for index in 0..=MAX_EVENTS_PER_SESSION {
            log.push(
                "rd-event-replay",
                RemoteDesktopSessionState::Negotiating,
                "TARGET_MOVED",
                json!({ "index": index }),
            );
        }

        let replay = log
            .replay_from("rd-event-replay", RemoteDesktopSessionState::Negotiating, 0)
            .into_events();
        let diagnostic = replay.first().expect("replay starts with diagnostic");
        assert_eq!(diagnostic["event_type"], json!("EVENT_LOG_COMPACTED"));
        assert_eq!(diagnostic["reason_code"], json!("event_log_compacted"));
        assert_eq!(diagnostic["recoverability"], json!("resnapshot"));
        assert_eq!(
            diagnostic["payload"]["retained_event_capacity"],
            json!(MAX_EVENTS_PER_SESSION)
        );
        assert_eq!(diagnostic["payload"]["requested_from_sequence"], json!(0));
        assert_eq!(diagnostic["payload"]["first_retained_sequence"], json!(2));
        assert_eq!(replay[1]["sequence"], json!(2));
    }

    #[test]
    fn target_rebind_failed_projects_as_target_change_event() {
        assert_eq!(
            event_type_proto_name("TARGET_REBIND_FAILED"),
            "REMOTE_DESKTOP_EVENT_TARGET_CHANGED"
        );
    }

    #[test]
    fn spec_target_lifecycle_events_have_explicit_proto_projection() {
        let expected = [
            "CAPTURE_TARGET_STALE",
            "CAPTURE_TARGET_IDENTITY_MISMATCH",
            "CAPTURE_TARGET_AMBIGUOUS",
            "DISPLAY_FALLBACK_FORBIDDEN",
            "SCREEN_CAPTURE_PERMISSION_DENIED",
            "TARGET_MOVED",
            "TARGET_RESIZED",
            "TARGET_TITLE_CHANGED",
            "TARGET_FOCUSED",
            "TARGET_BLURRED",
            "TARGET_HIDDEN",
            "TARGET_VISIBLE",
            "TARGET_MINIMIZED",
            "TARGET_RESTORED",
            "TARGET_LOST",
            "TARGET_REBIND_ATTEMPTED",
            "TARGET_REBOUND",
            "TARGET_REBIND_FAILED",
            "TARGET_BINDING_CHANGED",
            "TARGET_PERMISSION_REVOKED",
            "DISPLAY_TOPOLOGY_CHANGED",
        ];

        assert_eq!(TARGET_CHANGED_EVENT_TYPES, expected);
        for event_type in expected {
            assert_eq!(
                event_type_proto_name(event_type),
                "REMOTE_DESKTOP_EVENT_TARGET_CHANGED",
                "{event_type} must stay in the SPEC target lifecycle taxonomy"
            );
        }
        assert_eq!(
            event_type_proto_name("CAPTURE_TARGET_RESOLVED"),
            "REMOTE_DESKTOP_EVENT_CAPTURE_TARGET_RESOLVED"
        );
        assert_eq!(
            event_type_proto_name("TARGET_BOUND"),
            "REMOTE_DESKTOP_EVENT_TARGET_BOUND"
        );
    }
}
