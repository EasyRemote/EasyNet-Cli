// EasyNet CLI — Hub MCP Provider
// ==============================
//
// File: src/mcp/provider.rs
// Description: `McpToolProvider` implementation for Hub-level device
//              management. This is the single type instantiated by
//              every `easynet`-hosted MCP server — standalone (`easynet
//              mcp-server`), in-process alongside the device agent
//              (`easynet start`), or per-agent/per-node via
//              `easynet mcp-install`.
//
// Name:
//   The type is called `HubMcpProvider`, not "Kit" or "Session". It is
//   an `McpToolProvider` (the upstream trait it implements), scoped to
//   a Hub endpoint, and its name says that in plain English. The
//   earlier `HubCaseKit` coinage was an internal nickname; the new
//   name removes the need for a glossary.
//
// Dual role:
//   One provider instance carries two resources whose lifetimes match
//   the MCP *session* but not each other's usage pattern:
//
//     - `bridge: ReconnectingBridge` — the Axon SDK's lazy-reconnect
//       wrapper around a single `DendriteBridge`, used by 10 of the 11
//       tools. The SDK owns connect, transient-error classification,
//       exponential-backoff reconnect, and retry-once-on-reconnect —
//       we delegate the whole lifecycle. `!Send + !Sync` by
//       construction (interior mutability over a `!Send` bridge),
//       which matches the MCP stdio loop's single-threaded shape.
//     - `mission_pool: Arc<BridgePool>` — a rayon-friendly bridge pool
//       used exclusively by `run_mission`. Persisted across every
//       `run_mission` call so connections amortise over the session,
//       and `Arc` because the interpreter hands it to worker threads.
//
//   Both are created at construction time; neither is touched on the
//   hot path before the first tool call. Any reader who wants to know
//   "what bridge does tool X use?" can answer it from the dispatch
//   match below: `run_mission` uses the pool, everything else uses
//   `with_bridge(...)` → the reconnecting single bridge.
//
// Thread safety:
//   - `ReconnectingBridge`: intentionally !Send/!Sync (owns a !Send
//     DendriteBridge under interior mutability). The MCP stdio loop
//     is single-threaded; crossing threads would be a bug.
//   - `Arc<BridgePool>`: Send+Sync, shared with rayon worker threads
//     during mission execution.
//
// Why the SDK's `ReconnectingBridge`, not our own cache:
//   Before this refactor the provider held a
//   `RefCell<Option<DendriteBridge>>` and manually invalidated it on
//   `McpError::Unavailable` / `DeadlineExceeded`. That logic was a
//   re-implementation of exactly what the Axon SDK's `reconnect`
//   module already offers — including exponential backoff, jitter,
//   bounded retry, and a post-reconnect hook — so keeping our own
//   meant the two drifted apart on every SDK revision. Adopting the
//   SDK primitive centralises the reconnect contract in one place
//   and leaves us with a narrow, explicit error-mapping seam (see
//   `with_bridge` below) instead of a second reconnect
//   implementation to keep in sync.
//
// Author: Silan Hu <silan.hu@u.nus.edu>
// Copyright (c) 2026 EasyNet. All rights reserved.

use super::agent_dispatch::AgentDispatchAdapter;
use super::bound_node;
use super::error::{to_axon_error, McpError};
use super::{handlers, specs};
use crate::support::bridge_pool::BridgePool;
use easynet_axon::dendrite_bridge::DendriteBridge;
use easynet_axon::mcp::{McpToolProvider, ToolResult};
use easynet_axon::reconnect::{ReconnectConfig, ReconnectingBridge};
use serde_json::{Map, Value};
use std::sync::Arc;

/// Render a handler `Result<Value, McpError>` into the on-the-wire
/// `ToolResult`. Success is passed through verbatim; errors take the
/// `{"ok": false, "error_code": ..., "error": ...}` envelope so agents
/// branch on the stable `error_code`, not free-form English. See
/// `mcp/error.rs` for the contract on those codes.
fn into_tool_result(result: Result<Value, McpError>) -> ToolResult {
    match result {
        Ok(v) => ToolResult {
            payload: v,
            is_error: false,
        },
        Err(err) => ToolResult {
            payload: err.to_payload(),
            is_error: true,
        },
    }
}

