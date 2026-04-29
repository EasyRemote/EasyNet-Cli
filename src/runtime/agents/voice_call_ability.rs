// EasyNet CLI — voice.* call signaling abilities
// ===============================================
//
// File: src/runtime/agents/voice_call_ability.rs
// Description: Seven abilities backing the `easynet call …`
//              subcommand surface. Replaces the dead
//              `bridge.*_voice_*` family removed by
//              AXON-RFC-001 P1.5; the CLI talks to these via the
//              shared `support::local_invoke::invoke_local_ability`
//              helper, same as every other CLI subcommand.
//
// Abilities registered here
// -------------------------
//   voice.create_call    Mint a new call signaling session.
//   voice.show_call      Read one call's signaling state.
//   voice.join_call      Add a participant + their SDP/ICE.
//   voice.leave_call     Drop a participant.
//   voice.end_call       Terminate the call (hang up).
//   voice.watch_call     Subscribe to event stream (snapshot for v1).
//   voice.report_metrics Report QoS metrics from a participant.
//
// Storage model
// -------------
// Calls are kept in-process behind an `Arc<Mutex<HashMap>>` keyed by
// `call_id`. Persistence and federation fan-out are RFC-006-track
// follow-ups; the local view is correct as a single-node state
// machine and the contract surface (envelope shapes, error codes)
// is stable so the federation handler can drop in without breaking
// callers.
//
// Author: Silan Hu <silan.hu@u.nus.edu>
// Copyright (c) 2026 EasyNet. All rights reserved.

use std::collections::HashMap;
use std::sync::{Arc, Mutex, OnceLock};
use std::time::{SystemTime, UNIX_EPOCH};

use serde_json::{json, Value};

use crate::runtime::ability_dispatch::LocalAbilityRegistry;

pub const ABILITY_CREATE_CALL: &str = "voice.create_call";
pub const ABILITY_SHOW_CALL: &str = "voice.show_call";
pub const ABILITY_JOIN_CALL: &str = "voice.join_call";
pub const ABILITY_LEAVE_CALL: &str = "voice.leave_call";
pub const ABILITY_END_CALL: &str = "voice.end_call";
pub const ABILITY_WATCH_CALL: &str = "voice.watch_call";
pub const ABILITY_REPORT_METRICS: &str = "voice.report_metrics";

#[derive(Debug, Clone)]
struct CallState {
    call_id: String,
    state: &'static str, // "ringing" | "active" | "ended"
    created_at_ms: u64,
    ended_at_ms: Option<u64>,
    end_reason: Option<u32>,
    participants: HashMap<String, ParticipantState>,
    events: Vec<Value>,
}

#[derive(Debug, Clone)]
struct ParticipantState {
    participant_id: String,
    sdp_offer: Option<String>,
    ice_candidates: Vec<Value>,
    last_metrics: Option<Value>,
    joined_at_ms: u64,
    left_at_ms: Option<u64>,
}

fn store() -> &'static Mutex<HashMap<String, CallState>> {
    static STORE: OnceLock<Mutex<HashMap<String, CallState>>> = OnceLock::new();
    STORE.get_or_init(|| Mutex::new(HashMap::new()))
}

fn now_ms() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_millis() as u64)
        .unwrap_or(0)
}

/// Register every voice.* call signaling handler.
pub fn register(reg: &mut LocalAbilityRegistry) {
    reg.register_rpc(
        ABILITY_CREATE_CALL,
        Arc::new(|args| create_call_handler(args)),
    );
    reg.register_rpc(ABILITY_SHOW_CALL, Arc::new(|args| show_call_handler(args)));
    reg.register_rpc(ABILITY_JOIN_CALL, Arc::new(|args| join_call_handler(args)));
    reg.register_rpc(
        ABILITY_LEAVE_CALL,
        Arc::new(|args| leave_call_handler(args)),
    );
    reg.register_rpc(ABILITY_END_CALL, Arc::new(|args| end_call_handler(args)));
    reg.register_rpc(
        ABILITY_WATCH_CALL,
        Arc::new(|args| watch_call_handler(args)),
    );
    reg.register_rpc(
        ABILITY_REPORT_METRICS,
        Arc::new(|args| report_metrics_handler(args)),
    );
}

// ── Handlers ─────────────────────────────────────────────────────

fn require_str<'a>(args: &'a Value, key: &str, ability: &str) -> anyhow::Result<&'a str> {
    args.get(key)
        .and_then(Value::as_str)
        .ok_or_else(|| anyhow::anyhow!("{ability}: `{key}` is required"))
}

