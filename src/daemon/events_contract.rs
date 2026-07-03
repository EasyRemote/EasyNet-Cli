// EasyNet CLI — Events shared contract
// =====================================
//
// File: src/daemon/events_contract.rs
// Description: Shared daemon SDK contract for Events profile directory stream
//              Invocation carriers and typed event-frame projection.
//
// Protocol Responsibility
// -----------------------
// Own the EasyNet-Cli SDK Events DTO projection for daemon directory stream
// frames. Directory event production remains daemon-owned through
// `federation.subscribe_directory_v2`; this module does not create a second
// event bus, perform backend fanout, or execute stream I/O.
//
// Implementation Approach
// -----------------------
// Reuse the shared daemon SDK Invocation carrier builder for the stream
// ability, and project the daemon's tagged `DirectoryEvent` JSON union into a
// binding-facing `EventFrame`. Cursor, drop-report, and terminal semantics are
// represented as explicit value objects rather than ad hoc string rewriting in
// each exported function.
//
// Usage Contract
// --------------
// Directory event projection requires an explicit cursor or sequence supplied
// by the stream reader. The daemon raw `DirectoryEvent` wire shape carries
// event facts, not resume state; the SDK must not infer cursor positions from
// array indexes or timestamps.
//
// Architectural Position
// ----------------------
// EasyNet-Cli daemon SDK Events profile. Runtime Core remains the only stream
// open/close path; this profile owns carrier construction and typed frame DTOs
// for language bindings.

use chrono::TimeZone;
use serde_json::{json, Map, Value};

use crate::core::ura;
use crate::daemon::sdk_contract::{
    build_system_invocation, object, optional_string, required_string, validate_ura,
    SdkContractError,
};

const EVENTS_PROFILE: &str = "events";
const DIRECTORY_STREAM: &str = "directory";
const DIRECTORY_ABILITY: &str =
    crate::daemon::ability::conformance::ABILITY_FEDERATION_SUBSCRIBE_DIRECTORY_V2;
const DEFAULT_RECONNECT_AFTER_MS: u64 = 1_000;
const MIN_HEARTBEAT_INTERVAL_MS: u64 = 1_000;
const MAX_HEARTBEAT_INTERVAL_MS: u64 = 300_000;

pub(crate) type EventsError = SdkContractError;

pub(crate) fn build_directory_subscription_invocation(
    request: &Value,
) -> Result<Value, EventsError> {
    let obj = object(request, "EventsDirectorySubscriptionRequest")?;
    let args = directory_subscription_args(obj)?;
    build_system_invocation(obj, EVENTS_PROFILE, DIRECTORY_ABILITY, args)
}

pub(crate) fn project_directory_event(input: &Value) -> Result<Value, EventsError> {
    let obj = object(input, "EventsDirectoryEventInput")?;
    let event = obj.get("event").unwrap_or(input);
    let event_obj = object(event, "DirectoryEvent")?;
    let cursor = EventCursor::from_input(obj)?;
    let resume_token =
        optional_string(obj, "resume_token").unwrap_or_else(|| cursor.resume_token());
    let event_id = optional_string(obj, "event_id").unwrap_or_else(|| cursor.event_id());
    let frame_kind = DirectoryFrameKind::from_event(event_obj)?;
    EventFrame::directory(
        frame_kind,
        cursor,
        event_id,
        resume_token,
        obj,
        Some(event.clone()),
    )?
    .to_json()
}

pub(crate) fn project_terminal(input: &Value) -> Result<Value, EventsError> {
    let obj = object(input, "EventsTerminalInput")?;
    let cursor = EventCursor::from_input(obj)?;
    let occurred_unix_ms = required_nonnegative_i64(obj, "occurred_unix_ms")?;
    let reconnect_after_ms = optional_u64(obj, "reconnect_after_ms")
        .map(|value| validate_reconnect_after_ms(value, "reconnect_after_ms"))
        .transpose()?;
    let reason = optional_string(obj, "reason").unwrap_or_else(|| "stream_closed".to_string());
    let mut payload = Map::new();
    payload.insert("reason".to_string(), Value::String(reason.clone()));

    EventFrame {
        stream: DIRECTORY_STREAM.to_string(),
        kind: "directory.terminal".to_string(),
        lifecycle: "terminal",
        event_id: optional_string(obj, "event_id").unwrap_or_else(|| cursor.event_id()),
        cursor,
        resume_token: optional_string(obj, "resume_token")
            .unwrap_or_else(|| "terminal".to_string()),
        occurred_unix_ms,
        subject_ref: Value::Null,
        tenant_ref: tenant_ref_from_input(obj, None)?,
        payload: Some(Value::Object(payload)),
        dropped_count: 0,
        reconnect_after_ms,
        terminal: true,
        daemon_event_type: None,
        metadata_extra: json!({ "reason": reason }),
    }
    .to_json()
}

