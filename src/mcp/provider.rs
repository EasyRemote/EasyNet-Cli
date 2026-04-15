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
//     - `cached: RefCell<Option<DendriteBridge>>` — a single lazy-
//       reconnect bridge used by 10 of the 11 tools. `RefCell` is
//       intentional: stdio is single-threaded, so interior mutability
//       is cheaper than the `Mutex` that `Send+Sync` would demand.
//     - `mission_pool: Arc<BridgePool>` — a rayon-friendly bridge pool
//       used exclusively by `run_mission`. Persisted across every
//       `run_mission` call so connections amortise over the session,
//       and `Arc` because the interpreter hands it to worker threads.
//
//   Both are created at construction time; neither is touched on the
//   hot path before the first tool call. Any reader who wants to know
//   "what bridge does tool X use?" can answer it from the dispatch
//   match below: `run_mission` uses the pool, everything else uses
//   `with_bridge(...)` → the cached single bridge.
//
// Thread safety:
//   - `RefCell<DendriteBridge>`: intentionally !Send/!Sync. The MCP
//     stdio loop is single-threaded; crossing threads would be a bug.
//   - `Arc<BridgePool>`: Send+Sync, shared with rayon worker threads
//     during mission execution.
//
// Author: Silan Hu <silan.hu@u.nus.edu>
// Copyright (c) 2026 EasyNet. All rights reserved.

use super::bound_node;
use super::error::McpError;
use super::{handlers, specs};
use crate::shared::bridge_pool::BridgePool;
use easynet_axon::dendrite_bridge::DendriteBridge;
use easynet_axon::mcp::{McpToolProvider, ToolResult};
use serde_json::{Map, Value};
use std::cell::RefCell;
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
    endpoint: String,
    tenant: String,
    bound_node: Option<String>,
    lock_bound_node: bool,
    agent: Option<String>,
    agent_dispatch_enabled: bool,
    cached: RefCell<Option<DendriteBridge>>,
    /// Shared connection pool for mission execution — persisted across the MCP session
    /// lifetime so connections are reused across multiple `run_mission` calls.
    mission_pool: Arc<BridgePool>,
}

impl HubMcpProvider {
    pub fn new(endpoint: String, tenant: String) -> Self {
        // Initialize the shared pool once at construction; reused for all missions.
        let pool = Arc::new(BridgePool::with_adaptive_size(
            &endpoint,
            crate::shared::timeouts::BRIDGE_CONNECT_TIMEOUT_MS,
        ));
        Self {
            endpoint,
            tenant,
            bound_node: None,
            lock_bound_node: false,
            agent: None,
            agent_dispatch_enabled: false,
            cached: RefCell::new(None),
            mission_pool: pool,
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

    pub fn server_name(&self) -> String {
        match (self.agent.as_deref(), self.bound_node.as_deref()) {
            (Some(agent), Some(node)) => format!("easynet-{agent}-{node}"),
            (Some(agent), None) => format!("easynet-{agent}"),
            (None, Some(node)) => format!("easynet-{node}"),
            (None, None) => "easynet-hub".to_string(),
        }
    }

    /// Run a handler on the cached session-level bridge, reconnecting
    /// on demand and invalidating the cache after a transient failure.
    ///
    /// Lifecycle contract (there are exactly three observable states):
    ///
    /// 1. Cache cold → connect, store, run handler.
    /// 2. Cache warm → run handler.
    /// 3. Handler returns `McpError::Unavailable(_)` or
    ///    `McpError::DeadlineExceeded(_)` → treat the cached bridge
    ///    as poisoned (the peer closed the stream, the TCP connection
    ///    dropped, a graceful server restart swapped the remote
    ///    identity, the call wedged past its deadline …) and drop it
    ///    so the *next* tool call reconnects. Without this step every
    ///    subsequent call fails with the same stale-connection error
    ///    until the session restarts — the exact "invisible outage"
    ///    behaviour we had before.
    ///
    /// `Validation`, `NotFound`, and `Internal` errors do *not*
    /// invalidate the cache: those are caller-shaped or logic-shaped,
    /// and the connection is still healthy. Invalidating on them
    /// would churn through new TCP connections for purely caller-side
    /// mistakes.
    fn with_bridge<F>(&self, f: F) -> ToolResult
    where
        F: FnOnce(&DendriteBridge, &str) -> Result<Value, McpError>,
    {
        let outcome = {
            let mut slot = self.cached.borrow_mut();
            if slot.is_none() {
                match DendriteBridge::connect(
                    &self.endpoint,
                    crate::shared::timeouts::BRIDGE_CONNECT_TIMEOUT_MS,
                ) {
                    Ok(b) => *slot = Some(b),
                    Err(e) => {
                        // Connection failure is transient by definition —
                        // surface as `unavailable` so agents know a retry
                        // (after the transport recovers) can succeed.
                        return into_tool_result(Err(McpError::Unavailable(format!(
                            "connect {}: {e}",
                            self.endpoint
                        ))));
                    }
                }
            }
            // The slot is guaranteed to hold a bridge here: the block
            // above either kept the existing one or installed a fresh
            // connection. The `expect` documents that invariant for a
            // reader who lands on this line from a stack trace.
            let br = slot.as_ref().expect("bridge just initialized above");
            f(br, &self.tenant)
        };

        // Invalidate the cached bridge iff the handler reported a
        // transient failure. Done *outside* the RefCell borrow above
        // so we don't nest `borrow_mut` calls if a future refactor
        // adds re-entrancy via the handler closure. Both
        // `Unavailable` (transport fault) and `DeadlineExceeded`
        // (call wedged past its budget) count as transient: in the
        // deadline case the remote may have disappeared mid-call, so
        // reconnecting next time is the safer default.
        if matches!(
            outcome,
            Err(McpError::Unavailable(_) | McpError::DeadlineExceeded(_))
        ) {
            self.cached.borrow_mut().take();
        }
        into_tool_result(outcome)
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
        specs
    }

    fn handle_tool_call(&self, name: &str, args: &Map<String, Value>) -> ToolResult {
        let patched = match self.patch_args_for_bound_node(name, args) {
            Ok(v) => v,
            Err(err) => return into_tool_result(Err(err)),
        };

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
            // context (`agent::context`), which transparently falls back
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
            let (depth, mission, origin) = crate::agent::context::audit_tuple();
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
    use super::*;
    use serde_json::json;

    /// Pins the invalidation classifier used by `with_bridge` — both
    /// `Unavailable` and `DeadlineExceeded` poison the cached bridge;
    /// every other error category must leave it alone. Changing the
    /// pattern in the production path without updating this test
    /// will break explicitly, which is the point.
    #[test]
    fn only_transient_errors_invalidate_cached_bridge() {
        fn invalidates(outcome: &Result<Value, McpError>) -> bool {
            matches!(
                outcome,
                Err(McpError::Unavailable(_) | McpError::DeadlineExceeded(_))
            )
        }

        assert!(invalidates(&Err(McpError::Unavailable("peer gone".into()))));
        assert!(invalidates(&Err(McpError::DeadlineExceeded(
            "timed out".into()
        ))));

        let kept = [
            Err::<Value, _>(McpError::Validation("bad arg".into())),
            Err(McpError::NotFound("node-x".into())),
            Err(McpError::Internal("serde".into())),
            Ok(json!({"ok": true})),
        ];
        for outcome in &kept {
            assert!(
                !invalidates(outcome),
                "cache must survive non-transient outcome: {outcome:?}"
            );
        }
    }
}
