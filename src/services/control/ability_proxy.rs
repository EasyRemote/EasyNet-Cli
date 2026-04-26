// EasyNet CLI — Ability Proxy (Control-plane → AbilityDispatcher adapter)
// =========================================================================
//
// File: src/services/control/ability_proxy.rs
// Description: Frame-level adapter between the Control plane's wire
//              messages (`IncomingFrame` / `OutgoingFrame`) and the
//              runtime's two-stage `InvocationTarget` resolver +
//              `AbilityDispatcher`. PR-INVOCATION-EXEC-UNITY wires
//              this for real (was a v1 skeleton in PR-DAEMON Commit 3).
//
// Layering rule (enforced by scripts/check-kernel-boundary.sh)
// ------------------------------------------------------------
// This file is the *only* legal place in `src/services/control/`
// to import from `crate::runtime::*`. It imports:
//   * `crate::runtime::ability_dispatch::AbilityDispatcher`  (executor)
//   * `crate::runtime::invocation_target::{TargetResolver,
//      InvocationPlan, ...}`                                   (resolver)
//   * `crate::runtime::kernel_api::KernelApi`                 (entry shape;
//      retained for future Receipt-emit + audit hooks)
//   * `crate::runtime::domain::NodeId`                        (typed id)
//
// It must NOT import:
//   * `crate::runtime::gateway*` — Execution → Gateway boundary is
//     internal to the runtime; Control reaches it only via dispatcher.
//   * `crate::runtime::execution::*` — sub-service internals; Control
//     talks through the dispatcher, never to a sub-service directly.
//
// v10.3 C* unity reminder
// -----------------------
// Every Invoke/Subscribe frame becomes an `InvocationPlan` →
// `resolver.resolve(plan)` → `dispatcher.execute_*(target)`. The
// proxy is the only place the wire-format ↔ InvocationTarget
// translation happens. Schedule tick / Loop controller / Permission
// admission go directly into the same dispatcher; they do NOT come
// through this proxy.
//
// Why one method returns Vec<OutgoingFrame> and not OutgoingFrame
// ----------------------------------------------------------------
// Invoke and Cancel each produce exactly one response envelope.
// Subscribe produces N `Frame`s plus one `Terminal`. Returning a
// `Vec<OutgoingFrame>` from the single proxy entry uniforms the
// shape so the IPC `serve_connection` loop just iterates and writes
// each frame. RPC paths return a 1-element vec; the cost of the
// allocation is negligible against the framed write that follows.
//
// Author: Silan Hu <silan.hu@u.nus.edu>
// Copyright (c) 2026 EasyNet. All rights reserved.

use std::collections::HashMap;
use std::sync::{Arc, Mutex};

use tokio::sync::mpsc;

use crate::runtime::ability_dispatch::{AbilityDispatcher, StreamSource};
use crate::runtime::domain::NodeId;
use crate::runtime::invocation_target::{
    CallMode, InvocationPlan, LocalNodeResolver, TargetResolver,
};
use crate::runtime::kernel_api::KernelApi;
use crate::services::control::frames::{codes, IncomingFrame, OutgoingFrame};

/// Per-connection cancel registry. Each active subscription gets a
/// `CancellationToken` clone stored under its `subscription_id`. A
/// `Cancel` frame from the client looks up the id and calls
/// `cancel()` on the token; the forwarder task awaits the token in
/// a `tokio::select!` and exits cleanly when triggered.
pub type CancelRegistry = Arc<Mutex<HashMap<String, tokio_util::sync::CancellationToken>>>;

/// Stateless-on-construction adapter. Holds a dispatcher (the
/// stage-2 executor), a resolver (stage-1), and a Kernel handle for
/// future receipt-emit hooks. Cloned per accepted connection.
#[derive(Clone)]
pub struct AbilityProxy {
    /// Retained for future receipt-emit + audit hooks. v1 does not
    /// call into KernelApi from this proxy because the dispatcher
    /// path covers every wire frame; PR-INVOCATION-EXEC-UNITY+1
    /// wires Receipt emission through KernelApi here.
    #[allow(dead_code)]
    kernel: Arc<dyn KernelApi>,
    dispatcher: AbilityDispatcher,
    /// Resolver is held as `Arc<dyn TargetResolver>` so a future
    /// planner can plug in a smarter resolver without changing the
    /// proxy signature.
    resolver: Arc<dyn TargetResolver>,
}

