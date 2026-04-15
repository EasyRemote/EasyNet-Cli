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

use crate::persistence::config;

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
            _ => anyhow::bail!(
                "unknown agent type: {s} (expected: claude-code, codex, codex-app-server)"
            ),
        }
    }
}

// ─── Agent Entry ─────────────────────────────────────────────────────────────

/// One row of the on-disk agent registry (`~/.easynet/agents.json`).
///
/// Field visibility policy:
///   - All fields are `pub(crate)` rather than `pub`. Two reasons:
///     (1) serde requires field visibility from *somewhere*, but the
///     downstream surface (other crates) should not see field layout —
///     when we add `tenant: String` later, only this crate has to change.
///     (2) intra-crate readers can still use field access for ergonomics,
///     but the *recommended* read path is the getter methods below, which
///     name-resolve identically to field access at the call site.
///
/// Mutation policy: prefer the typed builders (`with_label`, etc.) over
/// direct field writes — they keep CLI mutation paths consistent with the
/// invariants enforced at registration (`validate_agent_name`). Direct
/// `pub(crate)` field writes are available as the escape hatch for
/// `AgentEntry::new` and for tests.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct AgentEntry {
    pub(crate) agent_type: AgentType,
    #[serde(default = "default_command")]
    pub(crate) command: String,
    #[serde(default)]
    pub(crate) args: Vec<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub(crate) model: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub(crate) label: Option<String>,
    #[serde(default, skip_serializing_if = "BTreeMap::is_empty")]
    pub(crate) env: BTreeMap<String, String>,
    #[serde(default = "default_timeout")]
    pub(crate) timeout_secs: u64,
    #[serde(default = "default_max_output")]
    pub(crate) max_output_bytes: usize,
}

fn default_command() -> String {
    String::new()
}
fn default_timeout() -> u64 {
    300
}
fn default_max_output() -> usize {
    1_048_576
} // 1 MB

impl AgentEntry {
    // ── Typed builder ─────────────────────────────────────────────────
    //
    // `with_label` is the *only* sanctioned cross-module mutation path
    // for `AgentEntry` after construction. The fields themselves are
    // `pub(crate)` so intra-crate readers can use natural field-access
    // syntax, but writes must go through builders so the day a field
    // gains an invariant (e.g. labels capped at 64 chars), only the
    // builder needs to learn it.
    //
    // Read-side accessors are intentionally NOT added: every reader in
    // this crate uses field-access syntax against the `pub(crate)`
    // fields, and adding mirror getters would be either dead code (the
    // compiler flags it) or redundant noise. If a downstream crate ever
    // needs to *read* a field, add a getter at that point — not now.

    /// Replace the human-readable label. Returns `&mut Self` to allow
    /// the CLI `agent add` path to chain mutations without granting
    /// raw field-write permission to the rest of the crate.
    pub fn with_label(&mut self, label: Option<String>) -> &mut Self {
        self.label = label;
        self
    }

