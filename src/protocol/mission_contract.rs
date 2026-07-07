// EasyNet CLI — Mission shared contract
// ======================================
//
// File: src/protocol/mission_contract.rs
// Description: Shared daemon SDK contract for Mission/EAL Invocation carriers
//              and typed MissionStatus projection.
//
// Protocol Responsibility
// -----------------------
// Own the EasyNet-Cli SDK Mission DTO projection. Mission/EAL remains daemon
// orchestration; child ability calls remain complete Axon Invocations and
// receipts. This module does not execute EAL, sign envelopes, verify receipts,
// or create a second Mission runtime.
//
// Implementation Approach
// -----------------------
// Reuse the shared daemon SDK carrier builder for system abilities and project
// the existing mission run metadata plus EAL ability graph into one typed
// binding-facing status shape.
//
// Usage Contract
// --------------
// Callers supply explicit Invocation tuple fields for carrier construction.
// Status projection accepts daemon `mission.run`, `mission.track`, or
// `mission.cancel` JSON results and reports only observed child receipt refs;
// missing receipt anchors are represented as absent/null, never fabricated.
//
// Architectural Position
// ----------------------
// EasyNet-Cli daemon SDK Mission profile. Product DSLs may own syntax and EAL
// compilation, but daemon SDK owns submission carriers, typed status, and
// child Invocation/receipt projection.

use std::path::{Component, Path};

use serde_json::{json, Map, Value};

use crate::protocol::sdk_contract::{
    build_system_invocation, object, optional_string, required_string, SdkContractError,
};

const MISSION_PROFILE: &str = "mission";
const SYSTEM_ABILITY_RUN: &str = crate::daemon::ability::names::automation::MISSION_RUN;
const SYSTEM_ABILITY_TRACK: &str = crate::daemon::ability::names::automation::MISSION_TRACK;
const SYSTEM_ABILITY_CANCEL: &str = crate::daemon::ability::names::automation::MISSION_CANCEL;
const SYSTEM_ABILITY_EVENTS: &str = crate::daemon::ability::names::automation::MISSION_EVENTS;

pub(crate) type MissionError = SdkContractError;

pub(crate) fn build_run_eal_invocation(request: &Value) -> Result<Value, MissionError> {
    let obj = object(request, "MissionRunRequest")?;
    let source = required_source(obj)?;
    let mut args = json!({ "source": source });
    if let Some(label) = optional_string(obj, "label") {
        args["label"] = Value::String(label);
    }
    build_system_invocation(obj, MISSION_PROFILE, SYSTEM_ABILITY_RUN, args)
}

pub(crate) fn build_run_file_invocation(request: &Value) -> Result<Value, MissionError> {
    let obj = object(request, "MissionRunFileRequest")?;
    let path = validate_file_path(required_string(obj, "path")?)?;
    let source = std::fs::read_to_string(path).map_err(|err| {
        MissionError::Contract(format!("read EAL source file {}: {err}", path.display()))
    })?;
    if source.trim().is_empty() {
        return Err(MissionError::InvalidField(
            "path",
            "EAL source file must not be empty".to_string(),
        ));
    }
    let label = optional_string(obj, "label").unwrap_or_else(|| path.display().to_string());
    build_system_invocation(
        obj,
        MISSION_PROFILE,
        SYSTEM_ABILITY_RUN,
        json!({
            "source": source,
            "label": label,
        }),
    )
}

pub(crate) fn build_track_invocation(request: &Value) -> Result<Value, MissionError> {
    let obj = object(request, "MissionTrackRequest")?;
    let mission_id = required_mission_id(obj)?;
    build_system_invocation(
        obj,
        MISSION_PROFILE,
        SYSTEM_ABILITY_TRACK,
        json!({ "run_id": mission_id }),
    )
}

pub(crate) fn build_cancel_invocation(request: &Value) -> Result<Value, MissionError> {
    let obj = object(request, "MissionCancelRequest")?;
    let mission_id = required_mission_id(obj)?;
    build_system_invocation(
        obj,
        MISSION_PROFILE,
        SYSTEM_ABILITY_CANCEL,
        json!({ "run_id": mission_id }),
    )
}