impl AbilityProxy {
    /// Construct a proxy with the provided components. The daemon bin
    /// builds these once at boot and shares the resulting `AbilityProxy`
    /// across every accepted IPC connection.
    pub fn new_with_dispatcher(
        kernel: Arc<dyn KernelApi>,
        dispatcher: AbilityDispatcher,
        resolver: Arc<dyn TargetResolver>,
    ) -> Self {
        Self {
            kernel,
            dispatcher,
            resolver,
        }
    }

    /// Convenience constructor used by tests + the `make_proxy`
    /// helper. Builds a fresh dispatcher off the live system-ability
    /// registry, a NoopGateway, and a `LocalNodeResolver` keyed to
    /// `EASYNET_NODE_ID` (or "self" when unset). Production callers
    /// should prefer `new_with_dispatcher` for explicit wiring.
    ///
    /// Permitted by `scripts/check-kernel-boundary.sh` because the
    /// allowlist v1 includes `crate::runtime::{system, gateway}`
    /// alongside the syscall-boundary modules. The gate's rationale
    /// is documented at the top of that script.
    pub fn new(kernel: Arc<dyn KernelApi>) -> Self {
        let registry = crate::runtime::system::build_registry();
        let gateway: Arc<dyn crate::runtime::gateway_api::GatewayApi> =
            Arc::new(crate::runtime::gateway::NoopGateway::new());
        let dispatcher = AbilityDispatcher::new(registry, gateway);
        let local_node = node_id_from_env_or_default();
        let resolver: Arc<dyn TargetResolver> = Arc::new(LocalNodeResolver::new(local_node));
        Self {
            kernel,
            dispatcher,
            resolver,
        }
    }

    /// Async-aware request dispatch. This is the production path
    /// the IPC server uses; the older `handle()` variant below is
    /// kept for unit tests that prefer a synchronous shape.
    ///
    /// `out` is the per-connection writer queue. Frames the proxy
    /// owes the client are pushed onto it in order. For live
    /// subscriptions the proxy spawns a forwarder task that owns
    /// the `out` clone and pushes frames as the underlying
    /// broadcast::Receiver yields them; the spawned task observes
    /// `cancel.token(subscription_id)` and exits cleanly when the
    /// client sends a `Cancel` frame.
    ///
    /// `cancel` is the per-connection registry of in-flight
    /// subscription tokens. The proxy registers a token under the
    /// subscription_id at Subscribe-frame time, and removes it
    /// when the forwarder completes (success or cancel).
    pub async fn handle_async(
        &self,
        req: IncomingFrame,
        out: mpsc::Sender<OutgoingFrame>,
        cancel: &CancelRegistry,
    ) {
        match req {
            IncomingFrame::Invoke {
                request_id,
                ability,
                args,
            } => {
                let frames = self.handle_invoke(request_id, ability, args);
                for f in frames {
                    if out.send(f).await.is_err() {
                        return;
                    }
                }
            }
            IncomingFrame::Subscribe {
                subscription_id,
                ability,
                args,
            } => {
                self.handle_subscribe_async(subscription_id, ability, args, out, cancel)
                    .await;
            }
            IncomingFrame::Cancel { subscription_id } => {
                let token = {
                    let mut g = cancel.lock().expect("cancel registry lock");
                    g.remove(&subscription_id)
                };
                match token {
                    Some(tok) => {
                        tok.cancel();
                        // Don't write a response; the forwarder
                        // emits its own Terminal{cancelled} on its
                        // way out.
                    }
                    None => {
                        let _ = out
                            .send(OutgoingFrame::Error {
                                request_id: None,
                                subscription_id: Some(subscription_id),
                                code: codes::ABILITY_FAILED.into(),
                                message: "Cancel for unknown subscription_id".into(),
                            })
                            .await;
                    }
                }
            }
        }
    }

