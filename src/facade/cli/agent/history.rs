// EasyNet CLI — `easynet agent` chat-history surface: sessions list/show
// Split from facade/cli/agent.rs (F-033 / T4.6); bodies are move-only.

use serde_json::Value;


use super::*;

// ── Sessions inspection ────────────────────────────────────────────

pub(super) fn run_sessions(args: ChatHistoryArgs) -> anyhow::Result<()> {
    // Validate the agent exists. Lets us emit "no such agent"
    // rather than "no sessions" for a typo'd name.
    let daemon_client = required_local_daemon_agent_client()?;
    let _row = daemon_agent_row(&daemon_client, &args.name)?;
    match args.action {
        ChatHistoryAction::List(a) => run_sessions_list(&args.name, a),
        ChatHistoryAction::Show(a) => run_sessions_show(&args.name, a),
    }
}

pub(super) fn run_sessions_list(agent: &str, args: ChatHistoryListArgs) -> anyhow::Result<()> {
    use crate::persistence::chat_sessions;
    let sessions = chat_sessions::list_sessions(agent);
    if args.json {
        println!("{}", serde_json::to_string_pretty(&sessions)?);
        return Ok(());
    }
    if sessions.is_empty() {
        println!(
            "No sessions recorded for agent '{agent}'. \
             Run 'easynet agent send {agent} \"...\"' to start one."
        );
        return Ok(());
    }
    let latest = chat_sessions::latest_session(agent).unwrap_or_default();
    println!(
        "{:<38} {:<22} {:>6}  PROMPT",
        "SESSION_ID", "LAST_TURN_AT", "TURNS"
    );
    for s in &sessions {
        let marker = if s.session_id == latest { "*" } else { " " };
        println!(
            "{}{:<37} {:<22} {:>6}  {}",
            marker, s.session_id, s.last_turn_at, s.turn_count, s.prompt_preview,
        );
    }
    println!();
    println!("  '*' marks the most-recent session ('agent send {agent} --follow' resumes it).");
    Ok(())
}

pub(super) fn run_sessions_show(agent: &str, args: ChatHistoryShowArgs) -> anyhow::Result<()> {
    use crate::persistence::chat_sessions;
    let lines = chat_sessions::load_session(agent, &args.session_id)?;
    if args.json {
        for v in &lines {
            println!("{}", serde_json::to_string(v)?);
        }
        return Ok(());
    }
    // Human-readable transcript: meta header, then one block per turn.
    if lines.is_empty() {
        println!("Session '{}' is empty.", args.session_id);
        return Ok(());
    }
    let mut turn_no = 0usize;
    for v in &lines {
        match v.get("type").and_then(Value::as_str) {
            Some("session_meta") => {
                println!(
                    "Session: {}",
                    v.get("session_id").and_then(Value::as_str).unwrap_or("?")
                );
                println!(
                    "  agent:       {}",
                    v.get("agent").and_then(Value::as_str).unwrap_or("?")
                );
                println!(
                    "  started_at:  {}",
                    v.get("timestamp").and_then(Value::as_str).unwrap_or("?")
                );
                println!(
                    "  cwd:         {}",
                    v.get("cwd").and_then(Value::as_str).unwrap_or("?")
                );
                println!(
                    "  cli_version: {}",
                    v.get("cli_version").and_then(Value::as_str).unwrap_or("?")
                );
                println!();
            }
            Some("turn") => {
                turn_no += 1;
                println!(
                    "── Turn {turn_no} ── {}",
                    v.get("timestamp").and_then(Value::as_str).unwrap_or("?")
                );
                if let Some(p) = v.get("prompt").and_then(Value::as_str) {
                    println!("  user:");
                    for line in p.lines() {
                        println!("    {line}");
                    }
                }
                if let Some(r) = v.get("reply").and_then(Value::as_str) {
                    println!("  agent:");
                    for line in r.lines() {
                        println!("    {line}");
                    }
                }
                if let Some(usage) = v.get("usage") {
                    if !usage.is_null() {
                        println!("  usage: {}", usage);
                    }
                }
                println!();
            }
            _ => {
                // Unknown line type — surface verbatim so future
                // log additions don't disappear from `show`.
                println!("(unknown line type) {v}");
            }
        }
    }
    Ok(())
}
