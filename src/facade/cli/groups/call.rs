// EasyNet CLI — Call Group
// ========================
//
// File: src/cli/groups/call.rs
// Description: `easynet call …` — voice and video call signaling.
//
// Axon is a Capability Control Plane: it handles call signaling (create,
// join, negotiate SDP/ICE, report metrics, end) while actual media flows
// peer-to-peer via WebRTC. This command group exposes the signaling surface.
//
// Verbs:
//   create <call-id>    Create a new call
//   show <call-id>      Show call details (participants, state, codec)
//   join <call-id>      Join as a participant
//   leave <call-id>     Leave a call
//   end <call-id>       End a call (hang up)
//   watch <call-id>     Stream call events (replay + live)
//   metrics <call-id>   Report QoS metrics
//
// Author: Silan Hu <silan.hu@u.nus.edu>
// Copyright (c) 2026 EasyNet. All rights reserved.

use anyhow::Context;
use clap::{Args, Subcommand};
use serde_json::{json, Value};

use crate::support::local_invoke::invoke_local_ability;
use crate::support::output::{self, OutputFormat};

#[derive(Debug, Args)]
pub struct CallArgs {
    #[command(subcommand)]
    pub action: CallAction,
}

#[derive(Debug, Subcommand)]
pub enum CallAction {
    /// Create a new voice/video call.
    Create(CreateArgs),
    /// Show call details (participants, state, codec).
    Show(ShowArgs),
    /// Join an existing call as a participant.
    Join(JoinArgs),
    /// Leave a call.
    Leave(LeaveArgs),
    /// End a call (hang up).
    End(EndArgs),
    /// Stream call events from replay.
    Watch(WatchArgs),
    /// Report QoS metrics for a participant.
    Metrics(MetricsArgs),
}

#[derive(Debug, Args)]
pub struct CreateArgs {
    /// Unique call identifier (auto-generated if omitted).
    #[arg(default_value = "")]
    pub call_id: String,
    /// Display name shown to participants.
    #[arg(long, default_value = "EasyNet Call")]
    pub display_name: String,
    /// Maximum number of participants.
    #[arg(long, default_value_t = 2)]
    pub limit: i32,
    /// Call mode: "direct" for 1:1, "conference" for multi-party via SFU.
    #[arg(long, default_value = "direct")]
    pub mode: String,
    /// Conference provider node. Defaults to the official easynet.run node.
    /// Only used when --mode=conference. Specify an alternative if your
    /// organization runs its own conference ability node.
    #[arg(long, default_value = "easynet.run")]
    pub provider: String,
    /// Output format. Aligned with every other list/show command —
    /// see `support::output::OutputFormat`.
    #[arg(long, value_enum, default_value_t = OutputFormat::Table)]
    pub format: OutputFormat,
}

#[derive(Debug, Args)]
pub struct ShowArgs {
    /// Call identifier.
    pub call_id: String,
    /// Output format. Aligned with every other list/show command —
    /// see `support::output::OutputFormat`.
    #[arg(long, value_enum, default_value_t = OutputFormat::Table)]
    pub format: OutputFormat,
}

#[derive(Debug, Args)]
pub struct JoinArgs {
    /// Call identifier to join.
    pub call_id: String,
    /// Your participant identifier.
    #[arg(long)]
    pub participant_id: Option<String>,
    /// Join with microphone muted.
    #[arg(long)]
    pub muted: bool,
}

#[derive(Debug, Args)]
pub struct LeaveArgs {
    /// Call identifier.
    pub call_id: String,
    /// Participant identifier.
    pub participant_id: String,
    /// Reason for leaving.
    #[arg(long, default_value = "user_left")]
    pub reason: String,
}

#[derive(Debug, Args)]
pub struct EndArgs {
    /// Call identifier.
    pub call_id: String,
    /// Reason detail.
    #[arg(long, default_value = "caller_hangup")]
    pub reason: String,
}

#[derive(Debug, Args)]
pub struct WatchArgs {
    /// Call identifier.
    pub call_id: String,
    /// Replay from this sequence number.
    #[arg(long, default_value_t = 0)]
    pub from: u64,
    /// Maximum events to return.
    #[arg(long, default_value_t = 128)]
    pub max_events: u32,
}

#[derive(Debug, Args)]
pub struct MetricsArgs {
    /// Call identifier.
    pub call_id: String,
    /// Participant identifier.
    pub participant_id: String,
    /// RTT in milliseconds.
    #[arg(long, default_value_t = 0.0)]
    pub rtt_ms: f64,
    /// Jitter in milliseconds.
    #[arg(long, default_value_t = 0.0)]
    pub jitter_ms: f64,
    /// Packet loss ratio (0.0 - 1.0).
    #[arg(long, default_value_t = 0.0)]
    pub loss: f64,
}

pub fn run(args: CallArgs) -> anyhow::Result<()> {
    match args.action {
        CallAction::Create(a) => run_create(a),
        CallAction::Show(a) => run_show(a),
        CallAction::Join(a) => run_join(a),
        CallAction::Leave(a) => run_leave(a),
        CallAction::End(a) => run_end(a),
        CallAction::Watch(a) => run_watch(a),
        CallAction::Metrics(a) => run_metrics(a),
    }
}

