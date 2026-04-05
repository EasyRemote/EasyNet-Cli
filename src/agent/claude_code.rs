// EasyNet CLI — Claude Code Agent Wrapper
// =========================================
//
// File: src/agent/claude_code.rs
// Description: Invokes Claude Code in print mode (claude -p).
//
// MCP tools loaded via --mcp-config pointing to workspace .mcp.json.
// Knowledge loaded via CLAUDE.md in the workspace (auto-discovered by cwd).
// Uses -p mode which skips the workspace trust dialog.
//
// Author: Silan Hu <silan.hu@u.nus.edu>
// Copyright (c) 2026 EasyNet. All rights reserved.

use std::collections::BTreeMap;
use std::path::PathBuf;
use std::time::Duration;

use super::process_runner::{self, ChildOptions};

pub struct ClaudeOptions {
    pub model: Option<String>,
    pub timeout: Duration,
    pub max_output_bytes: usize,
    pub env: BTreeMap<String, String>,
    pub cwd: Option<PathBuf>,
}

impl Default for ClaudeOptions {
    fn default() -> Self {
        Self {
            model: None,
            timeout: Duration::from_secs(300),
            max_output_bytes: 1_048_576,
            env: BTreeMap::new(),
            cwd: None,
        }
    }
}

/// Invoke Claude Code in print mode.
///
/// Explicitly passes `--mcp-config .mcp.json` to guarantee MCP tool loading.
/// The `-p` flag skips workspace trust dialog, so explicit --mcp-config is needed.
pub fn invoke(prompt: &str, opts: ClaudeOptions) -> anyhow::Result<String> {
    let mut args: Vec<String> = vec![
        "-p".to_string(),
        "--output-format".to_string(),
        "text".to_string(),
    ];

    if let Some(m) = &opts.model {
        args.push("--model".to_string());
        args.push(m.clone());
    }

    // Explicitly load MCP config and system prompt from workspace.
    if let Some(cwd) = &opts.cwd {
        let mcp_json = cwd.join(".mcp.json");
        if mcp_json.exists() {
            args.push("--mcp-config".to_string());
            args.push(mcp_json.to_string_lossy().to_string());
        }
    }

    let arg_refs: Vec<&str> = args.iter().map(|s| s.as_str()).collect();

    let result = process_runner::run_child("claude", &arg_refs, ChildOptions {
        timeout: opts.timeout,
        max_stdout_bytes: opts.max_output_bytes,
        max_stderr_bytes: 262_144,
        stdin_data: Some(prompt.to_string()),
        env: opts.env,
        cwd: opts.cwd,
    })?;

    if result.exit_code != 0 {
        let err_msg = if result.stderr.is_empty() {
            format!("claude exited with code {}", result.exit_code)
        } else {
            format!("claude error (exit {}): {}", result.exit_code, result.stderr.trim())
        };
        anyhow::bail!(err_msg);
    }

    Ok(result.stdout)
}

/// Check if the `claude` CLI is available and return version info.
pub fn doctor() -> anyhow::Result<String> {
    let result = process_runner::run_child("claude", &["--version"], ChildOptions {
        timeout: Duration::from_secs(10),
        max_stdout_bytes: 4096,
        ..Default::default()
    })?;
    if result.exit_code != 0 {
        anyhow::bail!("claude --version failed (exit {})", result.exit_code);
    }
    Ok(result.stdout.trim().to_string())
}
