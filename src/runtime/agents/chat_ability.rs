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
// Unlike `observe.health` (one handler globally) or `fleet.list_sessions`
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
//     send_external_with_overrides. `driver.temperature` and
//     `driver.max_tokens` are rejected at parse time (no v1 CLI
//     driver exposes either knob; see parse_driver_overrides).
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

    // For every OTHER ability the agent declares via its
    // workspace `<root>/abilities/*.toml`, register a fallback
    // handler that dispatches back to this agent's chat with a
    // synthesised prompt instructing it to fulfill the named
    // ability with the given args. Without this, an agent could
    // declare abilities (which surface in MCP catalog and
    // skills_loaded) but the LLM running inside has no way to
    // invoke them: the dispatcher returns NOT_FOUND for every
    // <agent>.<ability> name that isn't `<agent>.chat`.
    //
    // The fallback handler is intentionally simple: \"act as
    // ability X with args Y\" — the agent's own CLAUDE.md /
    // SKILL.md define what fulfilling the ability means; this
    // function just routes the call.
    let other_abilities = crate::runtime::abilities::abilities_for(&agent_name, &entry);
    let chat_name = ability.clone();
    for spec in other_abilities {
        if spec.name() == chat_name {
            continue;
        }
        let ability_name = spec.name().to_string();
        let agent_for_handler = agent_name.clone();
        let entry_for_handler = entry.clone();
        let loaders_for_handler = Arc::clone(&loaders);
        let bare_ability = ability_name
            .strip_prefix(&format!("{agent_name}."))
            .unwrap_or(&ability_name)
            .to_string();
        reg.register_rpc(
            &ability_name,
            Arc::new(move |args: Value| {
                let prompt = format!(
                    "Fulfill your declared ability `{bare}` with the following arguments \
                     (JSON, may be empty object): {args}\n\n\
                     Reply with the ability's result as plain text — no preamble, no markdown \
                     fence, no commentary. If the arguments are invalid for this ability, \
                     reply with a single line starting with `error: `.",
                    bare = bare_ability,
                    args = serde_json::to_string(&args).unwrap_or_else(|_| "{}".to_string()),
                );
                let chat_args = serde_json::json!({
                    "prompt": prompt,
                    "stream": false,
                });
                handler(
                    &agent_for_handler,
                    &entry_for_handler,
                    &loaders_for_handler,
                    chat_args,
                )
                .map(|chat_resp| {
                    // Pull the reply text out of the chat
                    // response and return it as the ability's
                    // result. The MCP bridge expects the result
                    // value verbatim; wrapping in {result: ...}
                    // keeps it inspectable. Keep usage so the
                    // caller can see this WAS an LLM
                    // fulfillment, not a synchronous handler.
                    let reply = chat_resp
                        .get("reply")
                        .cloned()
                        .unwrap_or(serde_json::Value::Null);
                    let usage = chat_resp.get("usage").cloned().unwrap_or(serde_json::Value::Null);
                    let elapsed_ms = chat_resp
                        .get("elapsed_ms")
                        .cloned()
                        .unwrap_or(serde_json::Value::Null);
                    serde_json::json!({
                        "result": reply,
                        "fulfilled_by": "agent_chat",
                        "agent": agent_for_handler,
                        "usage": usage,
                        "elapsed_ms": elapsed_ms,
                    })
                })
            }),
        );
    }

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

    // Skills enumeration. `skills_loaded` reports what is exposed to
    // the LLM both as MCP tools (via the workspace `.mcp.json`) and
    // as a textual "Available skills" hint inside the context block.
    // The hint is what tells the LLM "you have a `voice` skill" —
    // without it the names live in the MCP tool list but the model
    // has no system-prompt-level reminder of which to reach for.
    let skill_specs = enumerate_skill_specs(agent_name, entry, &parsed.skills);
    let skills_loaded: Vec<String> = skill_specs.iter().map(|s| s.name().to_string()).collect();
    let skills_hint = format_skills_hint(&skill_specs);

    // Materialise attachments to a delimited block. Failures bail
    // loud — attachments are explicit input, not best-effort
    // context, so a missing path is the operator's bug to fix.
    let attachments_block = materialize_attachments(&parsed.attachments)?;

    // Compose the literal context. Order: skills hint, loader
    // output, attachments block, caller's explicit `context` arg —
    // each separated by blank lines so the LLM sees a coherent block.
    // An empty composition yields `None` so `compose_prompt` does not
    // insert a useless context wrapper.
    let composed_context: Option<String> = compose_chat_context(
        skills_hint.as_deref(),
        &loaded_chunks,
        attachments_block.as_deref(),
        parsed.context.as_deref(),
    );

    // The dispatch call. `send_external_with_overrides` is the
    // overrides-aware variant — when `parsed.driver` is the default
    // it behaves identically to `send_external`; when the caller set
    // `driver.model` that value flows into model resolution.
    // `driver.temperature` / `driver.max_tokens` cannot reach this
    // point (parse_driver_overrides rejects them).
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
/// Streaming topology
/// -------------------
/// The handler returns `StreamSource::SnapshotThenLive`:
///
///   * Snapshot half — `[session]` plus an optional `[loaded]` frame
///     when skills/context were enumerated. These are computable
///     synchronously without invoking the LLM, so the subscriber
///     receives them on the very first poll without waiting for the
///     subprocess to spawn.
///
///   * Live half — a `broadcast::Receiver<Value>` whose `Sender`
///     is held by a dedicated OS thread that runs the synchronous
///     dispatch path. When the LLM finishes the thread emits a
///     terminal frame (`done` on success, `error` on failure) and
///     drops the sender; the IPC frame-forwarder sees the channel
///     close and emits its own `Terminal` frame with `done`.
///
/// Why a dedicated thread (not tokio::spawn)
/// -----------------------------------------
/// `dispatch::send_external_with_overrides` is fully synchronous —
/// it spawns a child process and uses `Read`/`Write` blocking calls.
/// Putting it on a tokio worker would either monopolise the worker
/// for the duration of the LLM call or require `block_in_place`
/// inside an already-async producer. A standalone OS thread keeps
/// the cost predictable and matches the pattern the FFI layer uses
/// for its subscription callbacks (commit 94eba05).
///
/// `delta` and `tool_call_*` frames are reserved for when the
/// driver gains async token streaming. Today's sync driver cannot
/// emit them mid-flight; the topology above is the seam that lets
/// a future driver upgrade plug them in by passing the same
/// `Sender` into the driver's per-line callback.
fn stream_handler(
    agent_name: &str,
    entry: &AgentEntry,
    loaders: &[Arc<dyn ContextLoader>],
    args: Value,
) -> anyhow::Result<StreamSource> {
    use tokio::sync::broadcast;

    let started = Instant::now();
    let parsed = ChatArgs::parse(&args)?;
    let session_id = parsed
        .session_id
        .clone()
        .unwrap_or_else(|| format!("chat-{}", uuid_like()));

    // ── Snapshot half: session + loaded frames ─────────────────────────
    let mut snapshot: Vec<Value> = Vec::with_capacity(2);
    snapshot.push(json!({
        "type": "session",
        "session_id": session_id,
    }));

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
    let skill_specs = enumerate_skill_specs(agent_name, entry, &parsed.skills);
    let skills_loaded: Vec<String> = skill_specs.iter().map(|s| s.name().to_string()).collect();
    let skills_hint = format_skills_hint(&skill_specs);
    let attachments_block = materialize_attachments(&parsed.attachments)?;
    if !skills_loaded.is_empty() || !context_used.is_empty() {
        snapshot.push(json!({
            "type": "loaded",
            "skills_loaded": skills_loaded.clone(),
            "context_used": context_used.clone(),
        }));
    }

    let composed_context: Option<String> = compose_chat_context(
        skills_hint.as_deref(),
        &loaded_chunks,
        attachments_block.as_deref(),
        parsed.context.as_deref(),
    );

    // ── Live half: spawn dispatch thread, return receiver ──────────────
    //
    // Channel capacity is intentionally small (8) — chat dispatch
    // emits at most a handful of frames per turn (session + loaded
    // already in snapshot, then done|error). A larger buffer would
    // just delay the moment a slow subscriber's lag surfaces as a
    // `RecvError::Lagged`.
    let (tx, rx) = broadcast::channel::<Value>(8);

    // Move every cloneable into the thread closure. AgentEntry is
    // Clone (registry uses it that way). Skills_loaded/context_used
    // were computed above and are needed for the `done` frame.
    let agent_name_owned = agent_name.to_string();
    let entry_owned = entry.clone();
    let prompt_owned = parsed.prompt.clone();
    let driver_owned = parsed.driver.clone();
    let session_id_for_thread = session_id.clone();
    let skills_loaded_for_thread = skills_loaded;
    let context_used_for_thread = context_used;
    let composed_context_owned = composed_context;

    // Per-token progress forwarder. The driver invokes this
    // once per stdout line in stream-json mode; we wrap each
    // chunk in a `progress` frame and broadcast to subscribers.
    // Pre-fix the chat stream emitted only {session, loaded?,
    // done|error} — the audit conversation caught it. With
    // this callback the stream is now a real per-token stream.
    let tx_for_progress = tx.clone();
    let progress_callback: Arc<dyn Fn(serde_json::Value) + Send + Sync> =
        Arc::new(move |chunk: serde_json::Value| {
            let frame = json!({
                "type": "progress",
                "chunk": chunk,
            });
            // SendError when subscriber dropped. Same handling
            // as the terminal frame: discard silently — the IPC
            // forwarder already noticed the cancel.
            let _ = tx_for_progress.send(frame);
        });

    std::thread::Builder::new()
        .name(format!("chat-stream-{agent_name}"))
        .spawn(move || {
            let result = crate::runtime::dispatch::send_external_with_overrides_and_progress(
                &agent_name_owned,
                &entry_owned,
                &prompt_owned,
                composed_context_owned.as_deref(),
                Some(&driver_owned),
                Some(progress_callback),
            );
            let elapsed_ms = started.elapsed().as_millis() as u64;
            let frame = match result {
                Ok(resp) => {
                    let usage = resp.usage.as_ref().map(|u| {
                        json!({
                            "input_tokens": u.input_tokens,
                            "output_tokens": u.output_tokens,
                            "model": resp.model.clone(),
                        })
                    });
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
                    json!({
                        "type": "done",
                        "session_id": session_id_for_thread,
                        "reply": resp.content,
                        "skills_loaded": skills_loaded_for_thread,
                        "tool_calls": tool_calls_json,
                        "context_used": context_used_for_thread,
                        "usage": usage.unwrap_or(Value::Null),
                        "elapsed_ms": elapsed_ms,
                    })
                }
                Err(e) => json!({
                    "type": "error",
                    "session_id": session_id_for_thread,
                    "message": format!("{e}"),
                }),
            };
            // SendError when the receiver was dropped (subscriber
            // disconnected before the LLM finished). That is fine —
            // we just discard the terminal frame; the IPC server's
            // forwarder already noticed the cancel.
            let _ = tx.send(frame);
            // tx drops at end of scope → broadcast channel closes →
            // IPC forwarder emits Terminal{done}.
        })
        .map_err(|e| anyhow::anyhow!("chat stream: failed to spawn dispatch thread: {e}"))?;

    Ok(StreamSource::SnapshotThenLive(snapshot, rx))
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
    enumerate_skill_specs(agent_name, entry, selection)
        .into_iter()
        .map(|s| s.name().to_string())
        .collect()
}

