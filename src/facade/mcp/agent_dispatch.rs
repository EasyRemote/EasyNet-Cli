// EasyNet CLI — MCP Agent Dispatch Adapter
// =========================================
//
// File: src/facade/mcp/agent_dispatch.rs
// Description: Bridges incoming MCP tool calls named `<agent>.chat`
//              (etc.) to local subprocess dispatch. Wraps the Axon
//              SDK's `AbilityToolAdapter`, keeping the MCP-provider
//              side free of any knowledge about what an "agent" is
//              or how it runs.
//
// Role in the stack
// -----------------
//
//   incoming MCP call
//   → HubMcpProvider::handle_tool_call
//   → AgentDispatchAdapter::handle   ← (this module)
//   → AbilityToolAdapter::execute    ← (Axon SDK)
//   → handler closure                ← (registered here)
//   → runtime::dispatch::send_external
//   → subprocess (claude / codex / …)
//   → AgentResponse → Value → ToolResult
//
// Design goals
// ------------
//
// 1. **Narrow surface.** The provider only needs three operations
//    from this module: "list the tool specs you own", "is `name`
//    one of yours?", and "handle `name` + `args`, return a
//    Result<Value, McpError>". Everything else — the handler
//    closures, the SDK adapter, the agent registry — stays
//    encapsulated. The provider file shouldn't grow a second
//    reason to import `runtime::dispatch`.
//
// 2. **Single source of truth for ability enumeration.** Both the
//    discovery-side (`a2a_labels::build`) and the dispatch-side
//    (this module) call `runtime::abilities::abilities_for`. If
//    a future refactor adds a `voice` ability, it appears in
//    *both* paths without a second-pass edit, because both read
//    the same enumerator.
//
// 3. **Handler idempotency.** The SDK's `AbilityToolAdapter` only
//    invokes the handler once per `execute` call (reconnect logic
//    lives inside `ReconnectingBridge`, and dispatch to a local
//    subprocess doesn't touch that). So the `FnMut` retry concern
//    that haunts `HubMcpProvider::with_bridge` does *not* apply
//    here — each handler runs exactly once per incoming call.
//
// 4. **Option<Result<..>> return shape.** `handle` returns
//    `Option<Result<Value, McpError>>`. `None` means "not my tool,
//    fall through to the network dispatch path"; `Some(Ok)` is a
//    successful dispatch; `Some(Err)` is a handled but failed
//    dispatch. This lets the provider chain the two dispatch
//    paths with a single `if let Some(outcome) = agent.handle(name, args)`
//    without conflating "tool unknown" with "tool failed".
//
// Name-collision safety
// ---------------------
//
// Agent abilities use the form `<agent>.chat` (see
// `runtime::abilities`). Network tool names never contain a dot
// (`invoke_ability`, `list_devices`, …). A collision is therefore
// structurally impossible, and the provider can safely check
// "does `AgentDispatchAdapter` own this name?" without a
// precedence-rule comment — the name shape itself is the rule.
// Pinned by a regression test in this file.
//
// Author: Silan Hu <silan.hu@u.nus.edu>
// Copyright (c) 2026 EasyNet. All rights reserved.

use std::collections::HashSet;
use std::sync::Arc;

use easynet_axon::tool_adapter::{AbilityToolAdapter, ToolSpec};
use easynet_axon::AxonError;
use serde_json::{Map, Value};

use super::error::McpError;
use crate::registry::agents::AgentRegistry;
use crate::runtime::abilities::abilities_for;
use crate::runtime::ability_dispatch::LocalAbilityRegistry;