pub(crate) fn project_drop_report(input: &Value) -> Result<Value, EventsError> {
    let obj = object(input, "EventsDropReportInput")?;
    let cursor = EventCursor::from_input(obj)?;
    let occurred_unix_ms = required_nonnegative_i64(obj, "occurred_unix_ms")?;
    let dropped_count = required_u64(obj, "dropped_count")?;
    if dropped_count == 0 {
        return Err(EventsError::InvalidField(
            "dropped_count",
            "must be greater than zero".to_string(),
        ));
    }
    let reconnect_after_ms = optional_u64(obj, "reconnect_after_ms")
        .map(|value| validate_reconnect_after_ms(value, "reconnect_after_ms"))
        .transpose()?
        .or(Some(DEFAULT_RECONNECT_AFTER_MS));
    let reason = optional_string(obj, "reason").unwrap_or_else(|| "consumer_lagged".to_string());
    let payload = json!({
        "reason": reason,
        "dropped_count": dropped_count,
    });
    let metadata_extra = json!({ "reason": reason });

    EventFrame {
        stream: DIRECTORY_STREAM.to_string(),
        kind: "directory.drop_report".to_string(),
        lifecycle: "drop_report",
        event_id: optional_string(obj, "event_id").unwrap_or_else(|| cursor.event_id()),
        cursor,
        resume_token: optional_string(obj, "resume_token").unwrap_or_else(|| "resnapshot".into()),
        occurred_unix_ms,
        subject_ref: Value::Null,
        tenant_ref: tenant_ref_from_input(obj, None)?,
        payload: Some(payload),
        dropped_count,
        reconnect_after_ms,
        terminal: false,
        daemon_event_type: None,
        metadata_extra,
    }
    .to_json()
}

fn directory_subscription_args(obj: &Map<String, Value>) -> Result<Value, EventsError> {
    let mut args = Map::new();
    args.insert(
        "stream".to_string(),
        Value::String(DIRECTORY_STREAM.to_string()),
    );
    args.insert(
        "daemon_ability".to_string(),
        Value::String(DIRECTORY_ABILITY.to_string()),
    );

    if let Some(realm) = optional_string(obj, "realm") {
        validate_token(&realm, "realm")?;
        args.insert("realm".to_string(), Value::String(realm));
    }
    for field in ["owner_ura", "device_ura", "agent_ura"] {
        if let Some(value) = optional_string(obj, field) {
            validate_ura(&value, field)?;
            args.insert(field.to_string(), Value::String(value));
        }
    }
    if let Some(value) = obj.get("resume_cursor").filter(|value| !value.is_null()) {
        let cursor = EventCursor::parse(value, "resume_cursor")?;
        args.insert(
            "resume_cursor".to_string(),
            Value::String(cursor.resume_token()),
        );
    }
    if let Some(interval_ms) = optional_u64(obj, "heartbeat_interval_ms") {
        validate_heartbeat_interval_ms(interval_ms)?;
        args.insert(
            "heartbeat_interval_ms".to_string(),
            Value::Number(interval_ms.into()),
        );
    }

    Ok(Value::Object(args))
}

#[derive(Debug, Clone, PartialEq, Eq)]
struct EventCursor {
    stream: String,
    sequence: u64,
}

impl EventCursor {
    fn from_input(obj: &Map<String, Value>) -> Result<Self, EventsError> {
        if let Some(value) = obj.get("cursor").filter(|value| !value.is_null()) {
            return Self::parse(value, "cursor");
        }
        if let Some(sequence) = optional_u64(obj, "sequence") {
            return Self::new(DIRECTORY_STREAM, sequence, "sequence");
        }
        Err(EventsError::MissingField("cursor"))
    }