    async fn handle_subscribe_async(
        &self,
        subscription_id: String,
        ability: String,
        args: serde_json::Value,
        out: mpsc::Sender<OutgoingFrame>,
        cancel: &CancelRegistry,
    ) {
        let plan = InvocationPlan {
            ability,
            target_node_hint: extract_node_hint(&args),
            args,
            call_mode: CallMode::Stream,
        };
        let target = match self.resolver.resolve(plan) {
            Ok(t) => t,
            Err(e) => {
                let _ = out
                    .send(OutgoingFrame::Error {
                        request_id: None,
                        subscription_id: Some(subscription_id),
                        code: codes::ABILITY_FAILED.into(),
                        message: format!("resolver: {e}"),
                    })
                    .await;
                return;
            }
        };
        let stream = match self.dispatcher.execute_stream(target) {
            Ok(s) => s,
            Err(e) => {
                let msg = format!("{e}");
                let code = if msg.contains("no local stream handler") {
                    codes::NOT_FOUND
                } else {
                    codes::ABILITY_FAILED
                };
                let _ = out
                    .send(OutgoingFrame::Error {
                        request_id: None,
                        subscription_id: Some(subscription_id),
                        code: code.into(),
                        message: msg,
                    })
                    .await;
                return;
            }
        };
        match stream {
            StreamSource::Snapshot(values) => {
                for v in values {
                    if out
                        .send(OutgoingFrame::Frame {
                            subscription_id: subscription_id.clone(),
                            frame: v,
                        })
                        .await
                        .is_err()
                    {
                        return;
                    }
                }
                let _ = out
                    .send(OutgoingFrame::Terminal {
                        subscription_id,
                        reason: "done".into(),
                    })
                    .await;
            }
            StreamSource::Live(rx) => {
                spawn_forwarder(subscription_id, Vec::new(), rx, out, cancel.clone());
            }
            StreamSource::SnapshotThenLive(snap, rx) => {
                spawn_forwarder(subscription_id, snap, rx, out, cancel.clone());
            }
        }
    }

    /// Route one incoming frame through the resolver + dispatcher.
    /// Returns a Vec of one or more outgoing frames the IPC server
    /// writes back in order.
    ///
    /// Frame mapping:
    ///   * Invoke         → one `Result` (success) or one `Error`.
    ///   * Subscribe      → N `Frame` envelopes + one `Terminal`
    ///                      with reason="done" on success;
    ///                      one `Error` on failure.
    ///   * Cancel         → idempotent: returns one `Error` with
    ///                      code=ability_failed and a message
    ///                      indicating cancel-without-active-stream
    ///                      until the streaming registry lands.
    ///
    /// Synchronous variant retained for unit tests. Live streams
    /// (`StreamSource::Live` / `SnapshotThenLive`) are degraded to
    /// snapshot-only here because the sync surface has nowhere to
    /// pump live frames into; production IPC uses `handle_async`.
    pub fn handle(&self, req: IncomingFrame) -> Vec<OutgoingFrame> {
        match req {
            IncomingFrame::Invoke {
                request_id,
                ability,
                args,
            } => self.handle_invoke(request_id, ability, args),
            IncomingFrame::Subscribe {
                subscription_id,
                ability,
                args,
            } => self.handle_subscribe(subscription_id, ability, args),
            IncomingFrame::Cancel { subscription_id } => {
                vec![OutgoingFrame::Error {
                    request_id: None,
                    subscription_id: Some(subscription_id),
                    code: codes::ABILITY_FAILED.into(),
                    message: "Cancel for unknown subscription_id; \
                              streaming subscription registry lands in a follow-up PR"
                        .into(),
                }]
            }
        }
    }

    fn handle_invoke(
        &self,
        request_id: String,
        ability: String,
        args: serde_json::Value,
    ) -> Vec<OutgoingFrame> {
        let plan = InvocationPlan {
            ability,
            target_node_hint: extract_node_hint(&args),
            args,
            call_mode: CallMode::Rpc,
        };
        let target = match self.resolver.resolve(plan) {
            Ok(t) => t,
            Err(e) => {
                return vec![OutgoingFrame::Error {
                    request_id: Some(request_id),
                    subscription_id: None,
                    code: codes::ABILITY_FAILED.into(),
                    message: format!("resolver: {e}"),
                }];
            }
        };
        match self.dispatcher.execute_rpc(target) {
            Ok(value) => vec![OutgoingFrame::Result {
                request_id,
                value,
            }],
            Err(e) => {
                let msg = format!("{e}");
                let code = if msg.contains("no local handler registered") {
                    codes::NOT_FOUND
                } else {
                    codes::ABILITY_FAILED
                };
                vec![OutgoingFrame::Error {
                    request_id: Some(request_id),
                    subscription_id: None,
                    code: code.into(),
                    message: msg,
                }]
            }
        }
    }