/// Narrow MCP-side wrapper over `AbilityToolAdapter` that exposes
/// just the three operations `HubMcpProvider` needs. Owns the
/// underlying adapter plus a snapshot of the tool names it registered
/// (so the "is this mine?" check is O(1) without walking the SDK's
/// spec list twice).
///
/// `!Send + !Sync` by inheritance from `AbilityToolAdapter` —
/// matches the MCP stdio loop's single-threaded shape.
pub struct AgentDispatchAdapter {
    /// SDK adapter retained for `tool_specs()`. After the chat-as-
    /// ability collapse, the SDK adapter no longer drives dispatch —
    /// it carries only the advertised `ToolSpec`s so federated MCP
    /// peers see the full `{name, description, resource_uri,
    /// parameters}` quadruple. Dispatch goes through `registry`
    /// (see `handle`).
    ///
    /// We register a no-op handler under each spec name to satisfy
    /// the SDK adapter's "register before execute" invariant; the
    /// real handler body never runs because `handle` short-circuits
    /// to the unified registry instead of calling `inner.execute`.
    inner: AbilityToolAdapter,
    /// Pre-hashed set of tool names this adapter owns. O(1) "is
    /// this mine?" check at dispatch time.
    names: HashSet<String>,
    /// Unified ability registry. `handle` looks the chat handler up
    /// here — same handler the Kernel routes through, same handler
    /// the IPC proxy invokes for direct subscribers. Single source of
    /// truth across all entry points (Kernel, MCP, IPC).
    registry: Arc<LocalAbilityRegistry>,
}

impl AgentDispatchAdapter {
    /// Build an adapter from a loaded `AgentRegistry` and the unified
    /// `LocalAbilityRegistry` the daemon constructed at boot. Each
    /// agent's abilities (as enumerated by
    /// `runtime::abilities::abilities_for`) are registered as
    /// `ToolSpec`s on the SDK adapter for advertising; dispatch goes
    /// through the unified registry's chat handler.
    ///
    /// Why we still register stub handlers on the SDK adapter:
    /// `AbilityToolAdapter::register` is the only way to add a
    /// `ToolSpec` to its internal table, and the table is what
    /// `as_dicts()` reads to produce the advertised specs. We register
    /// a stub closure that always errors `not_implemented`; the stub
    /// is unreachable in practice because `handle` routes to the
    /// unified registry before ever calling `inner.execute`.
    ///
    /// `tenant_id` is propagated into the underlying SDK adapter for
    /// any future remote-fallback dispatch (none today). Kept as a
    /// parameter for symmetry with the SDK's constructor.
    pub fn build(
        registry: &AgentRegistry,
        local_registry: Arc<LocalAbilityRegistry>,
        tenant_id: impl Into<String>,
    ) -> Self {
        let mut inner = AbilityToolAdapter::new(tenant_id);
        let mut names = HashSet::new();
        for (agent_name, entry) in &registry.agents {
            for ability in abilities_for(agent_name, entry) {
                let name = ability.name().to_string();
                names.insert(name.clone());
                register_spec_only(&mut inner, &name, ability.description(), ability.parameters());
            }
            // Keep the unused-import warning quiet for AgentEntry
            // since a future per-agent-typed branch may want it.
            let _ = entry;
        }
        Self {
            inner,
            names,
            registry: local_registry,
        }
    }

    /// Construct an empty adapter (no agents registered). Used by
    /// callers that want to wire the adapter seam unconditionally —
    /// a zero-registry adapter is a no-op at dispatch time.
    pub fn empty(tenant_id: impl Into<String>) -> Self {
        Self {
            inner: AbilityToolAdapter::new(tenant_id),
            names: HashSet::new(),
            registry: Arc::new(LocalAbilityRegistry::new()),
        }
    }

    /// Whether any abilities are registered. The provider uses this
    /// to decide whether to merge ability tool specs into the tool
    /// list at all — a zero-agents node should not advertise an
    /// empty "abilities" section.
    pub fn is_empty(&self) -> bool {
        self.names.is_empty()
    }

    /// Snapshot of ability tool specs in the OpenAI-style envelope
    /// used by the MCP provider's `tool_specs()`. We delegate to the
    /// SDK's `as_dicts(None)` because that is the "generic JSON"
    /// shape with the full `{name, description, resource_uri,
    /// parameters}` quadruple — closest to what the other MCP tool
    /// specs in `facade::mcp::specs` emit.
    pub fn tool_specs(&self) -> Vec<Value> {
        self.inner.as_dicts(None)
    }