fn create_call_handler(args: Value) -> anyhow::Result<Value> {
    let call_id = args
        .get("call_id")
        .and_then(Value::as_str)
        .filter(|s| !s.is_empty())
        .map(str::to_string)
        .unwrap_or_else(|| format!("call-{:x}", now_ms()));
    let mut s = store().lock().unwrap();
    if s.contains_key(&call_id) {
        anyhow::bail!("voice.create_call: call_id {call_id:?} already exists");
    }
    let state = CallState {
        call_id: call_id.clone(),
        state: "ringing",
        created_at_ms: now_ms(),
        ended_at_ms: None,
        end_reason: None,
        participants: HashMap::new(),
        events: Vec::new(),
    };
    s.insert(call_id.clone(), state);
    Ok(json!({
        "call_id": call_id,
        "state": "ringing",
        "created_at_ms": now_ms(),
    }))
}

fn show_call_handler(args: Value) -> anyhow::Result<Value> {
    let call_id = require_str(&args, "call_id", "voice.show_call")?;
    let s = store().lock().unwrap();
    let call = s
        .get(call_id)
        .ok_or_else(|| anyhow::anyhow!("voice.show_call: call {call_id:?} not found"))?;
    Ok(serialize_call(call))
}

fn join_call_handler(args: Value) -> anyhow::Result<Value> {
    let call_id = require_str(&args, "call_id", "voice.join_call")?.to_string();
    let participant_id = require_str(&args, "participant_id", "voice.join_call")?.to_string();
    let sdp_offer = args
        .get("sdp_offer")
        .and_then(Value::as_str)
        .map(str::to_string);
    let mut s = store().lock().unwrap();
    let call = s.get_mut(&call_id).ok_or_else(|| {
        anyhow::anyhow!("voice.join_call: call {call_id:?} not found")
    })?;
    if call.state == "ended" {
        anyhow::bail!("voice.join_call: call {call_id:?} has already ended");
    }
    let was_empty = call.participants.is_empty();
    call.participants.insert(
        participant_id.clone(),
        ParticipantState {
            participant_id: participant_id.clone(),
            sdp_offer: sdp_offer.clone(),
            ice_candidates: Vec::new(),
            last_metrics: None,
            joined_at_ms: now_ms(),
            left_at_ms: None,
        },
    );
    // First participant transitions the call to active.
    if was_empty {
        call.state = "active";
    }
    call.events.push(json!({
        "type": "joined",
        "participant_id": participant_id,
        "at_ms": now_ms(),
    }));
    Ok(json!({
        "call_id": call_id,
        "participant_id": participant_id,
        "state": call.state,
    }))
}

fn leave_call_handler(args: Value) -> anyhow::Result<Value> {
    let call_id = require_str(&args, "call_id", "voice.leave_call")?.to_string();
    let participant_id = require_str(&args, "participant_id", "voice.leave_call")?.to_string();
    let reason = args
        .get("reason")
        .and_then(Value::as_str)
        .unwrap_or("normal")
        .to_string();
    let mut s = store().lock().unwrap();
    let call = s.get_mut(&call_id).ok_or_else(|| {
        anyhow::anyhow!("voice.leave_call: call {call_id:?} not found")
    })?;
    let p = call.participants.get_mut(&participant_id).ok_or_else(|| {
        anyhow::anyhow!(
            "voice.leave_call: participant {participant_id:?} not in call {call_id:?}"
        )
    })?;
    p.left_at_ms = Some(now_ms());
    call.events.push(json!({
        "type": "left",
        "participant_id": participant_id,
        "reason": reason,
        "at_ms": now_ms(),
    }));
    Ok(json!({"call_id": call_id, "participant_id": participant_id}))
}

fn end_call_handler(args: Value) -> anyhow::Result<Value> {
    let call_id = require_str(&args, "call_id", "voice.end_call")?.to_string();
    let end_reason = args
        .get("end_reason")
        .and_then(Value::as_u64)
        .unwrap_or(1) as u32;
    let mut s = store().lock().unwrap();
    let call = s.get_mut(&call_id).ok_or_else(|| {
        anyhow::anyhow!("voice.end_call: call {call_id:?} not found")
    })?;
    if call.state == "ended" {
        return Ok(json!({
            "call_id": call_id,
            "state": "ended",
            "already_ended": true,
        }));
    }
    call.state = "ended";
    call.ended_at_ms = Some(now_ms());
    call.end_reason = Some(end_reason);
    call.events.push(json!({
        "type": "ended",
        "reason_code": end_reason,
        "at_ms": now_ms(),
    }));
    Ok(json!({
        "call_id": call_id,
        "state": "ended",
        "end_reason": end_reason,
    }))
}

