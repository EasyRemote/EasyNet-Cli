// EasyNet CLI — `easynet mission discuss`
// =========================================
//
// File: src/facade/cli/discuss.rs
// Description: Human-bracketed multi-agent discussion. Per the
//              ability-only ontology this CLI subcommand is a thin
//              shell that drives three abilities in sequence:
//
//                discuss.create        — open a room with the
//                                        chosen participants.
//                discuss.post (human)  — post the human's first
//                                        turn (the topic).
//                mission.discuss_round — let the agents speak among
//                                        themselves until they all
//                                        skip a cycle, or
//                                        max_cycles is reached.
//
// After the sub-turn returns, the CLI reads the room transcript
// via `discuss.subscribe`-style snapshot (we use plain
// `easynet.invoke` against the subscribe ability's snapshot half
// for v1) and prints what was said. The operator can then choose
// to post another turn (`--continue`) or end the discussion.
//
// Why one room, many sub-turns
// ----------------------------
// The room state (transcript + per-(agent) chat session ids in
// the orchestration ability) carries across sub-turns. An agent
// resuming on the next sub-turn sees its prior reasoning in its
// chat driver's transcript — coherent multi-step thinking, not
// independent micro-conversations. This is the load-bearing
// property of `mission.discuss_round`: the human is in control
// of the discussion's tempo, but the agents have continuity.
//
// Author: Silan Hu <silan.hu@u.nus.edu>
// Copyright (c) 2026 EasyNet. All rights reserved.

use anyhow::{bail, Context};
use clap::Args;
use console::style;
use serde_json::{json, Value};

use crate::support::local_invoke::invoke_local_ability;

#[derive(Debug, Args)]
pub struct DiscussArgs {
    /// Comma-separated list of agent names (e.g. "claude,codex").
    /// Required when starting a new discussion (no `--room`).
    /// Ignored when `--room` is set — the room's existing
    /// participants are used.
    #[arg(long)]
    pub agents: Option<String>,

    /// Discussion topic / human turn message. On a new discussion
    /// (no `--room`) this is both the room's topic and the human's
    /// first posted turn. On a continuation (`--room ROOM_ID`)
    /// this is the next human turn.
    #[arg(long)]
    pub topic: String,

    /// Continue an existing room. When omitted, a new room is
    /// created and its id is printed so the operator can resume.
    #[arg(long)]
    pub room: Option<String>,

    /// Hard upper bound on cycles within this sub-turn. Each cycle
    /// queries every agent in parallel; the sub-turn ends when an
    /// entire cycle finishes with all agents skipping, or when
    /// this many cycles have run. Default 10.
    #[arg(long, default_value_t = 10)]
    pub max_cycles: u32,

    /// Optional per-agent role assignment, repeatable. Form:
    /// `--role <agent>=<role description>`. Agents listed here
    /// skip the cycle-1 self-nomination prompt and stay in the
    /// assigned role for the duration of the sub-turn. Useful
    /// when the operator wants explicit dramatis personae
    /// (e.g. `--role claude=skeptic --role codex=builder`).
    #[arg(long = "role", value_name = "AGENT=ROLE")]
    pub roles: Vec<String>,

    /// Write the room transcript to a markdown file on completion.
    #[arg(long)]
    pub output: Option<String>,
}