pub(crate) fn build_events_invocation(request: &Value) -> Result<Value, MissionError> {
    let obj = object(request, "MissionEventListRequest")?;
    let mission_id = required_mission_id(obj)?;
    let cursor_sequence = optional_i64(obj, "cursor_sequence").unwrap_or(0);
    if cursor_sequence < 0 {
        return Err(MissionError::InvalidField(
            "cursor_sequence",
            "must be non-negative".to_string(),
        ));
    }
    let limit = optional_i64(obj, "limit").unwrap_or(0);
    if limit < 0 {
        return Err(MissionError::InvalidField(
            "limit",
            "must be non-negative".to_string(),
        ));
    }
    if limit > 1000 {
        return Err(MissionError::InvalidField(
            "limit",
            "exceeds bounds".to_string(),
        ));
    }
    let mut args = Map::new();
    args.insert("run_id".to_string(), Value::String(mission_id.to_string()));
    args.insert("cursor_sequence".to_string(), json!(cursor_sequence));
    if limit > 0 {
        args.insert("limit".to_string(), json!(limit));
    }
    build_system_invocation(
        obj,
        MISSION_PROFILE,
        SYSTEM_ABILITY_EVENTS,
        Value::Object(args),
    )
}

pub(crate) fn project_status(input: &Value) -> Result<Value, MissionError> {
    let obj = object(input, "MissionStatusInput")?;
    let meta_value = obj.get("meta").unwrap_or(input);
    let meta_obj = object(meta_value, "MissionRunMeta")?;
    let mission_id = optional_string(obj, "mission_id")
        .or_else(|| optional_string(obj, "run_id"))
        .or_else(|| optional_string(meta_obj, "trace_id"))
        .ok_or(MissionError::MissingField("mission_id"))?;
    validate_mission_id(&mission_id)?;

    let raw_state = optional_string(meta_obj, "status")
        .or_else(|| optional_string(obj, "state"))
        .ok_or(MissionError::MissingField("status"))?;
    let state = normalize_state(&raw_state)?;
    let terminal = mission_state_is_terminal(state);
    let steps_failed = optional_usize(meta_obj, "steps_failed")
        .or_else(|| optional_usize(obj, "partial_failures"))
        .unwrap_or(0);
    let parent_invocation = meta_obj
        .get("invocation_context")
        .filter(|value| !value.is_null())
        .cloned()
        .or_else(|| {
            obj.get("parent_invocation")
                .filter(|value| !value.is_null())
                .cloned()
        });
    let parent_invocation_id = optional_string(obj, "parent_invocation_id")
        .or_else(|| optional_string(meta_obj, "parent_invocation_id"))
        .or_else(|| parent_invocation.as_ref().and_then(extract_invocation_id));
    let parent_receipt_ura = optional_string(obj, "parent_receipt_ura")
        .or_else(|| optional_string(meta_obj, "parent_receipt_ura"))
        .or_else(|| {
            parent_invocation
                .as_ref()
                .and_then(extract_parent_receipt_ura)
        });
    let child_invocations = project_child_invocations(meta_obj);
    let child_receipts = child_invocations
        .iter()
        .filter_map(project_child_receipt_ref)
        .collect::<Vec<_>>();
    let output_refs = project_output_refs(obj, meta_obj);

    let mut metadata = Map::new();
    metadata.insert(
        "profile".to_string(),
        Value::String(MISSION_PROFILE.to_string()),
    );
    metadata.insert(
        "carrier_owner".to_string(),
        Value::String("daemon_sdk".to_string()),
    );
    metadata.insert(
        "status_source".to_string(),
        Value::String(
            if obj.contains_key("meta") {
                "mission_result"
            } else {
                "mission_meta"
            }
            .to_string(),
        ),
    );
    metadata.insert(
        "running".to_string(),
        Value::Bool(optional_bool(obj, "running").unwrap_or(!terminal)),
    );
    copy_optional(meta_obj, &mut metadata, "name");
    copy_optional(meta_obj, &mut metadata, "source_file");
    copy_optional(meta_obj, &mut metadata, "trace_id");
    copy_optional(meta_obj, &mut metadata, "started_at");
    copy_optional(meta_obj, &mut metadata, "duration_ms");
    copy_optional(meta_obj, &mut metadata, "steps_total");
    copy_optional(meta_obj, &mut metadata, "steps_completed");
    copy_optional(meta_obj, &mut metadata, "steps_failed");

    let mut status = json!({
        "profile": MISSION_PROFILE,
        "kind": "mission_status",
        "mission_id": mission_id,
        "state": state,
        "terminal": terminal,
        "partial_failures": steps_failed,
        "cancelled": state == "cancelled",
        "parent_invocation_id": parent_invocation_id,
        "parent_receipt_ura": parent_receipt_ura,
        "parent_invocation": parent_invocation,
        "child_invocations": child_invocations,
        "child_receipts": child_receipts,
        "output_refs": output_refs,
        "metadata": metadata,
    });
    if let Some(error) = meta_obj.get("error").filter(|value| !value.is_null()) {
        status["error"] = json!({
            "code": if state == "cancelled" { "MISSION_CANCELLED" } else { "MISSION_FAILED" },
            "message": error.as_str().unwrap_or("mission failed"),
            "source": "mission",
            "retryable": false,
            "details": error,
        });
    }
    Ok(status)
}

