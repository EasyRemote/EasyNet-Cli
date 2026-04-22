// EasyNet CLI — Claude Code Agent Wrapper
// =========================================
//
// File: src/agent/claude_code.rs
// Description: Invokes Claude Code in print mode (claude -p) with streaming
//              JSON output so the user can observe tool calls live.
//
// Permission model:
//   - `--permission-mode acceptEdits` auto-approves file edits within the
//     agent's cwd (which is an isolated workspace under ~/.easynet/workspaces).
//   - `--allowedTools` additionally whitelists Bash operations that agents
//     commonly need (opening generated files, listing the workspace, etc.)
//     so they run without interactive prompts.
//
// Output model:
//   - `--output-format stream-json --verbose` emits one JSON event per line.
//   - We parse each line, print a compact human-readable trace to stderr via
//     the shared `stream_ui` module, and collect the final assistant text
//     and run statistics as the return value.
//   - If a `RunDir` is provided, every raw line is also mirrored to
//     `<run>/trace.jsonl` for later inspection.
//
// Author: Silan Hu <silan.hu@u.nus.edu>
// Copyright (c) 2026 EasyNet. All rights reserved.

use std::collections::BTreeMap;
use std::path::PathBuf;
use std::sync::{Arc, Mutex, MutexGuard};
use std::time::Duration;

use serde_json::Value;

use crate::runtime::process_runner::{self, ChildOptions};
use crate::runtime::run_store::RunDir;
use crate::runtime::stream_ui::{self, Usage};

/// Acquire a mutex guard, recovering from poisoning.
///
/// A poisoned mutex means *a previous holder of this lock panicked*.
/// For the shared accumulator mutexes used by the stream-reader and
/// the caller thread, the data inside is always safe to observe: it
/// is a plain data aggregator (`String` for final text, `RunStats` as
/// plain numeric fields) with no cross-field invariants that could be
/// mid-update. Treating poisoning as fatal here would escalate one
/// reader-thread panic into the death of the agent runner — the user
/// would see "agent invocation died" when the actual fault was a
/// single malformed JSON line that the line-callback's panic hook
/// already logged and recovered from. We accept the stale read
/// instead.
fn lock_or_recover<T>(m: &Mutex<T>) -> MutexGuard<'_, T> {
    match m.lock() {
        Ok(g) => g,
        Err(poisoned) => poisoned.into_inner(),
    }
}

/// Summary of a completed Claude Code run, extracted from the final
/// `result` event in the stream-json output.
#[derive(Default, Clone)]
pub struct RunStats {
    pub input_tokens: u64,
    pub output_tokens: u64,
    pub cache_read_tokens: u64,
    pub cache_creation_tokens: u64,
    pub num_turns: u64,
    pub total_cost_usd: f64,
    pub duration_ms: u64,
}

pub struct ClaudeOptions {
    pub model: Option<String>,
    pub timeout: Duration,
    pub max_output_bytes: usize,
    pub env: BTreeMap<String, String>,
    pub cwd: Option<PathBuf>,
    /// Persistent run directory. All stream events are mirrored into
    /// `<run>/trace.jsonl` when provided.
    pub run_dir: Option<Arc<RunDir>>,
    /// Binary to spawn. Empty string means "use the driver
    /// default" (`DEFAULT_CLAUDE_BINARY`). Dispatch fills this
    /// from `AgentEntry::command` so operators who have a
    /// custom install path (or a test that wires a fake binary
    /// through `dummy_entry`) see their override honored. The
    /// fallback mirrors the pre-refactor default so existing
    /// registry rows with an empty `command` field keep working.
    pub command: String,
}

/// Default binary name when `ClaudeOptions::command` is empty.
/// Exposed as a constant so tests and adapters can name it
/// without re-hardcoding the string.
pub const DEFAULT_CLAUDE_BINARY: &str = "claude";

impl Default for ClaudeOptions {
    fn default() -> Self {
        Self {
            model: None,
            timeout: Duration::from_secs(300),
            max_output_bytes: 1_048_576,
            env: BTreeMap::new(),
            cwd: None,
            run_dir: None,
            command: String::new(),
        }
    }
}

impl ClaudeOptions {
    /// Resolve the binary the driver should spawn. Empty
    /// `command` yields the default; anything else is returned
    /// verbatim. Kept as a small helper so both the main
    /// streaming path and the doctor / follow-up invocations
    /// share one rule.
    pub fn resolved_command(&self) -> &str {
        if self.command.is_empty() {
            DEFAULT_CLAUDE_BINARY
        } else {
            self.command.as_str()
        }
    }
}