    fn parse(value: &Value, field: &'static str) -> Result<Self, EventsError> {
        match value {
            Value::String(raw) => Self::parse_token(raw, field),
            Value::Object(obj) => {
                let stream =
                    optional_string(obj, "stream").unwrap_or_else(|| DIRECTORY_STREAM.to_string());
                let sequence = required_u64(obj, "sequence")?;
                Self::new(&stream, sequence, field)
            }
            _ => Err(EventsError::InvalidField(
                field,
                "must be a cursor string or object".to_string(),
            )),
        }
    }

    fn parse_token(raw: &str, field: &'static str) -> Result<Self, EventsError> {
        let trimmed = raw.trim();
        let Some((stream, sequence)) = trimmed.split_once(':') else {
            return Err(EventsError::InvalidField(
                field,
                "must use `<stream>:<sequence>` form".to_string(),
            ));
        };
        let sequence = sequence.parse::<u64>().map_err(|err| {
            EventsError::InvalidField(field, format!("invalid cursor sequence: {err}"))
        })?;
        Self::new(stream, sequence, field)
    }

    fn new(stream: &str, sequence: u64, field: &'static str) -> Result<Self, EventsError> {
        validate_token(stream, field)?;
        if stream != DIRECTORY_STREAM {
            return Err(EventsError::InvalidField(
                field,
                format!("unsupported stream {stream:?}; expected {DIRECTORY_STREAM:?}"),
            ));
        }
        Ok(Self {
            stream: stream.to_string(),
            sequence,
        })
    }

    fn resume_token(&self) -> String {
        format!("{}:{}", self.stream, self.sequence)
    }

    fn event_id(&self) -> String {
        format!("evt-{}-{}", self.stream, self.sequence)
    }

