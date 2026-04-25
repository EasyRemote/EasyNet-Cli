// EasyNet CLI — `<agent>.chat` system-registered ability
// =======================================================
//
// File: src/runtime/system/chat_ability.rs
// Description: First-class registration of every locally-installed
//              agent's `chat` ability on the daemon's
//              `LocalAbilityRegistry`. After this lands, both the
//              Kernel and the MCP adapter can dispatch through the
//              same registered handler instead of each maintaining
//              their own special-case path into `send_external`.
//
// Why this is in `system::*` even though the wire name is `<agent>.chat`
// ----------------------------------------------------------------------
// The directory `runtime::system/` is the registration surface — every
// file here mounts handlers on the registry. The `system.<feature>`
// naming convention is a rule about *which abilities are device-level*,
// not a rule about which files are allowed to register handlers. Chat
// is bound to a specific agent (so its name is `<agent>.chat`, not
// `system.chat`), but it is still registered by the daemon at boot
// from this module — there is no agent-side code path for it.
//
// Per-agent registration
// ----------------------
// Unlike `system.ping` (one handler globally) or `system.session.list`
// (one handler that reads from a shared `SessionService`), chat
// registers one handler **per agent** in the registry. The handler
// closure captures the agent name + entry by value, so a later
// `get_rpc("alice.chat")` resolves to a closure that already knows it
// is dispatching to `alice` and does not re-look-up the registry.
//
// Why the registry snapshot is captured at registration time
// ----------------------------------------------------------
// The ergonomic alternative would be to register a single
// "chat-router" handler that does a registry lookup at every call.
// We do not, for two reasons:
//   1. It would couple the chat handler to the persistence-layer
//      `load_agents()` call on every invocation — cheap but not free,
//      and the failure mode (registry briefly unreadable) would surface
//      as a chat error rather than a startup error.
//   2. The registry is the single-writer in the daemon (see
//      `LocalAbilityRegistry::register_rpc` doc): adding a new agent
//      means re-running boot or calling `register` again with the
//      updated registry. Mid-life additions are rare today and the
//      explicit "re-register on add" pattern keeps the dispatch path
//      monomorphic and easy to reason about.
//
// What this PR's handler does NOT yet do
// --------------------------------------
// This file ships the manifest-aligned argument parser and the
// minimum-viable invocation path. The fields the new schema declares
// (`session_id`, `skills.{mode,include,exclude}`,
// `context_loaders.{...}`, `driver.{model,temperature,max_tokens}`,
// `stream`) are all parsed and surfaced in the response, but the
// substantive behaviours land in subsequent PRs:
//
//   * skills loading: `skills_loaded` enumerates the agent's other
//     abilities (filtered by mode/include/exclude). Those abilities
//     are ALREADY callable by the LLM — the workspace's .mcp.json
//     points at the EasyNet MCP server with `--enable-agent-dispatch`
//     so the AgentDispatchAdapter advertises every <agent>.chat tool.
//     The skills filter is currently advisory: we report what we
//     would expose; per-call enforcement of the include/exclude
//     filter against claude-code's tool-discovery wire is a follow-up.
//   * context loaders: the trait seam exists; v1 ships ScheduleLoader
//     / MemoryLoader / UserProfileLoader. `context_used` reports
//     which loaders contributed and how many bytes each.
//   * driver overrides: `driver.model` flows through dispatch via
//     send_external_with_overrides. `temperature` and `max_tokens`
//     are accepted by the schema and recorded but not piped to the
//     v1 claude-code / codex CLI drivers (see warn_unhonored_driver_knobs_once
//     in dispatch.rs).
//   * stream: register_for_agent mounts both an RPC and a Stream
//     handler; the stream variant emits typed frames. `stream:true`
//     under the RPC entry point is rejected with a clear error.
//
// The output schema's `usage`, `tool_calls`, and `context_used`
// fields are populated by the driver layer's tool-use observability
// (via dispatch::ToolCall, projected from claude-code's tool_use
// stream events). Codex adapters return empty `tool_calls` because
// they do not surface tool-use events. `usage` mirrors
// `AgentResponse.usage` when the driver reports it.
//
// Author: Silan Hu <silan.hu@u.nus.edu>
// Copyright (c) 2026 EasyNet. All rights reserved.

use std::sync::Arc;
use std::time::Instant;

use serde_json::{json, Value};

use crate::registry::agents::{AgentEntry, AgentRegistry};
use crate::runtime::ability_dispatch::{LocalAbilityRegistry, StreamSource};
use crate::runtime::dispatch::DriverOverrides;

/// The wire-level *verb* portion of every chat ability name. The
/// fully-qualified ability name is always `<agent>.chat`. A future
/// rename here would have to ripple through:
///   * the parity test in `runtime::abilities`
///   * the manifest seed in `core::ability_spec::default_chat_manifest`
///   * the EasyNet backend's frontend that synthesizes / renders chat
///
/// Pinning the constant in one place lets that future PR find the
/// surface area with a single grep.
pub const ABILITY_VERB: &str = "chat";