pub(crate) fn project_events(input: &Value) -> Result<Value, MissionError> {
    let obj = object(input, "MissionEventPageInput")?;
    let page_value = obj
        .get("result")
        .filter(|value| !value.is_null())
        .unwrap_or(input);
    let page_obj = object(page_value, "MissionEventPageResult")?;
    let meta_value = page_obj.get("meta").unwrap_or(page_value);
    let meta_obj = object(meta_value, "MissionEventPageMeta").unwrap_or(obj);
    let mission_id = optional_string(page_obj, "mission_id")
        .or_else(|| optional_string(page_obj, "run_id"))
        .or_else(|| optional_string(page_obj, "trace_id"))
        .or_else(|| optional_string(meta_obj, "trace_id"))
        .or_else(|| optional_string(obj, "mission_id"))
        .or_else(|| optional_string(obj, "run_id"))
        .or_else(|| optional_string(obj, "trace_id"))
        .ok_or(MissionError::MissingField("mission_id"))?;
    validate_mission_id(&mission_id)?;

    let cursor_sequence = optional_i64(page_obj, "cursor_sequence")
        .or_else(|| optional_i64(page_obj, "from_sequence"))
        .or_else(|| optional_i64(obj, "cursor_sequence"))
        .or_else(|| optional_i64(obj, "from_sequence"))
        .unwrap_or(0);
    if cursor_sequence < 0 {
        return Err(MissionError::InvalidField(
            "cursor_sequence",
            "must be non-negative".to_string(),
        ));
    }
    let raw_events = event_array(page_obj)
        .or_else(|| event_array(meta_obj))
        .ok_or(MissionError::MissingField("events"))?;
    let mut events = raw_events
        .iter()
        .map(|event| project_mission_event(&mission_id, event))
        .collect::<Result<Vec<_>, _>>()?;
    events.sort_by_key(|event| event.sequence);
    reject_duplicate_event_sequences(&events)?;
    let next_cursor_sequence = events
        .last()
        .map(|event| event.sequence + 1)
        .unwrap_or(cursor_sequence);
    let has_more = optional_bool(page_obj, "has_more")
        .or_else(|| optional_bool(obj, "has_more"))
        .unwrap_or(false);
    let dropped_count = optional_i64(page_obj, "dropped_count")
        .or_else(|| optional_i64(obj, "dropped_count"))
        .unwrap_or(0);
    if dropped_count < 0 {
        return Err(MissionError::InvalidField(
            "dropped_count",
            "must be non-negative".to_string(),
        ));
    }
    let projected = events
        .into_iter()
        .map(MissionEventProjection::into_json)
        .collect::<Vec<_>>();
    let mut metadata = Map::new();
    metadata.insert(
        "profile".to_string(),
        Value::String(MISSION_PROFILE.to_string()),
    );
    metadata.insert(
        "carrier_owner".to_string(),
        Value::String("daemon_sdk".to_string()),
    );
    metadata.insert(
        "event_source".to_string(),
        Value::String("mission_timeline".to_string()),
    );
    copy_optional(page_obj, &mut metadata, "source");
    copy_optional(obj, &mut metadata, "source");
    copy_optional(meta_obj, &mut metadata, "trace_id");

    Ok(json!({
        "profile": MISSION_PROFILE,
        "kind": "mission_event_page",
        "mission_id": mission_id,
        "cursor_sequence": cursor_sequence,
        "next_cursor_sequence": next_cursor_sequence,
        "has_more": has_more,
        "dropped_count": dropped_count,
        "events": projected,
        "metadata": metadata,
    }))
}