    /// Attempt to dispatch `name` with `args`. Returns:
    ///
    ///   - `None` if `name` is not an agent ability on this adapter
    ///     (the provider should fall through to the network path).
    ///   - `Some(Ok(value))` on a successful local dispatch — `value`
    ///     is the typed chat response (`reply`, `session_id`,
    ///     `skills_loaded`, `tool_calls`, `context_used`, `usage`,
    ///     `elapsed_ms`).
    ///   - `Some(Err(err))` on a dispatch that reached the handler
    ///     but failed there (invalid args, subprocess error, …).
    ///
    /// Routing: the call resolves to the registered chat handler in
    /// `LocalAbilityRegistry` — the same handler `Kernel::invoke` and
    /// the IPC proxy reach. After this collapse, every entry point
    /// (Kernel, MCP, IPC) shares one chat code path.
    pub fn handle(&self, name: &str, args: &Map<String, Value>) -> Option<Result<Value, McpError>> {
        if !self.names.contains(name) {
            return None;
        }
        let payload = Value::Object(args.clone());
        // Look up the registered handler. If it's missing, that is a
        // boot-order bug (we advertised a spec for a name with no
        // handler) — surface it as a typed error rather than panic so
        // a Client gets a structured failure.
        match self.registry.get_rpc(name) {
            Some(handler) => Some(handler(payload).map_err(anyhow_to_mcp)),
            None => Some(Err(McpError::Internal(format!(
                "agent ability `{name}` is advertised but no handler is registered \
                 in LocalAbilityRegistry — boot ordering bug"
            )))),
        }
    }
}

/// Register a `ToolSpec` on the SDK adapter without binding a real
/// handler. The handler is required by `AbilityToolAdapter::register`
/// (no spec-only API), so we install a stub that returns
/// `Unimplemented` if it is ever reached. In practice it cannot be
/// reached because `AgentDispatchAdapter::handle` routes through the
/// unified registry before the SDK adapter's `execute` ever runs.
fn register_spec_only(
    adapter: &mut AbilityToolAdapter,
    name: &str,
    description: &str,
    parameters: &Value,
) {
    let spec = ToolSpec {
        name: name.to_string(),
        description: description.to_string(),
        resource_uri: format!("easynet:///r/org/{name}"),
        parameters: parameters.clone(),
    };
    let stub_name = name.to_string();
    adapter.register(
        name.to_string(),
        move |_args: Value| -> Result<Value, AxonError> {
            // Reaching this branch is a routing bug: AgentDispatchAdapter
            // should always handle dispatch through the unified
            // LocalAbilityRegistry. If you are seeing this error in a
            // log, the SDK adapter's `execute` was called directly
            // (skipping `AgentDispatchAdapter::handle`) — fix the
            // upstream call site.
            Err(AxonError::Invocation(format!(
                "agent ability `{stub_name}` dispatched through SDK stub handler; \
                 should have routed via unified LocalAbilityRegistry"
            )))
        },
        spec,
    );
}

/// `anyhow::Error` → `McpError` conversion for handler results.
/// The unified chat handler returns `anyhow::Result` (the
/// `LocalRpcHandler` contract); we project that to the typed MCP
/// taxonomy by inspecting the message for known prefixes the chat
/// handler emits. Everything else falls through to `Internal` so a
/// surprise error still surfaces as a structured envelope rather
/// than dropping detail.
fn anyhow_to_mcp(err: anyhow::Error) -> McpError {
    let msg = format!("{err}");
    let lower = msg.to_lowercase();
    // Argument-shape errors from `ChatArgs::parse` and the manifest
    // schema validators all start with `chat:`; treat them as
    // validation. The substring check is intentional rather than a
    // typed match because crossing a crate boundary (anyhow erases
    // the source type) is what `anyhow::Error` is for.
    if lower.contains("chat:") || lower.contains("required") || lower.contains("must be") {
        return McpError::Validation(msg);
    }
    if lower.contains("permission denied") {
        return McpError::Validation(msg);
    }
    McpError::Internal(msg)
}