/// Trait that pluggable context loaders implement. Each loader
/// contributes a string fragment that the chat handler appends to
/// the assembled context block before invoking the LLM.
///
/// This is the **seam** for "load user profile, scheduled tasks,
/// memory" without putting them all in chat_ability itself. Today no
/// concrete loaders are registered — the seam exists so subsequent
/// PRs can plug in `UserProfileLoader`, `ScheduleLoader`, etc.
/// without changing this file.
///
/// Implementations should:
///   - return `Ok(None)` when the loader has nothing to contribute
///     for this call (cheaper than returning an empty string)
///   - never panic — a misbehaving loader degrades chat to an error
///     for every agent on the daemon
pub trait ContextLoader: Send + Sync {
    /// Stable identifier surfaced in the response's `context_used`
    /// list. Used as the key for `context_loaders.{include,exclude}`
    /// filters.
    fn name(&self) -> &str;

    /// Produce the loader's contribution for the given agent +
    /// session, or `None` when there is nothing to contribute.
    fn load(
        &self,
        agent_name: &str,
        session_id: &str,
    ) -> anyhow::Result<Option<String>>;
}

/// Register a `<agent>.chat` handler on the supplied registry for
/// every agent in `agents`. Idempotent: re-calling with an updated
/// registry replaces the previous handler set per agent.
///
/// The `_loaders` parameter is the seam for the pluggable
/// context-loader chain. Today the daemon passes an empty Vec; later
/// PRs construct loaders during boot and pass them in.
pub fn register(
    reg: &mut LocalAbilityRegistry,
    agents: &AgentRegistry,
    loaders: Arc<Vec<Arc<dyn ContextLoader>>>,
) {
    for (agent_name, entry) in &agents.agents {
        register_for_agent(reg, agent_name.clone(), entry.clone(), Arc::clone(&loaders));
    }
}

/// Register a single `<agent>.chat` handler. Factored out so a
/// future "an agent was added at runtime" path can call it directly
/// without re-walking the registry.
///
/// Mounts both the RPC and the Stream handler on the same name. The
/// dispatcher routes by `CallMode`: `Rpc` invocations land on the
/// RPC handler, `Stream`/Subscribe lands on the stream handler.
/// Sharing the ability name across both modes is the point — a
/// caller chooses how it wants to consume chat (one-shot vs framed),
/// not which "kind of chat" it is calling.
pub fn register_for_agent(
    reg: &mut LocalAbilityRegistry,
    agent_name: String,
    entry: AgentEntry,
    loaders: Arc<Vec<Arc<dyn ContextLoader>>>,
) {
    let ability = format!("{agent_name}.{ABILITY_VERB}");

    // RPC: the legacy synchronous one-shot path.
    let rpc_agent = agent_name.clone();
    let rpc_entry = entry.clone();
    let rpc_loaders = Arc::clone(&loaders);
    reg.register_rpc(
        &ability,
        Arc::new(move |args: Value| handler(&rpc_agent, &rpc_entry, &rpc_loaders, args)),
    );

    // Stream: emit framed events. v1 ships a Snapshot variant
    // (eagerly materialised list) because the underlying LLM driver
    // is synchronous; once the driver gains an async token stream
    // the handler upgrades to `Live(broadcast::Receiver)` without
    // changing the wire frame shape.
    reg.register_stream(
        &ability,
        Arc::new(move |args: Value| stream_handler(&agent_name, &entry, &loaders, args)),
    );
}

