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

use crate::registry::agents::AgentEntry;

use super::adapter::InvokeOpts;
use super::context::{self, DispatchContext};
use super::drivers::adapter_for;
use super::run_store::{RunDir, RunMeta};
use super::session::Session;
use super::{directory::AgentDirectory, workspace};

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

/// Per-call driver knob overrides. Carried alongside the prompt
/// down through `send_to_agent_with_depth` to the driver layer; lets
/// a single chat invocation override the agent's default model
/// without editing `agent.toml`. `temperature` and `max_tokens` are
/// parsed and accepted but not honored by the v1 claude-code /
/// codex CLI drivers (those CLIs do not expose either knob); they
/// are recorded here so a future driver layer that does support them
/// (or a remote API path) can pick them up without re-shaping the
/// dispatch surface.
#[derive(Debug, Clone, Default)]
pub struct DriverOverrides {
    /// Override the agent's default model (`agent.toml::model` or
    /// `entry.model`). Wins over both when `Some`.
    pub model: Option<String>,
    /// Honored by future drivers; current claude-code / codex CLIs
    /// ignore this field. A one-shot warning prints on first ignored
    /// use so an operator setting this knob in chat args knows it is
    /// not currently piped through.
    pub temperature: Option<f64>,
    /// Same caveat as `temperature`.
    pub max_tokens: Option<u32>,
    /// Driver-side conversation resume id. When `Some`, the driver
    /// continues a prior conversation under that id (codex:
    /// `codex exec resume <id>`); when `None`, the driver starts a
    /// fresh conversation and the chat ability returns the newly
    /// minted id back to the caller as `session_id`. Drivers that
    /// do not support resume (claude-code today) ignore this field
    /// and treat each call as fresh.
    ///
    /// The chat ability sets this from the caller's `session_id`
    /// argument when it parses as a UUID — that shape is the
    /// signal that the caller is asking us to continue an existing
    /// thread rather than label a fresh one.
    pub resume_thread_id: Option<String>,
}

/// One tool call the LLM made during a run. Lifted from the driver
/// layer's tool-use observability so the chat ability can surface
/// `tool_calls` in its structured response. Result fields are
/// populated when the driver exposes them as structured stream
/// events; absent fields mean the driver did not provide them.
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct ToolCall {
    pub ability: String,
    pub args: serde_json::Value,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub result: Option<serde_json::Value>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub error: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub elapsed_ms: Option<u64>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub tool_use_id: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub mcp_tool_name: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub request_id: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub ability_ura: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub invocation_ura: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub caller_ura: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub callee_ura: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub subject_ura: Option<String>,
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
    /// Tool invocations the LLM made during this run, in order.
    /// Empty when the run made no tool calls. Drivers that do not
    /// expose tool-call observability (codex today) leave this empty.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub tool_calls: Vec<ToolCall>,
    /// Driver-assigned conversation id. Set by drivers whose backing
    /// CLI/runtime persists multi-turn state under a stable id (codex
    /// emits `thread.started` with a UUIDv7; claude-code does not yet
    /// expose one and leaves this `None`). The chat ability echoes
    /// this back to the caller as `session_id`, and a subsequent turn
    /// can pass it through `driver.resume_thread_id` (or its
    /// equivalent) to continue the same conversation. `None` is the
    /// fresh-conversation path.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub thread_id: Option<String>,
}

pub struct AgentDispatchRequest<'a> {
    pub agent_name: &'a str,
    pub entry: &'a AgentEntry,
    pub prompt: &'a str,
    pub context: Option<&'a str>,
    pub extra_trace_path: Option<&'a Path>,
    pub depth_override: Option<u32>,
    pub overrides: Option<&'a DriverOverrides>,
    pub progress_tx: Option<Arc<dyn Fn(serde_json::Value) + Send + Sync>>,
}

/// Resolve the dispatch timeout from spec + entry precedence.
///
/// `spec_timeout_secs = Some(n)` — operator set a timeout in
/// `agent.toml`; use it verbatim. `None` — no operator choice
/// in the spec; fall through to the v1/legacy
/// `entry.timeout_secs` so pre-migration rows keep working.
///
/// Extracted so production and tests call the same code: see
/// the doc block at the call site in `send_to_agent_with_depth`
/// for why.
pub(crate) fn resolve_timeout(spec_timeout_secs: Option<u64>, entry_timeout_secs: u64) -> Duration {
    Duration::from_secs(spec_timeout_secs.unwrap_or(entry_timeout_secs))
}

/// Resolve the dispatch model from spec + entry precedence.
///
/// `spec_model = Some(_)` — operator named a model in
/// `agent.toml`; it wins. `spec_model = None` — operator did
/// not name one; fall through to the v1/legacy `entry.model`
/// so pre-migration rows keep dispatching to the model their
/// registry row names. If both are `None`, the result is
/// `None` (the runtime driver picks its own default).
///
/// Extracted so production and tests call the same code.
///
/// Three-tier model resolution:
/// per-call override > agent.toml spec > legacy entry.
///
/// Extracted as its own helper (rather than inlined at the call site)
/// so production and tests bind to the same code path — the chat-ability
/// override precedence is itself a contract that should not have a
/// second place to drift to.
pub(crate) fn resolve_model_with_overrides(
    override_model: Option<String>,
    spec_model: Option<String>,
    entry_model: Option<String>,
) -> Option<String> {
    override_model.or(spec_model).or(entry_model)
}

#[cfg(test)]
pub(crate) fn resolve_model(
    spec_model: Option<String>,
    entry_model: Option<String>,
) -> Option<String> {
    spec_model.or(entry_model)
}

/// Send a prompt to a registered agent on behalf of an *external* caller —
/// a peer node, a federated agent, or a direct MCP tool invocation that
/// arrived over the network rather than from inside a local mission.
///
/// This is the production entry point for remote `<agent>.chat`
/// invocations — the agent's default-input ability surfaced
/// over MCP. When a remote caller invokes that ability against
/// this node via [`AbilityToolAdapter`], the request originates
/// outside any local mission, so recursion depth starts at 0
/// and no parent-mission id is propagated to the child
/// subprocess. Functionally equivalent to
/// `send_to_agent_with_depth(.., Some(0))`, but named for the role it
/// plays so call sites do not rely on a test-hatch comment to justify
/// their use.
///
/// # Ontology
///
/// The ontology (§6.2 derivation 3, "there is no second path") requires
/// every *intra-cluster* dispatch to belong to a mission. An external
/// tool invocation is, by construction, outside any *local* mission —
/// it is the network boundary between the remote caller's audit realm
/// and ours. Fabricating a synthetic mission id for it would make the
/// local audit trail lie about provenance; starting fresh at depth 0
/// and letting the remote caller's own audit system (if any) record
/// their side of the hop is the honest shape.
///
/// The recursion guard still applies to *this* node's sub-dispatches:
/// if the externally-triggered agent itself invokes a child agent
/// (e.g. claude → codex via `send_to_agent`), the child's depth is 1,
/// and so on, bounded by `MAX_AGENT_DEPTH`. The external entry point
/// just sets the floor.
///
/// # Not for test use
///
/// The mission-context invariant check (`check_mission_context_invariant`)
/// is skipped here because `depth_override = Some(0)` bypasses it. That
/// is correct for an external invocation but would be a silent footgun
/// for a test that happened to use this function without realising;
/// tests should continue to use `send_to_agent_with_depth` directly so
/// their bypass is visible at the call site.
#[cfg(test)]
pub fn send_external(
    agent_name: &str,
    entry: &AgentEntry,
    prompt: &str,
    context: Option<&str>,
) -> anyhow::Result<AgentResponse> {
    send_to_agent_with_depth(agent_name, entry, prompt, context, None, Some(0), None)
}

/// Pin per-call driver knobs (model, temperature, max_tokens) when
/// dispatching to a registered agent. Used by the chat ability
/// handler when the caller passes a `driver` sub-object in their
/// arguments. Pass `overrides = None` for the unmodified entry-default
/// behaviour.
pub fn send_external_with_overrides(
    agent_name: &str,
    entry: &AgentEntry,
    prompt: &str,
    context: Option<&str>,
    overrides: Option<&DriverOverrides>,
) -> anyhow::Result<AgentResponse> {
    send_to_agent_with_depth(agent_name, entry, prompt, context, None, Some(0), overrides)
}