pub fn run(args: DiscussArgs) -> anyhow::Result<()> {
    // Resolve participants + room id. New discussion: caller
    // provides --agents, we mint a room. Continuation: caller
    // provides --room, we read the room's existing participants.
    let (room_id, participants) = match (&args.room, &args.agents) {
        (Some(rid), _) => {
            // Continuation. Pull participants from the room snapshot.
            let participants = read_room_participants(rid)?;
            (rid.clone(), participants)
        }
        (None, Some(agents)) => {
            let participants: Vec<String> = agents
                .split(',')
                .map(|s| s.trim().to_string())
                .filter(|s| !s.is_empty())
                .collect();
            if participants.is_empty() {
                bail!("--agents was provided but parsed empty; pass `claude,codex,...`");
            }
            let create_resp = invoke_local_ability(
                "device.discuss.create",
                json!({
                    "participants": participants,
                    "topic":        args.topic,
                }),
            )
            .context("invoke discuss.create")?;
            let rid = create_resp
                .get("room_id")
                .and_then(Value::as_str)
                .ok_or_else(|| {
                    anyhow::anyhow!("discuss.create returned no room_id: {create_resp}")
                })?
                .to_string();
            (rid, participants)
        }
        (None, None) => bail!(
            "either --agents (to start a new discussion) or --room <id> (to continue an \
             existing one) is required"
        ),
    };

    eprintln!();
    eprintln!("{}", style("EasyNet Multi-Agent Discussion").cyan().bold());
    eprintln!("{}", style("═".repeat(40)).dim());
    eprintln!("  Room:    {}", style(&room_id).yellow());
    eprintln!("  Agents:  {}", participants.join(", "));
    eprintln!("  Cycles:  up to {} (per sub-turn)", args.max_cycles);
    eprintln!();

    // Post the human turn. On a fresh room this is the first
    // utterance the agents see; on a continuation it's the next
    // human turn after the prior sub-turn ended.
    let _ = invoke_local_ability(
        "device.discuss.post",
        json!({
            "room_id": room_id,
            "speaker": "human",
            "message": args.topic,
        }),
    )
    .context("invoke discuss.post (human turn)")?;

    eprintln!("{}  {}", style("[human]").bold(), &args.topic);
    eprintln!();
    eprintln!(
        "  {} agents are deliberating ({}, max {} cycles)...",
        style("·").dim(),
        participants.join(", "),
        args.max_cycles
    );
    eprintln!();

    // Parse `--role <agent>=<desc>` repeats. Each well-formed
    // entry contributes to the `roles` map passed into the
    // ability; malformed (missing `=`) entries surface a precise
    // CLI error rather than being silently dropped.
    let mut role_map = serde_json::Map::new();
    for entry in &args.roles {
        match entry.split_once('=') {
            Some((agent, role)) => {
                let agent = agent.trim();
                let role = role.trim();
                if agent.is_empty() || role.is_empty() {
                    bail!(
                        "--role {entry:?} parsed empty agent or role; \
                         expected `<agent>=<role description>`"
                    );
                }
                role_map.insert(agent.to_string(), Value::String(role.to_string()));
            }
            None => bail!(
                "--role {entry:?} is missing `=`; expected `<agent>=<role description>` \
                 (e.g. --role claude=skeptic)"
            ),
        }
    }

    // Run one sub-turn. mission.discuss_round handles the
    // parallel cycle loop. The handler embeds the full room
    // transcript in its response (`turns`) so we don't have to
    // make a second IPC call — that would race against any
    // `easynet mcp serve` subprocess the chat dispatch may have
    // started, since multiple listeners on the same control.sock
    // load-balance connections.
    let mut round_args = json!({
        "room_id":    room_id,
        "agents":     participants,
        "max_cycles": args.max_cycles,
        "topic":      args.topic,
    });
    if !role_map.is_empty() {
        round_args["roles"] = Value::Object(role_map);
    }
    let result = invoke_local_ability("device.mission.discuss_round", round_args)
        .context("invoke mission.discuss_round")?;

    // Print every agent turn from the embedded snapshot. We skip
    // the human turn (already echoed above) to avoid printing the
    // operator's input back to them.
    let turns = result
        .get("turns")
        .and_then(Value::as_array)
        .cloned()
        .unwrap_or_default();
    for turn in &turns {
        let speaker = turn.get("speaker").and_then(Value::as_str).unwrap_or("?");
        if speaker == "human" {
            continue;
        }
        let message = turn.get("message").and_then(Value::as_str).unwrap_or("");
        eprintln!(
            "{}  {}",
            style(format!("[{speaker}]")).cyan().bold(),
            message
        );
        eprintln!();
    }

    // Surface termination + error envelope.
    let cycles_used = result
        .get("cycles_used")
        .and_then(Value::as_u64)
        .unwrap_or(0);
    let terminated_reason = result
        .get("terminated_reason")
        .and_then(Value::as_str)
        .unwrap_or("?");
    eprintln!(
        "  {} sub-turn ended after {cycles_used} cycle(s) — {terminated_reason}",
        style("·").dim()
    );
    if let Some(errors) = result.get("errors").and_then(Value::as_array) {
        for err in errors {
            let agent = err.get("agent").and_then(Value::as_str).unwrap_or("?");
            let message = err.get("error").and_then(Value::as_str).unwrap_or("");
            let cycle = err.get("cycle").and_then(Value::as_u64).unwrap_or(0);
            eprintln!(
                "  {} {agent} failed in cycle {cycle}: {message}",
                style("⚠").yellow()
            );
        }
    }
    eprintln!();
    eprintln!(
        "  Continue with: {} {}",
        style("easynet mission discuss").dim(),
        style(format!("--room {room_id} --topic \"...\"")).dim()
    );
    eprintln!();

    // Optionally write the full transcript to markdown.
    if let Some(path) = &args.output {
        let transcript = read_turns_from(&room_id, 0)?;
        let markdown = render_markdown(&room_id, &participants, &transcript);
        crate::persistence::config::atomic_write(std::path::Path::new(path), markdown.as_bytes())
            .with_context(|| format!("write {path}"))?;
        eprintln!(
            "{} Transcript written to {}",
            style("✓").green(),
            style(path).cyan()
        );
    }
    Ok(())
}