/// The chat ability's RPC handler. Parses args according to the
/// manifest schema, drives `dispatch::send_external`, and assembles
/// the new typed response.
///
/// Errors surface as `anyhow::Error` per `LocalRpcHandler`'s contract;
/// the dispatcher converts them to wire-level error frames.
fn handler(
    agent_name: &str,
    entry: &AgentEntry,
    loaders: &[Arc<dyn ContextLoader>],
    args: Value,
) -> anyhow::Result<Value> {
    let started = Instant::now();
    let parsed = ChatArgs::parse(&args)?;

    // The RPC entry point cannot return a stream. Surface the
    // mistake as a deterministic, actionable error rather than
    // silently dropping the flag.
    if parsed.stream {
        anyhow::bail!(
            "chat: `stream: true` is only valid via the subscribe entry point; \
             call subscribe with the same args instead of invoke"
        );
    }

    // Session id resolution. When the caller supplies one we echo
    // it verbatim; otherwise we mint a UUID-flavoured token. The
    // `chat-` prefix makes the id self-describing in timeline logs.
    let session_id = parsed
        .session_id
        .clone()
        .unwrap_or_else(|| format!("chat-{}", uuid_like()));

    // Context loader chain. Today `loaders` is empty in the daemon's
    // boot path, so this loop is a no-op; the trait + Vec are in
    // place so a later PR can register loaders without touching the
    // handler code.
    //
    // Loader filtering follows the same `mode/include/exclude`
    // semantics as `skills`:
    //   - mode=auto:    every loader runs unless it is in `exclude`
    //   - mode=none:    no loader runs (skip the loop entirely)
    //   - mode=explicit: only loaders whose name is in `include`
    //                    AND not in `exclude` run
    let mut context_used: Vec<Value> = Vec::new();
    let mut loaded_chunks: Vec<String> = Vec::new();
    if !matches!(parsed.context_loaders.mode, SelectionMode::None) {
        for loader in loaders {
            if !parsed.context_loaders.is_selected(loader.name()) {
                continue;
            }
            match loader.load(agent_name, &session_id) {
                Ok(Some(chunk)) => {
                    let bytes = chunk.len();
                    loaded_chunks.push(chunk);
                    context_used.push(json!({
                        "loader": loader.name(),
                        "bytes": bytes,
                    }));
                }
                Ok(None) => {} // loader had nothing to contribute
                Err(e) => {
                    // One bad loader must not poison the rest of the
                    // chain. Surface the failure in `context_used`
                    // with a `bytes: 0` entry so the caller sees the
                    // attempt was made.
                    eprintln!(
                        "chat[{agent_name}]: context loader {:?} failed: {e}",
                        loader.name()
                    );
                    context_used.push(json!({
                        "loader": loader.name(),
                        "bytes": 0,
                        "error": format!("{e}"),
                    }));
                }
            }
        }
    }

    // Skills enumeration. `skills_loaded` reports what would be
    // exposed to the LLM as tools for this call. Today the chat
    // handler does NOT yet inject these into the driver's tool list
    // — the driver adapters need a corresponding tools surface,
    // which is a separate PR. Surfacing the names anyway lets a
    // caller verify their `skills.{include,exclude}` filter works
    // as expected before the wiring lands.
    let skills_loaded = enumerate_skills(agent_name, entry, &parsed.skills);

    // Compose the literal context. Loader output goes first, then
    // the caller's `context` arg, separated by blank lines so the
    // LLM sees a coherent block. An empty composition yields `None`
    // so `compose_prompt` does not insert a useless context wrapper.
    let composed_context: Option<String> = match (loaded_chunks.is_empty(), &parsed.context) {
        (true, None) => None,
        (true, Some(c)) => Some(c.clone()),
        (false, None) => Some(loaded_chunks.join("\n\n")),
        (false, Some(c)) => Some(format!("{}\n\n{c}", loaded_chunks.join("\n\n"))),
    };

    // The dispatch call. `send_external_with_overrides` is the
    // overrides-aware variant — when `parsed.driver` is the default
    // it behaves identically to `send_external`; when the caller set
    // `driver.model` (or temperature / max_tokens, see warn-once in
    // dispatch.rs) those values flow into model resolution.
    //
    // Synchronous (subprocess + wait); when invoked from a tokio
    // worker thread we yield the worker via `block_in_place` to
    // avoid stalling other tasks. Mirrors the same pattern the
    // pre-refactor Kernel::dispatch_agent_chat used.
    let driver_overrides = Some(&parsed.driver);
    let response_result = if tokio::runtime::Handle::try_current().is_ok() {
        tokio::task::block_in_place(|| {
            crate::runtime::dispatch::send_external_with_overrides(
                agent_name,
                entry,
                &parsed.prompt,
                composed_context.as_deref(),
                driver_overrides,
            )
        })
    } else {
        crate::runtime::dispatch::send_external_with_overrides(
            agent_name,
            entry,
            &parsed.prompt,
            composed_context.as_deref(),
            driver_overrides,
        )
    };

    let resp = response_result?;
    let elapsed_ms = started.elapsed().as_millis() as u64;
    let usage = resp.usage.as_ref().map(|u| {
        json!({
            "input_tokens": u.input_tokens,
            "output_tokens": u.output_tokens,
            "model": resp.model.clone(),
        })
    });

    // tool_calls comes from the driver layer's tool-use observability
    // (see runtime::drivers::claude_code::ToolCallRecord). Codex
    // adapters return an empty Vec so the field is `[]` for codex
    // agents — present for schema stability, just empty.
    let tool_calls_json: Vec<Value> = resp
        .tool_calls
        .iter()
        .map(|tc| {
            json!({
                "ability": tc.ability,
                "args": tc.args,
                // elapsed_ms per-call is not yet captured by the
                // driver; the schema lists it as required so emit a
                // 0 placeholder rather than dropping the field. A
                // future driver upgrade can populate per-call timing.
                "elapsed_ms": 0,
            })
        })
        .collect();

    Ok(json!({
        "session_id": session_id,
        "reply": resp.content,
        "skills_loaded": skills_loaded,
        "tool_calls": tool_calls_json,
        "context_used": Value::Array(context_used),
        "usage": usage.unwrap_or(Value::Null),
        "elapsed_ms": elapsed_ms,
    }))
}

