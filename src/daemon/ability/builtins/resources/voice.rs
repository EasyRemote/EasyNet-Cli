// EasyNet CLI — voice.* call signaling abilities
// ===============================================
//
// File: src/daemon/ability/builtins/resources/voice.rs
// Description: Eight abilities backing the `easynet call …`
//              subcommand surface. Replaces the dead
//              `bridge.*_voice_*` family removed by
//              AXON-RFC-001 P1.5; the CLI talks to these through the
//              call signaling issuer, which routes paired calls to the
//              realm Authority and binds an explicit local daemon subject for
//              unpaired local signaling.
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
//   voice.list_calls     List realm-owned call signaling sessions.
//
// Storage model
// -------------
// The realm Authority aggregate is owned by an explicitly injected repository.
// Production registration requires a provider whose compare-and-swap scope
// covers every realm Authority replica; no process-local map or daemon-local
// file is an authority.
//
// Author: Silan Hu <silan.hu@u.nus.edu>
// Copyright (c) 2026 EasyNet. All rights reserved.

use std::sync::Arc;
use std::time::{SystemTime, UNIX_EPOCH};

use serde_json::{json, Value};

#[cfg(test)]
use super::voice_contract::VoiceCallRepositoryEntry;
use super::voice_contract::{
    VoiceCallAggregate, VoiceCallCasOutcome, VoiceCallProviderAssembly, VoiceCallRepository,
    VoiceCallState, VoiceEndReason, VoiceNetworkMetrics,
};
use crate::daemon::ability::dispatch::{AxonAbilityCatalog, EnvelopeContext};

pub const ABILITY_CREATE_CALL: &str = crate::daemon::ability::names::resources::VOICE_CREATE_CALL;
pub const ABILITY_SHOW_CALL: &str = crate::daemon::ability::names::resources::VOICE_SHOW_CALL;
pub const ABILITY_JOIN_CALL: &str = crate::daemon::ability::names::resources::VOICE_JOIN_CALL;
pub const ABILITY_LEAVE_CALL: &str = crate::daemon::ability::names::resources::VOICE_LEAVE_CALL;
pub const ABILITY_END_CALL: &str = crate::daemon::ability::names::resources::VOICE_END_CALL;
pub const ABILITY_WATCH_CALL: &str = crate::daemon::ability::names::resources::VOICE_WATCH_CALL;
pub const ABILITY_REPORT_METRICS: &str =
    crate::daemon::ability::names::resources::VOICE_REPORT_METRICS;
pub const ABILITY_LIST_CALLS: &str = crate::daemon::ability::names::resources::VOICE_LIST_CALLS;

/// Executor over the durable realm Authority voice aggregate repository.
#[derive(Debug, Clone)]
struct VoiceCallService {
    calls: Arc<dyn VoiceCallRepository>,
}

impl VoiceCallService {
    fn new(calls: Arc<dyn VoiceCallRepository>) -> Self {
        Self { calls }
    }

    fn update<T>(
        &self,
        authority_ura: &str,
        call_id: &str,
        command_id: &str,
        transition: impl Fn(&mut VoiceCallAggregate, &str) -> anyhow::Result<T>,
    ) -> anyhow::Result<T> {
        const MAX_CONFLICT_RETRIES: usize = 8;
        for _ in 0..MAX_CONFLICT_RETRIES {
            let mut aggregate = self.load_required(authority_ura, call_id, "voice mutation")?;
            let expected_revision = aggregate.revision();
            let before = aggregate.clone();
            let result = transition(&mut aggregate, command_id)?;
            if aggregate == before {
                return Ok(result);
            }
            aggregate.bump_revision()?;
            aggregate.validate_repository_key(authority_ura, call_id)?;
            aggregate.validate_cas_replacement(expected_revision)?;
            let replacement = aggregate.clone();
            match self.calls.compare_and_swap(
                authority_ura,
                call_id,
                expected_revision,
                aggregate,
            )? {
                VoiceCallCasOutcome::Committed(committed) => {
                    committed.validate_repository_key(authority_ura, call_id)?;
                    if committed != replacement {
                        anyhow::bail!(
                            "voice repository acknowledged CAS for ({authority_ura:?}, {call_id:?}) without storing the proposed aggregate"
                        );
                    }
                    return Ok(result);
                }
                VoiceCallCasOutcome::Current(current) => {
                    current.validate_repository_key(authority_ura, call_id)?;
                    if current.has_command(command_id) {
                        if current.command_matches(command_id, &replacement) {
                            return Ok(result);
                        }
                        anyhow::bail!(
                            "voice repository reused command {command_id:?} for different event facts"
                        );
                    }
                    if current.revision() <= expected_revision {
                        anyhow::bail!(
                            "voice repository returned a non-advancing CAS conflict revision"
                        );
                    }
                }
                VoiceCallCasOutcome::Ambiguous => {
                    let current = self.load_required(
                        authority_ura,
                        call_id,
                        "voice mutation ambiguous commit verification",
                    )?;
                    if current.has_command(command_id) {
                        if current.command_matches(command_id, &replacement) {
                            return Ok(result);
                        }
                        anyhow::bail!(
                            "voice repository reused command {command_id:?} for different event facts"
                        );
                    }
                    if current.revision() == expected_revision && current != before {
                        anyhow::bail!(
                            "voice repository returned an ambiguous CAS with an invalid current aggregate"
                        );
                    }
                }
            }
        }
        anyhow::bail!(
            "voice aggregate {call_id:?} changed concurrently after {MAX_CONFLICT_RETRIES} retries"
        )
    }

