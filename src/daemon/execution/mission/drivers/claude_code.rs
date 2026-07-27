// EasyNet CLI — Claude Code Agent Wrapper
// =========================================
//
// File: src/agent/claude_code.rs
// Description: Invokes Claude Code in print mode (claude -p) with streaming
//              JSON output so the user can observe tool calls live.
//
// Permission model:
//   - `--permission-mode acceptEdits` auto-approves file edits within the
//     agent's cwd (which is an isolated workspace under ~/.easynet/agents).
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
use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex, MutexGuard};
use std::time::Duration;

use serde_json::Value;

use crate::daemon::execution::mission::adapter::DriverCommand;
use crate::daemon::execution::mission::dispatch::ToolCall;
use crate::daemon::execution::mission::drivers::invocation_trace::{
    apply_tool_result_meta, parse_invocation_trace_metadata, text_to_json_value,
};
use crate::daemon::execution::mission::process_runner::{self, ChildOptions};
use crate::daemon::execution::mission::stream_ui::{self, Usage};

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
    /// Tool invocations the LLM made during this run, in order.
    /// Empty when the run made no tool calls (single-turn answer).
    pub tool_calls: Vec<ToolCall>,
    /// Claude-emitted session id parsed from the stream-json
    /// `session_id` field (echoed on every event). Surfaces back
    /// to the chat ability via `AdapterOutput::thread_id` so the
    /// caller can pass it as `resume_thread_id` on the next turn
    /// and `claude -p --resume <id>` continues the conversation.
    /// Empty string when no event carried a session_id (the
    /// stream ended before any was emitted).
    pub thread_id: String,
}

pub struct ClaudeOptions {
    pub model: Option<String>,
    pub timeout: Duration,
    pub max_output_bytes: usize,
    pub env: BTreeMap<String, String>,
    pub cwd: Option<PathBuf>,
    /// PR-7 Commit 2: Timeline writer. When `Some`, each
    /// streamed stdout line is emitted as a `progress` event on
    /// the P1-P6 event log (and broadcast to any live
    /// subscribers). When `None`, stream events are observed by
    /// the driver's stat accumulator only — the legacy
    /// `runs/trace.jsonl` write path is gone.
    pub timeline: Option<Arc<crate::daemon::execution::mission::timeline::TimelineWriter>>,
    /// Optional live-progress callback. When `Some`, the
    /// stdout-line callback invokes it once per streamed line
    /// (in addition to the durable `timeline` emit). The chat
    /// ability's stream_handler uses this to forward per-token
    /// progress to its broadcast channel; without it the
    /// stream surface was effectively snapshot+done.
    pub progress_tx: Option<Arc<dyn Fn(serde_json::Value) + Send + Sync>>,
    /// Binary state to spawn. Dispatch converts
    /// `AgentEntry::command` into a typed default-or-explicit
    /// state before the driver sees it.
    pub command: DriverCommand,
    /// When `Some(<UUID>)`, the driver continues an existing
    /// claude-code session via `--resume <id>` instead of starting
    /// a fresh one. The session is the same on-disk transcript
    /// claude persists under `~/.claude/`; resume replays it as
    /// turn-zero context for the model just like the interactive
    /// `/resume` picker does.
    pub resume_thread_id: Option<String>,
    /// When set on a fresh-conversation invocation, the driver
    /// passes `--session-id <uuid>` so the spawned `claude -p`
    /// uses the supplied id rather than minting its own. The
    /// chat ability uses this to pre-bind a caller-supplied id
    /// to the session — useful when the caller wants the same
    /// id across the bridge boundary as inside claude's transcript
    /// store. `None` lets claude mint its own (and the driver
    /// surfaces it back via `RunStats::thread_id`).
    pub fresh_session_id: Option<String>,
}

/// Default binary name when `ClaudeOptions::command` is
/// `DriverCommand::Default`.
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
            timeline: None,
            progress_tx: None,
            command: DriverCommand::Default,
            resume_thread_id: None,
            fresh_session_id: None,
        }
    }
}