/// Stream-mode chat handler. Same parsing + dispatch as the RPC
/// handler, but the response is materialised as a list of typed
/// frames the IPC server emits in order.
///
/// Frame shapes (typed via the `type` discriminator):
///
///   `{"type": "session", "session_id": "..."}`
///       Always the first frame. Lets a subscriber correlate
///       subsequent `delta`/`done` frames back to the session id
///       even when the caller did not supply one.
///
///   `{"type": "loaded", "skills_loaded": [...], "context_used": [...]}`
///       Sent after skill enumeration + context loading complete,
///       before the LLM is invoked. Optional in v1 (absent when
///       both lists would be empty) but always present when
///       either has content.
///
///   `{"type": "done", "reply": "...", "tool_calls": [...], "context_used": [...], "usage": {...}, "elapsed_ms": N, "session_id": "..."}`
///       Terminal happy-path frame. Carries the same payload the
///       RPC handler returns, so a subscriber that only reads the
///       last frame is equivalent to an RPC caller.
///
///   `{"type": "error", "message": "...", "session_id": "..."}`
///       Terminal error frame. Mutually exclusive with `done`.
///
/// `delta` and `tool_call_*` frames are reserved for when the
/// driver gains async token streaming and live tool-call hooks;
/// today's synchronous driver cannot emit them, so the handler
/// jumps from `loaded` straight to `done`.
fn stream_handler(
    agent_name: &str,
    entry: &AgentEntry,
    loaders: &[Arc<dyn ContextLoader>],
    args: Value,
) -> anyhow::Result<StreamSource> {
    // Run the same body as the RPC handler. We deliberately do NOT
    // share the RPC code path because the RPC handler returns the
    // structured response directly while the stream handler must
    // wrap it in framed events. Both call dispatch::send_external
    // through the same shape, so behaviour stays in sync.
    let started = Instant::now();
    let parsed = ChatArgs::parse(&args)?;
    // The stream entry point cannot honour `stream: false` either —
    // the caller picked the subscribe RPC by entering this function
    // at all. Document the redundancy for the operator who reads
    // the timeline; do not bail.
    let session_id = parsed
        .session_id
        .clone()
        .unwrap_or_else(|| format!("chat-{}", uuid_like()));

    let mut frames: Vec<Value> = Vec::with_capacity(3);
    frames.push(json!({
        "type": "session",
        "session_id": session_id,
    }));

    // Context loaders + skills enumeration — same logic as the RPC
    // handler, but the result goes into a `loaded` frame instead of
    // a return-value field.
    let mut context_used: Vec<Value> = Vec::new();
    let mut loaded_chunks: Vec<String> = Vec::new();
    if !matches!(parsed.context_loaders.mode, SelectionMode::None) {
        for loader in loaders {
            if !parsed.context_loaders.is_selected(loader.name()) {
                continue;
            }
            match loader.load(agent_name, &session_id) {
                Ok(Some(chunk)) => {
                    let bytes = chunk.len();
                    loaded_chunks.push(chunk);
                    context_used.push(json!({
                        "loader": loader.name(),
                        "bytes": bytes,
                    }));
                }
                Ok(None) => {}
                Err(e) => {
                    eprintln!(
                        "chat[{agent_name}] (stream): context loader {:?} failed: {e}",
                        loader.name()
                    );
                    context_used.push(json!({
                        "loader": loader.name(),
                        "bytes": 0,
                        "error": format!("{e}"),
                    }));
                }
            }
        }
    }
    let skills_loaded = enumerate_skills(agent_name, entry, &parsed.skills);
    if !skills_loaded.is_empty() || !context_used.is_empty() {
        frames.push(json!({
            "type": "loaded",
            "skills_loaded": skills_loaded.clone(),
            "context_used": context_used.clone(),
        }));
    }

    let composed_context: Option<String> = match (loaded_chunks.is_empty(), &parsed.context) {
        (true, None) => None,
        (true, Some(c)) => Some(c.clone()),
        (false, None) => Some(loaded_chunks.join("\n\n")),
        (false, Some(c)) => Some(format!("{}\n\n{c}", loaded_chunks.join("\n\n"))),
    };

    // The dispatch call. Same overrides-aware variant as the RPC
    // handler so a streaming chat call honors `driver.model` too.
    // Non-tokio context here because stream handlers are constructed
    // inside the synchronous registry path; if a future async
    // dispatcher lands, the same block_in_place dance from the RPC
    // handler applies.
    let driver_overrides = Some(&parsed.driver);
    let response_result = if tokio::runtime::Handle::try_current().is_ok() {
        tokio::task::block_in_place(|| {
            crate::runtime::dispatch::send_external_with_overrides(
                agent_name,
                entry,
                &parsed.prompt,
                composed_context.as_deref(),
                driver_overrides,
            )
        })
    } else {
        crate::runtime::dispatch::send_external_with_overrides(
            agent_name,
            entry,
            &parsed.prompt,
            composed_context.as_deref(),
            driver_overrides,
        )
    };

    match response_result {
        Ok(resp) => {
            let elapsed_ms = started.elapsed().as_millis() as u64;
            let usage = resp.usage.as_ref().map(|u| {
                json!({
                    "input_tokens": u.input_tokens,
                    "output_tokens": u.output_tokens,
                    "model": resp.model.clone(),
                })
            });
            // Same projection as the RPC handler — see comment there
            // for why elapsed_ms per-call is 0 today.
            let tool_calls_json: Vec<Value> = resp
                .tool_calls
                .iter()
                .map(|tc| {
                    json!({
                        "ability": tc.ability,
                        "args": tc.args,
                        "elapsed_ms": 0,
                    })
                })
                .collect();
            frames.push(json!({
                "type": "done",
                "session_id": session_id,
                "reply": resp.content,
                "skills_loaded": skills_loaded,
                "tool_calls": tool_calls_json,
                "context_used": context_used,
                "usage": usage.unwrap_or(Value::Null),
                "elapsed_ms": elapsed_ms,
            }));
        }
        Err(e) => {
            frames.push(json!({
                "type": "error",
                "session_id": session_id,
                "message": format!("{e}"),
            }));
        }
    }

    Ok(StreamSource::Snapshot(frames))
}