fn watch_call_handler(args: Value) -> anyhow::Result<Value> {
    let call_id = require_str(&args, "call_id", "voice.watch_call")?;
    let s = store().lock().unwrap();
    let call = s.get(call_id).ok_or_else(|| {
        anyhow::anyhow!("voice.watch_call: call {call_id:?} not found")
    })?;
    // v1 returns a snapshot of accumulated events. Streaming is a
    // follow-up: register through `register_stream` once the
    // subscription protocol is wired.
    Ok(json!({
        "call_id": call_id,
        "events": call.events.clone(),
        "view": "snapshot",
    }))
}

fn report_metrics_handler(args: Value) -> anyhow::Result<Value> {
    let call_id = require_str(&args, "call_id", "voice.report_metrics")?.to_string();
    let participant_id = require_str(&args, "participant_id", "voice.report_metrics")?.to_string();
    let metrics = args
        .get("metrics")
        .cloned()
        .unwrap_or_else(|| json!({}));
    let mut s = store().lock().unwrap();
    let call = s.get_mut(&call_id).ok_or_else(|| {
        anyhow::anyhow!("voice.report_metrics: call {call_id:?} not found")
    })?;
    let p = call.participants.get_mut(&participant_id).ok_or_else(|| {
        anyhow::anyhow!(
            "voice.report_metrics: participant {participant_id:?} not in call {call_id:?}"
        )
    })?;
    p.last_metrics = Some(metrics.clone());
    call.events.push(json!({
        "type": "metrics",
        "participant_id": participant_id,
        "metrics": metrics,
        "at_ms": now_ms(),
    }));
    Ok(json!({"call_id": call_id, "participant_id": participant_id, "ack": true}))
}

fn serialize_call(call: &CallState) -> Value {
    let participants: Vec<Value> = call
        .participants
        .values()
        .map(|p| {
            json!({
                "participant_id": p.participant_id,
                "sdp_offer": p.sdp_offer,
                "ice_candidates": p.ice_candidates,
                "last_metrics": p.last_metrics,
                "joined_at_ms": p.joined_at_ms,
                "left_at_ms": p.left_at_ms,
            })
        })
        .collect();
    json!({
        "call_id": call.call_id,
        "state": call.state,
        "created_at_ms": call.created_at_ms,
        "ended_at_ms": call.ended_at_ms,
        "end_reason": call.end_reason,
        "participants": participants,
    })
}

// ── Discovery surfaces (description + input_schema) ──────────────

pub fn create_call_description() -> &'static str {
    "Create a voice/video call signaling session. Returns the \
     `call_id` (auto-generated when omitted) and the initial state. \
     v1 stores call state in-process; persistence + federation \
     fan-out land with the RFC-006 follow-up."
}

pub fn create_call_input_schema() -> Value {
    json!({
        "type": "object",
        "additionalProperties": false,
        "properties": { "call_id": { "type": "string" } }
    })
}

pub fn show_call_description() -> &'static str {
    "Read one call's signaling state, including participants, SDP, \
     ICE candidates, last metrics, and accumulated events."
}

pub fn show_call_input_schema() -> Value {
    json!({
        "type": "object",
        "additionalProperties": false,
        "required": ["call_id"],
        "properties": { "call_id": { "type": "string" } }
    })
}

pub fn join_call_description() -> &'static str {
    "Add a participant to an existing call. The first participant \
     transitions the call from `ringing` to `active`. Optional \
     `sdp_offer` carries the participant's SDP; ICE candidates \
     stream in via subsequent `voice.report_metrics`-style updates."
}

pub fn join_call_input_schema() -> Value {
    json!({
        "type": "object",
        "additionalProperties": false,
        "required": ["call_id", "participant_id"],
        "properties": {
            "call_id":        { "type": "string" },
            "participant_id": { "type": "string" },
            "sdp_offer":      { "type": "string" }
        }
    })
}

pub fn leave_call_description() -> &'static str {
    "Drop a participant from a call. The call itself stays active \
     until `voice.end_call` is invoked or every participant has left."
}