    fn load_required(
        &self,
        authority_ura: &str,
        call_id: &str,
        operation: &str,
    ) -> anyhow::Result<VoiceCallAggregate> {
        let aggregate = self
            .calls
            .load(authority_ura, call_id)?
            .ok_or_else(|| anyhow::anyhow!("{operation}: call {call_id:?} not found"))?;
        aggregate.validate_repository_key(authority_ura, call_id)?;
        Ok(aggregate)
    }
}

fn mutation_command_id(envelope: &EnvelopeContext) -> String {
    format!("{}:{}", envelope.invocation_id(), envelope.ability())
}

fn now_ms() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_millis() as u64)
        .unwrap_or(0)
}

/// Register every voice.* call signaling handler.
///
/// Call signaling is realm-wide state. Paired CLI and Backend callers address
/// the realm Authority, so the live catalog publishes exactly one Authority-owned row per
/// verb. Mirroring the same state machine under Device would create two
/// descriptors for one authority and make owner-free resolution ambiguous.
pub fn register(reg: &mut AxonAbilityCatalog, provider: VoiceCallProviderAssembly) {
    register_with_repository(reg, provider.repository());
}

fn register_with_repository(
    reg: &mut AxonAbilityCatalog,
    repository: Arc<dyn VoiceCallRepository>,
) {
    use crate::daemon::ability::dispatch::OwnerKind;
    let service = Arc::new(VoiceCallService::new(repository));
    let owner = OwnerKind::RealmAuthority;
    let create = Arc::clone(&service);
    reg.register_rpc_with_envelope_and_owner(
        ABILITY_CREATE_CALL,
        owner.clone(),
        Arc::new(move |envelope, args| create.create_call(signaling_authority(&envelope)?, args)),
    );
    let show = Arc::clone(&service);
    reg.register_rpc_with_envelope_and_owner(
        ABILITY_SHOW_CALL,
        owner.clone(),
        Arc::new(move |envelope, args| show.show_call(signaling_authority(&envelope)?, args)),
    );
    let join = Arc::clone(&service);
    reg.register_rpc_with_envelope_and_owner(
        ABILITY_JOIN_CALL,
        owner.clone(),
        Arc::new(move |envelope, args| {
            let command_id = mutation_command_id(&envelope);
            join.join_call(signaling_authority(&envelope)?, &command_id, args)
        }),
    );
    let leave = Arc::clone(&service);
    reg.register_rpc_with_envelope_and_owner(
        ABILITY_LEAVE_CALL,
        owner.clone(),
        Arc::new(move |envelope, args| {
            let command_id = mutation_command_id(&envelope);
            leave.leave_call(signaling_authority(&envelope)?, &command_id, args)
        }),
    );
    let end = Arc::clone(&service);
    reg.register_rpc_with_envelope_and_owner(
        ABILITY_END_CALL,
        owner.clone(),
        Arc::new(move |envelope, args| {
            let command_id = mutation_command_id(&envelope);
            end.end_call(signaling_authority(&envelope)?, &command_id, args)
        }),
    );
    let watch = Arc::clone(&service);
    reg.register_rpc_with_envelope_and_owner(
        ABILITY_WATCH_CALL,
        owner.clone(),
        Arc::new(move |envelope, args| watch.watch_call(signaling_authority(&envelope)?, args)),
    );
    let report = Arc::clone(&service);
    reg.register_rpc_with_envelope_and_owner(
        ABILITY_REPORT_METRICS,
        owner.clone(),
        Arc::new(move |envelope, args| {
            let command_id = mutation_command_id(&envelope);
            report.report_metrics(signaling_authority(&envelope)?, &command_id, args)
        }),
    );
    let list = Arc::clone(&service);
    reg.register_rpc_with_envelope_and_owner(
        ABILITY_LIST_CALLS,
        owner,
        Arc::new(move |envelope, args| list.list_calls(signaling_authority(&envelope)?, args)),
    );
}

// ── Handlers ─────────────────────────────────────────────────────

fn require_str<'a>(args: &'a Value, key: &str, ability: &str) -> anyhow::Result<&'a str> {
    args.get(key)
        .and_then(Value::as_str)
        .ok_or_else(|| anyhow::anyhow!("{ability}: `{key}` is required"))
}

fn signaling_authority(envelope: &EnvelopeContext) -> anyhow::Result<&str> {
    let authority = crate::core::ura::parse_ura(envelope.callee()).map_err(|error| {
        anyhow::anyhow!(
            "{}: invalid Authority callee {:?}: {error}",
            envelope.ability(),
            envelope.callee()
        )
    })?;
    if authority.kind != crate::core::ura::URAKind::Authority {
        anyhow::bail!(
            "{}: voice signaling requires an Authority callee, got {:?}",
            envelope.ability(),
            envelope.callee()
        );
    }
    Ok(envelope.callee())
}