#[derive(Debug)]
struct MissionEventProjection {
    mission_id: String,
    sequence: i64,
    event_type: String,
    occurred_unix_ms: i64,
    terminal: bool,
    payload: Value,
    receipt: Value,
    metadata: Map<String, Value>,
}

impl MissionEventProjection {
    fn into_json(self) -> Value {
        json!({
            "profile": MISSION_PROFILE,
            "kind": "mission_event",
            "mission_id": self.mission_id,
            "sequence": self.sequence,
            "event_type": self.event_type,
            "occurred_unix_ms": self.occurred_unix_ms,
            "terminal": self.terminal,
            "payload": self.payload,
            "receipt": self.receipt,
            "metadata": self.metadata,
        })
    }
}

fn event_array<'a>(obj: &'a Map<String, Value>) -> Option<&'a Vec<Value>> {
    obj.get("events")
        .or_else(|| obj.get("timeline_events"))
        .or_else(|| obj.get("timeline"))
        .and_then(Value::as_array)
}

fn project_mission_event(
    mission_id: &str,
    event: &Value,
) -> Result<MissionEventProjection, MissionError> {
    let obj = object(event, "MissionEvent")?;
    let sequence = optional_i64(obj, "sequence").ok_or(MissionError::MissingField("sequence"))?;
    if sequence < 0 {
        return Err(MissionError::InvalidField(
            "sequence",
            "must be non-negative".to_string(),
        ));
    }
    let event_type = optional_string(obj, "event_type")
        .or_else(|| optional_string(obj, "type"))
        .ok_or(MissionError::MissingField("event_type"))?;
    let occurred_unix_ms = optional_i64(obj, "occurred_unix_ms")
        .or_else(|| optional_i64(obj, "timestamp_unix_ms"))
        .unwrap_or(0);
    if occurred_unix_ms < 0 {
        return Err(MissionError::InvalidField(
            "occurred_unix_ms",
            "must be non-negative".to_string(),
        ));
    }
    let payload = obj.get("payload").cloned().unwrap_or(Value::Null);
    let receipt = event_receipt(obj, &payload);
    let terminal = optional_bool(obj, "terminal").unwrap_or_else(|| {
        matches!(
            event_type.as_str(),
            "completed" | "failed" | "cancelled" | "canceled"
        )
    });
    if terminal
        && !matches!(
            event_type.as_str(),
            "completed" | "failed" | "cancelled" | "canceled"
        )
    {
        return Err(MissionError::InvalidField(
            "terminal",
            "terminal mission events must use a terminal event_type".to_string(),
        ));
    }
    let mut metadata = Map::new();
    copy_optional(obj, &mut metadata, "step_id");
    copy_optional(obj, &mut metadata, "request_id");
    copy_optional(obj, &mut metadata, "trace_id");
    copy_optional(obj, &mut metadata, "ability");
    copy_optional(obj, &mut metadata, "invocation_ura");
    Ok(MissionEventProjection {
        mission_id: mission_id.to_string(),
        sequence,
        event_type,
        occurred_unix_ms,
        terminal,
        payload,
        receipt,
        metadata,
    })
}

fn event_receipt(obj: &Map<String, Value>, payload: &Value) -> Value {
    obj.get("receipt")
        .filter(|value| !value.is_null())
        .cloned()
        .or_else(|| {
            payload
                .as_object()
                .and_then(|payload| payload.get("receipt").filter(|value| !value.is_null()))
                .cloned()
        })
        .unwrap_or(Value::Null)
}

fn reject_duplicate_event_sequences(events: &[MissionEventProjection]) -> Result<(), MissionError> {
    let mut previous = None;
    for event in events {
        if previous == Some(event.sequence) {
            return Err(MissionError::InvalidField(
                "sequence",
                "duplicate mission event sequence".to_string(),
            ));
        }
        previous = Some(event.sequence);
    }
    Ok(())
}

