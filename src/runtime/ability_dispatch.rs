// EasyNet CLI — Ability Dispatch Executor (stage 2)
// ==================================================
//
// File: src/runtime/ability_dispatch.rs
// Description: Stage 2 of two-stage dispatch (plan v10.1). Consumes
//              an `InvocationTarget` from the stage-1 resolver and
//              executes it — locally via the in-process system-
//              ability handler registry, or remotely via the
//              GatewayApi.
//
// Why this is a separate file from the resolver
// ---------------------------------------------
// Resolution is "where does this go" (a policy decision: future
// planner, capability router, locality preference all hang off
// stage 1). Execution is "send the bytes" (a transport concern:
// loopback handler invocation vs. GatewayApi forwarding). Mixing
// the two means every routing-policy change has to walk through
// transport code and vice versa.
//
// CI rule reinforcing this split: handlers under
// `src/runtime/system/*` may NOT branch on `target_node` /
// `self.node_id` (`scripts/check-dispatch-boundary.sh`). They get
// a resolved `InvocationTarget` and act on it.
//
// v1 scope
// --------
// `LocalAbility` registry is keyed by full ability name
// (`system.ping`, future `system.session.attach`, etc.). The
// remote path delegates to `GatewayApi::invoke_remote_ability`
// which already exists. Streaming abilities (`subscribe`-mode
// invocations) follow in PR-ATTACH/PR-PERM/PR-DISCUSS/PR-LOOP;
// this executor's stream surface is a pass-through stub here so
// PR-SYS does not block them.
//
// Author: Silan Hu <silan.hu@u.nus.edu>
// Copyright (c) 2026 EasyNet. All rights reserved.

use std::collections::BTreeMap;
use std::sync::Arc;

use serde_json::Value;

use crate::runtime::gateway_api::{GatewayApi, RemoteTarget};
use crate::runtime::invocation_target::{CallMode, InvocationTarget, TargetScope};

/// One in-process ability handler. Boxed closure so the registry
/// can hold heterogeneous handlers behind a uniform key.
pub type LocalRpcHandler = Arc<dyn Fn(Value) -> anyhow::Result<Value> + Send + Sync>;

/// Local-ability registry. Keyed by full ability name. v1 shape is
/// a `BTreeMap` for deterministic iteration order; the registry
/// is read-mostly (built once at daemon start, queried per
/// invocation), so RwLock + per-invocation hash is overkill.
#[derive(Default)]
pub struct LocalAbilityRegistry {
    rpc: BTreeMap<String, LocalRpcHandler>,
}

impl LocalAbilityRegistry {
    pub fn new() -> Self {
        Self::default()
    }

    /// Register an RPC handler under `ability`. Replaces any prior
    /// handler at the same key — the daemon owns this registry and
    /// is the only writer, so accidental duplicate registration
    /// would be a bug at startup, not a race.
    pub fn register_rpc(&mut self, ability: impl Into<String>, handler: LocalRpcHandler) {
        self.rpc.insert(ability.into(), handler);
    }

    /// Lookup helper — exposed because PR-ATTACH onwards will need
    /// a way to introspect "what abilities does this daemon
    /// publish?" without reflecting through the dispatcher.
    pub fn list_abilities(&self) -> Vec<String> {
        self.rpc.keys().cloned().collect()
    }

    /// Returns Some when an RPC handler is registered for `ability`.
    pub fn get_rpc(&self, ability: &str) -> Option<&LocalRpcHandler> {
        self.rpc.get(ability)
    }
}

/// Stage-2 executor. Holds a registry of local ability handlers
/// and an Arc<dyn GatewayApi> for the remote path. Construction
/// is cheap (Arc clones); the real cost is registry build at
/// daemon start.
#[derive(Clone)]
pub struct AbilityDispatcher {
    local: Arc<LocalAbilityRegistry>,
    gateway: Arc<dyn GatewayApi>,
}

impl AbilityDispatcher {
    pub fn new(local: Arc<LocalAbilityRegistry>, gateway: Arc<dyn GatewayApi>) -> Self {
        Self { local, gateway }
    }

