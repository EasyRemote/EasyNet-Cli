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

use crate::shared::{self, output};

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
    /// Emit raw JSON instead of the human-readable view.
    #[arg(long)]
    pub json: bool,
}

#[derive(Debug, Args)]
pub struct ShowArgs {
    /// Call identifier.
    pub call_id: String,
    /// Emit raw JSON.
    #[arg(long)]
    pub json: bool,
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

fn default_codec() -> Value {
    json!({
        "codec": "opus",
        "sample_rate_hz": 48000,
        "channels": 1,
        "ptime_ms": 20,
        "max_bitrate_kbps": 64,
        "fec_enabled": true,
        "dtx_enabled": true,
    })
}

fn run_create(args: CreateArgs) -> anyhow::Result<()> {
    let (br, rt) = shared::connect_bridge()?;
    let tenant = rt.tenant_or_default();
    let call_id = if args.call_id.is_empty() {
        format!("call-{}", uuid::Uuid::new_v4())
    } else {
        args.call_id
    };

    let metadata = if args.mode == "conference" {
        json!({
            "mode": "conference",
            "ability": "easynet.conference",
            "provider": &args.provider,
        })
    } else {
        json!({ "mode": "direct" })
    };

    let result = br
        .create_voice_call(
            tenant,
            &call_id,
            &args.display_name,
            args.limit,
            &default_codec(),
            &metadata,
        )
        .context("create_voice_call")?;

    if args.json {
        println!("{}", serde_json::to_string_pretty(&result)?);
    } else {
        output::success(&format!("Call created: {call_id}"));
        output::detail("mode", &args.mode);
        if args.mode == "conference" {
            output::detail("provider", &args.provider);
        }
        if let Some(call) = result.get("call") {
            let state = call.get("state").and_then(Value::as_i64).unwrap_or(0);
            output::detail("state", &format!("{state}"));
        }
    }
    Ok(())
}

fn run_show(args: ShowArgs) -> anyhow::Result<()> {
    let (br, rt) = shared::connect_bridge()?;
    let tenant = rt.tenant_or_default();

    let result = br
        .get_voice_call(tenant, &args.call_id)
        .context("get_voice_call")?;

    if args.json {
        println!("{}", serde_json::to_string_pretty(&result)?);
        return Ok(());
    }

    let call = result.get("call").cloned().unwrap_or(json!(null));
    let state = call.get("state").and_then(Value::as_i64).unwrap_or(0);
    let state_name = match state {
        0 => "UNKNOWN",
        1 => "RINGING",
        2 => "ACTIVE",
        3 => "DEGRADED",
        4 => "ENDING",
        5 => "ENDED",
        6 => "FAILED",
        _ => "?",
    };
    output::detail("call_id", &args.call_id);
    output::detail("state", &format!("{state_name} ({state})"));

    if let Some(participants) = call.get("participants").and_then(Value::as_array) {
        output::detail("participants", &format!("{}", participants.len()));
        for p in participants {
            let pid = p.get("participant_id").and_then(Value::as_str).unwrap_or("?");
            let muted = p.get("muted").and_then(Value::as_bool).unwrap_or(false);
            let mute_tag = if muted { " (muted)" } else { "" };
            output::step(&format!("  {pid}{mute_tag}"));
        }
    }
    Ok(())
}

fn run_join(args: JoinArgs) -> anyhow::Result<()> {
    let (br, rt) = shared::connect_bridge()?;
    let tenant = rt.tenant_or_default();
    let pid = args.participant_id.unwrap_or_else(|| {
        gethostname::gethostname().to_string_lossy().to_string()
    });

    let result = br
        .join_voice_call(
            tenant,
            &args.call_id,
            &pid,
            2, // VOICE_TRANSPORT_WEBRTC_SFU
            &default_codec(),
            args.muted,
        )
        .context("join_voice_call")?;

    output::success(&format!("Joined call {} as {pid}", args.call_id));
    let call = result.get("call").cloned().unwrap_or(json!(null));
    if let Some(count) = call.get("participants").and_then(Value::as_array).map(|a| a.len()) {
        output::detail("participants", &format!("{count}"));
    }
    Ok(())
}

fn run_leave(args: LeaveArgs) -> anyhow::Result<()> {
    let (br, rt) = shared::connect_bridge()?;
    let tenant = rt.tenant_or_default();

    br.leave_voice_call(tenant, &args.call_id, &args.participant_id, &args.reason)
        .context("leave_voice_call")?;

    output::success(&format!(
        "{} left call {}", args.participant_id, args.call_id
    ));
    Ok(())
}

fn run_end(args: EndArgs) -> anyhow::Result<()> {
    let (br, rt) = shared::connect_bridge()?;
    let tenant = rt.tenant_or_default();

    let result = br
        .end_voice_call(tenant, &args.call_id, 1, &args.reason) // 1 = CALLER_HANGUP
        .context("end_voice_call")?;

    output::success(&format!("Call {} ended", args.call_id));
    let call = result.get("call").cloned().unwrap_or(json!(null));
    let state = call.get("state").and_then(Value::as_i64).unwrap_or(0);
    output::detail("terminal_state", &format!("{state}"));
    Ok(())
}

fn run_watch(args: WatchArgs) -> anyhow::Result<()> {
    let (br, rt) = shared::connect_bridge()?;
    let tenant = rt.tenant_or_default();

    let result = br
        .watch_voice_call_events(tenant, &args.call_id, args.from, args.max_events, 5000)
        .context("watch_voice_call_events")?;

    let events = result.get("events").and_then(Value::as_array);
    let count = result.get("count").and_then(Value::as_i64).unwrap_or(0);
    let terminal = result
        .get("reached_terminal")
        .and_then(Value::as_bool)
        .unwrap_or(false);

    if let Some(events) = events {
        for evt in events {
            let seq = evt.get("sequence").and_then(Value::as_u64).unwrap_or(0);
            let etype = evt
                .get("event_type")
                .and_then(Value::as_str)
                .unwrap_or("?");
            let ts = evt
                .get("timestamp_unix_ms")
                .and_then(Value::as_i64)
                .unwrap_or(0);
            println!("  [{seq:>4}] {etype:<30} ts={ts}");
        }
    }
    output::detail("events", &format!("{count}"));
    output::detail("reached_terminal", &format!("{terminal}"));
    Ok(())
}

fn run_metrics(args: MetricsArgs) -> anyhow::Result<()> {
    let (br, rt) = shared::connect_bridge()?;
    let tenant = rt.tenant_or_default();

    let metrics = json!({
        "rtt_ms": args.rtt_ms,
        "jitter_ms": args.jitter_ms,
        "packet_loss_ratio": args.loss,
        "concealed_samples": 0,
        "audio_level_dbov": -26.0,
    });

    let result = br
        .report_voice_call_metrics(tenant, &args.call_id, &args.participant_id, &metrics)
        .context("report_voice_call_metrics")?;

    output::success("Metrics reported");
    if let Some(adaptation) = result.get("adaptation") {
        let reason = adaptation
            .get("reason")
            .and_then(Value::as_str)
            .unwrap_or("none");
        output::detail("adaptation", reason);
    }
    Ok(())
}