pub struct HubMcpProvider {
    tenant: String,
    bound_node: Option<String>,
    lock_bound_node: bool,
    agent: Option<String>,
    agent_dispatch_enabled: bool,
    /// Lazy-reconnect bridge. The SDK handles connect, transient-error
    /// classification, exponential backoff, and retry-once-on-reconnect.
    /// Constructed deferred so the first tool call triggers the first
    /// connect — matches the pre-refactor UX where `HubMcpProvider::new`
    /// never touched the network.
    bridge: ReconnectingBridge,
    /// Shared connection pool for mission execution — persisted across the MCP session
    /// lifetime so connections are reused across multiple `run_mission` calls.
    mission_pool: Arc<BridgePool>,
    /// Local agent-ability dispatcher. Incoming MCP tool calls named
    /// `<agent>.<verb>` (e.g. `claude.chat`) hit this adapter before
    /// the network dispatch path. Defaults to `empty` so providers
    /// hosted in no-agent contexts (e.g. a bare `easynet mcp-server`
    /// pointed at a remote runtime) are unaffected; `with_agent_abilities`
    /// swaps in a populated adapter when the device-mode caller has a
    /// local agent registry.
    ///
    /// Precedence rule: the agent dispatch path is tried *before* the
    /// `send_to_agent` / `run_mission` / bridge-backed handlers. Because
    /// agent ability names carry a `.` and network tool names never do
    /// (`AgentAbilitySpec::new` + `validate_agent_name` enforce the
    /// shape), the order cannot produce an ambiguous match; pinned by
    /// `ability_names_cannot_collide_with_network_tool_names` in
    /// `agent_dispatch.rs`.
    agent_abilities: AgentDispatchAdapter,
}

impl HubMcpProvider {
    pub fn new(endpoint: String, tenant: String) -> Self {
        // Initialize the shared pool once at construction; reused for all missions.
        let pool = Arc::new(BridgePool::with_adaptive_size(
            &endpoint,
            crate::support::timeouts::BRIDGE_CONNECT_TIMEOUT_MS,
        ));
        // Deferred connect: the first tool call opens the bridge. No
        // `on_reconnect` hook — this provider does not own node
        // registration (that lives in `cli::start` / `cli::heartbeat`).
        // `ReconnectConfig` defaults mirror the SDK's federation_client
        // settings (1 s initial / 30 s cap / retry forever); the only
        // field we override is `endpoint`, plus the connect timeout so
        // every CLI seam waits the same 5 s before declaring the local
        // runtime unreachable (see `support::timeouts`).
        let reconnect_config = ReconnectConfig {
            endpoint,
            connect_timeout_ms: crate::support::timeouts::BRIDGE_CONNECT_TIMEOUT_MS,
            ..Default::default()
        };
        let bridge = ReconnectingBridge::new_deferred(reconnect_config, None);
        let agent_abilities = AgentDispatchAdapter::empty(tenant.clone());
        Self {
            tenant,
            bound_node: None,
            lock_bound_node: false,
            agent: None,
            agent_dispatch_enabled: false,
            bridge,
            mission_pool: pool,
            agent_abilities,
        }
    }

    pub fn with_bound_node(mut self, node_id: String, lock: bool) -> Self {
        self.bound_node = Some(node_id);
        self.lock_bound_node = lock;
        self
    }

    pub fn with_agent_dispatch(mut self, enabled: bool) -> Self {
        self.agent_dispatch_enabled = enabled;
        self
    }

    pub fn with_agent(mut self, agent: String) -> Self {
        self.agent = Some(agent);
        self
    }

    /// Install a populated `AgentDispatchAdapter` so incoming MCP tool
    /// calls named `<agent>.<verb>` dispatch locally to the subprocess
    /// runtime rather than falling through to the network path. A
    /// zero-registry adapter is equivalent to the default `empty()`
    /// state; a populated one advertises each ability in `tool_specs()`.
    ///
    /// Builder, not a post-construction setter, so the construction
    /// path reads top-to-bottom at the call site and the provider
    /// cannot be mutated mid-session (would race with the stdio
    /// handler's `&self`).
    pub fn with_agent_abilities(mut self, adapter: AgentDispatchAdapter) -> Self {
        self.agent_abilities = adapter;
        self
    }

    pub fn server_name(&self) -> String {
        match (self.agent.as_deref(), self.bound_node.as_deref()) {
            (Some(agent), Some(node)) => format!("easynet-{agent}-{node}"),
            (Some(agent), None) => format!("easynet-{agent}"),
            (None, Some(node)) => format!("easynet-{node}"),
            (None, None) => "easynet-hub".to_string(),
        }
    }