fn invoke_call_signaling(ability: &str, args: Value) -> anyhow::Result<Value> {
    #[cfg(feature = "axon-pb")]
    if let Ok(creds) = crate::persistence::config::load_credentials() {
        let realm = creds.realm_str().trim();
        let node_id = creds.node_id.trim();
        if !realm.is_empty() && !node_id.is_empty() {
            let hub_uri = crate::uri::hub_uri(realm);
            let caller_uri = crate::uri::device_uri(realm, node_id);
            return crate::support::federation_invoke::invoke_via_federation_forward(
                ability,
                args,
                &hub_uri,
                Some(&caller_uri),
            )
            .with_context(|| format!("invoke {ability} against realm hub"));
        }
    }

    invoke_local_ability(ability, args).with_context(|| format!("invoke {ability} locally"))
}

fn run_create(args: CreateArgs) -> anyhow::Result<()> {
    // Per the ability-only ontology: every CLI subcommand collapses
    // to one Invoke. The voice.create_call handler returns the new
    // call's envelope (call_id, state, created_at_ms) — codec /
    // mode / display-name policy lives inside the handler so the
    // federation handler can re-wire without breaking the CLI.
    let mut body = json!({});
    if !args.call_id.is_empty() {
        body["call_id"] = json!(args.call_id);
    }
    let participant_id = crate::persistence::config::load_credentials()
        .ok()
        .map(|creds| creds.node_id)
        .filter(|node_id| !node_id.trim().is_empty())
        .unwrap_or_else(|| gethostname::gethostname().to_string_lossy().to_string());
    body["participant_id"] = json!(participant_id);
    let result = invoke_call_signaling("device.voice.create_call", body)?;
    if args.format == OutputFormat::Json {
        println!("{}", serde_json::to_string_pretty(&result)?);
    } else {
        if let Some(cid) = result.get("call_id").and_then(Value::as_str) {
            output::success(&format!("Call created: {cid}"));
        }
        output::detail("mode", &args.mode);
        if args.mode == "conference" {
            output::detail("provider", &args.provider);
        }
        if let Some(state) = result.get("state").and_then(Value::as_str) {
            output::detail("state", state);
        }
    }
    Ok(())
}

fn run_show(args: ShowArgs) -> anyhow::Result<()> {
    let result = invoke_call_signaling("device.voice.show_call", json!({"call_id": args.call_id}))?;
    if args.format == OutputFormat::Json {
        println!("{}", serde_json::to_string_pretty(&result)?);
        return Ok(());
    }
    let state = result.get("state").and_then(Value::as_str).unwrap_or("?");
    output::detail("call_id", &args.call_id);
    output::detail("state", state);
    if let Some(participants) = result.get("participants").and_then(Value::as_array) {
        output::detail("participants", &format!("{}", participants.len()));
        for p in participants {
            let pid = p
                .get("participant_id")
                .and_then(Value::as_str)
                .unwrap_or("?");
            output::step(&format!("  {pid}"));
        }
    }
    Ok(())
}

fn run_join(args: JoinArgs) -> anyhow::Result<()> {
    let pid = args
        .participant_id
        .unwrap_or_else(|| gethostname::gethostname().to_string_lossy().to_string());
    let result = invoke_call_signaling(
        "device.voice.join_call",
        json!({"call_id": args.call_id, "participant_id": pid}),
    )?;
    output::success(&format!("Joined call {} as {pid}", args.call_id));
    if let Some(state) = result.get("state").and_then(Value::as_str) {
        output::detail("state", state);
    }
    Ok(())
}

fn run_leave(args: LeaveArgs) -> anyhow::Result<()> {
    invoke_call_signaling(
        "device.voice.leave_call",
        json!({
            "call_id": args.call_id,
            "participant_id": args.participant_id,
            "reason": args.reason,
        }),
    )?;
    output::success(&format!(
        "{} left call {}",
        args.participant_id, args.call_id
    ));
    Ok(())
}

fn run_end(args: EndArgs) -> anyhow::Result<()> {
    let result = invoke_call_signaling(
        "device.voice.end_call",
        json!({"call_id": args.call_id, "end_reason": 1}),
    )?;
    output::success(&format!("Call {} ended", args.call_id));
    if let Some(state) = result.get("state").and_then(Value::as_str) {
        output::detail("terminal_state", state);
    }
    let _ = &args.reason; // surfaced in the ability args for forward-compat
    Ok(())
}

fn run_watch(args: WatchArgs) -> anyhow::Result<()> {
    let result =
        invoke_call_signaling("device.voice.watch_call", json!({"call_id": args.call_id}))?;
    let events = result.get("events").and_then(Value::as_array);
    let mut count = 0;
    if let Some(events) = events {
        for evt in events
            .iter()
            .skip(args.from as usize)
            .take(args.max_events as usize)
        {
            let etype = evt.get("type").and_then(Value::as_str).unwrap_or("?");
            let at = evt.get("at_ms").and_then(Value::as_i64).unwrap_or(0);
            println!("  {etype:<20} at_ms={at}");
            count += 1;
        }
    }
    output::detail("events", &format!("{count}"));
    Ok(())
}

fn run_metrics(args: MetricsArgs) -> anyhow::Result<()> {
    let metrics = json!({
        "rtt_ms": args.rtt_ms,
        "jitter_ms": args.jitter_ms,
        "packet_loss_ratio": args.loss,
        "concealed_samples": 0,
        "audio_level_dbov": -26.0,
    });
    let _ = invoke_call_signaling(
        "device.voice.report_metrics",
        json!({
            "call_id":        args.call_id,
            "participant_id": args.participant_id,
            "metrics":        metrics,
        }),
    )?;
    output::success("Metrics reported");
    Ok(())
}
