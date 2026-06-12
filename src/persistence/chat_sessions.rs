// EasyNet CLI — Agent session persistence
// ========================================
//
// File: src/persistence/agent_sessions.rs
//
// Per-agent on-disk conversation log. Every `easynet agent send`
// turn (prompt + reply + usage + tool calls) appends one JSONL
// line under:
//
//   ~/.easynet/agents/<agent>/sessions/<session-uuid>.jsonl
//
// Plus a tiny pointer file:
//
//   ~/.easynet/agents/<agent>/sessions/index.json
//
// The pointer carries the latest session_id so `--follow` is an
// O(1) lookup, and a list of every session_id with its last-touched
// timestamp + turn count so `--resume` can render a picker without
// reading every JSONL.
//
// Why per-agent JSONL
// -------------------
// Codex CLI persists each conversation as a single rollout JSONL
// at `~/.codex/sessions/YYYY/MM/DD/rollout-<ISO>-<UUID>.jsonl` —
// session_meta line first, then one response_item per line.
// Claude Code persists per-cwd JSONL at
// `~/.claude/projects/<sanitised-cwd>/<UUID>.jsonl`, where each
// line records a turn / queue-operation / tool-call. EasyNet
// agents are owner-rooted (per RFC-001 §3.2), so we scope by
// agent name rather than by date or cwd. The line shape mixes
// the most useful fields from both:
//
//   {
//     "type": "turn",          // or "session_meta"
//     "timestamp": "...",      // RFC3339, UTC
//     "session_id": "...",
//     "prompt": "...",
//     "reply": "...",
//     "tool_calls": [...],
//     "usage": { input_tokens, output_tokens, model }
//   }
//
// First line of every file is a `session_meta` row capturing
// `cwd`, `agent`, `cli_version` so `agent sessions show` can
// reconstruct provenance even after the agent_type / model in
// agents.json drifts.
//
// Failure semantics
// -----------------
// Persistence MUST NOT fail an in-flight `agent send` — the
// caller wraps every write in a best-effort branch. Disk full,
// permission errors, etc. surface as a stderr warning but the
// chat reply still reaches the user. This matches the Codex /
// Claude rule: the conversation comes first, the log follows.
//
// Author: Silan Hu <silan.hu@u.nus.edu>
// Copyright (c) 2026 EasyNet. All rights reserved.

use std::fs;
use std::io::Write;
use std::path::PathBuf;

use anyhow::Context;
use serde::{Deserialize, Serialize};
use serde_json::Value;

use crate::persistence::config::{agents_root, atomic_write_with_permissions, WritePermissions};

/// Maximum bytes of `prompt` / `reply` we capture for the index
/// preview. The full text always lives in the JSONL; this cap
/// only governs `index.json::sessions[].prompt_preview`.
const PREVIEW_BYTE_CAP: usize = 80;

/// One JSONL line representing a single chat turn. The wire shape
/// is duck-typed by `agent sessions show` and other consumers, so
/// adding fields is a non-breaking change as long as
/// `serde(default)` covers them.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct TurnRecord {
    /// Always `"turn"`. Lets a future writer add other line types
    /// (`tool_call`, `error`, `session_close`) without breaking the
    /// reader.
    #[serde(rename = "type")]
    pub kind: String,
    /// RFC3339, UTC. When the turn completed (server-side reply
    /// returned), not when the prompt was sent.
    pub timestamp: String,
    pub session_id: String,
    pub prompt: String,
    pub reply: String,
    /// Each tool call is whatever JSON the chat handler emitted —
    /// passed through verbatim so we don't have to re-implement
    /// the schema every time a new tool surfaces.
    #[serde(default)]
    pub tool_calls: Vec<Value>,
    /// Free-form usage bag (input_tokens, output_tokens, cache_read,
    /// cache_write, model). The chat handler's own shape; we don't
    /// repackage.
    #[serde(default)]
    pub usage: Value,
}