/// Read the room's participant list from `discuss.list`-style
/// metadata. v1: we don't have a dedicated `discuss.show`
/// ability, so we synthesise the participant list from the
/// turn log's distinct non-`human` speakers. Good enough for
/// the continuation path; a future `discuss.show` ability
/// would replace this.
fn read_room_participants(room_id: &str) -> anyhow::Result<Vec<String>> {
    let turns = read_turns_from(room_id, 0)?;
    let mut set: std::collections::BTreeSet<String> = std::collections::BTreeSet::new();
    for t in &turns {
        if let Some(s) = t.get("speaker").and_then(Value::as_str) {
            if s != "human" {
                set.insert(s.to_string());
            }
        }
    }
    if set.is_empty() {
        anyhow::bail!(
            "room {room_id:?} has no recorded agent turns yet — cannot recover the \
             participant list. Pass --agents on the first sub-turn explicitly."
        );
    }
    Ok(set.into_iter().collect())
}

/// Read the raw turn array for the room from `since_seq`. Routes
/// through the canonical `discuss.list_turns` ability so the CLI
/// rendering shares the same surface every other invocation does
/// — no direct DiscussService access from CLI code.
fn read_turns_from(room_id: &str, since_seq: i64) -> anyhow::Result<Vec<Value>> {
    let resp = invoke_local_ability(
        "device.discuss.list_turns",
        json!({
            "room_id":   room_id,
            "since_seq": since_seq,
        }),
    )
    .with_context(|| format!("invoke discuss.list_turns for room {room_id}"))?;
    Ok(resp
        .get("turns")
        .and_then(Value::as_array)
        .cloned()
        .unwrap_or_default())
}

// Pre-rewrite the CLI took a snapshot of the room's max sequence
// before invoking mission.discuss_round, then read turns >= that
// sequence after. That works only when both calls hit the same
// daemon process — which isn't guaranteed when an `easynet mcp
// serve` subprocess is also listening on control.sock (multiple
// listeners load-balance connections). The rewrite embeds the
// full transcript in mission.discuss_round's own response, so the
// pre-turn-sequence helper is no longer needed.

/// Render the discussion as a markdown article. Falls back to a
/// "no transcript captured" line when the room snapshot is
/// empty; this happens whenever `read_turns_from` returns empty
/// (the v1 stub does so until `discuss.list_turns` ships).
fn render_markdown(room_id: &str, participants: &[String], turns: &[Value]) -> String {
    let mut md = String::new();
    md.push_str(&format!("# Discussion {room_id}\n\n"));
    md.push_str(&format!("Participants: {}\n\n", participants.join(", ")));
    md.push_str("---\n\n");
    if turns.is_empty() {
        md.push_str("_(transcript snapshot not captured by CLI in v1)_\n");
        return md;
    }
    for turn in turns {
        let speaker = turn.get("speaker").and_then(Value::as_str).unwrap_or("?");
        let message = turn.get("message").and_then(Value::as_str).unwrap_or("");
        md.push_str(&format!("**{speaker}**\n\n{message}\n\n"));
    }
    md
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn render_markdown_includes_participants_and_turns() {
        let turns = vec![
            json!({"speaker": "human", "message": "what should we build?"}),
            json!({"speaker": "claude", "message": "an ability dispatcher"}),
        ];
        let md = render_markdown(
            "room-x",
            &["claude".to_string(), "codex".to_string()],
            &turns,
        );
        assert!(md.contains("# Discussion room-x"));
        assert!(md.contains("claude, codex"));
        assert!(md.contains("**human**"));
        assert!(md.contains("an ability dispatcher"));
    }

    #[test]
    fn render_markdown_falls_back_when_transcript_empty() {
        let md = render_markdown("room-x", &["claude".to_string()], &[]);
        assert!(md.contains("not captured"));
    }
}
