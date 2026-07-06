// EasyNet CLI — Events shared contract
// =====================================
//
// File: src/protocol/events_contract.rs
// Description: Shared daemon SDK contract for Events profile stream Invocation
//              carriers and typed event-frame projection.
//
// Protocol Responsibility
// -----------------------
// Own the EasyNet-Cli SDK Events DTO projection for daemon stream frames.
// Event production remains daemon-owned through governed abilities; this module
// does not create a second event bus, perform backend fanout, or execute stream
// I/O.
//
// Implementation Approach
// -----------------------
// Reuse the shared daemon SDK Invocation carrier builder for stream abilities,
// and project daemon JSON payloads into binding-facing `EventFrame` values.
// Cursor, drop-report, and terminal semantics are represented as explicit value
// objects rather than ad hoc string rewriting in each exported function.
//
// Usage Contract
// --------------
// Live event projection requires an explicit cursor supplied by the stream
// reader. Daemon raw event payloads carry event facts, not resume state; the SDK
// must not infer cursor positions from array indexes or timestamps.
//
// Architectural Position
// ----------------------
// EasyNet-Cli daemon SDK Events profile. Runtime Core remains the only stream
// open/close path; this profile owns carrier construction and typed frame DTOs
// for language bindings.

use chrono::TimeZone;
use serde_json::{json, Map, Value};

use crate::core::ura;
use crate::protocol::sdk_contract::{
    build_system_invocation, object, optional_string, optional_string_field, required_string,
    validate_ura, SdkContractError,
};

const EVENTS_PROFILE: &str = "events";
const DIRECTORY_STREAM: &str = "directory";
const DEVICE_STREAM: &str = "device";
const SESSION_STREAM: &str = "session";
const INVOCATION_STREAM: &str = "invocation";
const DIRECTORY_ABILITY: &str =
    crate::daemon::ability::conformance::ABILITY_FEDERATION_SUBSCRIBE_DIRECTORY_V2;
const SESSION_ATTACH_ABILITY: &str = crate::daemon::ability::names::device_control::SESSION_ATTACH;
const DEVICE_SUBSCRIBE_ABILITY: &str = "events.device.subscribe";
const INVOCATION_SUBSCRIBE_ABILITY: &str = "events.invocation.subscribe";
const DEVICE_HISTORY_ABILITY: &str = "events.device.history";
const DEFAULT_RECONNECT_AFTER_MS: u64 = 1_000;
const MIN_HEARTBEAT_INTERVAL_MS: u64 = 1_000;
const MAX_HEARTBEAT_INTERVAL_MS: u64 = 300_000;
const DEFAULT_EVENT_PAGE_SIZE: usize = 50;
const MAX_EVENT_PAGE_SIZE: usize = 500;

pub(crate) type EventsError = SdkContractError;

pub(crate) fn build_directory_subscription_invocation(
    request: &Value,
) -> Result<Value, EventsError> {
    let obj = object(request, "EventsDirectorySubscriptionRequest")?;
    let args = directory_subscription_args(obj)?;
    build_system_invocation(obj, EVENTS_PROFILE, DIRECTORY_ABILITY, args)
}

pub(crate) fn build_device_subscription_invocation(request: &Value) -> Result<Value, EventsError> {
    let obj = object(request, "EventsDeviceSubscriptionRequest")?;
    let args = device_subscription_args(obj)?;
    build_system_invocation(obj, EVENTS_PROFILE, DEVICE_SUBSCRIBE_ABILITY, args)
}

pub(crate) fn build_session_subscription_invocation(request: &Value) -> Result<Value, EventsError> {
    let obj = object(request, "EventsSessionSubscriptionRequest")?;
    let args = session_subscription_args(obj)?;
    build_system_invocation(obj, EVENTS_PROFILE, SESSION_ATTACH_ABILITY, args)
}

pub(crate) fn build_invocation_subscription_invocation(
    request: &Value,
) -> Result<Value, EventsError> {
    let obj = object(request, "EventsInvocationSubscriptionRequest")?;
    let args = invocation_subscription_args(obj)?;
    build_system_invocation(obj, EVENTS_PROFILE, INVOCATION_SUBSCRIBE_ABILITY, args)
}

pub(crate) fn build_device_event_history_invocation(request: &Value) -> Result<Value, EventsError> {
    let obj = object(request, "EventsDeviceEventListRequest")?;
    let controls = PageControls::from_request(obj)?;
    let filter = EventFilter::from_request(obj)?;
    let mut args = Map::new();
    args.insert(
        "stream".to_string(),
        Value::String(DEVICE_STREAM.to_string()),
    );
    args.insert("limit".to_string(), Value::Number(controls.limit.into()));
    if let Some(device_ura) = filter.device_ura {
        validate_device_ura(&device_ura, "device_ura")?;
        args.insert("device_ura".to_string(), Value::String(device_ura));
    }
    if let Some(cursor) = optional_string(obj, "cursor") {
        let cursor = EventCursor::parse_token(&cursor, "cursor")?;
        cursor.require_stream(DEVICE_STREAM, "cursor")?;
        args.insert("cursor".to_string(), Value::String(cursor.resume_token()));
    }
    build_system_invocation(
        obj,
        EVENTS_PROFILE,
        DEVICE_HISTORY_ABILITY,
        Value::Object(args),
    )
}

