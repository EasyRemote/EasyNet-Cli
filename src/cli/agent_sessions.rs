// EasyNet CLI — Agent Session Store
// =================================
//
// File: src/cli/agent_sessions.rs
// Description: Tiny on-disk store for multi-turn agent sessions.
//
// Layout:
//   ~/.easynet/agent_sessions/<session-id>.json
//
// Each file is a single JSON object capturing the agent name, created/updated
// timestamps, and the rolling list of turns. Turns are pasted into the agent
// prompt as plain text — we deliberately do *not* try to model the agent's
// native conversation protocol (Claude's `--continue` mode, Codex's
// app-server thread ids), because the goal here is a portable shim that
// works against every backend the dispatcher already supports.
//
// Author: Silan Hu <silan.hu@u.nus.edu>
// Copyright (c) 2026 EasyNet. All rights reserved.

use std::fs;
use std::path::PathBuf;

use chrono::Local;
use serde::{Deserialize, Serialize};

use crate::shared::config;

pub fn root_dir() -> PathBuf {
    config::state_dir().join("agent_sessions")
}

/// Reject session ids that would escape the sessions directory or would
/// otherwise be unsafe as a filename. We're intentionally strict: a session
/// id is a short user-chosen label, not arbitrary user input from the
/// network, so the legitimate set is small.
pub fn validate_id(id: &str) -> anyhow::Result<()> {
    let trimmed = id.trim();
    if trimmed.is_empty() {
        anyhow::bail!("session id is empty");
    }
    if trimmed != id {
        anyhow::bail!("session id must not have leading/trailing whitespace");
    }
    if id.contains('/')
        || id.contains('\\')
        || id.contains('\0')
        || id.contains("..")
    {
        anyhow::bail!(
            "session id '{id}' contains illegal characters (/, \\, \\0, or '..')"
        );
    }
    Ok(())
}