impl VoiceCallService {
    fn create_call(&self, authority_ura: &str, args: Value) -> anyhow::Result<Value> {
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
        let created_at_ms = now_ms();
        let aggregate = VoiceCallAggregate::new(
            authority_ura.to_string(),
            call_id.clone(),
            creator_participant_id.clone(),
            created_at_ms,
        );
        let inserted = self.calls.insert_if_absent(aggregate.clone())?;
        if !inserted {
            anyhow::bail!("voice.create_call: call_id {call_id:?} already exists");
        }
        let committed = self.load_required(
            authority_ura,
            &call_id,
            "voice.create_call commit verification",
        )?;
        if committed != aggregate {
            anyhow::bail!(
                "voice repository acknowledged create for ({authority_ura:?}, {call_id:?}) without storing the proposed aggregate"
            );
        }
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
            "state": VoiceCallState::Ringing.wire_name(),
            "state_code": VoiceCallState::Ringing.to_wire_i32(),
            "created_at_ms": created_at_ms,
        }))
    }

    fn show_call(&self, authority_ura: &str, args: Value) -> anyhow::Result<Value> {
        let call_id = require_str(&args, "call_id", "voice.show_call")?;
        let call = self.load_required(authority_ura, call_id, "voice.show_call")?;
        Ok(call.to_json())
    }

    fn join_call(
        &self,
        authority_ura: &str,
        command_id: &str,
        args: Value,
    ) -> anyhow::Result<Value> {
        let call_id = require_str(&args, "call_id", "voice.join_call")?.to_string();
        let participant_id = require_str(&args, "participant_id", "voice.join_call")?.to_string();
        let sdp_offer = args
            .get("sdp_offer")
            .and_then(Value::as_str)
            .map(str::to_string);
        let outcome = self.update(authority_ura, &call_id, command_id, |call, command_id| {
            call.join(
                command_id,
                participant_id.clone(),
                sdp_offer.clone(),
                now_ms(),
            )
        })?;
        let state = outcome.state.wire_name();
        crate::op_event!(
            component = voice_call,
            kind = participant_joined,
            call_id = call_id,
            participant_id = participant_id,
            participant_count = outcome.participant_count,
            state = state,
        );
        Ok(json!({
            "call_id": call_id,
            "participant_id": participant_id,
            "state": outcome.state.wire_name(),
            "state_code": outcome.state.to_wire_i32(),
        }))
    }

    fn leave_call(
        &self,
        authority_ura: &str,
        command_id: &str,
        args: Value,
    ) -> anyhow::Result<Value> {
        let call_id = require_str(&args, "call_id", "voice.leave_call")?.to_string();
        let participant_id = require_str(&args, "participant_id", "voice.leave_call")?.to_string();
        let reason = args
            .get("reason")
            .and_then(Value::as_str)
            .unwrap_or("normal")
            .to_string();
        self.update(authority_ura, &call_id, command_id, |call, command_id| {
            call.leave(command_id, &participant_id, reason.clone(), now_ms())
        })?;
        Ok(json!({"call_id": call_id, "participant_id": participant_id}))
    }

    fn end_call(
        &self,
        authority_ura: &str,
        command_id: &str,
        args: Value,
    ) -> anyhow::Result<Value> {
        let call_id = require_str(&args, "call_id", "voice.end_call")?.to_string();
        let end_reason = args
            .get("end_reason")
            .and_then(Value::as_i64)
            .map(VoiceEndReason::from_wire)
            .transpose()?
            .unwrap_or(VoiceEndReason::CallerHangup);
        let outcome = self.update(authority_ura, &call_id, command_id, |call, command_id| {
            call.end(command_id, end_reason, now_ms())
        })?;
        if outcome.already_ended {
            return Ok(json!({
                "call_id": call_id,
                "state": outcome.state.wire_name(),
                "state_code": outcome.state.to_wire_i32(),
                "already_ended": true,
            }));
        }
        Ok(json!({
            "call_id": call_id,
            "state": outcome.state.wire_name(),
            "state_code": outcome.state.to_wire_i32(),
            "end_reason": outcome.end_reason.wire_name(),
            "end_reason_code": outcome.end_reason.to_wire_i32(),
        }))
    }

    fn watch_call(&self, authority_ura: &str, args: Value) -> anyhow::Result<Value> {
        let call_id = require_str(&args, "call_id", "voice.watch_call")?;
        let call = self.load_required(authority_ura, call_id, "voice.watch_call")?;
        // v1 returns a snapshot of accumulated events. Streaming is a
        // follow-up: register through `register_stream` once the
        // subscription protocol is wired.
        Ok(json!({
            "call_id": call_id,
            "events": call.events_json(),
            "view": "snapshot",
        }))
    }

    fn report_metrics(
        &self,
        authority_ura: &str,
        command_id: &str,
        args: Value,
    ) -> anyhow::Result<Value> {
        let call_id = require_str(&args, "call_id", "voice.report_metrics")?.to_string();
        let participant_id =
            require_str(&args, "participant_id", "voice.report_metrics")?.to_string();
        let metrics = args
            .get("metrics")
            .map(VoiceNetworkMetrics::from_json)
            .transpose()?
            .unwrap_or_default();
        self.update(authority_ura, &call_id, command_id, |call, command_id| {
            call.report_metrics(command_id, &participant_id, metrics.clone(), now_ms())
        })?;
        Ok(json!({"call_id": call_id, "participant_id": participant_id, "ack": true}))
    }

    fn list_calls(&self, authority_ura: &str, _args: Value) -> anyhow::Result<Value> {
        let mut keys = std::collections::BTreeSet::new();
        let items = self
            .calls
            .list(authority_ura)?
            .into_iter()
            .map(|entry| {
                let aggregate = entry.validate_and_into_aggregate(authority_ura)?;
                if !keys.insert(aggregate.call_id().to_string()) {
                    anyhow::bail!(
                        "voice repository returned duplicate list key {:?}",
                        aggregate.call_id()
                    );
                }
                Ok(aggregate.to_json())
            })
            .collect::<anyhow::Result<Vec<_>>>()?;
        Ok(json!({ "items": items }))
    }
}

