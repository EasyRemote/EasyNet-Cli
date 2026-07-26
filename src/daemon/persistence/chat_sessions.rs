// EasyNet CLI — Agent session persistence
// ========================================
//
// File: src/daemon/persistence/agent_sessions.rs
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

use crate::daemon::persistence::config::{
    agents_root, atomic_write_with_permissions, WritePermissions,
};

/// Maximum bytes of `prompt` / `reply` we capture for the index
/// preview. The full text always lives in the JSONL; this cap
/// only governs `index.json::sessions[].prompt_preview`.
const PREVIEW_BYTE_CAP: usize = 80;

/// One JSONL line representing a single chat turn. The wire shape
/// is duck-typed by `agent sessions show` and other consumers, so
/// adding fields is a non-breaking change only when the line reader
/// owns an explicit projection for the new state.
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
    /// End-to-end model turn duration captured by the chat ability.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub elapsed_ms: Option<u64>,
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
/// the daemon successfully logs. The index is the canonical session
/// inventory. JSONL files hold transcript content, not discovery
/// authority.
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct SessionIndex {
    /// Most recently touched session id. `--follow` resumes this.
    /// Empty on a fresh agent or when every session has been
    /// pruned.
    pub latest: String,
    /// The agent's lifelong (default) session id. The chat ability
    /// binds it on the first turn sent with the `lifelong` sentinel
    /// and resumes it on every later sentinel turn, so the agent
    /// keeps one continuous default thread across reloads. Empty until
    /// the first lifelong turn. Existing index files must carry this
    /// field explicitly; missing-file handling is the only place that
    /// constructs an empty index.
    pub lifelong: String,
    /// One entry per session, sorted most-recent-first. The picker
    /// for `--resume` reads this directly.
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

/// Validated read projection over [`SessionIndex`].
///
/// `SessionIndex` is the on-disk serde shape and may represent invalid pointer
/// state if the file was edited or partially migrated. `SessionInventory` is
/// the product read model: a non-empty inventory must prove exactly which
/// session is latest before callers render markers or offer follow semantics.
#[derive(Debug, Clone)]
pub struct SessionInventory {
    latest: Option<String>,
    sessions: Vec<SessionDescriptor>,
}

impl SessionInventory {
    fn from_index(agent: &str, index: SessionIndex) -> anyhow::Result<Self> {
        let latest = index.latest.trim();
        if index.sessions.is_empty() {
            if !latest.is_empty() {
                anyhow::bail!(
                    "session index for agent {agent:?} has latest session {latest:?} but no sessions"
                );
            }
            return Ok(Self {
                latest: None,
                sessions: index.sessions,
            });
        }
        if latest.is_empty() {
            anyhow::bail!(
                "session index for agent {agent:?} has sessions but no latest session pointer"
            );
        }
        if !index
            .sessions
            .iter()
            .any(|session| session.session_id == latest)
        {
            anyhow::bail!(
                "session index for agent {agent:?} latest session {latest:?} is not listed"
            );
        }
        Ok(Self {
            latest: Some(latest.to_string()),
            sessions: index.sessions,
        })
    }

    pub fn sessions(&self) -> &[SessionDescriptor] {
        &self.sessions
    }

    pub fn latest_session(&self) -> Option<&str> {
        self.latest.as_deref()
    }
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

/// Read the canonical session index. Missing index means the agent
/// has no recorded sessions yet; malformed or unreadable index state
/// is rejected instead of reconstructed from transcript files.
pub fn load_index(agent: &str) -> anyhow::Result<SessionIndex> {
    let path = index_file(agent);
    match fs::read_to_string(&path) {
        Ok(raw) => serde_json::from_str(&raw)
            .with_context(|| format!("parse session index {}", path.display())),
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => Ok(SessionIndex::default()),
        Err(error) => Err(error).with_context(|| format!("read session index {}", path.display())),
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
pub fn latest_session(agent: &str) -> anyhow::Result<Option<String>> {
    let idx = load_index(agent)?;
    if idx.latest.is_empty() {
        Ok(None)
    } else {
        Ok(Some(idx.latest))
    }
}

/// Every session for `agent`, most-recent-first. Reads the index
/// inventory only. Transcript JSONL files are loaded only after the
/// caller names a session id from this index.
pub fn list_sessions(agent: &str) -> anyhow::Result<Vec<SessionDescriptor>> {
    Ok(load_index(agent)?.sessions)
}

/// Validated session inventory for product read views.
///
/// Unlike [`list_sessions`] + [`latest_session`], this reads the index once and
/// validates that marker state and rendered rows belong to the same snapshot.
pub fn load_session_inventory(agent: &str) -> anyhow::Result<SessionInventory> {
    SessionInventory::from_index(agent, load_index(agent)?)
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
    write_turn_inner(agent, session_id, prompt, reply, tool_calls, usage, None)
}

pub fn write_turn_with_elapsed(
    agent: &str,
    session_id: &str,
    prompt: &str,
    reply: &str,
    tool_calls: &[Value],
    usage: &Value,
    elapsed_ms: u64,
) -> anyhow::Result<()> {
    write_turn_inner(
        agent,
        session_id,
        prompt,
        reply,
        tool_calls,
        usage,
        Some(elapsed_ms),
    )
}

fn write_turn_inner(
    agent: &str,
    session_id: &str,
    prompt: &str,
    reply: &str,
    tool_calls: &[Value],
    usage: &Value,
    elapsed_ms: Option<u64>,
) -> anyhow::Result<()> {
    let dir = sessions_dir(agent);
    fs::create_dir_all(&dir).with_context(|| format!("create sessions dir {}", dir.display()))?;

    let path = session_file(agent, session_id);
    let now = chrono::Utc::now().to_rfc3339();
    let is_new_file = !path.exists();
    let mut idx = load_index(agent)?;

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
        elapsed_ms,
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
pub fn lifelong_session(agent: &str) -> anyhow::Result<Option<String>> {
    let idx = load_index(agent)?;
    if idx.lifelong.is_empty() {
        Ok(None)
    } else {
        Ok(Some(idx.lifelong))
    }
}

/// Bind `session_id` as the agent's lifelong session. Called by the
/// chat ability right after the first lifelong-sentinel turn was
/// persisted, so the id is already in the index; like the index
/// refresh in `write_turn` this must not fail the in-flight reply,
/// hence the best-effort wrapper below.
pub fn set_lifelong_session(agent: &str, session_id: &str) -> anyhow::Result<()> {
    let mut idx = load_index(agent)?;
    let known = idx.sessions.iter().any(|s| s.session_id == session_id);
    if !known {
        anyhow::bail!(
            "session {session_id} not in {agent}'s index — \
             nothing to bind as lifelong"
        );
    }
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
    let mut idx = load_index(agent)?;
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

pub fn write_turn_best_effort_with_elapsed(
    agent: &str,
    session_id: &str,
    prompt: &str,
    reply: &str,
    tool_calls: &[Value],
    usage: &Value,
    elapsed_ms: u64,
) {
    if let Err(e) = write_turn_with_elapsed(
        agent, session_id, prompt, reply, tool_calls, usage, elapsed_ms,
    ) {
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

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn write_then_load_round_trip() {
        let _g = crate::cli::commands::test_support::HomeGuard::new();
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
    fn write_turn_with_elapsed_persists_duration() {
        let _g = crate::cli::commands::test_support::HomeGuard::new();
        write_turn_with_elapsed(
            "demot",
            "timed",
            "hi",
            "hello",
            &[json!({"ability": "clock.now"})],
            &json!({"input_tokens": 1}),
            42,
        )
        .expect("write");
        let lines = load_session("demot", "timed").expect("load");
        assert_eq!(lines.len(), 2, "session_meta + 1 turn");
        assert_eq!(lines[1]["elapsed_ms"], 42);
        assert_eq!(lines[1]["tool_calls"][0]["ability"], "clock.now");
    }

    #[test]
    fn second_write_appends_does_not_double_meta() {
        let _g = crate::cli::commands::test_support::HomeGuard::new();
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
        let _g = crate::cli::commands::test_support::HomeGuard::new();
        write_turn("demot", "older", "x", "y", &[], &json!({})).unwrap();
        // Sleep is unnecessary in tests since timestamps include
        // sub-second precision via chrono. The second write below
        // has a strictly later RFC3339 stamp.
        write_turn("demot", "newer", "x", "y", &[], &json!({})).unwrap();
        assert_eq!(latest_session("demot").unwrap(), Some("newer".to_string()));
    }

    #[test]
    fn list_sessions_orders_most_recent_first() {
        let _g = crate::cli::commands::test_support::HomeGuard::new();
        write_turn("demot", "a", "first", "r1", &[], &json!({})).unwrap();
        write_turn("demot", "b", "second", "r2", &[], &json!({})).unwrap();
        let sessions = list_sessions("demot").unwrap();
        assert_eq!(sessions.len(), 2);
        assert_eq!(sessions[0].session_id, "b");
        assert_eq!(sessions[0].turn_count, 1);
        assert_eq!(sessions[0].prompt_preview, "second");
        assert_eq!(sessions[1].session_id, "a");
    }

    #[test]
    fn list_sessions_uses_index_only_when_index_missing() {
        let _g = crate::cli::commands::test_support::HomeGuard::new();
        write_turn("demot", "scratch", "hi", "ok", &[], &json!({})).unwrap();
        // Simulate retired local state: transcript JSONL remains but
        // the canonical inventory is absent.
        let _ = std::fs::remove_file(index_file("demot"));
        let sessions = list_sessions("demot").unwrap();
        assert!(
            sessions.is_empty(),
            "JSONL transcript files must not reconstruct session inventory"
        );
    }

    #[test]
    fn session_inventory_missing_index_is_empty() {
        let _g = crate::cli::commands::test_support::HomeGuard::new();
        let inventory = load_session_inventory("demot").expect("missing index is empty inventory");
        assert!(inventory.sessions().is_empty());
        assert_eq!(inventory.latest_session(), None);
    }

    #[test]
    fn session_inventory_rejects_sessions_without_latest_pointer() {
        let _g = crate::cli::commands::test_support::HomeGuard::new();
        write_turn("demot", "scratch", "hi", "ok", &[], &json!({})).unwrap();
        let mut index = load_index("demot").unwrap();
        index.latest.clear();
        save_index("demot", &index).unwrap();

        let error = load_session_inventory("demot")
            .expect_err("non-empty inventory without latest pointer must fail closed");
        assert!(
            error
                .to_string()
                .contains("sessions but no latest session pointer"),
            "unexpected error: {error}"
        );
    }

    #[test]
    fn session_inventory_rejects_unknown_latest_pointer() {
        let _g = crate::cli::commands::test_support::HomeGuard::new();
        write_turn("demot", "scratch", "hi", "ok", &[], &json!({})).unwrap();
        let mut index = load_index("demot").unwrap();
        index.latest = "ghost".to_string();
        save_index("demot", &index).unwrap();

        let error = load_session_inventory("demot")
            .expect_err("latest pointer must reference one listed session");
        assert!(
            error
                .to_string()
                .contains("latest session \"ghost\" is not listed"),
            "unexpected error: {error}"
        );
    }

    #[test]
    fn list_sessions_rejects_corrupt_index() {
        let _g = crate::cli::commands::test_support::HomeGuard::new();
        write_turn("demot", "scratch", "hi", "ok", &[], &json!({})).unwrap();
        std::fs::write(index_file("demot"), b"{not-json").unwrap();
        let err = list_sessions("demot").expect_err("corrupt index must fail closed");
        assert!(format!("{err:#}").contains("parse session index"));
    }

    #[test]
    fn write_turn_rejects_corrupt_index_before_appending() {
        let _g = crate::cli::commands::test_support::HomeGuard::new();
        write_turn("demot", "scratch", "hi", "ok", &[], &json!({})).unwrap();
        let path = session_file("demot", "scratch");
        let before = std::fs::metadata(&path).unwrap().len();
        std::fs::write(index_file("demot"), b"{not-json").unwrap();
        let err = write_turn("demot", "scratch", "again", "nope", &[], &json!({}))
            .expect_err("corrupt index must fail before transcript append");
        assert!(format!("{err:#}").contains("parse session index"));
        let after = std::fs::metadata(&path).unwrap().len();
        assert_eq!(after, before, "failed index refresh must not append JSONL");
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
        let _g = crate::cli::commands::test_support::HomeGuard::new();
        write_turn("demot", "old", "x", "y", &[], &json!({})).unwrap();
        write_turn("demot", "new", "x", "y", &[], &json!({})).unwrap();
        // After both writes "new" is latest.
        assert_eq!(latest_session("demot").unwrap(), Some("new".to_string()));
        // Pick the older one — pointer must follow without
        // touching the JSONL.
        set_latest_session("demot", "old").expect("known id");
        assert_eq!(latest_session("demot").unwrap(), Some("old".to_string()));
        // The two JSONL files still exist with their original
        // turn counts (only the index pointer moved).
        let old_lines = load_session("demot", "old").unwrap();
        assert_eq!(old_lines.len(), 2, "meta + 1 turn");
    }

    #[test]
    fn lifelong_session_round_trip() {
        let _g = crate::cli::commands::test_support::HomeGuard::new();
        assert_eq!(
            lifelong_session("demot").unwrap(),
            None,
            "fresh agent has none"
        );
        write_turn("demot", "sess-1", "hi", "hello", &[], &json!({})).unwrap();
        assert_eq!(
            lifelong_session("demot").unwrap(),
            None,
            "an ordinary turn must not bind the lifelong pointer"
        );
        set_lifelong_session("demot", "sess-1").expect("bind");
        assert_eq!(
            lifelong_session("demot").unwrap(),
            Some("sess-1".to_string())
        );
        // Re-binding the same id is a no-op; a different indexed id moves it.
        set_lifelong_session("demot", "sess-1").expect("idempotent");
        write_turn(
            "demot",
            "sess-2",
            "hi again",
            "hello again",
            &[],
            &json!({}),
        )
        .unwrap();
        set_lifelong_session("demot", "sess-2").expect("rebind");
        assert_eq!(
            lifelong_session("demot").unwrap(),
            Some("sess-2".to_string())
        );
    }

    #[test]
    fn set_lifelong_session_rejects_unknown_id() {
        let _g = crate::cli::commands::test_support::HomeGuard::new();
        write_turn("demot", "real", "x", "y", &[], &json!({})).unwrap();
        let err = set_lifelong_session("demot", "ghost")
            .expect_err("unknown id must not become pointer state");
        assert!(format!("{err}").contains("not in demot's index"));
        assert_eq!(lifelong_session("demot").unwrap(), None);
    }

    #[test]
    fn existing_index_without_lifelong_field_fails_closed() {
        let raw = r#"{"latest": "a", "sessions": []}"#;
        let error =
            serde_json::from_str::<SessionIndex>(raw).expect_err("missing lifelong must fail");
        assert!(
            error.to_string().contains("missing field `lifelong`"),
            "unexpected error: {error}"
        );
    }

    #[test]
    fn existing_index_without_latest_field_fails_closed() {
        let raw = r#"{"lifelong": "", "sessions": []}"#;
        let error =
            serde_json::from_str::<SessionIndex>(raw).expect_err("missing latest must fail");
        assert!(
            error.to_string().contains("missing field `latest`"),
            "unexpected error: {error}"
        );
    }

    #[test]
    fn existing_index_without_sessions_field_fails_closed() {
        let raw = r#"{"latest": "a", "lifelong": ""}"#;
        let error =
            serde_json::from_str::<SessionIndex>(raw).expect_err("missing sessions must fail");
        assert!(
            error.to_string().contains("missing field `sessions`"),
            "unexpected error: {error}"
        );
    }

    #[test]
    fn set_latest_session_rejects_unknown_id() {
        let _g = crate::cli::commands::test_support::HomeGuard::new();
        write_turn("demot", "real", "x", "y", &[], &json!({})).unwrap();
        let err = set_latest_session("demot", "ghost")
            .expect_err("unknown id must surface a typed error");
        assert!(format!("{err}").contains("not in demot's index"));
        // Pointer must not have moved.
        assert_eq!(latest_session("demot").unwrap(), Some("real".to_string()));
    }
}