    fn to_json(&self) -> Value {
        json!({
            "stream": self.stream,
            "sequence": self.sequence,
            "token": self.resume_token(),
        })
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
struct DirectoryFrameKind {
    sdk_kind: &'static str,
    daemon_event_type: &'static str,
    lifecycle: &'static str,
    occurred_unix_ms: i64,
    subject_ura: Option<String>,
}

impl DirectoryFrameKind {
    fn from_event(obj: &Map<String, Value>) -> Result<Self, EventsError> {
        let event_type = required_string(obj, "type")?;
        match event_type {
            "snapshot" => Ok(Self {
                sdk_kind: "directory.snapshot",
                daemon_event_type: "snapshot",
                lifecycle: "snapshot",
                occurred_unix_ms: required_nonnegative_i64(obj, "snapshot_unix_ms")?,
                subject_ura: None,
            }),
            "agent_advertised" => {
                let agent_ura = required_string(obj, "agent_ura")?;
                validate_ura(agent_ura, "agent_ura")?;
                Ok(Self {
                    sdk_kind: "directory.agent_advertised",
                    daemon_event_type: "agent_advertised",
                    lifecycle: "delta",
                    occurred_unix_ms: required_nonnegative_i64(obj, "unix_ms")?,
                    subject_ura: Some(agent_ura.to_string()),
                })
            }
            "agent_revoked" => {
                let agent_ura = required_string(obj, "agent_ura")?;
                validate_ura(agent_ura, "agent_ura")?;
                required_string(obj, "reason")?;
                Ok(Self {
                    sdk_kind: "directory.agent_revoked",
                    daemon_event_type: "agent_revoked",
                    lifecycle: "delta",
                    occurred_unix_ms: required_nonnegative_i64(obj, "unix_ms")?,
                    subject_ura: Some(agent_ura.to_string()),
                })
            }
            "heartbeat" => Ok(Self {
                sdk_kind: "directory.heartbeat",
                daemon_event_type: "heartbeat",
                lifecycle: "heartbeat",
                occurred_unix_ms: required_nonnegative_i64(obj, "unix_ms")?,
                subject_ura: None,
            }),
            "owner_projection_changed" => {
                let owner_ura = required_string(obj, "owner_ura")?;
                validate_ura(owner_ura, "owner_ura")?;
                let host_device_ura = required_string(obj, "host_device_ura")?;
                validate_ura(host_device_ura, "host_device_ura")?;
                Ok(Self {
                    sdk_kind: "directory.owner_projection_changed",
                    daemon_event_type: "owner_projection_changed",
                    lifecycle: "delta",
                    occurred_unix_ms: required_nonnegative_i64(obj, "unix_ms")?,
                    subject_ura: Some(owner_ura.to_string()),
                })
            }
            other => Err(EventsError::InvalidField(
                "type",
                format!("unknown DirectoryEvent type {other:?}"),
            )),
        }
    }
}

#[derive(Debug, Clone)]
struct EventFrame {
    stream: String,
    kind: String,
    lifecycle: &'static str,
    event_id: String,
    cursor: EventCursor,
    resume_token: String,
    occurred_unix_ms: i64,
    subject_ref: Value,
    tenant_ref: Value,
    payload: Option<Value>,
    dropped_count: u64,
    reconnect_after_ms: Option<u64>,
    terminal: bool,
    daemon_event_type: Option<&'static str>,
    metadata_extra: Value,
}

impl EventFrame {
    fn directory(
        frame_kind: DirectoryFrameKind,
        cursor: EventCursor,
        event_id: String,
        resume_token: String,
        input: &Map<String, Value>,
        payload: Option<Value>,
    ) -> Result<Self, EventsError> {
        let subject_ref = match frame_kind.subject_ura.as_deref() {
            Some(ura) => typed_ura_ref(ura, "subject_ref")?,
            None => Value::Null,
        };
        let tenant_ref = tenant_ref_from_input(input, frame_kind.subject_ura.as_deref())?;
        Ok(Self {
            stream: DIRECTORY_STREAM.to_string(),
            kind: frame_kind.sdk_kind.to_string(),
            lifecycle: frame_kind.lifecycle,
            event_id,
            cursor,
            resume_token,
            occurred_unix_ms: frame_kind.occurred_unix_ms,
            subject_ref,
            tenant_ref,
            payload,
            dropped_count: 0,
            reconnect_after_ms: None,
            terminal: false,
            daemon_event_type: Some(frame_kind.daemon_event_type),
            metadata_extra: Value::Null,
        })
    }

    fn to_json(self) -> Result<Value, EventsError> {
        let mut metadata = Map::new();
        metadata.insert(
            "profile".to_string(),
            Value::String(EVENTS_PROFILE.to_string()),
        );
        metadata.insert(
            "stream".to_string(),
            Value::String(DIRECTORY_STREAM.to_string()),
        );
        metadata.insert(
            "carrier_owner".to_string(),
            Value::String("daemon_sdk".to_string()),
        );
        metadata.insert(
            "source".to_string(),
            Value::String("daemon_directory_event".to_string()),
        );
        metadata.insert(
            "stream_ability".to_string(),
            Value::String(DIRECTORY_ABILITY.to_string()),
        );
        metadata.insert(
            "lifecycle".to_string(),
            Value::String(self.lifecycle.to_string()),
        );
        if let Some(event_type) = self.daemon_event_type {
            metadata.insert(
                "daemon_event_type".to_string(),
                Value::String(event_type.to_string()),
            );
        }
        if let Value::Object(extra) = self.metadata_extra {
            for (key, value) in extra {
                metadata.insert(key, value);
            }
        }

        Ok(json!({
            "profile": EVENTS_PROFILE,
            "stream": self.stream,
            "kind": self.kind,
            "event_id": self.event_id,
            "cursor": self.cursor.to_json(),
            "resume_token": self.resume_token,
            "occurred_unix_ms": self.occurred_unix_ms,
            "occurred_at": unix_ms_to_rfc3339(self.occurred_unix_ms),
            "subject_ref": self.subject_ref,
            "tenant_ref": self.tenant_ref,
            "payload": self.payload.unwrap_or(Value::Null),
            "dropped_count": self.dropped_count,
            "reconnect_after_ms": self.reconnect_after_ms,
            "terminal": self.terminal,
            "metadata": metadata,
        }))
    }
}

fn typed_ura_ref(raw: &str, field: &'static str) -> Result<Value, EventsError> {
    let parsed =
        ura::parse_ura(raw).map_err(|err| EventsError::InvalidField(field, err.to_string()))?;
    Ok(json!({
        "kind": "ura",
        "ura": raw,
        "role": ura_role(parsed.kind),
    }))
}

fn tenant_ref_from_input(
    obj: &Map<String, Value>,
    subject_ura: Option<&str>,
) -> Result<Value, EventsError> {
    if let Some(value) = obj.get("tenant_ref").filter(|value| !value.is_null()) {
        if value.as_str().is_some_and(|raw| raw.trim().is_empty()) {
            return Err(EventsError::InvalidField(
                "tenant_ref",
                "must not be empty".to_string(),
            ));
        }
        return Ok(value.clone());
    }
    let Some(subject_ura) = subject_ura else {
        return Ok(Value::Null);
    };
    let parsed = ura::parse_ura(subject_ura)
        .map_err(|err| EventsError::InvalidField("subject_ref", err.to_string()))?;
    Ok(json!({
        "kind": "realm",
        "realm": parsed.realm,
    }))
}

fn ura_role(kind: ura::URAKind) -> &'static str {
    match kind {
        ura::URAKind::User => "user",
        ura::URAKind::Device => "device",
        ura::URAKind::Agent => "agent",
        ura::URAKind::Ability => "ability",
        ura::URAKind::Hub => "hub",
        ura::URAKind::Resource => "resource",
        _ => "unknown",
    }
}

fn validate_token(raw: &str, field: &'static str) -> Result<(), EventsError> {
    if raw.trim().is_empty() {
        return Err(EventsError::InvalidField(
            field,
            "must not be empty".to_string(),
        ));
    }
    if raw.chars().any(char::is_whitespace) || raw.chars().any(char::is_control) {
        return Err(EventsError::InvalidField(
            field,
            "must not contain whitespace or control characters".to_string(),
        ));
    }
    Ok(())
}

fn validate_heartbeat_interval_ms(raw: u64) -> Result<(), EventsError> {
    if !(MIN_HEARTBEAT_INTERVAL_MS..=MAX_HEARTBEAT_INTERVAL_MS).contains(&raw) {
        return Err(EventsError::InvalidField(
            "heartbeat_interval_ms",
            format!("must be between {MIN_HEARTBEAT_INTERVAL_MS} and {MAX_HEARTBEAT_INTERVAL_MS}"),
        ));
    }
    Ok(())
}

fn validate_reconnect_after_ms(raw: u64, field: &'static str) -> Result<u64, EventsError> {
    if raw > MAX_HEARTBEAT_INTERVAL_MS {
        return Err(EventsError::InvalidField(
            field,
            format!("must be at most {MAX_HEARTBEAT_INTERVAL_MS}"),
        ));
    }
    Ok(raw)
}

fn required_u64(obj: &Map<String, Value>, field: &'static str) -> Result<u64, EventsError> {
    optional_u64(obj, field).ok_or(EventsError::MissingField(field))
}

fn optional_u64(obj: &Map<String, Value>, field: &'static str) -> Option<u64> {
    match obj.get(field) {
        Some(Value::Number(number)) => number.as_u64(),
        Some(Value::String(raw)) => raw.trim().parse::<u64>().ok(),
        _ => None,
    }
}

fn required_nonnegative_i64(
    obj: &Map<String, Value>,
    field: &'static str,
) -> Result<i64, EventsError> {
    let value = match obj.get(field) {
        Some(Value::Number(number)) => number
            .as_i64()
            .or_else(|| number.as_u64().and_then(|value| i64::try_from(value).ok())),
        Some(Value::String(raw)) => raw.trim().parse::<i64>().ok(),
        _ => None,
    }
    .ok_or(EventsError::MissingField(field))?;
    if value < 0 {
        return Err(EventsError::InvalidField(
            field,
            "must be non-negative".to_string(),
        ));
    }
    Ok(value)
}

fn unix_ms_to_rfc3339(ms: i64) -> String {
    chrono::Utc
        .timestamp_millis_opt(ms)
        .single()
        .unwrap_or_else(|| chrono::Utc.timestamp_millis_opt(0).single().unwrap())
        .to_rfc3339_opts(chrono::SecondsFormat::Millis, true)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn base_request(extra: Value) -> Value {
        let mut request = json!({
            "caller_ura": "easynet:///r/example/agent/alice.sdk",
            "callee_ura": "easynet:///r/example/device/dev-a",
            "subject_ura": "easynet:///r/example/device/dev-a",
            "descriptor_version": "1.0.0",
            "nonce_base64": "AQIDBAUGBwgJCgsMDQ4PEA==",
            "causal_context": {"form": "none"},
        });
        let Value::Object(extra) = extra else {
            return request;
        };
        let obj = request.as_object_mut().unwrap();
        for (key, value) in extra {
            obj.insert(key, value);
        }
        request
    }

    #[test]
    fn directory_subscription_builds_complete_stream_invocation() {
        let request = base_request(json!({
            "realm": "example",
            "agent_ura": "easynet:///r/example/agent/alice.main",
            "resume_cursor": {"stream": "directory", "sequence": 7},
            "heartbeat_interval_ms": 30_000
        }));

        let carrier = build_directory_subscription_invocation(&request).unwrap();

        assert_eq!(
            carrier["metadata"]["system_ability"],
            "federation.subscribe_directory_v2"
        );
        assert_eq!(carrier["metadata"]["profile"], "events");
        assert_eq!(carrier["args"]["stream"], "directory");
        assert_eq!(carrier["args"]["resume_cursor"], "directory:7");
        assert_eq!(
            carrier["descriptor_ref"],
            "easynet:///r/example/ability/device.dev-a.federation.subscribe_directory_v2@1.0.0"
        );
    }

    #[test]
    fn directory_event_projects_agent_delta_with_typed_refs() {
        let input = json!({
            "cursor": {"stream": "directory", "sequence": 8},
            "event": {
                "type": "agent_advertised",
                "agent_ura": "easynet:///r/example/agent/alice.main",
                "signing_authority": "self_signed",
                "replaced_prior": false,
                "unix_ms": 1783100000123i64
            }
        });

        let frame = project_directory_event(&input).unwrap();

        assert_eq!(frame["kind"], "directory.agent_advertised");
        assert_eq!(frame["cursor"]["token"], "directory:8");
        assert_eq!(frame["resume_token"], "directory:8");
        assert_eq!(frame["subject_ref"]["role"], "agent");
        assert_eq!(frame["tenant_ref"]["realm"], "example");
        assert_eq!(frame["metadata"]["lifecycle"], "delta");
        assert_eq!(frame["terminal"], false);
    }

    #[test]
    fn directory_event_requires_explicit_cursor() {
        let input = json!({
            "event": {
                "type": "heartbeat",
                "unix_ms": 1783100000123i64
            }
        });

        let err = project_directory_event(&input).unwrap_err();

        assert_eq!(err.to_string(), "missing required field cursor");
    }

    #[test]
    fn directory_event_rejects_unknown_variant() {
        let input = json!({
            "cursor": "directory:9",
            "event": {
                "type": "agent_changed",
                "unix_ms": 1783100000123i64
            }
        });

        let err = project_directory_event(&input).unwrap_err();

        assert!(err.to_string().contains("unknown DirectoryEvent type"));
    }

    #[test]
    fn drop_report_is_first_class_nonterminal_frame() {
        let frame = project_drop_report(&json!({
            "cursor": "directory:10",
            "occurred_unix_ms": 1783100000123i64,
            "dropped_count": 4,
            "reason": "queue_overflow"
        }))
        .unwrap();

        assert_eq!(frame["kind"], "directory.drop_report");
        assert_eq!(frame["dropped_count"], 4);
        assert_eq!(frame["payload"]["reason"], "queue_overflow");
        assert_eq!(frame["metadata"]["reason"], "queue_overflow");
        assert_eq!(frame["reconnect_after_ms"], DEFAULT_RECONNECT_AFTER_MS);
        assert_eq!(frame["terminal"], false);
    }

    #[test]
    fn terminal_frame_is_explicit_final_state() {
        let frame = project_terminal(&json!({
            "cursor": "directory:11",
            "occurred_unix_ms": 1783100000123i64,
            "reason": "client_closed"
        }))
        .unwrap();

        assert_eq!(frame["kind"], "directory.terminal");
        assert_eq!(frame["terminal"], true);
        assert_eq!(frame["payload"]["reason"], "client_closed");
    }
}