/// Same selection logic as [`enumerate_skills`], but returns the full
/// `AgentAbilitySpec` so callers (specifically the system-prompt
/// hint builder) can read the description alongside the qualified
/// name without re-walking the registry.
fn enumerate_skill_specs(
    agent_name: &str,
    entry: &AgentEntry,
    selection: &Selection,
) -> Vec<crate::runtime::abilities::AgentAbilitySpec> {
    if matches!(selection.mode, SelectionMode::None) {
        return Vec::new();
    }
    let self_chat = format!("{agent_name}.{ABILITY_VERB}");
    crate::runtime::abilities::abilities_for(agent_name, entry)
        .into_iter()
        .filter(|s| s.name() != self_chat)
        .filter(|s| selection.is_selected(s.name()))
        .collect()
}

/// Build the system-prompt-style "Available skills" hint that we
/// prepend to the chat context. The block lists every selected
/// ability as `- <qualified-name> — <description>` so the LLM can
/// see the names + purposes alongside the rest of the context block.
///
/// Returns `None` when no skills are selected (so callers can
/// short-circuit and avoid emitting an empty section header).
///
/// The block is intentionally terse and stable — one line per skill,
/// description trimmed to a single line. The MCP tool surface still
/// owns the canonical schemas; this hint exists only so prompts
/// like "use your `voice` skill" resolve to a name the model has
/// actually seen in-context.
/// Glue together the four context fragments — skills hint, loader
/// output, attachments block, caller-supplied `context` — into a
/// single block, with blank-line separators so the LLM reads them as
/// discrete sections. Returns `None` when every fragment is empty so
/// the downstream `compose_prompt` skips the wrapper entirely.
fn compose_chat_context(
    skills_hint: Option<&str>,
    loaded_chunks: &[String],
    attachments_block: Option<&str>,
    caller_context: Option<&str>,
) -> Option<String> {
    let mut parts: Vec<String> = Vec::new();
    if let Some(h) = skills_hint {
        if !h.trim().is_empty() {
            parts.push(h.trim_end().to_string());
        }
    }
    if !loaded_chunks.is_empty() {
        parts.push(loaded_chunks.join("\n\n"));
    }
    if let Some(a) = attachments_block {
        if !a.trim().is_empty() {
            parts.push(a.trim_end().to_string());
        }
    }
    if let Some(c) = caller_context {
        if !c.trim().is_empty() {
            parts.push(c.to_string());
        }
    }
    if parts.is_empty() {
        None
    } else {
        Some(parts.join("\n\n"))
    }
}

