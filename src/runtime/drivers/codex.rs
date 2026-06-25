// EasyNet CLI — Codex Agent Wrapper
// ===================================
//
// File: src/agent/codex.rs
// Description: Invokes Codex in two modes:
//   1. `codex exec --json` — streaming JSONL mode, used for `agent send`.
//      Matches the Claude Code wrapper's UX: live timeline, run directory,
//      trace file, run stats.
//   2. `codex app-server` — JSON-RPC 2.0 protocol (used by higher-level
//      conversation flows). Unchanged apart from option plumbing.
//
// Codex JSONL event types we care about (observed from `codex exec --json`):
//   - thread.started           : session init with thread_id
//   - turn.started             : turn begins
//   - item.started             : tool/command starting (e.g. command_execution)
//   - item.completed           : tool/command/reasoning/agent_message finished
//       * type=command_execution : shell command + output
//       * type=reasoning         : model thinking text
//       * type=agent_message     : final assistant message to the user
//       * type=file_change       : write/edit result (if emitted)
//   - turn.completed           : turn done, includes `usage`
//
// Usage field shape (Codex):
//   {"input_tokens": N, "cached_input_tokens": N, "output_tokens": N}
// There is no separate cache_creation field — we map cached_input_tokens to
// our `cache_read_tokens` slot and leave cache_creation at zero.
//
// Author: Silan Hu <silan.hu@u.nus.edu>
// Copyright (c) 2026 EasyNet. All rights reserved.

use std::collections::BTreeMap;
use std::path::PathBuf;
use std::sync::{Arc, Mutex, MutexGuard};
use std::time::Duration;

use serde_json::Value;

use crate::runtime::dispatch::ToolCall;
use crate::runtime::drivers::invocation_trace::{
    apply_tool_result_meta, parse_invocation_trace_metadata, text_to_json_value, EASYNET_MCP_SERVER,
};
use crate::runtime::process_runner::{self, ChildOptions};
use crate::runtime::stream_ui::{self, Usage};
use crate::runtime::toml_escape::toml_basic_string;
use crate::runtime::workspace;

/// Acquire a mutex guard, recovering from poisoning.
///
/// See the matching helper in `claude_code.rs` for the full
/// rationale: our accumulator mutexes carry plain data with no
/// cross-field invariants, so treating a previous-holder panic as
/// fatal would escalate a tolerable stream-reader-thread panic into
/// a dead agent runner. The line-callback panic hook has already
/// logged the failure; accepting a possibly-stale read here is the
/// graceful-degradation path.
fn lock_or_recover<T>(m: &Mutex<T>) -> MutexGuard<'_, T> {
    match m.lock() {
        Ok(g) => g,
        Err(poisoned) => poisoned.into_inner(),
    }
}

/// Stats collected from a Codex run. Mirrors the Claude Code shape so the
/// dispatch layer can treat both agents uniformly.
#[derive(Default, Clone)]
pub struct RunStats {
    pub input_tokens: u64,
    pub output_tokens: u64,
    pub cache_read_tokens: u64,
    pub cache_creation_tokens: u64,
    pub num_turns: u64,
    pub total_cost_usd: f64,
    pub duration_ms: u64,
    /// Codex-assigned conversation/thread id parsed from the
    /// `thread.started` event. The chat ability returns this back
    /// to the caller as `session_id` so a subsequent invocation can
    /// pass it as `resume_thread_id` and continue the same
    /// conversation. Empty string when no `thread.started` line
    /// was observed (codex exited before emitting one).
    pub thread_id: String,
    /// Tool invocations the LLM made during this run, in order.
    pub tool_calls: Vec<ToolCall>,
}

pub struct CodexOptions {
    pub model: Option<String>,
    pub timeout: Duration,
    pub max_output_bytes: usize,
    pub env: BTreeMap<String, String>,
    #[allow(dead_code)] // Reserved for future --write / --full-auto mode toggle.
    pub write_mode: bool,
    /// Workspace directory (with .codex/ config). If set, codex runs in this cwd.
    pub cwd: Option<PathBuf>,
    /// PR-7 Commit 2: Timeline writer. Same semantics as
    /// `ClaudeOptions::timeline` — each streamed stdout line
    /// emits a `progress` event on the P1-P6 event log.
    pub timeline: Option<Arc<crate::runtime::timeline::TimelineWriter>>,
    /// Optional live-progress callback. Mirrors
    /// `ClaudeOptions::progress_tx`: when set, the driver
    /// invokes it once per stdout line (in addition to the
    /// durable timeline emit). The chat ability's
    /// stream_handler installs this so InvokeBidi/Stream
    /// subscribers see per-token progress, not just the
    /// terminal frame. Pre-fix this hook didn't exist on the
    /// codex side, so claude.chat streamed but codex.chat did
    /// not — a wire-shape inconsistency between drivers
    /// caught in the audit conversation right after the
    /// claude streaming was fixed.
    pub progress_tx: Option<Arc<dyn Fn(serde_json::Value) + Send + Sync>>,
    /// Binary to spawn. Empty string means "use the driver
    /// default" (`DEFAULT_CODEX_BINARY`). Mirrors the behaviour
    /// of `ClaudeOptions::command`; see that field's rustdoc
    /// for the motivation.
    pub command: String,
    /// When `Some(<UUIDv7>)`, the driver invokes
    /// `codex exec resume <thread_id> <prompt>` instead of
    /// `codex exec <prompt>`, continuing the codex-side
    /// conversation that was started under that id. `None` is the
    /// fresh-conversation path (the legacy default).
    ///
    /// Why we delegate session storage to codex itself rather than
    /// re-implementing it: codex already persists every turn under
    /// `~/.codex/sessions/<yyyy>/<mm>/<dd>/rollout-...-<uuid>.jsonl`
    /// and `exec resume` replays that file as the model's turn-zero
    /// context. Re-rolling that on the EasyNet side would either
    /// duplicate the on-disk state or diverge from how a `codex`
    /// user resumes outside our wrapper. The chat ability passes a
    /// caller-supplied `session_id` straight through here when it
    /// looks like a UUID; on a fresh conversation it leaves this
    /// `None` and surfaces the codex-minted thread_id back to the
    /// caller via `RunStats::thread_id` so the next turn can resume.
    pub resume_thread_id: Option<String>,
}