pub fn session_path(id: &str) -> anyhow::Result<PathBuf> {
    validate_id(id)?;
    Ok(root_dir().join(format!("{id}.json")))
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Session {
    pub id: String,
    pub agent: String,
    pub created_at: String,
    pub updated_at: String,
    #[serde(default)]
    pub turns: Vec<Turn>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Turn {
    pub role: String, // "user" | "assistant"
    pub content: String,
    pub at: String,
}

impl Session {
    /// Create an in-memory session. Validates the id immediately so callers
    /// fail at construction time, not at save time.
    pub fn new(id: String, agent: String) -> anyhow::Result<Self> {
        validate_id(&id)?;
        if agent.trim().is_empty() {
            anyhow::bail!("agent name is empty");
        }
        let now = Local::now().to_rfc3339();
        Ok(Self {
            id,
            agent,
            created_at: now.clone(),
            updated_at: now,
            turns: Vec::new(),
        })
    }

    pub fn save(&self) -> anyhow::Result<()> {
        let path = session_path(&self.id)?;
        fs::create_dir_all(root_dir())?;
        let json = serde_json::to_string_pretty(self)?;
        fs::write(path, json)?;
        Ok(())
    }

    pub fn load(id: &str) -> anyhow::Result<Self> {
        let path = session_path(id)?;
        let raw = fs::read_to_string(&path)
            .map_err(|_| anyhow::anyhow!("session '{id}' not found at {}", path.display()))?;
        let s: Self = serde_json::from_str(&raw)?;
        Ok(s)
    }

    pub fn append(&mut self, role: &str, content: &str) {
        self.turns.push(Turn {
            role: role.to_string(),
            content: content.to_string(),
            at: Local::now().to_rfc3339(),
        });
        self.updated_at = Local::now().to_rfc3339();
    }

    /// Render the conversation as a plain-text transcript suitable for
    /// pasting into the next prompt's context.
    pub fn transcript(&self) -> String {
        let mut buf = String::new();
        for t in &self.turns {
            buf.push_str(&format!("[{}] {}\n", t.role, t.content));
            buf.push('\n');
        }
        buf
    }
}

pub fn list_sessions() -> anyhow::Result<Vec<Session>> {
    let dir = root_dir();
    if !dir.exists() {
        return Ok(Vec::new());
    }
    let mut out = Vec::new();
    for entry in fs::read_dir(&dir)? {
        let entry = entry?;
        let path = entry.path();
        if path.extension().and_then(|s| s.to_str()) != Some("json") {
            continue;
        }
        if let Ok(raw) = fs::read_to_string(&path) {
            if let Ok(s) = serde_json::from_str::<Session>(&raw) {
                out.push(s);
            }
        }
    }
    out.sort_by(|a, b| b.updated_at.cmp(&a.updated_at));
    Ok(out)
}

pub fn delete_session(id: &str) -> anyhow::Result<()> {
    let path = session_path(id)?;
    if !path.exists() {
        anyhow::bail!("session '{id}' not found");
    }
    fs::remove_file(path)?;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::cli::test_support::HomeGuard;

    // ── id validation: pure ───────────────────────────────────────────────

    #[test]
    fn validate_id_accepts_normal_labels() {
        for ok in ["chat", "session-1", "abc_123", "x.y", "热聊"] {
            assert!(validate_id(ok).is_ok(), "should accept '{ok}'");
        }
    }

    #[test]
    fn validate_id_rejects_empty_and_whitespace() {
        assert!(validate_id("").is_err());
        assert!(validate_id("   ").is_err());
        assert!(validate_id(" leading").is_err());
        assert!(validate_id("trailing ").is_err());
    }

    #[test]
    fn validate_id_rejects_path_traversal() {
        for bad in [
            "../etc/passwd",
            "..",
            "a/b",
            "a\\b",
            "a..b",
            "with\0null",
        ] {
            assert!(validate_id(bad).is_err(), "should reject '{bad}'");
        }
    }

    #[test]
    fn new_rejects_empty_id_and_empty_agent() {
        let _g = HomeGuard::new();
        assert!(Session::new(String::new(), "claude".into()).is_err());
        assert!(Session::new("ok".into(), String::new()).is_err());
        assert!(Session::new("ok".into(), "   ".into()).is_err());
    }

    #[test]
    fn save_and_load_round_trip() {
        let _g = HomeGuard::new();
        let mut s = Session::new("smoke".into(), "claude".into()).expect("new");
        s.append("user", "hello");
        s.append("assistant", "hi there");
        s.save().expect("save");

        let loaded = Session::load("smoke").expect("load");
        assert_eq!(loaded.id, "smoke");
        assert_eq!(loaded.agent, "claude");
        assert_eq!(loaded.turns.len(), 2);
        assert_eq!(loaded.turns[0].role, "user");
        assert_eq!(loaded.turns[0].content, "hello");
        assert_eq!(loaded.turns[1].content, "hi there");
    }

    #[test]
    fn load_missing_session_errors() {
        let _g = HomeGuard::new();
        assert!(Session::load("nope").is_err());
    }

    #[test]
    fn load_rejects_path_traversal_id() {
        let _g = HomeGuard::new();
        // Even if the file existed (which it doesn't), the validation must
        // refuse to construct the path at all.
        let err = Session::load("../escape").unwrap_err();
        assert!(err.to_string().contains("illegal characters"));
    }

    #[test]
    fn save_writes_into_root_only() {
        let _g = HomeGuard::new();
        let s = Session::new("contained".into(), "claude".into()).expect("new");
        s.save().expect("save");
        let path = session_path("contained").expect("path");
        assert!(path.starts_with(root_dir()));
        assert!(path.exists());
    }

    #[test]
    fn list_sessions_orders_by_updated_desc() {
        let _g = HomeGuard::new();
        // Three sessions, each with a deliberate updated_at delta.
        let mut a = Session::new("a".into(), "claude".into()).expect("a");
        a.updated_at = "2026-01-01T00:00:00+00:00".into();
        a.save().expect("save a");

        let mut b = Session::new("b".into(), "claude".into()).expect("b");
        b.updated_at = "2026-03-01T00:00:00+00:00".into();
        b.save().expect("save b");

        let mut c = Session::new("c".into(), "claude".into()).expect("c");
        c.updated_at = "2026-02-01T00:00:00+00:00".into();
        c.save().expect("save c");

        let listed = list_sessions().expect("list");
        let ids: Vec<&str> = listed.iter().map(|s| s.id.as_str()).collect();
        assert_eq!(ids, vec!["b", "c", "a"]);
    }

    #[test]
    fn delete_session_works_and_errors_on_missing() {
        let _g = HomeGuard::new();
        let s = Session::new("temp".into(), "claude".into()).expect("new");
        s.save().expect("save");
        assert!(session_path("temp").unwrap().exists());

        delete_session("temp").expect("delete");
        assert!(!session_path("temp").unwrap().exists());
        assert!(delete_session("temp").is_err());
    }

    #[test]
    fn append_advances_updated_at() {
        let _g = HomeGuard::new();
        let mut s = Session::new("t".into(), "claude".into()).expect("new");
        let original = s.updated_at.clone();
        std::thread::sleep(std::time::Duration::from_millis(5));
        s.append("user", "hi");
        // updated_at must move forward (or at least change), and turns grew.
        assert_ne!(s.updated_at, original);
        assert_eq!(s.turns.len(), 1);
    }

    #[test]
    fn transcript_includes_every_turn() {
        let _g = HomeGuard::new();
        let mut s = Session::new("t".into(), "claude".into()).expect("new");
        s.append("user", "Q1");
        s.append("assistant", "A1");
        s.append("user", "Q2");
        let t = s.transcript();
        assert!(t.contains("[user] Q1"));
        assert!(t.contains("[assistant] A1"));
        assert!(t.contains("[user] Q2"));
    }
}
