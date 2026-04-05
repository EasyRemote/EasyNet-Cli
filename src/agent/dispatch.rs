// EasyNet CLI — Agent Dispatch
// =============================
//
// File: src/agent/dispatch.rs
// Description: Unified routing for agent invocation + MCP config generation + recursion guard.
//
// Author: Silan Hu <silan.hu@u.nus.edu>
// Copyright (c) 2026 EasyNet. All rights reserved.

use std::path::Path;
use std::time::{Duration, Instant};

use serde::{Deserialize, Serialize};

use crate::shared::agents::{AgentEntry, AgentType};

use super::claude_code::{self, ClaudeOptions};
use super::codex::{self, CodexOptions};
use super::workspace;

/// Maximum recursion depth for agent dispatch (prevents infinite loops).
const MAX_AGENT_DEPTH: u32 = 2;

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct AgentResponse {
    pub agent: String,
    pub content: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub model: Option<String>,
    pub duration_ms: u64,
    pub truncated: bool,
}

/// Send a prompt to a registered agent and return the response.
///
/// - Routes to the appropriate agent wrapper based on `entry.agent_type`.
/// - Sets `EASYNET_AGENT_DEPTH` in the child environment for recursion prevention.
/// - Optionally generates a temp MCP config so the agent can call back into EasyNet.
pub fn send_to_agent(
    agent_name: &str,
    entry: &AgentEntry,
    prompt: &str,
    context: Option<&str>,
    _mcp_config: Option<&Path>,
) -> anyhow::Result<AgentResponse> {
    // Recursion guard.
    let current_depth: u32 = std::env::var("EASYNET_AGENT_DEPTH")
        .ok()
        .and_then(|v| v.parse().ok())
        .unwrap_or(0);

    if current_depth >= MAX_AGENT_DEPTH {
        anyhow::bail!(
            "agent dispatch depth limit reached ({MAX_AGENT_DEPTH}). \
             Refusing to spawn nested agent to prevent infinite recursion."
        );
    }

    // Build full prompt with context.
    let full_prompt = compose_prompt(prompt, context);

    // Build env with depth guard.
    let mut env = entry.env.clone();
    env.insert("EASYNET_AGENT_DEPTH".to_string(), (current_depth + 1).to_string());

    let timeout = Duration::from_secs(entry.timeout_secs);
    let max_output = entry.max_output_bytes;
    let start = Instant::now();

    // Provision workspace with .claude/ or .codex/ config + CLAUDE.md/AGENTS.md.
    let workspace = workspace::ensure_workspace(agent_name, entry).ok();
    let cwd = workspace.clone();

    let content = match entry.agent_type {
        AgentType::ClaudeCode => {
            claude_code::invoke(&full_prompt, ClaudeOptions {
                model: entry.model.clone(),
                timeout,
                max_output_bytes: max_output,
                env,
                cwd,
            })?
        }
        AgentType::Codex => {
            codex::invoke_exec(&full_prompt, CodexOptions {
                model: entry.model.clone(),
                timeout,
                max_output_bytes: max_output,
                env,
                write_mode: false,
                cwd: workspace,
            })?
        }
        AgentType::CodexAppServer => {
            codex::invoke_app_server(&full_prompt, CodexOptions {
                model: entry.model.clone(),
                timeout,
                max_output_bytes: max_output,
                env,
                write_mode: false,
                cwd: workspace,
            })?
        }
    };

    Ok(AgentResponse {
        agent: agent_name.to_string(),
        content,
        model: entry.model.clone(),
        duration_ms: start.elapsed().as_millis() as u64,
        truncated: false,
    })
}

fn compose_prompt(prompt: &str, context: Option<&str>) -> String {
    match context.map(str::trim).filter(|s| !s.is_empty()) {
        Some(ctx) => format!("{prompt}\n\n## Context (previous discussion)\n\n{ctx}\n"),
        None => prompt.to_string(),
    }
}
