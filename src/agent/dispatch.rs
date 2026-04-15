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

use crate::registry::agents::{AgentEntry, AgentType};

use super::claude_code::{self, ClaudeOptions};
use super::codex::{self, CodexOptions};
use super::context::{self, DispatchContext};
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
/// Production entry point — reads the active `DispatchContext` via the
/// thread-local channel in `agent::context` (which transparently falls
/// back to the env vars for subprocess children that inherited only the
/// env state from their parent). Tests should use
/// `send_to_agent_with_depth(.., Some(depth))` to inject a depth without
/// touching either channel.
///
/// - Routes to the appropriate agent wrapper based on `entry.agent_type`.
/// - Propagates a *child* `DispatchContext` into the spawned agent's
///   environment so the next link in the chain inherits the mission id
///   and incremented depth.
/// - Creates a per-run directory under the agent workspace and writes
///   prompt / response / trace / meta files.
pub fn send_to_agent(
    agent_name: &str,
    entry: &AgentEntry,
    prompt: &str,
    context: Option<&str>,
    extra_trace_path: Option<&Path>,
) -> anyhow::Result<AgentResponse> {
    send_to_agent_with_depth(agent_name, entry, prompt, context, extra_trace_path, None)
}

/// Same as `send_to_agent` but accepts an explicit `depth_override`. When
/// `depth_override` is `Some(d)`, that value is used as the current
/// recursion depth instead of consulting the typed dispatch context. This
/// exists so the dispatch tests can exercise the depth guard without
/// installing a full mission context — see the `recursion_guard_*` tests
/// at the bottom of this file.
///
/// Mission context invariant
/// -------------------------
/// Every cross-agent dispatch in EasyNet is required to originate from
/// a mission runtime context (ontology §6.2 derivation 3, "there is no
/// second path"). This function enforces that invariant in a 2-stage
/// check at the top:
///
///   Stage 1 (presence): a `DispatchContext` must be active for this
///   thread (installed via `mission_runs::run_inproc`'s guard, or
///   inherited from a parent process via the env-var fallback).
///   Stage 2 (anti-forgery): the context's `mission_id` must correspond
///   to an existing mission run dir on disk under
///   `~/.easynet/missions/runs/`. This catches the trivial-forgery case
///   ("user types `EASYNET_MISSION_ID=fake`") without claiming to be a
///   cryptographic guarantee.
///
/// Both checks are skipped when `depth_override` is `Some(_)`. The
/// override is the test escape hatch — it explicitly turns this
/// function into a unit-testable code path that exercises the recursion
/// guard without requiring the full mission runtime stack to be present.
pub fn send_to_agent_with_depth(
    agent_name: &str,
    entry: &AgentEntry,
    prompt: &str,
    context: Option<&str>,
    extra_trace_path: Option<&Path>,
    depth_override: Option<u32>,
) -> anyhow::Result<AgentResponse> {
    // Mission context invariant — only enforced in production, skipped
    // when a test passes `depth_override` to exercise the recursion
    // guard in isolation.
    if depth_override.is_none() {
        check_mission_context_invariant()?;
    }

    // Resolve the active dispatch context. The new typed channel
    // (`agent::context`) is consulted first; the env-var fallback inside
    // `context::current()` keeps backwards compatibility with subprocess
    // children that inherit only the env vars from their parent.
    //
    // `depth_override` remains the test escape hatch — it bypasses both
    // the typed context and the env vars so the dispatch tests can
    // exercise the recursion guard without setting up a full mission
    // runtime stack.
    let active = depth_override
        .map(|d| DispatchContext {
            mission_id: "<test-override>".to_string(),
            depth: d,
            mission_run_dir: None,
            origin_agent: None,
        })
        .or_else(context::current);

    let current_depth = active.as_ref().map(|c| c.depth).unwrap_or(0);

    if current_depth >= MAX_AGENT_DEPTH {
        anyhow::bail!(
            "agent dispatch depth limit reached ({MAX_AGENT_DEPTH}). \
             Refusing to spawn nested agent to prevent infinite recursion."
        );
    }

    // Build full prompt with context.
    let full_prompt = compose_prompt(prompt, context);

    // Build env for the child subprocess. The env vars are how the typed
    // context crosses the process boundary into the spawned agent CLI —
    // see `agent::context` for the design rationale. We always emit the
    // depth (incremented by one for the child) and propagate the mission
    // id when one is active.
    let mut env = entry.env.clone();
    // The `active.is_none()` branch is reachable only in release builds
    // when `check_mission_context_invariant` observed a missing mission
    // context and chose to log rather than fail (a backcompat shim for
    // legacy callers — see that function's rustdoc). In that degraded
    // mode we still propagate the depth so the child's recursion guard
    // works, but we have no mission id or origin to emit. The typed
    // `DispatchContext` is deliberately not constructed with a synthetic
    // mission_id here: silently fabricating one would make the audit
    // trail lie about which mission a run belonged to.
    if let Some(parent) = active.as_ref() {
        parent.child(agent_name).serialize_to_env(&mut env);
    } else {
        env.insert(
            "EASYNET_AGENT_DEPTH".to_string(),
            current_depth.saturating_add(1).to_string(),
        );
    }

    let timeout = Duration::from_secs(entry.timeout_secs);
    let max_output = entry.max_output_bytes;
    let start = Instant::now();

    // Provision workspace with .claude/ or .codex/ config + CLAUDE.md/AGENTS.md.
    // We log the failure rather than swallow silently: a missing workspace
    // means the agent runs without project-level MCP discovery and without
    // CLAUDE.md / AGENTS.md context — the user's results will be silently
    // worse, and they should see *why* on stderr instead of having to guess.
    let workspace = match workspace::ensure_workspace(agent_name, entry) {
        Ok(p) => Some(p),
        Err(e) => {
            eprintln!(
                "[easynet warn] agent {agent_name}: workspace provisioning failed ({e}); \
                 continuing without project-level MCP / context"
            );
            None
        }
    };
    let cwd = workspace.clone();

    // Create a per-run directory. If creation fails (e.g. workspace dir is
    // unwritable), skip persistence — the agent call still runs, but we
    // surface the reason so the operator knows the run is unrecorded.
    let run_dir: Option<Arc<RunDir>> = match RunDir::create(agent_name) {
        Ok(dir) => Some(Arc::new(dir)),
        Err(e) => {
            eprintln!(
                "[easynet warn] agent {agent_name}: run dir creation failed ({e}); \
                 continuing without per-run persistence"
            );
            None
        }
    };
    if let Some(dir) = &run_dir {
        if let Err(e) = dir.write_prompt(&full_prompt) {
            eprintln!(
                "[easynet warn] run {}: write prompt.txt failed ({e})",
                dir.path().display()
            );
        }
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
        AgentType::ClaudeCode => claude_code::invoke(
            &full_prompt,
            ClaudeOptions {
                model: entry.model.clone(),
                timeout,
                max_output_bytes: max_output,
                env,
                cwd,
                run_dir: run_dir.clone(),
            },
        )
        .map(|(text, stats)| {
            (
                text,
                Some(AgentUsage {
                    input_tokens: stats.input_tokens,
                    output_tokens: stats.output_tokens,
                    cache_read_tokens: stats.cache_read_tokens,
                    cache_creation_tokens: stats.cache_creation_tokens,
                    num_turns: stats.num_turns,
                    total_cost_usd: stats.total_cost_usd,
                }),
            )
        }),
        AgentType::Codex => codex::invoke_exec(
            &full_prompt,
            CodexOptions {
                model: entry.model.clone(),
                timeout,
                max_output_bytes: max_output,
                env,
                write_mode: false,
                cwd: workspace,
                run_dir: run_dir.clone(),
            },
        )
        .map(|(text, stats)| {
            (
                text,
                Some(AgentUsage {
                    input_tokens: stats.input_tokens,
                    output_tokens: stats.output_tokens,
                    cache_read_tokens: stats.cache_read_tokens,
                    cache_creation_tokens: stats.cache_creation_tokens,
                    num_turns: stats.num_turns,
                    total_cost_usd: stats.total_cost_usd,
                }),
            )
        }),
        AgentType::CodexAppServer => codex::invoke_app_server(
            &full_prompt,
            CodexOptions {
                model: entry.model.clone(),
                timeout,
                max_output_bytes: max_output,
                env,
                write_mode: false,
                cwd: workspace,
                run_dir: run_dir.clone(),
            },
        )
        .map(|text| (text, None)),
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
            if let Err(e) = dir.write_response(text) {
                eprintln!(
                    "[easynet warn] run {}: write response.md failed ({e})",
                    dir.path().display()
                );
            }
        }
        let u = usage_for_meta.unwrap_or_default();
        let meta = RunMeta {
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
        };
        if let Err(e) = dir.write_meta(&meta) {
            eprintln!(
                "[easynet warn] run {}: write meta.json failed ({e})",
                dir.path().display()
            );
        }
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

/// Delimiters for injected context. HTML comments survive verbatim in
/// markdown and plain text, and the `easynet:context` tag is a unique
/// string no user content realistically collides with.
///
/// We pick HTML comments because:
/// - They render invisibly in markdown viewers used by downstream tools
///   (Claude Code's transcript panel, codex-exec logs) — the user sees
///   a clean "Context" heading, the model sees the delimiters.
/// - They are not interpreted by any shell or argv parser, so the
///   boundary cannot be mangled when the prompt crosses process lines.
/// - A literal `## Context` heading in the caller-supplied context can
///   no longer be mistaken for the boundary marker; the model can
///   parse on these tokens reliably.
const CONTEXT_OPEN: &str = "<!-- easynet:context-start -->";
const CONTEXT_CLOSE: &str = "<!-- easynet:context-end -->";

fn compose_prompt(prompt: &str, context: Option<&str>) -> String {
    match context.map(str::trim).filter(|s| !s.is_empty()) {
        Some(ctx) => format!(
            "{prompt}\n\n{CONTEXT_OPEN}\n## Context (previous discussion)\n\n{ctx}\n{CONTEXT_CLOSE}\n"
        ),
        None => prompt.to_string(),
    }
}

#[cfg(test)]
mod compose_prompt_tests {
    use super::*;

    #[test]
    fn absent_context_returns_prompt_unchanged() {
        assert_eq!(compose_prompt("hi", None), "hi");
        // An empty-after-trim context is treated as absent so we never
        // emit a dangling section header.
        assert_eq!(compose_prompt("hi", Some("   \n\t ")), "hi");
    }

    #[test]
    fn present_context_is_delimited() {
        let out = compose_prompt("Do X.", Some("earlier: A said B"));
        assert!(out.contains(CONTEXT_OPEN), "open sentinel must be present");
        assert!(out.contains(CONTEXT_CLOSE), "close sentinel must be present");
        // Open must precede close in byte order.
        let open_at = out.find(CONTEXT_OPEN).unwrap();
        let close_at = out.find(CONTEXT_CLOSE).unwrap();
        assert!(open_at < close_at);
    }

    #[test]
    fn context_containing_section_header_survives_boundary() {
        // The historical bug: caller-supplied context that itself
        // starts with `## Context` was indistinguishable from the
        // injected header. With sentinels the downstream parser can
        // locate the true boundary regardless of content.
        let hostile = "## Context\nuser-supplied section\n\n## Context\nsecond";
        let out = compose_prompt("Do X.", Some(hostile));
        assert!(out.contains(CONTEXT_OPEN));
        assert!(out.contains(CONTEXT_CLOSE));
        // The hostile payload appears verbatim between the sentinels.
        let open_at = out.find(CONTEXT_OPEN).unwrap();
        let close_at = out.find(CONTEXT_CLOSE).unwrap();
        assert!(out[open_at..close_at].contains(hostile));
    }
}

/// Two-stage mission context check. See `send_to_agent_with_depth`'s
/// rustdoc for the load-bearing reasoning.
///
/// Stage 1 — presence: a `DispatchContext` must be active for this
/// thread, either installed via `with_context` (the typed in-process
/// channel) or recovered from the env-var fallback (the cross-process
/// channel for spawned subprocesses).
/// Stage 2 — anti-forgery: the context's mission id must correspond to
/// an existing mission run directory under `~/.easynet/missions/runs/`.
///
/// In **debug** builds the function panics on failure, making the
/// invariant impossible to silently violate during development. In
/// **release** builds it logs a warning and (for stage 2) returns an
/// error so the dispatch fails loudly without taking the process down.
fn check_mission_context_invariant() -> anyhow::Result<()> {
    let mission_id = match context::current() {
        Some(ctx) if !ctx.mission_id.is_empty() => ctx.mission_id,
        _ => {
            // Stage 1 failure: no context active and env-var fallback
            // also empty.
            #[cfg(debug_assertions)]
            panic!(
                "dispatch::send_to_agent called without a mission context. \
                 All agent dispatches must originate from a mission runtime. \
                 See docs/easynet_ontology.tex §6.2."
            );
            #[cfg(not(debug_assertions))]
            {
                eprintln!(
                    "[easynet warn] dispatch::send_to_agent called without \
                     mission context — this is an ontology violation, see \
                     docs/easynet_ontology.tex §6.2"
                );
                // Continue execution in release mode for backwards compat
                // with any caller that hasn't been migrated yet.
                return Ok(());
            }
        }
    };

    // Stage 2: anti-forgery. The mission ID must be the directory name
    // of a real mission run dir under ~/.easynet/missions/runs/. If not,
    // either the env var was forged ("EASYNET_MISSION_ID=fake easynet
    // ...") or the mission has already been cleaned up. Both cases are
    // pathological — refuse to dispatch.
    //
    // This check is local-fs only and cheap (one stat). It is not a
    // cryptographic guarantee — a determined attacker can `mkdir` a
    // fake dir — but it eliminates the trivial-forgery case and
    // catches the common bug pattern of "user set the env var by
    // mistake".
    let mission_run_dir = crate::cli::mission_runs::root_dir().join(&mission_id);
    if !mission_run_dir.exists() {
        #[cfg(debug_assertions)]
        panic!(
            "mission_id={} does not correspond to an existing \
             mission run dir at {}. Either the env var was forged or \
             the run dir has been cleaned up mid-execution. Refusing \
             to dispatch.",
            mission_id,
            mission_run_dir.display()
        );
        #[cfg(not(debug_assertions))]
        {
            eprintln!(
                "[easynet warn] mission_id={} does not match \
                 an existing mission run dir; possible env var forgery",
                mission_id
            );
            anyhow::bail!("invalid mission context: run dir not found");
        }
    }

    Ok(())
}

// ── Tests ──────────────────────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;
    use crate::registry::agents::AgentEntry;

    /// Construct a dummy `AgentEntry` for tests that exercise the
    /// dispatch guard logic in isolation. The command path is
    /// intentionally bogus — these tests must fail before reaching
    /// `process::Command::spawn`, so the binary path doesn't matter.
    fn dummy_entry() -> AgentEntry {
        AgentEntry::new(AgentType::ClaudeCode, None)
    }

    /// Recursion guard: depth_override=Some(2) must trip the limit
    /// before any subprocess is spawned. The error message must
    /// mention "depth limit reached" so operators can grep for it.
    #[test]
    fn recursion_guard_blocks_at_depth_2() {
        let entry = dummy_entry();
        let res =
            send_to_agent_with_depth("claude", &entry, "any prompt", None, None, Some(2));
        let err = res.expect_err("depth=2 must error");
        let msg = format!("{err}");
        assert!(
            msg.contains("depth limit reached"),
            "expected 'depth limit reached' in error, got: {msg}"
        );
    }

    /// Recursion guard at depth=1 must not fire. The function will
    /// still error eventually (no real claude binary in the test env)
    /// but the error must NOT be the depth-limit error — it should be
    /// a downstream failure (workspace creation, command exec, etc.).
    /// This test proves the guard isn't over-triggering.
    #[test]
    fn recursion_guard_allows_depth_1() {
        let entry = dummy_entry();
        // Use a HomeGuard so workspace creation lands in a temp dir
        // and doesn't pollute the developer's real ~/.easynet/.
        let _g = crate::cli::test_support::HomeGuard::new();
        let res =
            send_to_agent_with_depth("claude", &entry, "any prompt", None, None, Some(1));
        // We expect an error (no real claude binary), but it must
        // NOT be the depth-limit error. Anything else is acceptable.
        match res {
            Ok(_) => panic!("expected an error from missing claude binary"),
            Err(e) => {
                let msg = format!("{e}");
                assert!(
                    !msg.contains("depth limit"),
                    "depth=1 must not trigger depth-limit error, got: {msg}"
                );
            }
        }
    }

    /// `send_to_agent_with_depth` with a real `depth_override` must
    /// also bypass the mission-context invariant. This is the test
    /// escape hatch — without it, the unit tests above would have to
    /// set up a real mission run dir, which defeats the purpose of
    /// testing the dispatch path in isolation.
    #[test]
    fn depth_override_bypasses_mission_context_check() {
        // Even with no EASYNET_MISSION_ID set, depth_override=Some(2)
        // should still cleanly hit the depth-limit check (not panic on
        // a missing mission context).
        std::env::remove_var("EASYNET_MISSION_ID");
        let entry = dummy_entry();
        let res =
            send_to_agent_with_depth("claude", &entry, "any prompt", None, None, Some(2));
        assert!(res.is_err());
        let msg = format!("{}", res.unwrap_err());
        assert!(msg.contains("depth limit reached"));
    }

    // The next two tests are end-to-end and require external binaries
    // (claude CLI with auth, MCP server child, etc.). They are gated
    // by `#[ignore]` so they only run under
    // `cargo test -- --ignored`. They exist to validate the full
    // production path that the unit tests above only exercise in
    // pieces.

    /// End-to-end recursion guard via the MCP server. Spawns
    /// `easynet mcp serve --enable-agent-dispatch --agent claude` as
    /// a child with `EASYNET_AGENT_DEPTH=2` pre-set, then sends a
    /// `tools/call` for `send_to_agent`. The response must contain
    /// the depth-limit error.
    ///
    /// Inline JSON-RPC over stdio — no dev-dep added. ~30 lines.
    #[test]
    #[ignore]
    fn recursion_guard_e2e() {
        use std::io::{BufRead, BufReader, Write};
        use std::process::{Command, Stdio};
        use std::time::Duration;

        // Locate the binary the test was built against. Falls back to
        // `easynet` on PATH if neither path exists, but in practice
        // `cargo test` ensures `target/debug/easynet` is fresh.
        let bin = if std::path::Path::new("./target/release/easynet").exists() {
            "./target/release/easynet"
        } else if std::path::Path::new("./target/debug/easynet").exists() {
            "./target/debug/easynet"
        } else {
            "easynet"
        };

        let mut child = Command::new(bin)
            .args([
                "mcp",
                "serve",
                "--enable-agent-dispatch",
                "--agent",
                "claude",
            ])
            .env("EASYNET_AGENT_DEPTH", "2")
            // Set a fake mission id pointing at a tmp dir we control
            // so the anti-forgery check passes.
            .env("EASYNET_MISSION_ID", "test-recursion-guard-e2e")
            .stdin(Stdio::piped())
            .stdout(Stdio::piped())
            .stderr(Stdio::null())
            .spawn()
            .expect("spawn easynet mcp serve");

        // Create the fake mission run dir so the anti-forgery check
        // doesn't fire before the depth check does.
        let _g = crate::cli::test_support::HomeGuard::new();
        let runs_root = crate::persistence::config::state_dir()
            .join("missions")
            .join("runs");
        let _ = std::fs::create_dir_all(runs_root.join("test-recursion-guard-e2e"));

        let stdin = child.stdin.as_mut().expect("child stdin");
        let init = serde_json::json!({
            "jsonrpc": "2.0",
            "id": 1,
            "method": "initialize",
            "params": {
                "protocolVersion": "2024-11-05",
                "capabilities": {},
                "clientInfo": {"name": "test", "version": "1"},
            },
        });
        writeln!(stdin, "{init}").unwrap();

        let call = serde_json::json!({
            "jsonrpc": "2.0",
            "id": 2,
            "method": "tools/call",
            "params": {
                "name": "send_to_agent",
                "arguments": {
                    "agent": "claude",
                    "prompt": "hi",
                },
            },
        });
        writeln!(stdin, "{call}").unwrap();

        // Read responses until we see the call result or timeout.
        let stdout = child.stdout.take().expect("child stdout");
        let mut reader = BufReader::new(stdout);
        let deadline = std::time::Instant::now() + Duration::from_secs(10);
        let mut found_depth_error = false;
        let mut line = String::new();
        while std::time::Instant::now() < deadline {
            line.clear();
            if reader.read_line(&mut line).unwrap_or(0) == 0 {
                break;
            }
            if line.contains("depth limit") {
                found_depth_error = true;
                break;
            }
        }
        let _ = child.kill();
        let _ = child.wait();
        assert!(
            found_depth_error,
            "expected 'depth limit' in MCP server response stream"
        );
    }

    /// End-to-end success path: `easynet agent send claude "say only
    /// OK"` desugars to a mission and produces a real reply. Requires
    /// local claude CLI + auth.
    #[test]
    #[ignore]
    fn agent_send_desugar_e2e() {
        use std::process::Command;

        let bin = if std::path::Path::new("./target/release/easynet").exists() {
            "./target/release/easynet"
        } else if std::path::Path::new("./target/debug/easynet").exists() {
            "./target/debug/easynet"
        } else {
            "easynet"
        };

        let out = Command::new(bin)
            .args(["agent", "send", "claude", "say only the word OK"])
            .output()
            .expect("run easynet agent send");

        assert!(out.status.success(), "non-zero exit: {:?}", out);
        let stdout = String::from_utf8_lossy(&out.stdout);
        assert!(
            stdout.to_uppercase().contains("OK"),
            "expected 'OK' in stdout, got: {stdout}"
        );

        // The dispatching banner must appear on stderr.
        let stderr = String::from_utf8_lossy(&out.stderr);
        assert!(
            stderr.contains("dispatching via mission runtime"),
            "expected mission-runtime banner on stderr, got: {stderr}"
        );
    }
}