/// Invoke Claude Code in streaming print mode.
///
/// Prints a live timeline of tool calls to stderr and returns the final
/// assistant text plus run statistics.
pub fn invoke(prompt: &str, opts: ClaudeOptions) -> anyhow::Result<(String, RunStats)> {
    let mut args: Vec<String> = vec![
        "-p".to_string(),
        "--output-format".to_string(),
        "stream-json".to_string(),
        "--verbose".to_string(),
        // Auto-accept file edits within cwd (the isolated workspace).
        // Writes outside cwd still require approval.
        "--permission-mode".to_string(),
        "acceptEdits".to_string(),
        // Pre-authorise common read-only / launch shell commands so the
        // agent doesn't stall waiting for approval on `open`, `ls`, etc.
        "--allowedTools".to_string(),
        "Bash(open:*) Bash(ls:*) Bash(cat:*) Bash(pwd) Bash(mkdir:*) \
         Read Write Edit Glob Grep"
            .to_string(),
    ];

    if let Some(m) = &opts.model {
        args.push("--model".to_string());
        args.push(m.clone());
    }

    // Explicitly load MCP config from the workspace.
    if let Some(cwd) = &opts.cwd {
        let mcp_json = cwd.join(".mcp.json");
        if mcp_json.exists() {
            args.push("--mcp-config".to_string());
            args.push(mcp_json.to_string_lossy().to_string());
        }
    }

    let arg_refs: Vec<&str> = args.iter().map(|s| s.as_str()).collect();

    stream_ui::print_header(opts.timeout.as_secs(), opts.model.as_deref());
    let run_start = std::time::Instant::now();

    // Shared state assembled from the stream.
    let final_text = Arc::new(Mutex::new(String::new()));
    let stats = Arc::new(Mutex::new(RunStats::default()));
    let final_text_cb = Arc::clone(&final_text);
    let stats_cb = Arc::clone(&stats);
    let run_dir_cb = opts.run_dir.clone();

    let callback = Arc::new(move |line: &str| {
        if let Some(dir) = &run_dir_cb {
            dir.append_trace_line(line);
        }
        handle_stream_line(line, &final_text_cb, &stats_cb, run_start);
    });

    // Resolved once here so both the spawn call and the error
    // messages below name the same binary — an override like
    // `dummy_entry`'s bogus command must surface in the error
    // the operator reads, not be silently swapped out.
    let binary = opts.resolved_command().to_string();

    let result = process_runner::run_child(
        &binary,
        &arg_refs,
        ChildOptions {
            timeout: opts.timeout,
            max_stdout_bytes: opts.max_output_bytes,
            max_stderr_bytes: 262_144,
            stdin_data: Some(prompt.to_string()),
            env: opts.env,
            cwd: opts.cwd,
            stdout_line_callback: Some(callback),
        },
    )?;

    if result.exit_code != 0 {
        let err_msg = if result.stderr.is_empty() {
            format!("{binary} exited with code {}", result.exit_code)
        } else {
            format!(
                "{binary} error (exit {}): {}",
                result.exit_code,
                result.stderr.trim()
            )
        };
        anyhow::bail!(err_msg);
    }

    let text = lock_or_recover(&final_text).clone();
    let mut final_stats = lock_or_recover(&stats).clone();
    if final_stats.duration_ms == 0 {
        final_stats.duration_ms = run_start.elapsed().as_millis() as u64;
    }

    if text.is_empty() {
        // Fallback: stream didn't yield a result event (unexpected). Return
        // raw stdout so the caller still sees something useful.
        Ok((result.stdout, final_stats))
    } else {
        Ok((text, final_stats))
    }
}

