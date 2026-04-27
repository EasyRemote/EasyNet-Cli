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
// (`observe.health`, future `fleet.attach_session`, etc.). The
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
use tokio::sync::broadcast;

use crate::runtime::gateway_api::{GatewayApi, RemoteTarget};
use crate::runtime::invocation_target::{CallMode, InvocationTarget, TargetScope};

/// One in-process RPC handler. Boxed closure so the registry can
/// hold heterogeneous handlers behind a uniform key.
pub type LocalRpcHandler = Arc<dyn Fn(Value) -> anyhow::Result<Value> + Send + Sync>;

/// What a stream-mode ability handler may return.
///
/// Three shapes:
///
///   * `Snapshot(frames)` — finite, eagerly-materialised list. The
///     IPC server emits each frame in order then sends a `Terminal`
///     frame with reason `done`. Used for "give me what's on disk"
///     queries (replay-only).
///
///   * `Live(broadcast::Receiver<Value>)` — long-lived live tail.
///     The IPC server spawns a forwarder task that reads from the
///     receiver and emits each value as a `Frame`. Forwarder
///     terminates with reason `done` when the sender drops,
///     `error` on lag, or `cancelled` if the Client cancels.
///
///   * `SnapshotThenLive(snapshot, rx)` — snapshot first, live tail
///     after. The "replay then subscribe" composition every Paseo-
///     style UI wants: a Permission dialog joining mid-flight needs
///     to see currently-pending requests AND new ones; a Discuss
///     room view shows past turns AND new posts.
///
/// The `From` impls let handlers return either a `Vec<Value>` or a
/// `broadcast::Receiver<Value>` directly via `.into()`.
#[derive(Debug)]
pub enum StreamSource {
    Snapshot(Vec<Value>),
    Live(broadcast::Receiver<Value>),
    SnapshotThenLive(Vec<Value>, broadcast::Receiver<Value>),
}

impl From<Vec<Value>> for StreamSource {
    fn from(frames: Vec<Value>) -> Self {
        StreamSource::Snapshot(frames)
    }
}

impl From<broadcast::Receiver<Value>> for StreamSource {
    fn from(rx: broadcast::Receiver<Value>) -> Self {
        StreamSource::Live(rx)
    }
}

impl From<(Vec<Value>, broadcast::Receiver<Value>)> for StreamSource {
    fn from((snap, rx): (Vec<Value>, broadcast::Receiver<Value>)) -> Self {
        StreamSource::SnapshotThenLive(snap, rx)
    }
}

impl StreamSource {
    /// Take just the snapshot portion. Returns the `Snapshot`
    /// vec verbatim, the snapshot half of `SnapshotThenLive`, and
    /// an empty Vec for a pure `Live` source. Used by unit tests
    /// that only assert on the replayable history portion of a
    /// stream — the live tail is exercised separately.
    pub fn into_snapshot(self) -> Vec<Value> {
        match self {
            StreamSource::Snapshot(v) => v,
            StreamSource::Live(_) => Vec::new(),
            StreamSource::SnapshotThenLive(s, _) => s,
        }
    }
}

/// One in-process stream handler. Returns either an eager snapshot
/// or a live broadcast::Receiver — see `StreamSource` for the
/// contract.
pub type LocalStreamHandler =
    Arc<dyn Fn(Value) -> anyhow::Result<StreamSource> + Send + Sync>;

/// Local-ability registry. Keyed by full ability name. v1 shape is
/// a `BTreeMap` for deterministic iteration order; the registry
/// is read-mostly (built once at daemon start, queried per
/// invocation), so RwLock + per-invocation hash is overkill.
#[derive(Default)]
pub struct LocalAbilityRegistry {
    rpc: BTreeMap<String, LocalRpcHandler>,
    stream: BTreeMap<String, LocalStreamHandler>,
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

    /// Register a stream handler under `ability`. Same single-
    /// writer model as `register_rpc`.
    pub fn register_stream(&mut self, ability: impl Into<String>, handler: LocalStreamHandler) {
        self.stream.insert(ability.into(), handler);
    }

    /// Lookup helper — exposed because PR-ATTACH onwards will need
    /// a way to introspect "what abilities does this daemon
    /// publish?" without reflecting through the dispatcher.
    ///
    /// Returns the union of RPC + stream ability names, sorted.
    /// Discovery callers should not see the call-mode distinction.
    pub fn list_abilities(&self) -> Vec<String> {
        let mut names: Vec<String> = self.rpc.keys().cloned().collect();
        for k in self.stream.keys() {
            if !names.iter().any(|n| n == k) {
                names.push(k.clone());
            }
        }
        names.sort();
        names
    }

    /// Returns Some when an RPC handler is registered for `ability`.
    pub fn get_rpc(&self, ability: &str) -> Option<&LocalRpcHandler> {
        self.rpc.get(ability)
    }