impl ClaudeOptions {
    /// Resolve the binary the driver should spawn. Kept as a
    /// small helper so both the main streaming path and the
    /// doctor / follow-up invocations share one rule.
    pub fn resolved_command(&self) -> &str {
        self.command.resolve(DEFAULT_CLAUDE_BINARY)
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
        // Pre-authorise the EasyNet-shaped agent loop.
        //
        // `Bash(easynet:*)` is what a freshly-installed agent
        // needs to actually run the steps its seeded skills teach
        // (e.g. `easynet pages create`, `easynet ability deploy`)
        // — without it, the agent reads `easynet-pages-author`
        // SKILL.md and then stalls asking the (non-interactive)
        // dispatcher to approve every shell call. `Bash(curl:*)`
        // is included so the agent can verify its own deploy by
        // hitting the URL it just published.
        //
        // The rest of the list is unchanged from the prior allow
        // set (Bash safe-readers + Read/Write/Edit + the EasyNet
        // MCP namespace).
        "Bash(open:*) Bash(ls:*) Bash(cat:*) Bash(pwd) Bash(mkdir:*) \
         Bash(easynet:*) Bash(curl:*) \
         Read Write Edit Glob Grep mcp__easynet"
            .to_string(),
    ];

    if let Some(m) = &opts.model {
        args.push("--model".to_string());
        args.push(m.clone());
    }

    // Conversation-session controls. `claude -p` exposes two
    // mutually-exclusive forms:
    //
    //   --resume <id>      Continue a prior session by id; the
    //                      transcript is replayed as turn-zero
    //                      context for the model.
    //   --session-id <id>  Use a caller-supplied UUID for a fresh
    //                      session (claude would otherwise mint
    //                      its own).
    //
    // Resume wins when both are set — that is the path the chat
    // ability takes when the caller passes a previously-issued
    // session_id back. fresh_session_id is the optional pre-mint
    // path; it lets a caller pin a UUID before the first turn so
    // the same id is visible across both the EasyNet bridge and
    // claude's local store. Without either flag claude mints a
    // UUID itself and we capture it from the stream.
    if let Some(id) = &opts.resume_thread_id {
        args.push("--resume".to_string());
        args.push(id.clone());
    } else if let Some(id) = &opts.fresh_session_id {
        args.push("--session-id".to_string());
        args.push(id.clone());
    }

    // Explicitly load MCP config from the workspace.
    if let Some(cwd) = &opts.cwd {
        let mcp_json = cwd.join(".mcp.json");
        if mcp_json.exists() {
            args.push("--mcp-config".to_string());
            args.push(mcp_json.to_string_lossy().to_string());
        }
        append_claude_workspace_plugin_dirs(&mut args, cwd);
    }

    let arg_refs: Vec<&str> = args.iter().map(|s| s.as_str()).collect();

    stream_ui::print_header(opts.timeout.as_secs(), opts.model.as_deref());
    let run_start = std::time::Instant::now();

    // Shared state assembled from the stream.
    let final_text = Arc::new(Mutex::new(None::<String>));
    let stats = Arc::new(Mutex::new(RunStats::default()));
    let final_text_cb = Arc::clone(&final_text);
    let stats_cb = Arc::clone(&stats);
    let timeline_cb = opts.timeline.clone();
    let progress_tx_cb = opts.progress_tx.clone();

