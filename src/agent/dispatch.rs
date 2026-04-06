// EasyNet CLI — Agent Dispatch
// =============================
//
// File: src/agent/dispatch.rs
// Description: Unified routing for agent invocation + per-run persistence +
//              recursion guard.
//
// Every call creates a timestamped run directory under the agent's workspace
// (`~/.easynet/workspaces/<agent>/runs/<stamp>/`) that stores the composed
// prompt, the raw stream trace, the final markdown response, and a meta.json
// with timing / token counts. The run directory path is surfaced on the
// returned `AgentResponse` so CLI callers can show it to the user.
//
// Author: Silan Hu <silan.hu@u.nus.edu>
// Copyright (c) 2026 EasyNet. All rights reserved.

use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::time::{Duration, Instant};

use chrono::Local;
use serde::{Deserialize, Serialize};

use crate::shared::agents::{AgentEntry, AgentType};

use super::claude_code::{self, ClaudeOptions};
use super::codex::{self, CodexOptions};
use super::run_store::{RunDir, RunMeta};
use super::workspace;

/// Maximum recursion depth for agent dispatch (prevents infinite loops).
const MAX_AGENT_DEPTH: u32 = 2;

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct AgentUsage {
    pub input_tokens: u64,
    pub output_tokens: u64,
    pub cache_read_tokens: u64,
    pub cache_creation_tokens: u64,
    pub num_turns: u64,
    pub total_cost_usd: f64,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct AgentResponse {
    pub agent: String,
    pub content: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub model: Option<String>,
    pub duration_ms: u64,
    pub truncated: bool,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub usage: Option<AgentUsage>,
    /// Path to the per-run directory on disk (if persistence succeeded).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub run_dir: Option<PathBuf>,
}

/// Send a prompt to a registered agent and return the response.
///
/// - Routes to the appropriate agent wrapper based on `entry.agent_type`.
/// - Sets `EASYNET_AGENT_DEPTH` in the child environment for recursion prevention.
/// - Creates a per-run directory under the agent workspace and writes
///   prompt / response / trace / meta files.
pub fn send_to_agent(
    agent_name: &str,
    entry: &AgentEntry,
    prompt: &str,
    context: Option<&str>,
    _mcp_config: Option<&Path>,
    extra_trace_path: Option<&Path>,
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

    // Create a per-run directory. If creation fails (unlikely), we just skip
    // persistence — the agent call still runs as normal.
    let run_dir: Option<Arc<RunDir>> = RunDir::create(agent_name).ok().map(Arc::new);
    if let Some(dir) = &run_dir {
        dir.write_prompt(&full_prompt);
    }

    // Legacy `--trace <path>` still supported: mirror the prompt next to the
    // user-supplied trace file.
    if let Some(tp) = extra_trace_path {
        if let Some(parent) = tp.parent() {
            let _ = std::fs::create_dir_all(parent);
        }
        let prompt_path = tp.with_extension("prompt.txt");
        let _ = std::fs::write(&prompt_path, &full_prompt);
    }

    let started_at = Local::now().to_rfc3339();
    let run_result: anyhow::Result<(String, Option<AgentUsage>)> = match entry.agent_type {
        AgentType::ClaudeCode => {
            claude_code::invoke(&full_prompt, ClaudeOptions {
                model: entry.model.clone(),
                timeout,
                max_output_bytes: max_output,
                env,
                cwd,
                run_dir: run_dir.clone(),
            })
            .map(|(text, stats)| {
                (text, Some(AgentUsage {
                    input_tokens: stats.input_tokens,
                    output_tokens: stats.output_tokens,
                    cache_read_tokens: stats.cache_read_tokens,
                    cache_creation_tokens: stats.cache_creation_tokens,
                    num_turns: stats.num_turns,
                    total_cost_usd: stats.total_cost_usd,
                }))
            })
        }
        AgentType::Codex => {
            codex::invoke_exec(&full_prompt, CodexOptions {
                model: entry.model.clone(),
                timeout,
                max_output_bytes: max_output,
                env,
                write_mode: false,
                cwd: workspace,
                run_dir: run_dir.clone(),
            })
            .map(|(text, stats)| {
                (text, Some(AgentUsage {
                    input_tokens: stats.input_tokens,
                    output_tokens: stats.output_tokens,
                    cache_read_tokens: stats.cache_read_tokens,
                    cache_creation_tokens: stats.cache_creation_tokens,
                    num_turns: stats.num_turns,
                    total_cost_usd: stats.total_cost_usd,
                }))
            })
        }
        AgentType::CodexAppServer => {
            codex::invoke_app_server(&full_prompt, CodexOptions {
                model: entry.model.clone(),
                timeout,
                max_output_bytes: max_output,
                env,
                write_mode: false,
                cwd: workspace,
                run_dir: run_dir.clone(),
            })
            .map(|text| (text, None))
        }
    };

    // Write meta.json regardless of success/failure so failed runs are still
    // inspectable.
    let duration_ms = start.elapsed().as_millis() as u64;
    if let Some(dir) = &run_dir {
        let (exit_status, error, content_for_meta, usage_for_meta) = match &run_result {
            Ok((text, usage)) => ("ok".to_string(), None, Some(text.as_str()), usage.clone()),
            Err(e) => ("error".to_string(), Some(e.to_string()), None, None),
        };
        if let Some(text) = content_for_meta {
            dir.write_response(text);
        }
        let u = usage_for_meta.unwrap_or_default();
        dir.write_meta(&RunMeta {
            agent: agent_name.to_string(),
            agent_type: entry.agent_type.to_string(),
            model: entry.model.clone(),
            started_at,
            duration_ms,
            exit_status,
            error,
            input_tokens: u.input_tokens,
            output_tokens: u.output_tokens,
            cache_read_tokens: u.cache_read_tokens,
            cache_creation_tokens: u.cache_creation_tokens,
            num_turns: u.num_turns,
            total_cost_usd: u.total_cost_usd,
        });
    }

    let (content, usage) = run_result?;

    Ok(AgentResponse {
        agent: agent_name.to_string(),
        content,
        model: entry.model.clone(),
        duration_ms,
        truncated: false,
        usage,
        run_dir: run_dir.as_ref().map(|d| d.path().to_path_buf()),
    })
}

fn compose_prompt(prompt: &str, context: Option<&str>) -> String {
    match context.map(str::trim).filter(|s| !s.is_empty()) {
        Some(ctx) => format!("{prompt}\n\n## Context (previous discussion)\n\n{ctx}\n"),
        None => prompt.to_string(),
    }
}