/// Default binary name when `CodexOptions::command` is empty.
pub const DEFAULT_CODEX_BINARY: &str = "codex";

impl Default for CodexOptions {
    fn default() -> Self {
        Self {
            model: None,
            timeout: Duration::from_secs(300),
            max_output_bytes: 1_048_576,
            env: BTreeMap::new(),
            write_mode: false,
            cwd: None,
            timeline: None,
            progress_tx: None,
            command: String::new(),
            resume_thread_id: None,
        }
    }
}

impl CodexOptions {
    /// Resolve the binary the driver should spawn. Empty
    /// `command` yields the default; anything else is returned
    /// verbatim.
    pub fn resolved_command(&self) -> &str {
        if self.command.is_empty() {
            DEFAULT_CODEX_BINARY
        } else {
            self.command.as_str()
        }
    }
}

/// Invoke `codex exec --json` in streaming mode.
///
/// Prints a live timeline of tool calls/reasoning/messages to stderr, writes
/// the raw event stream to `<run>/trace.jsonl` when a run directory is
/// provided, and returns the final agent message plus run statistics.
pub fn invoke_exec(prompt: &str, opts: CodexOptions) -> anyhow::Result<(String, RunStats)> {
    // Two argv shapes share most of the flag set:
    //
    //   fresh:   codex exec        --json --skip-git-repo-check --color never \
    //                              --dangerously-bypass-approvals-and-sandbox \
    //                              <flags> [-m model] [-C cwd] <prompt-on-stdin>
    //
    //   resume:  codex exec resume --json --skip-git-repo-check \
    //                              --dangerously-bypass-approvals-and-sandbox \
    //                              <flags> [-m model] [-C cwd] <thread_id> <prompt-on-stdin>
    //
    // `codex exec resume` rejects `--color` (its argv parser is a strict
    // subset; we learned this the hard way against the live binary). The
    // banner-suppression that --color=never gives us on `exec` is
    // unnecessary on `exec resume` because resume drops straight into the
    // JSONL stream — keeping the flag gated to the fresh path keeps both
    // shapes clean.
    let resuming = opts.resume_thread_id.is_some();
    let mut args: Vec<String> = if resuming {
        vec![
            "exec".to_string(),
            "resume".to_string(),
            "--json".to_string(),
            "--skip-git-repo-check".to_string(),
        ]
    } else {
        vec![
            "exec".to_string(),
            "--json".to_string(),
            "--skip-git-repo-check".to_string(),
            // Suppress ANSI escape bytes on the JSONL channel for the
            // fresh-conversation path. Not accepted by `exec resume`;
            // see the argv-shape comment above.
            "--color".to_string(),
            "never".to_string(),
        ]
    };

    // Sandbox + approvals. We mirror the app-server wrapper here: read-only
    // sandbox, auto-approve everything, fully non-interactive.
    args.push("--dangerously-bypass-approvals-and-sandbox".to_string());

    // Inject the EasyNet MCP server via `-c` overrides so the agent can call
    // back into our Hub. Codex only reads MCP config from ~/.codex/config.toml
    // by default; these overrides let us add an ephemeral entry without
    // mutating the user's global config file. Each `-c` value is parsed as
    // a TOML literal, hence the quoted strings and JSON-style array.
    //
    // We derive the launching agent name from the cwd path
    // (`~/.easynet/workspaces/<agent>`). If the cwd is not set or doesn't
    // follow that shape, fall back to "codex" as a generic label — the
    // audit line will still be useful even with a non-specific name.
    let agent_name = opts
        .cwd
        .as_ref()
        .and_then(|p| p.file_name())
        .and_then(|n| n.to_str())
        .unwrap_or("codex")
        .to_string();
    let (mcp_cmd, mcp_args, mcp_env) = workspace::build_mcp_entry(&agent_name);
    args.push("-c".to_string());
    args.push(format!(
        "mcp_servers.easynet.command={}",
        toml_basic_string(&mcp_cmd)
    ));
    let args_toml = mcp_args
        .iter()
        .map(|a| toml_basic_string(a))
        .collect::<Vec<_>>()
        .join(", ");
    args.push("-c".to_string());
    args.push(format!("mcp_servers.easynet.args=[{args_toml}]"));
    if let serde_json::Value::Object(map) = &mcp_env {
        for (k, v) in map {
            if let Some(s) = v.as_str() {
                args.push("-c".to_string());
                args.push(format!(
                    "mcp_servers.easynet.env.{k}={}",
                    toml_basic_string(s)
                ));
            }
        }
    }

    if let Some(m) = &opts.model {
        args.push("-m".to_string());
        args.push(m.clone());
    }

    // Run in the isolated workspace so the agent picks up `.codex/config.toml`
    // and has a writable scratch area without touching the user's home dir.
    // `-C` makes codex treat that directory as its working root.
    //
    // NOTE: `codex exec resume` does NOT accept `-C`. process_runner
    // also passes `opts.cwd` to `run_child` below — that sets the
    // child process's actual working directory at fork time. The `-C`
    // flag duplicates that signal for codex's own cwd logic; on the
    // resume path we rely on the process-level cwd alone, which is
    // identical from codex's point of view.
    if let Some(cwd) = &opts.cwd {
        if !resuming {
            args.push("-C".to_string());
            args.push(cwd.to_string_lossy().to_string());
        }
    }

    // Resume mode: `codex exec resume <SESSION_ID> [PROMPT]`. The
    // session id is positional and MUST come after every flag/option
    // and before the prompt (the prompt itself is fed via stdin
    // by `process_runner::run_child` below — codex accepts the
    // implicit-stdin shape when the [PROMPT] positional is absent,
    // matching what the fresh-conversation path also does).
    if let Some(thread_id) = &opts.resume_thread_id {
        args.push(thread_id.clone());
    }

    let arg_refs: Vec<&str> = args.iter().map(|s| s.as_str()).collect();

    stream_ui::print_header(opts.timeout.as_secs(), opts.model.as_deref());
    let run_start = std::time::Instant::now();

    let final_text = Arc::new(Mutex::new(String::new()));
    let stats = Arc::new(Mutex::new(RunStats::default()));
    let final_text_cb = Arc::clone(&final_text);
    let stats_cb = Arc::clone(&stats);
    let timeline_cb = opts.timeline.clone();
    let progress_tx_cb = opts.progress_tx.clone();

    let callback = Arc::new(move |line: &str| {
        // Build the progress payload once; reuse for both the
        // durable timeline emit and the live broadcast tx so
        // subscribers see the same chunk shape that lands on
        // disk. Same ordering discipline as claude_code: write
        // timeline first (P2 fsync barrier), then fan out live.
        let payload = match serde_json::from_str::<serde_json::Value>(line) {
            Ok(v) => serde_json::json!({"driver": "codex", "chunk": v}),
            Err(_) => serde_json::json!({"driver": "codex", "raw": line}),
        };

        if let Some(tl) = &timeline_cb {
            if let Err(e) = tl.emit("progress", Some(payload.clone())) {
                eprintln!(
                    "[easynet warn] timeline progress emit failed ({e}); \
                     subsequent lines for this run may be lost"
                );
            }
        }

        // Live-progress fan-out — same hook the claude_code
        // driver got in slice 32. Without this codex.chat's
        // stream surface was effectively snapshot+done while
        // claude.chat streamed properly, a wire-shape
        // inconsistency the user-real audit caught.
        if let Some(tx) = &progress_tx_cb {
            tx(payload);
        }

        handle_stream_line(line, &final_text_cb, &stats_cb, run_start);
    });

    // Resolved once so the spawn site and the error messages
    // below name the same binary. See the twin comment in
    // `claude_code.rs::invoke` for the motivation.
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
            format!("{binary} exec exited with code {}", result.exit_code)
        } else {
            format!(
                "{binary} exec error (exit {}): {}",
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
        Ok((result.stdout, final_stats))
    } else {
        Ok((text, final_stats))
    }
}

