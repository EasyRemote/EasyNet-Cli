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

use crate::runtime::ability_dispatch::AxonAbilityCatalog;
use easynet_axon::{VoiceCallState, VoiceEndReason, VoiceEventType, VoiceNetworkMetrics};

pub const ABILITY_CREATE_CALL: &str = "device.voice.create_call";
pub const ABILITY_SHOW_CALL: &str = "device.voice.show_call";
pub const ABILITY_JOIN_CALL: &str = "device.voice.join_call";
pub const ABILITY_LEAVE_CALL: &str = "device.voice.leave_call";
pub const ABILITY_END_CALL: &str = "device.voice.end_call";
pub const ABILITY_WATCH_CALL: &str = "device.voice.watch_call";
pub const ABILITY_REPORT_METRICS: &str = "device.voice.report_metrics";
pub const ABILITY_LIST_CALLS: &str = "device.voice.list_calls";

#[derive(Debug, Clone)]
struct CallState {
    call_id: String,
    state: VoiceCallState,
    created_at_ms: u64,
    ended_at_ms: Option<u64>,
    end_reason: Option<VoiceEndReason>,
    participants: HashMap<String, ParticipantState>,
    events: Vec<Value>,
}

#[derive(Debug, Clone)]
struct ParticipantState {
    participant_id: String,
    sdp_offer: Option<String>,
    ice_candidates: Vec<Value>,
    last_metrics: Option<VoiceNetworkMetrics>,
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
///
/// **M0 owner-kind note**: today all 8 verbs mount under one
/// daemon-level `register()`. The terminal-state spec
/// (`docs/spec/owner-truth-table/`) classifies voice as
/// agent-owned because the signaling state is per-agent. That
/// rename ships at Stage 4 (RFC-001 v4.1.6) along with the
/// system-namespace partitioning; the registration surface
/// changes to per-agent at that point. Until then the daemon
/// hosts the dispatch surface and `OwnerKind::Device` is the
/// honest classification of where the handler runs today.
pub fn register(reg: &mut AxonAbilityCatalog) {
    use crate::runtime::ability_dispatch::OwnerKind;
    reg.register_rpc_with_owner(
        "device.voice.create_call",
        OwnerKind::Device,
        Arc::new(create_call_handler),
    );
    reg.register_rpc_with_owner(
        "device.voice.show_call",
        OwnerKind::Device,
        Arc::new(show_call_handler),
    );
    reg.register_rpc_with_owner(
        "device.voice.join_call",
        OwnerKind::Device,
        Arc::new(join_call_handler),
    );
    reg.register_rpc_with_owner(
        "device.voice.leave_call",
        OwnerKind::Device,
        Arc::new(leave_call_handler),
    );
    reg.register_rpc_with_owner(
        "device.voice.end_call",
        OwnerKind::Device,
        Arc::new(end_call_handler),
    );
    reg.register_rpc_with_owner(
        "device.voice.watch_call",
        OwnerKind::Device,
        Arc::new(watch_call_handler),
    );
    reg.register_rpc_with_owner(
        "device.voice.report_metrics",
        OwnerKind::Device,
        Arc::new(report_metrics_handler),
    );
    reg.register_rpc_with_owner(
        "device.voice.list_calls",
        OwnerKind::Device,
        Arc::new(list_calls_handler),
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
    let creator_participant_id = args
        .get("participant_id")
        .and_then(Value::as_str)
        .filter(|s| !s.is_empty())
        .map(str::to_string);
    let mut s = store().lock().unwrap();
    if s.contains_key(&call_id) {
        anyhow::bail!("voice.create_call: call_id {call_id:?} already exists");
    }
    let mut participants = HashMap::new();
    if let Some(participant_id) = creator_participant_id.clone() {
        participants.insert(
            participant_id.clone(),
            ParticipantState {
                participant_id,
                sdp_offer: None,
                ice_candidates: Vec::new(),
                last_metrics: None,
                joined_at_ms: now_ms(),
                left_at_ms: None,
            },
        );
    }
    let state = CallState {
        call_id: call_id.clone(),
        state: VoiceCallState::Ringing,
        created_at_ms: now_ms(),
        ended_at_ms: None,
        end_reason: None,
        participants,
        events: Vec::new(),
    };
    s.insert(call_id.clone(), state);
    // Render `Option<String>` as a stable string so SRE pipelines
    // grep `creator_participant_id=<value>` without seeing Rust's
    // `Some("…")` / `None` Debug literal in the field value.
    let creator = creator_participant_id.as_deref().unwrap_or("<none>");
    crate::op_event!(
        component = voice_call,
        kind = call_created,
        call_id = call_id,
        creator_participant_id = creator,
    );
    Ok(json!({
        "call_id": call_id,
        // `state` keeps the legacy label for wire compatibility with
        // existing consumers; `state_proto` carries the Axon contract
        // name. Mirrors the additive convention in `ping.rs`.
        "state": VoiceCallState::Ringing.legacy_label(),
        "state_proto": VoiceCallState::Ringing.as_proto_name(),
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
    let call = s
        .get_mut(&call_id)
        .ok_or_else(|| anyhow::anyhow!("voice.join_call: call {call_id:?} not found"))?;
    if call.state.is_terminal() {
        anyhow::bail!("voice.join_call: call {call_id:?} has already ended");
    }
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
    // A call becomes active once at least two participants are present:
    // the creator/caller plus the first remote joiner.
    if call.participants.len() >= 2 {
        call.state = VoiceCallState::Active;
    }
    call.events.push(json!({
        "event_type": VoiceEventType::ParticipantJoin.as_proto_name(),
        "type": "joined",
        "participant_id": participant_id,
        "state": call.state.legacy_label(),
        "state_proto": call.state.as_proto_name(),
        "at_ms": now_ms(),
    }));
    let participant_count = call.participants.len();
    let state = call.state.as_proto_name();
    crate::op_event!(
        component = voice_call,
        kind = participant_joined,
        call_id = call_id,
        participant_id = participant_id,
        participant_count = participant_count,
        state = state,
    );
    Ok(json!({
        "call_id": call_id,
        "participant_id": participant_id,
        "state": call.state.legacy_label(),
        "state_proto": call.state.as_proto_name(),
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
    let call = s
        .get_mut(&call_id)
        .ok_or_else(|| anyhow::anyhow!("voice.leave_call: call {call_id:?} not found"))?;
    let p = call.participants.get_mut(&participant_id).ok_or_else(|| {
        anyhow::anyhow!("voice.leave_call: participant {participant_id:?} not in call {call_id:?}")
    })?;
    p.left_at_ms = Some(now_ms());
    call.events.push(json!({
        "event_type": VoiceEventType::ParticipantLeave.as_proto_name(),
        "type": "left",
        "participant_id": participant_id,
        "reason": reason,
        "state": call.state.legacy_label(),
        "state_proto": call.state.as_proto_name(),
        "at_ms": now_ms(),
    }));
    Ok(json!({"call_id": call_id, "participant_id": participant_id}))
}

fn end_call_handler(args: Value) -> anyhow::Result<Value> {
    let call_id = require_str(&args, "call_id", "voice.end_call")?.to_string();
    let end_reason = args
        .get("end_reason")
        .and_then(Value::as_i64)
        .map(VoiceEndReason::from_wire)
        .transpose()?
        .unwrap_or(VoiceEndReason::CallerHangup);
    let mut s = store().lock().unwrap();
    let call = s
        .get_mut(&call_id)
        .ok_or_else(|| anyhow::anyhow!("voice.end_call: call {call_id:?} not found"))?;
    if call.state.is_terminal() {
        return Ok(json!({
            "call_id": call_id,
            "state": call.state.legacy_label(),
            "state_proto": call.state.as_proto_name(),
            "already_ended": true,
        }));
    }
    call.state = VoiceCallState::Ended;
    call.ended_at_ms = Some(now_ms());
    call.end_reason = Some(end_reason);
    call.events.push(json!({
        "event_type": VoiceEventType::CallEnded.as_proto_name(),
        "type": "ended",
        // `reason_code` is the legacy numeric event field; the proto
        // name rides the additive `end_reason_proto`.
        "reason_code": end_reason.to_wire_i32(),
        "end_reason_proto": end_reason.as_proto_name(),
        "state": call.state.legacy_label(),
        "state_proto": call.state.as_proto_name(),
        "at_ms": now_ms(),
    }));
    Ok(json!({
        "call_id": call_id,
        "state": call.state.legacy_label(),
        "state_proto": call.state.as_proto_name(),
        // `end_reason` keeps its legacy numeric code for wire
        // compatibility (it was a `u32` pre-#contract); the proto name
        // rides the additive `end_reason_proto` field.
        "end_reason": end_reason.to_wire_i32(),
        "end_reason_proto": end_reason.as_proto_name(),
    }))
}

fn watch_call_handler(args: Value) -> anyhow::Result<Value> {
    let call_id = require_str(&args, "call_id", "voice.watch_call")?;
    let s = store().lock().unwrap();
    let call = s
        .get(call_id)
        .ok_or_else(|| anyhow::anyhow!("voice.watch_call: call {call_id:?} not found"))?;
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
        .map(VoiceNetworkMetrics::from_json)
        .transpose()?
        .unwrap_or_default();
    let mut s = store().lock().unwrap();
    let call = s
        .get_mut(&call_id)
        .ok_or_else(|| anyhow::anyhow!("voice.report_metrics: call {call_id:?} not found"))?;
    let p = call.participants.get_mut(&participant_id).ok_or_else(|| {
        anyhow::anyhow!(
            "voice.report_metrics: participant {participant_id:?} not in call {call_id:?}"
        )
    })?;
    p.last_metrics = Some(metrics.clone());
    call.events.push(json!({
        "event_type": VoiceEventType::MetricsReported.as_proto_name(),
        "type": "metrics",
        "participant_id": participant_id,
        "metrics": metrics.to_json(),
        "state": call.state.legacy_label(),
        "state_proto": call.state.as_proto_name(),
        "at_ms": now_ms(),
    }));
    Ok(json!({"call_id": call_id, "participant_id": participant_id, "ack": true}))
}

fn list_calls_handler(_args: Value) -> anyhow::Result<Value> {
    let s = store().lock().unwrap();
    let mut items: Vec<_> = s.values().map(serialize_call).collect();
    items.sort_by(|a, b| {
        let lhs = a.get("call_id").and_then(Value::as_str).unwrap_or("");
        let rhs = b.get("call_id").and_then(Value::as_str).unwrap_or("");
        lhs.cmp(rhs)
    });
    Ok(json!({ "items": items }))
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
                "last_metrics": p.last_metrics.as_ref().map(VoiceNetworkMetrics::to_json),
                "joined_at_ms": p.joined_at_ms,
                "left_at_ms": p.left_at_ms,
            })
        })
        .collect();
    json!({
        "call_id": call.call_id,
        // `state` / `end_reason` keep their legacy values (string label
        // and numeric code respectively) for wire compatibility; the
        // Axon proto names ride additive `*_proto` fields, and the
        // numeric state code rides `state_code`.
        "state": call.state.legacy_label(),
        "state_proto": call.state.as_proto_name(),
        "state_code": call.state.to_wire_i32(),
        "created_at_ms": call.created_at_ms,
        "ended_at_ms": call.ended_at_ms,
        "end_reason": call.end_reason.map(VoiceEndReason::to_wire_i32),
        "end_reason_proto": call.end_reason.map(VoiceEndReason::as_proto_name),
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
     transitions the call from `ringing` to `active` (response `state` \
     carries the legacy label; `state_proto` carries the Axon contract \
     name). Optional `sdp_offer` carries the participant's SDP; ICE \
     candidates stream in via subsequent `voice.report_metrics`-style updates."
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
            "metrics": {
                "type": "object",
                "additionalProperties": false,
                "properties": {
                    "rtt_ms":            { "type": "number" },
                    "jitter_ms":         { "type": "number" },
                    "packet_loss_ratio": { "type": "number", "minimum": 0.0, "maximum": 1.0 },
                    "concealed_samples": { "type": "integer", "minimum": 0 },
                    "audio_level_dbov":  { "type": "number" }
                }
            }
        }
    })
}

pub fn list_calls_description() -> &'static str {
    "List the call signaling sessions currently known to this daemon. \
     Returns each call's state plus the current participant snapshot."
}

pub fn list_calls_input_schema() -> Value {
    json!({
        "type": "object",
        "additionalProperties": false
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
        let _ = create_call_handler(json!({
            "call_id": cid,
            "participant_id": "alice",
        }))
        .expect("create");
        let s1 = show_call_handler(json!({"call_id": cid})).unwrap();
        // `state` carries the legacy label (back-compat); `state_proto`
        // carries the Axon contract name.
        assert_eq!(s1.get("state").and_then(Value::as_str), Some("ringing"));
        assert_eq!(
            s1.get("state_proto").and_then(Value::as_str),
            Some("VOICE_CALL_STATE_RINGING")
        );

        join_call_handler(json!({
            "call_id": cid,
            "participant_id": "bob",
            "sdp_offer": "v=0",
        }))
        .expect("join");
        let s2 = show_call_handler(json!({"call_id": cid})).unwrap();
        assert_eq!(s2.get("state").and_then(Value::as_str), Some("active"));
        assert_eq!(
            s2.get("state_proto").and_then(Value::as_str),
            Some("VOICE_CALL_STATE_ACTIVE")
        );

        report_metrics_handler(json!({
            "call_id": cid,
            "participant_id": "bob",
            "metrics": { "rtt_ms": 42 },
        }))
        .expect("metrics");

        let watch = watch_call_handler(json!({"call_id": cid})).unwrap();
        let events = watch.get("events").and_then(Value::as_array).unwrap();
        assert!(
            events
                .iter()
                .any(|e| e.get("type") == Some(&json!("joined"))),
            "watch must surface join event: {watch}"
        );

        end_call_handler(json!({"call_id": cid})).expect("end");
        let s3 = show_call_handler(json!({"call_id": cid})).unwrap();
        assert_eq!(s3.get("state").and_then(Value::as_str), Some("ended"));
        assert_eq!(
            s3.get("state_proto").and_then(Value::as_str),
            Some("VOICE_CALL_STATE_ENDED")
        );
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

    #[test]
    fn list_calls_returns_created_call() {
        let cid = fresh_call_id("list");
        create_call_handler(json!({"call_id": cid})).unwrap();
        let listed = list_calls_handler(json!({})).unwrap();
        assert!(
            listed
                .get("items")
                .and_then(Value::as_array)
                .is_some_and(|items| items
                    .iter()
                    .any(|item| item.get("call_id") == Some(&json!(cid)))),
            "list_calls must surface the created call: {listed}"
        );
    }
}
