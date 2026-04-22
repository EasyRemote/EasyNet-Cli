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

use easynet_axon::tool_adapter::{AbilityToolAdapter, ToolSpec};
use easynet_axon::AxonError;
use serde_json::{json, Map, Value};

use super::error::McpError;
use crate::registry::agents::{AgentEntry, AgentRegistry};
use crate::runtime::abilities::{abilities_for, AgentAbilitySpec};

/// Narrow MCP-side wrapper over `AbilityToolAdapter` that exposes
/// just the three operations `HubMcpProvider` needs. Owns the
/// underlying adapter plus a snapshot of the tool names it registered
/// (so the "is this mine?" check is O(1) without walking the SDK's
/// spec list twice).
///
/// `!Send + !Sync` by inheritance from `AbilityToolAdapter` —
/// matches the MCP stdio loop's single-threaded shape.
pub struct AgentDispatchAdapter {
    inner: AbilityToolAdapter,
    /// Pre-hashed set of tool names registered on `inner`. The SDK's
    /// adapter has `specs(&self) -> &[ToolSpec]`, which would force
    /// a linear scan on every call; a `HashSet<String>` lets the
    /// "is this my tool" check stay O(1) even when a node hosts many
    /// agents.
    names: HashSet<String>,
}

impl AgentDispatchAdapter {
    /// Build an adapter from a loaded `AgentRegistry`. Each agent's
    /// abilities (as enumerated by `runtime::abilities::abilities_for`)
    /// are registered as local-handler tools. A later
    /// `HubMcpProvider::with_agent_abilities(adapter)` wires this
    /// into the MCP dispatch path.
    ///
    /// `tenant_id` is propagated into the underlying SDK adapter so
    /// any remote-fallback dispatch (not used today — every ability
    /// we register is local) would scope correctly. Kept as a
    /// parameter rather than a default because a future caller may
    /// own multi-tenant provisioning.
    pub fn build(registry: &AgentRegistry, tenant_id: impl Into<String>) -> Self {
        let mut inner = AbilityToolAdapter::new(tenant_id);
        let mut names = HashSet::new();
        for (agent_name, entry) in &registry.agents {
            for ability in abilities_for(agent_name, entry) {
                let name = ability.name().to_string();
                names.insert(name.clone());
                register_ability_on(&mut inner, agent_name.clone(), entry.clone(), ability);
            }
        }
        Self { inner, names }
    }