/// Parse one stream-json line and print a trace event to stderr.
fn handle_stream_line(
    line: &str,
    final_text: &Arc<Mutex<String>>,
    stats: &Arc<Mutex<RunStats>>,
    run_start: std::time::Instant,
) {
    let v: Value = match serde_json::from_str(line) {
        Ok(v) => v,
        Err(_) => return,
    };

    let kind = v.get("type").and_then(Value::as_str).unwrap_or("");

    match kind {
        "system" => {
            // init banner already printed by the caller; ignore.
        }
        "assistant" => {
            if let Some(content) = v.pointer("/message/content").and_then(Value::as_array) {
                for block in content {
                    let btype = block.get("type").and_then(Value::as_str).unwrap_or("");
                    match btype {
                        "text" => {
                            if let Some(text) = block.get("text").and_then(Value::as_str) {
                                stream_ui::print_assistant_text(run_start, text);
                            }
                        }
                        "tool_use" => {
                            let name = block.get("name").and_then(Value::as_str).unwrap_or("?");
                            let summary = stream_ui::summarise_tool_input(
                                name,
                                block.get("input").unwrap_or(&Value::Null),
                            );
                            stream_ui::print_tool_use(run_start, name, &summary);
                        }
                        _ => {}
                    }
                }
            }
            // Track cumulative token usage from the assistant message and
            // print a running total after each turn so the user can see how
            // the budget is growing live.
            if let Some(usage) = v.pointer("/message/usage") {
                let mut s = lock_or_recover(stats);
                s.input_tokens = usage
                    .get("input_tokens")
                    .and_then(Value::as_u64)
                    .unwrap_or(s.input_tokens);
                s.output_tokens = usage
                    .get("output_tokens")
                    .and_then(Value::as_u64)
                    .unwrap_or(s.output_tokens);
                s.cache_read_tokens = usage
                    .get("cache_read_input_tokens")
                    .and_then(Value::as_u64)
                    .unwrap_or(s.cache_read_tokens);
                s.cache_creation_tokens = usage
                    .get("cache_creation_input_tokens")
                    .and_then(Value::as_u64)
                    .unwrap_or(s.cache_creation_tokens);
                stream_ui::print_usage(
                    run_start,
                    &Usage {
                        input_tokens: s.input_tokens,
                        output_tokens: s.output_tokens,
                        cache_read_tokens: s.cache_read_tokens,
                        cache_creation_tokens: s.cache_creation_tokens,
                    },
                );
            }
        }
        "user" => {
            // A user-role message from the stream carries tool_result blocks.
            if let Some(content) = v.pointer("/message/content").and_then(Value::as_array) {
                for block in content {
                    if block.get("type").and_then(Value::as_str) == Some("tool_result") {
                        let is_err = block
                            .get("is_error")
                            .and_then(Value::as_bool)
                            .unwrap_or(false);
                        let text = extract_tool_result_text(block);
                        stream_ui::print_tool_result(run_start, &text, is_err);
                    }
                }
            }
        }
        "result" => {
            // Final result event — capture the assistant's final text plus
            // aggregate usage and cost metrics.
            if let Some(text) = v.get("result").and_then(Value::as_str) {
                *lock_or_recover(final_text) = text.to_string();
            }
            let mut s = lock_or_recover(stats);
            if let Some(n) = v.get("num_turns").and_then(Value::as_u64) {
                s.num_turns = n;
            }
            if let Some(c) = v.get("total_cost_usd").and_then(Value::as_f64) {
                s.total_cost_usd = c;
            }
            if let Some(d) = v.get("duration_ms").and_then(Value::as_u64) {
                s.duration_ms = d;
            }
            if let Some(usage) = v.get("usage") {
                s.input_tokens = usage
                    .get("input_tokens")
                    .and_then(Value::as_u64)
                    .unwrap_or(s.input_tokens);
                s.output_tokens = usage
                    .get("output_tokens")
                    .and_then(Value::as_u64)
                    .unwrap_or(s.output_tokens);
                s.cache_read_tokens = usage
                    .get("cache_read_input_tokens")
                    .and_then(Value::as_u64)
                    .unwrap_or(s.cache_read_tokens);
                s.cache_creation_tokens = usage
                    .get("cache_creation_input_tokens")
                    .and_then(Value::as_u64)
                    .unwrap_or(s.cache_creation_tokens);
            }
        }
        _ => {}
    }
}

fn extract_tool_result_text(block: &Value) -> String {
    match block.get("content") {
        Some(Value::String(s)) => s.clone(),
        Some(Value::Array(arr)) => arr
            .iter()
            .filter_map(|b| b.get("text").and_then(Value::as_str))
            .collect::<Vec<_>>()
            .join(" "),
        _ => String::new(),
    }
}

/// Check if the `claude` CLI is available and return version info.
pub fn doctor() -> anyhow::Result<String> {
    let result = process_runner::run_child(
        "claude",
        &["--version"],
        ChildOptions {
            timeout: Duration::from_secs(10),
            max_stdout_bytes: 4096,
            ..Default::default()
        },
    )?;
    if result.exit_code != 0 {
        anyhow::bail!("claude --version failed (exit {})", result.exit_code);
    }
    Ok(result.stdout.trim().to_string())
}

// ── AgentAdapter impl ───────────────────────────────────────────────────────
//
// The adapter is a zero-sized type so the dispatch table can hand
// out `&'static dyn AgentAdapter` references without any
// allocation. All invocation state lives on the stack inside
// `invoke`, matching the existing free-function shape.

use crate::registry::agents::AgentEntry;
use crate::runtime::adapter::{AgentAdapter, InvokeOpts};
use crate::runtime::dispatch::AgentUsage;

pub(crate) struct ClaudeCodeAdapter;

impl AgentAdapter for ClaudeCodeAdapter {
    fn runtime_id(&self) -> &'static str {
        "claude-code"
    }

    fn is_available(&self) -> bool {
        doctor().is_ok()
    }

    fn invoke(
        &self,
        entry: &AgentEntry,
        prompt: &str,
        opts: InvokeOpts,
    ) -> anyhow::Result<(String, Option<AgentUsage>)> {
        let (text, stats) = invoke(
            prompt,
            ClaudeOptions {
                model: entry.model.clone(),
                timeout: opts.timeout,
                max_output_bytes: opts.max_output_bytes,
                env: opts.env,
                cwd: Some(opts.cwd),
                run_dir: opts.run_dir,
                // Honor `InvokeOpts::command` — dispatch filled
                // it from `AgentEntry::command`. Empty string
                // falls through to the driver default inside
                // `ClaudeOptions::resolved_command`.
                command: opts.command,
            },
        )?;
        Ok((text, Some(run_stats_to_usage(&stats))))
    }
}

fn run_stats_to_usage(s: &RunStats) -> AgentUsage {
    AgentUsage {
        input_tokens: s.input_tokens,
        output_tokens: s.output_tokens,
        cache_read_tokens: s.cache_read_tokens,
        cache_creation_tokens: s.cache_creation_tokens,
        num_turns: s.num_turns,
        total_cost_usd: s.total_cost_usd,
    }
}
