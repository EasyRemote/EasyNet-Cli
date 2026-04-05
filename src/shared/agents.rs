// EasyNet CLI — Agent Registry
// =============================
//
// File: src/shared/agents.rs
// Description: Persistent registry for AI agent configurations (~/.easynet/agents.json).
//
// Stores agent definitions (Claude Code, Codex, etc.) that can be invoked by
// `easynet agent send` or the multi-agent `easynet discuss` orchestrator.
//
// Separated from config.rs to preserve its three-domain contract
// (runtime state / credentials / device settings).
//
// Author: Silan Hu <silan.hu@u.nus.edu>
// Copyright (c) 2026 EasyNet. All rights reserved.

use std::collections::BTreeMap;
use std::fs;
use std::path::PathBuf;

use anyhow::Context;
use serde::{Deserialize, Serialize};

use super::config;

// ─── Agent Type ──────────────────────────────────────────────────────────────

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub enum AgentType {
    ClaudeCode,
    Codex,
    CodexAppServer,
}

impl std::fmt::Display for AgentType {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::ClaudeCode => write!(f, "claude-code"),
            Self::Codex => write!(f, "codex"),
            Self::CodexAppServer => write!(f, "codex-app-server"),
        }
    }
}

impl std::str::FromStr for AgentType {
    type Err = anyhow::Error;
    fn from_str(s: &str) -> Result<Self, Self::Err> {
        match s {
            "claude-code" | "claude" => Ok(Self::ClaudeCode),
            "codex" => Ok(Self::Codex),
            "codex-app-server" | "codex-appserver" => Ok(Self::CodexAppServer),
            _ => anyhow::bail!("unknown agent type: {s} (expected: claude-code, codex, codex-app-server)"),
        }
    }
}

// ─── Agent Entry ─────────────────────────────────────────────────────────────

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct AgentEntry {
    pub agent_type: AgentType,
    #[serde(default = "default_command")]
    pub command: String,
    #[serde(default)]
    pub args: Vec<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub model: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub label: Option<String>,
    #[serde(default, skip_serializing_if = "BTreeMap::is_empty")]
    pub env: BTreeMap<String, String>,
    #[serde(default = "default_timeout")]
    pub timeout_secs: u64,
    #[serde(default = "default_max_output")]
    pub max_output_bytes: usize,
}

fn default_command() -> String { String::new() }
fn default_timeout() -> u64 { 300 }
fn default_max_output() -> usize { 1_048_576 } // 1 MB

impl AgentEntry {
    /// Create a new agent entry with sensible defaults for the given type.
    pub fn new(agent_type: AgentType, model: Option<String>) -> Self {
        let (command, args) = match agent_type {
            AgentType::ClaudeCode => (
                "claude".to_string(),
                vec!["-p".to_string(), "--output-format".to_string(), "text".to_string()],
            ),
            AgentType::Codex => (
                "codex".to_string(),
                vec!["exec".to_string()],
            ),
            AgentType::CodexAppServer => (
                "codex".to_string(),
                vec!["app-server".to_string()],
            ),
        };
        Self {
            agent_type,
            command,
            args,
            model,
            label: None,
            env: BTreeMap::new(),
            timeout_secs: default_timeout(),
            max_output_bytes: default_max_output(),
        }
    }
}

// ─── Agent Registry ──────────────────────────────────────────────────────────

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct AgentRegistry {
    pub agents: BTreeMap<String, AgentEntry>,
}

fn agents_path() -> PathBuf {
    config::state_dir().join("agents.json")
}

pub fn load_agents() -> anyhow::Result<AgentRegistry> {
    let path = agents_path();
    if !path.exists() {
        return Ok(AgentRegistry::default());
    }
    let data = fs::read_to_string(&path)
        .with_context(|| format!("read {}", path.display()))?;
    if data.trim().is_empty() {
        return Ok(AgentRegistry::default());
    }
    serde_json::from_str(&data)
        .with_context(|| format!("parse {}", path.display()))
}

pub fn save_agents(registry: &AgentRegistry) -> anyhow::Result<()> {
    let path = agents_path();
    if let Some(parent) = path.parent() {
        fs::create_dir_all(parent)?;
    }
    let json = serde_json::to_string_pretty(registry)? + "\n";
    // Atomic write (tmp → rename).
    let tmp = path.with_extension("tmp");
    fs::write(&tmp, json.as_bytes())?;
    fs::rename(&tmp, &path)?;

    // Restrict permissions (may contain env secrets).
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        fs::set_permissions(&path, fs::Permissions::from_mode(0o600))?;
    }
    Ok(())
}