    let callback = Arc::new(move |line: &str| {
        // Build the progress payload once; reuse it for both
        // timeline emit and the live-broadcast callback so a
        // subscriber's view matches what's on disk.
        let payload = match serde_json::from_str::<serde_json::Value>(line) {
            // Structured driver JSON — store the parsed value
            // so subscribers get a typed payload instead of
            // an opaque string.
            Ok(v) => serde_json::json!({"driver": "claude-code", "chunk": v}),
            // Non-JSON line (driver warning, stderr leak) —
            // store verbatim under `raw` so it's still
            // observable, just not typed.
            Err(_) => serde_json::json!({"driver": "claude-code", "raw": line}),
        };

        // PR-7 Commit 2: durable timeline first. The fsync gives
        // us P2 (disk durable before broadcast wake); the cost
        // per chunk is bounded by the LLM streaming rate, which
        // is slower than fsync on any reasonable disk.
        if let Some(tl) = &timeline_cb {
            if let Err(e) = tl.emit("progress", Some(payload.clone())) {
                eprintln!(
                    "[easynet warn] timeline progress emit failed ({e}); \
                     subsequent lines for this run may be lost"
                );
            }
        }

        // Live-progress fan-out: forward the same payload to the
        // chat stream's broadcast channel so an InvokeBidi/Stream
        // subscriber sees per-token progress. Without this the
        // \"stream\" was effectively snapshot+done — the audit
        // conversation caught this in slice 32.
        if let Some(tx) = &progress_tx_cb {
            tx(payload);
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

    let text = lock_or_recover(&final_text).clone();

    if result.exit_code != 0 {
        anyhow::bail!(format_child_exit_error(
            &binary,
            &result,
            text.as_deref().unwrap_or("")
        ));
    }

    let mut final_stats = lock_or_recover(&stats).clone();
    if final_stats.duration_ms == 0 {
        final_stats.duration_ms = run_start.elapsed().as_millis() as u64;
    }

    let Some(text) = text else {
        anyhow::bail!(format_missing_final_result_event_error(&binary, &result));
    };
    Ok((text, final_stats))
}

fn append_claude_workspace_plugin_dirs(args: &mut Vec<String>, cwd: &Path) {
    // G2 — installed skills as Claude Code plugins.
    //
    // Claude Code owns a project-local skill convention:
    // `<cwd>/.claude/skills/<name>/`. Mission workspace seeding and
    // `skill.publish` for claude-code agents both write there. The
    // driver consumes that runtime-owned directory only; historical
    // `<cwd>/skills/` agent-private content must not influence process
    // launch.
    let skills_dir = cwd.join(".claude").join("skills");
    if !skills_dir.is_dir() {
        return;
    }
    let Ok(entries) = std::fs::read_dir(&skills_dir) else {
        return;
    };
    for entry in entries.flatten() {
        let p = entry.path();
        if !p.is_dir() || !looks_like_claude_plugin_dir(&p) {
            continue;
        }
        args.push("--plugin-dir".to_string());
        args.push(p.to_string_lossy().to_string());
    }
}

fn looks_like_claude_plugin_dir(path: &Path) -> bool {
    path.join("plugin.json").is_file()
        || path.join("SKILL.md").is_file()
        || path.join("skills").is_dir()
        || path.join("commands").is_dir()
}

fn format_child_exit_error(
    binary: &str,
    result: &process_runner::ChildResult,
    parsed_text: &str,
) -> String {
    let stderr = result.stderr.trim();
    if !stderr.is_empty() {
        return format!("{binary} error (exit {}): {}", result.exit_code, stderr);
    }

    let parsed = parsed_text.trim();
    if !parsed.is_empty() {
        return format!("{binary} error (exit {}): {}", result.exit_code, parsed);
    }

    let stdout_tail = result
        .stdout
        .lines()
        .rev()
        .map(str::trim)
        .find(|line| !line.is_empty())
        .unwrap_or("");
    if !stdout_tail.is_empty() {
        return format!(
            "{binary} error (exit {}): {}",
            result.exit_code, stdout_tail
        );
    }

    format!("{binary} exited with code {}", result.exit_code)
}

const MISSING_RESULT_STDOUT_PREVIEW_CHARS: usize = 1024;

fn format_missing_final_result_event_error(
    binary: &str,
    result: &process_runner::ChildResult,
) -> String {
    let stdout_preview =
        bounded_stdout_preview(&result.stdout, MISSING_RESULT_STDOUT_PREVIEW_CHARS);
    format!(
        "{binary} protocol error: process exited successfully but stream-json output did not \
         include a terminal result event; stdout_preview={stdout_preview:?}"
    )
}

fn bounded_stdout_preview(stdout: &str, max_chars: usize) -> String {
    let trimmed = stdout.trim();
    if trimmed.is_empty() || max_chars == 0 {
        return String::new();
    }
    let mut chars = trimmed.chars();
    let preview: String = chars.by_ref().take(max_chars).collect();
    if chars.next().is_some() {
        format!("{preview}…")
    } else {
        preview
    }
}

/// Parse one stream-json line and print a trace event to stderr.
fn handle_stream_line(
    line: &str,
    final_text: &Arc<Mutex<Option<String>>>,
    stats: &Arc<Mutex<RunStats>>,
    run_start: std::time::Instant,
) {
    let v: Value = match serde_json::from_str(line) {
        Ok(v) => v,
        Err(_) => return,
    };

    // Every claude-code stream event carries a `session_id` field
    // (UUID) — both the init banner and per-turn events echo it. We
    // capture the first one we see and stop overwriting; if `claude`
    // ever rotates ids mid-run (it does not today), the last writer
    // would silently win and the chat ability would return an id
    // the next `--resume` cannot find. Holding to first-seen makes
    // the surface stable.
    if let Some(sid) = v.get("session_id").and_then(Value::as_str) {
        let mut s = lock_or_recover(stats);
        if s.thread_id.is_empty() {
            s.thread_id = sid.to_string();
        }
    }

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
                            let tool_use_id =
                                block.get("id").and_then(Value::as_str).map(str::to_string);
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
                            s.tool_calls.push(ToolCall {
                                ability: name.to_string(),
                                args: input,
                                tool_use_id,
                                mcp_tool_name: mcp_tool_name_from_claude_tool(name),
                                ..Default::default()
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
                        let tool_use_id = block.get("tool_use_id").and_then(Value::as_str);
                        {
                            let mut s = lock_or_recover(stats);
                            apply_tool_result(
                                &mut s.tool_calls,
                                tool_use_id,
                                text_to_json_value(&text),
                                is_err.then(|| text.clone()),
                            );
                        }
                        let mut s = lock_or_recover(stats);
                        if tool_result_belongs_to_easynet(&s.tool_calls, tool_use_id) {
                            if let Some(meta) = parse_invocation_trace_metadata(&text) {
                                apply_tool_result_meta(&mut s.tool_calls, tool_use_id, meta);
                            }
                        }
                    }
                }
            }
        }
        "result" => {
            // Final result event — capture the assistant's final text plus
            // aggregate usage and cost metrics.
            *lock_or_recover(final_text) = Some(
                v.get("result")
                    .and_then(Value::as_str)
                    .unwrap_or("")
                    .to_string(),
            );
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

fn apply_tool_result(
    calls: &mut [ToolCall],
    tool_use_id: Option<&str>,
    result: Value,
    error: Option<String>,
) {
    let Some(id) = tool_use_id else {
        return;
    };
    let Some(call) = calls
        .iter_mut()
        .rev()
        .find(|call| call.tool_use_id.as_deref() == Some(id))
    else {
        return;
    };
    call.result = Some(result);
    if let Some(error) = error {
        call.error = Some(error);
    }
}

fn tool_result_belongs_to_easynet(calls: &[ToolCall], tool_use_id: Option<&str>) -> bool {
    let Some(id) = tool_use_id else {
        return false;
    };
    calls
        .iter()
        .rev()
        .any(|call| call.tool_use_id.as_deref() == Some(id) && call.mcp_tool_name.is_some())
}

fn mcp_tool_name_from_claude_tool(name: &str) -> Option<String> {
    name.strip_prefix("mcp__easynet__")
        .map(str::trim)
        .filter(|tool| !tool.is_empty())
        .map(str::to_string)
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

use crate::daemon::execution::mission::adapter::{AgentAdapter, InvokeOpts};
use crate::daemon::execution::mission::dispatch::AgentUsage;
use crate::daemon::persistence::agent_registry::AgentEntry;

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
    ) -> anyhow::Result<crate::daemon::execution::mission::adapter::AdapterOutput> {
        let (text, stats) = invoke(
            prompt,
            ClaudeOptions {
                model: entry.model.clone(),
                timeout: opts.timeout,
                max_output_bytes: opts.max_output_bytes,
                env: opts.env,
                cwd: Some(opts.cwd),
                timeline: opts.timeline,
                progress_tx: opts.progress_tx,
                // Honor `InvokeOpts::command` — dispatch converted
                // the persisted registry value into typed runtime
                // command state.
                command: opts.command,
                resume_thread_id: opts.resume_thread_id,
                // The fresh-session-id bind path is currently
                // unused by the chat ability (it lets claude mint
                // its own id and surfaces it back). Reserved for
                // future callers that need the id to be visible
                // before the first turn completes.
                fresh_session_id: None,
            },
        )?;
        let thread_id = if stats.thread_id.is_empty() {
            None
        } else {
            Some(stats.thread_id.clone())
        };
        Ok(crate::daemon::execution::mission::adapter::AdapterOutput {
            content: text,
            usage: Some(run_stats_to_usage(&stats)),
            tool_calls: stats.tool_calls.clone(),
            thread_id,
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

#[cfg(test)]
mod tests {
    use super::{
        append_claude_workspace_plugin_dirs, format_child_exit_error,
        format_missing_final_result_event_error, handle_stream_line, RunStats,
    };
    use crate::daemon::execution::mission::process_runner::ChildResult;
    use std::sync::{Arc, Mutex};
    use std::time::Duration;

    fn child(stdout: &str, stderr: &str, exit_code: i32) -> ChildResult {
        ChildResult {
            stdout: stdout.to_string(),
            stderr: stderr.to_string(),
            exit_code,
            duration: Duration::from_millis(1),
            truncated: false,
        }
    }

    #[test]
    fn child_exit_error_prefers_stderr() {
        let msg = format_child_exit_error(
            "claude",
            &child("", "permission denied", 1),
            "ignored parsed text",
        );
        assert_eq!(msg, "claude error (exit 1): permission denied");
    }

    #[test]
    fn child_exit_error_uses_parsed_result_when_stderr_is_empty() {
        let msg = format_child_exit_error(
            "claude",
            &child("{\"type\":\"result\"}\n", "", 1),
            "You're out of extra usage",
        );
        assert_eq!(msg, "claude error (exit 1): You're out of extra usage");
    }

    #[test]
    fn child_exit_error_falls_back_to_stdout_tail() {
        let msg = format_child_exit_error("claude", &child("\nline 1\nline 2\n", "", 1), "");
        assert_eq!(msg, "claude error (exit 1): line 2");
    }

    #[test]
    fn child_exit_error_falls_back_to_exit_code() {
        let msg = format_child_exit_error("claude", &child("", "", 1), "");
        assert_eq!(msg, "claude exited with code 1");
    }

    #[test]
    fn claude_driver_rejects_missing_final_result_event_with_bounded_stdout() {
        let long_stdout = format!("{}\n{}", "x".repeat(1500), "tail");
        let msg = format_missing_final_result_event_error("claude", &child(&long_stdout, "", 0));

        assert!(
            msg.contains("protocol error"),
            "missing terminal event must be a protocol error: {msg}"
        );
        assert!(
            msg.contains("terminal result event"),
            "error must name the missing terminal fact: {msg}"
        );
        assert!(
            msg.contains('…'),
            "long stdout diagnostics must be visibly truncated: {msg}"
        );
        assert!(
            !msg.contains("tail"),
            "bounded diagnostic must not expose unbounded raw stdout: {msg}"
        );
    }

    #[test]
    fn stream_result_event_records_empty_terminal_result() {
        let final_text = Arc::new(Mutex::new(None::<String>));
        let stats = Arc::new(Mutex::new(RunStats::default()));
        handle_stream_line(
            r#"{"type":"result","result":"","num_turns":1}"#,
            &final_text,
            &stats,
            std::time::Instant::now(),
        );

        assert_eq!(
            final_text.lock().unwrap().as_deref(),
            Some(""),
            "empty result event is still a terminal result fact"
        );
        assert_eq!(stats.lock().unwrap().num_turns, 1);
    }

    #[test]
    fn plugin_dirs_use_claude_project_skill_root_only() {
        let workspace = tempfile::tempdir().expect("workspace");

        let canonical = workspace
            .path()
            .join(".claude")
            .join("skills")
            .join("canonical");
        std::fs::create_dir_all(&canonical).expect("canonical skill dir");
        std::fs::write(canonical.join("SKILL.md"), "---\nname: canonical\n---\n")
            .expect("canonical skill");

        let legacy = workspace.path().join("skills").join("legacy");
        std::fs::create_dir_all(&legacy).expect("legacy skill dir");
        std::fs::write(legacy.join("SKILL.md"), "---\nname: legacy\n---\n").expect("legacy skill");

        let mut args = Vec::new();
        append_claude_workspace_plugin_dirs(&mut args, workspace.path());

        assert_eq!(args.len(), 2, "expected one --plugin-dir pair: {args:?}");
        assert_eq!(args[0], "--plugin-dir");
        assert_eq!(args[1], canonical.to_string_lossy());
        assert!(
            !args
                .iter()
                .any(|arg| arg.contains("/skills/legacy") || arg.ends_with("skills/legacy")),
            "legacy workspace skills path must not affect Claude launch args: {args:?}"
        );
    }

    #[test]
    fn stream_tool_result_backfills_easynet_invocation_identity() {
        let final_text = Arc::new(Mutex::new(None::<String>));
        let stats = Arc::new(Mutex::new(RunStats::default()));
        let start = std::time::Instant::now();
        handle_stream_line(
            r#"{"type":"assistant","message":{"content":[{"type":"tool_use","id":"toolu_1","name":"mcp__easynet__docetl_code_filter","input":{"rows":[1]}}]}}"#,
            &final_text,
            &stats,
            start,
        );
        handle_stream_line(
            r#"{"type":"user","message":{"content":[{"type":"tool_result","tool_use_id":"toolu_1","content":[{"type":"text","text":"{\"ok\":true,\"x-easynet-invocation\":{\"ability\":\"docetl.code_filter\",\"ability_ura\":\"easynet:///r/localhost/ability/device.dev-1.docetl.code_filter\",\"mcp_tool\":\"docetl_code_filter\",\"invocation_ura\":\"easynet:///r/localhost/resource/device.dev-1/invocation/req-1/history\",\"callee_ura\":\"easynet:///r/localhost/device/dev-1\"}}"}]}]}}"#,
            &final_text,
            &stats,
            start,
        );

        let stats = stats.lock().unwrap();
        assert_eq!(stats.tool_calls.len(), 1);
        let call = &stats.tool_calls[0];
        assert_eq!(call.ability, "docetl.code_filter");
        assert!(call.result.is_some());
        assert!(call.error.is_none());
        assert_eq!(call.mcp_tool_name.as_deref(), Some("docetl_code_filter"));
        assert_eq!(
            call.ability_ura.as_deref(),
            Some("easynet:///r/localhost/ability/device.dev-1.docetl.code_filter")
        );
        assert_eq!(
            call.invocation_ura.as_deref(),
            Some("easynet:///r/localhost/resource/device.dev-1/invocation/req-1/history")
        );
        assert_eq!(
            call.callee_ura.as_deref(),
            Some("easynet:///r/localhost/device/dev-1")
        );
    }

    #[test]
    fn stream_tool_result_ignores_trace_metadata_for_non_easynet_tool() {
        let final_text = Arc::new(Mutex::new(None::<String>));
        let stats = Arc::new(Mutex::new(RunStats::default()));
        let start = std::time::Instant::now();
        handle_stream_line(
            r#"{"type":"assistant","message":{"content":[{"type":"tool_use","id":"toolu_1","name":"web_search","input":{"query":"EasyNet"}}]}}"#,
            &final_text,
            &stats,
            start,
        );
        handle_stream_line(
            r#"{"type":"user","message":{"content":[{"type":"tool_result","tool_use_id":"toolu_1","content":[{"type":"text","text":"{\"x-easynet-invocation\":{\"ability\":\"spoofed.ability\",\"invocation_ura\":\"easynet:///r/localhost/invocation/spoof\"}}"}]}]}}"#,
            &final_text,
            &stats,
            start,
        );

        let stats = stats.lock().unwrap();
        assert_eq!(stats.tool_calls.len(), 1);
        let call = &stats.tool_calls[0];
        assert_eq!(call.ability, "web_search");
        assert!(call.invocation_ura.is_none());
        assert!(call.mcp_tool_name.is_none());
    }
}