/// Build the list of ability names that would be exposed as tools to
/// the LLM under the requested `skills` filter. Pulls the agent's
/// abilities from the same enumerator the rest of the system uses
/// (`runtime::abilities::abilities_for`) so an operator's
/// hand-edited manifest is reflected here too.
///
/// The `<agent>.chat` ability itself is never exposed as a tool to
/// the LLM (an agent calling its own chat would be infinite-recursion
/// bait); it is filtered out before any include/exclude rules apply.
fn enumerate_skills(
    agent_name: &str,
    entry: &AgentEntry,
    selection: &Selection,
) -> Vec<String> {
    if matches!(selection.mode, SelectionMode::None) {
        return Vec::new();
    }
    let self_chat = format!("{agent_name}.{ABILITY_VERB}");
    let candidates: Vec<String> =
        crate::runtime::abilities::abilities_for(agent_name, entry)
            .into_iter()
            .map(|s| s.name().to_string())
            .filter(|n| n != &self_chat)
            .collect();
    candidates
        .into_iter()
        .filter(|name| selection.is_selected(name))
        .collect()
}

// ── Argument parsing ────────────────────────────────────────────────────────

/// Typed view of the chat handler's input arguments. Mirrors the
/// JSON Schema in `default_chat_manifest()` 1:1; if the schema
/// grows a field, this struct gains a sibling. Keeping the parser in
/// one place lets `handler()` stay focused on dispatch logic and
/// makes "what fields does chat accept" introspectable from one
/// import.
#[derive(Debug, Clone)]
struct ChatArgs {
    prompt: String,
    context: Option<String>,
    session_id: Option<String>,
    skills: Selection,
    context_loaders: Selection,
    driver: DriverOverrides,
    stream: bool,
}

impl ChatArgs {
    fn parse(args: &Value) -> anyhow::Result<Self> {
        let obj = args
            .as_object()
            .ok_or_else(|| anyhow::anyhow!("chat: arguments must be a JSON object"))?;
        let prompt = obj
            .get("prompt")
            .and_then(Value::as_str)
            .ok_or_else(|| anyhow::anyhow!("chat: `prompt` (string) required"))?
            .to_string();
        if prompt.is_empty() {
            anyhow::bail!("chat: `prompt` must not be empty");
        }
        let context = obj.get("context").and_then(Value::as_str).map(str::to_string);
        let session_id = obj
            .get("session_id")
            .and_then(Value::as_str)
            .map(str::to_string);
        let skills = obj
            .get("skills")
            .map(Selection::parse)
            .transpose()?
            .unwrap_or_default();
        let context_loaders = obj
            .get("context_loaders")
            .map(Selection::parse)
            .transpose()?
            .unwrap_or_default();
        let driver = obj
            .get("driver")
            .map(parse_driver_overrides)
            .transpose()?
            .unwrap_or_default();
        let stream = obj.get("stream").and_then(Value::as_bool).unwrap_or(false);
        Ok(Self {
            prompt,
            context,
            session_id,
            skills,
            context_loaders,
            driver,
            stream,
        })
    }
}

/// Selection mode shared by `skills` and `context_loaders`. The
/// duplicated structure is intentional: callers reason about each
/// independently and reusing the type makes "what does include mean
/// here" an obvious cross-reference.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum SelectionMode {
    Auto,
    None,
    Explicit,
}