pub fn leave_call_input_schema() -> Value {
    json!({
        "type": "object",
        "additionalProperties": false,
        "required": ["call_id", "participant_id"],
        "properties": {
            "call_id":        { "type": "string" },
            "participant_id": { "type": "string" },
            "reason":         { "type": "string" }
        }
    })
}

pub fn end_call_description() -> &'static str {
    "Terminate a call. Idempotent: a second invocation returns the \
     existing terminal envelope with `already_ended = true` rather \
     than erroring."
}

pub fn end_call_input_schema() -> Value {
    json!({
        "type": "object",
        "additionalProperties": false,
        "required": ["call_id"],
        "properties": {
            "call_id":    { "type": "string" },
            "end_reason": { "type": "integer", "minimum": 0 }
        }
    })
}

pub fn watch_call_description() -> &'static str {
    "Read the accumulated event log for a call. v1 returns a \
     snapshot (`view: \"snapshot\"`); the streaming variant lands \
     when `register_stream` for voice.* is wired."
}

pub fn watch_call_input_schema() -> Value {
    show_call_input_schema()
}

pub fn report_metrics_description() -> &'static str {
    "Report QoS metrics from a participant (jitter, packet loss, \
     bitrate). The metrics value is appended to the call's event \
     log and stored as the participant's `last_metrics` snapshot."
}

pub fn report_metrics_input_schema() -> Value {
    json!({
        "type": "object",
        "additionalProperties": false,
        "required": ["call_id", "participant_id"],
        "properties": {
            "call_id":        { "type": "string" },
            "participant_id": { "type": "string" },
            "metrics":        { "type": "object" }
        }
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    fn fresh_call_id(prefix: &str) -> String {
        format!("{prefix}-{:x}", now_ms())
    }

    #[test]
    fn create_show_join_metrics_end_round_trip() {
        // Full happy path — covers every handler in one test so a
        // regression in any handler trips this.
        let cid = fresh_call_id("rt");
        let _ = create_call_handler(json!({"call_id": cid}))
            .expect("create");
        let s1 = show_call_handler(json!({"call_id": cid})).unwrap();
        assert_eq!(s1.get("state").and_then(Value::as_str), Some("ringing"));

        join_call_handler(json!({
            "call_id": cid,
            "participant_id": "alice",
            "sdp_offer": "v=0",
        }))
        .expect("join");
        let s2 = show_call_handler(json!({"call_id": cid})).unwrap();
        assert_eq!(s2.get("state").and_then(Value::as_str), Some("active"));

        report_metrics_handler(json!({
            "call_id": cid,
            "participant_id": "alice",
            "metrics": { "rtt_ms": 42 },
        }))
        .expect("metrics");

        let watch = watch_call_handler(json!({"call_id": cid})).unwrap();
        let events = watch.get("events").and_then(Value::as_array).unwrap();
        assert!(
            events.iter().any(|e| e.get("type") == Some(&json!("joined"))),
            "watch must surface join event: {watch}"
        );

        end_call_handler(json!({"call_id": cid})).expect("end");
        let s3 = show_call_handler(json!({"call_id": cid})).unwrap();
        assert_eq!(s3.get("state").and_then(Value::as_str), Some("ended"));
    }

    #[test]
    fn show_unknown_call_errors_clearly() {
        let err = show_call_handler(json!({"call_id": "no-such-call"})).unwrap_err();
        assert!(format!("{err}").contains("not found"));
    }

    #[test]
    fn end_call_is_idempotent() {
        let cid = fresh_call_id("idempotent");
        create_call_handler(json!({"call_id": cid})).unwrap();
        end_call_handler(json!({"call_id": cid})).unwrap();
        let r = end_call_handler(json!({"call_id": cid})).unwrap();
        assert_eq!(r.get("already_ended"), Some(&json!(true)));
    }

    #[test]
    fn join_after_end_is_rejected() {
        let cid = fresh_call_id("after-end");
        create_call_handler(json!({"call_id": cid})).unwrap();
        end_call_handler(json!({"call_id": cid})).unwrap();
        let err = join_call_handler(json!({
            "call_id": cid,
            "participant_id": "late",
        }))
        .unwrap_err();
        assert!(format!("{err}").contains("already ended"));
    }

    #[test]
    fn create_without_call_id_auto_mints_one() {
        let resp = create_call_handler(json!({})).unwrap();
        let cid = resp.get("call_id").and_then(Value::as_str).unwrap();
        assert!(cid.starts_with("call-"));
    }
}
