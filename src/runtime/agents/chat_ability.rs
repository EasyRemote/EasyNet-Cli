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
    dispatch_handle: Arc<std::sync::OnceLock<Arc<LocalAbilityRegistry>>>,
) {
    for (agent_name, entry) in &agents.agents {
        register_for_agent(reg, agent_name.clone(), entry.clone(), Arc::clone(&loaders));
    }
    // The owner-namespaced `<agent>.discover` and `<agent>.invoke`
    // self-bundle abilities live in their own modules — see
    // `runtime::agents::build_registry_with_services` (called after
    // the dispatch handle is in scope, since `<agent>.invoke` needs
    // to resolve through the live registry).
    //
    // After every static `<agent>.chat` + per-verb handler is in
    // place, install a single fallback resolver so:
    //
    //   * a `<agent>.<verb>` whose `*.ability.toml` is added to the
    //     workspace post-boot becomes invokable at the next call
    //     without daemon restart (existing TOML hot-reload story);
    //   * a brand-new `easynet agent add <name>` is picked up
    //     automatically — the resolver re-reads `agents.json` per
    //     miss and synthesises `<self>.chat` / `<self>.discover` /
    //     `<self>.invoke` on the fly. Pre-fix this required a daemon
    //     restart.
    let agents_snapshot = Arc::new(agents.clone());
    register_dynamic_agent_fallback(reg, agents_snapshot, loaders, dispatch_handle);
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
    // The fallback handler is intentionally simple: "act as
    // ability X with args Y" — the agent's own CLAUDE.md /
    // SKILL.md define what fulfilling the ability means; this
    // function just routes the call.
    let other_abilities = crate::runtime::abilities::abilities_for(&agent_name, &entry);
    let chat_name = ability.clone();
    for spec in other_abilities {
        if spec.name() == chat_name {
            continue;
        }
        let ability_name = spec.name().to_string();
        let bare_ability = ability_name
            .strip_prefix(&format!("{agent_name}."))
            .unwrap_or(&ability_name)
            .to_string();
        let h = build_agent_ability_handler(
            agent_name.clone(),
            entry.clone(),
            Arc::clone(&loaders),
            bare_ability,
        );
        reg.register_rpc(&ability_name, h);
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

/// Build one chat-translation RPC handler for an agent's
/// non-`chat` ability. Pulled out as a free fn so both the
/// boot-time pre-registration loop in `register_for_agent` and
/// the post-boot fallback resolver in
/// `register_dynamic_agent_fallback` produce byte-for-byte the
/// same handler — keeps the "ability fulfilled by chat" contract
/// in exactly one place.
///
/// The handler synthesises a prompt instructing the agent to
/// fulfill the declared ability `<bare_ability>` with the caller's
/// args, then routes through the agent's chat handler. The chat
/// reply is wrapped in a `{result, fulfilled_by, agent, usage,
/// elapsed_ms}` envelope so callers can distinguish an LLM-
/// fulfilled call from a synchronous one.
pub(crate) fn build_agent_ability_handler(
    agent_name: String,
    entry: AgentEntry,
    loaders: Arc<Vec<Arc<dyn ContextLoader>>>,
    bare_ability: String,
) -> crate::runtime::ability_dispatch::LocalRpcHandler {
    Arc::new(move |args: Value| {
        // Re-read this agent's manifests at invoke time so we see
        // edits made post-boot. Two pieces of state come out of the
        // matching manifest:
        //
        //   * `exec`: when present, route directly to the bound
        //     executor (shell argv, future http/wasm) — no chat in
        //     the loop, sub-second turnaround, deterministic.
        //   * `description`: when no exec is bound, the chat
        //     fallback embeds the manifest's description verbatim
        //     into the prompt so the agent has the contract to
        //     fulfil. Losing this was the root cause of "the
        //     ability is registered, the call routes, but the
        //     agent ignores the brief and fabricates an answer".
        let matching_manifest = crate::runtime::abilities::manifests_for(&agent_name, &entry)
            .into_iter()
            .find(|m| m.name() == bare_ability);

        if let Some(manifest) = matching_manifest.as_ref() {
            if let Some(exec) = manifest.exec() {
                let timeout = manifest
                    .timeout_seconds()
                    .map(std::time::Duration::from_secs);
                return match exec {
                    crate::core::ability_spec::AbilityExec::Shell(spec) => {
                        crate::runtime::agents::shell_executor::run_shell_exec(
                            spec, &args, timeout,
                        )
                    }
                    crate::core::ability_spec::AbilityExec::Http(spec) => {
                        crate::runtime::agents::http_executor::run_http_exec(
                            spec, &args, timeout,
                        )
                    }
                    crate::core::ability_spec::AbilityExec::Eal(spec) => {
                        crate::runtime::agents::eal_executor::run_eal_exec(
                            spec, &args, timeout,
                        )
                    }
                };
            }
        }

        let manifest_description: String = matching_manifest
            .as_ref()
            .map(|m| m.description().to_string())
            .unwrap_or_default();

        let prompt = if manifest_description.trim().is_empty() {
            format!(
                "Fulfill your declared ability `{bare}` with the following arguments \
                 (JSON, may be empty object): {args}\n\n\
                 Reply with the ability's result as plain text — no preamble, no markdown \
                 fence, no commentary. If the arguments are invalid for this ability, \
                 reply with a single line starting with `error: `.",
                bare = bare_ability,
                args = serde_json::to_string(&args).unwrap_or_else(|_| "{}".to_string()),
            )
        } else {
            format!(
                "You are fulfilling your declared ability `{bare}`.\n\n\
                 The ability's contract (from its TOML manifest description) is:\n\n\
                 ----- BEGIN ABILITY CONTRACT -----\n\
                 {desc}\n\
                 ----- END ABILITY CONTRACT -----\n\n\
                 You MUST follow the contract literally — if it tells you to run a \
                 specific shell command (curl, ffmpeg, git, …), run THAT command via \
                 your Bash tool. Do not substitute a different tool (no WebSearch, \
                 no fabrication). If the contract names a particular response prefix \
                 or format, use it.\n\n\
                 Caller arguments (JSON, may be empty object): {args}\n\n\
                 Reply with the ability's result as plain text — no preamble, no markdown \
                 fence, no commentary. If the arguments are invalid for this ability, \
                 reply with a single line starting with `error: `.",
                bare = bare_ability,
                desc = manifest_description.trim(),
                args = serde_json::to_string(&args).unwrap_or_else(|_| "{}".to_string()),
            )
        };

        let chat_args = serde_json::json!({
            "prompt": prompt,
            "stream": false,
        });
        handler(&agent_name, &entry, &loaders, chat_args).map(|chat_resp| {
            let reply = chat_resp
                .get("reply")
                .cloned()
                .unwrap_or(serde_json::Value::Null);
            let usage = chat_resp
                .get("usage")
                .cloned()
                .unwrap_or(serde_json::Value::Null);
            let elapsed_ms = chat_resp
                .get("elapsed_ms")
                .cloned()
                .unwrap_or(serde_json::Value::Null);
            serde_json::json!({
                "result": reply,
                "fulfilled_by": "agent_chat",
                "agent": agent_name,
                "usage": usage,
                "elapsed_ms": elapsed_ms,
            })
        })
    })
}

/// Install the per-agent fallback resolver on `reg`. After every
/// agent's static `register_for_agent` has run, the daemon boot
/// code calls this once so a `<agent>.<verb>` whose `*.ability.toml`
/// landed in the workspace AFTER boot still routes correctly: the
/// dispatcher's lookup-miss path consults this resolver, which
/// re-reads the workspace's `abilities/` directory and synthesises
/// a fresh chat-translation handler on the fly.
///
/// Hot-add of brand-new agents
/// ---------------------------
/// The resolver re-loads `~/.easynet/agents.json` on every miss so
/// that `easynet agent add <name>` is picked up without a daemon
/// restart. Pre-fix the closure captured an `Arc<AgentRegistry>`
/// snapshot from boot — a newly-added agent's `<self>.chat` /
/// `<self>.discover` / `<self>.invoke` would all miss until the
/// daemon was killed and brought back. Re-loading on miss costs one
/// `read_to_string + serde_json` per miss; the registry is small and
/// the lookup miss is itself the slow path.
///
/// `dispatch_handle` is the same `OnceLock` consumed by
/// `runtime::agents::build_registry_with_services` for the
/// per-agent `<agent>.invoke` builtin. We thread it here so a
/// brand-new agent's invoke handler can resolve through the live
/// registry — without it, hot-added invoke would have nowhere to
/// dispatch.
///
/// `loaders` is shared across every agent. A miss in the registry
/// that does NOT match any `<agent>.<verb>` shape is passed through
/// to the legacy "no handler registered" error.
pub(crate) fn register_dynamic_agent_fallback(
    reg: &mut LocalAbilityRegistry,
    _agents_snapshot: Arc<crate::registry::agents::AgentRegistry>,
    loaders: Arc<Vec<Arc<dyn ContextLoader>>>,
    dispatch_handle: Arc<std::sync::OnceLock<Arc<LocalAbilityRegistry>>>,
) {
    reg.set_rpc_fallback(Arc::new(
        move |ability: &str| -> Option<crate::runtime::ability_dispatch::LocalRpcHandler> {
            // Split `<agent>.<verb>` once; trailing dots in the verb
            // are preserved for forward compat (a future ability
            // could legitimately contain a dot).
            let (agent_name, bare_verb) = ability.split_once('.')?;

            // Re-load agents.json on every miss. A daemon boot
            // ago `easynet agent add <newname>` would not be
            // visible; re-loading per call is what makes the
            // hot-add story real. Failure to read is treated as
            // "no agent" — the legacy not-found semantics still
            // apply.
            let live_agents = crate::registry::agents::load_agents().ok()?;
            let entry = live_agents.agents.get(agent_name)?.clone();

            // Three synthesis paths in priority order:
            //   1. self-bundle builtins (chat / discover / invoke)
            //      — synthesise the same handler the boot path
            //      would have registered, so a hot-added agent
            //      gets `<self>.chat` etc. immediately.
            //   2. workspace TOML — re-enumerate `abilities/*.toml`
            //      and build the chat-translation or shell-exec
            //      handler the manifest declares.
            //   3. miss — return None.
            match bare_verb {
                "chat" => {
                    return Some(build_chat_handler_for(
                        agent_name.to_string(),
                        entry,
                        Arc::clone(&loaders),
                    ));
                }
                "discover" => {
                    return Some(build_discover_handler_for(
                        agent_name.to_string(),
                        Arc::clone(&dispatch_handle),
                    ));
                }
                "invoke" => {
                    return Some(build_invoke_handler_for(
                        agent_name.to_string(),
                        Arc::clone(&dispatch_handle),
                    ));
                }
                _ => { /* fall through to TOML path */ }
            }

            // TOML path: re-enumerate this agent's abilities at
            // lookup time. `abilities_for` is filesystem-backed,
            // so a TOML written post-boot becomes visible
            // immediately.
            let specs = crate::runtime::abilities::abilities_for(agent_name, &entry);
            let qualified = format!("{agent_name}.{bare_verb}");
            let matched = specs.iter().any(|s| s.name() == qualified);
            if !matched {
                return None;
            }

            Some(build_agent_ability_handler(
                agent_name.to_string(),
                entry,
                Arc::clone(&loaders),
                bare_verb.to_string(),
            ))
        },
    ));
}

/// Synthesise an `<agent>.chat` handler for the fallback path. Same
/// shape as the boot-time registration in `register_for_agent`,
/// pulled out as a helper so the hot-add and boot paths produce
/// byte-identical handlers.
fn build_chat_handler_for(
    agent_name: String,
    entry: AgentEntry,
    loaders: Arc<Vec<Arc<dyn ContextLoader>>>,
) -> crate::runtime::ability_dispatch::LocalRpcHandler {
    Arc::new(move |args: Value| handler(&agent_name, &entry, &loaders, args))
}

/// Synthesise an `<agent>.discover` handler for a hot-added agent.
/// The handler closes over `agent_name` for caller identity and
/// re-loads `agents.json` per call so the discover ladder sees
/// every peer that exists *now*, including agents added after this
/// closure was built.
fn build_discover_handler_for(
    agent_name: String,
    dispatch_handle: Arc<std::sync::OnceLock<Arc<LocalAbilityRegistry>>>,
) -> crate::runtime::ability_dispatch::LocalRpcHandler {
    // Replicate the surface of `discover_ability::register_for_agent`
    // without going through that function (it expects a `&mut
    // LocalAbilityRegistry`, which we don't have here). The handler
    // re-loads agents on every call so a brand-new peer is visible
    // immediately — same hot-add story as the chat handler.
    let provider: Arc<dyn Fn() -> crate::registry::agents::AgentRegistry + Send + Sync> =
        Arc::new(|| crate::registry::agents::load_agents().unwrap_or_default());
    Arc::new(move |args: Value| {
        // Defer to the discover module's per-call entry. Public
        // entry exposed for this purpose (and test cases).
        crate::runtime::agents::discover_ability::dispatch(
            &agent_name,
            &provider,
            &dispatch_handle,
            args,
        )
    })
}

/// Synthesise an `<agent>.invoke` handler for a hot-added agent.
/// Routes through the same builtin invoke entry the boot-time
/// registration uses.
fn build_invoke_handler_for(
    agent_name: String,
    dispatch_handle: Arc<std::sync::OnceLock<Arc<LocalAbilityRegistry>>>,
) -> crate::runtime::ability_dispatch::LocalRpcHandler {
    let provider: Arc<dyn Fn() -> crate::registry::agents::AgentRegistry + Send + Sync> =
        Arc::new(|| crate::registry::agents::load_agents().unwrap_or_default());
    Arc::new(move |args: Value| {
        crate::runtime::agents::invoke_ability::dispatch(
            &agent_name,
            &provider,
            &dispatch_handle,
            args,
        )
    })
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

    // Cross-agent ability discovery. Lets agent A see (and call,
    // via MCP tools registered alongside its own) abilities owned
    // by other agents on the same device — the workflow where a
    // user asks the active agent for something only another agent
    // has the skill for. Surfaces as a separate context section
    // so the LLM treats own-agent vs other-agent abilities with
    // appropriate precedence.
    let other_specs = enumerate_other_agent_specs(agent_name);
    let cross_agent_hint = format_cross_agent_hint(&other_specs);

    // Materialise attachments to a delimited block. Failures bail
    // loud — attachments are explicit input, not best-effort
    // context, so a missing path is the operator's bug to fix.
    let attachments_block = materialize_attachments(&parsed.attachments)?;

    // Compose the literal context. Order: skills hint, cross-agent
    // hint, loader output, attachments block, caller's explicit
    // `context` arg — each separated by blank lines so the LLM
    // reads them as discrete sections. An empty composition yields
    // `None` so `compose_prompt` does not insert a useless wrapper.
    let composed_context: Option<String> = compose_chat_context(
        skills_hint.as_deref(),
        cross_agent_hint.as_deref(),
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
    // Conversation resume. The caller's `session_id` (when supplied
    // and shaped like a driver-issued thread id) tells us to
    // continue an existing conversation rather than start fresh.
    // We feed it through `DriverOverrides::resume_thread_id`; the
    // driver layer (codex today) maps it to `codex exec resume`.
    // For the fresh-conversation case the driver mints a new id and
    // returns it via `AgentResponse::thread_id`; we surface that
    // back to the caller as `session_id` so a follow-up turn can
    // come back here and pick up the same thread.
    //
    // `looks_like_thread_id` is intentionally permissive — codex
    // ids are UUIDv7 (8-4-4-4-12 hex with dashes), but operator
    // tooling that fabricates ids for replay or test deserves the
    // same path. The driver itself does the strict validation.
    let mut driver_with_resume = parsed.driver.clone();
    if let Some(sid) = parsed.session_id.as_deref() {
        if looks_like_thread_id(sid) {
            driver_with_resume.resume_thread_id = Some(sid.to_string());
        }
    }
    let driver_overrides = Some(&driver_with_resume);
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
    // The session id we report back. Three cases:
    //
    //   1. Resume turn (`driver_with_resume.resume_thread_id` was
    //      Some): the caller already owns the id; we MUST echo it
    //      back unchanged so subsequent turns keep finding the same
    //      transcript. Some drivers (claude-code) return a freshly-
    //      minted id on the resume's stream — that id is internal
    //      to the resumed run and is NOT a stable handle to the
    //      original transcript; passing it back would break R3.
    //
    //   2. Fresh turn, driver minted an id (codex / claude on
    //      first turn): use the driver's id so future resume can
    //      find the transcript.
    //
    //   3. Fresh turn, driver did NOT mint an id (no resume-capable
    //      driver wired to thread_id yet): fall back to the local
    //      `session_id` we resolved at handler entry — caller-
    //      supplied or our `uuid_like` mint.
    let session_id = if let Some(resume_id) = driver_with_resume.resume_thread_id.as_ref() {
        resume_id.clone()
    } else if let Some(driver_id) = resp.thread_id.as_ref() {
        driver_id.clone()
    } else {
        session_id
    };
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
    let other_specs = enumerate_other_agent_specs(agent_name);
    let cross_agent_hint = format_cross_agent_hint(&other_specs);
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
        cross_agent_hint.as_deref(),
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
    // Conversation resume on the stream path mirrors handle_invoke:
    // when the caller's session_id parses as a driver-issued thread
    // id we thread it through `DriverOverrides::resume_thread_id`,
    // and the driver maps it to `codex exec resume <id>` /
    // `claude -p --resume <id>`. See the matching block in
    // handle_invoke for the full rationale; the comment is not
    // duplicated here to keep the two paths visibly identical.
    let mut driver_owned = parsed.driver.clone();
    if let Some(sid) = parsed.session_id.as_deref() {
        if looks_like_thread_id(sid) {
            driver_owned.resume_thread_id = Some(sid.to_string());
        }
    }
    let resume_id_for_done = driver_owned.resume_thread_id.clone();
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
                    // Resolve the terminal-frame session id with the
                    // same precedence as handle_invoke:
                    //   1. Resume turn → echo caller's id unchanged.
                    //   2. Fresh turn, driver minted an id → use it.
                    //   3. Fresh turn, driver did not surface one →
                    //      fall back to the locally-resolved id.
                    let resolved_session_id = if let Some(rid) = resume_id_for_done.as_ref() {
                        rid.clone()
                    } else if let Some(did) = resp.thread_id.as_ref() {
                        did.clone()
                    } else {
                        session_id_for_thread.clone()
                    };
                    json!({
                        "type": "done",
                        "session_id": resolved_session_id,
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

/// Enumerate every other registered agent's published abilities so
/// the calling agent's LLM can compose against them as MCP tools.
///
/// This is what makes "agent A asks agent B's weather ability"
/// possible: agent A's chat handler runs, the chat context lists
/// `<other>.weather` alongside agent A's own skills, and the LLM
/// — seeing the qualified name as an available tool — calls it.
/// The registered cross-process route (runtime_local_tools +
/// daemon dispatcher) carries the call to agent B, whose own
/// chat-translation handler then fulfils it with whatever skills
/// agent B has installed (e.g. an HTTP weather skill).
///
/// Excluded:
///   * The calling agent itself (its own abilities are already
///     exposed via `enumerate_skill_specs`; including them again
///     would surface duplicates in the hint and the MCP tool list).
///   * Every `<x>.chat` — chat is the agent's outgoing surface,
///     not a callable tool. Including it would invite the LLM to
///     spawn nested chats just to "ask another agent something",
///     which is what the per-ability route exists to avoid.
fn enumerate_other_agent_specs(
    self_agent_name: &str,
) -> Vec<crate::runtime::abilities::AgentAbilitySpec> {
    let registry = match crate::registry::agents::load_agents() {
        Ok(r) => r,
        Err(_) => return Vec::new(),
    };
    let mut out = Vec::new();
    for (other_name, other_entry) in &registry.agents {
        if other_name == self_agent_name {
            continue;
        }
        let other_chat = format!("{other_name}.{ABILITY_VERB}");
        for spec in crate::runtime::abilities::abilities_for(other_name, other_entry) {
            if spec.name() == other_chat {
                continue;
            }
            out.push(spec);
        }
    }
    out
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
    cross_agent_hint: Option<&str>,
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
    if let Some(h) = cross_agent_hint {
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

/// Cross-agent counterpart to `format_skills_hint`. When the live
/// agent registry has more than one entry, the chat handler appends
/// this block so the LLM also sees what OTHER agents on the same
/// device can do — e.g. agent A asking agent B's `weather` ability
/// without the user needing to wire the connection by hand.
///
/// Returns `None` when there are no other agents (single-agent
/// installs see exactly the same prompt they did before this hint
/// was added; no spurious empty section).
fn format_cross_agent_hint(
    others: &[crate::runtime::abilities::AgentAbilitySpec],
) -> Option<String> {
    if others.is_empty() {
        return None;
    }
    let mut out = String::from("## Available abilities (other agents on this device)\n\n");
    out.push_str(
        "These abilities are owned by other agents installed alongside you. They are \
         exposed to you as MCP tools too — calling them lets the other agent fulfil \
         the request with its own skills. Prefer them over guessing when the user's \
         intent matches a name listed here.\n\n",
    );
    for s in others {
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
        // resume_thread_id is not parsed from the `driver` block; it
        // is set by the chat handler from the caller's top-level
        // `session_id` argument (see compute_resume_thread_id at
        // the call site). Keeping it None here means a caller that
        // tries to set `driver.resume_thread_id` is silently
        // ignored — the canonical path is `session_id`, not a
        // driver-shaped knob, and we do not want two surfaces.
        resume_thread_id: None,
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

/// Quick check: does this `session_id` look like a driver-issued
/// thread id that we should pass through as `resume_thread_id`?
///
/// Codex emits UUIDv7 (8-4-4-4-12 hex digits with dashes). We accept
/// any shape that matches that form regardless of the version nibble
/// — accepting a UUIDv4 the operator fabricated for a test or replay
/// is harmless; codex itself does the strict validation when we hand
/// the id to `exec resume`. Strings that match our local `uuid_like`
/// fallback (`<32-hex>-<16-hex>`) are intentionally NOT accepted —
/// those are the chat ability's own minted ids that no resume-capable
/// driver knows about; passing them through would force the driver
/// into a UUID-parse failure on every legacy session.
fn looks_like_thread_id(s: &str) -> bool {
    let bytes = s.as_bytes();
    if bytes.len() != 36 {
        return false;
    }
    let dash_positions = [8, 13, 18, 23];
    for (i, b) in bytes.iter().enumerate() {
        let is_dash_pos = dash_positions.contains(&i);
        let ok = if is_dash_pos {
            *b == b'-'
        } else {
            b.is_ascii_hexdigit()
        };
        if !ok {
            return false;
        }
    }
    true
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
        register(
            &mut reg,
            &agents,
            Arc::new(Vec::new()),
            Arc::new(std::sync::OnceLock::new()),
        );
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
        //
        // HomeGuard is load-bearing for parallel runs: `stream_handler`
        // calls `enumerate_skill_specs("alice", entry)`, which falls
        // back to `agents_root().join("alice")` when the entry has no
        // root_path. Under `cargo test` a sibling test landing an
        // `alice/abilities/*.toml` into the real `~/.easynet` between
        // thread switches would inject extra skills and bump the
        // snapshot length to 2. Scoping HOME to a private tmpdir
        // makes this test see the empty fallback every time, which
        // is what the assertion expects.
        let _g = crate::facade::cli::test_support::HomeGuard::new();
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

    #[test]
    fn looks_like_thread_id_accepts_uuid_shape_only() {
        // Codex emits UUIDv7 (`019dd304-60d9-74f2-8085-d4624e195d62`)
        // and claude emits UUIDv4 (`38e5640c-6843-4f15-8f3a-2c8de75d0209`)
        // — both are 8-4-4-4-12 hex with dashes, regardless of the
        // version nibble. The helper must accept both so the chat
        // ability routes resume requests through to either driver.
        assert!(looks_like_thread_id(
            "019dd304-60d9-74f2-8085-d4624e195d62"
        ));
        assert!(looks_like_thread_id(
            "38e5640c-6843-4f15-8f3a-2c8de75d0209"
        ));
        assert!(looks_like_thread_id(
            "00000000-0000-0000-0000-000000000000"
        ));

        // Locally-minted `chat-<uuid_like>` ids — `<32-hex>-<16-hex>`
        // — are NOT a driver-issued shape and MUST be rejected so the
        // chat ability does not try to feed them to `--resume`.
        let local = format!("chat-{}", uuid_like());
        assert!(!looks_like_thread_id(&local));
        assert!(!looks_like_thread_id(&uuid_like()));

        // Edge cases: wrong dash positions, wrong length, non-hex.
        assert!(!looks_like_thread_id(""));
        assert!(!looks_like_thread_id("not-a-uuid"));
        assert!(!looks_like_thread_id(
            "019dd304-60d9-74f2-8085-d4624e195d6"
        )); // 35 chars
        assert!(!looks_like_thread_id(
            "019dd304-60d9-74f2-8085-d4624e195d622"
        )); // 37 chars
        assert!(!looks_like_thread_id(
            "019dd30460d9-74f2-8085-d4624e195d62-"
        )); // dashes wrong
        assert!(!looks_like_thread_id(
            "019dd304-60d9-74f2-8085-d4624e195XYZ"
        )); // non-hex tail
    }

    #[test]
    fn stream_handler_resume_id_only_set_for_uuid_shaped_session() {
        // Pin the wiring contract for the stream path:
        //   - A caller-supplied uuid-shaped session_id should be
        //     surfaced as the terminal-frame session_id (so the
        //     driver can resume on the next turn).
        //   - A non-uuid-shaped session_id falls through unchanged.
        //
        // We can't drive a real dispatch without a live LLM in the
        // unit-test environment, but we CAN observe the snapshot
        // frame's session_id which the handler emits before any
        // dispatch happens. That is sufficient to exercise the
        // `looks_like_thread_id` branch the resume wiring keys on.
        let entry = entry();
        let resume_id = "019dd304-60d9-74f2-8085-d4624e195d62";
        let source = stream_handler(
            "alice",
            &entry,
            &[],
            json!({"prompt": "hi", "session_id": resume_id}),
        )
        .expect("stream handler must construct snapshot");
        match source {
            StreamSource::SnapshotThenLive(snapshot, _rx) => {
                let first = &snapshot[0];
                assert_eq!(
                    first.get("session_id").and_then(Value::as_str),
                    Some(resume_id),
                    "snapshot session frame must echo caller-supplied uuid id verbatim"
                );
            }
            other => panic!("expected SnapshotThenLive, got {other:?}"),
        }

        // Non-uuid id still echoed verbatim through the snapshot —
        // the resume branch is gated separately at the driver wire.
        let custom = "my-replay-tag";
        let source2 = stream_handler(
            "alice",
            &entry,
            &[],
            json!({"prompt": "hi", "session_id": custom}),
        )
        .expect("stream handler must construct snapshot");
        match source2 {
            StreamSource::SnapshotThenLive(snapshot, _rx) => {
                assert_eq!(
                    snapshot[0].get("session_id").and_then(Value::as_str),
                    Some(custom),
                );
            }
            other => panic!("expected SnapshotThenLive, got {other:?}"),
        }
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
        assert!(compose_chat_context(None, None, &[], None, None).is_none());
        assert!(
            compose_chat_context(Some("   "), None, &[], Some("   "), Some("   ")).is_none()
        );
    }

    #[test]
    fn compose_chat_context_orders_skills_loaders_attachments_caller() {
        let chunks = vec!["LOADER".to_string()];
        let out = compose_chat_context(
            Some("SKILLS"),
            None,
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

    /// Integration-level proof that an ability whose manifest pins
    /// `[exec] kind = "shell"` dispatches through the shell executor
    /// rather than the chat-translation fallback.
    ///
    /// What this protects against
    /// --------------------------
    /// Pre-fix the dispatcher always routed `<agent>.<verb>` through
    /// the chat handler. A weather ability whose contract was "run
    /// curl wttr.in" therefore took 28+ seconds (LLM cold-start, tool
    /// search, retries) instead of the < 500 ms a direct curl would
    /// take. Worse, the LLM was free to substitute a different tool
    /// (WebSearch, MCP http_request) and produce a different
    /// envelope shape — making the ability non-deterministic.
    ///
    /// Wiring under test
    /// -----------------
    ///   on-disk `<agent-root>/abilities/<verb>.ability.toml` with
    ///   [exec] kind = "shell"
    ///     │
    ///     ▼
    ///   abilities::manifests_for() reads the AbilityManifest
    ///     │
    ///     ▼
    ///   build_agent_ability_handler() spots manifest.exec().is_some()
    ///   and routes to runtime::agents::shell_executor::run_shell_exec
    ///     │
    ///     ▼
    ///   subprocess `printf %s ok` is spawned (no LLM, no chat)
    ///
    /// The probe asserts the returned envelope's `fulfilled_by`
    /// field is `"shell"` — which only the shell executor ever
    /// stamps. A regression that re-routed through chat would
    /// emit `"agent_chat"` instead and fail this assertion before
    /// any latency probe got a chance to run.
    #[test]
    fn build_agent_ability_handler_routes_shell_exec_manifest_through_shell_executor() {
        use crate::core::ability_spec::{AbilityExec, AbilityManifest, ShellExec};
        use crate::registry::agents::AgentEntry;

        let _g = crate::facade::cli::test_support::HomeGuard::new();

        // Materialise an agent root with a single ability manifest
        // that pins a shell executor. We use `printf` (POSIX,
        // deterministic, available on the macOS dev box and any
        // Linux CI runner) so the test is hermetic — no network,
        // no LLM, no system PATH guesses beyond a coreutils.
        let ws_root = crate::persistence::config::agents_root().join("alice");
        let abilities_dir = ws_root.join("abilities");
        std::fs::create_dir_all(&abilities_dir)
            .expect("HomeGuard provides a fresh tmp HOME, mkdir must succeed");

        let manifest = AbilityManifest::new(
            "echo",
            "Echo the input value back via printf.",
            json!({
                "type": "object",
                "additionalProperties": false,
                "required": ["value"],
                "properties": {
                    "value": {"type": "string"}
                }
            }),
        )
        .expect("hand-built manifest is well-formed")
        .with_exec(AbilityExec::Shell(ShellExec {
            argv: vec![
                "printf".to_string(),
                "%s".to_string(),
                "{{ value }}".to_string(),
            ],
            stdout: None,
            sandbox: None,
        }))
        .expect("with_exec rejects only an empty argv; ours has three elements");

        std::fs::write(
            abilities_dir.join("echo.ability.toml"),
            manifest.to_toml_string().expect("manifest serialises"),
        )
        .expect("HomeGuard'd tmp HOME is writable");

        // Also seed a minimal agent.toml so AgentDirectory::open
        // accepts the root. The fields here mirror what
        // `easynet agent add` writes; the test is targeting
        // dispatch, not the agent.toml schema.
        std::fs::write(
            ws_root.join("agent.toml"),
            r#"name = "alice"
runtime = "claude-code"
model = "sonnet"
"#,
        )
        .expect("agent.toml write");

        // Build the handler the same way the registration paths do
        // (boot-time pre-register and post-boot fallback both call
        // build_agent_ability_handler — see register_for_agent and
        // register_dynamic_agent_fallback).
        let mut entry =
            AgentEntry::new(crate::registry::agents::AgentType::ClaudeCode, None);
        // `root_path` is the field that `manifests_for` (and
        // `abilities_for`) read to find the on-disk abilities/
        // directory. Without it the helpers fall back to the
        // synthetic chat-only path and the test would silently pass
        // through chat dispatch.
        entry.root_path = Some(ws_root.clone());
        let loaders: Arc<Vec<Arc<dyn ContextLoader>>> = Arc::new(Vec::new());
        let handler = build_agent_ability_handler(
            "alice".to_string(),
            entry,
            loaders,
            "echo".to_string(),
        );

        let envelope = handler(json!({ "value": "hello" }))
            .expect("shell exec must succeed for printf %s hello");

        assert_eq!(
            envelope.get("fulfilled_by").and_then(|v| v.as_str()),
            Some("shell"),
            "manifest with [exec] kind=\"shell\" MUST dispatch through the shell \
             executor, not the chat fallback. Envelope was: {envelope}"
        );
        assert_eq!(
            envelope.get("result").and_then(|v| v.as_str()),
            Some("hello"),
            "shell executor must capture stdout verbatim. Envelope: {envelope}"
        );
        assert_eq!(
            envelope.get("exit_code").and_then(|v| v.as_i64()),
            Some(0),
            "printf returns 0 on success; envelope: {envelope}"
        );
    }
}
