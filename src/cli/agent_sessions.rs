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

pub fn session_path(id: &str) -> PathBuf {
    root_dir().join(format!("{id}.json"))
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
    pub fn new(id: String, agent: String) -> Self {
        let now = Local::now().to_rfc3339();
        Self {
            id,
            agent,
            created_at: now.clone(),
            updated_at: now,
            turns: Vec::new(),
        }
    }

    pub fn save(&self) -> anyhow::Result<()> {
        fs::create_dir_all(root_dir())?;
        let path = session_path(&self.id);
        let json = serde_json::to_string_pretty(self)?;
        fs::write(path, json)?;
        Ok(())
    }

    pub fn load(id: &str) -> anyhow::Result<Self> {
        let path = session_path(id);
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
    let path = session_path(id);
    if !path.exists() {
        anyhow::bail!("session '{id}' not found");
    }
    fs::remove_file(path)?;
    Ok(())
}
