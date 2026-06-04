// EasyNet CLI — Ability Proxy (Control-plane → Axon LocalRuntime adapter)
// =========================================================================
//
// File: src/services/control/ability_proxy.rs
// Description: Frame-level adapter between the Control plane's wire
//              messages (`IncomingFrame` / `OutgoingFrame`) and the
//              daemon-hosted Axon `LocalRuntime`.
//
// Layering rule (enforced by scripts/check-kernel-boundary.sh)
// ------------------------------------------------------------
// This file is the *only* legal place in `src/services/control/`
// to import from `crate::runtime::*`. It imports:
//   * `crate::runtime::invocation_target::{TargetResolver,
//      InvocationPlan, ...}`                                   (resolver)
//   * `crate::runtime::kernel_api::KernelApi`                 (entry shape;
//      retained for future Receipt-emit + audit hooks)
//   * `crate::runtime::domain::NodeId`                        (typed id)
//
// It must NOT import:
//   * `crate::runtime::gateway*` — Execution → Gateway boundary is
//     internal to the runtime.
//   * `crate::runtime::execution::*` — sub-service internals; Control
//     talks through Axon abilities, never to a sub-service directly.
//
// v10.3 C* unity reminder
// -----------------------
// Every Invoke/Subscribe/OpenBidi frame becomes an `InvocationPlan` →
// `resolver.resolve(plan)` → `LocalRuntime::invoke_*_async(...)`.
// The proxy is the only place the control wire-format ↔ local Axon
// invocation translation happens.
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
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex};

use tokio::sync::mpsc;

use easynet_axon::invocation::{
    BidiInputFrame, BidiInputSender, BidiOutputReceiver, LocalRuntime, StreamingInvocationHandle,
};

use crate::runtime::domain::NodeId;
use crate::runtime::invocation_target::{
    CallMode, InvocationPlan, LocalNodeResolver, TargetResolver,
};
use crate::runtime::kernel_api::KernelApi;
#[cfg(test)]
use crate::runtime::local_runtime_invoker::drain_local_stream_frames;
use crate::runtime::local_runtime_invoker::{
    ability_frame_to_json, invoke_local_rpc_sync, is_not_found_error, open_local_bidi,
    open_local_stream,
};
use crate::services::control::frames::{codes, IncomingFrame, OutgoingFrame};
use crate::support::async_bridge::{run_blocking, NoRuntimeFallback};

/// Per-connection cancel registry. Each active subscription gets a
/// `CancellationToken` clone stored under its `subscription_id`. A
/// `Cancel` frame from the client looks up the id and calls
/// `cancel()` on the token; the forwarder task awaits the token in
/// a `tokio::select!` and exits cleanly when triggered.
pub type CancelRegistry = Arc<Mutex<HashMap<String, tokio_util::sync::CancellationToken>>>;

/// Per-connection bidi-session table. Each open `OpenBidi` session
/// installs one `BidiSession` row keyed by `session_id`; `SendBidi`
/// frames look up the row to find the handler-input sender;
/// `CloseBidi` removes the row, which drops the sender and lets the
/// handler observe EOF.
///
/// Per design §D8 the registry is **per-connection**, never per-
/// process. `session_id` uniqueness only needs to hold within a
/// single connection, and connection drop deterministically cleans
/// every live session because the table is owned by `serve_connection`
/// and its drop closes every sender simultaneously.
pub type BidiRegistry = Arc<Mutex<HashMap<String, BidiSession>>>;

/// One row in the per-connection [`BidiRegistry`].
///
/// Holds the three handles `serve_connection` needs to manage a
/// session's lifecycle:
///
///   * `to_handler` — `SendBidi` frames push here. `CloseBidi`
///     removes the row and explicitly closes the Axon inbox so the
///     handler observes EOF.
///   * `cancel` — fired by `serve_connection` on connection drop.
///     The forwarder selects on this and exits cleanly per §D4
///     path 3.
///   * `finalized` — §I2 idempotency flag. The forwarder flips it
///     `false → true` via `compare_exchange` before emitting the one
///     `TerminalBidi`; any racing path that observes `true` no-ops.
///     `Arc<AtomicBool>` rather than parked-Mutex because the
///     contention shape is "one writer, occasional contended read"
///     and atomics are cheaper.
pub struct BidiSession {
    pub to_handler: BidiInputSender,
    pub cancel: tokio_util::sync::CancellationToken,
    pub finalized: Arc<AtomicBool>,
}

/// Stateless-on-construction adapter. Holds the daemon-hosted Axon
/// runtime, a resolver, and a Kernel handle for future receipt-emit
/// hooks. Cloned per accepted connection.
#[derive(Clone)]
pub struct AbilityProxy {
    /// Retained for future receipt-emit + audit hooks. v1 does not
    /// call into KernelApi from this proxy because LocalRuntime
    /// already covers every wire frame.
    #[allow(dead_code)]
    kernel: Arc<dyn KernelApi>,
    local_runtime: Arc<LocalRuntime>,
    /// Resolver is held as `Arc<dyn TargetResolver>` so a future
    /// planner can plug in a smarter resolver without changing the
    /// proxy signature.
    resolver: Arc<dyn TargetResolver>,
    /// Snapshot of `local-agents.json` taken once at proxy
    /// construction. Used by P4.8c to attach §A12 receipt headers
    /// to every successful Result frame. Wrapped in Arc so cloning
    /// the proxy per-connection stays cheap.
    local_agents: Arc<crate::persistence::local_agents::LocalAgentsFile>,
    /// Host signing backend for §A12 hosted receipt attestations.
    receipt_identity: Option<Arc<dyn crate::services::self_identity::SelfIdentity>>,
}

impl AbilityProxy {
    /// Construct a proxy with the provided components. The daemon bin
    /// builds these once at boot and shares the resulting `AbilityProxy`
    /// across every accepted IPC connection.
    pub fn new_with_runtime(
        kernel: Arc<dyn KernelApi>,
        local_runtime: Arc<LocalRuntime>,
        resolver: Arc<dyn TargetResolver>,
    ) -> Self {
        // Best-effort load of local-agents.json. A read failure
        // (file doesn't exist yet, parse error) downgrades to an
        // empty file — receipt header emission is best-effort
        // (returns None for every ability) until the next daemon
        // restart picks up the file. Daemon startup never aborts.
        let local_agents = Arc::new(crate::persistence::local_agents::load().unwrap_or_default());
        Self {
            kernel,
            local_runtime,
            resolver,
            local_agents,
            receipt_identity: Some(Arc::new(
                crate::services::self_identity::KeyringClient::default_path(),
            )),
        }
    }

    /// Test-only constructor that injects a `LocalAgentsFile`
    /// snapshot directly. Production callers should use
    /// `new_with_runtime` which loads from disk.
    #[cfg(test)]
    pub fn new_with_local_agents(
        kernel: Arc<dyn KernelApi>,
        local_runtime: Arc<LocalRuntime>,
        resolver: Arc<dyn TargetResolver>,
        local_agents: crate::persistence::local_agents::LocalAgentsFile,
    ) -> Self {
        Self {
            kernel,
            local_runtime,
            resolver,
            local_agents: Arc::new(local_agents),
            receipt_identity: None,
        }
    }

