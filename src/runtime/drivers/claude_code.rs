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

/// One tool the LLM invoked during a run. Captured from the
/// stream-json `assistant.content[*].type == "tool_use"` blocks so
/// the chat ability handler can surface them in `tool_calls` for
/// observability.
///
/// The driver does not see (and does not need to see) the tool
/// result — claude-code sends results back to the LLM internally
/// via subsequent `user.content[*].type == "tool_result"` blocks.
/// We could capture those too in a future pass; v1 records only the
/// invocation so a caller can answer "did the LLM use my skill?".
#[derive(Debug, Clone, Default)]
pub struct ToolCallRecord {
    pub ability: String,
    pub args: serde_json::Value,
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
    /// Tool invocations the LLM made during this run, in order.
    /// Empty when the run made no tool calls (single-turn answer).
    pub tool_calls: Vec<ToolCallRecord>,
}

pub struct ClaudeOptions {
    pub model: Option<String>,
    pub timeout: Duration,
    pub max_output_bytes: usize,
    pub env: BTreeMap<String, String>,
    pub cwd: Option<PathBuf>,
    /// Persistent run directory. Used for per-run artefacts
    /// (`prompt.txt`, `response.md`, `meta.json`); the stream
    /// event log moved to the Timeline in PR-7 Commit 2.
    pub run_dir: Option<Arc<RunDir>>,
    /// PR-7 Commit 2: Timeline writer. When `Some`, each
    /// streamed stdout line is emitted as a `progress` event on
    /// the P1-P6 event log (and broadcast to any live
    /// subscribers). When `None`, stream events are observed by
    /// the driver's stat accumulator only — the legacy
    /// `runs/trace.jsonl` write path is gone.
    pub timeline: Option<Arc<crate::runtime::timeline::TimelineWriter>>,
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
            timeline: None,
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
        // The trailing `mcp__easynet` (no parens, no glob) authorises
        // every MCP tool exposed by the EasyNet workspace MCP server
        // — i.e. fs.read / fs.write / process.exec / shell.run /
        // http.request and the agent's own per-workspace abilities.
        // Without this, the spawned `claude -p` runs in non-interactive
        // mode and refuses to call MCP tools because no human is there
        // to approve. Claude Code's CLI accepts `mcp__<server>` to
        // mean "every tool from this MCP server is pre-allowed".
        "--allowedTools".to_string(),
        "Bash(open:*) Bash(ls:*) Bash(cat:*) Bash(pwd) Bash(mkdir:*) \
         Read Write Edit Glob Grep mcp__easynet"
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
        // G2 — installed skills as Claude Code plugins.
        // `fleet.skill_install` writes to <cwd>/skills/<name>/.
        // Claude Code's `--plugin-dir <path>` accepts a directory
        // whose subdirs each look like a plugin (containing a
        // skills/ / commands/ / agents/ / hooks/ subtree). When
        // an EasyNet-installed skill matches that layout — which
        // a github:owner/repo source typically does because
        // upstream Claude-skill repos are shaped that way — the
        // plugin gets discovered as `/{skill-name}` and the agent
        // can invoke it.
        //
        // Pre-fix the skill files were dropped on disk but the
        // adapter never told claude to look at them. The skill
        // was inert.
        let skills_dir = cwd.join("skills");
        if skills_dir.is_dir() {
            // Each subdirectory of skills/ is a candidate plugin.
            // Only push --plugin-dir entries for ones that look
            // plugin-shaped (contain a SKILL.md or plugin.json,
            // or have a skills/ subdir of their own — claude's
            // discovery is forgiving but we'd rather not point
            // it at empty dirs that would just print a warning).
            if let Ok(entries) = std::fs::read_dir(&skills_dir) {
                for entry in entries.flatten() {
                    let p = entry.path();
                    if !p.is_dir() {
                        continue;
                    }
                    let looks_plugin_shaped = p.join("plugin.json").is_file()
                        || p.join("SKILL.md").is_file()
                        || p.join("skills").is_dir()
                        || p.join("commands").is_dir();
                    if looks_plugin_shaped {
                        args.push("--plugin-dir".to_string());
                        args.push(p.to_string_lossy().to_string());
                    }
                }
            }
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
    let timeline_cb = opts.timeline.clone();

    let callback = Arc::new(move |line: &str| {
        // PR-7 Commit 2: emit each streamed line as a `progress`
        // event on the Timeline. The payload carries the raw
        // JSONL line from the driver (driver-specific shape);
        // downstream consumers (PR-10 services/chat, Frontend
        // resume) parse the shape they expect and ignore the
        // rest. The fsync on each emit is what gives us P2
        // (disk durable before broadcast wake); the cost per
        // chunk is bounded by LLM streaming rate, which is
        // slower than fsync on any reasonable disk.
        if let Some(tl) = &timeline_cb {
            let payload = match serde_json::from_str::<serde_json::Value>(line) {
                // Structured driver JSON — store the parsed
                // value so subscribers get a typed payload
                // instead of an opaque string.
                Ok(v) => serde_json::json!({"driver": "claude-code", "chunk": v}),
                // Non-JSON line (driver warning, stderr leak) —
                // store verbatim under `raw` so it's still
                // observable, just not typed.
                Err(_) => serde_json::json!({"driver": "claude-code", "raw": line}),
            };
            if let Err(e) = tl.emit("progress", Some(payload)) {
                // A sustained emit failure means the disk is
                // unreachable. One stderr line is the compromise
                // between surfacing the problem and not flooding
                // when the LLM is streaming tokens at high rate.
                eprintln!(
                    "[easynet warn] timeline progress emit failed ({e}); \
                     subsequent lines for this run may be lost"
                );
            }
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
                            let input = block.get("input").cloned().unwrap_or(Value::Null);
                            let summary = stream_ui::summarise_tool_input(name, &input);
                            stream_ui::print_tool_use(run_start, name, &summary);
                            // Capture the call into stats so the chat
                            // ability handler can surface it as
                            // tool_calls in the structured response.
                            // Recording happens here rather than in the
                            // UI helper because this is the only point
                            // where we hold both the parsed name and
                            // the unfiltered input value.
                            let mut s = lock_or_recover(stats);
                            s.tool_calls.push(ToolCallRecord {
                                ability: name.to_string(),
                                args: input,
                            });
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
    ) -> anyhow::Result<crate::runtime::adapter::AdapterOutput> {
        let (text, stats) = invoke(
            prompt,
            ClaudeOptions {
                model: entry.model.clone(),
                timeout: opts.timeout,
                max_output_bytes: opts.max_output_bytes,
                env: opts.env,
                cwd: Some(opts.cwd),
                run_dir: opts.run_dir,
                timeline: opts.timeline,
                // Honor `InvokeOpts::command` — dispatch filled
                // it from `AgentEntry::command`. Empty string
                // falls through to the driver default inside
                // `ClaudeOptions::resolved_command`.
                command: opts.command,
            },
        )?;
        // Project the driver's ToolCallRecord into the dispatch-
        // layer ToolCall (same shape, different module). The
        // driver layer can't depend on dispatch::ToolCall directly
        // without a circular import, so we do the projection at
        // the trait boundary.
        let tool_calls = stats
            .tool_calls
            .iter()
            .map(|r| crate::runtime::dispatch::ToolCall {
                ability: r.ability.clone(),
                args: r.args.clone(),
            })
            .collect();
        Ok(crate::runtime::adapter::AdapterOutput {
            content: text,
            usage: Some(run_stats_to_usage(&stats)),
            tool_calls,
        })
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