/// Same as `send_external_with_overrides` but threads a
/// per-token progress callback through to the driver. Used by
/// the chat ability's stream_handler to forward live LLM
/// progress to its broadcast channel.
pub fn send_external_with_overrides_and_progress(
    agent_name: &str,
    entry: &AgentEntry,
    prompt: &str,
    context: Option<&str>,
    overrides: Option<&DriverOverrides>,
    progress_tx: Option<Arc<dyn Fn(serde_json::Value) + Send + Sync>>,
) -> anyhow::Result<AgentResponse> {
    send_to_agent_with_depth_and_progress(AgentDispatchRequest {
        agent_name,
        entry,
        prompt,
        context,
        extra_trace_path: None,
        depth_override: Some(0),
        overrides,
        progress_tx,
    })
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
    overrides: Option<&DriverOverrides>,
) -> anyhow::Result<AgentResponse> {
    send_to_agent_with_depth_and_progress(AgentDispatchRequest {
        agent_name,
        entry,
        prompt,
        context,
        extra_trace_path,
        depth_override,
        overrides,
        progress_tx: None,
    })
}

/// Same as `send_to_agent_with_depth` but threads through an
/// optional per-token progress callback. Pre-fix the chat
/// ability's stream surface emitted only {session, loaded?,
/// done|error} — three frames per call regardless of LLM
/// response length, despite calling itself \"streaming\".
/// Threading a callback in here is what makes the stream a
/// real per-token stream: the driver invokes `progress_tx`
/// once per stdout line in stream-json mode, and chat's
/// stream_handler forwards that into its broadcast channel.
pub fn send_to_agent_with_depth_and_progress(
    request: AgentDispatchRequest<'_>,
) -> anyhow::Result<AgentResponse> {
    let AgentDispatchRequest {
        agent_name,
        entry,
        prompt,
        context,
        extra_trace_path,
        depth_override,
        overrides,
        progress_tx,
    } = request;

    // Mission context invariant — only enforced in production, skipped
    // when a test passes `depth_override` to exercise the recursion
    // guard in isolation.
    if depth_override.is_none() {
        check_mission_context_invariant()?;
    }

    // Resolve the active dispatch context. The typed channel
    // (`runtime::context`) is consulted first; the env-var reader inside
    // `context::current()` is the explicit subprocess boundary for children
    // that inherit their parent context through env vars.
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
            parent_invocation: None,
        })
        .or_else(context::current);

    let active = active.ok_or_else(|| anyhow::anyhow!("agent dispatch missing mission context"))?;
    let current_depth = active.depth;

    if current_depth >= MAX_AGENT_DEPTH {
        anyhow::bail!(
            "agent dispatch depth limit reached ({MAX_AGENT_DEPTH}). \
             Refusing to spawn nested agent to prevent infinite recursion."
        );
    }

    // Build full prompt with context.
    let full_prompt = compose_prompt(prompt, context);

    // ── Project the registered AgentDirectory ──
    //
    // The registry row must point at an on-disk AgentDirectory whose
    // `agent.toml` is the source of truth. Dispatch no longer
    // reconstructs specs from fat `AgentEntry` fields. A bad root is a
    // product-state error and must stop before spawning the runtime.
    let start = Instant::now();
    let root = entry
        .root_path
        .as_ref()
        .ok_or_else(|| anyhow::anyhow!("agent {agent_name:?} registry row is missing root_path"))?;
    let directory = AgentDirectory::open(root)?;
    if directory.spec().name != agent_name {
        anyhow::bail!(
            "agent {agent_name:?} registry row points at {}, whose agent.toml names {:?}",
            root.display(),
            directory.spec().name
        );
    }
    let cwd = workspace::ensure_from_directory(&directory)?;
    let spec_source = directory.spec().clone();
    // ── Resolve dispatch knobs from spec (falling back to entry) ──
    //
    // Precedence per field:
    //
    //   * `timeout`: `spec.timeout_secs` (Some = explicit user
    //     choice) wins; else `entry.timeout_secs` (v1 / legacy).
    //     Matches `AgentSpec::validate`'s timeout-0 rejection —
    //     if the spec says 60, the dispatch uses 60 even if a
    //     stale registry row still carries 300.
    //   * `max_output_bytes`: no spec field yet — stays on
    //     entry. Future `agent.toml` addition will plug in the
    //     same way.
    //   * `command`: `spec.name`-to-binary is indirect today;
    //     `entry.command` is the override. Keeping this from
    //     entry retains the PR-3b.1.5 test escape hatch for
    //     `dummy_entry()`.
    //   * `env`: `entry.env` only, because PR-3b.2's migration
    //     already evacuated env entries to `.env` files (and
    //     clears `entry.env` on v2 rows). A v2 row has
    //     `entry.env` empty; a v1 row still carries it and
    //     migration will move it on the next load.
    //   * `model`: `spec.model` wins when set; else
    //     `entry.model`. A post-`agent add` edit to
    //     `agent.toml` setting a new model takes effect on the
    //     next dispatch without re-running the CLI.
    //
    // The two `resolve_*` helpers below are the single
    // implementation of the timeout and model precedence rules.
    // Production calls them here; tests call the same functions
    // directly. An equivalent-looking `spec.xxx.unwrap_or(entry.xxx)`
    // *inlined* here would make tests' "assert the rule" either a
    // re-statement of the equation (test theatre) or an E2E
    // subprocess observation (flaky + environment-dependent).
    // Extracting the rule into a function keeps both production
    // and tests bound to the same code path: a refactor that
    // inverts the order must touch the function, and the test
    // calling that function breaks.
    let timeout = resolve_timeout(spec_source.timeout_secs, entry.timeout_secs);
    let max_output = entry.max_output_bytes;
    // Per-call overrides win over spec / entry defaults. The chat
    // ability handler threads its `driver.model` arg through here so
    // a single chat call can pin a different model than what
    // agent.toml carries, without touching the manifest.
    let effective_model = resolve_model_with_overrides(
        overrides.and_then(|o| o.model.clone()),
        spec_source.model.clone(),
        entry.model.clone(),
    );

    // The other DriverOverrides fields (temperature, max_tokens)
    // are rejected at the chat-ability parse boundary — by the time
    // dispatch sees overrides, only `model` can be set. See
    // chat_ability::parse_driver_overrides for the rationale.
    debug_assert!(
        overrides
            .map(|o| o.temperature.is_none() && o.max_tokens.is_none())
            .unwrap_or(true),
        "DriverOverrides reached dispatch with unsupported fields set; chat_ability \
         parse_driver_overrides should have rejected this earlier"
    );

    // Build env for the child subprocess. The env vars are how the typed
    // context crosses the process boundary into the spawned agent CLI —
    // see `runtime::context` for the design rationale. We always emit the
    // depth (incremented by one for the child) and propagate the mission
    // id when one is active.
    //
    // Base env is the v1 `entry.env`; on a v2-migrated row this is empty
    // (migration moved credentials into `<agent-root>/.env`, which the
    // runtime driver reads on its own via the child's cwd). The typed
    // context keys are layered on top.
    let mut env = entry.env.clone();
    active.child(agent_name).serialize_to_env(&mut env);

    // Create a per-run directory. If creation fails (e.g. workspace dir is
    // unwritable), skip persistence — the agent call still runs, but we
    // surface the reason so the operator knows the run is unrecorded.
    let run_dir: Option<Arc<RunDir>> = match RunDir::create(agent_name) {
        Ok(dir) => Some(Arc::new(dir)),
        Err(e) => {
            let err_msg = format!("{e}");
            crate::op_event!(
                component = dispatch,
                kind = run_dir_create_failed,
                level = "warn",
                agent = agent_name,
                error = err_msg,
                fallback = "no_per_run_persistence",
            );
            None
        }
    };
    if let Some(dir) = &run_dir {
        if let Err(e) = dir.write_prompt(&full_prompt) {
            let path_display = format!("{}", dir.path().display());
            let err_msg = format!("{e}");
            crate::op_event!(
                component = dispatch,
                kind = prompt_write_failed,
                level = "warn",
                run_path = path_display,
                error = err_msg,
            );
        }
    }

    // Allocate a PR-7 Session for this dispatch. The Session's
    // invocation_id becomes the cross-reference key between the
    // legacy `runs/<ts>/` directory (human-facing artefacts) and
    // the PersistentLog event log (machine-auditable stream,
    // P1-P6 compliant). Commit 1 of PR-7: dual-write — emit
    // admitted + terminal events to the Timeline alongside the
    // existing run_dir writes. Mid-stream progress events stay
    // in run_dir/trace.jsonl for now; Commit 2 routes them
    // through the Timeline broadcast path.
    //
    // Session construction is infallible: PersistentLog uses its
    // own env-var / tempdir default when the caller passes None.
    // A concurrent dispatch on the same host gets its own
    // invocation_id (uuid v4) and therefore its own log file.
    let session = Session::new(None);
    // Record an `admitted` event as the first timeline entry.
    // The payload names the agent, depth, prompt length, and
    // origin (local vs remote, if the caller established a
    // DispatchContext). A subscriber that joins here sees the
    // same first event as a resumer that replays from offset 0.
    let admitted_payload = serde_json::json!({
        "agent": agent_name,
        "depth": current_depth,
        "prompt_len": full_prompt.len(),
        // `origin_agent` names the root of the dispatch chain
        // when one is active; absent otherwise. "local" for a
        // CLI-direct invocation (`agent send`) with no mission
        // context, or the root agent name inside a mission.
        "origin_agent": active.origin_agent.clone(),
        "mission_id": active.mission_id.clone(),
        "parent_invocation": active
            .parent_invocation
            .as_ref()
            .map(|ctx| ctx.to_json_value()),
        "context_present": context.is_some(),
    });
    if let Err(e) = session.writer().emit("admitted", Some(admitted_payload)) {
        let err_msg = format!("{e}");
        crate::op_event!(
            component = dispatch,
            kind = timeline_admitted_emit_failed,
            level = "warn",
            agent = agent_name,
            error = err_msg,
            fallback = "run_dir_write_is_authoritative",
        );
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
    // Dispatch through the trait. `adapter_for` is the single place
    // the runtime layer matches on `AgentType`; adding a new runtime
    // is one arm there plus one `impl AgentAdapter` block, not a
    // sweep of this function. The `cwd` handed to the adapter is
    // the already-resolved workspace — we clone into `InvokeOpts`
    // because the adapter signature takes `cwd: PathBuf` (no
    // `Option`) to reflect "dispatch always picks a cwd".
    let adapter = adapter_for(entry.agent_type);
    // Hand the adapter a synthetic entry whose `model` reflects
    // the spec-resolved choice. We don't mutate the caller's
    // entry (it's an `&AgentEntry`) — cloning into a local is
    // cheap and keeps this seam narrow. When PR-3b's final
    // cleanup lands (AgentDirectory passed through the adapter
    // signature directly), this synth step collapses into the
    // adapter call.
    let mut entry_for_adapter = entry.clone();
    entry_for_adapter.model = effective_model.clone();
    let run_result = adapter.invoke(
        &entry_for_adapter,
        &full_prompt,
        InvokeOpts {
            timeout,
            max_output_bytes: max_output,
            env,
            cwd: cwd.clone(),
            // Commit 2: the driver emits mid-stream progress
            // events through the Timeline instead of (previously)
            // through run_dir/trace.jsonl. The writer_arc is the
            // same underlying Arc<TimelineWriter> the dispatch
            // layer used for admitted/terminal, so the driver's
            // progress events interleave between them in
            // sequence order.
            timeline: Some(session.writer_arc()),
            progress_tx: progress_tx.clone(),
            // Honor the operator-supplied binary override. Each
            // driver substitutes its own default when this is
            // empty (see `ClaudeOptions::resolved_command` and
            // `CodexOptions::resolved_command`). Plumbing it here
            // is what makes `dummy_entry`'s bogus command in
            // tests actually take effect and what lets operators
            // with a custom install path route through without
            // editing driver source.
            command: entry.command.clone(),
            // Conversation resume: `None` is fresh, `Some(id)` tells
            // a resume-capable driver (codex) to continue under that
            // thread id. Sourced from the chat ability's caller via
            // `DriverOverrides::resume_thread_id`.
            resume_thread_id: overrides.as_ref().and_then(|o| o.resume_thread_id.clone()),
        },
    );

    // Write meta.json regardless of success/failure so failed runs are still
    // inspectable.
    let duration_ms = start.elapsed().as_millis() as u64;
    if let Some(dir) = &run_dir {
        let (exit_status, error, content_for_meta, usage_for_meta) = match &run_result {
            Ok(out) => (
                "ok".to_string(),
                None,
                Some(out.content.as_str()),
                out.usage.clone(),
            ),
            Err(e) => ("error".to_string(), Some(e.to_string()), None, None),
        };
        if let Some(text) = content_for_meta {
            if let Err(e) = dir.write_response(text) {
                let path_display = format!("{}", dir.path().display());
                let err_msg = format!("{e}");
                crate::op_event!(
                    component = dispatch,
                    kind = response_write_failed,
                    level = "warn",
                    run_path = path_display,
                    error = err_msg,
                );
            }
        }
        let u = usage_for_meta.unwrap_or_default();
        let meta = RunMeta {
            agent: agent_name.to_string(),
            agent_type: entry.agent_type.to_string(),
            // Record the model actually dispatched, which is
            // spec-resolved above. A stale entry.model on the
            // registry row must not shadow the spec's choice in
            // the audit trail.
            model: effective_model.clone(),
            // Cross-reference key to the PersistentLog event log.
            // Operators grepping for this id under
            // `$AXON_INVOCATION_LOG_DIR/<id>.jsonl` find the
            // P1-P6-compliant stream of events for this run.
            invocation_id: session.invocation_id().to_string(),
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
            let path_display = format!("{}", dir.path().display());
            let err_msg = format!("{e}");
            crate::op_event!(
                component = dispatch,
                kind = meta_write_failed,
                level = "warn",
                run_path = path_display,
                error = err_msg,
            );
        }
    }

    // Terminal timeline event. One of `completed` / `failed`
    // matches the INVOCATION_STATE_MACHINE.md §2 event kinds.
    // `cancelled` is emitted by a future cancellation path
    // (PR-10 mission supervisor); not reachable from this sync
    // dispatch today. `timed_out` is folded into `failed` with a
    // reason payload — the state machine has separate events,
    // but CLI dispatch surfaces timeout as an error without a
    // distinct handler path, and emitting `failed { reason:
    // "timeout" }` keeps the wire shape consistent with the
    // typed-error future.
    //
    // The fsync on this emit is what gives the log its P4 terminal
    // idempotence: a reader opening the log after we exit finds
    // the terminal state durably recorded.
    let (terminal_type, terminal_payload) = match &run_result {
        Ok(out) => (
            "completed",
            serde_json::json!({
                "content_len": out.content.len(),
                "duration_ms": duration_ms,
                "usage": out.usage,
                "tool_call_count": out.tool_calls.len(),
            }),
        ),
        Err(e) => (
            "failed",
            dispatch_terminal_failure_payload(&e.to_string(), duration_ms),
        ),
    };
    if let Err(e) = session.writer().emit(terminal_type, Some(terminal_payload)) {
        let err_msg = format!("{e}");
        crate::op_event!(
            component = dispatch,
            kind = timeline_terminal_emit_failed,
            level = "warn",
            agent = agent_name,
            error = err_msg,
            fallback = "run_dir_meta_is_authoritative",
        );
    }

    let output = run_result?;

    Ok(AgentResponse {
        agent: agent_name.to_string(),
        content: output.content,
        // Mirror `meta.model`: the response reports the model
        // actually dispatched, which is the spec-resolved one.
        model: effective_model,
        duration_ms,
        truncated: false,
        usage: output.usage,
        run_dir: run_dir.as_ref().map(|d| d.path().to_path_buf()),
        tool_calls: output.tool_calls,
        thread_id: output.thread_id,
    })
}