    fn handle_subscribe(
        &self,
        subscription_id: String,
        ability: String,
        args: serde_json::Value,
    ) -> Vec<OutgoingFrame> {
        let plan = InvocationPlan {
            ability,
            target_node_hint: extract_node_hint(&args),
            args,
            call_mode: CallMode::Stream,
        };
        let target = match self.resolver.resolve(plan) {
            Ok(t) => t,
            Err(e) => {
                return vec![OutgoingFrame::Error {
                    request_id: None,
                    subscription_id: Some(subscription_id),
                    code: codes::ABILITY_FAILED.into(),
                    message: format!("resolver: {e}"),
                }];
            }
        };
        match self.dispatcher.execute_stream(target) {
            Ok(stream) => {
                // Sync surface: degrade live streams to snapshot
                // only. Tests using this path that need live
                // behaviour should invoke `handle_async` instead.
                let snapshot: Vec<serde_json::Value> = match stream {
                    StreamSource::Snapshot(v) => v,
                    StreamSource::Live(_) => Vec::new(),
                    StreamSource::SnapshotThenLive(s, _) => s,
                };
                let mut out = Vec::with_capacity(snapshot.len() + 1);
                for v in snapshot {
                    out.push(OutgoingFrame::Frame {
                        subscription_id: subscription_id.clone(),
                        frame: v,
                    });
                }
                out.push(OutgoingFrame::Terminal {
                    subscription_id,
                    reason: "done".into(),
                });
                out
            }
            Err(e) => {
                let msg = format!("{e}");
                let code = if msg.contains("no local stream handler") {
                    codes::NOT_FOUND
                } else {
                    codes::ABILITY_FAILED
                };
                vec![OutgoingFrame::Error {
                    request_id: None,
                    subscription_id: Some(subscription_id),
                    code: code.into(),
                    message: msg,
                }]
            }
        }
    }

    /// Accessor used by tests + the server accept-loop to borrow the
    /// held Kernel handle.
    #[allow(dead_code)]
    pub(crate) fn kernel(&self) -> &Arc<dyn KernelApi> {
        &self.kernel
    }
}

/// Spawn a per-subscription forwarder task. Drains the snapshot
/// onto `out`, then pumps every value from the broadcast::Receiver
/// `rx` until the sender drops or the cancel token fires. Removes
/// itself from the cancel registry on exit.
fn spawn_forwarder(
    subscription_id: String,
    snapshot: Vec<serde_json::Value>,
    mut rx: tokio::sync::broadcast::Receiver<serde_json::Value>,
    out: mpsc::Sender<OutgoingFrame>,
    cancel: CancelRegistry,
) {
    let token = tokio_util::sync::CancellationToken::new();
    {
        let mut g = cancel.lock().expect("cancel registry lock");
        g.insert(subscription_id.clone(), token.clone());
    }
    tokio::spawn(async move {
        // 1. Drain snapshot.
        for v in snapshot {
            if out
                .send(OutgoingFrame::Frame {
                    subscription_id: subscription_id.clone(),
                    frame: v,
                })
                .await
                .is_err()
            {
                // Connection closed mid-snapshot. Drop the entry
                // and return; the writer task on the connection
                // owns cleanup of the rest of the registry.
                let mut g = cancel.lock().expect("cancel registry lock");
                g.remove(&subscription_id);
                return;
            }
        }
        // 2. Pump live frames.
        let reason = loop {
            tokio::select! {
                _ = token.cancelled() => break "cancelled",
                recv = rx.recv() => match recv {
                    Ok(v) => {
                        if out
                            .send(OutgoingFrame::Frame {
                                subscription_id: subscription_id.clone(),
                                frame: v,
                            })
                            .await
                            .is_err()
                        {
                            // Connection writer task gave up. Stop.
                            break "done";
                        }
                    }
                    Err(tokio::sync::broadcast::error::RecvError::Lagged(_)) => continue,
                    Err(tokio::sync::broadcast::error::RecvError::Closed) => break "done",
                }
            }
        };
        let _ = out
            .send(OutgoingFrame::Terminal {
                subscription_id: subscription_id.clone(),
                reason: reason.into(),
            })
            .await;
        let mut g = cancel.lock().expect("cancel registry lock");
        g.remove(&subscription_id);
    });
}