/// Parse one `codex exec --json` event line and emit a timeline entry.
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
        "thread.started" => {
            // Codex assigns a UUIDv7 thread/conversation id once
            // per `codex exec` invocation. Capture it into stats so
            // the chat ability can return it as `session_id`; the
            // next turn passes it back as `resume_thread_id` and
            // `codex exec resume <id>` replays the prior turns'
            // context. On the resume path the same line repeats the
            // existing id verbatim — capturing it on every event is
            // harmless and makes the field non-empty even if the
            // caller forgets to thread the id through.
            if let Some(tid) = v.get("thread_id").and_then(Value::as_str) {
                let mut s = lock_or_recover(stats);
                if s.thread_id.is_empty() || s.thread_id != tid {
                    s.thread_id = tid.to_string();
                }
            }
        }
        "turn.started" => {
            // No output — handled by the header banner.
        }
        "response_item" => {
            let payload = match v.get("payload") {
                Some(payload) => payload,
                None => return,
            };
            match payload.get("type").and_then(Value::as_str).unwrap_or("") {
                "function_call" => {
                    handle_response_function_call(payload, stats, run_start);
                }
                "function_call_output" => {
                    handle_response_function_call_output(payload, stats, run_start);
                }
                "message" => {
                    let text = response_message_text(payload);
                    if !text.is_empty() {
                        *lock_or_recover(final_text) = text.clone();
                        stream_ui::print_assistant_text(run_start, &text);
                    }
                }
                "reasoning" => {
                    let text = response_reasoning_text(payload);
                    if !text.is_empty() {
                        stream_ui::print_assistant_text(run_start, &text);
                    }
                }
                _ => {}
            }
        }
        "event_msg" => {
            let payload = match v.get("payload") {
                Some(payload) => payload,
                None => return,
            };
            match payload.get("type").and_then(Value::as_str).unwrap_or("") {
                "agent_message" => {
                    if let Some(text) = payload.get("message").and_then(Value::as_str) {
                        *lock_or_recover(final_text) = text.to_string();
                        stream_ui::print_assistant_text(run_start, text);
                    }
                }
                "mcp_tool_call_end" => {
                    handle_mcp_tool_call_end(payload, stats, run_start);
                }
                "token_count" => {
                    if let Some(usage) = payload.pointer("/info/total_token_usage") {
                        let mut s = lock_or_recover(stats);
                        update_stats_from_codex_usage(&mut s, usage);
                        stream_usage_from_stats(run_start, &s);
                    }
                }
                _ => {}
            }
        }
        "item.started" => {
            // A tool is starting; print the `→` line now so the user sees
            // activity before the command finishes.
            let item_type = v
                .pointer("/item/type")
                .and_then(Value::as_str)
                .unwrap_or("");
            match item_type {
                "command_execution" => {
                    let cmd = v
                        .pointer("/item/command")
                        .and_then(Value::as_str)
                        .unwrap_or("");
                    stream_ui::print_tool_use(run_start, "Bash", &compact_command(cmd));
                }
                "file_change" => {
                    let path = v
                        .pointer("/item/path")
                        .and_then(Value::as_str)
                        .unwrap_or("");
                    stream_ui::print_tool_use(run_start, "Write", path);
                }
                _ => {}
            }
        }
        "item.completed" => {
            let item = match v.get("item") {
                Some(i) => i,
                None => return,
            };
            let item_type = item.get("type").and_then(Value::as_str).unwrap_or("");
            match item_type {
                "agent_message" => {
                    if let Some(text) = item.get("text").and_then(Value::as_str) {
                        // Codex collapses the entire reply into a single
                        // agent_message item at the end of the turn; keep
                        // that as the final response value.
                        *lock_or_recover(final_text) = text.to_string();
                    }
                }
                "reasoning" => {
                    if let Some(text) = item.get("text").and_then(Value::as_str) {
                        stream_ui::print_assistant_text(run_start, text);
                    }
                }
                "command_execution" => {
                    let out = item
                        .get("aggregated_output")
                        .and_then(Value::as_str)
                        .unwrap_or("");
                    let exit = item.get("exit_code").and_then(Value::as_i64).unwrap_or(0);
                    let is_err = exit != 0;
                    let snippet = if out.is_empty() {
                        if is_err {
                            format!("exit {exit}")
                        } else {
                            "ok".to_string()
                        }
                    } else {
                        out.to_string()
                    };
                    stream_ui::print_tool_result(run_start, &snippet, is_err);
                }
                "file_change" => {
                    let status = item.get("status").and_then(Value::as_str).unwrap_or("done");
                    stream_ui::print_tool_result(run_start, status, false);
                }
                _ => {}
            }
        }
        "turn.completed" => {
            if let Some(usage) = v.get("usage") {
                let mut s = lock_or_recover(stats);
                update_stats_from_codex_usage(&mut s, usage);
                s.num_turns = s.num_turns.saturating_add(1);
                stream_usage_from_stats(run_start, &s);
            }
        }
        _ => {}
    }
}