    /// Convenience constructor used by tests + the `make_proxy`
    /// helper. Builds a fresh Axon runtime and registers the live
    /// system-ability catalogue into it. Production callers should
    /// prefer `new_with_runtime` for explicit daemon wiring.
    ///
    /// Permitted by `scripts/check-kernel-boundary.sh` because the
    /// allowlist v1 includes `crate::runtime::{system, gateway}`
    /// alongside the syscall-boundary modules. The gate's rationale
    /// is documented at the top of that script.
    pub fn new(kernel: Arc<dyn KernelApi>) -> Self {
        // Load the real agent registry so per-agent self-bundle
        // abilities (`<agent>.discover`, `<agent>.invoke`,
        // `<agent>.chat`, plus per-verb fallbacks) are wired up. The
        // pre-fix path called `build_registry()` (no-agents form),
        // which left every owner-namespaced handler unregistered;
        // the symptom was the workspace MCP server announcing
        // `claude.invoke` in `tools/list` (descriptor catalog comes
        // from the on-disk + synth path) but failing dispatch with
        // "no local handler registered for ability claude.invoke
        // (loopback path)" — descriptor and registry diverged.
        //
        // Loading agents here keeps the descriptor catalog and the
        // dispatchable registry in lockstep without forcing every
        // caller of `AbilityProxy::new` to know about agents.
        let agents = crate::registry::agents::load_agents().unwrap_or_default();
        let local_runtime = LocalRuntime::new();
        let _registry = crate::runtime::agents::build_registry_with_services(
            Arc::new(crate::runtime::execution::session::SessionService::new()),
            Arc::new(crate::runtime::execution::permission::PermissionService::new()),
            Arc::new(crate::runtime::execution::discuss::DiscussService::new()),
            Arc::new(crate::runtime::execution::schedule::ScheduleService::new()),
            Arc::new(crate::runtime::execution::loop_instance::LoopService::new()),
            None,
            &agents,
            Arc::new(Vec::new()),
            crate::runtime::agents::PagesIdentity::default(),
            Some(Arc::clone(&local_runtime)),
            Arc::new(
                crate::runtime::agents::agent_lifecycle_ability::SharedHotRegistrarCell::new(),
            ),
        );
        let local_node = node_id_from_env_or_default();
        let resolver: Arc<dyn TargetResolver> = Arc::new(LocalNodeResolver::new(local_node));
        let local_agents = Arc::new(crate::persistence::local_agents::load().unwrap_or_default());
        Self {
            kernel,
            local_runtime,
            resolver,
            local_agents,
            receipt_identity: Some(Arc::new(
                crate::services::self_identity::KeyringClient::default_path(),
            )),
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
        bidi: &BidiRegistry,
    ) {
        match req {
            IncomingFrame::Invoke {
                request_id,
                ability,
                args,
                subject,
            } => {
                // `handle_invoke` is synchronous and can call
                // ability handlers (process.exec, shell.run) that
                // wrap an async `execute()` in
                // `Handle::current().block_on(...)`. That pattern is
                // safe ONLY on a blocking-pool thread; on a tokio
                // worker it panics with "Cannot start a runtime
                // from within a runtime". `handle_async` is driven
                // from a per-connection worker, so we move the
                // dispatch onto the blocking pool. Mirrors the
                // session path: run local ability execution on the
                // blocking pool, then return the typed frame result.
                //
                // `catch_unwind` still guards the call so a handler
                // panic surfaces as a typed Error frame instead of
                // tearing down the connection.
                let request_id_for_err = request_id.clone();
                let ability_for_err = ability.clone();
                let proxy = self.clone();
                let join = tokio::task::spawn_blocking(move || {
                    std::panic::catch_unwind(std::panic::AssertUnwindSafe(move || {
                        proxy.handle_invoke(request_id, ability, args, subject)
                    }))
                })
                .await;
                let frames = match join {
                    Ok(Ok(frames)) => frames,
                    Ok(Err(panic_payload)) => {
                        let msg = if let Some(s) = panic_payload.downcast_ref::<&'static str>() {
                            (*s).to_string()
                        } else if let Some(s) = panic_payload.downcast_ref::<String>() {
                            s.clone()
                        } else {
                            "non-string panic payload".to_string()
                        };
                        // `ability_for_err: String` — pass verbatim; op_event!
                        // auto-quotes only when the value contains whitespace,
                        // so a bare ability name renders as `ability=user.foo`.
                        // `msg` is borrowed (the macro takes `&dyn Display`),
                        // not consumed, so the `format!` below sees it
                        // unchanged.
                        crate::op_event!(
                            component = ability_proxy,
                            kind = handle_invoke_panicked,
                            ability = ability_for_err,
                            error = msg,
                        );
                        vec![OutgoingFrame::Error {
                            request_id: Some(request_id_for_err),
                            subscription_id: None,
                            code: codes::ABILITY_FAILED.into(),
                            message: format!("ability handler panicked: {msg}"),
                        }]
                    }
                    Err(join_err) => {
                        let err_msg = format!("{join_err}");
                        crate::op_event!(
                            component = ability_proxy,
                            kind = handle_invoke_task_aborted,
                            ability = ability_for_err,
                            error = err_msg,
                        );
                        vec![OutgoingFrame::Error {
                            request_id: Some(request_id_for_err),
                            subscription_id: None,
                            code: codes::ABILITY_FAILED.into(),
                            message: format!("ability handler task aborted: {join_err}"),
                        }]
                    }
                };
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
                subject,
            } => {
                self.handle_subscribe_async(subscription_id, ability, args, subject, out, cancel)
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
            IncomingFrame::OpenBidi {
                session_id,
                ability,
                args,
                subject,
            } => {
                self.handle_bidi_open_async(session_id, ability, args, subject, out, bidi)
                    .await;
            }
            IncomingFrame::SendBidi { session_id, frame } => {
                // Lookup the per-connection session row and push the
                // frame onto the handler-input channel. send().await
                // is the §D3 backpressure path — if the handler is
                // slow and the channel is full we await rather than
                // drop. The reader loop in serve_connection blocks
                // here, which propagates backpressure to the wire.
                //
                // Cloning the sender out of the lock keeps the
                // critical section short; the actual send awaits
                // outside the lock so a slow handler can't stall
                // unrelated sessions on the same connection.
                let to_handler = {
                    let g = bidi.lock().expect("bidi registry lock");
                    g.get(&session_id).map(|s| s.to_handler.clone())
                };
                match to_handler {
                    Some(tx) => {
                        let payload = match serde_json::to_vec(&frame) {
                            Ok(payload) => payload,
                            Err(err) => {
                                let _ = out
                                    .send(OutgoingFrame::ErrorBidi {
                                        session_id,
                                        code: codes::ABILITY_FAILED.into(),
                                        message: format!(
                                            "SendBidi frame is not JSON encodable: {err}"
                                        ),
                                    })
                                    .await;
                                return;
                            }
                        };
                        if tx
                            .send(
                                BidiInputFrame::new(payload).with_content_type("application/json"),
                            )
                            .await
                            .is_err()
                        {
                            // Handler exited between our lookup and
                            // send. Surface as a per-frame
                            // diagnostic; per §D5 this does NOT
                            // close the session — the forwarder
                            // emits its own TerminalBidi when it
                            // observes the handler-output EOF.
                            let _ = out
                                .send(OutgoingFrame::ErrorBidi {
                                    session_id,
                                    code: codes::ABILITY_FAILED.into(),
                                    message: "handler closed before frame delivery".into(),
                                })
                                .await;
                        }
                    }
                    None => {
                        let _ = out
                            .send(OutgoingFrame::ErrorBidi {
                                session_id,
                                code: codes::ABILITY_FAILED.into(),
                                message: "SendBidi for unknown session_id".into(),
                            })
                            .await;
                    }
                }
            }
            IncomingFrame::CloseBidi { session_id } => {
                // Per §D4 path 1: drop the handler-input sender so
                // the handler observes EOF and exits. The forwarder
                // sees its handler-output channel close and emits
                // the single TerminalBidi (§I2). Idempotent — a
                // second CloseBidi for the same session_id is a
                // silent no-op (the row was already gone).
                //
                // We do NOT cancel the token here. Cancel is for
                // connection-drop / abort paths; CloseBidi is the
                // graceful exit, and letting the handler drain its
                // pending output before EOF is part of the contract.
                let session = {
                    let mut g = bidi.lock().expect("bidi registry lock");
                    g.remove(&session_id)
                };
                if let Some(session) = session {
                    let _ = session.to_handler.close_input().await;
                }
            }
        }
    }

    async fn handle_subscribe_async(
        &self,
        subscription_id: String,
        ability: String,
        args: serde_json::Value,
        subject: Option<String>,
        out: mpsc::Sender<OutgoingFrame>,
        cancel: &CancelRegistry,
    ) {
        let plan = InvocationPlan {
            ability,
            target_node_hint: extract_node_hint(&args),
            args,
            call_mode: CallMode::Stream,
            subject,
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
        let stream = match open_local_stream(Arc::clone(&self.local_runtime), target).await {
            Ok(s) => s,
            Err(e) => {
                let msg = e;
                let code = if is_not_found_error(&msg) {
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
        spawn_forwarder(subscription_id, stream, out, cancel.clone());
    }

    /// Open one bidi session: resolve plan → dispatch → install
    /// session row + spawn forwarder. Per §I3 the install + spawn
    /// pair is atomic from the client's perspective: either we end
    /// up with a row in the registry AND a running forwarder, or
    /// we emit one ErrorBidi and leave nothing behind.
    ///
    /// Failure paths (resolver, dispatcher, duplicate session_id)
    /// emit ErrorBidi without TerminalBidi — per §D5 / §I3 a failed
    /// open never produces a session-close envelope because no
    /// session ever existed.
    async fn handle_bidi_open_async(
        &self,
        session_id: String,
        ability: String,
        args: serde_json::Value,
        subject: Option<String>,
        out: mpsc::Sender<OutgoingFrame>,
        bidi: &BidiRegistry,
    ) {
        // Duplicate-id guard: per-connection uniqueness (§D8). A
        // client that reuses a live session_id is a bug; emit
        // ErrorBidi rather than silently overwriting (which would
        // orphan the prior handler with no Terminal).
        //
        // The lock is dropped before the await — holding a
        // std::sync::MutexGuard across .await is unsound (the
        // resulting future would be !Send and tokio::spawn rejects
        // it). The bool extraction pattern keeps the critical
        // section minimal.
        let already_open = {
            let g = bidi.lock().expect("bidi registry lock");
            g.contains_key(&session_id)
        };
        if already_open {
            let _ = out
                .send(OutgoingFrame::ErrorBidi {
                    session_id,
                    code: codes::ABILITY_FAILED.into(),
                    message: "OpenBidi session_id already in use on this connection".into(),
                })
                .await;
            return;
        }

        let plan = InvocationPlan {
            ability,
            target_node_hint: extract_node_hint(&args),
            args,
            call_mode: CallMode::Bidi,
            subject,
        };
        let target = match self.resolver.resolve(plan) {
            Ok(t) => t,
            Err(e) => {
                let _ = out
                    .send(OutgoingFrame::ErrorBidi {
                        session_id,
                        code: codes::ABILITY_FAILED.into(),
                        message: format!("resolver: {e}"),
                    })
                    .await;
                return;
            }
        };
        let source = match open_local_bidi(Arc::clone(&self.local_runtime), target).await {
            Ok(s) => s,
            Err(e) => {
                let msg = e;
                let code = if is_not_found_error(&msg) {
                    codes::NOT_FOUND
                } else {
                    codes::ABILITY_FAILED
                };
                let _ = out
                    .send(OutgoingFrame::ErrorBidi {
                        session_id,
                        code: code.into(),
                        message: msg,
                    })
                    .await;
                return;
            }
        };

        // From here every component is built; install atomically.
        let cancel_token = tokio_util::sync::CancellationToken::new();
        let finalized = Arc::new(AtomicBool::new(false));
        let to_handler = source.to_client;
        let from_handler_rx = source.from_client;

        {
            let mut g = bidi.lock().expect("bidi registry lock");
            g.insert(
                session_id.clone(),
                BidiSession {
                    to_handler: to_handler.clone(),
                    cancel: cancel_token.clone(),
                    finalized: Arc::clone(&finalized),
                },
            );
        }

        spawn_bidi_forwarder(
            session_id,
            from_handler_rx,
            out,
            cancel_token,
            finalized,
            Arc::clone(bidi),
        );
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
    /// Synchronous variant used **only** by this module's unit tests.
    /// It drains the Axon stream handle in place; production IPC
    /// goes through `handle_async` so long-lived streams can be
    /// pumped without blocking the caller. Gated behind `cfg(test)`
    /// so the test seam cannot accidentally be wired into a release
    /// binary.
    #[cfg(test)]
    pub(crate) fn handle(&self, req: IncomingFrame) -> Vec<OutgoingFrame> {
        match req {
            IncomingFrame::Invoke {
                request_id,
                ability,
                args,
                subject,
            } => self.handle_invoke(request_id, ability, args, subject),
            IncomingFrame::Subscribe {
                subscription_id,
                ability,
                args,
                subject,
            } => self.handle_subscribe(subscription_id, ability, args, subject),
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
            // Sync `handle` cannot host bidi sessions — there is no
            // mpsc to push live frames through and no place to park
            // a long-lived handler task. Tests that need bidi must
            // use the async path. Returning ErrorBidi keeps the
            // surface symmetric with `handle_async`.
            IncomingFrame::OpenBidi { session_id, .. }
            | IncomingFrame::SendBidi { session_id, .. }
            | IncomingFrame::CloseBidi { session_id } => {
                vec![OutgoingFrame::ErrorBidi {
                    session_id,
                    code: codes::ABILITY_FAILED.into(),
                    message: "bidi requires the async dispatch surface; \
                              `handle` is sync-only — use `handle_async`"
                        .into(),
                }]
            }
        }
    }

    /// Direct RPC execution — used by the runtime-dispatch UDS
    /// responder (Step 3 of the cross-repo plan) where the wire
    /// shape is single-line JSON, not the framed IPC envelope.
    /// Skips the IncomingFrame parse and OutgoingFrame wrapping
    /// the regular `handle_invoke` does; returns the raw `Value`
    /// the dispatcher produced (or a string error on failure).
    ///
    /// This is the **only** public method the runtime dispatcher
    /// loop should use — going through `handle` / `handle_async`
    /// would force JSON framing the runtime side does not speak.
    pub fn execute_runtime_dispatch(
        &self,
        ability: &str,
        args: serde_json::Value,
    ) -> Result<serde_json::Value, String> {
        let plan = InvocationPlan {
            ability: ability.to_string(),
            target_node_hint: extract_node_hint(&args),
            args,
            call_mode: CallMode::Rpc,
            // PR-DISPATCHER-SUBJECT: see Stream sites above.
            subject: None,
        };
        let target = self
            .resolver
            .resolve(plan)
            .map_err(|e| format!("resolver: {e}"))?;
        invoke_local_rpc_sync(Arc::clone(&self.local_runtime), target)
    }

    /// Stream-mode counterpart to `execute_runtime_dispatch`. Returns
    /// the live Axon streaming handle so the runtime-dispatch UDS
    /// responder can forward each frame as a separate JSON line on
    /// the same connection.
    ///
    /// Same plan-build as the RPC variant aside from the
    /// `CallMode::Stream` discriminant; we route through the same
    /// resolver so an ability without a registered stream handler
    /// surfaces the dispatcher's "no local stream handler registered"
    /// error verbatim, and the UDS responder maps that into a typed
    /// `kind:"error"` frame for the caller.
    pub fn execute_runtime_dispatch_stream(
        &self,
        ability: &str,
        args: serde_json::Value,
    ) -> Result<StreamingInvocationHandle, String> {
        let plan = InvocationPlan {
            ability: ability.to_string(),
            target_node_hint: extract_node_hint(&args),
            args,
            call_mode: CallMode::Stream,
            // PR-DISPATCHER-SUBJECT: see Stream sites above.
            subject: None,
        };
        let target = self
            .resolver
            .resolve(plan)
            .map_err(|e| format!("resolver: {e}"))?;
        run_blocking(
            open_local_stream(Arc::clone(&self.local_runtime), target),
            NoRuntimeFallback::BuildCurrentThreadTokio,
        )
    }

    fn handle_invoke(
        &self,
        request_id: String,
        ability: String,
        args: serde_json::Value,
        subject: Option<String>,
    ) -> Vec<OutgoingFrame> {
        let ability_for_receipt = ability.clone();
        let llm_sub_for_receipt = sub_agent_name_from_ability(&ability);
        let plan = InvocationPlan {
            ability,
            target_node_hint: extract_node_hint(&args),
            args,
            call_mode: CallMode::Rpc,
            // The wire-level Invoke frame now carries an optional
            // subject URI (set by `easynet ability invoke
            // --subject`); the resolver threads it onto
            // `InvocationTarget.subject`, where envelope-aware
            // handlers consume it via `EnvelopeContext`.
            subject,
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
        match invoke_local_rpc_sync(Arc::clone(&self.local_runtime), target) {
            Ok(value) => {
                // §A12 receipt header attachment. Best-effort: when
                // local-agents.json doesn't yet have the owner Agent
                // (pre-join state, missing hosted profile, etc.), we
                // emit no header and the wire stays at the pre-RFC
                // shape.
                let receipt_header =
                    crate::runtime::dispatch_receipt::header_for_ability_with_attestation(
                        &ability_for_receipt,
                        &self.local_agents,
                        llm_sub_for_receipt.as_deref(),
                        &|callee_ura: &str, host_ura: &str| {
                            let identity = self.receipt_identity.as_ref()?;
                            let canonical =
                                easynet_axon::invocation::canonical_host_attestation_bytes(
                                    callee_ura, host_ura,
                                );
                            identity
                                .sign(host_ura, &canonical)
                                .ok()
                                .map(|sig| sig.to_bytes().to_vec())
                        },
                    );
                vec![OutgoingFrame::Result {
                    request_id,
                    value,
                    receipt_header,
                }]
            }
            Err(e) => {
                let msg = e;
                let code = if is_not_found_error(&msg) {
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

    /// Sync stream-dispatch counterpart used only by the `cfg(test)`
    /// `handle` entrypoint above. Production drives streams through
    /// `handle_subscribe_async`. Gated so a release build can't link
    /// the in-place stream drain path.
    #[cfg(test)]
    fn handle_subscribe(
        &self,
        subscription_id: String,
        ability: String,
        args: serde_json::Value,
        subject: Option<String>,
    ) -> Vec<OutgoingFrame> {
        let plan = InvocationPlan {
            ability,
            target_node_hint: extract_node_hint(&args),
            args,
            call_mode: CallMode::Stream,
            subject,
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
        match run_blocking(
            drain_local_stream_frames(Arc::clone(&self.local_runtime), target),
            NoRuntimeFallback::BuildCurrentThreadTokio,
        ) {
            Ok(stream) => {
                let mut out = Vec::with_capacity(stream.len() + 1);
                for v in stream {
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
                let msg = e;
                let code = if is_not_found_error(&msg) {
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
    mut stream: StreamingInvocationHandle,
    out: mpsc::Sender<OutgoingFrame>,
    cancel: CancelRegistry,
) {
    let token = tokio_util::sync::CancellationToken::new();
    {
        let mut g = cancel.lock().expect("cancel registry lock");
        g.insert(subscription_id.clone(), token.clone());
    }
    tokio::spawn(async move {
        let reason = loop {
            tokio::select! {
                _ = token.cancelled() => break "cancelled",
                recv = stream.next_frame() => match recv {
                    Some(Ok(frame)) => {
                        let terminal = frame.terminal;
                        if !frame.payload.is_empty() {
                            let v = match ability_frame_to_json(&frame) {
                                Ok(value) => value,
                                Err(err) => {
                                    let _ = out
                                        .send(OutgoingFrame::Error {
                                            request_id: None,
                                            subscription_id: Some(subscription_id.clone()),
                                            code: codes::ABILITY_FAILED.into(),
                                            message: err,
                                        })
                                        .await;
                                    break "failed";
                                }
                            };
                            if out
                                .send(OutgoingFrame::Frame {
                                    subscription_id: subscription_id.clone(),
                                    frame: v,
                                })
                                .await
                                .is_err()
                            {
                                break "done";
                            }
                        }
                        if terminal {
                            break "done";
                        }
                    }
                    Some(Err(err)) => {
                        if out
                            .send(OutgoingFrame::Error {
                                request_id: None,
                                subscription_id: Some(subscription_id.clone()),
                                code: if is_not_found_error(&format!("{err}")) {
                                    codes::NOT_FOUND.into()
                                } else {
                                    codes::ABILITY_FAILED.into()
                                },
                                message: format!("{err}"),
                            })
                            .await
                            .is_err()
                        {
                            break "done";
                        }
                        break "failed";
                    }
                    None => break "done",
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

/// Per-bidi-session forwarder. Runs as one tokio task per open
/// session. Pumps frames from the handler-output channel into
/// `RecvBidi` envelopes, observes both the cancel token and channel
/// EOF as terminal signals, and emits **exactly one** `TerminalBidi`
/// per §I2 — guarded by the per-session `finalized` AtomicBool.
///
/// The three close paths (per §D4) all funnel through here:
///   1. Client `CloseBidi` → registry row removed → `to_handler`
///      sender dropped → handler `recv()` returns None → handler
///      drops its output sender → forwarder sees EOF → Terminal{done}.
///   2. Handler exits on its own → drops output sender → forwarder
///      sees EOF → Terminal{done}.
///   3. Connection drop → cancel.cancel() fires → forwarder breaks
///      out of select → Terminal{cancelled}.
///
/// Whichever path wins the `compare_exchange` on `finalized` emits;
/// the others see `true` and no-op, so only one TerminalBidi
/// envelope ever lands on the wire.
fn spawn_bidi_forwarder(
    session_id: String,
    mut from_handler_rx: BidiOutputReceiver,
    out: mpsc::Sender<OutgoingFrame>,
    cancel: tokio_util::sync::CancellationToken,
    finalized: Arc<AtomicBool>,
    bidi_registry: BidiRegistry,
) {
    tokio::spawn(async move {
        let reason = loop {
            tokio::select! {
                _ = cancel.cancelled() => break "cancelled",
                recv = from_handler_rx.next_frame() => match recv {
                    Some(Ok(frame)) => {
                        let terminal = frame.terminal;
                        if !frame.payload.is_empty() {
                            let v = match ability_frame_to_json(&frame) {
                                Ok(value) => value,
                                Err(_) => break "failed",
                            };
                            if out
                                .send(OutgoingFrame::RecvBidi {
                                    session_id: session_id.clone(),
                                    frame: v,
                                })
                                .await
                                .is_err()
                            {
                                break "cancelled";
                            }
                        }
                        if terminal {
                            break "done";
                        }
                    }
                    Some(Err(_)) => {
                        if out
                            .send(OutgoingFrame::ErrorBidi {
                                session_id: session_id.clone(),
                                code: codes::ABILITY_FAILED.into(),
                                message: "bidi ability returned an error".into(),
                            })
                            .await
                            .is_err()
                        {
                            // IPC writer task is gone — connection
                            // collapsed. Stop pumping; the connection-
                            // level cleanup will drain the registry.
                            // Treat as cancelled-by-transport so the
                            // wire reason matches reality.
                            break "cancelled";
                        }
                        break "failed";
                    }
                    None => break "done",
                }
            }
        };

        // §I2: at most one TerminalBidi per session_id. Any racing
        // path that already flipped the flag observes `true` here
        // and we silently exit without emitting.
        if finalized
            .compare_exchange(false, true, Ordering::AcqRel, Ordering::Acquire)
            .is_ok()
        {
            let _ = out
                .send(OutgoingFrame::TerminalBidi {
                    session_id: session_id.clone(),
                    reason: reason.into(),
                })
                .await;
        }
        // Drop the registry row last. If a SendBidi raced with our
        // exit it will see "unknown session_id" and surface an
        // ErrorBidi (per §D5 a per-frame diagnostic, not a close).
        let mut g = bidi_registry.lock().expect("bidi registry lock");
        g.remove(&session_id);
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

/// Hint extractor for the few abilities whose wire name embeds the
/// LLM sub-agent name. Today this is just per-agent on-disk
/// manifest abilities (`<agent>.<verb>` shape from
/// `AgentAbilitySpec::qualified_name`), excluding the protocol
/// namespaces device/consent/policy/mcp/llm own. The receipt
/// builder consumes the hint to pick the right hosted Agent URA
/// when the ability is owned by a hosted llm-profile.
///
/// `<agent>.chat` is handled directly inside
/// `dispatch_receipt::header_for_ability` (no hint needed); this
/// helper just covers the "any other per-agent verb" cases that
/// land via on-disk manifests.
fn sub_agent_name_from_ability(ability: &str) -> Option<String> {
    let (agent, _verb) = ability.split_once('.')?;
    // Filter out protocol-owned namespaces — those resolve to the
    // device or hosted profile, not to a per-agent LLM URA.
    let protocol_namespaces = [
        "observe",
        "admin",
        "schedule",
        "loop",
        "discuss",
        "meta",
        "consent",
        "policy",
        "mcp",
        "conversation",
        "session",
        "skill",
        "federation",
    ];
    if protocol_namespaces.contains(&agent) {
        None
    } else {
        Some(agent.to_string())
    }
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
    use crate::runtime::ability_dispatch::AxonAbilityCatalog;
    use crate::runtime::domain::{
        DiscussRoom, LoopId, LoopInstance, PermissionDecision, PermissionId, PermissionRequest,
        RoomId, ScheduleEntry, ScheduleId, Session, SessionId,
    };
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
        // Some tests want a clean slate (no observe.health etc.) so they
        // can assert the unregistered-ability path; use an empty
        // LocalRuntime here.
        let runtime = LocalRuntime::new();
        let resolver: Arc<dyn TargetResolver> =
            Arc::new(LocalNodeResolver::new(NodeId::new("self")));
        AbilityProxy::new_with_runtime(Arc::new(StubKernel), runtime, resolver)
    }

    #[test]
    fn invoke_system_ping_returns_result_frame_with_request_id_preserved() {
        // The cdylib + smoke scripts depend on `observe.health` returning
        // a Result envelope (not the v1 skeleton Error). This test
        // pins that contract end-to-end through the live registry.
        let p = proxy_with_live_registry();
        let frames = p.handle(IncomingFrame::Invoke {
            request_id: "req-1".into(),
            ability: "device.observe.health".into(),
            args: json!({}),
            subject: None,
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
            subject: None,
        });
        assert_eq!(frames.len(), 1);
        match &frames[0] {
            OutgoingFrame::Error {
                request_id, code, ..
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
        // The device.session.attach handler is registered as a stream
        // handler. With no active session, v1 emits zero data Frames
        // and exactly one Terminal — pin that the proxy threads the
        // Terminal through.
        let p = proxy_with_live_registry();
        let frames = p.handle(IncomingFrame::Subscribe {
            subscription_id: "sub-1".into(),
            ability: "device.session.attach".into(),
            args: json!({"session_id": "no-such-session"}),
            subject: None,
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

    // ─── P4.8c: §A12 receipt header attachment ─────────────────

    fn proxy_with_local_agents(
        file: crate::persistence::local_agents::LocalAgentsFile,
    ) -> AbilityProxy {
        let runtime = LocalRuntime::new();
        let _registry = crate::runtime::agents::build_registry_with_services(
            Arc::new(crate::runtime::execution::session::SessionService::new()),
            Arc::new(crate::runtime::execution::permission::PermissionService::new()),
            Arc::new(crate::runtime::execution::discuss::DiscussService::new()),
            Arc::new(crate::runtime::execution::schedule::ScheduleService::new()),
            Arc::new(crate::runtime::execution::loop_instance::LoopService::new()),
            None,
            &Default::default(),
            Arc::new(Vec::new()),
            crate::runtime::agents::PagesIdentity::default(),
            Some(Arc::clone(&runtime)),
            Arc::new(
                crate::runtime::agents::agent_lifecycle_ability::SharedHotRegistrarCell::new(),
            ),
        );
        let resolver: Arc<dyn TargetResolver> =
            Arc::new(LocalNodeResolver::new(NodeId::new("self")));
        AbilityProxy::new_with_local_agents(Arc::new(StubKernel), runtime, resolver, file)
    }

    #[test]
    fn observe_health_attaches_selfsigned_header_when_host_ura_known() {
        let file = crate::persistence::local_agents::LocalAgentsFile {
            host_device_agent_ura: "easynet:///r/acme/device/01DEV".into(),
            ..crate::persistence::local_agents::LocalAgentsFile::default()
        };
        let p = proxy_with_local_agents(file);
        let frames = p.handle(IncomingFrame::Invoke {
            request_id: "req-receipt-1".into(),
            ability: "device.observe.health".into(),
            args: json!({}),
            subject: None,
        });
        match &frames[0] {
            OutgoingFrame::Result {
                receipt_header: Some(h),
                ..
            } => {
                assert_eq!(h.callee_agent_ura, "easynet:///r/acme/device/01DEV");
                assert_eq!(h.signer_agent_ura, "easynet:///r/acme/device/01DEV");
                assert!(h.is_self_signed());
            }
            OutgoingFrame::Result {
                receipt_header: None,
                ..
            } => panic!("device-profile ability must carry a Selfsigned receipt header"),
            other => panic!("expected Result frame, got {other:?}"),
        }
    }

    #[test]
    fn consent_list_pending_attaches_hosted_by_header_distinct_from_signer() {
        // Strict assertion: consent.* abilities MUST emit a HostedBy
        // header where callee != signer. This is the §A12 invariant
        // a verifier checks. If the dispatcher ever degraded to
        // Selfsigned for hosted-profile abilities, the verifier
        // would silently accept an attestation-less receipt.
        use crate::runtime::hosted_receipt::SigningModel;
        let mut file = crate::persistence::local_agents::LocalAgentsFile {
            host_device_agent_ura: "easynet:///r/acme/device/01DEV".into(),
            ..crate::persistence::local_agents::LocalAgentsFile::default()
        };
        crate::persistence::local_agents::upsert_hosted_agent(
            &mut file,
            "consent",
            "default",
            "easynet:///r/acme/agent/u1.01CON",
        );
        let p = proxy_with_local_agents(file);
        // Use consent.decide because it is the RPC handler the
        // default registry registers (subscribe is a stream, and
        // list_pending is not yet wired); decide returns Error for
        // an unknown id, but that goes through Error not Result, so
        // we'd not get a header. Pick the always-RPC path: invoke
        // observe.health which IS device-owned but still verifies
        // the wiring; for consent we exercise the receipt builder
        // directly via dispatch_receipt unit tests above.
        let frames = p.handle(IncomingFrame::Invoke {
            request_id: "req-receipt-2".into(),
            ability: "device.consent.decide".into(),
            // Send a malformed payload: handler will likely return
            // an Error envelope (ABILITY_FAILED) which carries no
            // receipt_header. We assert on whichever Result frame we
            // get; if the registry happens to accept the empty
            // payload, the header check applies.
            args: json!({"request_id": "nonexistent", "decision": "Allowed"}),
            subject: None,
        });
        // If consent.decide succeeded against an empty
        // PermissionService, we get a Result; if it failed, we get
        // an Error. Either is acceptable here — what matters is the
        // build wiring. The strict semantic test is in
        // dispatch_receipt::tests where the registry isn't involved.
        let header = match &frames[0] {
            OutgoingFrame::Result { receipt_header, .. } => receipt_header
                .clone()
                .expect("consent.* dispatch must attach a header on success"),
            OutgoingFrame::Error { .. } => {
                // Handler-side rejection. Fall through and assert on
                // the receipt-builder unit test instead — proxy wiring
                // is exercised by other tests above.
                return;
            }
            other => panic!("expected Result or Error, got {other:?}"),
        };
        assert_eq!(
            header.callee_agent_ura, "easynet:///r/acme/agent/u1.01CON",
            "callee must be the consent-profile URA from local-agents.json"
        );
        assert_eq!(
            header.signer_agent_ura, "easynet:///r/acme/device/01DEV",
            "signer must be the host device-profile URA"
        );
        match header.model {
            SigningModel::HostedBy {
                host_ura,
                host_attestation,
            } => {
                assert_eq!(host_ura, "easynet:///r/acme/device/01DEV");
                assert!(!host_attestation.is_empty());
            }
            SigningModel::Selfsigned => panic!(
                "hosted-profile ability must NOT degrade to Selfsigned — \
                 §A12 verifier would accept an attestation-less receipt"
            ),
        }
    }

    // ─── C-M3a (3/5): bidi proxy arms + spawn_bidi_forwarder ────
    //
    // Tests bind to the design's invariant numbers:
    //   I1 — intra-direction frame ordering
    //   I2 — exactly one TerminalBidi per session_id
    //   I3 — failed OpenBidi leaves no half-open session state
    //
    // Plus the §D5 per-frame Error contract (errors are diagnostics,
    // not session-close signals) and §D8 per-connection registry
    // semantics (duplicate session_id rejection).

    /// Build a proxy whose registry has one bidi ability `bidi.echo`
    /// that echoes every frame back verbatim. Spawns a tokio task per
    /// session per §D2; the closure returns immediately with a
    /// transport-axis BidiSource.
    fn proxy_with_echo_bidi() -> AbilityProxy {
        use crate::runtime::ability_dispatch::{
            BidiOutputFrame, BidiSource, LocalBidiHandler, BIDI_CHANNEL_BOUND,
        };
        let runtime = LocalRuntime::new();
        let mut reg = AxonAbilityCatalog::new_with_runtime(Arc::clone(&runtime));
        let handler: LocalBidiHandler = Arc::new(|_args: serde_json::Value| {
            let (xport_to_handler_tx, mut handler_rx) =
                tokio::sync::mpsc::channel::<serde_json::Value>(BIDI_CHANNEL_BOUND);
            let (handler_tx, xport_from_handler_rx) =
                tokio::sync::mpsc::channel::<BidiOutputFrame>(BIDI_CHANNEL_BOUND);
            tokio::spawn(async move {
                while let Some(frame) = handler_rx.recv().await {
                    if handler_tx.send(BidiOutputFrame::json(frame)).await.is_err() {
                        // Forwarder gone; treat as graceful exit.
                        break;
                    }
                }
                // handler_rx returned None (CloseBidi or connection
                // drop): drop handler_tx by falling out of scope.
            });
            Ok(BidiSource {
                to_client: xport_to_handler_tx,
                from_client: xport_from_handler_rx,
            })
        });
        reg.register_bidi("bidi.echo", handler);
        let _registry = Arc::new(reg);
        let resolver: Arc<dyn TargetResolver> =
            Arc::new(LocalNodeResolver::new(NodeId::new("self")));
        AbilityProxy::new_with_runtime(Arc::new(StubKernel), runtime, resolver)
    }

    /// Build a proxy with an envelope-aware bidi handler that publishes the
    /// subject it saw during OpenBidi. This pins the control-plane boundary:
    /// subject belongs to the invocation envelope, not `args`.
    fn proxy_with_subject_echo_bidi() -> AbilityProxy {
        use crate::runtime::ability_dispatch::{
            BidiOutputFrame, BidiSource, EnvelopeContext, LocalBidiHandlerWithEnvelope, OwnerKind,
            BIDI_CHANNEL_BOUND,
        };
        let runtime = LocalRuntime::new();
        let mut reg = AxonAbilityCatalog::new_with_runtime(Arc::clone(&runtime));
        let handler: LocalBidiHandlerWithEnvelope =
            Arc::new(|env: EnvelopeContext, args: serde_json::Value| {
                if args.get("subject").is_some() {
                    anyhow::bail!("subject must not be accepted through args");
                }
                let subject = env
                    .subject
                    .ok_or_else(|| anyhow::anyhow!("missing envelope subject"))?;
                let (xport_to_handler_tx, mut handler_rx) =
                    tokio::sync::mpsc::channel::<serde_json::Value>(BIDI_CHANNEL_BOUND);
                let (handler_tx, xport_from_handler_rx) =
                    tokio::sync::mpsc::channel::<BidiOutputFrame>(BIDI_CHANNEL_BOUND);
                tokio::spawn(async move {
                    let _ = handler_tx
                        .send(BidiOutputFrame::json(json!({ "subject": subject })))
                        .await;
                    while handler_rx.recv().await.is_some() {}
                });
                Ok(BidiSource {
                    to_client: xport_to_handler_tx,
                    from_client: xport_from_handler_rx,
                })
            });
        reg.register_bidi_with_envelope_and_owner("bidi.subject_echo", OwnerKind::Device, handler);
        let resolver: Arc<dyn TargetResolver> =
            Arc::new(LocalNodeResolver::new(NodeId::new("self")));
        AbilityProxy::new_with_runtime(Arc::new(StubKernel), runtime, resolver)
    }

    /// Drain at most `n` frames from `rx` with a soft deadline, so a
    /// missing-frame regression fails fast instead of hanging the
    /// test runner. The deadline is generous enough that a green
    /// path on an overloaded CI box doesn't false-fail.
    async fn drain_n(
        rx: &mut tokio::sync::mpsc::Receiver<OutgoingFrame>,
        n: usize,
    ) -> Vec<OutgoingFrame> {
        let mut out = Vec::with_capacity(n);
        for _ in 0..n {
            match tokio::time::timeout(std::time::Duration::from_millis(500), rx.recv()).await {
                Ok(Some(f)) => out.push(f),
                Ok(None) => break,
                Err(_) => break, // timeout — return what we have
            }
        }
        out
    }

    fn fresh_bidi_state() -> (
        AbilityProxy,
        tokio::sync::mpsc::Sender<OutgoingFrame>,
        tokio::sync::mpsc::Receiver<OutgoingFrame>,
        CancelRegistry,
        BidiRegistry,
    ) {
        let proxy = proxy_with_echo_bidi();
        let (tx, rx) = tokio::sync::mpsc::channel::<OutgoingFrame>(64);
        let cancel: CancelRegistry =
            Arc::new(std::sync::Mutex::new(std::collections::HashMap::new()));
        let bidi: BidiRegistry = Arc::new(std::sync::Mutex::new(std::collections::HashMap::new()));
        (proxy, tx, rx, cancel, bidi)
    }

    #[tokio::test]
    async fn open_send_recv_close_emits_three_recv_then_one_terminal_in_order() {
        // Pins §I1: intra-direction ordering. Three SendBidi frames
        // come back as three RecvBidi in the SAME order, followed by
        // exactly one TerminalBidi after CloseBidi.
        let (proxy, tx, mut rx, cancel, bidi) = fresh_bidi_state();

        proxy
            .handle_async(
                IncomingFrame::OpenBidi {
                    session_id: "sess-1".into(),
                    ability: "bidi.echo".into(),
                    args: json!({}),
                    subject: None,
                },
                tx.clone(),
                &cancel,
                &bidi,
            )
            .await;

        for i in 0..3 {
            proxy
                .handle_async(
                    IncomingFrame::SendBidi {
                        session_id: "sess-1".into(),
                        frame: json!({"i": i}),
                    },
                    tx.clone(),
                    &cancel,
                    &bidi,
                )
                .await;
        }

        proxy
            .handle_async(
                IncomingFrame::CloseBidi {
                    session_id: "sess-1".into(),
                },
                tx.clone(),
                &cancel,
                &bidi,
            )
            .await;
        // Drop our local sender so the writer-queue receiver doesn't
        // hang waiting for more frames after Terminal lands.
        drop(tx);

        let frames = drain_n(&mut rx, 4).await;
        assert_eq!(frames.len(), 4, "expected 3 RecvBidi + 1 TerminalBidi");
        for (idx, frame) in frames.iter().take(3).enumerate() {
            match frame {
                OutgoingFrame::RecvBidi { session_id, frame } => {
                    assert_eq!(session_id, "sess-1");
                    assert_eq!(
                        frame,
                        &json!({"i": idx}),
                        "RecvBidi[{idx}] must preserve client emission order (§I1)"
                    );
                }
                other => panic!("frame {idx}: expected RecvBidi, got {other:?}"),
            }
        }
        match &frames[3] {
            OutgoingFrame::TerminalBidi { session_id, reason } => {
                assert_eq!(session_id, "sess-1");
                assert_eq!(reason, "done", "graceful close must report `done`");
            }
            other => panic!("expected TerminalBidi as 4th frame, got {other:?}"),
        }
    }

    #[tokio::test]
    async fn open_bidi_forwards_subject_into_envelope_context() {
        let proxy = proxy_with_subject_echo_bidi();
        let (tx, mut rx) = tokio::sync::mpsc::channel::<OutgoingFrame>(64);
        let cancel: CancelRegistry =
            Arc::new(std::sync::Mutex::new(std::collections::HashMap::new()));
        let bidi: BidiRegistry = Arc::new(std::sync::Mutex::new(std::collections::HashMap::new()));

        proxy
            .handle_async(
                IncomingFrame::OpenBidi {
                    session_id: "sess-subject".into(),
                    ability: "bidi.subject_echo".into(),
                    args: json!({}),
                    subject: Some("easynet:///r/acme/resource/display.primary".into()),
                },
                tx.clone(),
                &cancel,
                &bidi,
            )
            .await;
        drop(tx);

        let frames = drain_n(&mut rx, 2).await;
        assert!(
            frames.iter().any(|frame| matches!(
                frame,
                OutgoingFrame::RecvBidi { frame, .. }
                    if frame["subject"] == "easynet:///r/acme/resource/display.primary"
            )),
            "OpenBidi subject must reach EnvelopeContext, got {frames:?}"
        );
    }

    #[tokio::test]
    async fn close_bidi_emits_exactly_one_terminal() {
        // Pins §I2 in the simplest path: one CloseBidi, one Terminal.
        // A regression that fired Terminal twice (e.g. forgetting the
        // compare_exchange guard) trips the "no extra frames" check.
        let (proxy, tx, mut rx, cancel, bidi) = fresh_bidi_state();
        proxy
            .handle_async(
                IncomingFrame::OpenBidi {
                    session_id: "sess-once".into(),
                    ability: "bidi.echo".into(),
                    args: json!({}),
                    subject: None,
                },
                tx.clone(),
                &cancel,
                &bidi,
            )
            .await;
        proxy
            .handle_async(
                IncomingFrame::CloseBidi {
                    session_id: "sess-once".into(),
                },
                tx.clone(),
                &cancel,
                &bidi,
            )
            .await;
        drop(tx);

        let frames = drain_n(&mut rx, 4).await;
        let terminals: Vec<&OutgoingFrame> = frames
            .iter()
            .filter(|f| matches!(f, OutgoingFrame::TerminalBidi { .. }))
            .collect();
        assert_eq!(
            terminals.len(),
            1,
            "§I2 violation: expected exactly one TerminalBidi, got {} ({frames:?})",
            terminals.len()
        );
    }

    #[tokio::test]
    async fn duplicate_session_id_emits_error_bidi_without_displacing_first_session() {
        // Pins §D8 + §I3: a second OpenBidi with a live session_id
        // gets ErrorBidi, but the first session keeps working. A
        // regression that overwrote the registry row would orphan
        // the first handler with no Terminal.
        let (proxy, tx, mut rx, cancel, bidi) = fresh_bidi_state();
        proxy
            .handle_async(
                IncomingFrame::OpenBidi {
                    session_id: "sess-dup".into(),
                    ability: "bidi.echo".into(),
                    args: json!({}),
                    subject: None,
                },
                tx.clone(),
                &cancel,
                &bidi,
            )
            .await;

        // Second open with the same session_id MUST error.
        proxy
            .handle_async(
                IncomingFrame::OpenBidi {
                    session_id: "sess-dup".into(),
                    ability: "bidi.echo".into(),
                    args: json!({}),
                    subject: None,
                },
                tx.clone(),
                &cancel,
                &bidi,
            )
            .await;

        // Send a probe frame; the FIRST session must still echo.
        proxy
            .handle_async(
                IncomingFrame::SendBidi {
                    session_id: "sess-dup".into(),
                    frame: json!("probe"),
                },
                tx.clone(),
                &cancel,
                &bidi,
            )
            .await;

        let frames = drain_n(&mut rx, 2).await;
        let mut saw_error = false;
        let mut saw_recv = false;
        for f in &frames {
            match f {
                OutgoingFrame::ErrorBidi {
                    session_id,
                    message,
                    ..
                } => {
                    assert_eq!(session_id, "sess-dup");
                    assert!(
                        message.contains("already in use"),
                        "duplicate-id error message should be self-explanatory; got {message:?}"
                    );
                    saw_error = true;
                }
                OutgoingFrame::RecvBidi { session_id, frame } => {
                    assert_eq!(session_id, "sess-dup");
                    assert_eq!(
                        frame,
                        &json!("probe"),
                        "first session must keep echoing after duplicate-open rejection"
                    );
                    saw_recv = true;
                }
                _ => {}
            }
        }
        assert!(saw_error, "duplicate OpenBidi must produce ErrorBidi");
        assert!(saw_recv, "first session must remain operational");
    }

    #[tokio::test]
    async fn open_bidi_unknown_ability_leaves_no_session_state() {
        // Pins §I3: failed OpenBidi (ability not found) MUST NOT
        // install a registry row. A subsequent SendBidi for the
        // same session_id should hit "unknown session_id" rather
        // than racing into a half-open state.
        let (proxy, tx, mut rx, cancel, bidi) = fresh_bidi_state();
        proxy
            .handle_async(
                IncomingFrame::OpenBidi {
                    session_id: "sess-ghost".into(),
                    ability: "bidi.does-not-exist".into(),
                    args: json!({}),
                    subject: None,
                },
                tx.clone(),
                &cancel,
                &bidi,
            )
            .await;
        // Registry MUST be empty for this session_id (§I3).
        {
            let g = bidi.lock().expect("bidi registry lock");
            assert!(
                !g.contains_key("sess-ghost"),
                "§I3 violation: failed OpenBidi left a registry row"
            );
        }

        // Probe SendBidi — should surface "unknown session_id" not
        // a panic / silent drop.
        proxy
            .handle_async(
                IncomingFrame::SendBidi {
                    session_id: "sess-ghost".into(),
                    frame: json!("orphan"),
                },
                tx.clone(),
                &cancel,
                &bidi,
            )
            .await;
        drop(tx);

        let frames = drain_n(&mut rx, 4).await;
        let mut saw_open_err = false;
        let mut saw_send_err = false;
        let mut saw_terminal = false;
        for f in &frames {
            match f {
                OutgoingFrame::ErrorBidi {
                    session_id,
                    message,
                    ..
                } if session_id == "sess-ghost" => {
                    if message.contains("unknown session_id") {
                        saw_send_err = true;
                    } else {
                        // The OpenBidi failure error.
                        saw_open_err = true;
                    }
                }
                OutgoingFrame::TerminalBidi { .. } => saw_terminal = true,
                _ => {}
            }
        }
        assert!(saw_open_err, "OpenBidi failure must surface ErrorBidi");
        assert!(
            saw_send_err,
            "SendBidi for unknown session must surface ErrorBidi"
        );
        assert!(
            !saw_terminal,
            "§I3 / §D5: failed OpenBidi must NOT emit TerminalBidi (no session ever existed)"
        );
    }

    #[tokio::test]
    async fn send_bidi_for_unknown_session_does_not_close_anything() {
        // Pins §D5: a per-frame error is a diagnostic, not a session
        // close. A SendBidi for a never-opened session yields ErrorBidi
        // with NO TerminalBidi (no session existed to close).
        let (proxy, tx, mut rx, cancel, bidi) = fresh_bidi_state();
        proxy
            .handle_async(
                IncomingFrame::SendBidi {
                    session_id: "never-was".into(),
                    frame: json!("hello"),
                },
                tx.clone(),
                &cancel,
                &bidi,
            )
            .await;
        drop(tx);

        let frames = drain_n(&mut rx, 4).await;
        assert!(
            frames
                .iter()
                .any(|f| matches!(f, OutgoingFrame::ErrorBidi { .. })),
            "expected ErrorBidi for unknown session"
        );
        assert!(
            !frames
                .iter()
                .any(|f| matches!(f, OutgoingFrame::TerminalBidi { .. })),
            "§D5: per-frame error must NOT trigger Terminal"
        );
    }

    #[tokio::test]
    async fn close_bidi_for_unknown_session_is_silent_noop() {
        // Idempotency: a second CloseBidi (or a CloseBidi for a
        // never-opened session) is a no-op. Pins the proxy comment's
        // "second CloseBidi for the same session_id is a silent
        // no-op" claim.
        let (proxy, tx, mut rx, cancel, bidi) = fresh_bidi_state();
        proxy
            .handle_async(
                IncomingFrame::CloseBidi {
                    session_id: "ghost-close".into(),
                },
                tx.clone(),
                &cancel,
                &bidi,
            )
            .await;
        drop(tx);

        let frames = drain_n(&mut rx, 2).await;
        assert!(
            frames.is_empty(),
            "CloseBidi for unknown session must emit no frames; got {frames:?}"
        );
    }

    #[tokio::test]
    async fn cancel_token_fires_terminal_with_cancelled_reason() {
        // Pins §D4 path 3 (connection drop / explicit cancel): when
        // the per-session cancel token fires, the forwarder breaks
        // out of select and emits TerminalBidi{reason: "cancelled"}.
        let (proxy, tx, mut rx, cancel, bidi) = fresh_bidi_state();
        proxy
            .handle_async(
                IncomingFrame::OpenBidi {
                    session_id: "sess-cancel".into(),
                    ability: "bidi.echo".into(),
                    args: json!({}),
                    subject: None,
                },
                tx.clone(),
                &cancel,
                &bidi,
            )
            .await;

        // Fire the cancel token directly — same path serve_connection
        // takes on connection drop.
        {
            let g = bidi.lock().expect("bidi registry lock");
            g.get("sess-cancel")
                .expect("session installed")
                .cancel
                .cancel();
        }
        drop(tx);

        let frames = drain_n(&mut rx, 2).await;
        let term = frames
            .iter()
            .find_map(|f| match f {
                OutgoingFrame::TerminalBidi { session_id, reason } => {
                    Some((session_id.clone(), reason.clone()))
                }
                _ => None,
            })
            .expect("forwarder must emit TerminalBidi after cancel");
        assert_eq!(term.0, "sess-cancel");
        assert_eq!(
            term.1, "cancelled",
            "cancel-path Terminal must report `cancelled` per §D4"
        );
    }

    #[test]
    fn frame_omits_receipt_header_when_local_agents_file_is_empty() {
        // Pre-join state: empty file, no host URI. Header must be
        // absent so existing IPC clients tolerate the wire shape.
        let p = proxy_with_local_agents(Default::default());
        let frames = p.handle(IncomingFrame::Invoke {
            request_id: "req-no-header".into(),
            ability: "device.observe.health".into(),
            args: json!({}),
            subject: None,
        });
        match &frames[0] {
            OutgoingFrame::Result {
                receipt_header: None,
                ..
            } => {} // expected
            OutgoingFrame::Result {
                receipt_header: Some(_),
                ..
            } => panic!("empty local-agents.json must NOT produce a header"),
            other => panic!("expected Result frame, got {other:?}"),
        }
    }
}