    /// Execute an RPC-mode `InvocationTarget`. Returns the response
    /// value (for local) or the gateway's response (for remote).
    pub fn execute_rpc(&self, target: InvocationTarget) -> anyhow::Result<Value> {
        if target.call_mode != CallMode::Rpc {
            anyhow::bail!(
                "AbilityDispatcher::execute_rpc called with non-Rpc call_mode \
                 (got {:?}); use a streaming method instead",
                target.call_mode
            );
        }
        match target.scope {
            TargetScope::Local => match self.local.get_rpc(&target.ability) {
                Some(handler) => handler(target.normalized_args),
                None => anyhow::bail!(
                    "no local handler registered for ability {} (loopback path)",
                    target.ability
                ),
            },
            TargetScope::Remote { node } => self.gateway.invoke_remote_ability(
                &RemoteTarget {
                    node,
                    ability: target.ability,
                },
                &target.normalized_args,
            ),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::runtime::domain::NodeId;
    use crate::runtime::gateway::NoopGateway;
    use crate::runtime::gateway_api::PeerInfo;
    use serde_json::json;

    fn empty_registry() -> Arc<LocalAbilityRegistry> {
        Arc::new(LocalAbilityRegistry::new())
    }

    fn ping_target_local() -> InvocationTarget {
        InvocationTarget {
            scope: TargetScope::Local,
            ability: "system.ping".into(),
            normalized_args: json!({}),
            call_mode: CallMode::Rpc,
        }
    }

    #[test]
    fn unregistered_local_ability_returns_clear_error() {
        // The error must name the ability so an operator can grep
        // "is system.ping registered?" against the daemon log.
        let dispatcher =
            AbilityDispatcher::new(empty_registry(), Arc::new(NoopGateway::new()));
        let err = dispatcher
            .execute_rpc(ping_target_local())
            .unwrap_err();
        let msg = format!("{err}");
        assert!(msg.contains("system.ping"), "error must name ability, got: {msg}");
        assert!(msg.contains("local"), "error must indicate loopback path");
    }

    #[test]
    fn registered_local_ability_runs_handler() {
        // Smoke: the dispatcher actually calls the registered
        // handler with the normalised args; the handler's return
        // value is surfaced verbatim.
        let mut reg = LocalAbilityRegistry::new();
        reg.register_rpc(
            "system.ping",
            Arc::new(|args: Value| Ok(json!({"echo": args}))),
        );
        let dispatcher = AbilityDispatcher::new(Arc::new(reg), Arc::new(NoopGateway::new()));
        let mut t = ping_target_local();
        t.normalized_args = json!({"k": "v"});
        let resp = dispatcher.execute_rpc(t).unwrap();
        assert_eq!(resp, json!({"echo": {"k": "v"}}));
    }

    #[test]
    fn remote_target_routes_through_gateway() {
        // The remote path goes through GatewayApi. NoopGateway
        // returns a clear "not connected" error; we just need to
        // see that the dispatcher reached for it instead of
        // looking up the local registry.
        let dispatcher =
            AbilityDispatcher::new(empty_registry(), Arc::new(NoopGateway::new()));
        let target = InvocationTarget {
            scope: TargetScope::Remote {
                node: NodeId::new("peer"),
            },
            ability: "system.ping".into(),
            normalized_args: json!({}),
            call_mode: CallMode::Rpc,
        };
        let err = dispatcher.execute_rpc(target).unwrap_err();
        let msg = format!("{err}");
        assert!(
            msg.contains("NoopGateway"),
            "error must come from the gateway, got: {msg}"
        );
    }

    #[test]
    fn stream_call_mode_rejected_at_rpc_path() {
        // A handler asking the RPC executor to dispatch a stream
        // mode is calling the wrong method. Returning a clear
        // error catches the misuse at the call site instead of
        // silently degrading to an RPC return.
        let dispatcher =
            AbilityDispatcher::new(empty_registry(), Arc::new(NoopGateway::new()));
        let mut t = ping_target_local();
        t.call_mode = CallMode::Stream;
        let err = dispatcher.execute_rpc(t).unwrap_err();
        assert!(format!("{err}").contains("Rpc"));
    }

    #[test]
    fn list_abilities_returns_registered_keys_in_order() {
        // Deterministic iteration order matters because PR-SYS
        // builds the `system_skills[]` label from this list, and
        // the byte-stable golden fixture depends on it.
        let mut reg = LocalAbilityRegistry::new();
        reg.register_rpc("system.ping", Arc::new(|_| Ok(Value::Null)));
        reg.register_rpc("system.foo", Arc::new(|_| Ok(Value::Null)));
        reg.register_rpc("system.bar", Arc::new(|_| Ok(Value::Null)));
        let names = reg.list_abilities();
        assert_eq!(names, vec!["system.bar", "system.foo", "system.ping"]);
    }

    // Smoke for PeerInfo type — keeps the import "live" in tests
    // that touch GatewayApi-adjacent types.
    #[allow(dead_code)]
    fn _peer_info_is_constructible() -> PeerInfo {
        PeerInfo {
            node: NodeId::new("x"),
            labels: BTreeMap::new(),
        }
    }
}