    /// Construct an empty adapter (no agents registered). Used by
    /// callers that want to wire the adapter seam unconditionally —
    /// a zero-registry adapter is a no-op at dispatch time.
    ///
    /// Kept as a distinct constructor rather than forcing the common
    /// path through `build(&AgentRegistry::default(), ...)` so the
    /// intent ("I know there are no agents and that is fine") is
    /// visible at the call site.
    pub fn empty(tenant_id: impl Into<String>) -> Self {
        Self {
            inner: AbilityToolAdapter::new(tenant_id),
            names: HashSet::new(),
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
    ///     is the agent's response serialized by the handler.
    ///   - `Some(Err(err))` on a dispatch that reached the handler
    ///     but failed there (invalid args, subprocess error, …).
    ///
    /// `args` is the full tool-call argument map as received over
    /// the MCP wire; validation (missing `prompt`, non-string
    /// `context`, …) happens inside the handler and surfaces via
    /// `McpError`.
    pub fn handle(&self, name: &str, args: &Map<String, Value>) -> Option<Result<Value, McpError>> {
        if !self.names.contains(name) {
            return None;
        }
        // Bundle the argument map into a JSON value for the SDK
        // adapter. The SDK takes `serde_json::Value`; cloning the
        // map keeps the caller's map untouched so it can be used
        // for audit logging alongside the dispatch.
        let payload = Value::Object(args.clone());
        Some(self.inner.execute(name, payload).map_err(axon_to_mcp))
    }
}

/// Register one agent-ability closure on the SDK's adapter. Factored
/// out so `build` doesn't nest a five-level closure — this level of
/// separation is necessary because the closure captures owned
/// `agent_name` and `entry` clones (the handler outlives the build
/// scope).
fn register_ability_on(
    adapter: &mut AbilityToolAdapter,
    agent_name: String,
    entry: AgentEntry,
    ability: AgentAbilitySpec,
) {
    let spec = ToolSpec {
        name: ability.name().to_string(),
        description: ability.description().to_string(),
        resource_uri: format!("easynet:///r/org/{}", ability.name()),
        parameters: ability.parameters().clone(),
    };
    let handler_name = ability.name().to_string();
    adapter.register(
        handler_name,
        move |args: Value| -> Result<Value, AxonError> {
            dispatch_chat(&agent_name, &entry, &args)
        },
        spec,
    );
}

/// The handler body shared by every registered ability. Extracted
/// so it can be unit-tested with a fake dispatcher (see `tests`
/// below — the real function calls into `runtime::dispatch`, which
/// needs a real subprocess; we indirect through this shape so the
/// argument-validation half is testable in isolation).
fn dispatch_chat(
    agent_name: &str,
    entry: &AgentEntry,
    args: &Value,
) -> Result<Value, AxonError> {
    let (prompt, context) = parse_chat_args(args)?;
    let resp = crate::runtime::dispatch::send_external(agent_name, entry, prompt, context)
        .map_err(|e| AxonError::Invocation(format!("agent `{agent_name}` dispatch failed: {e}")))?;
    Ok(json!({
        "ok": true,
        "agent": resp.agent,
        "content": resp.content,
        "model": resp.model,
        "duration_ms": resp.duration_ms,
        "truncated": resp.truncated,
    }))
}

/// Extract `(prompt, Option<context>)` from the tool-call argument
/// value, with the validation the JSON-Schema on the tool spec
/// advertises but the MCP runtime does not enforce. Making the
/// checks explicit here means a handler error carries the typed
/// `AxonError::Validation` variant the agent-side caller can
/// branch on, instead of a cryptic string from deep inside
/// `send_external`.
fn parse_chat_args(args: &Value) -> Result<(&str, Option<&str>), AxonError> {
    let obj = args.as_object().ok_or_else(|| {
        AxonError::Validation("ability arguments must be a JSON object".into())
    })?;
    let prompt = match obj.get("prompt") {
        Some(Value::String(s)) => s.as_str(),
        Some(Value::Null) | None => {
            return Err(AxonError::Validation(
                "missing required string field `prompt`".into(),
            ))
        }
        Some(other) => {
            return Err(AxonError::Validation(format!(
                "`prompt` must be a string, got {}",
                json_type_name(other)
            )))
        }
    };
    let context = match obj.get("context") {
        None | Some(Value::Null) => None,
        Some(Value::String(s)) => Some(s.as_str()),
        Some(other) => {
            return Err(AxonError::Validation(format!(
                "`context` must be a string if present, got {}",
                json_type_name(other)
            )))
        }
    };
    Ok((prompt, context))
}

/// Human-readable JSON type name for validation error messages.
/// Lowercase (`"number"`, `"boolean"`, …) so error text reads
/// naturally.
fn json_type_name(v: &Value) -> &'static str {
    match v {
        Value::Null => "null",
        Value::Bool(_) => "boolean",
        Value::Number(_) => "number",
        Value::String(_) => "string",
        Value::Array(_) => "array",
        Value::Object(_) => "object",
    }
}

/// `AxonError` → `McpError` conversion specialized for the adapter
/// boundary. We delegate to the typed `From<AxonError> for McpError`
/// impl, which is the shared taxonomy pin between MCP and EAL
/// surfaces (see `mcp/error.rs`). A bespoke match here would be a
/// second source of truth for the mapping, which is exactly what
/// the shared impl exists to prevent.
fn axon_to_mcp(err: AxonError) -> McpError {
    McpError::from(err)
}

// ── Tests ──────────────────────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    //! Tests cover four layers:
    //!
    //! 1. Argument parsing (`parse_chat_args`) — exhaustive on valid /
    //!    invalid shapes. The production handler builds its error
    //!    messages from this, so any regression here mis-shapes every
    //!    downstream agent-visible error.
    //! 2. Adapter construction (`AgentDispatchAdapter::{build, empty}`)
    //!    — ensures zero-agent and multi-agent registries produce the
    //!    right `tool_specs` and `handle` behaviour.
    //! 3. Dispatch routing (`handle`) — unknown tool returns `None`
    //!    (provider must fall through), known tool reaches the handler,
    //!    handler errors surface as `Some(Err(McpError))`.
    //! 4. Name-collision regression — network tool names and ability
    //!    names cannot alias.

    use super::*;
    use crate::registry::agents::{AgentEntry, AgentRegistry, AgentType};

    fn registry_with(agents: Vec<(&str, AgentType)>) -> AgentRegistry {
        let mut r = AgentRegistry::default();
        for (name, t) in agents {
            r.agents.insert(name.to_string(), AgentEntry::new(t, None));
        }
        r
    }

    // ── parse_chat_args ────────────────────────────────────────────────────

    #[test]
    fn parse_chat_args_accepts_prompt_only() {
        let v = json!({"prompt": "hello"});
        let (p, c) = parse_chat_args(&v).unwrap();
        assert_eq!(p, "hello");
        assert!(c.is_none());
    }