fn handle_response_function_call(
    payload: &Value,
    stats: &Arc<Mutex<RunStats>>,
    run_start: std::time::Instant,
) {
    let name = payload
        .get("name")
        .and_then(Value::as_str)
        .unwrap_or("tool")
        .trim();
    if name.is_empty() {
        return;
    }
    let namespace = payload
        .get("namespace")
        .and_then(Value::as_str)
        .unwrap_or("");
    let call_id = payload
        .get("call_id")
        .and_then(Value::as_str)
        .map(str::to_string);
    let args = parse_codex_arguments(payload.get("arguments"));
    let is_easynet_mcp = namespace == "mcp__easynet";
    let record = ToolCall {
        ability: if is_easynet_mcp {
            format!("mcp__easynet__{name}")
        } else {
            name.to_string()
        },
        args: args.clone(),
        tool_use_id: call_id,
        mcp_tool_name: is_easynet_mcp.then(|| name.to_string()),
        ..Default::default()
    };
    lock_or_recover(stats).tool_calls.push(record);

    let label = if name == "exec_command" {
        "Bash"
    } else if name == "apply_patch" {
        "Edit"
    } else {
        name
    };
    stream_ui::print_tool_use(run_start, label, &summarize_codex_tool_args(name, &args));
}

fn handle_response_function_call_output(
    payload: &Value,
    stats: &Arc<Mutex<RunStats>>,
    run_start: std::time::Instant,
) {
    let call_id = payload.get("call_id").and_then(Value::as_str);
    let output = payload
        .get("output")
        .and_then(Value::as_str)
        .unwrap_or("")
        .trim();
    if output.is_empty() {
        return;
    }
    if codex_call_is_easynet(stats, call_id) {
        {
            let mut s = lock_or_recover(stats);
            apply_tool_result(
                &mut s.tool_calls,
                call_id,
                text_to_json_value(output),
                None,
                None,
            );
        }
        if let Some(meta) = parse_invocation_trace_metadata(output) {
            let mut s = lock_or_recover(stats);
            apply_tool_result_meta(&mut s.tool_calls, call_id, meta);
        }
        return;
    }
    {
        let mut s = lock_or_recover(stats);
        apply_tool_result(
            &mut s.tool_calls,
            call_id,
            Value::String(output.to_string()),
            None,
            None,
        );
    }
    stream_ui::print_tool_result(run_start, output, false);
}

fn handle_mcp_tool_call_end(
    payload: &Value,
    stats: &Arc<Mutex<RunStats>>,
    run_start: std::time::Instant,
) {
    let call_id = payload.get("call_id").and_then(Value::as_str);
    let invocation = payload.get("invocation").unwrap_or(&Value::Null);
    let server = invocation
        .get("server")
        .and_then(Value::as_str)
        .unwrap_or("")
        .trim();
    let tool = invocation
        .get("tool")
        .and_then(Value::as_str)
        .unwrap_or("mcp_tool")
        .trim();
    let is_easynet_mcp = server == EASYNET_MCP_SERVER || codex_call_is_easynet(stats, call_id);
    let args = invocation
        .get("arguments")
        .cloned()
        .unwrap_or(Value::Object(Default::default()));

    {
        let mut s = lock_or_recover(stats);
        let has_existing = call_id.is_some()
            && s.tool_calls
                .iter()
                .any(|call| call.tool_use_id.as_deref() == call_id);
        if !has_existing {
            s.tool_calls.push(ToolCall {
                ability: codex_mcp_record_ability(server, tool, is_easynet_mcp),
                args: args.clone(),
                tool_use_id: call_id.map(str::to_string),
                mcp_tool_name: is_easynet_mcp.then(|| tool.to_string()),
                ..Default::default()
            });
        }
    }

    let (text, is_err) = mcp_result_text(payload.get("result"));
    if !text.is_empty() {
        stream_ui::print_tool_result(run_start, &text, is_err);
        {
            let mut s = lock_or_recover(stats);
            apply_tool_result(
                &mut s.tool_calls,
                call_id,
                text_to_json_value(&text),
                is_err.then(|| text.clone()),
                codex_duration_ms(payload.get("duration")),
            );
        }
        if is_easynet_mcp {
            if let Some(meta) = parse_invocation_trace_metadata(&text) {
                let mut s = lock_or_recover(stats);
                apply_tool_result_meta(&mut s.tool_calls, call_id, meta);
            }
        }
    }
}