/// Total byte budget (across all attachments in one chat call) for
/// the materialised file content embedded into the prompt context.
/// Picked at 1 MiB to match the order of magnitude of fs.read's
/// per-call cap; budgets a tens-of-pages context window without
/// risking a runaway prompt.
const ATTACHMENTS_BUDGET_BYTES: usize = 1024 * 1024;

/// Read every attachment off disk and assemble a single delimited
/// markdown block. Returns `Ok(None)` when the input list is empty
/// so callers skip the wrapper.
///
/// Failure modes (all loud — chat does not silently swallow these):
///   * any path on the fs.read blocked list (e.g. /dev/zero)
///   * file open/read I/O failure
///   * encoding=utf8 on a non-UTF-8 byte sequence
///   * accumulated bytes exceed `ATTACHMENTS_BUDGET_BYTES`
fn materialize_attachments(
    specs: &[AttachmentSpec],
) -> anyhow::Result<Option<String>> {
    if specs.is_empty() {
        return Ok(None);
    }
    use std::io::Read;
    let mut out = String::from("## Attachments\n\n");
    let mut budget = ATTACHMENTS_BUDGET_BYTES;
    for (idx, spec) in specs.iter().enumerate() {
        if super::fs_ability::is_blocked_read_path_for_chat(&spec.path) {
            anyhow::bail!(
                "chat: attachments[{idx}] {:?} is on the blocked-device path list",
                spec.path
            );
        }
        let path = std::path::Path::new(&spec.path);
        let metadata = std::fs::metadata(path).map_err(|e| {
            anyhow::anyhow!("chat: attachments[{idx}] stat {:?}: {e}", spec.path)
        })?;
        if metadata.len() as usize > budget {
            anyhow::bail!(
                "chat: attachments[{idx}] {:?} ({} bytes) would exceed the {} byte \
                 attachments budget",
                spec.path,
                metadata.len(),
                ATTACHMENTS_BUDGET_BYTES
            );
        }
        let mut file = std::fs::File::open(path).map_err(|e| {
            anyhow::anyhow!("chat: attachments[{idx}] open {:?}: {e}", spec.path)
        })?;
        // +1 over budget so an oversized file (e.g. one that grew
        // between stat and open) still fails loud rather than
        // truncating silently.
        let mut limited = file.by_ref().take(budget as u64 + 1);
        let mut bytes: Vec<u8> = Vec::with_capacity(metadata.len() as usize);
        limited
            .read_to_end(&mut bytes)
            .map_err(|e| anyhow::anyhow!("chat: attachments[{idx}] read {:?}: {e}", spec.path))?;
        if bytes.len() > budget {
            anyhow::bail!(
                "chat: attachments[{idx}] {:?} grew past the {} byte attachments budget \
                 mid-read",
                spec.path,
                ATTACHMENTS_BUDGET_BYTES
            );
        }
        budget = budget.saturating_sub(bytes.len());

        let body = match spec.encoding {
            AttachmentEncoding::Utf8 => {
                let text = std::str::from_utf8(&bytes).map_err(|_| {
                    anyhow::anyhow!(
                        "chat: attachments[{idx}] {:?} is not valid UTF-8; \
                         use encoding=\"base64\"",
                        spec.path
                    )
                })?;
                format!(
                    "<file path={:?} encoding=\"utf8\">\n{text}\n</file>\n",
                    spec.path
                )
            }
            AttachmentEncoding::Base64 => {
                let encoded = base64_encode(&bytes);
                format!(
                    "<file path={:?} encoding=\"base64\">\n{encoded}\n</file>\n",
                    spec.path
                )
            }
        };
        out.push_str(&body);
    }
    Ok(Some(out))
}