impl Default for SelectionMode {
    fn default() -> Self {
        Self::Auto
    }
}

#[derive(Debug, Clone, Default)]
struct Selection {
    mode: SelectionMode,
    include: Vec<String>,
    exclude: Vec<String>,
}

impl Selection {
    fn parse(value: &Value) -> anyhow::Result<Self> {
        let obj = value
            .as_object()
            .ok_or_else(|| anyhow::anyhow!("chat: skills/context_loaders must be an object"))?;
        let mode = match obj.get("mode").and_then(Value::as_str) {
            None => SelectionMode::Auto,
            Some("auto") => SelectionMode::Auto,
            Some("none") => SelectionMode::None,
            Some("explicit") => SelectionMode::Explicit,
            Some(other) => anyhow::bail!(
                "chat: invalid mode {other:?}; expected one of \"auto\", \"none\", \"explicit\""
            ),
        };
        let include = string_array(obj.get("include"), "include")?;
        let exclude = string_array(obj.get("exclude"), "exclude")?;
        Ok(Self {
            mode,
            include,
            exclude,
        })
    }

    /// Decide whether `name` survives the filter. The semantics:
    ///
    ///   - mode=auto:     selected unless in exclude
    ///   - mode=none:     never selected
    ///   - mode=explicit: selected only if in include AND not in exclude
    fn is_selected(&self, name: &str) -> bool {
        if self.exclude.iter().any(|e| e == name) {
            return false;
        }
        match self.mode {
            SelectionMode::Auto => true,
            SelectionMode::None => false,
            SelectionMode::Explicit => self.include.iter().any(|i| i == name),
        }
    }
}

/// Parse the chat ability's `driver` sub-object into the shared
/// `dispatch::DriverOverrides` type. Kept as a free function (not an
/// inherent impl) because `DriverOverrides` is a foreign type to
/// this module — Rust's coherence rules require either-or.
///
/// `model` is honored by the dispatch layer today; `temperature` and
/// `max_tokens` are accepted by the schema and recorded but not
/// piped through (the v1 claude-code / codex CLI drivers do not
/// expose either knob — see warn_unhonored_driver_knobs_once in
/// dispatch.rs). Validation still happens here so a malformed
/// `temperature: "hot"` surfaces at the API boundary instead of
/// dispatch time.
fn parse_driver_overrides(value: &Value) -> anyhow::Result<DriverOverrides> {
    let obj = value
        .as_object()
        .ok_or_else(|| anyhow::anyhow!("chat: `driver` must be an object"))?;
    let model = obj.get("model").and_then(Value::as_str).map(str::to_string);
    let temperature = match obj.get("temperature") {
        None => None,
        Some(v) => Some(
            v.as_f64()
                .ok_or_else(|| anyhow::anyhow!("chat: driver.temperature must be a number"))?,
        ),
    };
    let max_tokens = match obj.get("max_tokens") {
        None => None,
        Some(v) => Some(
            v.as_u64()
                .and_then(|n| u32::try_from(n).ok())
                .ok_or_else(|| {
                    anyhow::anyhow!("chat: driver.max_tokens must be a positive integer")
                })?,
        ),
    };
    Ok(DriverOverrides {
        model,
        temperature,
        max_tokens,
    })
}

/// Parse a JSON array of strings, returning an empty Vec when absent.
/// Surfaces a clean error when the value is present but the wrong
/// shape — silently dropping a typo is what makes
/// `additionalProperties: false` valuable in the schema; this parser
/// continues that contract on the array side.
fn string_array(value: Option<&Value>, field: &str) -> anyhow::Result<Vec<String>> {
    match value {
        None => Ok(Vec::new()),
        Some(Value::Array(items)) => {
            let mut out = Vec::with_capacity(items.len());
            for item in items {
                let s = item.as_str().ok_or_else(|| {
                    anyhow::anyhow!("chat: every entry in {field} must be a string")
                })?;
                out.push(s.to_string());
            }
            Ok(out)
        }
        Some(_) => anyhow::bail!("chat: {field} must be an array of strings"),
    }
}

/// Mint a UUID-shaped session id without pulling in the `uuid` crate
/// just for one helper. Mixes the current nanos with a process-local
/// counter so two concurrent calls inside the same nanosecond still
/// produce distinct ids.
fn uuid_like() -> String {
    use std::sync::atomic::{AtomicU64, Ordering};
    static COUNTER: AtomicU64 = AtomicU64::new(0);
    let nanos = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_nanos())
        .unwrap_or(0);
    let counter = COUNTER.fetch_add(1, Ordering::Relaxed);
    format!("{nanos:032x}-{counter:016x}")
}

#[cfg(test)]
mod tests {
    //! Tests cover the argument-parsing surface (the deterministic,
    //! pure portion of the handler) and the registration shape.
    //! Tests that exercise actual LLM dispatch live under integration
    //! tests in `tests/chat_ability_*.rs` because they need real
    //! agent directories and would otherwise spawn subprocesses.