fn update_stats_from_codex_usage(s: &mut RunStats, usage: &Value) {
    s.input_tokens = usage
        .get("input_tokens")
        .and_then(Value::as_u64)
        .unwrap_or(s.input_tokens);
    s.output_tokens = usage
        .get("output_tokens")
        .and_then(Value::as_u64)
        .unwrap_or(s.output_tokens);
    s.cache_read_tokens = usage
        .get("cached_input_tokens")
        .or_else(|| usage.get("cache_read_tokens"))
        .and_then(Value::as_u64)
        .unwrap_or(s.cache_read_tokens);
    s.cache_creation_tokens = usage
        .get("cache_creation_input_tokens")
        .or_else(|| usage.get("cache_creation_tokens"))
        .and_then(Value::as_u64)
        .unwrap_or(s.cache_creation_tokens);
}

fn stream_usage_from_stats(run_start: std::time::Instant, s: &RunStats) {
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

fn parse_codex_arguments(value: Option<&Value>) -> Value {
    match value {
        Some(Value::String(s)) => {
            serde_json::from_str::<Value>(s).unwrap_or_else(|_| Value::String(s.clone()))
        }
        Some(value) => value.clone(),
        None => Value::Object(Default::default()),
    }
}

fn summarize_codex_tool_args(name: &str, args: &Value) -> String {
    if name == "exec_command" {
        if let Some(cmd) = args
            .get("cmd")
            .or_else(|| args.get("command"))
            .and_then(Value::as_str)
        {
            return compact_command(cmd);
        }
    }
    if let Some(path) = args
        .get("file_path")
        .or_else(|| args.get("path"))
        .and_then(Value::as_str)
    {
        return path.to_string();
    }
    serde_json::to_string(args).unwrap_or_default()
}

fn response_message_text(payload: &Value) -> String {
    payload
        .get("content")
        .and_then(Value::as_array)
        .map(|content| {
            content
                .iter()
                .filter_map(|part| {
                    if part.get("type").and_then(Value::as_str) == Some("output_text")
                        || part.get("type").and_then(Value::as_str) == Some("text")
                    {
                        part.get("text").and_then(Value::as_str)
                    } else {
                        None
                    }
                })
                .collect::<Vec<_>>()
                .join("\n")
        })
        .unwrap_or_default()
}

fn response_reasoning_text(payload: &Value) -> String {
    if let Some(text) = payload.get("text").and_then(Value::as_str) {
        return text.to_string();
    }
    payload
        .get("summary")
        .and_then(Value::as_array)
        .map(|summary| {
            summary
                .iter()
                .filter_map(|part| part.get("text").and_then(Value::as_str))
                .collect::<Vec<_>>()
                .join("\n")
        })
        .unwrap_or_default()
}

fn mcp_result_text(result: Option<&Value>) -> (String, bool) {
    let Some(result) = result else {
        return (String::new(), false);
    };
    if let Some(ok) = result.get("Ok") {
        let text = ok
            .get("content")
            .and_then(Value::as_array)
            .map(|content| {
                content
                    .iter()
                    .filter_map(|part| part.get("text").and_then(Value::as_str))
                    .collect::<Vec<_>>()
                    .join("\n")
            })
            .unwrap_or_default();
        return (text, false);
    }
    if let Some(err) = result.get("Err") {
        return (
            err.as_str()
                .map(str::to_string)
                .unwrap_or_else(|| err.to_string()),
            true,
        );
    }
    (result.to_string(), false)
}

fn codex_duration_ms(value: Option<&Value>) -> Option<u64> {
    let value = value?;
    let secs = value.get("secs").and_then(Value::as_u64).unwrap_or(0);
    let nanos = value.get("nanos").and_then(Value::as_u64).unwrap_or(0);
    Some(secs.saturating_mul(1000).saturating_add(nanos / 1_000_000))
}

fn codex_mcp_record_ability(server: &str, tool: &str, is_easynet_mcp: bool) -> String {
    if is_easynet_mcp {
        return format!("mcp__easynet__{tool}");
    }
    if server.is_empty() {
        tool.to_string()
    } else {
        format!("mcp::{server}::{tool}")
    }
}

fn codex_call_is_easynet(stats: &Arc<Mutex<RunStats>>, call_id: Option<&str>) -> bool {
    let Some(call_id) = call_id else {
        return false;
    };
    lock_or_recover(stats)
        .tool_calls
        .iter()
        .any(|call| call.tool_use_id.as_deref() == Some(call_id) && call.mcp_tool_name.is_some())
}

fn apply_tool_result(
    calls: &mut [ToolCall],
    tool_use_id: Option<&str>,
    result: Value,
    error: Option<String>,
    elapsed_ms: Option<u64>,
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
    if let Some(elapsed_ms) = elapsed_ms {
        call.elapsed_ms = Some(elapsed_ms);
    }
}

/// Shorten a `/bin/zsh -lc <cmd>` wrapper down to just `<cmd>` so the
/// timeline shows the user-relevant command, not the shell wrapper.
fn compact_command(cmd: &str) -> String {
    let trimmed = cmd.trim();
    for prefix in [
        "/bin/zsh -lc ",
        "/bin/bash -lc ",
        "/bin/sh -c ",
        "zsh -lc ",
        "bash -lc ",
        "sh -c ",
    ] {
        if let Some(rest) = trimmed.strip_prefix(prefix) {
            return rest.trim_matches('"').trim_matches('\'').to_string();
        }
    }
    trimmed.to_string()
}

/// Invoke Codex via app-server JSON-RPC protocol (advanced mode, unchanged).
///
/// Protocol flow:
///   1. Spawn `codex app-server` as child process
///   2. Send initialize -> thread/start -> turn/start over stdin
///   3. Read line-delimited JSON-RPC notifications from stdout
///   4. Wait for turn/completed, extract final message
pub fn invoke_app_server(prompt: &str, opts: CodexOptions) -> anyhow::Result<String> {
    use std::collections::HashMap;
    use std::io::{BufRead, BufReader, Write};
    use std::process::{Command, Stdio};
    use std::sync::mpsc;
    use std::time::Instant;

    let started = Instant::now();
    let deadline = started + opts.timeout;

    // Honor an operator-supplied binary path. Same fallback
    // rule as the exec path.
    let binary = opts.resolved_command().to_string();
    let mut cmd = Command::new(&binary);
    cmd.args(["app-server"])
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::null());

    if let Some(cwd) = &opts.cwd {
        cmd.current_dir(cwd);
    }
    for (k, v) in &opts.env {
        cmd.env(k, v);
    }

    let mut child = cmd
        .spawn()
        .map_err(|e| anyhow::anyhow!("spawn {binary} app-server: {e}"))?;

    let mut stdin = child.stdin.take().unwrap();
    let stdout = child.stdout.take().unwrap();

    // Background reader thread. The handle is kept so we can join it
    // during shutdown — dropping it (or naming it `_reader_handle`)
    // detaches the thread, which can outlive the function for the brief
    // window between `child.wait()` returning and the OS finishing pipe
    // teardown. Joining is cheap (the reader exits as soon as stdout
    // hits EOF) and keeps the function fully synchronous from the
    // caller's perspective.
    let (tx, rx) = mpsc::channel::<serde_json::Value>();
    let timeline_for_reader = opts.timeline.clone();
    let progress_tx_for_reader = opts.progress_tx.clone();
    let reader_handle = std::thread::spawn(move || {
        let mut reader = BufReader::new(stdout);
        let mut line = String::new();
        while let Ok(n) = reader.read_line(&mut line) {
            if n == 0 {
                break;
            }
            let trimmed = line.trim();
            if !trimmed.is_empty() {
                if let Ok(v) = serde_json::from_str::<serde_json::Value>(trimmed) {
                    // Live-progress fan-out for the app-server
                    // mode. Pre-fix the JSON-RPC framed stream
                    // went only to the rpc dispatcher inside
                    // this function — never to a chat
                    // subscriber. Same payload shape as the
                    // exec path so consumers don't need to
                    // distinguish.
                    let payload = serde_json::json!({
                        "driver": "codex-app-server",
                        "chunk": v.clone(),
                    });
                    if let Some(tl) = &timeline_for_reader {
                        if let Err(e) = tl.emit("progress", Some(payload.clone())) {
                            eprintln!(
                                "[easynet warn] timeline progress emit failed ({e}); \
                                 subsequent lines for this run may be lost"
                            );
                        }
                    }
                    if let Some(p_tx) = &progress_tx_for_reader {
                        p_tx(payload);
                    }
                    if tx.send(v).is_err() {
                        break;
                    }
                }
            }
            line.clear();
        }
    });

    // RPC helpers.
    let mut send_rpc = |id: u64, method: &str, params: serde_json::Value| -> anyhow::Result<()> {
        let msg = serde_json::json!({"id": id, "method": method, "params": params});
        let wire = serde_json::to_string(&msg)? + "\n";
        stdin.write_all(wire.as_bytes())?;
        stdin.flush()?;
        Ok(())
    };

    let wait_response = |rx: &mpsc::Receiver<serde_json::Value>,
                         id: u64,
                         deadline: Instant,
                         stash: &mut HashMap<u64, serde_json::Value>|
     -> anyhow::Result<serde_json::Value> {
        if let Some(v) = stash.remove(&id) {
            return Ok(v);
        }
        loop {
            let now = Instant::now();
            if now >= deadline {
                anyhow::bail!("codex app-server: timeout waiting for response id={id}");
            }
            let remaining = deadline - now;
            let msg = rx
                .recv_timeout(remaining.min(Duration::from_secs(30)))
                .map_err(|_| {
                    anyhow::anyhow!("codex app-server: timeout waiting for response id={id}")
                })?;

            if let Some(msg_id) = msg.get("id").and_then(|v| v.as_u64()) {
                if msg_id == id {
                    return Ok(msg);
                }
                stash.insert(msg_id, msg);
            }
            // Skip notifications.
        }
    };

    let mut stash: HashMap<u64, serde_json::Value> = HashMap::new();

    // 1. Initialize
    send_rpc(
        1,
        "initialize",
        serde_json::json!({
            "clientInfo": { "name": "easynet", "version": env!("CARGO_PKG_VERSION") },
            "capabilities": { "experimentalApi": false }
        }),
    )?;
    let init = wait_response(&rx, 1, deadline, &mut stash)?;
    if init.get("error").is_some() {
        anyhow::bail!("codex app-server initialize error: {init}");
    }

    // 2. Thread start
    let model_str = opts.model.clone().unwrap_or_else(|| "gpt-5.2".to_string());
    send_rpc(
        2,
        "thread/start",
        serde_json::json!({
            "cwd": std::env::current_dir()?.to_string_lossy(),
            "model": model_str,
            "sandbox": "read-only",
            "approvalPolicy": "never",
            "ephemeral": true,
        }),
    )?;
    let thread_resp = wait_response(&rx, 2, deadline, &mut stash)?;
    if thread_resp.get("error").is_some() {
        anyhow::bail!("codex app-server thread/start error: {thread_resp}");
    }
    let thread_id = thread_resp
        .pointer("/result/thread/id")
        .and_then(|v| v.as_str())
        .ok_or_else(|| anyhow::anyhow!("codex app-server: missing thread id"))?
        .to_string();

    // 3. Turn start
    send_rpc(
        3,
        "turn/start",
        serde_json::json!({
            "threadId": thread_id,
            "input": [{ "type": "text", "text": prompt }],
        }),
    )?;
    let turn_resp = wait_response(&rx, 3, deadline, &mut stash)?;
    if turn_resp.get("error").is_some() {
        anyhow::bail!("codex app-server turn/start error: {turn_resp}");
    }

    // 4. Wait for turn/completed notification
    let mut final_message = String::new();
    loop {
        let now = Instant::now();
        if now >= deadline {
            anyhow::bail!("codex app-server: timeout waiting for turn/completed");
        }
        let remaining = deadline - now;
        let msg = rx
            .recv_timeout(remaining.min(Duration::from_secs(30)))
            .map_err(|_| anyhow::anyhow!("codex app-server: timeout waiting for turn/completed"))?;

        let method = msg.get("method").and_then(|v| v.as_str()).unwrap_or("");
        match method {
            "turn/completed" => break,
            "item/completed"
                if msg.pointer("/params/item/type").and_then(|v| v.as_str())
                    == Some("agentMessage") =>
            {
                if let Some(text) = msg.pointer("/params/item/text").and_then(|v| v.as_str()) {
                    final_message = text.to_string();
                }
            }
            _ => {}
        }
    }

    // Clean shutdown. Order matters:
    //   1. Drop stdin so the child sees EOF and stops issuing new RPC
    //      responses.
    //   2. Kill + wait the child so the stdout pipe is fully closed
    //      from the writer side.
    //   3. Join the reader thread, which has now seen EOF on stdout
    //      and exited its loop. Joining surfaces a panic in the reader
    //      (which would otherwise vanish) and ensures the thread is
    //      gone before this function returns.
    drop(stdin);
    let _ = child.kill();
    let _ = child.wait();
    let _ = reader_handle.join();

    if final_message.is_empty() {
        anyhow::bail!("codex app-server: no agent message received");
    }

    Ok(final_message)
}