// ── Discovery surfaces (description + input_schema) ──────────────

pub fn create_call_description() -> &'static str {
    "Create a realm Authority-owned voice/video call signaling session. Returns the \
     `call_id` (auto-generated when omitted) and the initial state. The Authority \
     persists the call aggregate before acknowledging creation."
}

pub fn create_call_input_schema() -> Value {
    json!({
        "type": "object",
        "additionalProperties": false,
        "properties": {
            "call_id": { "type": "string" },
            "participant_id": { "type": "string" }
        }
    })
}

/// Result body committed into the canonical descriptor receipt contract.
/// These schemas describe the handler value carried by a successful receipt;
/// they are intentionally concrete rather than an empty/Null placeholder.
pub fn output_receipt_schema_for(ability: &str) -> Option<Value> {
    let state = json!({ "type": "string" });
    let state_code = json!({ "type": "integer" });
    let call_id = json!({ "type": "string" });
    let participant_id = json!({ "type": "string" });
    let schema = match ability {
        ABILITY_CREATE_CALL => json!({
            "type": "object",
            "required": ["call_id", "state", "state_code", "created_at_ms"],
            "additionalProperties": false,
            "properties": {
                "call_id": call_id,
                "state": state,
                "state_code": state_code,
                "created_at_ms": { "type": "integer", "minimum": 0 }
            }
        }),
        ABILITY_SHOW_CALL => json!({
            "type": "object",
            "required": ["call_id", "state", "state_code", "created_at_ms", "participants"],
            "additionalProperties": false,
            "properties": {
                "call_id": call_id,
                "state": state,
                "state_code": state_code,
                "created_at_ms": { "type": "integer", "minimum": 0 },
                "ended_at_ms": { "type": ["integer", "null"], "minimum": 0 },
                "end_reason": { "type": ["string", "null"] },
                "end_reason_code": { "type": ["integer", "null"] },
                "participants": { "type": "array", "items": { "type": "object" } }
            }
        }),
        ABILITY_JOIN_CALL => json!({
            "type": "object",
            "required": ["call_id", "participant_id", "state", "state_code"],
            "additionalProperties": false,
            "properties": {
                "call_id": call_id,
                "participant_id": participant_id,
                "state": state,
                "state_code": state_code
            }
        }),
        ABILITY_LEAVE_CALL => json!({
            "type": "object",
            "required": ["call_id", "participant_id"],
            "additionalProperties": false,
            "properties": { "call_id": call_id, "participant_id": participant_id }
        }),
        ABILITY_END_CALL => json!({
            "type": "object",
            "required": ["call_id", "state", "state_code"],
            "additionalProperties": false,
            "properties": {
                "call_id": call_id,
                "state": state,
                "state_code": state_code,
                "end_reason": { "type": "string" },
                "end_reason_code": { "type": "integer" },
                "already_ended": { "type": "boolean" }
            }
        }),
        ABILITY_WATCH_CALL => json!({
            "type": "object",
            "required": ["call_id", "events", "view"],
            "additionalProperties": false,
            "properties": {
                "call_id": call_id,
                "events": { "type": "array", "items": { "type": "object" } },
                "view": { "type": "string", "enum": ["snapshot"] }
            }
        }),
        ABILITY_REPORT_METRICS => json!({
            "type": "object",
            "required": ["call_id", "participant_id", "ack"],
            "additionalProperties": false,
            "properties": {
                "call_id": call_id,
                "participant_id": participant_id,
                "ack": { "type": "boolean" }
            }
        }),
        ABILITY_LIST_CALLS => json!({
            "type": "object",
            "required": ["items"],
            "additionalProperties": false,
            "properties": { "items": { "type": "array", "items": { "type": "object" } } }
        }),
        _ => return None,
    };
    Some(schema)
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
    "Add a participant to an existing call. The call transitions from \
     `VOICE_CALL_STATE_RINGING` to `VOICE_CALL_STATE_ACTIVE` only when at \
     least two participants are present; an optional creator supplied to \
     `voice.create_call` counts as one participant. Response `state` carries \
     the product contract name and `state_code` carries the numeric wire \
     value. Optional `sdp_offer` carries the participant's SDP; ICE \
     candidates stream in via subsequent `voice.report_metrics`-style \
     updates."
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
    "Mark a participant as having left an open call. Leaving does not end or \
     reclassify the call; only `voice.end_call` transitions it to the terminal \
     `VOICE_CALL_STATE_ENDED` state."
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
    "List the call signaling sessions currently owned by this realm Authority. \
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
    use crate::daemon::ability::builtins::resources::voice_contract::{
        TestVoiceCallRepository, VoiceCallRepositoryQualification,
    };
    use std::sync::atomic::{AtomicBool, AtomicUsize, Ordering};

    const HUB_AUTHORITY: &str = "easynet:///r/voice-test/authority";

    fn fresh_call_id(prefix: &str) -> String {
        format!("{prefix}-{:x}", now_ms())
    }

    fn service() -> VoiceCallService {
        VoiceCallService::new(Arc::new(TestVoiceCallRepository::default()))
    }

    #[derive(Debug, Clone, Default)]
    struct ConflictOnceRepository {
        inner: TestVoiceCallRepository,
        reject_next_cas: Arc<AtomicBool>,
        cas_attempts: Arc<AtomicUsize>,
    }

    #[derive(Debug, Clone, Default)]
    struct CommitThenAmbiguousRepository {
        inner: TestVoiceCallRepository,
        ambiguous_once: Arc<AtomicBool>,
        cas_attempts: Arc<AtomicUsize>,
    }

    #[derive(Debug)]
    struct MaliciousKeyRepository {
        requested: VoiceCallAggregate,
        foreign: VoiceCallAggregate,
        corrupt_reads: bool,
        cas_acknowledged: AtomicBool,
        cas_attempts: AtomicUsize,
    }

    impl MaliciousKeyRepository {
        fn read_attack(requested: VoiceCallAggregate, foreign: VoiceCallAggregate) -> Self {
            Self {
                requested,
                foreign,
                corrupt_reads: true,
                cas_acknowledged: AtomicBool::new(false),
                cas_attempts: AtomicUsize::new(0),
            }
        }

        fn write_attack(requested: VoiceCallAggregate, foreign: VoiceCallAggregate) -> Self {
            Self {
                requested,
                foreign,
                corrupt_reads: false,
                cas_acknowledged: AtomicBool::new(false),
                cas_attempts: AtomicUsize::new(0),
            }
        }
    }

    impl VoiceCallRepository for MaliciousKeyRepository {
        fn qualification(&self) -> VoiceCallRepositoryQualification {
            VoiceCallRepositoryQualification::unqualified("malicious-test")
        }

        fn insert_if_absent(&self, _aggregate: VoiceCallAggregate) -> anyhow::Result<bool> {
            Ok(false)
        }

        fn load(
            &self,
            _authority_ura: &str,
            _call_id: &str,
        ) -> anyhow::Result<Option<VoiceCallAggregate>> {
            if self.corrupt_reads || self.cas_acknowledged.load(Ordering::SeqCst) {
                Ok(Some(self.foreign.clone()))
            } else {
                Ok(Some(self.requested.clone()))
            }
        }

        fn list(&self, authority_ura: &str) -> anyhow::Result<Vec<VoiceCallRepositoryEntry>> {
            Ok(vec![VoiceCallRepositoryEntry::new(
                authority_ura.to_string(),
                self.requested.call_id().to_string(),
                self.foreign.clone(),
            )])
        }

        fn compare_and_swap(
            &self,
            _authority_ura: &str,
            _call_id: &str,
            _expected_revision: u64,
            _replacement: VoiceCallAggregate,
        ) -> anyhow::Result<VoiceCallCasOutcome> {
            self.cas_attempts.fetch_add(1, Ordering::SeqCst);
            self.cas_acknowledged.store(true, Ordering::SeqCst);
            Ok(VoiceCallCasOutcome::Committed(self.foreign.clone()))
        }
    }

    impl ConflictOnceRepository {
        fn armed() -> Self {
            Self {
                inner: TestVoiceCallRepository::default(),
                reject_next_cas: Arc::new(AtomicBool::new(true)),
                cas_attempts: Arc::new(AtomicUsize::new(0)),
            }
        }
    }

    impl VoiceCallRepository for ConflictOnceRepository {
        fn qualification(&self) -> VoiceCallRepositoryQualification {
            self.inner.qualification()
        }

        fn insert_if_absent(&self, aggregate: VoiceCallAggregate) -> anyhow::Result<bool> {
            self.inner.insert_if_absent(aggregate)
        }

        fn load(
            &self,
            authority_ura: &str,
            call_id: &str,
        ) -> anyhow::Result<Option<VoiceCallAggregate>> {
            self.inner.load(authority_ura, call_id)
        }

        fn list(&self, authority_ura: &str) -> anyhow::Result<Vec<VoiceCallRepositoryEntry>> {
            self.inner.list(authority_ura)
        }

        fn compare_and_swap(
            &self,
            authority_ura: &str,
            call_id: &str,
            expected_revision: u64,
            replacement: VoiceCallAggregate,
        ) -> anyhow::Result<VoiceCallCasOutcome> {
            self.cas_attempts.fetch_add(1, Ordering::SeqCst);
            if self.reject_next_cas.swap(false, Ordering::SeqCst) {
                let mut current = self
                    .inner
                    .load(authority_ura, call_id)?
                    .expect("fixture current aggregate");
                current.join(
                    "concurrent:join",
                    "concurrent-participant".to_string(),
                    None,
                    now_ms(),
                )?;
                current.bump_revision()?;
                let outcome = self.inner.compare_and_swap(
                    authority_ura,
                    call_id,
                    expected_revision,
                    current,
                )?;
                return match outcome {
                    VoiceCallCasOutcome::Committed(current) => {
                        Ok(VoiceCallCasOutcome::Current(current))
                    }
                    other => Ok(other),
                };
            }
            self.inner
                .compare_and_swap(authority_ura, call_id, expected_revision, replacement)
        }
    }

    impl VoiceCallRepository for CommitThenAmbiguousRepository {
        fn qualification(&self) -> VoiceCallRepositoryQualification {
            self.inner.qualification()
        }

        fn insert_if_absent(&self, aggregate: VoiceCallAggregate) -> anyhow::Result<bool> {
            self.inner.insert_if_absent(aggregate)
        }

        fn load(
            &self,
            authority_ura: &str,
            call_id: &str,
        ) -> anyhow::Result<Option<VoiceCallAggregate>> {
            self.inner.load(authority_ura, call_id)
        }

        fn list(&self, authority_ura: &str) -> anyhow::Result<Vec<VoiceCallRepositoryEntry>> {
            self.inner.list(authority_ura)
        }

        fn compare_and_swap(
            &self,
            authority_ura: &str,
            call_id: &str,
            expected_revision: u64,
            replacement: VoiceCallAggregate,
        ) -> anyhow::Result<VoiceCallCasOutcome> {
            self.cas_attempts.fetch_add(1, Ordering::SeqCst);
            let outcome = self.inner.compare_and_swap(
                authority_ura,
                call_id,
                expected_revision,
                replacement,
            )?;
            if matches!(outcome, VoiceCallCasOutcome::Committed(_))
                && !self.ambiguous_once.swap(true, Ordering::SeqCst)
            {
                return Ok(VoiceCallCasOutcome::Ambiguous);
            }
            Ok(outcome)
        }
    }

    #[test]
    fn create_show_join_metrics_end_round_trip() {
        // Full happy path — covers every handler in one test so a
        // regression in any handler trips this.
        let service = service();
        let cid = fresh_call_id("rt");
        let _ = service
            .create_call(
                HUB_AUTHORITY,
                json!({
                    "call_id": cid,
                    "participant_id": "alice",
                }),
            )
            .expect("create");
        let s1 = service
            .show_call(HUB_AUTHORITY, json!({"call_id": cid}))
            .unwrap();
        assert_eq!(
            s1.get("state").and_then(Value::as_str),
            Some("VOICE_CALL_STATE_RINGING")
        );
        assert_eq!(s1.get("state_code"), Some(&json!(1)));

        service
            .join_call(
                HUB_AUTHORITY,
                "round-trip:join",
                json!({
                    "call_id": cid,
                    "participant_id": "bob",
                    "sdp_offer": "v=0",
                }),
            )
            .expect("join");
        let s2 = service
            .show_call(HUB_AUTHORITY, json!({"call_id": cid}))
            .unwrap();
        assert_eq!(
            s2.get("state").and_then(Value::as_str),
            Some("VOICE_CALL_STATE_ACTIVE")
        );
        assert_eq!(s2.get("state_code"), Some(&json!(2)));

        service
            .report_metrics(
                HUB_AUTHORITY,
                "round-trip:metrics",
                json!({
                    "call_id": cid,
                    "participant_id": "bob",
                    "metrics": { "rtt_ms": 42 },
                }),
            )
            .expect("metrics");

        let watch = service
            .watch_call(HUB_AUTHORITY, json!({"call_id": cid}))
            .unwrap();
        let events = watch.get("events").and_then(Value::as_array).unwrap();
        assert!(
            events
                .iter()
                .any(|e| e.get("type") == Some(&json!("joined"))),
            "watch must surface join event: {watch}"
        );

        service
            .end_call(HUB_AUTHORITY, "round-trip:end", json!({"call_id": cid}))
            .expect("end");
        let s3 = service
            .show_call(HUB_AUTHORITY, json!({"call_id": cid}))
            .unwrap();
        assert_eq!(
            s3.get("state").and_then(Value::as_str),
            Some("VOICE_CALL_STATE_ENDED")
        );
        assert_eq!(s3.get("state_code"), Some(&json!(5)));
    }

    #[test]
    fn show_unknown_call_errors_clearly() {
        let err = service()
            .show_call(HUB_AUTHORITY, json!({"call_id": "no-such-call"}))
            .unwrap_err();
        assert!(format!("{err}").contains("not found"));
    }

    #[test]
    fn end_call_is_idempotent() {
        let service = service();
        let cid = fresh_call_id("idempotent");
        service
            .create_call(HUB_AUTHORITY, json!({"call_id": cid}))
            .unwrap();
        service
            .end_call(HUB_AUTHORITY, "idempotent:end", json!({"call_id": cid}))
            .unwrap();
        let r = service
            .end_call(HUB_AUTHORITY, "idempotent:end", json!({"call_id": cid}))
            .unwrap();
        assert_eq!(r.get("already_ended"), Some(&json!(true)));
    }

    #[test]
    fn join_after_end_is_rejected() {
        let service = service();
        let cid = fresh_call_id("after-end");
        service
            .create_call(HUB_AUTHORITY, json!({"call_id": cid}))
            .unwrap();
        service
            .end_call(HUB_AUTHORITY, "after-end:end", json!({"call_id": cid}))
            .unwrap();
        let err = service
            .join_call(
                HUB_AUTHORITY,
                "after-end:join",
                json!({
                    "call_id": cid,
                    "participant_id": "late",
                }),
            )
            .unwrap_err();
        assert!(format!("{err}").contains("already ended"));
    }

    #[test]
    fn ended_rejects_leave_and_metrics_without_changing_revision_or_events() {
        let provider = Arc::new(TestVoiceCallRepository::default());
        let service = VoiceCallService::new(provider.clone());
        let call_id = fresh_call_id("ended-terminal");
        service
            .create_call(
                HUB_AUTHORITY,
                json!({"call_id": call_id, "participant_id": "alice"}),
            )
            .expect("create");
        service
            .end_call(HUB_AUTHORITY, "terminal:end", json!({"call_id": call_id}))
            .expect("end");
        let terminal = provider
            .load(HUB_AUTHORITY, &call_id)
            .expect("load terminal aggregate")
            .expect("terminal aggregate exists");

        let leave_error = service
            .leave_call(
                HUB_AUTHORITY,
                "terminal:leave",
                json!({"call_id": call_id, "participant_id": "alice"}),
            )
            .expect_err("leave after end must fail");
        assert!(leave_error.to_string().contains("already ended"));
        let metrics_error = service
            .report_metrics(
                HUB_AUTHORITY,
                "terminal:metrics",
                json!({
                    "call_id": call_id,
                    "participant_id": "alice",
                    "metrics": {"rtt_ms": 12}
                }),
            )
            .expect_err("metrics after end must fail");
        assert!(metrics_error.to_string().contains("already ended"));

        let after = provider
            .load(HUB_AUTHORITY, &call_id)
            .expect("reload terminal aggregate")
            .expect("terminal aggregate exists");
        assert_eq!(after, terminal);
    }

    #[test]
    fn participant_lifecycle_rejects_duplicate_and_reverse_transitions_without_mutation() {
        let provider = Arc::new(TestVoiceCallRepository::default());
        let service = VoiceCallService::new(provider.clone());
        let call_id = fresh_call_id("participant-fsm");
        service
            .create_call(
                HUB_AUTHORITY,
                json!({"call_id": call_id, "participant_id": "alice"}),
            )
            .expect("create");

        let before_duplicate = provider.load(HUB_AUTHORITY, &call_id).unwrap().unwrap();
        assert!(service
            .join_call(
                HUB_AUTHORITY,
                "participant:duplicate-alice",
                json!({"call_id": call_id, "participant_id": "alice"}),
            )
            .is_err());
        assert_eq!(
            provider.load(HUB_AUTHORITY, &call_id).unwrap().unwrap(),
            before_duplicate
        );

        service
            .join_call(
                HUB_AUTHORITY,
                "participant:join-bob",
                json!({"call_id": call_id, "participant_id": "bob"}),
            )
            .expect("bob joins");
        service
            .leave_call(
                HUB_AUTHORITY,
                "participant:leave-bob",
                json!({"call_id": call_id, "participant_id": "bob"}),
            )
            .expect("bob leaves");
        let left = provider.load(HUB_AUTHORITY, &call_id).unwrap().unwrap();

        for error in [
            service.leave_call(
                HUB_AUTHORITY,
                "participant:duplicate-leave-bob",
                json!({"call_id": call_id, "participant_id": "bob"}),
            ),
            service.report_metrics(
                HUB_AUTHORITY,
                "participant:left-metrics-bob",
                json!({
                    "call_id": call_id,
                    "participant_id": "bob",
                    "metrics": {"rtt_ms": 1}
                }),
            ),
            service.join_call(
                HUB_AUTHORITY,
                "participant:rejoin-bob",
                json!({"call_id": call_id, "participant_id": "bob"}),
            ),
        ] {
            assert!(error.is_err());
            assert_eq!(
                provider.load(HUB_AUTHORITY, &call_id).unwrap().unwrap(),
                left
            );
        }
    }

    #[test]
    fn malicious_provider_cannot_substitute_repository_keys_on_read_or_write() {
        let call_id = fresh_call_id("requested");
        let requested = VoiceCallAggregate::new(
            HUB_AUTHORITY.to_string(),
            call_id.clone(),
            Some("alice".to_string()),
            1,
        );
        let foreign = VoiceCallAggregate::new(
            "easynet:///r/foreign-voice/authority".to_string(),
            "foreign-call".to_string(),
            Some("mallory".to_string()),
            1,
        );

        let read_provider = Arc::new(MaliciousKeyRepository::read_attack(
            requested.clone(),
            foreign.clone(),
        ));
        let read_service = VoiceCallService::new(read_provider);
        let read_error = read_service
            .show_call(HUB_AUTHORITY, json!({"call_id": call_id}))
            .expect_err("foreign read must fail closed");
        assert!(read_error.to_string().contains("repository key mismatch"));

        let foreign_call = VoiceCallAggregate::new(
            HUB_AUTHORITY.to_string(),
            "foreign-call-in-same-realm".to_string(),
            Some("mallory".to_string()),
            1,
        );
        let call_substitution = VoiceCallService::new(Arc::new(
            MaliciousKeyRepository::read_attack(requested.clone(), foreign_call),
        ));
        let call_error = call_substitution
            .show_call(HUB_AUTHORITY, json!({"call_id": call_id}))
            .expect_err("same-realm foreign call must fail closed");
        assert!(call_error.to_string().contains("repository key mismatch"));

        let write_provider = Arc::new(MaliciousKeyRepository::write_attack(requested, foreign));
        let write_service = VoiceCallService::new(write_provider.clone());
        let write_error = write_service
            .join_call(
                HUB_AUTHORITY,
                "malicious:join-bob",
                json!({"call_id": call_id, "participant_id": "bob"}),
            )
            .expect_err("foreign post-CAS read must fail closed");
        assert!(write_error.to_string().contains("repository key mismatch"));
        assert_eq!(write_provider.cas_attempts.load(Ordering::SeqCst), 1);

        let list_error = write_service
            .list_calls(HUB_AUTHORITY, json!({}))
            .expect_err("foreign list row must fail closed");
        assert!(list_error.to_string().contains("repository key mismatch"));
    }

    #[test]
    fn cas_conflict_retry_commits_one_revision_and_one_event() {
        let provider = Arc::new(ConflictOnceRepository::armed());
        let service = VoiceCallService::new(provider.clone());
        let call_id = fresh_call_id("cas-retry");
        service
            .create_call(
                HUB_AUTHORITY,
                json!({"call_id": call_id, "participant_id": "alice"}),
            )
            .expect("create");
        service
            .join_call(
                HUB_AUTHORITY,
                "cas-retry:join-bob",
                json!({"call_id": call_id, "participant_id": "bob"}),
            )
            .expect("join after one forced CAS conflict");

        let aggregate = provider
            .load(HUB_AUTHORITY, &call_id)
            .expect("load aggregate")
            .expect("aggregate exists");
        assert_eq!(aggregate.revision(), 3);
        assert_eq!(
            aggregate
                .events_json()
                .iter()
                .filter(|event| { event.get("command_id") == Some(&json!("cas-retry:join-bob")) })
                .count(),
            1
        );
        assert_eq!(provider.cas_attempts.load(Ordering::SeqCst), 2);
    }

    #[test]
    fn commit_then_ambiguous_is_detected_by_command_id_without_duplicate_event() {
        let provider = Arc::new(CommitThenAmbiguousRepository::default());
        let service = VoiceCallService::new(provider.clone());
        let call_id = fresh_call_id("ambiguous");
        service
            .create_call(
                HUB_AUTHORITY,
                json!({"call_id": call_id, "participant_id": "alice"}),
            )
            .unwrap();
        service
            .join_call(
                HUB_AUTHORITY,
                "ambiguous:join-bob",
                json!({"call_id": call_id, "participant_id": "bob"}),
            )
            .expect("ambiguous acknowledgement resolves from committed command id");
        let aggregate = provider.load(HUB_AUTHORITY, &call_id).unwrap().unwrap();
        assert_eq!(aggregate.revision(), 2);
        assert_eq!(aggregate.events_json().len(), 1);
        assert_eq!(provider.cas_attempts.load(Ordering::SeqCst), 1);
    }

    #[test]
    fn create_without_call_id_auto_mints_one() {
        let resp = service().create_call(HUB_AUTHORITY, json!({})).unwrap();
        let cid = resp.get("call_id").and_then(Value::as_str).unwrap();
        assert!(cid.starts_with("call-"));
    }

    #[test]
    fn first_join_without_a_creator_remains_ringing() {
        let service = service();
        let call_id = fresh_call_id("first-participant");
        service
            .create_call(HUB_AUTHORITY, json!({"call_id": call_id}))
            .expect("create without creator");
        let joined = service
            .join_call(
                HUB_AUTHORITY,
                "first-participant:join-alice",
                json!({"call_id": call_id, "participant_id": "alice"}),
            )
            .expect("first participant joins");
        assert_eq!(joined["state"], "VOICE_CALL_STATE_RINGING");
        assert_eq!(joined["state_code"], 1);
    }

    #[test]
    fn active_call_returns_to_ringing_below_two_joined_participants() {
        let service = service();
        let call_id = fresh_call_id("active-leave");
        service
            .create_call(
                HUB_AUTHORITY,
                json!({"call_id": call_id, "participant_id": "alice"}),
            )
            .unwrap();
        service
            .join_call(
                HUB_AUTHORITY,
                "active-leave:join-bob",
                json!({"call_id": call_id, "participant_id": "bob"}),
            )
            .unwrap();
        service
            .leave_call(
                HUB_AUTHORITY,
                "active-leave:leave-bob",
                json!({"call_id": call_id, "participant_id": "bob"}),
            )
            .unwrap();

        let shown = service
            .show_call(HUB_AUTHORITY, json!({"call_id": call_id}))
            .unwrap();
        assert_eq!(shown["state"], "VOICE_CALL_STATE_RINGING");
        assert_eq!(shown["state_code"], 1);
    }

    #[test]
    fn list_calls_returns_created_call() {
        let service = service();
        let cid = fresh_call_id("list");
        service
            .create_call(HUB_AUTHORITY, json!({"call_id": cid}))
            .unwrap();
        let listed = service.list_calls(HUB_AUTHORITY, json!({})).unwrap();
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

    #[test]
    fn call_ids_are_scoped_by_authority() {
        let service = service();
        let call_id = fresh_call_id("authority");
        service
            .create_call(HUB_AUTHORITY, json!({"call_id": call_id}))
            .unwrap();
        service
            .create_call(
                "easynet:///r/other-voice-test/authority",
                json!({"call_id": call_id}),
            )
            .expect("another realm Authority may use the same local call id");

        let primary = service.list_calls(HUB_AUTHORITY, json!({})).unwrap();
        let secondary = service
            .list_calls("easynet:///r/other-voice-test/authority", json!({}))
            .unwrap();
        assert_eq!(primary["items"].as_array().map(Vec::len), Some(1));
        assert_eq!(secondary["items"].as_array().map(Vec::len), Some(1));
    }

    #[test]
    fn voice_handler_rejects_non_authority_callee() {
        let device = crate::core::ura::device_ura("voice-test", "device-1");
        let envelope = EnvelopeContext::for_test_targeted_ability(
            &device,
            &device,
            ABILITY_CREATE_CALL,
            &device,
        );
        let error =
            signaling_authority(&envelope).expect_err("Device must not own voice signaling");
        assert!(error.to_string().contains("requires an Authority callee"));
    }
}