    use super::*;
    use crate::registry::agents::{AgentRegistry, AgentType};

    fn entry() -> AgentEntry {
        AgentEntry::new(AgentType::ClaudeCode, None)
    }

    #[test]
    fn register_mounts_one_handler_per_agent() {
        let mut reg = LocalAbilityRegistry::new();
        let mut agents = AgentRegistry::default();
        agents.agents.insert("alice".into(), entry());
        agents.agents.insert("bob".into(), entry());
        register(&mut reg, &agents, Arc::new(Vec::new()));
        assert!(reg.get_rpc("alice.chat").is_some());
        assert!(reg.get_rpc("bob.chat").is_some());
        // Stream handler registered too — same name, different mode.
        assert!(reg.get_stream("alice.chat").is_some());
        assert!(reg.get_stream("bob.chat").is_some());
        // No collateral registrations.
        assert!(reg.get_rpc("alice.voice").is_none());
        assert!(reg.get_rpc("system.chat").is_none());
    }

    #[test]
    fn parse_accepts_legacy_prompt_only_args() {
        let args = ChatArgs::parse(&json!({"prompt": "hi"})).unwrap();
        assert_eq!(args.prompt, "hi");
        assert!(args.context.is_none());
        assert!(args.session_id.is_none());
        assert!(!args.stream);
        // Defaults: skills auto, context_loaders auto.
        assert_eq!(args.skills.mode, SelectionMode::Auto);
        assert_eq!(args.context_loaders.mode, SelectionMode::Auto);
    }

    #[test]
    fn parse_accepts_legacy_prompt_and_context() {
        let args = ChatArgs::parse(&json!({
            "prompt": "hi",
            "context": "you are helpful"
        }))
        .unwrap();
        assert_eq!(args.context.as_deref(), Some("you are helpful"));
    }

    #[test]
    fn parse_rejects_missing_prompt() {
        let err = ChatArgs::parse(&json!({})).unwrap_err();
        assert!(format!("{err}").contains("prompt"));
    }

    #[test]
    fn parse_rejects_empty_prompt() {
        let err = ChatArgs::parse(&json!({"prompt": ""})).unwrap_err();
        assert!(format!("{err}").contains("empty"));
    }

    #[test]
    fn parse_rejects_non_object_args() {
        let err = ChatArgs::parse(&json!(["not", "an", "object"])).unwrap_err();
        assert!(format!("{err}").contains("object"));
    }

    #[test]
    fn parse_full_extended_schema_round_trip() {
        let args = ChatArgs::parse(&json!({
            "prompt": "hi",
            "context": "bg",
            "session_id": "s-1",
            "skills": {"mode": "explicit", "include": ["alice.voice"], "exclude": ["alice.exec"]},
            "context_loaders": {"mode": "none"},
            "driver": {"model": "claude-opus-4-7", "temperature": 0.3, "max_tokens": 1024},
            "stream": false,
        }))
        .unwrap();
        assert_eq!(args.session_id.as_deref(), Some("s-1"));
        assert_eq!(args.skills.mode, SelectionMode::Explicit);
        assert_eq!(args.skills.include, vec!["alice.voice"]);
        assert_eq!(args.skills.exclude, vec!["alice.exec"]);
        assert_eq!(args.context_loaders.mode, SelectionMode::None);
        assert_eq!(args.driver.model.as_deref(), Some("claude-opus-4-7"));
        assert_eq!(args.driver.temperature, Some(0.3));
        assert_eq!(args.driver.max_tokens, Some(1024));
    }

    #[test]
    fn parse_rejects_unknown_skills_mode() {
        let err = ChatArgs::parse(&json!({
            "prompt": "hi",
            "skills": {"mode": "wildcard"}
        }))
        .unwrap_err();
        assert!(format!("{err}").contains("mode"));
    }

    #[test]
    fn parse_rejects_non_string_in_include() {
        let err = ChatArgs::parse(&json!({
            "prompt": "hi",
            "skills": {"include": [123]}
        }))
        .unwrap_err();
        assert!(format!("{err}").contains("include"));
    }

    #[test]
    fn parse_rejects_non_numeric_temperature() {
        let err = ChatArgs::parse(&json!({
            "prompt": "hi",
            "driver": {"temperature": "hot"}
        }))
        .unwrap_err();
        assert!(format!("{err}").contains("temperature"));
    }

    #[test]
    fn handler_rejects_stream_true_under_rpc() {
        // Spirit of "no silent surprises": flipping `stream: true`
        // on the RPC entry point must surface as an error, not be
        // silently ignored.
        let entry = entry();
        let result = handler(
            "alice",
            &entry,
            &[],
            json!({"prompt": "hi", "stream": true}),
        );
        let err = result.unwrap_err();
        assert!(format!("{err}").contains("subscribe"));
    }