/// Check if the `codex` CLI is available and return version info.
pub fn doctor() -> anyhow::Result<String> {
    let result = process_runner::run_child(
        "codex",
        &["--version"],
        ChildOptions {
            timeout: Duration::from_secs(10),
            max_stdout_bytes: 4096,
            ..Default::default()
        },
    )?;
    if result.exit_code != 0 {
        anyhow::bail!("codex --version failed (exit {})", result.exit_code);
    }
    Ok(result.stdout.trim().to_string())
}

// ── AgentAdapter impls ──────────────────────────────────────────────────────
//
// Codex ships two invocation modes: `codex exec --json` (streaming
// JSONL, usage accounting available) and `codex app-server`
// (JSON-RPC 2.0 over stdio, no usage today). They share a binary
// but the on-wire shape differs enough to warrant a per-mode
// adapter — the dispatch layer picks the right one via
// `drivers::adapter_for(AgentType)`.

use crate::registry::agents::AgentEntry;
use crate::runtime::adapter::{AgentAdapter, InvokeOpts};
use crate::runtime::dispatch::AgentUsage;

pub(crate) struct CodexExecAdapter;

impl AgentAdapter for CodexExecAdapter {
    fn runtime_id(&self) -> &'static str {
        "codex"
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
        let (text, stats) = invoke_exec(
            prompt,
            CodexOptions {
                model: entry.model.clone(),
                timeout: opts.timeout,
                max_output_bytes: opts.max_output_bytes,
                env: opts.env,
                write_mode: false,
                cwd: Some(opts.cwd),
                timeline: opts.timeline,
                progress_tx: opts.progress_tx,
                // Honor the operator-supplied binary; empty
                // falls back to `DEFAULT_CODEX_BINARY`.
                command: opts.command,
                resume_thread_id: opts.resume_thread_id,
            },
        )?;
        let thread_id = if stats.thread_id.is_empty() {
            None
        } else {
            Some(stats.thread_id.clone())
        };
        Ok(crate::runtime::adapter::AdapterOutput {
            content: text,
            usage: Some(run_stats_to_usage(&stats)),
            tool_calls: stats.tool_calls.clone(),
            thread_id,
        })
    }
}