/// Provenance row written as the first JSONL line of a fresh
/// session. Codex calls this `session_meta`; we keep the same name
/// so log inspectors that already know the Codex shape can read
/// our files too.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SessionMeta {
    #[serde(rename = "type")]
    pub kind: String,
    pub timestamp: String,
    pub session_id: String,
    pub agent: String,
    pub cwd: String,
    pub cli_version: String,
}

/// Pointer file at `<agent>/sessions/index.json`. Read on every
/// `--follow` / `--resume`; rewritten atomically after every turn
/// the daemon successfully logs. Scanning the directory works as a
/// fallback when the index is missing or stale, but the index is
/// the fast path.
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct SessionIndex {
    /// Most recently touched session id. `--follow` resumes this.
    /// Empty on a fresh agent or when every session has been
    /// pruned.
    #[serde(default)]
    pub latest: String,
    /// The agent's lifelong (default) session id. The chat ability
    /// binds it on the first turn sent with the `lifelong` sentinel
    /// and resumes it on every later sentinel turn, so the agent
    /// keeps one continuous default thread across reloads. Empty
    /// until the first lifelong turn (pre-existing index files
    /// deserialize with the field empty).
    #[serde(default)]
    pub lifelong: String,
    /// One entry per session, sorted most-recent-first. The picker
    /// for `--resume` reads this directly.
    #[serde(default)]
    pub sessions: Vec<SessionDescriptor>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SessionDescriptor {
    pub session_id: String,
    /// First time we saw a turn for this session.
    pub started_at: String,
    /// Most recent turn's timestamp. Used to sort the picker.
    pub last_turn_at: String,
    pub turn_count: usize,
    /// First ~80 bytes of the most recent prompt — what `--resume`
    /// renders as the human-readable preview row.
    pub prompt_preview: String,
}

/// `<agents_root>/<agent>/sessions/`.
fn sessions_dir(agent: &str) -> PathBuf {
    agents_root().join(agent).join("sessions")
}

fn session_file(agent: &str, session_id: &str) -> PathBuf {
    sessions_dir(agent).join(format!("{session_id}.jsonl"))
}

fn index_file(agent: &str) -> PathBuf {
    sessions_dir(agent).join("index.json")
}

/// Read the index file. Returns an empty default if missing or
/// corrupt — a corrupt index is recoverable by re-scanning the
/// directory; we don't want a corrupt cache to kill `agent send`.
pub fn load_index(agent: &str) -> SessionIndex {
    let path = index_file(agent);
    match fs::read_to_string(&path) {
        Ok(raw) => serde_json::from_str(&raw).unwrap_or_default(),
        Err(_) => SessionIndex::default(),
    }
}

fn save_index(agent: &str, index: &SessionIndex) -> anyhow::Result<()> {
    let path = index_file(agent);
    if let Some(parent) = path.parent() {
        fs::create_dir_all(parent)
            .with_context(|| format!("create sessions dir {}", parent.display()))?;
    }
    let json = serde_json::to_string_pretty(index)?;
    atomic_write_with_permissions(&path, json.as_bytes(), WritePermissions::Default)
        .with_context(|| format!("write {}", path.display()))?;
    Ok(())
}

/// Latest session id for `agent`, or `None` when nothing has been
/// written yet. Backs `easynet agent send --follow`.
pub fn latest_session(agent: &str) -> Option<String> {
    let idx = load_index(agent);
    if idx.latest.is_empty() {
        None
    } else {
        Some(idx.latest)
    }
}

/// Every session for `agent`, most-recent-first. Reads the index
/// when it exists; falls back to a directory scan when it doesn't.
/// The directory scan is the cold-start path: a daemon that wrote
/// JSONLs but never updated the index (interrupted process, manual
/// file copy) still shows up in `--resume`.
pub fn list_sessions(agent: &str) -> Vec<SessionDescriptor> {
    let idx = load_index(agent);
    if !idx.sessions.is_empty() {
        return idx.sessions;
    }
    rescan_dir(agent).unwrap_or_default()
}

/// Read every JSONL line of one session. Used by
/// `agent sessions show <session_id>`.
pub fn load_session(agent: &str, session_id: &str) -> anyhow::Result<Vec<Value>> {
    let path = session_file(agent, session_id);
    let raw = fs::read_to_string(&path).with_context(|| format!("read {}", path.display()))?;
    let mut out = Vec::new();
    for (lineno, line) in raw.lines().enumerate() {
        if line.trim().is_empty() {
            continue;
        }
        let v: Value = serde_json::from_str(line)
            .with_context(|| format!("parse line {} of {}", lineno + 1, path.display()))?;
        out.push(v);
    }
    Ok(out)
}

/// Append one turn to the agent's session JSONL and refresh the
/// index. This is the ONLY public mutator — both `agent send` and
/// future `--resume` callers go through here so the index stays in
/// sync.
///
/// Best-effort by contract: the caller in `agent.rs::run_send`
/// wraps the call in a warn-on-error block so a write failure
/// never aborts the in-flight chat response.
pub fn write_turn(
    agent: &str,
    session_id: &str,
    prompt: &str,
    reply: &str,
    tool_calls: &[Value],
    usage: &Value,
) -> anyhow::Result<()> {
    let dir = sessions_dir(agent);
    fs::create_dir_all(&dir).with_context(|| format!("create sessions dir {}", dir.display()))?;

    let path = session_file(agent, session_id);
    let now = chrono::Utc::now().to_rfc3339();
    let is_new_file = !path.exists();

    // Append the meta row on first write.
    let mut buf = String::new();
    if is_new_file {
        let meta = SessionMeta {
            kind: "session_meta".to_string(),
            timestamp: now.clone(),
            session_id: session_id.to_string(),
            agent: agent.to_string(),
            cwd: std::env::current_dir()
                .map(|p| p.display().to_string())
                .unwrap_or_default(),
            cli_version: env!("CARGO_PKG_VERSION").to_string(),
        };
        buf.push_str(&serde_json::to_string(&meta)?);
        buf.push('\n');
    }

    let turn = TurnRecord {
        kind: "turn".to_string(),
        timestamp: now.clone(),
        session_id: session_id.to_string(),
        prompt: prompt.to_string(),
        reply: reply.to_string(),
        tool_calls: tool_calls.to_vec(),
        usage: usage.clone(),
    };
    buf.push_str(&serde_json::to_string(&turn)?);
    buf.push('\n');

    // Append (don't atomic-write) so we never lose prior turns to
    // a partial write. JSONL is append-safe by design — a torn
    // last line is detectable by the reader's parse-error branch
    // and dropped without affecting earlier turns.
    let mut f = fs::OpenOptions::new()
        .create(true)
        .append(true)
        .open(&path)
        .with_context(|| format!("open append {}", path.display()))?;
    f.write_all(buf.as_bytes())
        .with_context(|| format!("append {}", path.display()))?;

    // Refresh the index. We update in-place then rewrite atomically.
    let mut idx = load_index(agent);
    idx.latest = session_id.to_string();
    let preview = make_preview(prompt);
    if let Some(existing) = idx.sessions.iter_mut().find(|s| s.session_id == session_id) {
        existing.last_turn_at = now.clone();
        existing.turn_count += 1;
        existing.prompt_preview = preview;
    } else {
        idx.sessions.push(SessionDescriptor {
            session_id: session_id.to_string(),
            started_at: now.clone(),
            last_turn_at: now,
            turn_count: 1,
            prompt_preview: preview,
        });
    }
    // Sort most-recent-first so the picker doesn't have to.
    idx.sessions
        .sort_by(|a, b| b.last_turn_at.cmp(&a.last_turn_at));
    save_index(agent, &idx)?;

    Ok(())
}

/// The agent's lifelong (default) session id, or `None` while no
/// lifelong turn has been recorded yet. The chat ability resolves
/// the `lifelong` sentinel through this before dispatch.
pub fn lifelong_session(agent: &str) -> Option<String> {
    let idx = load_index(agent);
    if idx.lifelong.is_empty() {
        None
    } else {
        Some(idx.lifelong)
    }
}

/// Bind `session_id` as the agent's lifelong session. Called by the
/// chat ability right after the first lifelong-sentinel turn was
/// persisted, so the id is already in the index; like the index
/// refresh in `write_turn` this must not fail the in-flight reply,
/// hence the best-effort wrapper below.
pub fn set_lifelong_session(agent: &str, session_id: &str) -> anyhow::Result<()> {
    let mut idx = load_index(agent);
    if idx.lifelong == session_id {
        return Ok(());
    }
    idx.lifelong = session_id.to_string();
    save_index(agent, &idx)
}

/// Best-effort variant of [`set_lifelong_session`], mirroring
/// `write_turn_best_effort`'s contract.
pub fn set_lifelong_session_best_effort(agent: &str, session_id: &str) {
    if let Err(e) = set_lifelong_session(agent, session_id) {
        eprintln!("[chat] warning: failed to bind lifelong session: {e}");
    }
}

/// Mark `session_id` as the agent's most-recent session without
/// recording a new turn. Used by `easynet agent send <name>
/// --resume` (no prompt): the operator picks a prior session,
/// the picker returns its id, and we bump the index pointer so
/// the next `--follow` lands on the chosen session. The session
/// must already exist in the on-disk log; calling this for an
/// unknown id is treated as an error so `--resume` doesn't
/// accidentally create a placeholder index row.
pub fn set_latest_session(agent: &str, session_id: &str) -> anyhow::Result<()> {
    let mut idx = load_index(agent);
    let known = idx.sessions.iter().any(|s| s.session_id == session_id);
    if !known {
        anyhow::bail!(
            "session {session_id} not in {agent}'s index — \
             nothing to mark as latest"
        );
    }
    idx.latest = session_id.to_string();
    save_index(agent, &idx)?;
    Ok(())
}

/// Best-effort wrapper used by `run_send`. Logs a stderr warning
/// on failure but never propagates the error to the caller, so a
/// disk-full / read-only-mount situation can't break the chat
/// experience.
pub fn write_turn_best_effort(
    agent: &str,
    session_id: &str,
    prompt: &str,
    reply: &str,
    tool_calls: &[Value],
    usage: &Value,
) {
    if let Err(e) = write_turn(agent, session_id, prompt, reply, tool_calls, usage) {
        eprintln!("[agent send] warning: failed to persist session turn: {e}");
    }
}

/// Truncate `s` to the configured byte cap, marking the cut with
/// `…`. Returns the slice unchanged when it already fits. We work
/// in chars (not bytes) so the truncation is unicode-safe even when
/// the prompt contains CJK. CJK is rare in agent prompts but the
/// CLI ships the chat surface to a global audience, so the safe
/// path is cheap insurance.
fn make_preview(s: &str) -> String {
    let trimmed: String = s.chars().take(PREVIEW_BYTE_CAP).collect();
    let trimmed = trimmed.replace('\n', " ");
    if s.chars().count() > PREVIEW_BYTE_CAP {
        format!("{trimmed}…")
    } else {
        trimmed
    }
}

/// Cold-start fallback for `list_sessions` when the index file is
/// missing or empty. Walks `<agent>/sessions/` for `*.jsonl`,
/// reads the meta + last turn of each, and assembles a synthetic
/// index. Slow on directories with thousands of files; fine for
/// the human-scale single-digit / dozens we expect in practice.
fn rescan_dir(agent: &str) -> anyhow::Result<Vec<SessionDescriptor>> {
    let dir = sessions_dir(agent);
    if !dir.exists() {
        return Ok(Vec::new());
    }
    let mut out = Vec::new();
    for entry in
        fs::read_dir(&dir).with_context(|| format!("scan sessions dir {}", dir.display()))?
    {
        let entry = entry?;
        let path = entry.path();
        if path.extension().and_then(|s| s.to_str()) != Some("jsonl") {
            continue;
        }
        let session_id = match path.file_stem().and_then(|s| s.to_str()) {
            Some(s) => s.to_string(),
            None => continue,
        };
        let raw = match fs::read_to_string(&path) {
            Ok(r) => r,
            Err(_) => continue,
        };
        let mut started_at = String::new();
        let mut last_turn_at = String::new();
        let mut turn_count = 0usize;
        let mut last_prompt = String::new();
        for line in raw.lines() {
            if line.trim().is_empty() {
                continue;
            }
            let v: Value = match serde_json::from_str(line) {
                Ok(v) => v,
                Err(_) => continue,
            };
            let ts = v
                .get("timestamp")
                .and_then(Value::as_str)
                .unwrap_or("")
                .to_string();
            if started_at.is_empty() {
                started_at = ts.clone();
            }
            if v.get("type").and_then(Value::as_str) == Some("turn") {
                last_turn_at = ts;
                turn_count += 1;
                last_prompt = v
                    .get("prompt")
                    .and_then(Value::as_str)
                    .unwrap_or("")
                    .to_string();
            }
        }
        if turn_count == 0 {
            continue; // meta-only file, no turns yet
        }
        out.push(SessionDescriptor {
            session_id,
            started_at,
            last_turn_at,
            turn_count,
            prompt_preview: make_preview(&last_prompt),
        });
    }
    out.sort_by(|a, b| b.last_turn_at.cmp(&a.last_turn_at));
    Ok(out)
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn write_then_load_round_trip() {
        let _g = crate::facade::cli::test_support::HomeGuard::new();
        write_turn(
            "demot",
            "sess-1",
            "hi",
            "hello",
            &[],
            &json!({"input_tokens": 10}),
        )
        .expect("write");
        let lines = load_session("demot", "sess-1").expect("load");
        assert_eq!(lines.len(), 2, "session_meta + 1 turn");
        assert_eq!(lines[0]["type"], "session_meta");
        assert_eq!(lines[1]["type"], "turn");
        assert_eq!(lines[1]["prompt"], "hi");
        assert_eq!(lines[1]["reply"], "hello");
        assert_eq!(lines[1]["usage"]["input_tokens"], 10);
    }

    #[test]
    fn second_write_appends_does_not_double_meta() {
        let _g = crate::facade::cli::test_support::HomeGuard::new();
        write_turn("demot", "sess-1", "hi", "hello", &[], &json!({})).unwrap();
        write_turn("demot", "sess-1", "again", "ok", &[], &json!({})).unwrap();
        let lines = load_session("demot", "sess-1").unwrap();
        assert_eq!(lines.len(), 3, "meta + 2 turns");
        assert_eq!(lines[0]["type"], "session_meta");
        assert_eq!(lines[1]["prompt"], "hi");
        assert_eq!(lines[2]["prompt"], "again");
    }

    #[test]
    fn latest_session_tracks_most_recent() {
        let _g = crate::facade::cli::test_support::HomeGuard::new();
        write_turn("demot", "older", "x", "y", &[], &json!({})).unwrap();
        // Sleep is unnecessary in tests since timestamps include
        // sub-second precision via chrono. The second write below
        // has a strictly later RFC3339 stamp.
        write_turn("demot", "newer", "x", "y", &[], &json!({})).unwrap();
        assert_eq!(latest_session("demot"), Some("newer".to_string()));
    }

    #[test]
    fn list_sessions_orders_most_recent_first() {
        let _g = crate::facade::cli::test_support::HomeGuard::new();
        write_turn("demot", "a", "first", "r1", &[], &json!({})).unwrap();
        write_turn("demot", "b", "second", "r2", &[], &json!({})).unwrap();
        let sessions = list_sessions("demot");
        assert_eq!(sessions.len(), 2);
        assert_eq!(sessions[0].session_id, "b");
        assert_eq!(sessions[0].turn_count, 1);
        assert_eq!(sessions[0].prompt_preview, "second");
        assert_eq!(sessions[1].session_id, "a");
    }

    #[test]
    fn list_sessions_falls_back_to_dir_scan_when_index_missing() {
        let _g = crate::facade::cli::test_support::HomeGuard::new();
        write_turn("demot", "scratch", "hi", "ok", &[], &json!({})).unwrap();
        // Simulate a corrupt / missing index.
        let _ = std::fs::remove_file(index_file("demot"));
        let sessions = list_sessions("demot");
        assert_eq!(sessions.len(), 1);
        assert_eq!(sessions[0].session_id, "scratch");
        assert_eq!(sessions[0].turn_count, 1);
    }

    #[test]
    fn preview_truncates_long_prompts() {
        let long = "x".repeat(200);
        let preview = make_preview(&long);
        assert!(preview.ends_with('…'));
        assert_eq!(preview.chars().count(), PREVIEW_BYTE_CAP + 1); // 80 + …
    }

    #[test]
    fn preview_replaces_newlines_with_spaces() {
        let preview = make_preview("line1\nline2\nline3");
        assert_eq!(preview, "line1 line2 line3");
    }

    #[test]
    fn write_turn_best_effort_swallows_errors() {
        // No HomeGuard — but we still expect this not to panic.
        // We can't easily induce a write error without root, so
        // this test just confirms the wrapper exists and returns.
        write_turn_best_effort("nope", "x", "p", "r", &[], &json!({}));
    }

    #[test]
    fn set_latest_session_bumps_pointer_for_known_id() {
        let _g = crate::facade::cli::test_support::HomeGuard::new();
        write_turn("demot", "old", "x", "y", &[], &json!({})).unwrap();
        write_turn("demot", "new", "x", "y", &[], &json!({})).unwrap();
        // After both writes "new" is latest.
        assert_eq!(latest_session("demot"), Some("new".to_string()));
        // Pick the older one — pointer must follow without
        // touching the JSONL.
        set_latest_session("demot", "old").expect("known id");
        assert_eq!(latest_session("demot"), Some("old".to_string()));
        // The two JSONL files still exist with their original
        // turn counts (only the index pointer moved).
        let old_lines = load_session("demot", "old").unwrap();
        assert_eq!(old_lines.len(), 2, "meta + 1 turn");
    }

    #[test]
    fn lifelong_session_round_trip() {
        let _g = crate::facade::cli::test_support::HomeGuard::new();
        assert_eq!(lifelong_session("demot"), None, "fresh agent has none");
        write_turn("demot", "sess-1", "hi", "hello", &[], &json!({})).unwrap();
        assert_eq!(
            lifelong_session("demot"),
            None,
            "an ordinary turn must not bind the lifelong pointer"
        );
        set_lifelong_session("demot", "sess-1").expect("bind");
        assert_eq!(lifelong_session("demot"), Some("sess-1".to_string()));
        // Re-binding the same id is a no-op; a different id moves it.
        set_lifelong_session("demot", "sess-1").expect("idempotent");
        set_lifelong_session("demot", "sess-2").expect("rebind");
        assert_eq!(lifelong_session("demot"), Some("sess-2".to_string()));
    }

    #[test]
    fn index_without_lifelong_field_deserializes() {
        // Pre-lifelong index.json files have no `lifelong` key; they
        // must keep loading (serde default = empty string → None).
        let raw = r#"{"latest": "a", "sessions": []}"#;
        let idx: SessionIndex = serde_json::from_str(raw).expect("back-compat parse");
        assert_eq!(idx.latest, "a");
        assert!(idx.lifelong.is_empty());
    }

    #[test]
    fn set_latest_session_rejects_unknown_id() {
        let _g = crate::facade::cli::test_support::HomeGuard::new();
        write_turn("demot", "real", "x", "y", &[], &json!({})).unwrap();
        let err = set_latest_session("demot", "ghost")
            .expect_err("unknown id must surface a typed error");
        assert!(format!("{err}").contains("not in demot's index"));
        // Pointer must not have moved.
        assert_eq!(latest_session("demot"), Some("real".to_string()));
    }
}
