// EasyNet CLI — `<agent>.chat` system-registered ability
// =======================================================
//
// File: src/daemon/ability/builtins/agents/chat.rs
// Description: First-class registration of every locally-installed
//              agent's `chat` ability on the daemon's
//              `AxonAbilityCatalog`. After this lands, both the
//              Kernel and the MCP adapter can dispatch through the
//              same registered handler instead of each maintaining
//              their own special-case path into `send_external`.
//
// Why this lives in daemon builtins even though the wire name is `<agent>.chat`
// --------------------------------------------------------------------------------
// The daemon ability builtins tree is the registration surface: files here
// mount handlers on the registry. The `system.<feature>`
// naming convention is a rule about *which abilities are device-level*,
// not a rule about which files are allowed to register handlers. Chat
// is bound to a specific agent (so its name is `<agent>.chat`, not
// `system.chat`), but it is still registered by the daemon at boot
// from this module — there is no agent-side code path for it.
//
// Per-agent registration
// ----------------------
// Unlike `observe.health` (one handler globally) or `session.list`
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
//      `AxonAbilityCatalog::register_rpc` doc): adding a new agent
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
//     abilities (filtered by mode/include/exclude). The workspace MCP
//     projection exposes callable abilities by geometry; cross-agent
//     execution remains owned by the mission runtime. The skills filter is
//     currently advisory: we report what we would expose; per-call
//     enforcement of the include/exclude filter against the driver's
//     tool-discovery wire is a follow-up.
//   * context loaders: the trait seam exists; v1 ships ScheduleLoader
//     / MemoryLoader / UserProfileLoader. `context_used` reports
//     which loaders contributed and how many bytes each.
//   * driver overrides: `driver.model` flows through dispatch via
//     send_external_with_overrides. `driver.temperature` and
//     `driver.max_tokens` are rejected at parse time (no v1 CLI
//     driver exposes either knob), and every other driver-shaped
//     field is rejected instead of becoming a hidden lifecycle
//     surface (see parse_driver_overrides).
//   * stream: register_for_agent mounts both an RPC and a Stream
//     handler; the stream variant emits typed frames. `stream:true`
//     under the RPC entry point is rejected with a clear error.
//
// The output schema's `usage`, `tool_calls`, and `context_used`
// fields are populated by the driver layer's tool-use observability
// (via dispatch::ToolCall, projected from driver stream events).
// Drivers that do not expose tool-use observability return empty
// `tool_calls`. `usage` mirrors `AgentResponse.usage` when the driver
// reports it.
//
// Author: Silan Hu <silan.hu@u.nus.edu>
// Copyright (c) 2026 EasyNet. All rights reserved.

use std::path::{Component, PathBuf};
use std::sync::Arc;
use std::time::{Duration, Instant};

use serde_json::{json, Map, Value};

use crate::daemon::ability::dispatch::{AxonAbilityCatalog, StreamSource};
use crate::daemon::execution::mission::adapter::DriverIsolation;
use crate::daemon::execution::mission::dispatch::{
    AgentExecution, AgentResponse, DriverOverrides, ToolCall, MAX_INVOCATION_TIMEOUT_MS,
};
use crate::daemon::persistence::agent_aggregate::AgentAggregateRepository;
use crate::daemon::persistence::agent_registry::{AgentEntry, AgentRegistry};

/// The wire-level *verb* portion of every chat ability name. The
/// fully-qualified ability name is always `<agent>.chat`. A future
/// rename here would have to ripple through:
///   * the parity test in `daemon::execution::mission::agent_ability_specs`
///   * the manifest seed in `daemon::ability::manifest::default_chat_manifest`
///   * the EasyNet backend's frontend that synthesizes / renders chat
///
/// Pinning the constant in one place lets that future PR find the
/// surface area with a single grep.
pub const ABILITY_VERB: &str = crate::daemon::ability::names::agents::CHAT;

/// Wire sentinel for the agent's lifelong (default) session. A caller
/// that sends `session_id: "lifelong"` selects the agent's one durable
/// default thread instead of naming a concrete session: the handler
/// resolves the sentinel through the per-agent pointer persisted in
/// `chat_sessions::SessionIndex.lifelong` — resuming the bound session
/// when one exists, otherwise running a fresh turn and binding the
/// resulting id. The pointer (not a fixed literal id) is what gives
/// the thread real LLM continuity: drivers only resume UUID-shaped
/// thread ids they minted themselves, so a constant id would persist
/// a transcript while the LLM forgot every prior turn.
pub const LIFELONG_SESSION_ID: &str = "lifelong";

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
    fn load(&self, agent_name: &str, session_id: &str) -> anyhow::Result<Option<String>>;
}

/// Register a `<agent>.chat` handler on the supplied registry for
/// every agent in `agents`. Idempotent: re-calling with an updated
/// registry replaces the previous handler set per agent.
///
/// The `_loaders` parameter is the seam for the pluggable
/// context-loader chain. Today the daemon passes an empty Vec; later
/// PRs construct loaders during boot and pass them in.
pub fn register(
    reg: &mut AxonAbilityCatalog,
    agents: &AgentRegistry,
    loaders: Arc<Vec<Arc<dyn ContextLoader>>>,
    _dispatch_handle: Arc<std::sync::OnceLock<Arc<AxonAbilityCatalog>>>,
) {
    for (agent_name, entry) in &agents.agents {
        register_for_agent(reg, agent_name.clone(), entry.clone(), Arc::clone(&loaders));
    }
    // The owner-namespaced `<agent>.discover` and `<agent>.invoke`
    // self-bundle abilities live in their own modules — see
    // `daemon::ability::catalog::build_registry_with_services` (called after
    // the dispatch handle is in scope, since `<agent>.invoke` needs
    // to resolve through the live registry).
    //
    // No lookup-miss fallback is installed here. Post-boot agent
    // additions flow through HotAgentRegistrar, which materialises
    // handlers in LocalRuntime and advertises the owner projection.
    // A name that is absent from LocalRuntime must remain absent so
    // RFC-005 resolve-before-invoke can return a typed negative
    // instead of a hidden, locally-synthesised route.
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
    reg: &mut AxonAbilityCatalog,
    agent_name: String,
    entry: AgentEntry,
    loaders: Arc<Vec<Arc<dyn ContextLoader>>>,
) {
    use crate::daemon::ability::dispatch::OwnerKind;
    let ability = format!("{agent_name}.{ABILITY_VERB}");
    let owner = OwnerKind::Agent(agent_name.clone());

    // RPC: the synchronous one-shot path. Registered with the
    // canonical chat manifest so the Frontend
    // `InvokeAbilityDialog` renders a SchemaForm (prompt /
    // context / session_id / skills / context_loaders / driver /
    // stream / attachments) instead of a free-text JSON box.
    // Without this, the dialog falls back to "no declared
    // schema" and the user has to guess the args shape.
    let rpc_agent = agent_name.clone();
    let rpc_entry = entry.clone();
    let rpc_loaders = Arc::clone(&loaders);
    reg.register_rpc_with_spec(
        &ability,
        owner.clone(),
        crate::daemon::ability::manifest::default_chat_manifest()
            .with_admission_action(
                crate::daemon::ability::descriptors::AdmissionAction::Invoke.as_str(),
            )
            .expect("chat manifest accepts invoke admission_action"),
        Arc::new(move |args: Value| handler(&rpc_agent, &rpc_entry, &rpc_loaders, args)),
    );

    // For every OTHER ability the agent declares via its
    // workspace `<root>/abilities/*.toml`, register an adapter
    // handler that dispatches back to this agent's chat with a
    // synthesised prompt instructing it to fulfill the named
    // ability with the given args. Without this, an agent could
    // declare abilities (which surface in MCP catalog and
    // skills_loaded) but the LLM running inside has no way to
    // invoke them: the dispatcher returns NOT_FOUND for every
    // <agent>.<ability> name that isn't `<agent>.chat`.
    //
    let other_abilities =
        crate::daemon::execution::mission::agent_ability_specs::abilities_for(&agent_name, &entry);
    let manifests =
        crate::daemon::execution::mission::agent_ability_specs::manifests_for(&agent_name, &entry);
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

        let Some(manifest) = manifests.iter().find(|m| m.name() == bare_ability) else {
            continue;
        };
        let Some(exec) = manifest.exec() else {
            continue;
        };
        match exec {
            crate::daemon::ability::manifest::AbilityExec::HostStream(stream_spec) => {
                let h = build_host_stream_handler(stream_spec.clone());
                let manifest = manifest
                    .clone()
                    .with_admission_action(
                        crate::daemon::ability::descriptors::AdmissionAction::Stream.as_str(),
                    )
                    .expect("agent host_stream manifest accepts stream admission_action");
                reg.register_stream_with_envelope_and_spec(
                    &ability_name,
                    owner.clone(),
                    manifest,
                    h,
                );
            }
            _ => {
                let h = build_agent_ability_handler(
                    agent_name.clone(),
                    entry.clone(),
                    Arc::clone(&loaders),
                    bare_ability,
                );
                let manifest = manifest
                    .clone()
                    .with_admission_action(
                        crate::daemon::ability::descriptors::AdmissionAction::Invoke.as_str(),
                    )
                    .expect("agent executor manifest accepts invoke admission_action");
                reg.register_rpc_with_envelope_and_spec(&ability_name, owner.clone(), manifest, h);
            }
        }
    }

    // Stream: emit framed events. v1 ships a Snapshot variant
    // (eagerly materialised list) because the underlying LLM driver
    // is synchronous; once the driver gains an async token stream
    // the handler upgrades to `Live(broadcast::Receiver)` without
    // changing the wire frame shape.
    reg.register_stream_with_spec(
        &ability,
        owner,
        crate::daemon::ability::manifest::default_chat_manifest()
            .with_admission_action(
                crate::daemon::ability::descriptors::AdmissionAction::Stream.as_str(),
            )
            .expect("chat manifest accepts stream admission_action"),
        Arc::new(move |args: Value| stream_handler(&agent_name, &entry, &loaders, args)),
    );
}

/// Build the envelope-aware stream handler for a `host_stream` ability.
///
/// Registered via `register_stream_with_envelope_and_spec` so the
/// ability is stream-mode (`modes.stream = true`) and so the handler
/// sees the AXIOM seven-tuple: the runtime invocation id becomes the
/// wire `call_id` correlating the request to the external host. The
/// handler is the once-per-call `Fn` the stream registry expects — it
/// opens the host stream and returns the live `StreamSource`
/// immediately; anything that can fail the open surfaces as `Err` so a
/// failed open never produces a half-live session.
pub(crate) fn build_host_stream_handler(
    spec: crate::daemon::ability::manifest::HostStreamExec,
) -> crate::daemon::ability::dispatch::LocalStreamHandlerWithEnvelope {
    Arc::new(
        move |env: crate::daemon::ability::dispatch::EnvelopeContext, args: Value| {
            let call_id = env.invocation_id().to_string();
            let caller = env.caller().to_string();
            crate::daemon::execution::mission::executors::host_stream::run_host_stream(
                &spec, &args, &call_id, &caller,
            )
        },
    )
}