/// Minimal base64 encoder (standard alphabet, with padding). Lifted
/// here so chat_ability does not pull in a new dep just for the
/// attachments path; the alphabet + padding are stable enough to
/// inline. Mirrors RFC 4648 §4.
fn base64_encode(input: &[u8]) -> String {
    const ALPHABET: &[u8; 64] =
        b"ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghijklmnopqrstuvwxyz0123456789+/";
    let mut out = String::with_capacity(input.len().div_ceil(3) * 4);
    for chunk in input.chunks(3) {
        let b0 = chunk[0] as u32;
        let b1 = if chunk.len() > 1 { chunk[1] as u32 } else { 0 };
        let b2 = if chunk.len() > 2 { chunk[2] as u32 } else { 0 };
        let triple = (b0 << 16) | (b1 << 8) | b2;
        out.push(ALPHABET[((triple >> 18) & 0x3f) as usize] as char);
        out.push(ALPHABET[((triple >> 12) & 0x3f) as usize] as char);
        out.push(if chunk.len() > 1 {
            ALPHABET[((triple >> 6) & 0x3f) as usize] as char
        } else {
            '='
        });
        out.push(if chunk.len() > 2 {
            ALPHABET[(triple & 0x3f) as usize] as char
        } else {
            '='
        });
    }
    out
}