pub(crate) fn project_device_event_page(input: &Value) -> Result<Value, EventsError> {
    let input = object(input, "EventsDeviceEventPageInput")?;
    let controls = PageControls::from_request(input)?;
    let fallback_result = Value::Object(input.clone());
    let result = input
        .get("result")
        .filter(|value| !value.is_null())
        .unwrap_or(&fallback_result);
    let rows = event_rows(result)?;
    let page = controls.slice(rows)?;
    let mut items = Vec::with_capacity(page.rows.len());
    for row in page.rows {
        items.push(project_device_event_row(row)?);
    }
    let has_more = page.next_cursor.is_some();
    Ok(json!({
        "profile": EVENTS_PROFILE,
        "stream": DEVICE_STREAM,
        "item_kind": "device_event",
        "items": items,
        "next_cursor": page.next_cursor,
        "has_more": has_more,
        "limit": controls.limit,
        "metadata": {
            "profile": EVENTS_PROFILE,
            "source": "device_event_history",
            "source_ability": DEVICE_HISTORY_ABILITY,
            "total_items": rows.len(),
        },
    }))
}

pub(crate) fn project_directory_event(input: &Value) -> Result<Value, EventsError> {
    let obj = object(input, "EventsDirectoryEventInput")?;
    let event = obj.get("event").unwrap_or(input);
    let event_obj = object(event, "DirectoryEvent")?;
    let cursor = EventCursor::from_input(obj)?;
    cursor.require_stream(DIRECTORY_STREAM, "cursor")?;
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

pub(crate) fn project_live_event(input: &Value) -> Result<Value, EventsError> {
    let obj = object(input, "EventsLiveEventInput")?;
    let cursor = EventCursor::from_input(obj)?;
    match cursor.stream.as_str() {
        DIRECTORY_STREAM => project_directory_event(input),
        DEVICE_STREAM => project_device_live_event(obj, cursor),
        INVOCATION_STREAM => project_invocation_live_event(obj, cursor),
        SESSION_STREAM => Err(EventsError::InvalidField(
            "cursor",
            "session live event projection is not part of the Events profile contract"
                .to_string(),
        )),
        _ => Err(EventsError::InvalidField(
            "cursor",
            "unsupported Events profile stream".to_string(),
        )),
    }
}

pub(crate) fn project_terminal(input: &Value) -> Result<Value, EventsError> {
    let obj = object(input, "EventsTerminalInput")?;
    let cursor = EventCursor::from_input(obj)?;
    cursor.require_stream(DIRECTORY_STREAM, "cursor")?;
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
    cursor.require_stream(DIRECTORY_STREAM, "cursor")?;
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
    validate_stream_field(obj, DIRECTORY_STREAM)?;
    let filter = EventFilter::from_request(obj)?;
    let mut args = Map::new();
    args.insert(
        "stream".to_string(),
        Value::String(DIRECTORY_STREAM.to_string()),
    );
    args.insert(
        "daemon_ability".to_string(),
        Value::String(DIRECTORY_ABILITY.to_string()),
    );

    if let Some(realm) = filter.realm {
        validate_token(&realm, "realm")?;
        args.insert("realm".to_string(), Value::String(realm));
    }
    if let Some(owner_ura) = filter.owner_ura {
        validate_ura(&owner_ura, "owner_ura")?;
        args.insert("owner_ura".to_string(), Value::String(owner_ura));
    }
    if let Some(device_ura) = filter.device_ura {
        validate_device_ura(&device_ura, "device_ura")?;
        args.insert("device_ura".to_string(), Value::String(device_ura));
    }
    if let Some(agent_ura) = filter.agent_ura {
        validate_ura(&agent_ura, "agent_ura")?;
        args.insert("agent_ura".to_string(), Value::String(agent_ura));
    }
    if let Some(value) = obj.get("resume_cursor").filter(|value| !value.is_null()) {
        let cursor = EventCursor::parse(value, "resume_cursor")?;
        cursor.require_stream(DIRECTORY_STREAM, "resume_cursor")?;
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

fn device_subscription_args(obj: &Map<String, Value>) -> Result<Value, EventsError> {
    validate_stream_field(obj, DEVICE_STREAM)?;
    let filter = EventFilter::from_request(obj)?;
    let mut args = subscription_common_args(obj, DEVICE_STREAM, DEVICE_SUBSCRIBE_ABILITY)?;
    if let Some(device_ura) = filter.device_ura {
        validate_device_ura(&device_ura, "device_ura")?;
        args.insert("device_ura".to_string(), Value::String(device_ura));
    }
    if let Some(owner_ura) = filter.owner_ura {
        validate_ura(&owner_ura, "owner_ura")?;
        args.insert("owner_ura".to_string(), Value::String(owner_ura));
    }
    if let Some(agent_ura) = filter.agent_ura {
        validate_ura(&agent_ura, "agent_ura")?;
        args.insert("agent_ura".to_string(), Value::String(agent_ura));
    }
    Ok(Value::Object(args))
}

fn session_subscription_args(obj: &Map<String, Value>) -> Result<Value, EventsError> {
    validate_stream_field(obj, SESSION_STREAM)?;
    if obj
        .get("session_ura")
        .filter(|value| !value.is_null())
        .is_some()
    {
        return Err(EventsError::InvalidField(
            "session_ura",
            "session.attach requires explicit daemon session_id".to_string(),
        ));
    }
    let filter = EventFilter::from_request(obj)?;
    let session_id = filter
        .session_id
        .ok_or(EventsError::MissingField("session_id"))?;
    validate_token(&session_id, "session_id")?;
    let mut args = Map::new();
    args.insert("session_id".to_string(), Value::String(session_id));
    if let Some(value) = obj.get("resume_cursor").filter(|value| !value.is_null()) {
        let cursor = EventCursor::parse(value, "resume_cursor")?;
        cursor.require_stream(SESSION_STREAM, "resume_cursor")?;
        args.insert(
            "since_seq".to_string(),
            Value::Number(cursor.sequence.into()),
        );
    }
    Ok(Value::Object(args))
}

fn invocation_subscription_args(obj: &Map<String, Value>) -> Result<Value, EventsError> {
    validate_stream_field(obj, INVOCATION_STREAM)?;
    let filter = EventFilter::from_request(obj)?;
    let invocation_id = filter
        .invocation_id
        .ok_or(EventsError::MissingField("invocation_id"))?;
    validate_token(&invocation_id, "invocation_id")?;
    let mut args = subscription_common_args(obj, INVOCATION_STREAM, INVOCATION_SUBSCRIBE_ABILITY)?;
    args.insert("invocation_id".to_string(), Value::String(invocation_id));
    Ok(Value::Object(args))
}

#[derive(Debug, Clone, Default, PartialEq, Eq)]
struct EventFilter {
    realm: Option<String>,
    owner_ura: Option<String>,
    device_ura: Option<String>,
    agent_ura: Option<String>,
    session_id: Option<String>,
    invocation_id: Option<String>,
}

impl EventFilter {
    fn from_request(obj: &Map<String, Value>) -> Result<Self, EventsError> {
        let nested = match obj.get("filter") {
            None | Some(Value::Null) => None,
            Some(Value::Object(filter)) => {
                for key in filter.keys() {
                    if !matches!(
                        key.as_str(),
                        "realm"
                            | "owner_ura"
                            | "device_ura"
                            | "agent_ura"
                            | "session_id"
                            | "invocation_id"
                    ) {
                        return Err(EventsError::InvalidField(
                            "filter",
                            format!("unsupported event filter field {key:?}"),
                        ));
                    }
                }
                Some(filter)
            }
            Some(_) => {
                return Err(EventsError::InvalidField(
                    "filter",
                    "must be an object or null".to_string(),
                ))
            }
        };
        Ok(Self {
            realm: merged_filter_string(obj, nested, "realm")?,
            owner_ura: merged_filter_string(obj, nested, "owner_ura")?,
            device_ura: merged_filter_string(obj, nested, "device_ura")?,
            agent_ura: merged_filter_string(obj, nested, "agent_ura")?,
            session_id: merged_filter_string(obj, nested, "session_id")?,
            invocation_id: merged_filter_string(obj, nested, "invocation_id")?,
        })
    }
}

fn merged_filter_string(
    obj: &Map<String, Value>,
    nested: Option<&Map<String, Value>>,
    field: &'static str,
) -> Result<Option<String>, EventsError> {
    let top = optional_string_field(obj, field)?;
    let nested = nested
        .map(|filter| optional_string_field(filter, field))
        .transpose()?
        .flatten();
    if let (Some(top), Some(nested)) = (&top, &nested) {
        if top != nested {
            return Err(EventsError::InvalidField(
                field,
                "top-level field conflicts with filter field".to_string(),
            ));
        }
    }
    Ok(nested.or(top))
}

fn subscription_common_args(
    obj: &Map<String, Value>,
    stream: &'static str,
    daemon_ability: &'static str,
) -> Result<Map<String, Value>, EventsError> {
    let mut args = Map::new();
    args.insert("stream".to_string(), Value::String(stream.to_string()));
    args.insert(
        "daemon_ability".to_string(),
        Value::String(daemon_ability.to_string()),
    );
    if let Some(value) = obj.get("resume_cursor").filter(|value| !value.is_null()) {
        let cursor = EventCursor::parse(value, "resume_cursor")?;
        cursor.require_stream(stream, "resume_cursor")?;
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
    Ok(args)
}

fn validate_stream_field(
    obj: &Map<String, Value>,
    expected: &'static str,
) -> Result<(), EventsError> {
    if let Some(stream) = optional_string(obj, "stream") {
        validate_token(&stream, "stream")?;
        if stream != expected {
            return Err(EventsError::InvalidField(
                "stream",
                format!("expected {expected:?}, got {stream:?}"),
            ));
        }
    }
    Ok(())
}

fn validate_device_ura(raw: &str, field: &'static str) -> Result<(), EventsError> {
    let parsed =
        ura::parse_ura(raw).map_err(|err| EventsError::InvalidField(field, err.to_string()))?;
    if parsed.kind != ura::URAKind::Device {
        return Err(EventsError::InvalidField(
            field,
            format!("must be a Device URA, got {}", parsed.kind),
        ));
    }
    Ok(())
}

#[derive(Debug, Clone, Copy)]
struct PageControls {
    limit: usize,
    offset: usize,
}

impl PageControls {
    fn from_request(obj: &Map<String, Value>) -> Result<Self, EventsError> {
        let limit = optional_usize(obj, "limit")?.unwrap_or(DEFAULT_EVENT_PAGE_SIZE);
        if limit == 0 || limit > MAX_EVENT_PAGE_SIZE {
            return Err(EventsError::InvalidField(
                "limit",
                format!("must be between 1 and {MAX_EVENT_PAGE_SIZE}"),
            ));
        }
        let offset = optional_cursor_offset(obj, "cursor")?.unwrap_or(0);
        Ok(Self { limit, offset })
    }

    fn slice<'a, T>(&self, rows: &'a [T]) -> Result<PageSlice<'a, T>, EventsError> {
        if self.offset > rows.len() {
            return Err(EventsError::InvalidField(
                "cursor",
                "must not point past the current event snapshot".to_string(),
            ));
        }
        let end = self.offset.saturating_add(self.limit).min(rows.len());
        let next_cursor = if end < rows.len() {
            Some(format!("{DEVICE_STREAM}:{end}"))
        } else {
            None
        };
        Ok(PageSlice {
            rows: &rows[self.offset..end],
            next_cursor,
        })
    }
}

struct PageSlice<'a, T> {
    rows: &'a [T],
    next_cursor: Option<String>,
}

fn event_rows(input: &Value) -> Result<&Vec<Value>, EventsError> {
    let obj = object(input, "DeviceEventRows")?;
    obj.get("events")
        .or_else(|| obj.get("items"))
        .and_then(Value::as_array)
        .ok_or_else(|| EventsError::InvalidField("events", "must be an array".to_string()))
}

fn project_device_event_row(row: &Value) -> Result<Value, EventsError> {
    let obj = object(row, "DeviceEventRow")?;
    if obj.get("profile").and_then(Value::as_str) == Some(EVENTS_PROFILE) {
        let stream = required_string(obj, "stream")?;
        if stream != DEVICE_STREAM {
            return Err(EventsError::InvalidField(
                "stream",
                format!("expected {DEVICE_STREAM:?}, got {stream:?}"),
            ));
        }
        let cursor = EventCursor::parse(
            obj.get("cursor")
                .ok_or(EventsError::MissingField("cursor"))?,
            "cursor",
        )?;
        cursor.require_stream(DEVICE_STREAM, "cursor")?;
        return Ok(Value::Object(obj.clone()));
    }
    let sequence = required_u64(obj, "sequence")?;
    let cursor = EventCursor::new(DEVICE_STREAM, sequence, "sequence")?;
    let device_ura = required_string(obj, "device_ura")?;
    validate_device_ura(device_ura, "device_ura")?;
    let occurred_unix_ms = required_nonnegative_i64(obj, "occurred_unix_ms")?;
    let kind = optional_string(obj, "kind").unwrap_or_else(|| "device.event".to_string());
    let payload = obj.get("payload").cloned().unwrap_or_else(|| json!({}));
    Ok(json!({
        "profile": EVENTS_PROFILE,
        "stream": DEVICE_STREAM,
        "kind": kind,
        "event_id": optional_string(obj, "event_id").unwrap_or_else(|| cursor.event_id()),
        "cursor": cursor.to_json(),
        "resume_token": optional_string(obj, "resume_token").unwrap_or_else(|| cursor.resume_token()),
        "occurred_unix_ms": occurred_unix_ms,
        "occurred_at": unix_ms_to_rfc3339(occurred_unix_ms),
        "subject_ref": typed_ura_ref(device_ura, "device_ura")?,
        "tenant_ref": tenant_ref_from_input(obj, Some(device_ura))?,
        "payload": payload,
        "dropped_count": 0,
        "reconnect_after_ms": Value::Null,
        "terminal": false,
        "metadata": {
            "profile": EVENTS_PROFILE,
            "stream": DEVICE_STREAM,
            "carrier_owner": "daemon_sdk",
            "source": "daemon_device_event",
            "stream_ability": DEVICE_HISTORY_ABILITY,
            "lifecycle": "history",
        },
    }))
}

fn project_device_live_event(
    input: &Map<String, Value>,
    cursor: EventCursor,
) -> Result<Value, EventsError> {
    cursor.require_stream(DEVICE_STREAM, "cursor")?;
    let event = input.get("event").ok_or(EventsError::MissingField("event"))?;
    let event_obj = object(event, "DeviceEvent")?;
    reject_sequence_mismatch(event_obj, &cursor)?;
    let device_ura = required_string(event_obj, "device_ura")?;
    validate_device_ura(device_ura, "device_ura")?;
    let occurred_unix_ms = event_unix_ms(event_obj)?;
    let kind = optional_string(event_obj, "kind").unwrap_or_else(|| "device.event".to_string());
    let payload = event_obj
        .get("payload")
        .cloned()
        .unwrap_or_else(|| Value::Object(event_obj.clone()));
    EventFrame {
        stream: DEVICE_STREAM.to_string(),
        kind,
        lifecycle: "live",
        event_id: optional_string(input, "event_id").unwrap_or_else(|| cursor.event_id()),
        cursor: cursor.clone(),
        resume_token: optional_string(input, "resume_token")
            .unwrap_or_else(|| cursor.resume_token()),
        occurred_unix_ms,
        subject_ref: typed_ura_ref(device_ura, "device_ura")?,
        tenant_ref: tenant_ref_from_input(input, Some(device_ura))?,
        payload: Some(payload),
        dropped_count: 0,
        reconnect_after_ms: None,
        terminal: false,
        daemon_event_type: optional_string(event_obj, "type"),
        metadata_extra: Value::Null,
    }
    .to_json()
}

fn project_invocation_live_event(
    input: &Map<String, Value>,
    cursor: EventCursor,
) -> Result<Value, EventsError> {
    cursor.require_stream(INVOCATION_STREAM, "cursor")?;
    let event = input.get("event").ok_or(EventsError::MissingField("event"))?;
    let event_obj = object(event, "InvocationEvent")?;
    reject_sequence_mismatch(event_obj, &cursor)?;
    let invocation_id = required_string(event_obj, "invocation_id")?;
    validate_token(invocation_id, "invocation_id")?;
    let occurred_unix_ms = event_unix_ms(event_obj)?;
    let kind =
        optional_string(event_obj, "kind").unwrap_or_else(|| "invocation.event".to_string());
    let payload = event_obj
        .get("payload")
        .cloned()
        .unwrap_or_else(|| Value::Object(event_obj.clone()));
    EventFrame {
        stream: INVOCATION_STREAM.to_string(),
        kind,
        lifecycle: "live",
        event_id: optional_string(input, "event_id").unwrap_or_else(|| cursor.event_id()),
        cursor: cursor.clone(),
        resume_token: optional_string(input, "resume_token")
            .unwrap_or_else(|| cursor.resume_token()),
        occurred_unix_ms,
        subject_ref: invocation_ref(invocation_id),
        tenant_ref: tenant_ref_from_input(input, None)?,
        payload: Some(payload),
        dropped_count: 0,
        reconnect_after_ms: None,
        terminal: false,
        daemon_event_type: optional_string(event_obj, "type"),
        metadata_extra: Value::Null,
    }
    .to_json()
}

fn reject_sequence_mismatch(
    event: &Map<String, Value>,
    cursor: &EventCursor,
) -> Result<(), EventsError> {
    let Some(sequence) = optional_u64(event, "sequence") else {
        return Ok(());
    };
    if sequence == cursor.sequence {
        return Ok(());
    }
    Err(EventsError::InvalidField(
        "sequence",
        "event sequence must match stream cursor sequence".to_string(),
    ))
}

fn event_unix_ms(event: &Map<String, Value>) -> Result<i64, EventsError> {
    if event.get("occurred_unix_ms").is_some() {
        return required_nonnegative_i64(event, "occurred_unix_ms");
    }
    required_nonnegative_i64(event, "unix_ms")
}

fn invocation_ref(invocation_id: &str) -> Value {
    json!({
        "kind": "invocation",
        "invocation_id": invocation_id,
    })
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
        if ![
            DIRECTORY_STREAM,
            DEVICE_STREAM,
            SESSION_STREAM,
            INVOCATION_STREAM,
        ]
        .contains(&stream)
        {
            return Err(EventsError::InvalidField(
                field,
                format!("unsupported stream {stream:?}; expected an Events profile stream"),
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

    fn require_stream(
        &self,
        expected: &'static str,
        field: &'static str,
    ) -> Result<(), EventsError> {
        if self.stream == expected {
            return Ok(());
        }
        Err(EventsError::InvalidField(
            field,
            format!("expected {expected:?}, got {:?}", self.stream),
        ))
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
    daemon_event_type: Option<String>,
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
            daemon_event_type: Some(frame_kind.daemon_event_type.to_string()),
            metadata_extra: Value::Null,
        })
    }

    fn to_json(self) -> Result<Value, EventsError> {
        let mut metadata = Map::new();
        metadata.insert(
            "profile".to_string(),
            Value::String(EVENTS_PROFILE.to_string()),
        );
        metadata.insert("stream".to_string(), Value::String(self.stream.clone()));
        metadata.insert(
            "carrier_owner".to_string(),
            Value::String("daemon_sdk".to_string()),
        );
        metadata.insert(
            "source".to_string(),
            Value::String(event_source_for_stream(&self.stream).to_string()),
        );
        metadata.insert(
            "stream_ability".to_string(),
            Value::String(event_stream_ability(&self.stream).to_string()),
        );
        metadata.insert(
            "lifecycle".to_string(),
            Value::String(self.lifecycle.to_string()),
        );
        if let Some(event_type) = self.daemon_event_type {
            metadata.insert(
                "daemon_event_type".to_string(),
                Value::String(event_type),
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

fn event_source_for_stream(stream: &str) -> &'static str {
    match stream {
        DIRECTORY_STREAM => "daemon_directory_event",
        DEVICE_STREAM => "daemon_device_event",
        SESSION_STREAM => "daemon_session_event",
        INVOCATION_STREAM => "daemon_invocation_event",
        _ => "daemon_event",
    }
}

fn event_stream_ability(stream: &str) -> &'static str {
    match stream {
        DIRECTORY_STREAM => DIRECTORY_ABILITY,
        DEVICE_STREAM => DEVICE_SUBSCRIBE_ABILITY,
        SESSION_STREAM => SESSION_ATTACH_ABILITY,
        INVOCATION_STREAM => INVOCATION_SUBSCRIBE_ABILITY,
        _ => "events.unknown",
    }
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

fn optional_usize(
    obj: &Map<String, Value>,
    field: &'static str,
) -> Result<Option<usize>, EventsError> {
    match obj.get(field) {
        None | Some(Value::Null) => Ok(None),
        Some(Value::Number(number)) => number
            .as_u64()
            .and_then(|value| usize::try_from(value).ok())
            .map(Some)
            .ok_or_else(|| EventsError::InvalidField(field, "must be unsigned".to_string())),
        Some(Value::String(raw)) => {
            let trimmed = raw.trim();
            if trimmed.is_empty() {
                return Ok(None);
            }
            trimmed
                .parse::<usize>()
                .map(Some)
                .map_err(|err| EventsError::InvalidField(field, err.to_string()))
        }
        Some(_) => Err(EventsError::InvalidField(
            field,
            "must be an integer or decimal string".to_string(),
        )),
    }
}

fn optional_cursor_offset(
    obj: &Map<String, Value>,
    field: &'static str,
) -> Result<Option<usize>, EventsError> {
    match obj.get(field) {
        None | Some(Value::Null) => Ok(None),
        Some(Value::String(raw)) => {
            let trimmed = raw.trim();
            if trimmed.is_empty() {
                return Ok(None);
            }
            let cursor = EventCursor::parse_token(trimmed, field)?;
            cursor.require_stream(DEVICE_STREAM, field)?;
            usize::try_from(cursor.sequence)
                .map(Some)
                .map_err(|err| EventsError::InvalidField(field, err.to_string()))
        }
        Some(_) => Err(EventsError::InvalidField(
            field,
            "must be a cursor string".to_string(),
        )),
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
    fn directory_subscription_rejects_session_resume_cursor() {
        let request = base_request(json!({
            "resume_cursor": {"stream": "session", "sequence": 7}
        }));

        let err = build_directory_subscription_invocation(&request).unwrap_err();

        assert_eq!(
            err.to_string(),
            "invalid field resume_cursor: expected \"directory\", got \"session\""
        );
    }

    #[test]
    fn session_subscription_builds_session_attach_invocation() {
        let request = base_request(json!({
            "stream": "session",
            "session_id": "run-1",
            "resume_cursor": {"stream": "session", "sequence": 4}
        }));

        let carrier = build_session_subscription_invocation(&request).unwrap();

        assert_eq!(carrier["metadata"]["system_ability"], "session.attach");
        assert_eq!(carrier["metadata"]["profile"], "events");
        assert_eq!(carrier["args"]["session_id"], "run-1");
        assert_eq!(carrier["args"]["since_seq"], 4);
        assert_eq!(
            carrier["descriptor_ref"],
            "easynet:///r/example/ability/device.dev-a.session.attach@1.0.0"
        );
    }

    #[test]
    fn session_subscription_rejects_product_session_ura_facade_parsing() {
        let request = base_request(json!({
            "stream": "session",
            "session_id": "run-1",
            "session_ura": "easynet:///r/example/resource/daemon.browser/run-1"
        }));

        let err = build_session_subscription_invocation(&request).unwrap_err();

        assert_eq!(
            err.to_string(),
            "invalid field session_ura: session.attach requires explicit daemon session_id"
        );
    }

    #[test]
    fn session_subscription_rejects_directory_resume_cursor() {
        let request = base_request(json!({
            "stream": "session",
            "session_id": "run-1",
            "resume_cursor": {"stream": "directory", "sequence": 4}
        }));

        let err = build_session_subscription_invocation(&request).unwrap_err();

        assert_eq!(
            err.to_string(),
            "invalid field resume_cursor: expected \"session\", got \"directory\""
        );
    }

    #[test]
    fn device_subscription_builds_device_event_invocation() {
        let request = base_request(json!({
            "stream": "device",
            "device_ura": "easynet:///r/example/device/dev-a",
            "owner_ura": "easynet:///r/example/device/dev-a",
            "resume_cursor": {"stream": "device", "sequence": 2},
            "heartbeat_interval_ms": 30_000
        }));

        let carrier = build_device_subscription_invocation(&request).unwrap();

        assert_eq!(
            carrier["metadata"]["system_ability"],
            "events.device.subscribe"
        );
        assert_eq!(carrier["args"]["stream"], "device");
        assert_eq!(
            carrier["args"]["device_ura"],
            "easynet:///r/example/device/dev-a"
        );
        assert_eq!(carrier["args"]["resume_cursor"], "device:2");
        assert_eq!(
            carrier["descriptor_ref"],
            "easynet:///r/example/ability/device.dev-a.events.device.subscribe@1.0.0"
        );
    }

    #[test]
    fn events_filter_lowers_to_daemon_args() {
        let request = base_request(json!({
            "stream": "device",
            "filter": {
                "device_ura": "easynet:///r/example/device/dev-a",
                "agent_ura": "easynet:///r/example/agent/alice.main"
            },
            "resume_cursor": {"stream": "device", "sequence": 2}
        }));

        let carrier = build_device_subscription_invocation(&request).unwrap();

        assert_eq!(
            carrier["args"]["device_ura"],
            "easynet:///r/example/device/dev-a"
        );
        assert_eq!(
            carrier["args"]["agent_ura"],
            "easynet:///r/example/agent/alice.main"
        );
        assert!(carrier["args"].as_object().unwrap().get("filter").is_none());
    }

    #[test]
    fn events_filter_conflict_fails_closed() {
        let request = base_request(json!({
            "stream": "device",
            "device_ura": "easynet:///r/example/device/dev-a",
            "filter": {
                "device_ura": "easynet:///r/example/device/dev-b"
            }
        }));

        let err = build_device_subscription_invocation(&request).unwrap_err();

        assert_eq!(
            err.to_string(),
            "invalid field device_ura: top-level field conflicts with filter field"
        );
    }

    #[test]
    fn invocation_subscription_requires_invocation_id() {
        let request = base_request(json!({
            "stream": "invocation"
        }));

        let err = build_invocation_subscription_invocation(&request).unwrap_err();

        assert_eq!(err.to_string(), "missing required field invocation_id");
    }

    #[test]
    fn invocation_subscription_builds_invocation_event_invocation() {
        let request = base_request(json!({
            "stream": "invocation",
            "invocation_id": "inv-1",
            "resume_cursor": "invocation:9"
        }));

        let carrier = build_invocation_subscription_invocation(&request).unwrap();

        assert_eq!(
            carrier["metadata"]["system_ability"],
            "events.invocation.subscribe"
        );
        assert_eq!(carrier["args"]["stream"], "invocation");
        assert_eq!(carrier["args"]["invocation_id"], "inv-1");
        assert_eq!(carrier["args"]["resume_cursor"], "invocation:9");
    }

    #[test]
    fn device_event_history_builds_bounded_page_invocation() {
        let request = base_request(json!({
            "device_ura": "easynet:///r/example/device/dev-a",
            "limit": 25,
            "cursor": "device:10"
        }));

        let carrier = build_device_event_history_invocation(&request).unwrap();

        assert_eq!(
            carrier["metadata"]["system_ability"],
            "events.device.history"
        );
        assert_eq!(carrier["args"]["stream"], "device");
        assert_eq!(
            carrier["args"]["device_ura"],
            "easynet:///r/example/device/dev-a"
        );
        assert_eq!(carrier["args"]["limit"], 25);
        assert_eq!(carrier["args"]["cursor"], "device:10");
    }

    #[test]
    fn device_event_history_projects_bounded_device_page() {
        let page = project_device_event_page(&json!({
            "limit": 1,
            "result": {
                "events": [
                    {
                        "sequence": 8,
                        "device_ura": "easynet:///r/example/device/dev-a",
                        "occurred_unix_ms": 1783100000123i64,
                        "kind": "device.status_changed",
                        "payload": {"state": "online"}
                    },
                    {
                        "sequence": 9,
                        "device_ura": "easynet:///r/example/device/dev-a",
                        "occurred_unix_ms": 1783100001123i64
                    }
                ]
            }
        }))
        .unwrap();

        assert_eq!(page["stream"], "device");
        assert_eq!(page["item_kind"], "device_event");
        assert_eq!(page["items"].as_array().unwrap().len(), 1);
        assert_eq!(page["items"][0]["kind"], "device.status_changed");
        assert_eq!(page["items"][0]["cursor"]["token"], "device:8");
        assert_eq!(page["next_cursor"], "device:1");
        assert_eq!(page["has_more"], true);
    }

    #[test]
    fn device_event_history_rejects_directory_event_frame() {
        let err = project_device_event_page(&json!({
            "result": {
                "events": [{
                    "profile": "events",
                    "stream": "directory",
                    "cursor": {"stream": "directory", "sequence": 8}
                }]
            }
        }))
        .unwrap_err();

        assert_eq!(
            err.to_string(),
            "invalid field stream: expected \"device\", got \"directory\""
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
    fn directory_event_rejects_session_cursor() {
        let input = json!({
            "cursor": {"stream": "session", "sequence": 8},
            "event": {
                "type": "heartbeat",
                "unix_ms": 1783100000123i64
            }
        });

        let err = project_directory_event(&input).unwrap_err();

        assert_eq!(
            err.to_string(),
            "invalid field cursor: expected \"directory\", got \"session\""
        );
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
    fn live_event_projects_device_payload_with_stream_cursor() {
        let frame = project_live_event(&json!({
            "cursor": {"stream": "device", "sequence": 8},
            "event": {
                "sequence": 8,
                "device_ura": "easynet:///r/example/device/dev-a",
                "occurred_unix_ms": 1783100000123i64,
                "kind": "device.status_changed",
                "payload": {"state": "online"}
            }
        }))
        .unwrap();

        assert_eq!(frame["stream"], "device");
        assert_eq!(frame["kind"], "device.status_changed");
        assert_eq!(frame["cursor"]["token"], "device:8");
        assert_eq!(frame["subject_ref"]["role"], "device");
        assert_eq!(frame["metadata"]["stream_ability"], "events.device.subscribe");
        assert_eq!(frame["metadata"]["lifecycle"], "live");
    }

    #[test]
    fn live_event_projects_invocation_payload_without_fabricated_ura() {
        let frame = project_live_event(&json!({
            "cursor": {"stream": "invocation", "sequence": 4},
            "event": {
                "sequence": 4,
                "invocation_id": "inv-1",
                "occurred_unix_ms": 1783100001123i64,
                "kind": "invocation.completed",
                "payload": {"terminal_state": "Completed"}
            }
        }))
        .unwrap();

        assert_eq!(frame["stream"], "invocation");
        assert_eq!(frame["kind"], "invocation.completed");
        assert_eq!(frame["subject_ref"]["kind"], "invocation");
        assert_eq!(frame["subject_ref"]["invocation_id"], "inv-1");
        assert_eq!(
            frame["metadata"]["stream_ability"],
            "events.invocation.subscribe"
        );
    }

    #[test]
    fn live_event_rejects_payload_cursor_sequence_mismatch() {
        let err = project_live_event(&json!({
            "cursor": {"stream": "device", "sequence": 8},
            "event": {
                "sequence": 9,
                "device_ura": "easynet:///r/example/device/dev-a",
                "occurred_unix_ms": 1783100000123i64
            }
        }))
        .unwrap_err();

        assert_eq!(
            err.to_string(),
            "invalid field sequence: event sequence must match stream cursor sequence"
        );
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
    fn drop_report_rejects_session_cursor() {
        let err = project_drop_report(&json!({
            "cursor": "session:10",
            "occurred_unix_ms": 1783100000123i64,
            "dropped_count": 4
        }))
        .unwrap_err();

        assert_eq!(
            err.to_string(),
            "invalid field cursor: expected \"directory\", got \"session\""
        );
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

    #[test]
    fn terminal_frame_rejects_session_cursor() {
        let err = project_terminal(&json!({
            "cursor": "session:11",
            "occurred_unix_ms": 1783100000123i64
        }))
        .unwrap_err();

        assert_eq!(
            err.to_string(),
            "invalid field cursor: expected \"directory\", got \"session\""
        );
    }
}