/// Build the server-stream `<agent>.chat` handler used by both
/// boot-time and hot lifecycle registration.
///
/// This is intentionally a handler factory rather than a registration
/// helper: lifecycle registration owns the catalogue transaction, while
/// this module owns only chat execution semantics.
pub(crate) fn build_chat_stream_handler_for(
    agent_name: String,
    entry: AgentEntry,
    loaders: Arc<Vec<Arc<dyn ContextLoader>>>,
) -> crate::daemon::ability::dispatch::LocalStreamHandler {
    Arc::new(move |args: Value| stream_handler(&agent_name, &entry, &loaders, args))
}

/// Build one executor-bound RPC handler for an agent's
/// non-`chat` ability. Pulled out as a free fn so both the
/// boot-time pre-registration loop in `register_for_agent` and
/// HotAgentRegistrar produce byte-for-byte the same handler. This
/// keeps manifest executor routing in exactly one place.
pub(crate) fn build_agent_ability_handler(
    agent_name: String,
    entry: AgentEntry,
    loaders: Arc<Vec<Arc<dyn ContextLoader>>>,
    bare_ability: String,
) -> crate::daemon::ability::dispatch::LocalRpcHandlerWithEnvelope {
    Arc::new(move |env, args: Value| {
        // Re-read this agent's manifests at invoke time so edits made
        // post-boot change the executor binding without a daemon restart.
        let matching_manifest =
            crate::daemon::execution::mission::agent_ability_specs::manifests_for(
                &agent_name,
                &entry,
            )
            .into_iter()
            .find(|m| m.name() == bare_ability);

        if let Some(manifest) = matching_manifest.as_ref() {
            if let Some(exec) = manifest.exec() {
                let timeout = manifest
                    .timeout_seconds()
                    .map(std::time::Duration::from_secs);
                return match exec {
                    crate::daemon::ability::manifest::AbilityExec::Shell(spec) => {
                        crate::daemon::execution::mission::executors::shell::run_shell_exec(
                            spec, &args, timeout,
                        )
                    }
                    crate::daemon::ability::manifest::AbilityExec::Http(spec) => {
                        crate::daemon::execution::mission::executors::http::run_http_exec(
                            spec, &args, timeout,
                        )
                    }
                    crate::daemon::ability::manifest::AbilityExec::Eal(spec) => {
                        let gateway = Arc::new(
                            crate::daemon::execution::mission::invocation_gateway::DaemonMissionInvocationGateway::from_admitted_envelope(&env)?,
                        );
                        crate::daemon::execution::mission::executors::eal::run_eal_exec_with_gateway(
                            spec, &args, gateway, timeout,
                        )
                    }
                    crate::daemon::ability::manifest::AbilityExec::Mcp(spec) => {
                        let _ = timeout;
                        crate::daemon::ability::builtins::integrations::mcp::executor::run_mcp_exec(
                            spec, &args,
                        )
                    }
                    crate::daemon::ability::manifest::AbilityExec::HostStream(_) => {
                        // host_stream registers as a stream-mode ability
                        // (see register_for_agent): it is dispatched
                        // through the stream handler, never this unary RPC
                        // adapter. Reaching here means the ability was
                        // mis-registered as RPC — fail loudly rather than
                        // silently collapsing the stream to one value.
                        Err(anyhow::anyhow!(
                            "host_stream ability '{bare_ability}' reached the unary \
                             RPC path; it must be invoked as a server-stream"
                        ))
                    }
                };
            }
        }

        let _ = (&agent_name, &entry, &loaders, args);
        Err(anyhow::anyhow!(
            "ability {bare_ability:?} is not executable: its manifest has no [exec] binding"
        ))
    })
}

/// Build an `<agent>.chat` handler. Same
/// shape as the boot-time registration in `register_for_agent`,
/// pulled out as a helper so the hot-add and boot paths produce
/// byte-identical handlers.
pub(crate) fn build_chat_handler_for(
    agent_name: String,
    entry: AgentEntry,
    loaders: Arc<Vec<Arc<dyn ContextLoader>>>,
) -> crate::daemon::ability::dispatch::LocalRpcHandler {
    Arc::new(move |args: Value| handler(&agent_name, &entry, &loaders, args))
}

fn usage_to_json(resp: &AgentResponse) -> Option<Value> {
    resp.usage.as_ref().map(|u| {
        json!({
            "input_tokens": u.input_tokens,
            "output_tokens": u.output_tokens,
            "cache_read_tokens": u.cache_read_tokens,
            "cached_input_tokens": u.cache_read_tokens,
            "cache_creation_tokens": u.cache_creation_tokens,
            "num_turns": u.num_turns,
            "total_cost_usd": u.total_cost_usd,
            "model": resp.model.clone(),
        })
    })
}

fn tool_calls_to_json(tool_calls: &[ToolCall]) -> Vec<Value> {
    tool_calls.iter().map(tool_call_to_json).collect()
}

fn tool_call_to_json(tool_call: &ToolCall) -> Value {
    let mut object = serde_json::Map::new();
    object.insert("ability".to_string(), json!(tool_call.ability.clone()));
    object.insert("args".to_string(), tool_call.args.clone());
    insert_optional_value(&mut object, "result", tool_call.result.clone());
    insert_optional_string(&mut object, "error", tool_call.error.as_deref());
    if let Some(elapsed_ms) = tool_call.elapsed_ms {
        object.insert("elapsed_ms".to_string(), json!(elapsed_ms));
    }
    insert_optional_string(&mut object, "tool_use_id", tool_call.tool_use_id.as_deref());
    insert_optional_string(
        &mut object,
        "mcp_tool_name",
        tool_call.mcp_tool_name.as_deref(),
    );
    insert_optional_string(&mut object, "request_id", tool_call.request_id.as_deref());
    insert_optional_string(&mut object, "ability_ura", tool_call.ability_ura.as_deref());
    insert_optional_string(
        &mut object,
        "invocation_ura",
        tool_call.invocation_ura.as_deref(),
    );
    insert_optional_string(&mut object, "caller_ura", tool_call.caller_ura.as_deref());
    insert_optional_string(&mut object, "callee_ura", tool_call.callee_ura.as_deref());
    insert_optional_string(&mut object, "subject_ura", tool_call.subject_ura.as_deref());
    Value::Object(object)
}

fn insert_optional_string(
    object: &mut serde_json::Map<String, Value>,
    key: &'static str,
    value: Option<&str>,
) {
    if let Some(value) = value {
        object.insert(key.to_string(), json!(value));
    }
}

fn insert_optional_value(
    object: &mut serde_json::Map<String, Value>,
    key: &'static str,
    value: Option<Value>,
) {
    if let Some(value) = value {
        object.insert(key.to_string(), value);
    }
}

/// Synthesise an `<agent>.discover` handler for a hot-added agent.
/// The handler closes over `agent_name` for caller identity and
/// re-loads `agents.json` per call so the discover ladder sees
/// every peer that exists *now*, including agents added after this
/// closure was built.
pub(crate) fn build_discover_handler_for(
    agent_name: String,
    dispatch_handle: Arc<std::sync::OnceLock<Arc<AxonAbilityCatalog>>>,
    federation_resolver: crate::daemon::ability::builtins::agents::discover::SharedDiscoverFederationResolver,
) -> crate::daemon::ability::dispatch::LocalRpcHandler {
    // Replicate the surface of `discover_ability::register_for_agent`
    // without going through that function (it expects a `&mut
    // AxonAbilityCatalog`, which we don't have here). The handler
    // re-loads agents on every call so a brand-new peer is visible
    // immediately — same hot-add story as the chat handler.
    let provider: crate::daemon::ability::builtins::agents::discover::AgentDirectoryProvider =
        Arc::new(|| {
            AgentAggregateRepository::load_snapshot()
                .map_err(|error| anyhow::anyhow!("load discover Agent aggregate: {error:#}"))
        });
    Arc::new(move |args: Value| {
        // Defer to the discover module's per-call entry. Public
        // entry exposed for this purpose (and test cases).
        crate::daemon::ability::builtins::agents::discover::dispatch(
            &agent_name,
            &provider,
            &dispatch_handle,
            federation_resolver.as_ref(),
            args,
        )
    })
}