    /// Returns Some when a stream handler is registered for `ability`.
    pub fn get_stream(&self, ability: &str) -> Option<&LocalStreamHandler> {
        self.stream.get(ability)
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

    /// Borrow the unified local-ability registry. Used by `Kernel`
    /// to look up handlers without going through `execute_rpc`'s
    /// `InvocationTarget` envelope — Kernel admission has already
    /// resolved scope to local by this point. Exposed as a borrow
    /// (rather than a clone) so the caller chooses whether to
    /// retain a handle.
    pub fn local_registry(&self) -> &Arc<LocalAbilityRegistry> {
        &self.local
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

    /// Execute a Stream-mode `InvocationTarget`. Returns a
    /// `StreamSource` — either an eager snapshot (Vec) or a live
    /// broadcast::Receiver. The caller (IPC server) decides how to
    /// fan it out into wire frames.
    ///
    /// Remote streams are not yet supported in v1 —
    /// `subscribe_remote_ability` on the gateway is callback-shaped
    /// and would need a separate plumbing pass to forward through
    /// the IPC connection.
    pub fn execute_stream(&self, target: InvocationTarget) -> anyhow::Result<StreamSource> {
        if target.call_mode != CallMode::Stream {
            anyhow::bail!(
                "AbilityDispatcher::execute_stream called with non-Stream call_mode \
                 (got {:?}); use execute_rpc instead",
                target.call_mode
            );
        }
        match target.scope {
            TargetScope::Local => match self.local.get_stream(&target.ability) {
                Some(handler) => handler(target.normalized_args),
                None => anyhow::bail!(
                    "no local stream handler registered for ability {} (loopback path)",
                    target.ability
                ),
            },
            TargetScope::Remote { .. } => anyhow::bail!(
                "remote stream dispatch not yet wired in v1; \
                 lands once GatewayApi::subscribe_remote_ability is plumbed \
                 to forward into the IPC stream"
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
            ability: "observe.health".into(),
            normalized_args: json!({}),
            call_mode: CallMode::Rpc,
        }
    }

    #[test]
    fn unregistered_local_ability_returns_clear_error() {
        // The error must name the ability so an operator can grep
        // "is observe.health registered?" against the daemon log.
        let dispatcher =
            AbilityDispatcher::new(empty_registry(), Arc::new(NoopGateway::new()));
        let err = dispatcher
            .execute_rpc(ping_target_local())
            .unwrap_err();
        let msg = format!("{err}");
        assert!(msg.contains("observe.health"), "error must name ability, got: {msg}");
        assert!(msg.contains("local"), "error must indicate loopback path");
    }

    #[test]
    fn registered_local_ability_runs_handler() {
        // Smoke: the dispatcher actually calls the registered
        // handler with the normalised args; the handler's return
        // value is surfaced verbatim.
        let mut reg = LocalAbilityRegistry::new();
        reg.register_rpc(
            "observe.health",
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
            ability: "observe.health".into(),
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
    fn bidi_call_mode_rejected_at_rpc_path() {
        // Symmetric to `stream_call_mode_rejected_at_rpc_path`. The
        // bidi executor (lands in C-M3a commit 2) is the right
        // surface for CallMode::Bidi; routing a bidi target into the
        // RPC executor would silently swallow the session contract.
        // Pin the rejection so a future refactor can't relax this
        // check to `== Stream`.
        let dispatcher =
            AbilityDispatcher::new(empty_registry(), Arc::new(NoopGateway::new()));
        let mut t = ping_target_local();
        t.call_mode = CallMode::Bidi;
        let err = dispatcher.execute_rpc(t).unwrap_err();
        assert!(format!("{err}").contains("Rpc"));
    }

    #[test]
    fn bidi_call_mode_rejected_at_stream_path() {
        // The stream executor accepts only CallMode::Stream. A bidi
        // target arriving here means a wiring bug upstream; pin the
        // bail so the misroute surfaces immediately rather than
        // silently returning an empty StreamSource.
        let dispatcher =
            AbilityDispatcher::new(empty_registry(), Arc::new(NoopGateway::new()));
        let mut t = ping_target_local();
        t.call_mode = CallMode::Bidi;
        let err = dispatcher.execute_stream(t).unwrap_err();
        assert!(format!("{err}").contains("Stream"));
    }

    #[test]
    fn list_abilities_returns_registered_keys_in_order() {
        // Deterministic iteration order matters because PR-SYS
        // builds the `system_skills[]` label from this list, and
        // the byte-stable golden fixture depends on it.
        let mut reg = LocalAbilityRegistry::new();
        reg.register_rpc("observe.health", Arc::new(|_| Ok(Value::Null)));
        reg.register_rpc("test.foo", Arc::new(|_| Ok(Value::Null)));
        reg.register_rpc("test.bar", Arc::new(|_| Ok(Value::Null)));
        let names = reg.list_abilities();
        // BTreeMap iteration order is alphabetical (test.bar < test.foo,
        // observe.health < test.*).
        assert_eq!(names, vec!["observe.health", "test.bar", "test.foo"]);
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