fn format_skills_hint(skills: &[crate::runtime::abilities::AgentAbilitySpec]) -> Option<String> {
    if skills.is_empty() {
        return None;
    }
    let mut out = String::from("## Available skills\n\n");
    out.push_str(
        "These abilities are exposed to you as MCP tools under the `easynet` server. \
         Call them by their qualified name when the user's request matches.\n\n",
    );
    for s in skills {
        let desc = s.description().lines().next().unwrap_or("").trim();
        if desc.is_empty() {
            out.push_str(&format!("- `{}`\n", s.name()));
        } else {
            out.push_str(&format!("- `{}` — {desc}\n", s.name()));
        }
    }
    Some(out)
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
    attachments: Vec<AttachmentSpec>,
}

#[derive(Debug, Clone)]
struct AttachmentSpec {
    path: String,
    encoding: AttachmentEncoding,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum AttachmentEncoding {
    Utf8,
    Base64,
}

impl Default for AttachmentEncoding {
    fn default() -> Self {
        Self::Utf8
    }
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
        let attachments = parse_attachments(obj.get("attachments"))?;
        Ok(Self {
            prompt,
            context,
            session_id,
            skills,
            context_loaders,
            driver,
            stream,
            attachments,
        })
    }
}

/// Parse the optional `attachments` array into typed AttachmentSpecs.
/// Absent/null → empty Vec; present-but-not-an-array → loud error so
/// the caller sees the typo at the API boundary.
fn parse_attachments(value: Option<&Value>) -> anyhow::Result<Vec<AttachmentSpec>> {
    let arr = match value {
        None | Some(Value::Null) => return Ok(Vec::new()),
        Some(Value::Array(items)) => items,
        Some(_) => anyhow::bail!("chat: `attachments` must be an array of objects"),
    };
    let mut out = Vec::with_capacity(arr.len());
    for (idx, item) in arr.iter().enumerate() {
        let obj = item.as_object().ok_or_else(|| {
            anyhow::anyhow!("chat: attachments[{idx}] must be an object")
        })?;
        let path = obj
            .get("path")
            .and_then(Value::as_str)
            .ok_or_else(|| {
                anyhow::anyhow!("chat: attachments[{idx}].path (string) required")
            })?
            .to_string();
        if path.is_empty() {
            anyhow::bail!("chat: attachments[{idx}].path must not be empty");
        }
        let encoding = match obj.get("encoding").and_then(Value::as_str) {
            None => AttachmentEncoding::default(),
            Some("utf8") => AttachmentEncoding::Utf8,
            Some("base64") => AttachmentEncoding::Base64,
            Some(other) => anyhow::bail!(
                "chat: attachments[{idx}].encoding must be \"utf8\" or \"base64\" (got {other:?})"
            ),
        };
        out.push(AttachmentSpec { path, encoding });
    }
    Ok(out)
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
/// `model` is honored by the dispatch layer today.
///
/// `temperature` and `max_tokens` are part of the chat schema for
/// forward compatibility but no v1 driver can pipe them through:
/// neither the claude-code CLI nor the codex CLI exposes those
/// knobs, and silently accepting them would leave a caller who
/// wrote `temperature: 0.3` thinking the value took effect. We
/// therefore validate the shape here AND reject loudly at the API
/// boundary — the alternative (warn-once on stderr) was tried in
/// the previous slice and audit caught it as a silent surprise.
/// A future driver that supports these knobs can drop the rejection
/// here and forward through DriverOverrides without re-shaping the
/// dispatch surface.
fn parse_driver_overrides(value: &Value) -> anyhow::Result<DriverOverrides> {
    let obj = value
        .as_object()
        .ok_or_else(|| anyhow::anyhow!("chat: `driver` must be an object"))?;
    let model = obj.get("model").and_then(Value::as_str).map(str::to_string);
    // Validate temperature shape first (so a malformed value still
    // surfaces a precise error) before the unsupported-knob rejection.
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
    let mut unsupported: Vec<&'static str> = Vec::new();
    if temperature.is_some() {
        unsupported.push("temperature");
    }
    if max_tokens.is_some() {
        unsupported.push("max_tokens");
    }
    if !unsupported.is_empty() {
        anyhow::bail!(
            "chat: driver.{} not supported by the v1 claude-code / codex CLI drivers \
             (the underlying CLIs do not expose this knob). Remove the field, or set \
             it via the agent's CLI-side configuration if that runtime supports it.",
            unsupported.join(", driver."),
        );
    }
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
    fn stream_handler_returns_snapshot_then_live_with_session_frame_first() {
        // Topology pin: the stream handler must return a
        // SnapshotThenLive whose snapshot starts with the `session`
        // frame. A subscriber connecting before the LLM finishes
        // sees this frame on the very first poll, not after the
        // LLM completes (which would defeat the streaming refactor).
        //
        // We pass an entry with no on-disk root so the spawned
        // dispatch thread will fail fast (no mission context, no
        // workspace). The Live half then carries an `error` frame
        // — fine for this test, which only asserts the snapshot.
        let entry = entry();
        let result = stream_handler(
            "alice",
            &entry,
            &[],
            json!({"prompt": "hi"}),
        );
        let source = result.expect("snapshot construction must succeed even if dispatch will fail");
        match source {
            StreamSource::SnapshotThenLive(snapshot, _rx) => {
                assert!(!snapshot.is_empty(), "snapshot must not be empty");
                let first = &snapshot[0];
                assert_eq!(first.get("type").and_then(Value::as_str), Some("session"));
                assert!(first.get("session_id").is_some());
            }
            other => panic!("expected SnapshotThenLive, got {other:?}"),
        }
    }

    #[test]
    fn stream_handler_snapshot_includes_loaded_frame_when_skills_or_context_present() {
        // The `loaded` frame is optional — emitted only when there
        // is content to surface. With no agent skills (in-memory
        // entry → fallback chat-only) and no loaders, the snapshot
        // is just `[session]`. Pin that here so a future patch
        // that always emits `loaded` (which would mislead the
        // subscriber when both lists are empty) trips the test.
        let entry = entry();
        let source = stream_handler("alice", &entry, &[], json!({"prompt": "hi"})).unwrap();
        match source {
            StreamSource::SnapshotThenLive(snapshot, _rx) => {
                assert_eq!(
                    snapshot.len(),
                    1,
                    "no skills + no loaders → snapshot is just [session]; got {snapshot:?}"
                );
            }
            other => panic!("expected SnapshotThenLive, got {other:?}"),
        }
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
        // The extended schema accepts model, skills, context_loaders,
        // session_id, stream — temperature/max_tokens are rejected at
        // parse time (no v1 driver supports them; see the dedicated
        // rejection tests below).
        let args = ChatArgs::parse(&json!({
            "prompt": "hi",
            "context": "bg",
            "session_id": "s-1",
            "skills": {"mode": "explicit", "include": ["alice.voice"], "exclude": ["alice.exec"]},
            "context_loaders": {"mode": "none"},
            "driver": {"model": "claude-opus-4-7"},
            "stream": false,
        }))
        .unwrap();
        assert_eq!(args.session_id.as_deref(), Some("s-1"));
        assert_eq!(args.skills.mode, SelectionMode::Explicit);
        assert_eq!(args.skills.include, vec!["alice.voice"]);
        assert_eq!(args.skills.exclude, vec!["alice.exec"]);
        assert_eq!(args.context_loaders.mode, SelectionMode::None);
        assert_eq!(args.driver.model.as_deref(), Some("claude-opus-4-7"));
        assert_eq!(args.driver.temperature, None);
        assert_eq!(args.driver.max_tokens, None);
    }

    #[test]
    fn parse_rejects_driver_temperature_as_unsupported() {
        // No v1 CLI driver pipes temperature through; the previous
        // warn-once-and-continue treatment was a silent surprise.
        let err = ChatArgs::parse(&json!({
            "prompt": "hi",
            "driver": {"temperature": 0.3}
        }))
        .unwrap_err();
        let msg = format!("{err}");
        assert!(msg.contains("temperature"), "msg should name the knob: {msg}");
        assert!(msg.contains("not supported"), "msg should explain: {msg}");
    }

    #[test]
    fn parse_rejects_driver_max_tokens_as_unsupported() {
        let err = ChatArgs::parse(&json!({
            "prompt": "hi",
            "driver": {"max_tokens": 1024}
        }))
        .unwrap_err();
        let msg = format!("{err}");
        assert!(msg.contains("max_tokens"), "msg should name the knob: {msg}");
        assert!(msg.contains("not supported"), "msg should explain: {msg}");
    }

    #[test]
    fn parse_rejects_both_unsupported_knobs_in_one_error() {
        // When the caller sets both, the error must name both — not
        // just the first one we happened to check — so they fix the
        // payload in one round-trip.
        let err = ChatArgs::parse(&json!({
            "prompt": "hi",
            "driver": {"temperature": 0.3, "max_tokens": 1024}
        }))
        .unwrap_err();
        let msg = format!("{err}");
        assert!(msg.contains("temperature"));
        assert!(msg.contains("max_tokens"));
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

    #[test]
    fn format_skills_hint_returns_none_for_empty_input() {
        assert!(format_skills_hint(&[]).is_none());
    }

    #[test]
    fn format_skills_hint_lists_each_skill_with_one_line_description() {
        let skills = vec![
            crate::runtime::abilities::AgentAbilitySpec::new(
                "alice.voice",
                "Speak text via the local TTS engine.\nMore detail.",
                json!({"type": "object"}),
            )
            .unwrap(),
            crate::runtime::abilities::AgentAbilitySpec::new(
                "alice.fs.read",
                "Read a file from disk.",
                json!({"type": "object"}),
            )
            .unwrap(),
        ];
        let hint = format_skills_hint(&skills).expect("non-empty");
        assert!(hint.contains("Available skills"));
        assert!(hint.contains("- `alice.voice` — Speak text via the local TTS engine."));
        assert!(hint.contains("- `alice.fs.read` — Read a file from disk."));
        // Multi-line descriptions get trimmed to first line.
        assert!(!hint.contains("More detail"));
    }

    #[test]
    fn compose_chat_context_returns_none_when_all_fragments_empty() {
        assert!(compose_chat_context(None, &[], None, None).is_none());
        assert!(
            compose_chat_context(Some("   "), &[], Some("   "), Some("   ")).is_none()
        );
    }

    #[test]
    fn compose_chat_context_orders_skills_loaders_attachments_caller() {
        let chunks = vec!["LOADER".to_string()];
        let out = compose_chat_context(
            Some("SKILLS"),
            &chunks,
            Some("ATTACH"),
            Some("CALLER"),
        )
        .unwrap();
        let skills_at = out.find("SKILLS").unwrap();
        let loader_at = out.find("LOADER").unwrap();
        let attach_at = out.find("ATTACH").unwrap();
        let caller_at = out.find("CALLER").unwrap();
        assert!(skills_at < loader_at, "skills must precede loader output");
        assert!(loader_at < attach_at, "loader output must precede attachments");
        assert!(attach_at < caller_at, "attachments must precede caller context");
    }

    // ── attachments ──────────────────────────────────────────────────

    #[test]
    fn parse_attachments_absent_yields_empty_vec() {
        let args = ChatArgs::parse(&json!({"prompt": "hi"})).unwrap();
        assert!(args.attachments.is_empty());
    }

    #[test]
    fn parse_attachments_defaults_encoding_to_utf8() {
        let args = ChatArgs::parse(&json!({
            "prompt": "hi",
            "attachments": [{"path": "/etc/hosts"}]
        }))
        .unwrap();
        assert_eq!(args.attachments.len(), 1);
        assert_eq!(args.attachments[0].path, "/etc/hosts");
        assert_eq!(args.attachments[0].encoding, AttachmentEncoding::Utf8);
    }

    #[test]
    fn parse_attachments_rejects_non_array() {
        let err = ChatArgs::parse(&json!({
            "prompt": "hi",
            "attachments": "/etc/hosts"
        }))
        .unwrap_err();
        assert!(format!("{err}").contains("array"));
    }

    #[test]
    fn parse_attachments_rejects_missing_path() {
        let err = ChatArgs::parse(&json!({
            "prompt": "hi",
            "attachments": [{"encoding": "utf8"}]
        }))
        .unwrap_err();
        assert!(format!("{err}").contains("path"));
    }

    #[test]
    fn parse_attachments_rejects_unknown_encoding() {
        let err = ChatArgs::parse(&json!({
            "prompt": "hi",
            "attachments": [{"path": "/x", "encoding": "rot13"}]
        }))
        .unwrap_err();
        assert!(format!("{err}").contains("encoding"));
    }

    #[test]
    fn materialize_attachments_empty_returns_none() {
        assert!(materialize_attachments(&[]).unwrap().is_none());
    }

    #[test]
    fn materialize_attachments_reads_utf8_file_and_wraps_with_delimiters() {
        let dir = std::env::temp_dir().join(format!(
            "chat-attachments-{}-{}",
            std::process::id(),
            uuid_like()
        ));
        std::fs::create_dir_all(&dir).unwrap();
        let path = dir.join("hello.txt");
        std::fs::write(&path, b"hello world").unwrap();
        let specs = vec![AttachmentSpec {
            path: path.to_string_lossy().to_string(),
            encoding: AttachmentEncoding::Utf8,
        }];
        let block = materialize_attachments(&specs).unwrap().unwrap();
        assert!(block.contains("## Attachments"));
        assert!(block.contains("encoding=\"utf8\""));
        assert!(block.contains("hello world"));
        assert!(block.contains("</file>"));
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn materialize_attachments_reads_binary_as_base64() {
        let dir = std::env::temp_dir().join(format!(
            "chat-attachments-bin-{}-{}",
            std::process::id(),
            uuid_like()
        ));
        std::fs::create_dir_all(&dir).unwrap();
        let path = dir.join("blob.bin");
        // Bytes that are NOT valid UTF-8 — would panic on utf8 path.
        std::fs::write(&path, [0xff, 0xfe, 0xfd]).unwrap();
        let specs = vec![AttachmentSpec {
            path: path.to_string_lossy().to_string(),
            encoding: AttachmentEncoding::Base64,
        }];
        let block = materialize_attachments(&specs).unwrap().unwrap();
        assert!(block.contains("encoding=\"base64\""));
        // base64("\xff\xfe\xfd") = "//79"
        assert!(
            block.contains("//79"),
            "expected base64 of 0xff 0xfe 0xfd in block, got: {block}"
        );
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn materialize_attachments_bails_on_non_utf8_under_utf8_encoding() {
        let dir = std::env::temp_dir().join(format!(
            "chat-attachments-bad-{}-{}",
            std::process::id(),
            uuid_like()
        ));
        std::fs::create_dir_all(&dir).unwrap();
        let path = dir.join("blob.bin");
        std::fs::write(&path, [0xff, 0xfe, 0xfd]).unwrap();
        let specs = vec![AttachmentSpec {
            path: path.to_string_lossy().to_string(),
            encoding: AttachmentEncoding::Utf8,
        }];
        let err = materialize_attachments(&specs).unwrap_err();
        assert!(format!("{err}").contains("UTF-8"));
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn materialize_attachments_bails_on_missing_file() {
        let specs = vec![AttachmentSpec {
            path: "/nonexistent/really/not/here.txt".to_string(),
            encoding: AttachmentEncoding::Utf8,
        }];
        let err = materialize_attachments(&specs).unwrap_err();
        assert!(
            format!("{err}").contains("stat") || format!("{err}").contains("open"),
            "expected an I/O error, got: {err}"
        );
    }

    #[test]
    fn base64_encode_round_trip_known_vectors() {
        // RFC 4648 §10
        assert_eq!(base64_encode(b""), "");
        assert_eq!(base64_encode(b"f"), "Zg==");
        assert_eq!(base64_encode(b"fo"), "Zm8=");
        assert_eq!(base64_encode(b"foo"), "Zm9v");
        assert_eq!(base64_encode(b"foob"), "Zm9vYg==");
        assert_eq!(base64_encode(b"fooba"), "Zm9vYmE=");
        assert_eq!(base64_encode(b"foobar"), "Zm9vYmFy");
    }
}