fn dispatch_terminal_failure_payload(message: &str, duration_ms: u64) -> serde_json::Value {
    let code = crate::runtime::failure_codes::FailureCodeClassifier::classify_or(
        message,
        "INVOCATION_FAILED",
    );
    let class = crate::runtime::failure_codes::FailureCodeClassifier::classify_error_class(&code);
    let retryable = dispatch_failure_retryable(&code);
    let stage = class.stage.as_str_name();
    let security_class = class.security_class.as_str_name();
    serde_json::json!({
        "error": message,
        "duration_ms": duration_ms,
        "failure": {
            "code": code,
            "message": message,
            "retryable": retryable,
            "stage": stage,
            "security_class": security_class,
        },
    })
}

fn dispatch_failure_retryable(code: &str) -> bool {
    let code = code.trim().to_ascii_uppercase();
    code.starts_with("TARGET_")
        || code.starts_with("PRESENCE_")
        || code.starts_with("DEVICE_")
        || code.starts_with("RESOLVE_")
        || matches!(
            code.as_str(),
            "INVOCATION_TIMED_OUT" | "DENDRITE_BRIDGE_LIBRARY_NOT_FOUND"
        )
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
        assert!(
            out.contains(CONTEXT_CLOSE),
            "close sentinel must be present"
        );
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
/// The function returns an error on failure so dispatch fails before
/// subprocess spawn. Missing or forged mission context is product-state
/// corruption, not a recoverable degraded mode.
fn check_mission_context_invariant() -> anyhow::Result<()> {
    let mission_id = match context::current() {
        Some(ctx) if !ctx.mission_id.is_empty() => ctx.mission_id,
        _ => {
            anyhow::bail!(
                "dispatch::send_to_agent called without a mission context. \
                 All agent dispatches must originate from a mission runtime. \
                 See docs/easynet_ontology.tex §6.2."
            );
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
    let mission_run_dir = crate::facade::cli::mission_runs::root_dir().join(&mission_id);
    if !mission_run_dir.exists() {
        anyhow::bail!(
            "mission_id={} does not correspond to an existing \
             mission run dir at {}. Either the env var was forged or \
             the run dir has been cleaned up mid-execution. Refusing \
             to dispatch.",
            mission_id,
            mission_run_dir.display()
        );
    }

    Ok(())
}

// ── Tests ──────────────────────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;
    use crate::registry::agents::{AgentEntry, AgentType};
    use crate::runtime::adapter::AgentAdapter;

    /// Construct a dummy `AgentEntry` for tests that exercise the
    /// dispatch guard logic in isolation.
    ///
    /// Critical: we override two fields of `AgentEntry::new` to keep
    /// these tests fast even on a developer machine that has `claude`
    /// or `codex` installed on `$PATH`:
    ///
    ///   * `command` is scrambled to a name that cannot resolve, so
    ///     `process::Command::spawn` fails with ENOENT in ~milliseconds
    ///     instead of the real binary starting an interactive REPL.
    ///     Tests that only exercise the depth / mission-context
    ///     guards never reach the spawn path; tests that *do* reach
    ///     it (`recursion_guard_allows_depth_1`, the `send_external_*`
    ///     tests) must observe a fast downstream error.
    ///   * `timeout_secs` defaults to 300 s for production use; if a
    ///     test somehow races a real subprocess, that timeout would
    ///     mean 5-minute test hangs. 1 s is still generous for any
    ///     subprocess boot and cheap to observe.
    ///
    /// Without these two overrides a machine with `claude` in PATH
    /// would hang every test that reaches the spawn path, because
    /// Claude Code is interactive-by-default and would wait on stdin
    /// for the full `timeout_secs` window.
    fn dummy_entry() -> AgentEntry {
        let mut e = AgentEntry::new(AgentType::ClaudeCode, None);
        e.command = "easynet-test-nonexistent-agent-binary".to_string();
        e.timeout_secs = 1;
        e
    }

    /// Recursion guard: depth_override=Some(2) must trip the limit
    /// before any subprocess is spawned. The error message must
    /// mention "depth limit reached" so operators can grep for it.
    #[test]
    fn recursion_guard_blocks_at_depth_2() {
        let entry = dummy_entry();
        let res =
            send_to_agent_with_depth("claude", &entry, "any prompt", None, None, Some(2), None);
        let err = res.expect_err("depth=2 must error");
        let msg = format!("{err}");
        assert!(
            msg.contains("depth limit reached"),
            "expected 'depth limit reached' in error, got: {msg}"
        );
    }

    /// Recursion guard at depth=1 must not fire. With the clean
    /// AgentDirectory boundary a dummy entry without `root_path`
    /// fails during registry-root validation before any subprocess
    /// spawn, but it must not be reported as a depth-limit failure.
    #[test]
    fn recursion_guard_allows_depth_1() {
        let entry = dummy_entry();
        let _g = crate::facade::cli::test_support::HomeGuard::new();
        let res =
            send_to_agent_with_depth("claude", &entry, "any prompt", None, None, Some(1), None);
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

    #[test]
    fn dispatch_rejects_registry_row_without_root_path() {
        let _g = crate::facade::cli::test_support::HomeGuard::new();
        let entry = dummy_entry();
        let err = send_to_agent_with_depth("alice", &entry, "prompt", None, None, Some(1), None)
            .expect_err("missing root_path must stop dispatch");
        let msg = format!("{err}");
        assert!(
            msg.contains("missing root_path"),
            "expected missing-root error, got: {msg}"
        );
    }

    #[test]
    fn dispatch_rejects_registry_root_whose_spec_names_another_agent() {
        let _g = crate::facade::cli::test_support::HomeGuard::new();
        let root = crate::persistence::config::agents_root().join("alice");
        let spec = crate::core::agent_spec::AgentSpec::new(
            "bob",
            crate::core::agent_spec::RuntimeKind::ClaudeCode,
        );
        crate::runtime::directory::AgentDirectory::create(
            &crate::runtime::directory::Location::Local { root: root.clone() },
            spec,
        )
        .unwrap();

        let mut entry = dummy_entry();
        entry.root_path = Some(root);
        let err = send_to_agent_with_depth("alice", &entry, "prompt", None, None, Some(1), None)
            .expect_err("spec name mismatch must stop dispatch");
        let msg = format!("{err}");
        assert!(
            msg.contains("agent.toml names \"bob\""),
            "expected spec-name mismatch, got: {msg}"
        );
    }

    /// `send_to_agent_with_depth` with a real `depth_override` must
    /// also bypass the mission-context invariant. This is the test
    /// escape hatch — without it, the unit tests above would have to
    /// set up a real mission run dir, which defeats the purpose of
    /// testing the dispatch path in isolation.
    #[test]
    fn depth_override_bypasses_mission_context_check() {
        // Even with no EASYNET_MISSION_ID set, depth_override=Some(2)
        // should still cleanly hit the depth-limit check, not the
        // mission-context check.
        std::env::remove_var("EASYNET_MISSION_ID");
        let entry = dummy_entry();
        let res =
            send_to_agent_with_depth("claude", &entry, "any prompt", None, None, Some(2), None);
        assert!(res.is_err());
        let msg = format!("{}", res.unwrap_err());
        assert!(msg.contains("depth limit reached"));
    }

    #[test]
    fn send_to_agent_rejects_missing_mission_context_before_registry_root() {
        let _g = crate::facade::cli::test_support::HomeGuard::new();
        std::env::remove_var("EASYNET_MISSION_ID");
        std::env::remove_var("EASYNET_AGENT_DEPTH");
        let entry = dummy_entry();
        let err = send_to_agent_with_depth("alice", &entry, "prompt", None, None, None, None)
            .expect_err("missing mission context must stop dispatch");
        let msg = format!("{err}");
        assert!(
            msg.contains("without a mission context"),
            "expected mission-context error, got: {msg}"
        );
        assert!(
            !msg.contains("missing root_path"),
            "mission-context check must run before registry root validation: {msg}"
        );
    }

    #[test]
    fn send_to_agent_rejects_forged_mission_id_without_run_dir() {
        let _g = crate::facade::cli::test_support::HomeGuard::new();
        std::env::set_var("EASYNET_MISSION_ID", "forged-mission");
        std::env::set_var("EASYNET_AGENT_DEPTH", "0");
        let entry = dummy_entry();
        let err = send_to_agent_with_depth("alice", &entry, "prompt", None, None, None, None)
            .expect_err("unknown mission id must stop dispatch");
        std::env::remove_var("EASYNET_MISSION_ID");
        std::env::remove_var("EASYNET_AGENT_DEPTH");
        let msg = format!("{err}");
        assert!(
            msg.contains("does not correspond to an existing"),
            "expected unknown mission run dir error, got: {msg}"
        );
        assert!(
            !msg.contains("missing root_path"),
            "anti-forgery check must run before registry root validation: {msg}"
        );
    }

    // ── send_external — external-origin entry point ─────────────────────────
    //
    // `send_external` is the entry point the chat-ability adapter uses
    // when a remote caller invokes `<agent>.chat` via MCP. Two invariants
    // matter here:
    //
    //   1. **Depth starts at 0.** An external call is outside any local
    //      mission by definition, so the recursion guard's budget is
    //      fresh. A broken implementation that inherited the current
    //      process's `EASYNET_AGENT_DEPTH` would trip the guard on any
    //      call that landed inside a daemon already running at depth N.
    //   2. **No mission-context invariant.** Because depth 0 < MAX,
    //      `send_external` reaches the spawn path — if the invariant
    //      check were enforced (via `depth_override = None`), no
    //      external caller could invoke this node's agents without
    //      forging a mission id. The test below pins that the function
    //      makes it *past* the invariant check, by observing that it
    //      fails with a downstream error (missing binary) rather than
    //      the mission-context panic/error.

    // These three tests exercise `send_external` end-to-end. Now
    // that the drivers honour `entry.command`, `dummy_entry()` wires
    // a bogus binary name through to the spawn site — the call
    // fails with ENOENT in milliseconds, regardless of what the
    // developer has installed on `$PATH`. The previous `#[ignore]`
    // guards were necessary when `claude_code.rs` / `codex.rs`
    // hard-coded the binary; removing them is the whole point of
    // PR-3b.1.5.

    /// Safe unit test: pin that `send_external` is a one-line
    /// delegation to `send_to_agent_with_depth` with
    /// `depth_override = Some(0)`. We cannot run `send_external`
    /// inline (it would spawn the real `claude` binary — see the
    /// `#[ignore]` notes on the e2e tests below), so we symbolically
    /// test the two halves of its contract via the already-guarded
    /// inner function:
    ///
    ///   * override = MAX → depth guard trips (the baseline).
    ///   * override = MAX-1 → depth guard does NOT trip on the early
    ///     check; any error must come from downstream.
    ///
    /// The equality `send_external(x) == send_to_agent_with_depth(x, Some(0))`
    /// is enforced structurally by the implementation (single
    /// delegation line, see `send_external`'s body). If a future
    /// refactor breaks that structural equality, the `#[ignore]`
    /// e2e tests will catch the behavioural regression when run.
    #[test]
    fn send_external_depth_guard_pins_at_max_not_below() {
        let entry = dummy_entry();

        let tripped = send_to_agent_with_depth(
            "claude",
            &entry,
            "p",
            None,
            None,
            Some(MAX_AGENT_DEPTH),
            None,
        );
        let msg = format!("{}", tripped.expect_err("override=MAX must trip"));
        assert!(
            msg.contains("depth limit"),
            "override=MAX must trip the guard; got: {msg}"
        );

        // The inverse — "override < MAX does not trip the guard" —
        // cannot be tested inline without spawning the real binary,
        // because the path past the early-check leads straight to
        // `adapter.invoke()`. That's what the `#[ignore]` tests
        // below pin. Here we only verify the MAX boundary.
    }

    #[test]
    fn send_external_ignores_inherited_depth_env_var() {
        // A parent process at depth MAX-1 would otherwise poison a child
        // daemon's `send_external` path. Setting the env var here and
        // observing that the depth-limit is not tripped is the guarantee
        // callers need.
        let entry = dummy_entry();
        let _g = crate::facade::cli::test_support::HomeGuard::new();
        std::env::set_var("EASYNET_AGENT_DEPTH", MAX_AGENT_DEPTH.to_string());
        let res = send_external("claude", &entry, "hello", None);
        std::env::remove_var("EASYNET_AGENT_DEPTH");
        // Must not be the depth-limit error — external origin resets to 0.
        let err = res.expect_err("no real claude binary in tests");
        let msg = format!("{err}");
        assert!(
            !msg.contains("depth limit"),
            "send_external must start at depth 0 regardless of inherited \
             env vars; got {msg}"
        );
    }

    #[test]
    fn send_external_does_not_require_mission_context() {
        // No EASYNET_MISSION_ID, no typed context installed — the
        // invariant check would otherwise panic (debug) or error
        // (release). `send_external` must bypass via depth_override = 0
        // and fail only at the downstream spawn path.
        std::env::remove_var("EASYNET_MISSION_ID");
        let _g = crate::facade::cli::test_support::HomeGuard::new();
        let entry = dummy_entry();
        let res = send_external("claude", &entry, "hello", None);
        let err = res.expect_err("no real claude binary in tests");
        let msg = format!("{err}");
        // The two strings the invariant check produces:
        assert!(
            !msg.contains("mission context"),
            "send_external must not enforce the mission-context invariant; got {msg}"
        );
        assert!(
            !msg.contains("mission run dir"),
            "send_external must not trip stage-2 anti-forgery; got {msg}"
        );
    }

    #[test]
    fn send_external_accepts_optional_context_preamble() {
        // The adapter may or may not pass `context` — both shapes must
        // get past the invariant/depth checks. We don't assert on the
        // content of the downstream error (process spawn), only that
        // neither flavour is gated by the top-of-function checks.
        let entry = dummy_entry();
        let _g = crate::facade::cli::test_support::HomeGuard::new();
        for ctx in [None, Some("be terse")] {
            let res = send_external("claude", &entry, "hello", ctx);
            let err = res.expect_err("no real claude binary in tests");
            let msg = format!("{err}");
            assert!(!msg.contains("depth limit"));
            assert!(!msg.contains("mission context"));
        }
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
        let _g = crate::facade::cli::test_support::HomeGuard::new();
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

    // ── AgentAdapter trait contract ─────────────────────────────────

    /// A synthetic adapter used to prove the trait's contract holds
    /// in isolation: dispatch hands the adapter a prompt and opts,
    /// the adapter returns a `(String, Option<AgentUsage>)` pair,
    /// dispatch returns an `AgentResponse` that preserves both.
    ///
    /// This is a lightweight equivalent of the failure-mode tests we
    /// run against each real driver; it lets us verify the trait
    /// seam without spawning a real binary.
    struct MockAdapter {
        runtime_id: &'static str,
        response: String,
        usage: Option<crate::runtime::dispatch::AgentUsage>,
        tool_calls: Vec<crate::runtime::dispatch::ToolCall>,
    }

    impl crate::runtime::adapter::AgentAdapter for MockAdapter {
        fn runtime_id(&self) -> &'static str {
            self.runtime_id
        }

        fn is_available(&self) -> bool {
            true
        }

        fn invoke(
            &self,
            _entry: &AgentEntry,
            _prompt: &str,
            _opts: crate::runtime::adapter::InvokeOpts,
        ) -> anyhow::Result<crate::runtime::adapter::AdapterOutput> {
            Ok(crate::runtime::adapter::AdapterOutput {
                content: self.response.clone(),
                usage: self.usage.clone(),
                tool_calls: self.tool_calls.clone(),
                thread_id: None,
            })
        }
    }

    #[test]
    fn mock_adapter_returns_its_scripted_response() {
        let adapter = MockAdapter {
            runtime_id: "mock",
            response: "synthetic reply".into(),
            usage: Some(AgentUsage {
                input_tokens: 7,
                output_tokens: 13,
                ..Default::default()
            }),
            tool_calls: Vec::new(),
        };

        // Call the adapter directly — this is the narrow seam the
        // dispatch layer uses. If the trait contract ever
        // regresses (wrong argument order, missing field, etc.)
        // this test fails before any driver-specific test does.
        let entry = dummy_entry();
        let opts = crate::runtime::adapter::InvokeOpts {
            timeout: Duration::from_secs(1),
            max_output_bytes: 1024,
            env: std::collections::BTreeMap::new(),
            cwd: std::path::PathBuf::from("."),
            timeline: None,
            progress_tx: None,
            command: String::new(),
            resume_thread_id: None,
        };
        let out = adapter
            .invoke(&entry, "ignored prompt", opts)
            .expect("mock adapter must succeed");
        assert_eq!(out.content, "synthetic reply");
        let usage = out.usage.expect("mock returned Some(usage)");
        assert_eq!(usage.input_tokens, 7);
        assert_eq!(usage.output_tokens, 13);
    }

    #[test]
    fn mock_adapter_can_omit_usage() {
        // Codex `app-server` mode has no structured usage today.
        // A real adapter returns `None`; the trait's Option<Usage>
        // must flow through dispatch without special-casing.
        let adapter = MockAdapter {
            runtime_id: "mock-no-usage",
            response: "ok".into(),
            usage: None,
            tool_calls: Vec::new(),
        };
        let entry = dummy_entry();
        let opts = crate::runtime::adapter::InvokeOpts {
            timeout: Duration::from_secs(1),
            max_output_bytes: 64,
            env: std::collections::BTreeMap::new(),
            cwd: std::path::PathBuf::from("."),
            timeline: None,
            progress_tx: None,
            command: String::new(),
            resume_thread_id: None,
        };
        let out = adapter.invoke(&entry, "p", opts).unwrap();
        assert!(out.usage.is_none());
    }

    #[test]
    fn mock_adapter_round_trips_tool_calls() {
        // Phase: Fix-3 wiring. The chat ability surfaces
        // `tool_calls` in its structured response. Capture pipes
        // through: adapter records → AdapterOutput.tool_calls →
        // AgentResponse.tool_calls → chat handler json. Pin the
        // first three hops here; the chat handler hop is covered
        // by chat_ability::tests::handler_surfaces_tool_calls.
        let adapter = MockAdapter {
            runtime_id: "mock-with-tools",
            response: "I called two tools".into(),
            usage: None,
            tool_calls: vec![
                ToolCall {
                    ability: "alice.voice".into(),
                    args: serde_json::json!({"text": "hi"}),
                    ..Default::default()
                },
                ToolCall {
                    ability: "alice.exec".into(),
                    args: serde_json::json!({"cmd": "ls"}),
                    ..Default::default()
                },
            ],
        };
        let entry = dummy_entry();
        let opts = crate::runtime::adapter::InvokeOpts {
            timeout: Duration::from_secs(1),
            max_output_bytes: 64,
            env: std::collections::BTreeMap::new(),
            cwd: std::path::PathBuf::from("."),
            timeline: None,
            progress_tx: None,
            command: String::new(),
            resume_thread_id: None,
        };
        let out = adapter.invoke(&entry, "p", opts).unwrap();
        assert_eq!(out.tool_calls.len(), 2);
        assert_eq!(out.tool_calls[0].ability, "alice.voice");
        assert_eq!(out.tool_calls[1].ability, "alice.exec");
    }

    #[test]
    fn adapter_for_returns_distinct_singletons_per_agent_type() {
        // Each AgentType must map to its own adapter. The runtime_id
        // accessor is the visible fingerprint — if a future mapper
        // accidentally aliases two types to one adapter, the id's
        // drift from `agent_type.to_string()` immediately.
        use crate::runtime::drivers::adapter_for;
        let a = adapter_for(AgentType::ClaudeCode);
        let b = adapter_for(AgentType::Codex);
        let c = adapter_for(AgentType::CodexAppServer);
        assert_eq!(a.runtime_id(), "claude-code");
        assert_eq!(b.runtime_id(), "codex");
        assert_eq!(c.runtime_id(), "codex-app-server");
    }

    // ── spec-over-entry precedence (PR-3b.5 / 3b.5.1) ───────────────────
    //
    // These tests exercise the real `send_to_agent_with_depth`
    // code path rather than restate its equation in the test
    // body. The shape:
    //
    //   1. Materialize an AgentDirectory with a distinct
    //      spec.model / spec.timeout_secs.
    //   2. Build an AgentEntry carrying conflicting
    //      entry.model / entry.timeout_secs values.
    //   3. Call `send_to_agent_with_depth` with
    //      `depth_override = Some(1)` so the real dispatch
    //      flows: `AgentDirectory::open` validates the registered
    //      root, `effective_model` / `effective_timeout` resolve
    //      from spec, and the adapter invoke fails fast
    //      (dummy_entry's bogus `command` → ENOENT in ms).
    //   4. Inspect `<run-dir>/meta.json` — the authoritative
    //      audit record the dispatcher wrote BEFORE returning
    //      the error. The field values there are what
    //      production really computed, so if the precedence
    //      rule regresses these assertions break.
    //
    // A refactor that inverts the `or_else` order in
    // `send_to_agent_with_depth:268-277` would now flip the
    // model written to meta.json — which the test catches.
    // The previous "equation-restating" tests would have
    // stayed green because they recomputed the same equation
    // in the test body; these don't.

    /// Read the most-recently-created `meta.json` under
    /// `<agent-root>/runs/`. Tests use this to inspect what
    /// the dispatcher actually persisted for a run that
    /// failed downstream (at spawn time). Returns `None` if
    /// no run dir exists yet (e.g. run_dir creation itself
    /// failed, which is the degraded-mode branch we are not
    /// testing here).
    fn read_latest_meta(agent_root: &std::path::Path) -> Option<RunMeta> {
        let runs = agent_root.join("runs");
        if !runs.is_dir() {
            return None;
        }
        let mut entries: Vec<_> = std::fs::read_dir(&runs)
            .ok()?
            .filter_map(|e| e.ok())
            .filter(|e| e.path().is_dir())
            .collect();
        // Sort by file name (ISO-8601 timestamp → lexicographic
        // order matches chronological order for same-day stamps)
        // and take the latest.
        entries.sort_by_key(|e| e.file_name());
        let latest = entries.last()?;
        let meta_path = latest.path().join("meta.json");
        let data = std::fs::read_to_string(&meta_path).ok()?;
        serde_json::from_str(&data).ok()
    }

    /// Build an AgentDirectory at the global `agents_root()`
    /// with a custom spec, then return the entry that points
    /// at it. Factored so the three tests below share the
    /// setup dance without copy-paste.
    fn seed_agent_with_spec(
        name: &str,
        spec_model: Option<&str>,
        spec_timeout: Option<u64>,
    ) -> AgentEntry {
        use crate::core::agent_spec::{AgentSpec, RuntimeKind};
        use crate::runtime::directory::{AgentDirectory, Location};

        let root = crate::persistence::config::agents_root().join(name);
        let mut spec = AgentSpec::new(name, RuntimeKind::ClaudeCode);
        spec.model = spec_model.map(str::to_string);
        spec.timeout_secs = spec_timeout;
        AgentDirectory::create(&Location::Local { root: root.clone() }, spec).unwrap();

        let mut entry = dummy_entry();
        entry.root_path = Some(root);
        entry
    }

    #[test]
    fn spec_model_is_written_to_meta_json_when_set() {
        // Install a spec with its own model; entry carries a
        // disagreeing one. Run the real dispatch path. The
        // meta.json left behind by the dispatcher must record
        // the spec's model, not the entry's.
        let _g = crate::facade::cli::test_support::HomeGuard::new();
        let mut entry = seed_agent_with_spec("alice", Some("spec-chosen-model"), None);
        entry.model = Some("stale-registry-model".into());
        let root = entry.root_path.clone().unwrap();

        // depth_override=Some(1) bypasses mission-context
        // enforcement (see recursion_guard_allows_depth_1's
        // rationale) and lets the dispatch reach the spawn
        // step, where `dummy_entry`'s bogus command fails fast.
        let res = send_to_agent_with_depth("alice", &entry, "prompt", None, None, Some(1), None);
        // Failure is expected — we don't need the response.
        // meta.json is written whether the run succeeded or
        // failed (see dispatch.rs's "Write meta.json regardless"
        // block), so we still have something to observe.
        let _ = res;

        let meta = read_latest_meta(&root).expect("meta.json must exist after dispatch");
        assert_eq!(
            meta.model.as_deref(),
            Some("spec-chosen-model"),
            "dispatcher must record spec.model (got {:?}; entry.model was {:?})",
            meta.model,
            entry.model
        );
    }

    #[test]
    fn entry_model_is_used_when_spec_model_is_none() {
        // Legacy path: spec carries no model, entry does.
        // dispatcher must fall back to entry.model in meta.json
        // so pre-v2 registry rows continue to dispatch to the
        // model their row names — operators whose agents have
        // not been touched since upgrade must see no regression.
        let _g = crate::facade::cli::test_support::HomeGuard::new();
        let mut entry = seed_agent_with_spec("alice", None, None);
        entry.model = Some("legacy-entry-model".into());
        let root = entry.root_path.clone().unwrap();

        let _ = send_to_agent_with_depth("alice", &entry, "prompt", None, None, Some(1), None);

        let meta = read_latest_meta(&root).expect("meta.json must exist");
        assert_eq!(
            meta.model.as_deref(),
            Some("legacy-entry-model"),
            "spec.model = None must fall back to entry.model; got {:?}",
            meta.model
        );
    }

    #[test]
    fn both_models_none_yields_meta_model_none() {
        // Neither side names a model — meta.json records None
        // (serializes as absent under `skip_serializing_if`).
        // The dispatched runtime then falls back to its own
        // default model, but that's the driver's concern; from
        // dispatch's perspective the correct record is "no
        // operator preference".
        let _g = crate::facade::cli::test_support::HomeGuard::new();
        let mut entry = seed_agent_with_spec("alice", None, None);
        entry.model = None;
        let root = entry.root_path.clone().unwrap();

        let _ = send_to_agent_with_depth("alice", &entry, "prompt", None, None, Some(1), None);

        let meta = read_latest_meta(&root).expect("meta.json must exist");
        assert!(
            meta.model.is_none(),
            "both-None must record None in meta; got {:?}",
            meta.model
        );
    }

    // ── resolve_timeout / resolve_model (spec-over-entry rule) ──
    //
    // These two helpers are the single implementation of the
    // spec-vs-entry precedence rule for timeouts and model
    // selection. Production `send_to_agent_with_depth` calls
    // them; these tests call the same functions.
    //
    // This is the honest form of the rule we pin: the tests
    // are not "recompute the equation and assert the result"
    // (the test-theatre shape the earlier iteration took);
    // they are "drive the one function production uses and
    // observe its output". A refactor that inverts the
    // precedence order in `resolve_timeout` (say, swaps to
    // `entry_timeout_secs.unwrap_or(spec_timeout_secs)` or
    // somesuch) flips these test outputs and the tests break.
    //
    // The sibling `*_is_written_to_meta_json_when_set` tests
    // above complete the picture by proving that the resolved
    // values flow all the way into `meta.json` through real
    // `send_to_agent_with_depth`. Unit tests on the helper
    // pin the rule; integration tests on the dispatcher pin
    // the wiring. Either can fail independently and point at
    // the correct layer.

    #[test]
    fn resolve_timeout_prefers_spec_when_set() {
        let t = resolve_timeout(Some(42), 300);
        assert_eq!(t, Duration::from_secs(42));
    }

    #[test]
    fn resolve_timeout_falls_back_to_entry_when_spec_none() {
        let t = resolve_timeout(None, 300);
        assert_eq!(t, Duration::from_secs(300));
    }

    #[test]
    fn resolve_timeout_spec_trumps_entry_even_when_entry_is_smaller() {
        // Guard against "pick the smaller of the two" drift
        // — some tempting but wrong rules would satisfy the
        // simple "spec wins when larger" case. If a refactor
        // ever changes to `.min(entry_timeout_secs)` this
        // test catches it.
        let t = resolve_timeout(Some(600), 30);
        assert_eq!(t, Duration::from_secs(600));
    }

    #[test]
    fn resolve_model_prefers_spec_when_set() {
        let m = resolve_model(Some("spec-model".into()), Some("entry-model".into()));
        assert_eq!(m.as_deref(), Some("spec-model"));
    }

    #[test]
    fn resolve_model_falls_back_to_entry_when_spec_none() {
        let m = resolve_model(None, Some("entry-model".into()));
        assert_eq!(m.as_deref(), Some("entry-model"));
    }

    #[test]
    fn resolve_model_both_none_yields_none() {
        let m = resolve_model(None, None);
        assert_eq!(m, None);
    }

    #[test]
    fn resolve_model_spec_some_overrides_entry_none() {
        // The asymmetric case: a spec that explicitly names a
        // model must not be shadowed by a None on the entry
        // row. This one tripped the original "or_else vs or"
        // choice — `Option::or_else` evaluates the entry
        // closure only when spec is None, which is the shape
        // we want.
        let m = resolve_model(Some("spec-model".into()), None);
        assert_eq!(m.as_deref(), Some("spec-model"));
    }

    // ── resolve_model_with_overrides (chat ability driver.model) ────────────

    #[test]
    fn resolve_model_with_overrides_per_call_wins_over_spec_and_entry() {
        // The whole point of the chat `driver.model` field: a
        // per-invocation override beats both the agent.toml spec
        // and the legacy entry default. Pin that explicitly so a
        // refactor of `or` chains here can't silently invert
        // precedence.
        let m = resolve_model_with_overrides(
            Some("override-model".into()),
            Some("spec-model".into()),
            Some("entry-model".into()),
        );
        assert_eq!(m.as_deref(), Some("override-model"));
    }

    #[test]
    fn resolve_model_with_overrides_falls_through_to_spec_when_override_none() {
        // No per-call override → spec wins (matches resolve_model's
        // pre-overrides contract).
        let m = resolve_model_with_overrides(
            None,
            Some("spec-model".into()),
            Some("entry-model".into()),
        );
        assert_eq!(m.as_deref(), Some("spec-model"));
    }

    #[test]
    fn resolve_model_with_overrides_falls_through_to_entry_when_override_and_spec_none() {
        let m = resolve_model_with_overrides(None, None, Some("entry-model".into()));
        assert_eq!(m.as_deref(), Some("entry-model"));
    }

    #[test]
    fn resolve_model_with_overrides_all_none_yields_none() {
        let m = resolve_model_with_overrides(None, None, None);
        assert_eq!(m, None);
    }

    #[test]
    fn resolve_model_with_overrides_override_none_does_not_shadow_spec() {
        // An explicit `Some("...")` on a lower tier must not be
        // shadowed by `None` from a higher tier. `Option::or` has
        // the right semantics here; pin it because a `unwrap_or`
        // chain or a `match` statement could easily invert the
        // shape.
        let m = resolve_model_with_overrides(None, Some("spec".into()), None);
        assert_eq!(m.as_deref(), Some("spec"));
        let m = resolve_model_with_overrides(None, None, Some("entry".into()));
        assert_eq!(m.as_deref(), Some("entry"));
    }

    // ── PR-7 Session + Timeline dispatch integration ────────────────────────

    /// Read the Timeline events for the most recent run, using the
    /// invocation_id recorded in that run's meta.json as the key.
    /// Returns `None` when either meta.json or the events file is
    /// missing (which, under the dual-write discipline, should not
    /// happen for any dispatch that produced a run_dir).
    fn read_timeline_for_latest_run(
        agent_root: &std::path::Path,
    ) -> Option<Vec<serde_json::Value>> {
        let meta = read_latest_meta(agent_root)?;
        if meta.invocation_id.is_empty() {
            return None;
        }
        // PersistentLog uses $AXON_INVOCATION_LOG_DIR (set by the
        // HomeGuard-equivalent tempdir redirect) or a shared
        // tempdir by default. Open a bare PersistentLog on the
        // same dir the dispatch wrote to and read by id.
        use easynet_axon::invocation::persistence::PersistentLog;
        let log = PersistentLog::new(None);
        Some(log.read_events(&meta.invocation_id, 0))
    }

    #[test]
    fn dispatch_emits_admitted_and_failed_events_when_adapter_fails() {
        // The real dispatch path: `dummy_entry` has a bogus
        // command, so the adapter spawn fails fast. We expect
        // TWO timeline events to land on disk: `admitted` at
        // dispatch entry, and `failed` at the terminal point.
        // The meta.json must carry the invocation_id; the
        // timeline file must carry both events in sequence order.
        let _g = crate::facade::cli::test_support::HomeGuard::new();
        let entry = seed_agent_with_spec("alice", Some("model-x"), None);
        let root = entry.root_path.clone().unwrap();

        let _ = send_to_agent_with_depth("alice", &entry, "hello", None, None, Some(1), None);

        let meta = read_latest_meta(&root).expect("meta.json must exist");
        assert!(
            !meta.invocation_id.is_empty(),
            "meta.invocation_id must be populated after PR-7 Commit 1"
        );
        assert!(
            meta.invocation_id.starts_with("cli-"),
            "invocation_id prefix signals CLI-allocated uuid, got {:?}",
            meta.invocation_id
        );

        let events = read_timeline_for_latest_run(&root)
            .expect("timeline events must exist for this invocation_id");
        assert_eq!(
            events.len(),
            2,
            "expected exactly admitted + failed (2 events), got {}: {events:?}",
            events.len()
        );
        assert_eq!(events[0]["sequence"], 0);
        assert_eq!(events[0]["type"], "admitted");
        assert_eq!(events[0]["payload"]["agent"], "alice");
        // prompt_len is the length of the composed prompt —
        // pinning only that it's present and non-negative avoids
        // coupling to the exact prompt-composition format while
        // still asserting the payload was populated.
        assert!(events[0]["payload"]["prompt_len"].as_i64().unwrap() > 0);

        assert_eq!(events[1]["sequence"], 1);
        assert_eq!(events[1]["type"], "failed");
        // The failure payload carries the driver error message.
        // We don't pin the exact text (driver-dependent), only
        // that an `error` string is present and non-empty.
        assert!(
            events[1]["payload"]["error"]
                .as_str()
                .map(|s| !s.is_empty())
                .unwrap_or(false),
            "failed event must carry non-empty error string, got {:?}",
            events[1]["payload"]
        );
        assert_eq!(
            events[1]["payload"]["failure"]["code"], "INVOCATION_FAILED",
            "failed event must carry canonical typed failure, got {:?}",
            events[1]["payload"]
        );
        assert_eq!(
            events[1]["payload"]["failure"]["stage"],
            "ERROR_STAGE_EXECUTION"
        );
        assert_eq!(
            events[1]["payload"]["failure"]["security_class"],
            "SECURITY_CLASS_UNSPECIFIED"
        );
        assert_eq!(events[1]["payload"]["failure"]["retryable"], false);
    }

    #[test]
    fn dispatch_terminal_failure_payload_preserves_specific_runtime_code() {
        let payload =
            dispatch_terminal_failure_payload("target device is not in PresenceRegistry", 42);

        assert_eq!(payload["error"], "target device is not in PresenceRegistry");
        assert_eq!(payload["duration_ms"], 42);
        assert_eq!(
            payload["failure"]["code"],
            "TARGET_NOT_IN_PRESENCE_REGISTRY"
        );
        assert_eq!(payload["failure"]["stage"], "ERROR_STAGE_TRANSPORT");
        assert_eq!(
            payload["failure"]["security_class"],
            "SECURITY_CLASS_TRANSPORT"
        );
        assert_eq!(payload["failure"]["retryable"], true);
    }

    #[test]
    fn terminal_event_marks_index_terminal_on_disk() {
        // P4 (terminal idempotence) composed with dispatch: after
        // a failed run, a fresh PersistentLog reader must see the
        // FAILED terminal state in the index. This catches a
        // hypothetical refactor where the terminal emit gets
        // skipped (e.g. added behind a flag) — the index would
        // stay at RUNNING and P4 would be silently violated.
        let _g = crate::facade::cli::test_support::HomeGuard::new();
        let entry = seed_agent_with_spec("alice", None, None);
        let root = entry.root_path.clone().unwrap();
        let _ = send_to_agent_with_depth("alice", &entry, "hello", None, None, Some(1), None);

        let meta = read_latest_meta(&root).expect("meta.json");
        use easynet_axon::invocation::persistence::PersistentLog;
        let log = PersistentLog::new(None);
        let idx = log
            .read_index(&meta.invocation_id)
            .expect("index must be present after terminal emit");
        assert_eq!(
            idx.terminal_state.as_deref(),
            Some("FAILED"),
            "terminal state must be recorded, got {:?}",
            idx.terminal_state
        );
        assert_eq!(
            idx.last_sequence, 1,
            "two events emitted, last_sequence = 1"
        );
    }

    /// Make a progress event shaped exactly like the driver
    /// callbacks emit (see `runtime/drivers/claude_code.rs` and
    /// `runtime/drivers/codex.rs`). Called from the test below
    /// to simulate the stream without spawning a real binary.
    ///
    /// The payload shape is part of the contract PR-10
    /// services/chat + Frontend AgentDetailPage will read, so
    /// pinning it here protects downstream consumers against a
    /// future driver refactor that drops the `driver` / `chunk`
    /// keys.
    fn simulate_driver_progress(
        timeline: &crate::runtime::timeline::TimelineWriter,
        driver: &str,
        line: &str,
    ) {
        let payload = match serde_json::from_str::<serde_json::Value>(line) {
            Ok(v) => serde_json::json!({"driver": driver, "chunk": v}),
            Err(_) => serde_json::json!({"driver": driver, "raw": line}),
        };
        timeline.emit("progress", Some(payload)).unwrap();
    }

    #[test]
    fn timeline_progress_events_carry_driver_and_chunk_shape() {
        // The contract PR-10 services relies on: every progress
        // event carries `{driver, chunk|raw}`. Structured
        // JSONL driver output becomes `chunk`; garbage lines
        // become `raw`. Both shapes round-trip through disk.
        use crate::runtime::session::Session;
        let _g = crate::facade::cli::test_support::HomeGuard::new();
        let session = Session::new(None);
        session.writer().emit("admitted", None).unwrap();
        simulate_driver_progress(
            session.writer(),
            "claude-code",
            r#"{"kind":"delta","text":"hi"}"#,
        );
        simulate_driver_progress(session.writer(), "codex", "not-json-at-all");
        session.writer().emit("completed", None).unwrap();

        let events = session.resume_replay(0);
        assert_eq!(events.len(), 4);
        assert_eq!(events[1].event_type, "progress");
        let p1 = events[1].payload.as_ref().unwrap();
        assert_eq!(p1["driver"], "claude-code");
        assert_eq!(p1["chunk"]["kind"], "delta");
        assert_eq!(p1["chunk"]["text"], "hi");

        assert_eq!(events[2].event_type, "progress");
        let p2 = events[2].payload.as_ref().unwrap();
        assert_eq!(p2["driver"], "codex");
        assert_eq!(p2["raw"], "not-json-at-all");

        assert_eq!(events[3].event_type, "completed");
    }

    #[test]
    fn timeline_broadcast_delivers_progress_events_live() {
        // A subscriber attached BEFORE emit sees each progress
        // event in the order the driver produced it. This is the
        // path services/chat will use in PR-10 to tail active
        // invocations; pinning it here protects the capability
        // against a future refactor that drops broadcast wake
        // from the emit path.
        use crate::runtime::session::Session;
        let _g = crate::facade::cli::test_support::HomeGuard::new();
        let session = Session::new(None);
        let mut rx = session.subscribe();
        session.writer().emit("admitted", None).unwrap();
        for n in 0..5 {
            simulate_driver_progress(
                session.writer(),
                "claude-code",
                &format!(r#"{{"chunk_n":{n}}}"#),
            );
        }
        session.writer().emit("completed", None).unwrap();

        // 1 admitted + 5 progress + 1 completed = 7 events
        // queued on the broadcast receiver.
        let mut received = Vec::new();
        while let Ok(ev) = rx.try_recv() {
            received.push(ev);
        }
        assert_eq!(received.len(), 7);
        assert_eq!(received[0].event_type, "admitted");
        for (i, ev) in received[1..6].iter().enumerate() {
            assert_eq!(
                ev.event_type,
                "progress",
                "event {} must be progress",
                i + 1
            );
            assert_eq!(
                ev.payload.as_ref().unwrap()["chunk"]["chunk_n"],
                i as i64,
                "progress events must arrive in driver-emit order (no reordering)"
            );
        }
        assert_eq!(received[6].event_type, "completed");
    }

    #[test]
    fn runs_directory_no_longer_contains_trace_jsonl() {
        // PR-7 Commit 2 removed the runs/trace.jsonl write path.
        // A dispatch that uses `dummy_entry` (fails at spawn, so
        // no driver stream emits anyway) must still produce a
        // run_dir with {prompt.txt, meta.json} but NOT
        // trace.jsonl. This is the visible contract change
        // operators see on disk.
        let _g = crate::facade::cli::test_support::HomeGuard::new();
        let entry = seed_agent_with_spec("alice", None, None);
        let root = entry.root_path.clone().unwrap();
        let _ = send_to_agent_with_depth("alice", &entry, "hello", None, None, Some(1), None);

        // Find the latest run directory.
        let runs = root.join("runs");
        let mut entries: Vec<_> = std::fs::read_dir(&runs)
            .expect("runs dir must exist")
            .filter_map(|e| e.ok())
            .filter(|e| e.path().is_dir())
            .collect();
        entries.sort_by_key(|e| e.file_name());
        let latest = entries.last().expect("at least one run");
        let run_path = latest.path();

        assert!(
            run_path.join("prompt.txt").exists(),
            "prompt.txt still persists"
        );
        assert!(
            run_path.join("meta.json").exists(),
            "meta.json still persists"
        );
        assert!(
            !run_path.join("trace.jsonl").exists(),
            "trace.jsonl must NOT exist — moved to Timeline in PR-7 Commit 2"
        );
    }

    #[test]
    fn meta_json_and_timeline_agree_on_invocation_id() {
        // Cross-reference invariant: the invocation_id in
        // meta.json MUST equal the file stem of the Timeline
        // log. Operators rely on this — copy-pasting the id
        // from meta.json into a grep on the log dir must land
        // on the run's events. If a future refactor separates
        // the two allocation sites, this test trips.
        let _g = crate::facade::cli::test_support::HomeGuard::new();
        let entry = seed_agent_with_spec("alice", None, None);
        let root = entry.root_path.clone().unwrap();
        let _ = send_to_agent_with_depth("alice", &entry, "hello", None, None, Some(1), None);

        let meta = read_latest_meta(&root).expect("meta.json");
        use easynet_axon::invocation::persistence::PersistentLog;
        let log = PersistentLog::new(None);
        let expected_path = log.events_path(&meta.invocation_id);
        assert!(
            expected_path.exists(),
            "Timeline log file must exist at PersistentLog path for meta.invocation_id. \
             meta.invocation_id = {:?}, expected path = {}",
            meta.invocation_id,
            expected_path.display(),
        );
    }
}
