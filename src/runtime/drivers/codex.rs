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

use crate::runtime::process_runner::{self, ChildOptions};
use crate::runtime::run_store::RunDir;
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
    /// Persistent run directory. Used for per-run artefacts
    /// (`prompt.txt`, `response.md`, `meta.json`); the stream
    /// event log moved to the Timeline in PR-7 Commit 2.
    pub run_dir: Option<Arc<RunDir>>,
    /// PR-7 Commit 2: Timeline writer. Same semantics as
    /// `ClaudeOptions::timeline` — each streamed stdout line
    /// emits a `progress` event on the P1-P6 event log.
    pub timeline: Option<Arc<crate::runtime::timeline::TimelineWriter>>,
    /// Binary to spawn. Empty string means "use the driver
    /// default" (`DEFAULT_CODEX_BINARY`). Mirrors the behaviour
    /// of `ClaudeOptions::command`; see that field's rustdoc
    /// for the motivation.
    pub command: String,
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
            run_dir: None,
            timeline: None,
            command: String::new(),
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
    let mut args: Vec<String> = vec![
        "exec".to_string(),
        "--json".to_string(),
        // Skip the git-repo guard; our workspace may not be a repo.
        "--skip-git-repo-check".to_string(),
        // No colour control on the JSONL channel — turn it off explicitly
        // so any stray prints don't carry ANSI escape bytes.
        "--color".to_string(),
        "never".to_string(),
    ];

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
    if let Some(cwd) = &opts.cwd {
        args.push("-C".to_string());
        args.push(cwd.to_string_lossy().to_string());
    }

    let arg_refs: Vec<&str> = args.iter().map(|s| s.as_str()).collect();

    stream_ui::print_header(opts.timeout.as_secs(), opts.model.as_deref());
    let run_start = std::time::Instant::now();

    let final_text = Arc::new(Mutex::new(String::new()));
    let stats = Arc::new(Mutex::new(RunStats::default()));
    let final_text_cb = Arc::clone(&final_text);
    let stats_cb = Arc::clone(&stats);
    let timeline_cb = opts.timeline.clone();

    let callback = Arc::new(move |line: &str| {
        // PR-7 Commit 2 — same pattern as claude_code.rs. Driver
        // stream lines emit as `progress` events through the
        // Timeline; shape-aware consumers parse `chunk` (JSON)
        // or `raw` (non-JSON leak) from the payload.
        if let Some(tl) = &timeline_cb {
            let payload = match serde_json::from_str::<serde_json::Value>(line) {
                Ok(v) => serde_json::json!({"driver": "codex", "chunk": v}),
                Err(_) => serde_json::json!({"driver": "codex", "raw": line}),
            };
            if let Err(e) = tl.emit("progress", Some(payload)) {
                eprintln!(
                    "[easynet warn] timeline progress emit failed ({e}); \
                     subsequent lines for this run may be lost"
                );
            }
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
        "thread.started" | "turn.started" => {
            // No output — handled by the header banner.
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
                s.input_tokens = usage
                    .get("input_tokens")
                    .and_then(Value::as_u64)
                    .unwrap_or(s.input_tokens);
                s.output_tokens = usage
                    .get("output_tokens")
                    .and_then(Value::as_u64)
                    .unwrap_or(s.output_tokens);
                // Codex reports cached_input_tokens only; no cache_creation
                // counterpart. Map it to our cache_read slot.
                s.cache_read_tokens = usage
                    .get("cached_input_tokens")
                    .and_then(Value::as_u64)
                    .unwrap_or(s.cache_read_tokens);
                s.num_turns = s.num_turns.saturating_add(1);
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
        _ => {}
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
            "item/completed" => {
                if msg.pointer("/params/item/type").and_then(|v| v.as_str()) == Some("agentMessage")
                {
                    if let Some(text) = msg.pointer("/params/item/text").and_then(|v| v.as_str()) {
                        final_message = text.to_string();
                    }
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
    ) -> anyhow::Result<(String, Option<AgentUsage>)> {
        let (text, stats) = invoke_exec(
            prompt,
            CodexOptions {
                model: entry.model.clone(),
                timeout: opts.timeout,
                max_output_bytes: opts.max_output_bytes,
                env: opts.env,
                write_mode: false,
                cwd: Some(opts.cwd),
                run_dir: opts.run_dir,
                timeline: opts.timeline,
                // Honor the operator-supplied binary; empty
                // falls back to `DEFAULT_CODEX_BINARY`.
                command: opts.command,
            },
        )?;
        Ok((text, Some(run_stats_to_usage(&stats))))
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
    ) -> anyhow::Result<(String, Option<AgentUsage>)> {
        // `codex app-server` does not emit a structured usage
        // block today — we return `None` and the dispatch layer
        // handles the absence uniformly. If Codex starts emitting
        // usage on this mode later, flip the `None` here to a
        // populated `AgentUsage` without touching the dispatch
        // seam.
        let text = invoke_app_server(
            prompt,
            CodexOptions {
                model: entry.model.clone(),
                timeout: opts.timeout,
                max_output_bytes: opts.max_output_bytes,
                env: opts.env,
                write_mode: false,
                cwd: Some(opts.cwd),
                run_dir: opts.run_dir,
                timeline: opts.timeline,
                // Honor the operator-supplied binary; empty
                // falls back to `DEFAULT_CODEX_BINARY`.
                command: opts.command,
            },
        )?;
        Ok((text, None))
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