fn required_mission_id(obj: &Map<String, Value>) -> Result<&str, MissionError> {
    let mission_id = required_string(obj, "mission_id")?;
    validate_mission_id(mission_id)?;
    Ok(mission_id)
}

fn required_source(obj: &Map<String, Value>) -> Result<&str, MissionError> {
    obj.get("source")
        .and_then(Value::as_str)
        .filter(|value| !value.trim().is_empty())
        .ok_or(MissionError::MissingField("source"))
}

fn validate_mission_id(raw: &str) -> Result<(), MissionError> {
    if raw.contains('/') || raw.contains('\\') || raw.split('-').any(|part| part == "..") {
        return Err(MissionError::InvalidField(
            "mission_id",
            "must be a run id or unambiguous prefix, not a path".to_string(),
        ));
    }
    Ok(())
}

fn validate_file_path<'a>(raw: &'a str) -> Result<&'a Path, MissionError> {
    let path = Path::new(raw);
    if !path.is_absolute() {
        return Err(MissionError::InvalidField(
            "path",
            "must be an absolute path".to_string(),
        ));
    }
    if path
        .components()
        .any(|component| matches!(component, Component::ParentDir))
    {
        return Err(MissionError::InvalidField(
            "path",
            "must not contain `..` components".to_string(),
        ));
    }
    if !path.is_file() {
        return Err(MissionError::InvalidField(
            "path",
            "must be an existing EAL source file".to_string(),
        ));
    }
    Ok(path)
}

fn normalize_state(raw: &str) -> Result<&'static str, MissionError> {
    match raw.trim().to_ascii_lowercase().as_str() {
        "running" => Ok("running"),
        "ok" | "completed" | "succeeded" => Ok("ok"),
        "partial" => Ok("partial"),
        "error" | "failed" => Ok("error"),
        "cancelled" | "canceled" => Ok("cancelled"),
        other => Err(MissionError::InvalidField(
            "status",
            format!("unsupported mission state {other:?}"),
        )),
    }
}

fn mission_state_is_terminal(state: &str) -> bool {
    !matches!(state, "running")
}

fn optional_usize(obj: &Map<String, Value>, key: &'static str) -> Option<usize> {
    obj.get(key)
        .and_then(Value::as_u64)
        .and_then(|value| usize::try_from(value).ok())
}

fn optional_i64(obj: &Map<String, Value>, key: &'static str) -> Option<i64> {
    obj.get(key).and_then(Value::as_i64)
}

fn optional_bool(obj: &Map<String, Value>, key: &'static str) -> Option<bool> {
    obj.get(key).and_then(Value::as_bool)
}

fn copy_optional(src: &Map<String, Value>, dst: &mut Map<String, Value>, key: &'static str) {
    if let Some(value) = src.get(key) {
        dst.insert(key.to_string(), value.clone());
    }
}

fn extract_invocation_id(value: &Value) -> Option<String> {
    [
        "parent_invocation_id",
        "invocation_id",
        "invocation_ura",
        "request_id",
    ]
    .into_iter()
    .find_map(|key| value.get(key).and_then(Value::as_str))
    .map(str::trim)
    .filter(|value| !value.is_empty())
    .map(str::to_string)
}

fn extract_parent_receipt_ura(value: &Value) -> Option<String> {
    string_at(value, &["receipt_ura"])
        .or_else(|| string_at(value, &["causal_context", "receipt_ura"]))
        .or_else(|| {
            value
                .get("causal_context")
                .and_then(|causal| string_at(causal, &["prior", "0", "receipt_ura"]))
        })
        .or_else(|| {
            value
                .get("causal_context")
                .and_then(|causal| string_at(causal, &["parents", "0", "receipt_ura"]))
        })
}