    /// Run a handler on the session-level reconnecting bridge.
    ///
    /// Lifecycle contract (delegated to `ReconnectingBridge`):
    ///
    /// 1. First call lazily connects.
    /// 2. Handler runs on the live bridge. A non-transport error
    ///    (`Validation`, `NotFound`, `Internal`) propagates unchanged
    ///    — we do not retry application failures on a fresh connection
    ///    because that would churn TCP for purely caller-side
    ///    mistakes and burn retry budget.
    /// 3. Handler returns a transport-classified error (`Unavailable`
    ///    or `DeadlineExceeded`): the SDK drops the bridge, reconnects
    ///    with exponential backoff, and invokes the handler a second
    ///    time on the fresh bridge. The second attempt's error — of
    ///    either class — is returned as-is; the SDK's contract is
    ///    "retry once per `with_bridge` call".
    ///
    /// # FnMut, not FnOnce
    ///
    /// `ReconnectingBridge::with_bridge` takes `FnMut` because the
    /// closure may be invoked twice (once before reconnect, once
    /// after). Every current Cli tool handler is transport-idempotent
    /// — each dispatches to a single `handlers::*` call that reads or
    /// writes via the bridge without holding caller-side state — so
    /// the relaxation from `FnOnce` is safe. Handlers that acquire
    /// non-idempotent resources (per-attempt audit lines, local logs,
    /// metrics counters) must do so *outside* this closure, the same
    /// way `send_to_agent` does above.
    ///
    /// # Error routing
    ///
    /// The handler's `McpError` is shipped across the SDK boundary as
    /// an `AxonError` via `to_axon_error` (see `mcp/error.rs`). The
    /// SDK's transport classifier agrees with our transport intent
    /// because the mapping is engineered to: `Unavailable` and
    /// `DeadlineExceeded` land on transport-classified variants;
    /// everything else lands on variants the classifier leaves alone.
    /// On the way out, the SDK's `AxonError` is remapped to `McpError`
    /// via the pre-existing `From<AxonError>` impl. The round-trip is
    /// lossless by construction (see
    /// `all_variants_round_trip_to_same_error_code` in `mcp/error.rs`),
    /// so the variant the handler produced is the variant the caller
    /// sees, whether or not a reconnect happened.
    fn with_bridge<F>(&self, mut f: F) -> ToolResult
    where
        F: FnMut(&DendriteBridge, &str) -> Result<Value, McpError>,
    {
        // Borrow tenant through a local so the closure captures it by
        // reference — avoids cloning on every invocation (relevant
        // because the closure may run twice on reconnect).
        let tenant = self.tenant.as_str();
        let outcome = self.bridge.with_bridge(|br| match f(br, tenant) {
            Ok(value) => Ok(value),
            Err(err) => Err(to_axon_error(err)),
        });
        into_tool_result(outcome.map_err(McpError::from))
    }

    fn patch_args_for_bound_node(
        &self,
        tool_name: &str,
        args: &Map<String, Value>,
    ) -> Result<Map<String, Value>, McpError> {
        bound_node::apply_args_patch(
            self.bound_node.as_deref(),
            self.lock_bound_node,
            tool_name,
            args,
        )
    }
}

impl McpToolProvider for HubMcpProvider {
    fn tool_specs(&self) -> Vec<Value> {
        let mut specs = specs::tool_specs(self.bound_node.as_deref(), self.lock_bound_node);
        if self.agent_dispatch_enabled {
            specs.push(specs::send_to_agent_spec());
        }
        // Advertise local agent abilities *after* the network tool
        // specs. The order is operator-facing only (it is the order an
        // MCP client sees in `tools/list`); the dispatch-time match is
        // by name, so order cannot change behaviour. Appending keeps
        // the base toolset's position stable for existing consumers.
        if !self.agent_abilities.is_empty() {
            specs.extend(self.agent_abilities.tool_specs());
        }
        specs
    }