    /// Create a new agent entry with sensible defaults for the given type.
    pub fn new(agent_type: AgentType, model: Option<String>) -> Self {
        let (command, args) = match agent_type {
            AgentType::ClaudeCode => (
                "claude".to_string(),
                vec![
                    "-p".to_string(),
                    "--output-format".to_string(),
                    "text".to_string(),
                ],
            ),
            AgentType::Codex => ("codex".to_string(), vec!["exec".to_string()]),
            AgentType::CodexAppServer => ("codex".to_string(), vec!["app-server".to_string()]),
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
    // Read directly and classify the error, rather than `exists()`-then-
    // `read_to_string()`. The two-step form races with `easynet reset`
    // and `easynet agent remove` running in another terminal: the file
    // disappears between the exists check and the read, producing a
    // misleading "read failed" error when the correct answer is "no
    // registry exists, return default". Matching on `NotFound` gives
    // the same UX without the TOCTOU window.
    let data = match fs::read_to_string(&path) {
        Ok(s) => s,
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => {
            return Ok(AgentRegistry::default());
        }
        Err(e) => {
            return Err(anyhow::Error::new(e).context(format!("read {}", path.display())));
        }
    };
    if data.trim().is_empty() {
        return Ok(AgentRegistry::default());
    }
    serde_json::from_str(&data).with_context(|| format!("parse {}", path.display()))
}

/// Validate an agent name before it lands in the registry.
///
/// Agent names flow from this registry into:
/// 1. the A2A discovery label `a2a.agents_json` (see `shared/a2a_labels.rs`),
/// 2. the workspace path `~/.easynet/workspaces/<name>/`,
/// 3. the codex/claude `--agent <name>` argument,
/// 4. EAL member-call syntax (`<name>.chat(...)`).
///
/// All four surfaces assume a constrained character set. We reject anything
/// that would break path joins, shell argv, or the `a2a.*` reserved
/// prefix at the *registration* boundary so the bad input never reaches a
/// downstream consumer.
pub fn validate_agent_name(name: &str) -> anyhow::Result<()> {
    if name.is_empty() {
        anyhow::bail!("agent name must not be empty");
    }
    if name.len() > 63 {
        anyhow::bail!(
            "agent name '{name}' is too long ({} chars; max 63)",
            name.len()
        );
    }
    if !name
        .chars()
        .all(|c| c.is_ascii_lowercase() || c.is_ascii_digit() || c == '_' || c == '-')
    {
        anyhow::bail!(
            "agent name '{name}' must contain only lowercase ASCII letters, digits, '_', or '-'"
        );
    }
    // Reserved prefixes — the `a2a.*` namespace is owned by the A2A label
    // schema (`shared/a2a_labels.rs`), and `easynet*` is reserved for the
    // built-in MCP server identity. Both rules block the trivial collision
    // case where a user names an agent after a system identifier.
    if name.starts_with("a2a") || name.starts_with("easynet") {
        anyhow::bail!("agent name '{name}' uses a reserved prefix ('a2a*' or 'easynet*')");
    }
    Ok(())
}

pub fn save_agents(registry: &AgentRegistry) -> anyhow::Result<()> {
    // Validate every key once at the persistence boundary, so every code
    // path that builds a registry (CLI add, programmatic insert, manual
    // edit followed by re-save) gets the same guarantees.
    for name in registry.agents.keys() {
        validate_agent_name(name)?;
    }
    let path = agents_path();
    if let Some(parent) = path.parent() {
        fs::create_dir_all(parent)?;
    }
    let json = serde_json::to_string_pretty(registry)? + "\n";
    // Reuse the race-safe primitive in `config`. The previous open-coded
    // `path.with_extension("tmp"); write; rename` raced with itself: two
    // concurrent `easynet agent add` invocations both staged to the same
    // `.tmp` path, and writer B's contents could overwrite the file
    // between writer A's stage and rename. See iteration-1 audit notes.
    //
    // The registry can contain `env` entries with API tokens, so we ask
    // for owner-only permissions applied at stage time — a post-rename
    // chmod would leave a TOCTOU window where the file is briefly
    // world-readable at its final path.
    config::atomic_write_with_permissions(
        &path,
        json.as_bytes(),
        config::WritePermissions::OwnerReadWrite,
    )?;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn validate_agent_name_accepts_well_formed_names() {
        for name in ["claude", "codex", "claude-2", "my_agent", "a", "agent42"] {
            assert!(
                validate_agent_name(name).is_ok(),
                "expected '{name}' to be accepted"
            );
        }
    }

    #[test]
    fn validate_agent_name_rejects_empty_and_oversize() {
        assert!(validate_agent_name("").is_err());
        let too_long = "a".repeat(64);
        assert!(validate_agent_name(&too_long).is_err());
    }

    #[test]
    fn validate_agent_name_rejects_path_and_shell_metachars() {
        // Each of these would either break a path join, escape an EAL
        // member-call, or get re-interpreted by a shell expansion — block
        // them at the registration boundary.
        for bad in [
            "claude/foo",
            "../etc",
            "agent.name", // dot would collide with EAL `<agent>.chat` syntax
            "Claude",     // uppercase rejected for canonicalization
            "agent name",
            "agent;rm",
            "agent$VAR",
            "agent\n",
            "agent🤖", // non-ASCII rejected
        ] {
            assert!(
                validate_agent_name(bad).is_err(),
                "expected '{bad}' to be rejected"
            );
        }
    }

    #[test]
    fn validate_agent_name_rejects_reserved_prefixes() {
        // `a2a.*` is the discovery-label namespace; `easynet*` is the
        // built-in MCP server identity. Both must not be shadowable by a
        // user-registered agent.
        for reserved in ["a2a", "a2a-clone", "easynet", "easynet-fake"] {
            assert!(
                validate_agent_name(reserved).is_err(),
                "expected reserved name '{reserved}' to be rejected"
            );
        }
    }
}