fn project_child_invocations(meta_obj: &Map<String, Value>) -> Vec<Value> {
    let Some(traces) = meta_obj
        .get("ability_graph_traces")
        .and_then(Value::as_array)
    else {
        return Vec::new();
    };
    traces
        .iter()
        .filter_map(|entry| {
            let obj = entry.as_object()?;
            let receipt = receipt_from_trace(entry);
            Some(json!({
                "step_id": optional_string(obj, "step_id"),
                "request_id": optional_string(obj, "request_id"),
                "trace_id": optional_string(obj, "trace_id"),
                "ability": optional_string(obj, "ability"),
                "invocation_ura": optional_string(obj, "invocation_ura"),
                "caller_ura": optional_string(obj, "caller_ura"),
                "callee_ura": optional_string(obj, "callee_ura"),
                "subject_ura": optional_string(obj, "subject_ura"),
                "metadata_state": optional_string(obj, "metadata_state"),
                "ledger_state": obj.get("ledger_state").cloned().unwrap_or(Value::Null),
                "receipt": receipt,
            }))
        })
        .collect()
}

fn project_child_receipt_ref(child: &Value) -> Option<Value> {
    let receipt = child.get("receipt")?.as_object()?;
    Some(json!({
        "step_id": child.get("step_id").cloned().unwrap_or(Value::Null),
        "invocation_ura": child.get("invocation_ura").cloned().unwrap_or(Value::Null),
        "receipt_ura": receipt.get("receipt_ura").cloned().unwrap_or(Value::Null),
        "receipt_hash": receipt.get("receipt_hash").cloned().unwrap_or(Value::Null),
    }))
}

fn receipt_from_trace(trace: &Value) -> Value {
    let Some(anchor) = trace.pointer("/receipt/anchor").and_then(Value::as_object) else {
        return Value::Null;
    };
    let receipt_ura = optional_string(anchor, "receipt_ura");
    let receipt_hash = optional_string(anchor, "receipt_hash");
    match (receipt_ura, receipt_hash) {
        (Some(receipt_ura), Some(receipt_hash)) => json!({
            "receipt_ura": receipt_ura,
            "receipt_hash": receipt_hash,
            "head_receipt_hash": string_at(trace, &["receipt", "head_receipt_hash"]),
        }),
        _ => Value::Null,
    }
}

fn project_output_refs(obj: &Map<String, Value>, meta_obj: &Map<String, Value>) -> Vec<Value> {
    let mut refs = Vec::new();
    if let Some(existing) = obj.get("output_refs").and_then(Value::as_array) {
        refs.extend(existing.iter().cloned());
    }
    if let Some(run_dir) = optional_string(obj, "run_dir") {
        refs.push(json!({
            "kind": "run_dir",
            "path": run_dir,
        }));
        refs.push(json!({
            "kind": "mission_meta",
            "path": format!("{run_dir}/meta.json"),
        }));
        refs.push(json!({
            "kind": "mission_trace",
            "path": format!("{run_dir}/trace.json"),
        }));
        refs.push(json!({
            "kind": "mission_source",
            "path": format!("{run_dir}/source.eal"),
        }));
    }
    if let Some(source_file) = optional_string(meta_obj, "source_file") {
        refs.push(json!({
            "kind": "source_input",
            "path": source_file,
        }));
    }
    refs
}