/// Synthesise an `<agent>.invoke` handler for a hot-added agent.
/// Routes through the same builtin invoke entry the boot-time
/// registration uses.
pub(crate) fn build_invoke_handler_for(
    agent_name: String,
    dispatch_handle: Arc<std::sync::OnceLock<Arc<AxonAbilityCatalog>>>,
) -> crate::daemon::ability::dispatch::LocalRpcHandler {
    Arc::new(move |args: Value| {
        let registry = AgentAggregateRepository::load_snapshot()
            .map(|snapshot| snapshot.registered_agent_registry_projection())
            .map_err(|error| anyhow::anyhow!("load invoke Agent aggregate: {error:#}"))?;
        let provider: Arc<
            dyn Fn() -> crate::daemon::persistence::agent_registry::AgentRegistry + Send + Sync,
        > = Arc::new(move || registry.clone());
        crate::daemon::ability::builtins::agents::invoke::dispatch(
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
    invoke_direct_with_progress(agent_name, entry, loaders, args, None)
}

/// Execute one `<agent>.chat` turn directly in the current process and
/// return the typed RPC payload the daemon handler normally returns.
///
/// Why this helper exists:
///   * The daemon's registered RPC handler uses this logic.
///   * The EAL interpreter's `agent.chat(...)` fast path also needs the
///     same behaviour, but it must stay in the caller's process so the
///     driver's live stderr timeline is visible to `easynet agent send`.
///
/// `progress_tx` is optional. When present, the underlying driver emits
/// per-chunk progress into it while still returning the same final JSON
/// envelope as the RPC handler.
pub(crate) fn invoke_direct_with_progress(
    agent_name: &str,
    entry: &AgentEntry,
    loaders: &[Arc<dyn ContextLoader>],
    args: Value,
    progress_tx: Option<Arc<dyn Fn(serde_json::Value) + Send + Sync>>,
) -> anyhow::Result<Value> {
    let started = Instant::now();
    let mut parsed = ChatArgs::parse(&args)?;

    // The RPC entry point cannot return a stream. Surface the
    // mistake as a deterministic, actionable error rather than
    // silently dropping the flag.
    if parsed.stream {
        anyhow::bail!(
            "chat: `stream: true` is only valid via the subscribe entry point; \
             call subscribe with the same args instead of invoke"
        );
    }

    // Lifelong-sentinel resolution (see LIFELONG_SESSION_ID). Resolve
    // before any session_id consumer below: the pointer either yields
    // a concrete id (resume path, identical to the caller naming it
    // directly) or `None` (fresh-turn path; the id that turn ends up
    // with is bound as the pointer after the transcript write).
    let lifelong_requested = parsed.session_id.as_deref() == Some(LIFELONG_SESSION_ID);
    if lifelong_requested {
        parsed.session_id =
            crate::daemon::persistence::chat_sessions::lifelong_session(agent_name)?;
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
                    let loader_name = loader.name();
                    let err_msg = format!("{e}");
                    crate::op_event!(
                        component = chat,
                        kind = context_loader_failed,
                        level = "warn",
                        agent = agent_name,
                        loader = loader_name,
                        error = err_msg,
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
    let other_specs = if parsed.execution.isolation == DriverIsolation::Strict {
        Vec::new()
    } else {
        enumerate_other_agent_specs(agent_name)?
    };
    let cross_agent_hint = format_cross_agent_hint(&other_specs);

    // Materialise attachments to a delimited block. Failures bail
    // loud — attachments are explicit input, not best-effort
    // context, so a missing path is the operator's bug to fix.
    // Files-store root resolved only when a URA attachment is present:
    // root_from_env creates the store directory as a side effect, which
    // an attachment-less chat turn has no business doing.
    let files_root = if parsed.attachments.iter().any(AttachmentSpec::is_ura) {
        Some(crate::daemon::ability::builtins::resources::files_store::state::root_from_env()?)
    } else {
        None
    };
    let attachments_block = materialize_attachments(
        &parsed.attachments,
        entry.root_path.as_deref(),
        files_root.as_deref(),
    )?;

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
    let dispatch_call = || {
        if parsed.structured {
            crate::daemon::execution::mission::dispatch::send_external_structured(
                agent_name,
                entry,
                &parsed.prompt,
                parsed.system_prompt.as_deref(),
                driver_overrides,
                &parsed.execution,
                progress_tx.clone(),
            )
        } else if let Some(progress_tx) = progress_tx.clone() {
            crate::daemon::execution::mission::dispatch::send_external_with_overrides_and_progress(
                agent_name,
                entry,
                &parsed.prompt,
                composed_context.as_deref(),
                driver_overrides,
                Some(progress_tx),
            )
        } else {
            crate::daemon::execution::mission::dispatch::send_external_with_overrides(
                agent_name,
                entry,
                &parsed.prompt,
                composed_context.as_deref(),
                driver_overrides,
            )
        }
    };
    let response_result = if tokio::runtime::Handle::try_current().is_ok() {
        tokio::task::block_in_place(dispatch_call)
    } else {
        dispatch_call()
    };

    let resp = response_result?;
    let elapsed_ms = started.elapsed().as_millis() as u64;
    let session_id = ChatTurnSessionId::select(
        driver_with_resume.resume_thread_id.as_deref(),
        resp.thread_id.as_deref(),
        &session_id,
    )
    .into_string();
    let usage_value = usage_to_json(&resp).unwrap_or(Value::Null);
    let tool_calls_json = tool_calls_to_json(&resp.tool_calls);

    // Persist the turn to the agent's per-session JSONL transcript —
    // same contract as the `agent send` CLI path (run_send). Without
    // this, hub-routed chat (backend → daemon dispatch, e.g. the
    // Frontend Group page) leaves no trace for `chat.history.{list,
    // get}` to read. Best-effort: a disk failure must not break the
    // in-flight reply.
    crate::daemon::persistence::chat_sessions::write_turn_best_effort_with_elapsed(
        agent_name,
        &session_id,
        &parsed.prompt,
        &resp.content,
        &tool_calls_json,
        &usage_value,
        elapsed_ms,
    );
    // Bind (or re-affirm) the lifelong pointer after the turn is on
    // disk, so the next sentinel turn resumes this same thread.
    if lifelong_requested {
        crate::daemon::persistence::chat_sessions::set_lifelong_session_best_effort(
            agent_name,
            &session_id,
        );
    }

    Ok(json!({
        "session_id": session_id,
        "reply": resp.content,
        "skills_loaded": skills_loaded,
        "tool_calls": tool_calls_json,
        "timeline": resp.timeline,
        "context_used": Value::Array(context_used),
        "usage": usage_value,
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
///   `{"type": "done", "reply": "...", "tool_calls": [...], "timeline": [...], "context_used": [...], "usage": {...}, "elapsed_ms": N, "session_id": "..."}`
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
    let mut parsed = ChatArgs::parse(&args)?;
    // Lifelong-sentinel resolution — mirrors invoke_direct_with_progress;
    // see the comment there and on LIFELONG_SESSION_ID.
    let lifelong_requested = parsed.session_id.as_deref() == Some(LIFELONG_SESSION_ID);
    if lifelong_requested {
        parsed.session_id =
            crate::daemon::persistence::chat_sessions::lifelong_session(agent_name)?;
    }
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
                    let loader_name = loader.name();
                    let err_msg = format!("{e}");
                    crate::op_event!(
                        component = chat_stream,
                        kind = context_loader_failed,
                        level = "warn",
                        agent = agent_name,
                        loader = loader_name,
                        error = err_msg,
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
    let other_specs = if parsed.execution.isolation == DriverIsolation::Strict {
        Vec::new()
    } else {
        enumerate_other_agent_specs(agent_name)?
    };
    let cross_agent_hint = format_cross_agent_hint(&other_specs);
    // Files-store root resolved only when a URA attachment is present:
    // root_from_env creates the store directory as a side effect, which
    // an attachment-less chat turn has no business doing.
    let files_root = if parsed.attachments.iter().any(AttachmentSpec::is_ura) {
        Some(crate::daemon::ability::builtins::resources::files_store::state::root_from_env()?)
    } else {
        None
    };
    let attachments_block = materialize_attachments(
        &parsed.attachments,
        entry.root_path.as_deref(),
        files_root.as_deref(),
    )?;
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
    let system_prompt_owned = parsed.system_prompt.clone();
    let execution_owned = parsed.execution.clone();
    let structured = parsed.structured;
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
            let result = if structured {
                crate::daemon::execution::mission::dispatch::send_external_structured(
                    &agent_name_owned,
                    &entry_owned,
                    &prompt_owned,
                    system_prompt_owned.as_deref(),
                    Some(&driver_owned),
                    &execution_owned,
                    Some(progress_callback),
                )
            } else {
                crate::daemon::execution::mission::dispatch::send_external_with_overrides_and_progress(
                    &agent_name_owned,
                    &entry_owned,
                    &prompt_owned,
                    composed_context_owned.as_deref(),
                    Some(&driver_owned),
                    Some(progress_callback),
                )
            };
            let elapsed_ms = started.elapsed().as_millis() as u64;
            let frame = match result {
                Ok(resp) => {
                    let usage_value = usage_to_json(&resp).unwrap_or(Value::Null);
                    let tool_calls_json = tool_calls_to_json(&resp.tool_calls);
                    let resolved_session_id = ChatTurnSessionId::select(
                        resume_id_for_done.as_deref(),
                        resp.thread_id.as_deref(),
                        &session_id_for_thread,
                    )
                    .into_string();
                    // Persist the streamed turn too — same transcript
                    // contract as the RPC path (invoke_direct_with_progress).
                    crate::daemon::persistence::chat_sessions::write_turn_best_effort_with_elapsed(
                        &agent_name_owned,
                        &resolved_session_id,
                        &prompt_owned,
                        &resp.content,
                        &tool_calls_json,
                        &usage_value,
                        elapsed_ms,
                    );
                    if lifelong_requested {
                        crate::daemon::persistence::chat_sessions::set_lifelong_session_best_effort(
                            &agent_name_owned,
                            &resolved_session_id,
                        );
                    }
                    json!({
                        "type": "done",
                        "session_id": resolved_session_id,
                        "reply": resp.content,
                        "skills_loaded": skills_loaded_for_thread,
                        "tool_calls": tool_calls_json,
                        "timeline": resp.timeline,
                        "context_used": context_used_for_thread,
                        "usage": usage_value,
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
/// (`daemon::execution::mission::agent_ability_specs::abilities_for`) so an operator's
/// hand-edited manifest is reflected here too.
///
/// The `<agent>.chat` ability itself is never exposed as a tool to
/// the LLM (an agent calling its own chat would be infinite-recursion
/// bait); it is filtered out before any include/exclude rules apply.
#[cfg(test)]
fn enumerate_skills(agent_name: &str, entry: &AgentEntry, selection: &Selection) -> Vec<String> {
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
) -> Vec<crate::daemon::execution::mission::agent_ability_specs::AgentAbilitySpec> {
    if matches!(selection.mode, SelectionMode::None) {
        return Vec::new();
    }
    let self_chat = format!("{agent_name}.{ABILITY_VERB}");
    crate::daemon::execution::mission::agent_ability_specs::abilities_for(agent_name, entry)
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
/// daemon dispatcher) carries the call to agent B's executor-bound
/// handler.
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
) -> anyhow::Result<Vec<crate::daemon::execution::mission::agent_ability_specs::AgentAbilitySpec>> {
    let snapshot = AgentAggregateRepository::load_snapshot().map_err(|error| {
        anyhow::anyhow!("load cross-agent ability registry projection: {error:#}")
    })?;
    let mut out = Vec::new();
    for (other_name, other_entry) in snapshot.registered_agents() {
        if other_name == self_agent_name {
            continue;
        }
        let other_chat = format!("{other_name}.{ABILITY_VERB}");
        for spec in crate::daemon::execution::mission::agent_ability_specs::abilities_for(
            other_name,
            other_entry,
        ) {
            if spec.name() == other_chat {
                continue;
            }
            out.push(spec);
        }
    }
    Ok(out)
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

/// Read every attachment and assemble a single delimited markdown
/// block. Returns `Ok(None)` when the input list is empty so callers
/// skip the wrapper.
///
/// `Path` attachments embed file content inline (subject to the
/// 1 MiB budget). `Ura` attachments copy the files-store blob into
/// `<workspace_root>/uploads/<sha8>-<name>` and contribute only a
/// one-line note with the workspace-relative path — the driver runs
/// with cwd = workspace_root (dispatch's `ensure_from_directory`),
/// so the agent reads the file itself with its own tools.
///
/// `workspace_root` is the agent's root_path; `files_root` is the
/// content-addressed store root. Both are injected (not read from
/// env here) so tests stay parallel-safe; callers resolve
/// `files_root` lazily only when a URA attachment is present.
///
/// Failure modes (all loud — chat does not silently swallow these):
///   * any path on the fs.read blocked list (e.g. /dev/zero)
///   * file open/read/copy I/O failure
///   * encoding=utf8 on a non-UTF-8 byte sequence
///   * accumulated inline bytes exceed `ATTACHMENTS_BUDGET_BYTES`
///   * a URA that does not parse as a `<u>.files` resource, names a
///     blob missing from the store, or arrives when the agent has no
///     workspace / the store root is unavailable
fn materialize_attachments(
    specs: &[AttachmentSpec],
    workspace_root: Option<&std::path::Path>,
    files_root: Option<&std::path::Path>,
) -> anyhow::Result<Option<String>> {
    if specs.is_empty() {
        return Ok(None);
    }
    use std::io::Read;
    let mut out = String::from("## Attachments\n\n");
    let mut budget = ATTACHMENTS_BUDGET_BYTES;
    for (idx, spec) in specs.iter().enumerate() {
        let (path, encoding) = match spec {
            AttachmentSpec::Ura { ura, filename } => {
                let note = materialize_ura_attachment(
                    idx,
                    ura,
                    filename.as_deref(),
                    workspace_root,
                    files_root,
                )?;
                out.push_str(&note);
                continue;
            }
            AttachmentSpec::Path { path, encoding } => (path, *encoding),
        };
        if crate::daemon::ability::builtins::device_control::files::is_blocked_read_path_for_chat(
            path,
        ) {
            anyhow::bail!("chat: attachments[{idx}] {path:?} is on the blocked-device path list");
        }
        let fs_path = std::path::Path::new(path);
        let metadata = std::fs::metadata(fs_path)
            .map_err(|e| anyhow::anyhow!("chat: attachments[{idx}] stat {path:?}: {e}"))?;
        if metadata.len() as usize > budget {
            anyhow::bail!(
                "chat: attachments[{idx}] {path:?} ({} bytes) would exceed the {} byte \
                 attachments budget",
                metadata.len(),
                ATTACHMENTS_BUDGET_BYTES
            );
        }
        let mut file = std::fs::File::open(fs_path)
            .map_err(|e| anyhow::anyhow!("chat: attachments[{idx}] open {path:?}: {e}"))?;
        // +1 over budget so an oversized file (e.g. one that grew
        // between stat and open) still fails loud rather than
        // truncating silently.
        let mut limited = file.by_ref().take(budget as u64 + 1);
        let mut bytes: Vec<u8> = Vec::with_capacity(metadata.len() as usize);
        limited
            .read_to_end(&mut bytes)
            .map_err(|e| anyhow::anyhow!("chat: attachments[{idx}] read {path:?}: {e}"))?;
        if bytes.len() > budget {
            anyhow::bail!(
                "chat: attachments[{idx}] {path:?} grew past the {} byte attachments budget \
                 mid-read",
                ATTACHMENTS_BUDGET_BYTES
            );
        }
        budget = budget.saturating_sub(bytes.len());

        let body = match encoding {
            AttachmentEncoding::Utf8 => {
                let text = std::str::from_utf8(&bytes).map_err(|_| {
                    anyhow::anyhow!(
                        "chat: attachments[{idx}] {path:?} is not valid UTF-8; \
                         use encoding=\"base64\""
                    )
                })?;
                format!("<file path={path:?} encoding=\"utf8\">\n{text}\n</file>\n")
            }
            AttachmentEncoding::Base64 => {
                let encoded = base64_encode(&bytes);
                format!("<file path={path:?} encoding=\"base64\">\n{encoded}\n</file>\n")
            }
        };
        out.push_str(&body);
    }
    Ok(Some(out))
}

/// Materialise one files-store blob into the workspace's `uploads/`
/// directory and return its one-line context note. The copy target
/// is `<sha8>-<sanitised filename>` — sha-prefixed so two uploads
/// sharing a display name cannot collide, deterministic so re-sending
/// the same turn is idempotent.
fn materialize_ura_attachment(
    idx: usize,
    ura: &str,
    filename: Option<&str>,
    workspace_root: Option<&std::path::Path>,
    files_root: Option<&std::path::Path>,
) -> anyhow::Result<String> {
    let workspace = workspace_root.ok_or_else(|| {
        anyhow::anyhow!(
            "chat: attachments[{idx}] is a URA but this agent has no workspace \
             (registry row is missing root_path)"
        )
    })?;
    let files_root = files_root.ok_or_else(|| {
        anyhow::anyhow!("chat: attachments[{idx}] is a URA but the files store root is unavailable")
    })?;
    let sha =
        crate::daemon::ability::builtins::resources::files_store::handlers::sha256_from_ura(ura)
            .map_err(|e| anyhow::anyhow!("chat: attachments[{idx}]: {e}"))?;
    let blob = crate::daemon::ability::builtins::resources::files_store::state::blob_path(
        files_root, &sha,
    )
    .map_err(|e| anyhow::anyhow!("chat: attachments[{idx}] resolve {sha}: {e}"))?;
    let size = std::fs::metadata(&blob)
        .map_err(|e| {
            anyhow::anyhow!(
                "chat: attachments[{idx}] {ura} names a blob missing from the files \
                 store: {e}"
            )
        })?
        .len();
    let uploads_dir = workspace.join("uploads");
    std::fs::create_dir_all(&uploads_dir)
        .map_err(|e| anyhow::anyhow!("chat: attachments[{idx}] create {uploads_dir:?}: {e}"))?;
    let target_name = sanitized_upload_name(filename, &sha);
    let target = uploads_dir.join(&target_name);
    std::fs::copy(&blob, &target).map_err(|e| {
        anyhow::anyhow!("chat: attachments[{idx}] copy {blob:?} -> {target:?}: {e}")
    })?;
    let rel = format!("uploads/{target_name}");
    Ok(format!(
        "- `{rel}` ({size} bytes) — user-uploaded file {ura}, materialised at this \
         workspace-relative path; read it from there with your file tools.\n"
    ))
}

/// Display name for a materialised upload: `<sha8>-<basename>`.
/// Only the final path component of the caller-supplied filename
/// survives (no traversal, no separators); empty or pathological
/// names fall back to "file".
fn sanitized_upload_name(filename: Option<&str>, sha256_hex: &str) -> String {
    let base = filename
        .map(str::trim)
        .filter(|s| !s.is_empty())
        .and_then(|s| {
            std::path::Path::new(s)
                .file_name()
                .map(|os| os.to_string_lossy().to_string())
        })
        .unwrap_or_else(|| "file".to_string());
    format!("{}-{base}", &sha256_hex[..8])
}

/// Minimal base64 encoder (standard alphabet, with padding). Lifted
/// here so chat_ability does not pull in a new dep just for the
/// attachments path; the alphabet + padding are stable enough to
/// inline. Mirrors RFC 4648 §4.
fn base64_encode(input: &[u8]) -> String {
    const ALPHABET: &[u8; 64] = b"ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghijklmnopqrstuvwxyz0123456789+/";
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

fn format_skills_hint(
    skills: &[crate::daemon::execution::mission::agent_ability_specs::AgentAbilitySpec],
) -> Option<String> {
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
    others: &[crate::daemon::execution::mission::agent_ability_specs::AgentAbilitySpec],
) -> Option<String> {
    if others.is_empty() {
        return None;
    }
    let mut out = String::from("## Available abilities (other agents on this device)\n\n");
    out.push_str(
        "These abilities are advertised by other agents installed alongside you. They are \
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
    system_prompt: Option<String>,
    structured: bool,
    session_id: Option<String>,
    skills: Selection,
    context_loaders: Selection,
    driver: DriverOverrides,
    stream: bool,
    attachments: Vec<AttachmentSpec>,
    execution: AgentExecution,
}

#[derive(Debug, Clone)]
enum AttachmentSpec {
    /// Daemon-local file embedded inline in the prompt's context
    /// block (the original v1 shape).
    Path {
        path: String,
        encoding: AttachmentEncoding,
    },
    /// Files-store blob addressed by its v4.1.5 resource URA
    /// (`easynet:///r/<realm>/resource/<u>.files/<sha256>`). Not
    /// inlined: the blob is materialised into the agent workspace's
    /// `uploads/` directory and the context block lists its
    /// workspace-relative path, so the agent reads it with its own
    /// file tools (works for files far past the inline budget).
    Ura {
        ura: String,
        filename: Option<String>,
    },
}

impl AttachmentSpec {
    fn is_ura(&self) -> bool {
        matches!(self, AttachmentSpec::Ura { .. })
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
enum AttachmentEncoding {
    #[default]
    Utf8,
    Base64,
}

impl ChatArgs {
    fn parse(args: &Value) -> anyhow::Result<Self> {
        let obj = args
            .as_object()
            .ok_or_else(|| anyhow::anyhow!("chat: arguments must be a JSON object"))?;
        reject_unknown_fields(
            obj,
            "chat",
            &[
                "prompt",
                "messages",
                "context",
                "session_id",
                "skills",
                "context_loaders",
                "driver",
                "stream",
                "attachments",
                "execution",
            ],
        )?;
        let context = optional_string_field(obj, "context")?;
        let session_id = optional_string_field(obj, "session_id")?;
        let (prompt, system_prompt, structured) = match (obj.get("prompt"), obj.get("messages")) {
            (Some(_), Some(_)) => {
                anyhow::bail!("chat: exactly one of `prompt` or `messages` is required")
            }
            (Some(value), None) => {
                let prompt = value
                    .as_str()
                    .ok_or_else(|| anyhow::anyhow!("chat: `prompt` must be a string"))?;
                if prompt.is_empty() {
                    anyhow::bail!("chat: `prompt` must not be empty");
                }
                (prompt.to_string(), None, false)
            }
            (None, Some(value)) => {
                let (system, user) = parse_structured_messages(value)?;
                (user, system, true)
            }
            (None, None) => {
                anyhow::bail!("chat: exactly one of `prompt` or `messages` is required")
            }
        };
        let selection_default = || {
            if structured {
                Selection::none()
            } else {
                Selection::default()
            }
        };
        let skills = obj
            .get("skills")
            .map(|value| Selection::parse(value, "skills"))
            .transpose()?
            .unwrap_or_else(selection_default);
        let context_loaders = obj
            .get("context_loaders")
            .map(|value| Selection::parse(value, "context_loaders"))
            .transpose()?
            .unwrap_or_else(selection_default);
        let driver = obj
            .get("driver")
            .map(parse_driver_overrides)
            .transpose()?
            .unwrap_or_default();
        let stream = optional_bool_field(obj, "stream")?.unwrap_or(false);
        let attachments = parse_attachments(obj.get("attachments"))?;
        let execution = parse_execution(obj.get("execution"), structured)?;
        if structured {
            if context.is_some() {
                anyhow::bail!("chat: `context` cannot be combined with `messages`");
            }
            if session_id.is_some() {
                anyhow::bail!("chat: structured single-turn messages cannot resume `session_id`");
            }
            if !skills.is_none() || !context_loaders.is_none() {
                anyhow::bail!(
                    "chat: structured benchmark messages require skills.mode and context_loaders.mode to be `none`"
                );
            }
            if !attachments.is_empty() {
                anyhow::bail!("chat: strict structured messages do not accept attachments");
            }
            if execution.isolation != DriverIsolation::Strict {
                anyhow::bail!("chat: structured messages require execution.isolation `strict`");
            }
            if execution.cwd.is_none() {
                anyhow::bail!("chat: structured messages require execution.cwd");
            }
        }
        Ok(Self {
            prompt,
            context,
            system_prompt,
            structured,
            session_id,
            skills,
            context_loaders,
            driver,
            stream,
            attachments,
            execution,
        })
    }
}

fn parse_structured_messages(value: &Value) -> anyhow::Result<(Option<String>, String)> {
    let messages = value
        .as_array()
        .ok_or_else(|| anyhow::anyhow!("chat: `messages` must be an array"))?;
    if !(1..=2).contains(&messages.len()) {
        anyhow::bail!(
            "chat: `messages` supports exactly one user message with one optional preceding system message"
        );
    }
    let mut parsed = Vec::with_capacity(messages.len());
    for (index, message) in messages.iter().enumerate() {
        let object = message
            .as_object()
            .ok_or_else(|| anyhow::anyhow!("chat: messages[{index}] must be an object"))?;
        reject_unknown_fields(
            object,
            &format!("chat: messages[{index}]"),
            &["role", "content"],
        )?;
        let role = object
            .get("role")
            .and_then(Value::as_str)
            .ok_or_else(|| anyhow::anyhow!("chat: messages[{index}].role must be a string"))?;
        let content = object
            .get("content")
            .and_then(Value::as_str)
            .ok_or_else(|| anyhow::anyhow!("chat: messages[{index}].content must be a string"))?;
        if content.is_empty() {
            anyhow::bail!("chat: messages[{index}].content must not be empty");
        }
        parsed.push((role, content));
    }
    match parsed.as_slice() {
        [("user", user)] => Ok((None, (*user).to_string())),
        [("system", system), ("user", user)] => {
            Ok((Some((*system).to_string()), (*user).to_string()))
        }
        _ => anyhow::bail!(
            "chat: `messages` must be `[user]` or `[system, user]`; assistant history and multi-turn input are not supported"
        ),
    }
}

fn parse_execution(value: Option<&Value>, structured: bool) -> anyhow::Result<AgentExecution> {
    let Some(value) = value else {
        return Ok(AgentExecution {
            isolation: if structured {
                DriverIsolation::Strict
            } else {
                DriverIsolation::Agent
            },
            ..AgentExecution::default()
        });
    };
    let object = value
        .as_object()
        .ok_or_else(|| anyhow::anyhow!("chat: `execution` must be an object"))?;
    reject_unknown_fields(
        object,
        "chat: execution",
        &["cwd", "timeout_ms", "isolation"],
    )?;
    let cwd = optional_string_field(object, "cwd")?.map(PathBuf::from);
    if let Some(path) = cwd.as_deref() {
        if path.as_os_str().is_empty()
            || path
                .components()
                .any(|part| !matches!(part, Component::Normal(_)))
        {
            anyhow::bail!(
                "chat: execution.cwd must be a non-empty agent-root-relative descendant path"
            );
        }
    }
    let timeout = match object.get("timeout_ms") {
        None => None,
        Some(Value::Number(number)) => {
            let value = number.as_u64().ok_or_else(|| {
                anyhow::anyhow!("chat: execution.timeout_ms must be a positive integer")
            })?;
            if value == 0 || value > MAX_INVOCATION_TIMEOUT_MS {
                anyhow::bail!(
                    "chat: execution.timeout_ms must be between 1 and {MAX_INVOCATION_TIMEOUT_MS}"
                );
            }
            Some(Duration::from_millis(value))
        }
        Some(_) => {
            anyhow::bail!("chat: execution.timeout_ms must be a positive integer")
        }
    };
    let isolation = match object.get("isolation").and_then(Value::as_str) {
        None if structured => DriverIsolation::Strict,
        None => DriverIsolation::Agent,
        Some("agent") => DriverIsolation::Agent,
        Some("strict") => DriverIsolation::Strict,
        Some(other) => anyhow::bail!(
            "chat: invalid execution.isolation {other:?}; expected `agent` or `strict`"
        ),
    };
    Ok(AgentExecution {
        cwd,
        timeout,
        isolation,
    })
}

fn reject_unknown_fields(
    obj: &Map<String, Value>,
    context: &str,
    allowed: &[&str],
) -> anyhow::Result<()> {
    let mut unknown: Vec<&str> = obj
        .keys()
        .map(String::as_str)
        .filter(|field| !allowed.contains(field))
        .collect();
    unknown.sort_unstable();
    if unknown.is_empty() {
        return Ok(());
    }
    anyhow::bail!(
        "{context}: unsupported field(s): {}",
        unknown
            .iter()
            .map(|field| format!("`{field}`"))
            .collect::<Vec<_>>()
            .join(", ")
    )
}

fn optional_string_field(
    obj: &Map<String, Value>,
    field: &'static str,
) -> anyhow::Result<Option<String>> {
    match obj.get(field) {
        None => Ok(None),
        Some(value) => Ok(Some(
            value
                .as_str()
                .ok_or_else(|| anyhow::anyhow!("chat: `{field}` must be a string"))?
                .to_string(),
        )),
    }
}

fn optional_bool_field(
    obj: &Map<String, Value>,
    field: &'static str,
) -> anyhow::Result<Option<bool>> {
    match obj.get(field) {
        None => Ok(None),
        Some(value) => value
            .as_bool()
            .ok_or_else(|| anyhow::anyhow!("chat: `{field}` must be a boolean"))
            .map(Some),
    }
}

fn optional_attachment_string_field(
    obj: &Map<String, Value>,
    idx: usize,
    field: &'static str,
) -> anyhow::Result<Option<String>> {
    match obj.get(field) {
        None => Ok(None),
        Some(value) => Ok(Some(
            value
                .as_str()
                .ok_or_else(|| {
                    anyhow::anyhow!("chat: attachments[{idx}].{field} must be a string")
                })?
                .to_string(),
        )),
    }
}

/// Parse the optional `attachments` array into typed AttachmentSpecs.
/// Absent/null → empty Vec; present-but-not-an-array → loud error so
/// the caller sees the typo at the API boundary. Each entry names its
/// source with exactly one of `path` (daemon-local file, inlined) or
/// `ura` (files-store blob, materialised into the workspace).
fn parse_attachments(value: Option<&Value>) -> anyhow::Result<Vec<AttachmentSpec>> {
    let arr = match value {
        None | Some(Value::Null) => return Ok(Vec::new()),
        Some(Value::Array(items)) => items,
        Some(_) => anyhow::bail!("chat: `attachments` must be an array of objects"),
    };
    let mut out = Vec::with_capacity(arr.len());
    for (idx, item) in arr.iter().enumerate() {
        let obj = item
            .as_object()
            .ok_or_else(|| anyhow::anyhow!("chat: attachments[{idx}] must be an object"))?;
        reject_unknown_fields(
            obj,
            &format!("chat: attachments[{idx}]"),
            &["path", "ura", "filename", "encoding"],
        )?;
        let path = optional_attachment_string_field(obj, idx, "path")?;
        let ura = optional_attachment_string_field(obj, idx, "ura")?;
        match (path.as_deref(), ura.as_deref()) {
            (Some(_), Some(_)) => {
                anyhow::bail!("chat: attachments[{idx}] must set `path` or `ura`, not both")
            }
            (Some(path), None) => {
                if path.is_empty() {
                    anyhow::bail!("chat: attachments[{idx}].path must not be empty");
                }
                if obj.contains_key("filename") {
                    anyhow::bail!(
                        "chat: attachments[{idx}].filename is only valid with `ura` — \
                         path attachments are embedded inline"
                    );
                }
                let encoding =
                    match optional_attachment_string_field(obj, idx, "encoding")?.as_deref() {
                        None => AttachmentEncoding::default(),
                        Some("utf8") => AttachmentEncoding::Utf8,
                        Some("base64") => AttachmentEncoding::Base64,
                        Some(other) => anyhow::bail!(
                            "chat: attachments[{idx}].encoding must be \"utf8\" or \"base64\" \
                         (got {other:?})"
                        ),
                    };
                out.push(AttachmentSpec::Path {
                    path: path.to_string(),
                    encoding,
                });
            }
            (None, Some(ura)) => {
                if ura.is_empty() {
                    anyhow::bail!("chat: attachments[{idx}].ura must not be empty");
                }
                if obj.contains_key("encoding") {
                    anyhow::bail!(
                        "chat: attachments[{idx}].encoding is only valid with `path` — \
                         URA attachments are materialised to disk, not inlined"
                    );
                }
                let filename = optional_attachment_string_field(obj, idx, "filename")?;
                out.push(AttachmentSpec::Ura {
                    ura: ura.to_string(),
                    filename,
                });
            }
            (None, None) => {
                anyhow::bail!("chat: attachments[{idx}] requires `path` (string) or `ura` (string)")
            }
        }
    }
    Ok(out)
}

/// Selection mode shared by `skills` and `context_loaders`. The
/// duplicated structure is intentional: callers reason about each
/// independently and reusing the type makes "what does include mean
/// here" an obvious cross-reference.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
enum SelectionMode {
    #[default]
    Auto,
    None,
    Explicit,
}

#[derive(Debug, Clone, Default)]
struct Selection {
    mode: SelectionMode,
    include: Vec<String>,
    exclude: Vec<String>,
}

impl Selection {
    fn none() -> Self {
        Self {
            mode: SelectionMode::None,
            include: Vec::new(),
            exclude: Vec::new(),
        }
    }

    fn is_none(&self) -> bool {
        self.mode == SelectionMode::None && self.include.is_empty() && self.exclude.is_empty()
    }

    fn parse(value: &Value, field: &'static str) -> anyhow::Result<Self> {
        let obj = value
            .as_object()
            .ok_or_else(|| anyhow::anyhow!("chat: {field} must be an object"))?;
        reject_unknown_fields(
            obj,
            &format!("chat: {field}"),
            &["mode", "include", "exclude"],
        )?;
        let mode = match optional_selection_mode_field(obj, field)? {
            None => SelectionMode::Auto,
            Some(SelectionMode::Auto) => SelectionMode::Auto,
            Some(SelectionMode::None) => SelectionMode::None,
            Some(SelectionMode::Explicit) => SelectionMode::Explicit,
        };
        let include = string_array(obj.get("include"), &format!("{field}.include"))?;
        let exclude = string_array(obj.get("exclude"), &format!("{field}.exclude"))?;
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

fn optional_selection_mode_field(
    obj: &Map<String, Value>,
    field: &'static str,
) -> anyhow::Result<Option<SelectionMode>> {
    match obj.get("mode") {
        None => Ok(None),
        Some(value) => match value
            .as_str()
            .ok_or_else(|| anyhow::anyhow!("chat: {field}.mode must be a string"))?
        {
            "auto" => Ok(Some(SelectionMode::Auto)),
            "none" => Ok(Some(SelectionMode::None)),
            "explicit" => Ok(Some(SelectionMode::Explicit)),
            other => anyhow::bail!(
                "chat: invalid {field}.mode {other:?}; expected one of \"auto\", \"none\", \"explicit\""
            ),
        },
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
    let model = match obj.get("model") {
        None => None,
        Some(value) => Some(
            value
                .as_str()
                .ok_or_else(|| anyhow::anyhow!("chat: driver.model must be a string"))?
                .to_string(),
        ),
    };
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
    let mut unknown: Vec<&str> = obj
        .keys()
        .map(String::as_str)
        .filter(|field| !matches!(*field, "model" | "temperature" | "max_tokens"))
        .collect();
    unknown.sort_unstable();
    if !unknown.is_empty() {
        anyhow::bail!(
            "chat: unsupported driver field(s): {}. Canonical chat lifecycle uses top-level \
             `session_id`; driver-shaped lifecycle or runtime knobs are not accepted.",
            unknown
                .iter()
                .map(|field| format!("driver.{field}"))
                .collect::<Vec<_>>()
                .join(", "),
        );
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
        // Resume state is derived only from the caller's top-level
        // `session_id`. The parser rejects `driver.resume_thread_id`
        // above, so this field cannot become a second lifecycle
        // input surface.
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
/// the id to `exec resume`. Strings that match the chat ability's
/// locally minted id shape (`<32-hex>-<16-hex>`) are intentionally NOT
/// accepted — no resume-capable driver owns those ids, so forwarding
/// them would turn a local session token into a driver UUID parse
/// failure.
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

/// Canonical lifecycle state for the chat session id surfaced to callers.
///
/// The selector is shared by RPC and stream terminal frames:
/// 1. `ResumeRequested` preserves the caller-owned driver thread id.
/// 2. `DriverMinted` adopts a fresh driver thread id for future resume.
/// 3. `LocalResolved` uses the handler-resolved local session handle.
#[derive(Debug, Clone, PartialEq, Eq)]
enum ChatTurnSessionId {
    ResumeRequested(String),
    DriverMinted(String),
    LocalResolved(String),
}

impl ChatTurnSessionId {
    fn select(
        resume_thread_id: Option<&str>,
        driver_thread_id: Option<&str>,
        local_session_id: &str,
    ) -> Self {
        if let Some(resume_id) = resume_thread_id {
            Self::ResumeRequested(resume_id.to_string())
        } else if let Some(driver_id) = driver_thread_id {
            Self::DriverMinted(driver_id.to_string())
        } else {
            Self::LocalResolved(local_session_id.to_string())
        }
    }

    fn into_string(self) -> String {
        match self {
            Self::ResumeRequested(id) | Self::DriverMinted(id) | Self::LocalResolved(id) => id,
        }
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
    use crate::daemon::persistence::agent_registry::{AgentRegistry, AgentType};

    fn entry() -> AgentEntry {
        AgentEntry::new(AgentType::ClaudeCode, None)
    }

    fn agent_chat_test_catalog() -> AxonAbilityCatalog {
        AxonAbilityCatalog::new_test_metadata_for_device_authority(
            "easynet:///r/test/device/agent-chat",
        )
    }

    #[test]
    fn register_mounts_one_handler_per_agent() {
        // Hold the env lock: register() consults HOME-rooted registry
        // state, so a concurrent HOME-mutating test must not race it.
        let _home = crate::cli::commands::test_support::HomeGuard::new();
        let mut reg = agent_chat_test_catalog();
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
        assert!(
            reg.resolve_rpc("charlie.chat").is_none(),
            "lookup miss must stay a miss; hot agents are materialised through HotAgentRegistrar"
        );
    }

    #[test]
    fn register_does_not_mount_unbound_manifest_as_chat_route() {
        let _g = crate::cli::commands::test_support::HomeGuard::new();
        let root = crate::daemon::persistence::config::agents_root().join("alice");
        let abilities_dir = root.join("abilities");
        std::fs::create_dir_all(&abilities_dir).expect("abilities dir");
        std::fs::write(
            root.join("agent.toml"),
            "name = \"alice\"\nruntime = \"claude-code\"\n",
        )
        .expect("agent.toml");
        let manifest = crate::daemon::ability::manifest::AbilityManifest::new(
            "echo",
            "Unbound manifest.",
            json!({"type": "object"}),
        )
        .expect("manifest");
        std::fs::write(
            abilities_dir.join("echo.ability.toml"),
            manifest.to_toml_string().expect("manifest toml"),
        )
        .expect("ability manifest");

        let mut entry = entry();
        entry.root_path = Some(root);
        let mut agents = AgentRegistry::default();
        agents.agents.insert("alice".into(), entry);
        let mut reg = agent_chat_test_catalog();
        register(
            &mut reg,
            &agents,
            Arc::new(Vec::new()),
            Arc::new(std::sync::OnceLock::new()),
        );

        assert!(reg.get_rpc("alice.chat").is_some());
        assert!(
            reg.get_rpc("alice.echo").is_none(),
            "manifest without [exec] must not be routed through an LLM-mediated handler"
        );
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
        let result = stream_handler("alice", &entry, &[], json!({"prompt": "hi"}));
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
    fn lifelong_sentinel_is_not_a_driver_thread_id() {
        // The sentinel must never reach the driver as a resume id —
        // resolution replaces it before the resume-shape check runs,
        // and even if it leaked, the shape check rejects it.
        assert!(!looks_like_thread_id(LIFELONG_SESSION_ID));
    }

    #[test]
    fn stream_handler_resolves_lifelong_sentinel_against_bound_pointer() {
        // With a lifelong pointer bound, a sentinel turn must surface
        // the bound concrete id in the leading `session` frame — the
        // literal "lifelong" never appears on the wire as a session id.
        let _g = crate::cli::commands::test_support::HomeGuard::new();
        let bound = "38e5640c-6843-4f15-8f3a-2c8de75d0209";
        crate::daemon::persistence::chat_sessions::write_turn(
            "alice",
            bound,
            "seed",
            "seed",
            &[],
            &json!({}),
        )
        .expect("seed bound session");
        crate::daemon::persistence::chat_sessions::set_lifelong_session("alice", bound)
            .expect("bind");
        let entry = entry();
        let source = stream_handler(
            "alice",
            &entry,
            &[],
            json!({"prompt": "hi", "session_id": LIFELONG_SESSION_ID}),
        )
        .expect("snapshot construction must succeed even if dispatch will fail");
        match source {
            StreamSource::SnapshotThenLive(snapshot, _rx) => {
                assert_eq!(
                    snapshot[0].get("session_id").and_then(Value::as_str),
                    Some(bound),
                );
            }
            other => panic!("expected SnapshotThenLive, got {other:?}"),
        }
    }

    #[test]
    fn stream_handler_unbound_lifelong_sentinel_mints_fresh_id() {
        // No pointer bound: the sentinel falls through to the
        // fresh-turn path, so the session frame carries a minted id,
        // not the sentinel literal.
        let _g = crate::cli::commands::test_support::HomeGuard::new();
        let entry = entry();
        let source = stream_handler(
            "alice",
            &entry,
            &[],
            json!({"prompt": "hi", "session_id": LIFELONG_SESSION_ID}),
        )
        .expect("snapshot construction must succeed even if dispatch will fail");
        match source {
            StreamSource::SnapshotThenLive(snapshot, _rx) => {
                let sid = snapshot[0]
                    .get("session_id")
                    .and_then(Value::as_str)
                    .expect("session frame carries an id");
                assert_ne!(sid, LIFELONG_SESSION_ID);
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
        // makes this test see the empty baseline every time, which
        // is what the assertion expects.
        let _g = crate::cli::commands::test_support::HomeGuard::new();
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
    fn parse_accepts_canonical_minimal_prompt_args() {
        let args = ChatArgs::parse(&json!({"prompt": "hi"})).unwrap();
        assert_eq!(args.prompt, "hi");
        assert!(args.context.is_none());
        assert!(args.session_id.is_none());
        assert!(!args.stream);
        // Defaults: skills auto, context_loaders auto.
        assert_eq!(args.skills.mode, SelectionMode::Auto);
        assert_eq!(args.context_loaders.mode, SelectionMode::Auto);
        assert!(!args.structured);
        assert_eq!(args.execution.isolation, DriverIsolation::Agent);
    }

    #[test]
    fn parse_accepts_strict_structured_system_and_user_messages() {
        let args = ChatArgs::parse(&json!({
            "messages": [
                {"role": "system", "content": "translate SIGNAL"},
                {"role": "user", "content": "count cases"}
            ],
            "execution": {
                "cwd": "benchmark/run-1",
                "timeout_ms": 300000
            }
        }))
        .unwrap();
        assert!(args.structured);
        assert_eq!(args.system_prompt.as_deref(), Some("translate SIGNAL"));
        assert_eq!(args.prompt, "count cases");
        assert_eq!(args.execution.isolation, DriverIsolation::Strict);
        assert_eq!(
            args.execution.cwd.as_deref(),
            Some(std::path::Path::new("benchmark/run-1"))
        );
        assert_eq!(args.execution.timeout, Some(Duration::from_secs(300)));
        assert!(args.skills.is_none());
        assert!(args.context_loaders.is_none());
    }

    #[test]
    fn parse_accepts_strict_structured_user_only_message() {
        let args = ChatArgs::parse(&json!({
            "messages": [{"role": "user", "content": "count cases"}],
            "execution": {"cwd": "benchmark/run-2"}
        }))
        .unwrap();
        assert!(args.system_prompt.is_none());
        assert_eq!(args.prompt, "count cases");
    }

    #[test]
    fn parse_rejects_prompt_and_messages_together() {
        let err = ChatArgs::parse(&json!({
            "prompt": "prompt-shape",
            "messages": [{"role": "user", "content": "structured"}],
            "execution": {"cwd": "benchmark/run"}
        }))
        .unwrap_err();
        assert!(format!("{err}").contains("exactly one"));
    }

    #[test]
    fn parse_rejects_structured_assistant_or_multiturn_history() {
        for messages in [
            json!([
                {"role": "assistant", "content": "prior"},
                {"role": "user", "content": "next"}
            ]),
            json!([
                {"role": "system", "content": "rules"},
                {"role": "user", "content": "first"},
                {"role": "user", "content": "second"}
            ]),
        ] {
            let err = ChatArgs::parse(&json!({
                "messages": messages,
                "execution": {"cwd": "benchmark/run"}
            }))
            .unwrap_err();
            let message = format!("{err}");
            assert!(message.contains("messages"));
        }
    }

    #[test]
    fn parse_rejects_structured_ambient_context_surfaces() {
        for extra in [
            json!({"context": "ambient"}),
            json!({"session_id": "chat-existing"}),
            json!({"attachments": [{"path": "secret"}]}),
            json!({"skills": {"mode": "auto"}}),
            json!({"context_loaders": {"mode": "explicit", "include": ["memory"]}}),
            json!({"execution": {"cwd": "benchmark/run", "isolation": "agent"}}),
        ] {
            let mut request = json!({
                "messages": [{"role": "user", "content": "case"}],
                "execution": {"cwd": "benchmark/run"}
            });
            request
                .as_object_mut()
                .unwrap()
                .extend(extra.as_object().unwrap().clone());
            assert!(ChatArgs::parse(&request).is_err(), "request={request}");
        }
    }

    #[test]
    fn parse_rejects_unconfined_or_unbounded_execution() {
        for execution in [
            json!({"cwd": "/tmp/escape"}),
            json!({"cwd": "../escape"}),
            json!({"cwd": "benchmark/run", "timeout_ms": 0}),
            json!({"cwd": "benchmark/run", "timeout_ms": MAX_INVOCATION_TIMEOUT_MS + 1}),
        ] {
            let err = ChatArgs::parse(&json!({
                "messages": [{"role": "user", "content": "case"}],
                "execution": execution
            }))
            .unwrap_err();
            let message = format!("{err}");
            assert!(message.contains("execution"), "{message}");
        }
    }

    #[test]
    fn parse_accepts_canonical_prompt_and_context_args() {
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
    fn parse_rejects_unknown_top_level_fields() {
        let err = ChatArgs::parse(&json!({
            "prompt": "hi",
            "driver_context": "second-shape"
        }))
        .unwrap_err();
        let msg = format!("{err}");
        assert!(msg.contains("unsupported field"));
        assert!(msg.contains("driver_context"));
    }

    #[test]
    fn parse_rejects_wrongly_typed_optional_string_fields() {
        for field in ["context", "session_id"] {
            let mut payload = json!({"prompt": "hi"});
            payload
                .as_object_mut()
                .expect("object payload")
                .insert(field.to_string(), json!(123));
            let err = ChatArgs::parse(&payload).unwrap_err();
            let msg = format!("{err}");
            assert!(
                msg.contains(&format!("`{field}` must be a string")),
                "wrong error for {field}: {msg}"
            );
        }
    }

    #[test]
    fn parse_rejects_wrongly_typed_stream_flag() {
        let err = ChatArgs::parse(&json!({
            "prompt": "hi",
            "stream": "yes"
        }))
        .unwrap_err();
        assert!(format!("{err}").contains("`stream` must be a boolean"));
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
        assert!(
            msg.contains("temperature"),
            "msg should name the knob: {msg}"
        );
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
        assert!(
            msg.contains("max_tokens"),
            "msg should name the knob: {msg}"
        );
        assert!(msg.contains("not supported"), "msg should explain: {msg}");
    }

    #[test]
    fn parse_rejects_non_string_driver_model() {
        let err = ChatArgs::parse(&json!({
            "prompt": "hi",
            "driver": {"model": 42}
        }))
        .unwrap_err();
        assert!(format!("{err}").contains("driver.model must be a string"));
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
    fn parse_rejects_driver_resume_thread_id_as_second_lifecycle_surface() {
        let err = ChatArgs::parse(&json!({
            "prompt": "hi",
            "driver": {"resume_thread_id": "018f14f8-6bd7-7a21-8d25-7e7e3c8f6a11"}
        }))
        .unwrap_err();
        let msg = format!("{err}");
        assert!(msg.contains("driver.resume_thread_id"));
        assert!(
            msg.contains("top-level `session_id`"),
            "error must point at the canonical lifecycle field: {msg}"
        );
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
    fn parse_rejects_wrongly_typed_selection_mode() {
        for field in ["skills", "context_loaders"] {
            let mut payload = serde_json::Map::new();
            payload.insert("prompt".to_string(), json!("hi"));
            payload.insert(field.to_string(), json!({"mode": 123}));
            let err = ChatArgs::parse(&Value::Object(payload)).unwrap_err();
            let msg = format!("{err}");
            assert!(
                msg.contains(&format!("{field}.mode must be a string")),
                "wrong error for {field}: {msg}"
            );
        }
    }

    #[test]
    fn parse_rejects_unknown_selection_fields() {
        for field in ["skills", "context_loaders"] {
            let mut payload = serde_json::Map::new();
            payload.insert("prompt".to_string(), json!("hi"));
            payload.insert(
                field.to_string(),
                json!({"mode": "auto", "legacy_filter": true}),
            );
            let err = ChatArgs::parse(&Value::Object(payload)).unwrap_err();
            let msg = format!("{err}");
            assert!(
                msg.contains(&format!("chat: {field}")),
                "wrong context: {msg}"
            );
            assert!(msg.contains("unsupported field"), "wrong error: {msg}");
            assert!(msg.contains("legacy_filter"), "wrong field: {msg}");
        }
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
        // Use an in-memory entry (no root_path) so abilities_for returns
        // no manifest-backed tools.
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
        assert!(looks_like_thread_id("019dd304-60d9-74f2-8085-d4624e195d62"));
        assert!(looks_like_thread_id("38e5640c-6843-4f15-8f3a-2c8de75d0209"));
        assert!(looks_like_thread_id("00000000-0000-0000-0000-000000000000"));

        // Locally-minted `chat-<uuid_like>` ids — `<32-hex>-<16-hex>`
        // — are NOT a driver-issued shape and MUST be rejected so the
        // chat ability does not try to feed them to `--resume`.
        let local = format!("chat-{}", uuid_like());
        assert!(!looks_like_thread_id(&local));
        assert!(!looks_like_thread_id(&uuid_like()));

        // Edge cases: wrong dash positions, wrong length, non-hex.
        assert!(!looks_like_thread_id(""));
        assert!(!looks_like_thread_id("not-a-uuid"));
        assert!(!looks_like_thread_id("019dd304-60d9-74f2-8085-d4624e195d6")); // 35 chars
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
    fn chat_turn_session_id_prefers_resume_then_driver_then_local() {
        assert_eq!(
            ChatTurnSessionId::select(Some("resume-id"), Some("driver-id"), "local-id"),
            ChatTurnSessionId::ResumeRequested("resume-id".to_string())
        );
        assert_eq!(
            ChatTurnSessionId::select(None, Some("driver-id"), "local-id"),
            ChatTurnSessionId::DriverMinted("driver-id".to_string())
        );
        assert_eq!(
            ChatTurnSessionId::select(None, None, "local-id"),
            ChatTurnSessionId::LocalResolved("local-id".to_string())
        );
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
        use crate::daemon::boot::kernel::api::KernelApi;
        use crate::daemon::boot::kernel::Kernel;
        use axon_sdk::invocation::{make_ability, InvocationState};
        use std::sync::atomic::{AtomicUsize, Ordering};

        let _g = crate::cli::commands::test_support::HomeGuard::new();

        // Fake chat handler — increments a counter on every call so we
        // can prove the registered handler is the one that fired.
        let counter = Arc::new(AtomicUsize::new(0));
        let counter_for_handler = Arc::clone(&counter);
        let rt = crate::daemon::axon_bridge::runtime_factory::build_local_runtime(
            crate::daemon::axon_bridge::runtime_factory::rejecting_test_key_resolver(),
            None,
        );
        let chat_options = axon_sdk::invocation::AbilityOptions::default()
            .with_modes(axon_sdk::invocation::AbilityCallModes::RPC)
            .with_descriptor_proof(
                crate::daemon::ability::DEFAULT_ABILITY_DESCRIPTOR_VERSION,
                "invoke",
                [0x33; 32],
                [0x11; 32],
                [0x22; 32],
            );
        // Register under the canonical owner ability URA the kernel
        // resolves (`device.a.alice.chat`), not the bare handler name — a
        // raw LocalRuntime has no catalog to mirror the bare key.
        let chat_runtime_ability = crate::core::ura::owner_ability_ura(
            &crate::core::ura::device_ura("localhost", "a"),
            "alice.chat",
        )
        .expect("derive alice.chat runtime URA");
        crate::support::async_bridge::run_blocking(
            rt.register_ability_with_options(
                chat_runtime_ability,
                make_ability(move |_ctx| {
                    let counter_for_handler = Arc::clone(&counter_for_handler);
                    async move {
                        counter_for_handler.fetch_add(1, Ordering::SeqCst);
                        serde_json::to_vec(&json!({"reply": "fake"})).map_err(|err| {
                            axon_sdk::invocation::AxonError::internal(format!(
                                "encode fake chat reply: {err}"
                            ))
                        })
                    }
                }),
                chat_options,
            ),
            crate::support::async_bridge::SyncBridgeRuntimePolicy::BuildCurrentThreadTokio,
        )
        .expect("register runtime chat ability");
        let kernel = Kernel::new();
        kernel.set_local_runtime(Arc::clone(&rt));

        let device_ura = crate::core::ura::device_ura("localhost", "a");
        let request = kernel
            .prepare_local_system_rpc(
                &device_ura,
                "alice.chat",
                &device_ura,
                serde_json::to_vec(&json!({"prompt": "hi"})).unwrap(),
            )
            .expect("canonical descriptor-bound request");
        let finalized = kernel.invoke(request).expect("invoke ok");
        assert_eq!(finalized.terminal_state, InvocationState::Completed);
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
            crate::daemon::execution::mission::agent_ability_specs::AgentAbilitySpec::new(
                "alice.voice",
                "Speak text via the local TTS engine.\nMore detail.",
                json!({"type": "object"}),
            )
            .unwrap(),
            crate::daemon::execution::mission::agent_ability_specs::AgentAbilitySpec::new(
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
    fn enumerate_other_agent_specs_rejects_unreadable_registry_projection() {
        let _home = crate::cli::commands::test_support::HomeGuard::new();
        let state_dir = crate::daemon::persistence::config::state_dir();
        std::fs::create_dir_all(&state_dir).expect("state dir");
        std::fs::write(state_dir.join("agents.json"), "{not-json")
            .expect("corrupt agents registry");

        let err = enumerate_other_agent_specs("alice")
            .expect_err("corrupt registry must fail cross-agent discovery");
        assert!(
            format!("{err:#}").contains("load cross-agent ability registry projection"),
            "{err:#}"
        );
    }

    #[test]
    fn invoke_direct_rejects_unreadable_cross_agent_registry_projection() {
        let _home = crate::cli::commands::test_support::HomeGuard::new();
        let state_dir = crate::daemon::persistence::config::state_dir();
        std::fs::create_dir_all(&state_dir).expect("state dir");
        std::fs::write(state_dir.join("agents.json"), "{not-json")
            .expect("corrupt agents registry");

        let err =
            invoke_direct_with_progress("alice", &entry(), &[], json!({"prompt": "hi"}), None)
                .expect_err("chat must fail before dispatch on corrupt cross-agent registry");
        assert!(
            format!("{err:#}").contains("load cross-agent ability registry projection"),
            "{err:#}"
        );
    }

    #[test]
    fn stream_handler_rejects_unreadable_cross_agent_registry_projection() {
        let _home = crate::cli::commands::test_support::HomeGuard::new();
        let state_dir = crate::daemon::persistence::config::state_dir();
        std::fs::create_dir_all(&state_dir).expect("state dir");
        std::fs::write(state_dir.join("agents.json"), "{not-json")
            .expect("corrupt agents registry");

        let err = stream_handler("alice", &entry(), &[], json!({"prompt": "hi"}))
            .expect_err("stream chat must fail before dispatch on corrupt cross-agent registry");
        assert!(
            format!("{err:#}").contains("load cross-agent ability registry projection"),
            "{err:#}"
        );
    }

    #[test]
    fn compose_chat_context_returns_none_when_all_fragments_empty() {
        assert!(compose_chat_context(None, None, &[], None, None).is_none());
        assert!(compose_chat_context(Some("   "), None, &[], Some("   "), Some("   ")).is_none());
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
        assert!(
            loader_at < attach_at,
            "loader output must precede attachments"
        );
        assert!(
            attach_at < caller_at,
            "attachments must precede caller context"
        );
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
        match &args.attachments[0] {
            AttachmentSpec::Path { path, encoding } => {
                assert_eq!(path, "/etc/hosts");
                assert_eq!(*encoding, AttachmentEncoding::Utf8);
            }
            other => panic!("expected a Path attachment, got {other:?}"),
        }
    }

    #[test]
    fn parse_attachments_accepts_ura_with_filename() {
        let args = ChatArgs::parse(&json!({
            "prompt": "p",
            "attachments": [{
                "ura": "easynet:///r/easynet.run/resource/alice.files/aaaa",
                "filename": "report.pdf"
            }]
        }))
        .unwrap();
        assert_eq!(args.attachments.len(), 1);
        match &args.attachments[0] {
            AttachmentSpec::Ura { ura, filename } => {
                assert_eq!(ura, "easynet:///r/easynet.run/resource/alice.files/aaaa");
                assert_eq!(filename.as_deref(), Some("report.pdf"));
            }
            other => panic!("expected a Ura attachment, got {other:?}"),
        }
    }

    #[test]
    fn parse_attachments_rejects_path_and_ura_together() {
        let err = ChatArgs::parse(&json!({
            "prompt": "p",
            "attachments": [{"path": "/x", "ura": "easynet:///r/x/resource/u.files/a"}]
        }))
        .unwrap_err();
        assert!(format!("{err}").contains("not both"));
    }

    #[test]
    fn parse_attachments_rejects_encoding_on_ura() {
        let err = ChatArgs::parse(&json!({
            "prompt": "p",
            "attachments": [{"ura": "easynet:///r/x/resource/u.files/a", "encoding": "utf8"}]
        }))
        .unwrap_err();
        assert!(format!("{err}").contains("only valid with `path`"));
    }

    #[test]
    fn parse_attachments_rejects_filename_on_path() {
        let err = ChatArgs::parse(&json!({
            "prompt": "p",
            "attachments": [{"path": "/etc/hosts", "filename": "hosts.txt"}]
        }))
        .unwrap_err();
        assert!(format!("{err}").contains("filename is only valid with `ura`"));
    }

    #[test]
    fn parse_attachments_rejects_unknown_item_fields() {
        let err = ChatArgs::parse(&json!({
            "prompt": "p",
            "attachments": [{"path": "/etc/hosts", "content_type": "text/plain"}]
        }))
        .unwrap_err();
        let msg = format!("{err}");
        assert!(msg.contains("unsupported field"));
        assert!(msg.contains("content_type"));
    }

    #[test]
    fn parse_attachments_rejects_wrongly_typed_string_fields() {
        for (field, payload) in [
            ("path", json!({"path": 123})),
            ("ura", json!({"ura": 123})),
            (
                "filename",
                json!({"ura": "easynet:///r/x/resource/u.files/a", "filename": 123}),
            ),
            ("encoding", json!({"path": "/etc/hosts", "encoding": 123})),
        ] {
            let err = ChatArgs::parse(&json!({
                "prompt": "p",
                "attachments": [payload]
            }))
            .unwrap_err();
            let msg = format!("{err}");
            assert!(
                msg.contains(&format!(".{field} must be a string")),
                "wrong error for {field}: {msg}"
            );
        }
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
    fn parse_attachments_rejects_missing_source() {
        let err = ChatArgs::parse(&json!({
            "prompt": "hi",
            "attachments": [{"encoding": "utf8"}]
        }))
        .unwrap_err();
        assert!(format!("{err}").contains("`path` (string) or `ura`"));
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
        assert!(materialize_attachments(&[], None, None).unwrap().is_none());
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
        let specs = vec![AttachmentSpec::Path {
            path: path.to_string_lossy().to_string(),
            encoding: AttachmentEncoding::Utf8,
        }];
        let block = materialize_attachments(&specs, None, None)
            .unwrap()
            .unwrap();
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
        let specs = vec![AttachmentSpec::Path {
            path: path.to_string_lossy().to_string(),
            encoding: AttachmentEncoding::Base64,
        }];
        let block = materialize_attachments(&specs, None, None)
            .unwrap()
            .unwrap();
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
        let specs = vec![AttachmentSpec::Path {
            path: path.to_string_lossy().to_string(),
            encoding: AttachmentEncoding::Utf8,
        }];
        let err = materialize_attachments(&specs, None, None).unwrap_err();
        assert!(format!("{err}").contains("UTF-8"));
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn materialize_attachments_bails_on_missing_file() {
        let specs = vec![AttachmentSpec::Path {
            path: "/nonexistent/really/not/here.txt".to_string(),
            encoding: AttachmentEncoding::Utf8,
        }];
        let err = materialize_attachments(&specs, None, None).unwrap_err();
        assert!(
            format!("{err}").contains("stat") || format!("{err}").contains("open"),
            "expected an I/O error, got: {err}"
        );
    }

    /// Scratch (files_root, workspace_root, sha, ura) fixture for the
    /// URA-attachment tests: one blob in a temp store plus an empty
    /// temp workspace. Roots are injected into materialize_attachments
    /// directly — no EASYNET_FILES_ROOT env writes, parallel-safe.
    fn ura_attachment_fixture(
        tag: &str,
    ) -> (std::path::PathBuf, std::path::PathBuf, String, String) {
        let base = std::env::temp_dir().join(format!(
            "chat-ura-attach-{tag}-{}-{}",
            std::process::id(),
            uuid_like()
        ));
        let files_root = base.join("store");
        let workspace = base.join("workspace");
        std::fs::create_dir_all(&files_root).unwrap();
        std::fs::create_dir_all(&workspace).unwrap();
        let sha = "ab".repeat(32); // 64 hex chars
        std::fs::write(files_root.join(&sha), b"blob bytes").unwrap();
        let ura = crate::daemon::ability::builtins::resources::files_store::state::blob_ura(
            "easynet.run",
            "alice",
            &sha,
        );
        (files_root, workspace, sha, ura)
    }

    #[test]
    fn materialize_ura_attachment_copies_blob_and_notes_relative_path() {
        let (files_root, workspace, sha, ura) = ura_attachment_fixture("ok");
        let specs = vec![AttachmentSpec::Ura {
            ura: ura.clone(),
            filename: Some("report.pdf".to_string()),
        }];
        let block = materialize_attachments(&specs, Some(&workspace), Some(&files_root))
            .unwrap()
            .unwrap();
        let rel = format!("uploads/{}-report.pdf", &sha[..8]);
        assert!(
            block.contains(&format!("`{rel}`")),
            "block must cite the workspace-relative path, got: {block}"
        );
        assert!(block.contains(&ura), "block must cite the source URA");
        let copied = std::fs::read(workspace.join(&rel)).unwrap();
        assert_eq!(copied, b"blob bytes");
        let _ = std::fs::remove_dir_all(files_root.parent().unwrap());
    }

    #[test]
    fn materialize_ura_attachment_sanitizes_traversal_filenames() {
        let (files_root, workspace, sha, ura) = ura_attachment_fixture("dotdot");
        let specs = vec![AttachmentSpec::Ura {
            ura,
            filename: Some("../../etc/passwd".to_string()),
        }];
        let block = materialize_attachments(&specs, Some(&workspace), Some(&files_root))
            .unwrap()
            .unwrap();
        // Only the basename survives; the copy lands inside uploads/.
        let rel = format!("uploads/{}-passwd", &sha[..8]);
        assert!(block.contains(&format!("`{rel}`")), "got: {block}");
        assert!(workspace.join(&rel).is_file());
        assert!(!workspace.parent().unwrap().join("etc/passwd").exists());
        let _ = std::fs::remove_dir_all(files_root.parent().unwrap());
    }

    #[test]
    fn materialize_ura_attachment_bails_on_unknown_blob() {
        let (files_root, workspace, sha, _) = ura_attachment_fixture("missing");
        std::fs::remove_file(files_root.join(&sha)).unwrap();
        let ura = crate::daemon::ability::builtins::resources::files_store::state::blob_ura(
            "easynet.run",
            "alice",
            &sha,
        );
        let specs = vec![AttachmentSpec::Ura {
            ura,
            filename: None,
        }];
        let err = materialize_attachments(&specs, Some(&workspace), Some(&files_root)).unwrap_err();
        assert!(
            format!("{err}").contains("missing from the files store"),
            "got: {err}"
        );
        let _ = std::fs::remove_dir_all(files_root.parent().unwrap());
    }

    #[test]
    fn materialize_ura_attachment_requires_workspace_and_store_roots() {
        let (files_root, workspace, _, ura) = ura_attachment_fixture("roots");
        let specs = vec![AttachmentSpec::Ura {
            ura,
            filename: None,
        }];
        let err = materialize_attachments(&specs, None, Some(&files_root)).unwrap_err();
        assert!(format!("{err}").contains("no workspace"), "got: {err}");
        let err = materialize_attachments(&specs, Some(&workspace), None).unwrap_err();
        assert!(
            format!("{err}").contains("files store root is unavailable"),
            "got: {err}"
        );
        let _ = std::fs::remove_dir_all(files_root.parent().unwrap());
    }

    #[test]
    fn sanitized_upload_name_falls_back_and_strips_directories() {
        let sha = "ab".repeat(32);
        assert_eq!(
            sanitized_upload_name(Some("report.pdf"), &sha),
            "abababab-report.pdf"
        );
        assert_eq!(
            sanitized_upload_name(Some("a/b/c.txt"), &sha),
            "abababab-c.txt"
        );
        assert_eq!(sanitized_upload_name(Some(".."), &sha), "abababab-file");
        assert_eq!(sanitized_upload_name(Some("  "), &sha), "abababab-file");
        assert_eq!(sanitized_upload_name(None, &sha), "abababab-file");
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
    /// rather than an LLM-mediated route.
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
    ///   and routes to daemon::execution::mission::executors::shell::run_shell_exec
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
        use crate::daemon::ability::manifest::{AbilityExec, AbilityManifest, ShellExec};
        use crate::daemon::persistence::agent_registry::AgentEntry;

        let _g = crate::cli::commands::test_support::HomeGuard::new();

        // Materialise an agent root with a single ability manifest
        // that pins a shell executor. We use `printf` (POSIX,
        // deterministic, available on the macOS dev box and any
        // Linux CI runner) so the test is hermetic — no network,
        // no LLM, no system PATH guesses beyond a coreutils.
        let ws_root = crate::daemon::persistence::config::agents_root().join("alice");
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

        // Also seed a canonical agent.toml so AgentDirectory::open accepts the
        // root. The test is targeting dispatch, so it goes through the same
        // AgentSpec writer as production rather than hand-writing schema
        // details.
        let mut spec = crate::core::agent::spec::AgentSpec::new(
            "alice",
            crate::core::agent::spec::RuntimeKind::ClaudeCode,
        );
        spec.model = Some("sonnet".to_string());
        std::fs::write(
            ws_root.join("agent.toml"),
            spec.to_toml_string()
                .expect("test AgentSpec serialises with canonical schema stamp"),
        )
        .expect("agent.toml write");

        // Build the handler the same way the registration paths do:
        // boot-time pre-registration and HotAgentRegistrar both call
        // build_agent_ability_handler.
        let mut entry = AgentEntry::new(
            crate::daemon::persistence::agent_registry::AgentType::ClaudeCode,
            None,
        );
        // `root_path` is the field that `manifests_for` (and
        // `abilities_for`) read to find the on-disk abilities/
        // directory. Without it the helpers fall back to the
        // synthetic chat-only path and the test would silently pass
        // through chat dispatch.
        entry.root_path = Some(ws_root.clone());
        let loaders: Arc<Vec<Arc<dyn ContextLoader>>> = Arc::new(Vec::new());
        let handler =
            build_agent_ability_handler("alice".to_string(), entry, loaders, "echo".to_string());

        let envelope = handler(
            crate::daemon::ability::dispatch::EnvelopeContext::for_test(
                "easynet:///r/test/agent/caller",
                "easynet:///r/test/resource/shell",
            ),
            json!({ "value": "hello" }),
        )
        .expect("shell exec must succeed for printf %s hello");

        assert_eq!(
            envelope.get("fulfilled_by").and_then(|v| v.as_str()),
            Some("shell"),
            "manifest with [exec] kind=\"shell\" MUST dispatch through the shell \
             executor. Envelope was: {envelope}"
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