    #[test]
    fn parse_chat_args_accepts_prompt_and_context() {
        let v = json!({"prompt": "hello", "context": "be terse"});
        let (p, c) = parse_chat_args(&v).unwrap();
        assert_eq!(p, "hello");
        assert_eq!(c, Some("be terse"));
    }

    #[test]
    fn parse_chat_args_treats_null_context_as_absent() {
        // JSON callers sometimes emit `{"context": null}` when a
        // templated variable was left unset. That is semantically
        // "no context", not "validation error".
        let v = json!({"prompt": "hi", "context": null});
        let (_, c) = parse_chat_args(&v).unwrap();
        assert!(c.is_none());
    }

    #[test]
    fn parse_chat_args_rejects_non_object_payload() {
        for v in [json!(null), json!("string"), json!([1, 2, 3]), json!(42)] {
            let err = parse_chat_args(&v).expect_err("must reject");
            assert!(
                matches!(err, AxonError::Validation(_)),
                "non-object payload must be a Validation error, got {err:?}"
            );
        }
    }

    #[test]
    fn parse_chat_args_rejects_missing_prompt() {
        let v = json!({"context": "lone context is insufficient"});
        let err = parse_chat_args(&v).expect_err("must reject");
        let msg = format!("{err}");
        assert!(
            msg.contains("prompt"),
            "error must name the missing field, got {msg}"
        );
    }

    #[test]
    fn parse_chat_args_rejects_null_prompt() {
        let v = json!({"prompt": null});
        let err = parse_chat_args(&v).expect_err("must reject");
        assert!(matches!(err, AxonError::Validation(_)));
    }

    #[test]
    fn parse_chat_args_rejects_non_string_prompt() {
        for v in [
            json!({"prompt": 42}),
            json!({"prompt": true}),
            json!({"prompt": ["an", "array"]}),
            json!({"prompt": {"nested": "object"}}),
        ] {
            let err = parse_chat_args(&v).expect_err("must reject");
            let msg = format!("{err}");
            assert!(
                msg.contains("prompt"),
                "error must name the offending field, got {msg}"
            );
            assert!(
                msg.contains("string"),
                "error must name the expected type, got {msg}"
            );
        }
    }

    #[test]
    fn parse_chat_args_rejects_non_string_context() {
        let v = json!({"prompt": "hello", "context": 42});
        let err = parse_chat_args(&v).expect_err("must reject");
        let msg = format!("{err}");
        assert!(msg.contains("context"));
        assert!(msg.contains("string"));
    }

    /// Edge case: `context` is an empty string. An empty preamble is a
    /// legitimate caller choice (they wanted to pass an explicit "no
    /// system prompt" signal through a templated variable that happened
    /// to be empty), so it must be accepted — not conflated with `None`
    /// or rejected as a validation error.
    #[test]
    fn parse_chat_args_accepts_empty_string_context() {
        let v = json!({"prompt": "hi", "context": ""});
        let (_, c) = parse_chat_args(&v).unwrap();
        assert_eq!(c, Some(""));
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
        let a = AgentDispatchAdapter::build(&registry, "tenant-x");
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
        let a = AgentDispatchAdapter::build(&registry, "tenant-x");
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
        let a = AgentDispatchAdapter::build(&registry, "tenant-x");
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
        // Missing `prompt` arrives at the handler, fails the validation
        // inside `dispatch_chat`, and surfaces as a Validation McpError.
        // The whole point of this test is to prove the routing reached
        // the handler (Some(Err(...))), not None (fall-through).
        let registry = registry_with(vec![("claude", AgentType::ClaudeCode)]);
        let a = AgentDispatchAdapter::build(&registry, "tenant-x");
        let outcome = a.handle("claude.chat", &Map::new()).expect("known tool must return Some");
        let err = outcome.expect_err("missing prompt must fail validation");
        assert_eq!(err.error_code(), "validation_error");
        assert!(err.message().contains("prompt"));
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
        let a = AgentDispatchAdapter::build(&registry, "tenant-x");
        for spec in a.tool_specs() {
            let name = spec["name"].as_str().unwrap();
            assert!(
                name.contains('.'),
                "ability name {name:?} must carry the `.` shape to \
                 avoid collision with network tool names"
            );
        }
    }

    /// Regression: `AxonError::Validation` from the handler must map
    /// to `McpError::Validation` (error_code `validation_error`).
    /// Pinned because the round-trip is the visible contract an
    /// agent-side caller branches on.
    #[test]
    fn axon_validation_round_trips_to_mcp_validation() {
        let err = axon_to_mcp(AxonError::Validation("bad".into()));
        assert_eq!(err.error_code(), "validation_error");
    }
}