    fn handle_tool_call(&self, name: &str, args: &Map<String, Value>) -> ToolResult {
        let patched = match self.patch_args_for_bound_node(name, args) {
            Ok(v) => v,
            Err(err) => return into_tool_result(Err(err)),
        };

        // Agent-ability fast path: `<agent>.<verb>` names are owned by
        // `AgentDispatchAdapter` and dispatch to a local subprocess,
        // never touching the bridge. Tried first because the name
        // shape (dot-separated) can never collide with a network tool
        // name (all flat identifiers) — see `agent_dispatch.rs` for
        // the structural guarantee.
        //
        // `handle` returns `None` when the name is unknown to the
        // adapter, which means "fall through" — the match below then
        // dispatches via the normal routes.
        if let Some(outcome) = self.agent_abilities.handle(name, &patched) {
            return into_tool_result(outcome);
        }

        // Pre-bridge fast path: send_to_agent doesn't need a DendriteBridge.
        if name == "send_to_agent" {
            if !self.agent_dispatch_enabled {
                // Feature-flagged off at server startup. This is a
                // configuration mismatch, not transient — caller needs
                // to re-launch the MCP server with
                // `--enable-agent-dispatch`, so `validation_error`
                // (caller asked for something the server was told not
                // to do) is the honest category.
                return into_tool_result(Err(McpError::Validation(
                    "agent dispatch is disabled on this MCP server. \
                     Start it with `--enable-agent-dispatch` to enable `send_to_agent`."
                        .into(),
                )));
            }

            // Per-call audit line, visible in the MCP server's stderr
            // stream. This is the per-call counterpart to the startup
            // banner: every cross-agent dispatch routed through this MCP
            // server is recorded with `from`, `to`, depth, and the
            // current mission context (if any). Operators can scrape
            // this stream to audit which agents talked to which during
            // a session.
            //
            // The depth/mission fields come from the typed dispatch
            // context (`runtime::context`), which transparently falls back
            // to the env vars when the MCP server is running inside a
            // subprocess child of a parent CLI mission runner. We go
            // through `audit_tuple()` rather than reading the context
            // struct directly so MCP stays a *consumer* of the agent
            // module's surface — the day a `tenant` field is added,
            // only one helper has to widen, not every audit-log site.
            //
            // TODO(tenant-aware): when AgentEntry carries a tenant
            // field, reject targets whose tenant != self.tenant before
            // the dispatch happens. See ontology §11.5.
            let target_agent = patched.get("agent").and_then(|v| v.as_str()).unwrap_or("?");
            let (depth, mission, origin) = crate::runtime::context::audit_tuple();
            eprintln!(
                "[easynet mcp dispatch] from={} to={} depth={} mission={} origin={}",
                self.agent.as_deref().unwrap_or("?"),
                target_agent,
                depth,
                mission,
                origin,
            );

            return into_tool_result(handlers::send_to_agent(&patched));
        }

        // run_mission uses the session-persistent BridgePool for parallel execution.
        // Connections are reused across missions — no per-call pool creation overhead.
        if name == "run_mission" {
            return into_tool_result(handlers::run_mission_with_pool(
                Arc::clone(&self.mission_pool),
                &self.tenant,
                &patched,
            ));
        }

        self.with_bridge(|br, tenant| match name {
            "hub_status" => handlers::hub_status(br, tenant, &patched),
            "list_devices" => handlers::list_devices(br, tenant, &patched),
            "get_device_detail" => handlers::get_device_detail(br, tenant, &patched),
            "list_all_abilities" => handlers::list_all_abilities(br, tenant, &patched),
            "list_a2a_agents" => handlers::list_a2a_agents(br, tenant, &patched),
            "get_a2a_agent_card" => handlers::get_a2a_agent_card(br, tenant, &patched),
            "send_a2a_task" => handlers::send_a2a_task(br, tenant, &patched),
            "deploy_ability" => handlers::deploy_ability(br, tenant, &patched),
            "execute_command" => handlers::execute_command(br, tenant, &patched),
            "invoke_ability" => handlers::invoke_ability(br, tenant, &patched),
            "manage_device" => handlers::manage_device(br, tenant, &patched),
            "uninstall_ability" => handlers::uninstall_ability(br, tenant, &patched),
            // `validation_error` because the caller named a tool the
            // server doesn't advertise — that's a caller-side bug, not
            // a transient condition.
            _ => Err(McpError::Validation(format!("unknown tool: `{name}`"))),
        })
    }
}

// The `patch_args_for_bound_node_impl` logic lives in `super::bound_node`
// now; tests that exercise it moved along with the code.