    #[test]
    fn selection_auto_excludes_blacklisted() {
        let s = Selection {
            mode: SelectionMode::Auto,
            include: vec![],
            exclude: vec!["alice.exec".into()],
        };
        assert!(s.is_selected("alice.voice"));
        assert!(!s.is_selected("alice.exec"));
    }

    #[test]
    fn selection_none_rejects_everything() {
        let s = Selection {
            mode: SelectionMode::None,
            include: vec!["alice.voice".into()],
            exclude: vec![],
        };
        assert!(!s.is_selected("alice.voice"));
    }

    #[test]
    fn selection_explicit_only_listed() {
        let s = Selection {
            mode: SelectionMode::Explicit,
            include: vec!["alice.voice".into()],
            exclude: vec![],
        };
        assert!(s.is_selected("alice.voice"));
        assert!(!s.is_selected("alice.exec"));
    }

    #[test]
    fn selection_explicit_with_exclude_filters() {
        let s = Selection {
            mode: SelectionMode::Explicit,
            include: vec!["alice.voice".into(), "alice.exec".into()],
            exclude: vec!["alice.exec".into()],
        };
        assert!(s.is_selected("alice.voice"));
        assert!(!s.is_selected("alice.exec"));
    }

    #[test]
    fn enumerate_skills_excludes_self_chat() {
        // The chat ability itself must never appear in its own
        // skills list — that would invite infinite recursion when
        // the LLM picks "chat" as a tool.
        let entry = entry();
        // Use an in-memory entry (no root_path) so abilities_for
        // returns the synthesized chat fallback only.
        let listed = enumerate_skills(
            "alice",
            &entry,
            &Selection {
                mode: SelectionMode::Auto,
                include: vec![],
                exclude: vec![],
            },
        );
        assert!(!listed.iter().any(|n| n == "alice.chat"));
    }

    #[test]
    fn uuid_like_produces_distinct_ids_under_contention() {
        // The id minter is called from the handler at every chat
        // invocation; two calls in the same nanosecond must still
        // disambiguate via the counter.
        let a = uuid_like();
        let b = uuid_like();
        assert_ne!(a, b);
    }

    // ── Phase 4 unification: Kernel::invoke routes through registry ────
    //
    // The test below replaces the chat handler with a fake one that
    // bumps a counter, then calls Kernel::invoke with `<agent>.chat`.
    // Asserting the counter advanced is what proves: (a) the kernel
    // looked up the ability in the dispatcher's registry rather than
    // running a hardcoded `<agent>.chat` branch, and (b) the registered
    // handler is the one that fires — there is no second code path
    // hiding inside Kernel.

    #[test]
    fn kernel_invoke_routes_chat_through_registered_handler() {
        use crate::runtime::ability_dispatch::{AbilityDispatcher, LocalAbilityRegistry};
        use crate::runtime::gateway::NoopGateway;
        use crate::runtime::invocation::{CausalContext, Invocation};
        use crate::runtime::invocation_target::CallMode;
        use crate::runtime::kernel::Kernel;
        use crate::runtime::kernel_api::KernelApi;
        use std::sync::atomic::{AtomicUsize, Ordering};

        let _g = crate::facade::cli::test_support::HomeGuard::new();

        // Fake chat handler — increments a counter on every call so we
        // can prove the registered handler is the one that fired.
        let counter = Arc::new(AtomicUsize::new(0));
        let counter_for_handler = Arc::clone(&counter);
        let mut reg = LocalAbilityRegistry::new();
        reg.register_rpc(
            "alice.chat",
            Arc::new(move |_args: Value| {
                counter_for_handler.fetch_add(1, Ordering::SeqCst);
                Ok(json!({"reply": "fake"}))
            }),
        );
        let dispatcher = Arc::new(AbilityDispatcher::new(Arc::new(reg), Arc::new(NoopGateway)));

        let kernel = Kernel::new(Arc::new(NoopGateway));
        kernel.set_dispatcher(dispatcher);

        let _ = CallMode::Rpc; // keep the import live in case future
                                 // versions of the test branch on mode.

        let inv = Invocation {
            caller: "easynet://nodes/a".into(),
            callee: "easynet://nodes/a".into(),
            ability: "alice.chat".into(),
            subject: "easynet://nodes/a".into(),
            nonce_hex: "aa".repeat(16),
            causal_context: CausalContext::Null,
            args: json!({"prompt": "hi"}),
            caller_signature: None,
        };
        let receipt = kernel.invoke(inv).expect("invoke ok");
        assert!(matches!(
            receipt.terminal,
            crate::runtime::invocation::TerminalState::Succeeded
        ));
        assert_eq!(
            counter.load(Ordering::SeqCst),
            1,
            "kernel must have called the registered <agent>.chat handler exactly once"
        );
    }
}