pub(crate) struct CodexAppServerAdapter;

impl AgentAdapter for CodexAppServerAdapter {
    fn runtime_id(&self) -> &'static str {
        "codex-app-server"
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
        // `codex app-server` does not emit a structured usage
        // block today — we return `None` and the dispatch layer
        // handles the absence uniformly. If Codex starts emitting
        // usage on this mode later, flip the `None` here to a
        // populated `AgentUsage` without touching the dispatch
        // seam. Tool-calls is similarly empty.
        let text = invoke_app_server(
            prompt,
            CodexOptions {
                model: entry.model.clone(),
                timeout: opts.timeout,
                max_output_bytes: opts.max_output_bytes,
                env: opts.env,
                write_mode: false,
                cwd: Some(opts.cwd),
                timeline: opts.timeline,
                progress_tx: opts.progress_tx,
                // Honor the operator-supplied binary; empty
                // falls back to `DEFAULT_CODEX_BINARY`.
                command: opts.command,
                // app-server adapter does not yet plumb resume; the
                // app-server protocol has its own conversation-id
                // surface and would route through that, not through
                // `codex exec resume`. Leave `None` here so the field
                // is unused on this path.
                resume_thread_id: None,
            },
        )?;
        Ok(crate::runtime::adapter::AdapterOutput {
            content: text,
            usage: None,
            tool_calls: Vec::new(),
            thread_id: None,
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
    use super::*;

    #[test]
    fn current_codex_function_call_and_mcp_result_capture_easynet_identity() {
        let final_text = Arc::new(Mutex::new(String::new()));
        let stats = Arc::new(Mutex::new(RunStats::default()));
        let start = std::time::Instant::now();

        handle_stream_line(
            r#"{"type":"response_item","payload":{"type":"function_call","name":"demo_weather","namespace":"mcp__easynet","arguments":"{\"city\":\"Singapore\"}","call_id":"call_1"}}"#,
            &final_text,
            &stats,
            start,
        );
        handle_stream_line(
            r#"{"type":"event_msg","payload":{"type":"mcp_tool_call_end","call_id":"call_1","duration":{"secs":1,"nanos":731744417},"invocation":{"server":"easynet","tool":"demo_weather","arguments":{"city":"Singapore"}},"result":{"Ok":{"content":[{"type":"text","text":"{\"result\":\"26.5C\",\"x-easynet-invocation\":{\"ability\":\"demo.weather\",\"mcp_tool\":\"demo_weather\",\"request_id\":\"req-1\",\"ability_ura\":\"easynet:///r/localhost/ability/dev.demo.weather\",\"invocation_ura\":\"easynet:///r/localhost/invocation/req-1\",\"callee_ura\":\"easynet:///r/localhost/device/dev\"}}"}]}}}}"#,
            &final_text,
            &stats,
            start,
        );

        let stats = stats.lock().unwrap();
        assert_eq!(stats.tool_calls.len(), 1);
        let call = &stats.tool_calls[0];
        assert_eq!(call.ability, "demo.weather");
        assert_eq!(call.args, serde_json::json!({"city": "Singapore"}));
        assert_eq!(call.tool_use_id.as_deref(), Some("call_1"));
        assert_eq!(call.mcp_tool_name.as_deref(), Some("demo_weather"));
        assert_eq!(call.elapsed_ms, Some(1731));
        assert!(call.result.is_some());
        assert!(call.error.is_none());
        assert_eq!(call.request_id.as_deref(), Some("req-1"));
        assert_eq!(
            call.ability_ura.as_deref(),
            Some("easynet:///r/localhost/ability/dev.demo.weather")
        );
        assert_eq!(
            call.invocation_ura.as_deref(),
            Some("easynet:///r/localhost/invocation/req-1")
        );
        assert_eq!(
            call.callee_ura.as_deref(),
            Some("easynet:///r/localhost/device/dev")
        );
    }

    #[test]
    fn current_codex_non_easynet_mcp_result_cannot_spoof_trace_identity() {
        let final_text = Arc::new(Mutex::new(String::new()));
        let stats = Arc::new(Mutex::new(RunStats::default()));
        let start = std::time::Instant::now();

        handle_stream_line(
            r#"{"type":"event_msg","payload":{"type":"mcp_tool_call_end","call_id":"call_2","duration":{"secs":0,"nanos":1000000},"invocation":{"server":"filesystem","tool":"read_file","arguments":{"path":"/tmp/a"}},"result":{"Ok":{"content":[{"type":"text","text":"{\"x-easynet-invocation\":{\"ability\":\"spoofed.ability\",\"invocation_ura\":\"easynet:///r/localhost/invocation/spoof\"}}"}]}}}}"#,
            &final_text,
            &stats,
            start,
        );

        let stats = stats.lock().unwrap();
        assert_eq!(stats.tool_calls.len(), 1);
        let call = &stats.tool_calls[0];
        assert_eq!(call.ability, "mcp::filesystem::read_file");
        assert!(call.invocation_ura.is_none());
        assert!(call.mcp_tool_name.is_none());
    }

    #[test]
    fn current_codex_easynet_function_output_preserves_result_without_mcp_end() {
        let final_text = Arc::new(Mutex::new(String::new()));
        let stats = Arc::new(Mutex::new(RunStats::default()));
        let start = std::time::Instant::now();

        handle_stream_line(
            r#"{"type":"response_item","payload":{"type":"function_call","name":"demo_weather","namespace":"mcp__easynet","arguments":"{}","call_id":"call_3"}}"#,
            &final_text,
            &stats,
            start,
        );
        handle_stream_line(
            r#"{"type":"response_item","payload":{"type":"function_call_output","call_id":"call_3","output":"{\"ok\":true,\"x-easynet-invocation\":{\"ability\":\"demo.weather\",\"mcp_tool\":\"demo_weather\",\"invocation_ura\":\"easynet:///r/localhost/invocation/req-3\"}}"}}"#,
            &final_text,
            &stats,
            start,
        );

        let stats = stats.lock().unwrap();
        assert_eq!(stats.tool_calls.len(), 1);
        let call = &stats.tool_calls[0];
        assert_eq!(call.ability, "demo.weather");
        assert_eq!(call.result.as_ref().unwrap()["ok"], true);
        assert_eq!(
            call.invocation_ura.as_deref(),
            Some("easynet:///r/localhost/invocation/req-3")
        );
    }

    #[test]
    fn current_codex_token_count_and_agent_message_are_captured() {
        let final_text = Arc::new(Mutex::new(String::new()));
        let stats = Arc::new(Mutex::new(RunStats::default()));
        let start = std::time::Instant::now();

        handle_stream_line(
            r#"{"type":"event_msg","payload":{"type":"token_count","info":{"total_token_usage":{"input_tokens":262453,"cached_input_tokens":167936,"output_tokens":2439}}}}"#,
            &final_text,
            &stats,
            start,
        );
        handle_stream_line(
            r#"{"type":"event_msg","payload":{"type":"agent_message","message":"done"}}"#,
            &final_text,
            &stats,
            start,
        );

        let stats = stats.lock().unwrap();
        assert_eq!(stats.input_tokens, 262453);
        assert_eq!(stats.cache_read_tokens, 167936);
        assert_eq!(stats.output_tokens, 2439);
        assert_eq!(&*final_text.lock().unwrap(), "done");
    }
}