// ── Tests ──────────────────────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    //! Tests cover three layers:
    //!
    //! 1. Adapter construction (`AgentDispatchAdapter::{build, empty}`)
    //!    — zero-agent and multi-agent registries produce the right
    //!    `tool_specs` and `handle` behaviour.
    //! 2. Dispatch routing (`handle`) — unknown tool returns `None`
    //!    (provider must fall through), known tool reaches the unified
    //!    chat handler in `LocalAbilityRegistry`, handler errors
    //!    surface as `Some(Err(McpError))` with the right taxonomy.
    //! 3. Name-collision regression — network tool names and ability
    //!    names cannot alias.

    use super::*;
    use crate::registry::agents::{AgentEntry, AgentRegistry, AgentType};
    use crate::runtime::ability_dispatch::LocalAbilityRegistry;
    use crate::runtime::system::chat_ability;
    use serde_json::json;
    use std::sync::atomic::{AtomicUsize, Ordering};

    fn registry_with(agents: Vec<(&str, AgentType)>) -> AgentRegistry {
        let mut r = AgentRegistry::default();
        for (name, t) in agents {
            r.agents.insert(name.to_string(), AgentEntry::new(t, None));
        }
        r
    }

    /// Build a `LocalAbilityRegistry` with chat handlers registered
    /// for the agents in `agent_registry`. Mirrors what
    /// `system::build_registry_with_services` does at boot, but
    /// without the other system abilities — keeps the test focused
    /// on the chat path.
    fn local_registry_with_chat(agent_registry: &AgentRegistry) -> Arc<LocalAbilityRegistry> {
        let mut reg = LocalAbilityRegistry::new();
        chat_ability::register(&mut reg, agent_registry, Arc::new(Vec::new()));
        Arc::new(reg)
    }

    // ── AgentDispatchAdapter construction ──────────────────────────────────

    #[test]
    fn empty_adapter_advertises_zero_tools_and_handles_nothing() {
        let a = AgentDispatchAdapter::empty("tenant-x");
        assert!(a.is_empty());
        assert!(a.tool_specs().is_empty());
        // Any call name must return None (fall-through), not Some(Err).
        let result = a.handle("claude.chat", &Map::new());
        assert!(result.is_none());
    }

    #[test]
    fn build_registers_one_tool_per_agent() {
        let registry = registry_with(vec![
            ("claude", AgentType::ClaudeCode),
            ("codex", AgentType::Codex),
        ]);
        let local = local_registry_with_chat(&registry);
        let a = AgentDispatchAdapter::build(&registry, local, "tenant-x");
        assert!(!a.is_empty());
        let specs = a.tool_specs();
        assert_eq!(specs.len(), 2, "two agents → two abilities");
        let names: Vec<&str> = specs
            .iter()
            .map(|s| s["name"].as_str().expect("spec name must be string"))
            .collect();
        assert!(names.contains(&"claude.chat"));
        assert!(names.contains(&"codex.chat"));
    }

    #[test]
    fn tool_spec_shape_matches_openai_dict_contract() {
        // Federated MCP clients parse `{name, description, resource_uri,
        // parameters}`. Pin the keys so an SDK-side refactor of
        // `as_dicts` can't silently drop one.
        let registry = registry_with(vec![("claude", AgentType::ClaudeCode)]);
        let local = local_registry_with_chat(&registry);
        let a = AgentDispatchAdapter::build(&registry, local, "tenant-x");
        let spec = &a.tool_specs()[0];
        for key in ["name", "description", "resource_uri", "parameters"] {
            assert!(
                spec.get(key).is_some(),
                "tool spec missing key `{key}`, got {spec}"
            );
        }
    }

    // ── handle() routing ───────────────────────────────────────────────────

    #[test]
    fn handle_returns_none_for_unknown_tool() {
        let registry = registry_with(vec![("claude", AgentType::ClaudeCode)]);
        let local = local_registry_with_chat(&registry);
        let a = AgentDispatchAdapter::build(&registry, local, "tenant-x");
        // Classic network tool names — must fall through.
        for unknown in [
            "invoke_ability",
            "list_devices",
            "send_a2a_task",
            "codex.chat", // not registered
            "",
        ] {
            assert!(
                a.handle(unknown, &Map::new()).is_none(),
                "expected None (fall-through) for {unknown:?}"
            );
        }
    }

    #[test]
    fn handle_routes_known_tool_to_validation_path_on_bad_args() {
        // Missing `prompt` arrives at the registered chat handler,
        // fails the ChatArgs validation, and surfaces as a Validation
        // McpError. Proves the routing reached the handler — not None
        // (fall-through), not the deleted local dispatch_chat.
        let registry = registry_with(vec![("claude", AgentType::ClaudeCode)]);
        let local = local_registry_with_chat(&registry);
        let a = AgentDispatchAdapter::build(&registry, local, "tenant-x");
        let outcome = a.handle("claude.chat", &Map::new()).expect("known tool must return Some");
        let err = outcome.expect_err("missing prompt must fail validation");
        assert_eq!(err.error_code(), "validation_error");
        assert!(err.message().contains("prompt"));
    }

    /// Phase 4 fix: prove that MCP `handle()` calls the same
    /// `LocalAbilityRegistry` handler that `Kernel::invoke` would
    /// hit. Replace the registered chat handler with a counter-bumping
    /// fake; route a call through `AgentDispatchAdapter::handle`;
    /// assert the fake fired exactly once.
    #[test]
    fn handle_routes_through_unified_local_registry() {
        let counter = Arc::new(AtomicUsize::new(0));
        let counter_for_handler = Arc::clone(&counter);
        let mut reg = LocalAbilityRegistry::new();
        reg.register_rpc(
            "alice.chat",
            Arc::new(move |_args: Value| {
                counter_for_handler.fetch_add(1, Ordering::SeqCst);
                Ok(json!({"reply": "fake-from-mcp"}))
            }),
        );
        // Build agent registry with `alice` so the adapter advertises
        // the spec; build adapter with our hand-rolled registry so
        // the handler is the fake.
        let agents = registry_with(vec![("alice", AgentType::ClaudeCode)]);
        let adapter = AgentDispatchAdapter::build(&agents, Arc::new(reg), "tenant-x");

        let mut args = Map::new();
        args.insert("prompt".into(), json!("hi from mcp"));
        let outcome = adapter
            .handle("alice.chat", &args)
            .expect("alice.chat is registered");
        let value = outcome.expect("fake handler must succeed");
        assert_eq!(value.get("reply").and_then(Value::as_str), Some("fake-from-mcp"));
        assert_eq!(
            counter.load(Ordering::SeqCst),
            1,
            "unified handler must fire exactly once per MCP dispatch"
        );
    }

    /// Boot-ordering bug detector: if the agent registry advertises a
    /// chat ability but the LocalAbilityRegistry has no handler for
    /// it, `handle` must surface a typed Internal error rather than
    /// panic or fall through silently.
    #[test]
    fn handle_surfaces_internal_error_when_handler_missing() {
        let agents = registry_with(vec![("alice", AgentType::ClaudeCode)]);
        // Pass an empty LocalAbilityRegistry — handler missing on
        // purpose to simulate the boot-ordering bug.
        let empty_local = Arc::new(LocalAbilityRegistry::new());
        let adapter = AgentDispatchAdapter::build(&agents, empty_local, "tenant-x");

        let mut args = Map::new();
        args.insert("prompt".into(), json!("hi"));
        let outcome = adapter.handle("alice.chat", &args).expect("name is in `names`");
        let err = outcome.expect_err("missing handler must surface typed error");
        assert_eq!(err.error_code(), "internal_error");
        assert!(err.message().contains("boot ordering"));
    }

    // ── Name-collision regression ──────────────────────────────────────────

    #[test]
    fn ability_names_cannot_collide_with_network_tool_names() {
        // Network tools don't contain `.`; ability names always do
        // (enforced by AgentAbilitySpec::new and `<agent>.chat`). A
        // collision is structurally impossible. Pin that structural
        // property here so a future "let me add a namespaced network
        // tool like `system.status`" change forces an eyes-open
        // precedence rule in `HubMcpProvider::handle_tool_call`.
        let registry = registry_with(vec![
            ("claude", AgentType::ClaudeCode),
            ("codex", AgentType::Codex),
        ]);
        let local = local_registry_with_chat(&registry);
        let a = AgentDispatchAdapter::build(&registry, local, "tenant-x");
        for spec in a.tool_specs() {
            let name = spec["name"].as_str().unwrap();
            assert!(
                name.contains('.'),
                "ability name {name:?} must carry the `.` shape to \
                 avoid collision with network tool names"
            );
        }
    }

    /// Regression: an `anyhow::Error` whose message names the chat
    /// validation prefix maps to `McpError::Validation`. Pinned
    /// because the round-trip is the visible contract an agent-side
    /// caller branches on.
    #[test]
    fn anyhow_chat_prefix_maps_to_validation() {
        let err = anyhow_to_mcp(anyhow::anyhow!("chat: `prompt` (string) required"));
        assert_eq!(err.error_code(), "validation_error");
    }

    #[test]
    fn anyhow_unrelated_error_falls_through_to_internal() {
        let err = anyhow_to_mcp(anyhow::anyhow!("subprocess crashed with signal SIGSEGV"));
        assert_eq!(err.error_code(), "internal_error");
    }
}