fn string_at(value: &Value, path: &[&str]) -> Option<String> {
    let mut current = value;
    for part in path {
        if let Ok(index) = part.parse::<usize>() {
            current = current.as_array()?.get(index)?;
        } else {
            current = current.as_object()?.get(*part)?;
        }
    }
    current
        .as_str()
        .map(str::trim)
        .filter(|value| !value.is_empty())
        .map(str::to_string)
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::io::Write;

    fn nonce() -> &'static str {
        "AQIDBAUGBwgJCgsMDQ4PEA=="
    }

    fn base_request(extra: Value) -> Value {
        let mut obj = json!({
            "caller_ura": "easynet:///r/example/agent/alice.sdk",
            "callee_ura": "easynet:///r/example/device/dev-a",
            "subject_ura": "easynet:///r/example/device/dev-a",
            "descriptor_version": "1.0.0",
            "nonce_base64": nonce(),
            "causal_context": {"form": "none"}
        });
        obj.as_object_mut()
            .unwrap()
            .extend(extra.as_object().unwrap().clone());
        obj
    }

    #[test]
    fn build_run_eal_invocation_projects_complete_tuple() {
        let request = base_request(json!({
            "source": "  mission demo\nlet r = local.observe_health()\n",
            "label": "demo"
        }));

        let invocation = build_run_eal_invocation(&request).unwrap();

        assert_eq!(invocation["metadata"]["profile"], MISSION_PROFILE);
        assert_eq!(invocation["metadata"]["system_ability"], SYSTEM_ABILITY_RUN);
        assert_eq!(invocation["args"]["label"], "demo");
        assert_eq!(
            invocation["args"]["source"],
            "  mission demo\nlet r = local.observe_health()\n"
        );
        assert!(invocation["descriptor_ref"]
            .as_str()
            .unwrap()
            .contains("mission.run@1.0.0"));
    }

    #[test]
    fn build_run_file_invocation_reads_absolute_source() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("pipeline.eal");
        let mut file = std::fs::File::create(&path).unwrap();
        writeln!(file, "mission demo").unwrap();

        let request = base_request(json!({
            "path": path.display().to_string()
        }));

        let invocation = build_run_file_invocation(&request).unwrap();

        assert_eq!(invocation["metadata"]["system_ability"], SYSTEM_ABILITY_RUN);
        assert!(invocation["args"]["source"]
            .as_str()
            .unwrap()
            .contains("mission demo"));
        assert_eq!(invocation["args"]["label"], path.display().to_string());
    }

    #[test]
    fn build_track_invocation_maps_mission_id_to_run_id() {
        let request = base_request(json!({
            "mission_id": "2026-07-04_010203_demo"
        }));

        let invocation = build_track_invocation(&request).unwrap();

        assert_eq!(
            invocation["metadata"]["system_ability"],
            SYSTEM_ABILITY_TRACK
        );
        assert_eq!(invocation["args"]["run_id"], "2026-07-04_010203_demo");
    }

    #[test]
    fn build_events_invocation_projects_bounded_replay_args() {
        let request = base_request(json!({
            "mission_id": "2026-07-04_010203_demo",
            "cursor_sequence": 4,
            "limit": 25
        }));

        let invocation = build_events_invocation(&request).unwrap();

        assert_eq!(
            invocation["metadata"]["system_ability"],
            SYSTEM_ABILITY_EVENTS
        );
        assert_eq!(
            invocation["descriptor_ref"],
            "easynet:///r/example/ability/device.dev-a.mission.events@1.0.0"
        );
        assert_eq!(invocation["args"]["run_id"], "2026-07-04_010203_demo");
        assert_eq!(invocation["args"]["cursor_sequence"], 4);
        assert_eq!(invocation["args"]["limit"], 25);
    }

    #[test]
    fn build_events_invocation_rejects_unbounded_limit() {
        let request = base_request(json!({
            "mission_id": "2026-07-04_010203_demo",
            "cursor_sequence": 4,
            "limit": 1001
        }));

        let err = build_events_invocation(&request).unwrap_err();

        assert!(format!("{err}").contains("limit"));
    }

    #[test]
    fn project_status_exposes_child_receipts_and_parent_context() {
        let status = project_status(&json!({
            "run_id": "2026-07-04_010203_demo",
            "run_dir": "/tmp/easynet/missions/runs/2026-07-04_010203_demo",
            "running": false,
            "meta": {
                "name": "demo",
                "trace_id": "2026-07-04_010203_demo",
                "started_at": "2026-07-04T01:02:03Z",
                "duration_ms": 42,
                "status": "partial",
                "steps_total": 2,
                "steps_completed": 1,
                "steps_failed": 1,
                "invocation_context": {
                    "caller": "easynet:///r/example/agent/alice.sdk",
                    "causal_context": {
                        "form": "scalar",
                        "receipt_ura": "easynet:///r/example/resource/agent.alice.sdk/invocation/parent/receipt",
                        "receipt_hash_hex": "aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa"
                    }
                },
                "ability_graph_traces": [{
                    "step_id": "s1",
                    "request_id": "req-1",
                    "trace_id": "2026-07-04_010203_demo",
                    "ability": "observe.health",
                    "invocation_ura": "easynet:///r/example/invocation/req-1",
                    "receipt": {
                        "head_receipt_hash": "bbbb",
                        "anchor": {
                            "receipt_ura": "easynet:///r/example/resource/agent.alice.sdk/invocation/child/receipt",
                            "receipt_hash": "bbbb"
                        }
                    },
                    "metadata_state": "receipt_backed"
                }]
            }
        }))
        .unwrap();

        assert_eq!(status["state"], "partial");
        assert_eq!(status["terminal"], true);
        assert_eq!(status["partial_failures"], 1);
        assert_eq!(
            status["parent_receipt_ura"],
            "easynet:///r/example/resource/agent.alice.sdk/invocation/parent/receipt"
        );
        assert_eq!(status["child_invocations"].as_array().unwrap().len(), 1);
        assert_eq!(
            status["child_receipts"][0]["receipt_ura"],
            "easynet:///r/example/resource/agent.alice.sdk/invocation/child/receipt"
        );
        assert_eq!(status["output_refs"].as_array().unwrap().len(), 4);
    }

    #[test]
    fn project_events_orders_by_sequence_and_reports_cursor() {
        let page = project_events(&json!({
            "run_id": "2026-07-04_010203_demo",
            "cursor_sequence": 4,
            "has_more": false,
            "events": [
                {
                    "sequence": 6,
                    "timestamp_unix_ms": 1006,
                    "type": "completed",
                    "payload": {"reply": "done"},
                    "receipt": {
                        "receipt_ura": "easynet:///r/example/resource/agent.alice.sdk/invocation/terminal/receipt",
                        "receipt_hash": "bbbb"
                    }
                },
                {
                    "sequence": 4,
                    "timestamp_unix_ms": 1004,
                    "type": "progress",
                    "payload": {"delta": "hello"},
                    "step_id": "s1",
                    "invocation_ura": "easynet:///r/example/invocation/req-1"
                }
            ]
        }))
        .unwrap();

        assert_eq!(page["profile"], MISSION_PROFILE);
        assert_eq!(page["kind"], "mission_event_page");
        assert_eq!(page["cursor_sequence"], 4);
        assert_eq!(page["next_cursor_sequence"], 7);
        assert_eq!(page["events"][0]["sequence"], 4);
        assert_eq!(page["events"][0]["event_type"], "progress");
        assert_eq!(
            page["events"][0]["metadata"]["invocation_ura"],
            "easynet:///r/example/invocation/req-1"
        );
        assert_eq!(page["events"][1]["terminal"], true);
        assert_eq!(
            page["events"][1]["receipt"]["receipt_ura"],
            "easynet:///r/example/resource/agent.alice.sdk/invocation/terminal/receipt"
        );
    }

    #[test]
    fn project_events_accepts_runtime_result_wrapper() {
        let page = project_events(&json!({
            "mission_id": "2026-07-04_010203_demo",
            "cursor_sequence": 4,
            "result": {
                "has_more": false,
                "events": [{
                    "sequence": 4,
                    "timestamp_unix_ms": 1004,
                    "type": "completed",
                    "payload": {"ok": true}
                }]
            }
        }))
        .unwrap();

        assert_eq!(page["mission_id"], "2026-07-04_010203_demo");
        assert_eq!(page["cursor_sequence"], 4);
        assert_eq!(page["next_cursor_sequence"], 5);
        assert_eq!(page["events"][0]["event_type"], "completed");
    }

    #[test]
    fn project_events_rejects_invalid_sequence() {
        let err = project_events(&json!({
            "run_id": "2026-07-04_010203_demo",
            "events": [{
                "sequence": -1,
                "type": "progress"
            }]
        }))
        .unwrap_err();

        assert!(format!("{err}").contains("sequence"));
    }

    #[test]
    fn project_events_rejects_terminal_non_terminal_type() {
        let err = project_events(&json!({
            "run_id": "2026-07-04_010203_demo",
            "events": [{
                "sequence": 0,
                "type": "progress",
                "terminal": true
            }]
        }))
        .unwrap_err();

        assert!(format!("{err}").contains("terminal"));
    }

    #[test]
    fn project_status_rejects_unknown_state() {
        let err = project_status(&json!({
            "run_id": "2026-07-04_010203_demo",
            "meta": {"status": "maybe"}
        }))
        .unwrap_err();

        assert!(format!("{err}").contains("unsupported mission state"));
    }
}