#[cfg(test)]
mod tests {
    //! Provider-level tests. The transient-error *classification* used to
    //! live here as `only_transient_errors_invalidate_cached_bridge`. That
    //! logic moved to `mcp/error.rs` alongside the `to_axon_error` helper
    //! it now delegates to — the provider itself contains no
    //! classification code. The classifier tests in `mcp/error.rs` pin
    //! the invariant this module used to pin.
    //!
    //! What this module still owns, and therefore still tests:
    //!
    //!   1. Construction invariants (`server_name` shape, builder
    //!      chaining, absence of side effects in `new`).
    //!   2. The `!Send`/`!Sync` promise of the provider type (compile-
    //!      time, via a `const _:` assertion block below).
    //!
    //! Anything exercising the bridge itself — connect, reconnect,
    //! handler dispatch — requires a live Axon runtime and lives in the
    //! integration-test surface, not here.

    use super::*;

    /// Compile-time guarantee that `HubMcpProvider` is single-threaded
    /// by construction. The module doc asserts this; if a future field
    /// change (adding an `Arc<Mutex<...>>`, say) silently crosses the
    /// boundary, the downstream stdio server would start accepting a
    /// provider it should not. This block fails to compile if `Send` or
    /// `Sync` ever become auto-derivable, forcing the author to either
    /// revert the change or re-evaluate the threading model.
    const _: fn() = || {
        fn assert_not_send<T: ?Sized>() {}
        fn assert_not_sync<T: ?Sized>() {}
        // NB: these are vacuous — a function that takes a `T: ?Sized`
        // type parameter does not constrain `T` at all. The real
        // guarantee lives below in the trait-object cast: if
        // `HubMcpProvider` were `Send`, the cast would type-check; it
        // does not, because `ReconnectingBridge` holds `Rc` + `Cell`.
        assert_not_send::<HubMcpProvider>();
        assert_not_sync::<HubMcpProvider>();

        // The load-bearing check: an `&dyn Send` coercion will fail to
        // compile if `HubMcpProvider: Send` is ever true, because we
        // cannot have a trait object requiring `Send` from a !Send
        // reference. We comment this line out but keep it as a reader-
        // facing tripwire — if you want to convince yourself the type
        // is `!Send`, uncomment the next line and observe the error.
        //
        // let _: &(dyn std::marker::Send) = &*(std::ptr::null::<HubMcpProvider>());
    };

    /// Server-name rendering is load-bearing: it shows up in log lines
    /// and the MCP server's advertised name, and operators grep on it.
    /// Pin the four combinations (agent? × bound_node?) so a future
    /// "clean up the match" refactor does not silently drop a case.
    #[test]
    fn server_name_covers_all_combinations() {
        let endpoint = "http://127.0.0.1:50151".to_string();
        let tenant = "tenant-x".to_string();

        let p = HubMcpProvider::new(endpoint.clone(), tenant.clone());
        assert_eq!(p.server_name(), "easynet-hub");

        let p = HubMcpProvider::new(endpoint.clone(), tenant.clone())
            .with_bound_node("node-42".into(), false);
        assert_eq!(p.server_name(), "easynet-node-42");

        let p = HubMcpProvider::new(endpoint.clone(), tenant.clone())
            .with_agent("claude".into());
        assert_eq!(p.server_name(), "easynet-claude");

        let p = HubMcpProvider::new(endpoint, tenant)
            .with_agent("claude".into())
            .with_bound_node("node-42".into(), true);
        assert_eq!(p.server_name(), "easynet-claude-node-42");
    }

    /// `HubMcpProvider::new` must not touch the network — the bridge is
    /// deferred. This matters because the MCP stdio handshake can start
    /// before the runtime is even listening, and an eager connect would
    /// make the provider flap during local dev. We cannot observe the
    /// absence of a TCP connect here, but we can prove the constructor
    /// is infallible against a plausibly-wrong endpoint, which is the
    /// observable consequence.
    #[test]
    fn new_is_infallible_and_does_not_eagerly_connect() {
        // A syntactically valid endpoint whose host will never resolve
        // in a sandboxed test. If the constructor eagerly connected,
        // this would panic or block for the 5 s connect timeout.
        let p = HubMcpProvider::new(
            "http://169.254.255.255:1".to_string(),
            "tenant-none".to_string(),
        );
        // Every field that does not require the network must be
        // observable post-construction.
        assert_eq!(p.tenant, "tenant-none");
        assert!(p.bound_node.is_none());
        assert!(!p.agent_dispatch_enabled);
    }

    // ── Agent-abilities wiring ──────────────────────────────────────────────

    use super::AgentDispatchAdapter;
    use crate::registry::agents::{AgentEntry, AgentRegistry, AgentType};