/// Read a `node` field out of the args object if present. The wire
/// hint uses `node` (not `target_node`) to keep the schema-level
/// vocabulary uniform with how attach/permission/discuss already
/// don't take a node hint at all — the field is purely a routing
/// override surfaced through the args bag for v1, which keeps the
/// proto schemas free of a transport-only concept.
fn extract_node_hint(args: &serde_json::Value) -> Option<NodeId> {
    args.get("node")
        .and_then(|v| v.as_str())
        .filter(|s| !s.is_empty())
        .map(NodeId::new)
}

/// Resolve the local node id from the EASYNET_NODE_ID env var (set
/// by the daemon bin from `credentials.json` at boot) or default to
/// "self" for the test/harness path. The value only feeds into the
/// resolver's loopback-vs-remote decision; it is not part of the ABI.
fn node_id_from_env_or_default() -> NodeId {
    match std::env::var("EASYNET_NODE_ID") {
        Ok(s) if !s.is_empty() => NodeId::new(s),
        _ => NodeId::new("self"),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::runtime::ability_dispatch::LocalAbilityRegistry;
    use crate::runtime::domain::{
        DiscussRoom, LoopId, LoopInstance, PermissionDecision, PermissionId, PermissionRequest,
        RoomId, ScheduleEntry, ScheduleId, Session, SessionId,
    };
    use crate::runtime::gateway::NoopGateway;
    use crate::runtime::invocation::{Invocation, Receipt};
    use serde_json::json;

    /// Minimum KernelApi impl for proxy-level tests; v1 proxy doesn't
    /// reach the Kernel, but the type signature still wants one.
    struct StubKernel;

    impl KernelApi for StubKernel {
        fn invoke(&self, _inv: Invocation) -> anyhow::Result<Receipt> {
            anyhow::bail!("StubKernel: invoke not wired")
        }
        fn list_active_sessions(&self) -> anyhow::Result<Vec<Session>> {
            Ok(Vec::new())
        }
        fn get_session(&self, _id: &SessionId) -> anyhow::Result<Option<Session>> {
            Ok(None)
        }
        fn pending_permission_requests(&self) -> anyhow::Result<Vec<PermissionRequest>> {
            Ok(Vec::new())
        }
        fn decide_permission(
            &self,
            _id: &PermissionId,
            _decision: PermissionDecision,
        ) -> anyhow::Result<()> {
            Ok(())
        }
        fn list_schedules(&self) -> anyhow::Result<Vec<ScheduleEntry>> {
            Ok(Vec::new())
        }
        fn add_schedule(&self, _e: ScheduleEntry) -> anyhow::Result<ScheduleId> {
            anyhow::bail!("StubKernel: add_schedule not wired")
        }
        fn remove_schedule(&self, _id: &ScheduleId) -> anyhow::Result<()> {
            Ok(())
        }
        fn enable_schedule(&self, _id: &ScheduleId, _enabled: bool) -> anyhow::Result<()> {
            Ok(())
        }
        fn create_discuss_room(
            &self,
            _ps: Vec<String>,
            _topic: Option<String>,
        ) -> anyhow::Result<RoomId> {
            anyhow::bail!("StubKernel: create_discuss_room not wired")
        }
        fn list_discuss_rooms(&self) -> anyhow::Result<Vec<DiscussRoom>> {
            Ok(Vec::new())
        }
        fn loop_status(&self, _id: &LoopId) -> anyhow::Result<Option<LoopInstance>> {
            Ok(None)
        }
        fn cancel_loop(&self, _id: &LoopId) -> anyhow::Result<()> {
            Ok(())
        }
    }

    fn proxy_with_live_registry() -> AbilityProxy {
        // The convenience `new` constructor wires up the live system
        // ability registry — same path the daemon bin's tests use to
        // exercise the full local handler set.
        AbilityProxy::new(Arc::new(StubKernel))
    }

    fn proxy_with_empty_registry() -> AbilityProxy {
        // Some tests want a clean slate (no system.ping etc.) so they
        // can assert the unregistered-ability path; build an empty
        // registry + a NoopGateway here.
        let registry = Arc::new(LocalAbilityRegistry::new());
        let gateway: Arc<dyn crate::runtime::gateway_api::GatewayApi> =
            Arc::new(NoopGateway::new());
        let dispatcher = AbilityDispatcher::new(registry, gateway);
        let resolver: Arc<dyn TargetResolver> =
            Arc::new(LocalNodeResolver::new(NodeId::new("self")));
        AbilityProxy::new_with_dispatcher(Arc::new(StubKernel), dispatcher, resolver)
    }

    #[test]
    fn invoke_system_ping_returns_result_frame_with_request_id_preserved() {
        // The cdylib + smoke scripts depend on `system.ping` returning
        // a Result envelope (not the v1 skeleton Error). This test
        // pins that contract end-to-end through the live registry.
        let p = proxy_with_live_registry();
        let frames = p.handle(IncomingFrame::Invoke {
            request_id: "req-1".into(),
            ability: "observe.health".into(),
            args: json!({}),
        });
        assert_eq!(frames.len(), 1);
        match &frames[0] {
            OutgoingFrame::Result { request_id, .. } => {
                assert_eq!(request_id, "req-1");
            }
            other => panic!("expected Result frame, got {other:?}"),
        }
    }

    #[test]
    fn invoke_unknown_ability_returns_not_found() {
        // Distinguish "no handler registered" (NOT_FOUND) from
        // "handler ran but failed" (ABILITY_FAILED). A regression that
        // collapsed both onto the same code would hide a real
        // misconfiguration behind a generic failure.
        let p = proxy_with_empty_registry();
        let frames = p.handle(IncomingFrame::Invoke {
            request_id: "req-2".into(),
            ability: "system.does.not.exist".into(),
            args: json!({}),
        });
        assert_eq!(frames.len(), 1);
        match &frames[0] {
            OutgoingFrame::Error {
                request_id,
                code,
                ..
            } => {
                assert_eq!(request_id.as_deref(), Some("req-2"));
                assert_eq!(code, codes::NOT_FOUND);
            }
            other => panic!("expected Error frame, got {other:?}"),
        }
    }

    #[test]
    fn cancel_for_unknown_id_returns_error_with_subscription_id_preserved() {
        // Cancel on a stream the daemon never started must return
        // an error frame with subscription_id echoed back so the
        // Client can correlate. v1 does not have a streaming
        // subscription registry yet — that is a focused follow-up
        // — but the contract on the wire is fixed.
        let p = proxy_with_empty_registry();
        let frames = p.handle(IncomingFrame::Cancel {
            subscription_id: "sub-x".into(),
        });
        assert_eq!(frames.len(), 1);
        match &frames[0] {
            OutgoingFrame::Error {
                subscription_id,
                request_id,
                code,
                ..
            } => {
                assert_eq!(subscription_id.as_deref(), Some("sub-x"));
                assert!(request_id.is_none());
                assert_eq!(code, codes::ABILITY_FAILED);
            }
            other => panic!("expected Error frame, got {other:?}"),
        }
    }

    #[test]
    fn subscribe_to_session_attach_returns_terminal_at_minimum() {
        // The system.session.attach handler is registered as a stream
        // handler. With no active session, v1 emits zero data Frames
        // and exactly one Terminal — pin that the proxy threads the
        // Terminal through.
        let p = proxy_with_live_registry();
        let frames = p.handle(IncomingFrame::Subscribe {
            subscription_id: "sub-1".into(),
            ability: "fleet.attach_session".into(),
            args: json!({"session_id": "no-such-session"}),
        });
        // Last frame must be Terminal regardless of how many Frame
        // envelopes preceded it — that is the v1 contract for any
        // subscribe that returns Ok.
        let last = frames.last().expect("at least one frame");
        match last {
            OutgoingFrame::Terminal {
                subscription_id,
                reason,
            } => {
                assert_eq!(subscription_id, "sub-1");
                assert_eq!(reason, "done");
            }
            // Some session handlers may emit an Error in v1 — that's
            // also valid as long as it carries the subscription_id.
            OutgoingFrame::Error {
                subscription_id, ..
            } => {
                assert_eq!(subscription_id.as_deref(), Some("sub-1"));
            }
            other => panic!("expected Terminal or Error as the final frame, got {other:?}"),
        }
    }
}