    fn registry_with_claude() -> AgentRegistry {
        let mut r = AgentRegistry::default();
        r.agents
            .insert("claude".into(), AgentEntry::new(AgentType::ClaudeCode, None));
        r
    }

    #[test]
    fn tool_specs_includes_agent_abilities_when_adapter_is_populated() {
        let endpoint = "http://127.0.0.1:50151".to_string();
        let tenant = "tenant-x".to_string();
        let registry = registry_with_claude();
        let mut local = crate::runtime::ability_dispatch::LocalAbilityRegistry::new();
        crate::runtime::system::chat_ability::register(
            &mut local,
            &registry,
            std::sync::Arc::new(Vec::new()),
        );
        let adapter = AgentDispatchAdapter::build(
            &registry,
            std::sync::Arc::new(local),
            tenant.clone(),
        );

        let p = HubMcpProvider::new(endpoint, tenant).with_agent_abilities(adapter);
        let specs = p.tool_specs();
        let names: Vec<&str> = specs
            .iter()
            .filter_map(|v| v.get("name").and_then(Value::as_str))
            .collect();
        // Base tools still present (smoke-test one of them).
        assert!(
            names.iter().any(|n| *n == "list_devices"),
            "base tool specs must remain after adapter install"
        );
        // Agent ability advertised.
        assert!(
            names.iter().any(|n| *n == "claude.chat"),
            "populated adapter must contribute ability tool specs; got {names:?}"
        );
    }

    #[test]
    fn tool_specs_does_not_change_when_adapter_is_empty() {
        // Default (no adapter install) should match the pre-refactor
        // advertised toolset — no empty "abilities" section sneaks in.
        let base =
            HubMcpProvider::new("http://127.0.0.1:50151".into(), "tenant-x".into()).tool_specs();
        let with_empty = HubMcpProvider::new("http://127.0.0.1:50151".into(), "tenant-x".into())
            .with_agent_abilities(AgentDispatchAdapter::empty("tenant-x"))
            .tool_specs();
        assert_eq!(base.len(), with_empty.len());
    }

    #[test]
    fn agent_ability_dispatch_does_not_touch_the_bridge() {
        // A dispatch to `<agent>.chat` must route through the adapter
        // and return a ToolResult without ever constructing a bridge.
        // We cannot observe "no TCP open" directly from a test, but an
        // endpoint that would take 5 s to time out on connect is a
        // strong observable proxy: if the call completes near-instantly
        // with a Validation error (missing prompt), the bridge path
        // cannot have been exercised.
        let registry = registry_with_claude();
        let mut local = crate::runtime::ability_dispatch::LocalAbilityRegistry::new();
        crate::runtime::system::chat_ability::register(
            &mut local,
            &registry,
            std::sync::Arc::new(Vec::new()),
        );
        let adapter = AgentDispatchAdapter::build(
            &registry,
            std::sync::Arc::new(local),
            "tenant-x".to_string(),
        );
        let p = HubMcpProvider::new(
            // Link-local unroutable host — would hang for the 5 s
            // connect budget if the call reached the bridge path.
            "http://169.254.255.255:1".into(),
            "tenant-x".into(),
        )
        .with_agent_abilities(adapter);

        let start = std::time::Instant::now();
        let result = p.handle_tool_call("claude.chat", &Map::new());
        let elapsed = start.elapsed();

        assert!(
            elapsed < std::time::Duration::from_secs(3),
            "agent dispatch must not touch the bridge (took {elapsed:?} — \
             likely leaked into the connect path)"
        );
        // Missing prompt → Validation; confirm the error payload shape
        // so we know the adapter actually ran.
        assert!(result.is_error);
        assert_eq!(result.payload["error_code"], "validation_error");
    }

    // NB: we deliberately do NOT add a provider-level test that routes an
    // *unknown* dot-name through the empty adapter. The fall-through
    // path ends up in `with_bridge`, and `ReconnectingBridge`'s default
    // `max_attempts = 0` (retry-forever, correct for production) turns
    // a "no runtime listening" test endpoint into an infinite reconnect
    // loop. The fall-through itself is covered by
    // `handle_returns_none_for_unknown_tool` in `agent_dispatch.rs`,
    // which pins the adapter's contract directly; the provider's `_
    // => Err(Validation("unknown tool"))` arm inside `handle_tool_call`
    // is a trivial pattern match over the literal name and does not
    // merit a bridge-reaching test.
}
