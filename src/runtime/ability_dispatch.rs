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
use tokio::sync::{broadcast, mpsc};

use crate::runtime::gateway_api::GatewayApi;
use crate::runtime::invocation_target::{CallMode, InvocationTarget, TargetScope};

/// One in-process RPC handler. Boxed closure so the registry can
/// hold heterogeneous handlers behind a uniform key.
pub type LocalRpcHandler = Arc<dyn Fn(Value) -> anyhow::Result<Value> + Send + Sync>;

/// Slice of the AXIOM 7-tuple that an envelope-aware handler needs
/// access to. Currently carries `subject` only — extending this is
/// the path for future envelope fields (delegation, causal_context)
/// to reach handlers without another sweep through call sites.
///
/// Per **INV-SUBJECT-ENVELOPE**: this is the ONLY way a handler
/// reads its `subject`. Handlers MUST NOT accept `subject` in
/// `args`; the `register_*_with_envelope` family of methods is
/// the way for handlers to opt into envelope access.
#[derive(Debug, Clone, Default)]
pub struct EnvelopeContext {
    /// AXIOM 7-tuple `subject`. `None` for legacy abilities and
    /// for the degenerate `subject = callee` case (per
    /// INV-META-SUBJECT-EXEMPT). Resource handlers MUST treat
    /// `None` as a missing-subject failure (`resource_not_found`
    /// or InvalidArgument).
    pub subject: Option<String>,
}

/// Envelope-aware RPC handler. The dispatcher passes a snapshot of
/// the relevant envelope fields alongside the args. Used by media
/// abilities (which need `subject` for resource resolution) and any
/// future ability that needs to inspect AXIOM-layer state without
/// pulling it out of args.
pub type LocalRpcHandlerWithEnvelope =
    Arc<dyn Fn(EnvelopeContext, Value) -> anyhow::Result<Value> + Send + Sync>;

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
///     after. The "replay then subscribe" composition every
///     state-then-stream UI wants: a Permission dialog joining
///     mid-flight needs to see currently-pending requests AND new
///     ones; a Discuss room view shows past turns AND new posts.
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
pub type LocalStreamHandler = Arc<dyn Fn(Value) -> anyhow::Result<StreamSource> + Send + Sync>;

/// Envelope-aware stream handler. Mirrors `LocalRpcHandlerWithEnvelope`.
pub type LocalStreamHandlerWithEnvelope =
    Arc<dyn Fn(EnvelopeContext, Value) -> anyhow::Result<StreamSource> + Send + Sync>;

/// Channel bound for both directions of every bidi session. Per
/// C-M3a §D1: not exposed as a `register_bidi` parameter — the
/// transport layer enforces transport backpressure; per-ability
/// scrollback / history buffering belongs in adapter layers (PTY,
/// SSE) so the registration API never grows a tuning surface.
///
/// Sized to match the per-connection IPC writer queue
/// (services/control/server.rs) so a single saturated session
/// cannot exceed the writer's own backlog.
pub const BIDI_CHANNEL_BOUND: usize = 256;

/// Both ends of one open bidi session, **as seen by the transport
/// layer** (the IPC server). Per C-M3a §D1, two distinct `mpsc`
/// channels (not `broadcast`) — bidi sessions are point-to-point,
/// fan-out is wrong, and broadcast's lag-on-slow-consumer semantics
/// would turn every backpressured frame into an error rather than a
/// wait.
///
/// What each end owns (transport perspective)
/// ------------------------------------------
///   * `to_client` (Sender) — the **transport** pushes here when
///     `SendBidi` arrives. The handler's matching `Receiver` is
///     held by the spawned session loop; reading EOF is the
///     canonical "client side closed" signal (§D4 path 1).
///   * `from_client` (Receiver) — the **transport** reads here and
///     emits each frame as a `RecvBidi` envelope. The handler's
///     matching `Sender` is held by the spawned session loop;
///     dropping it is the "handler done" signal (§D4 path 2).
///
/// Field names sit on the *transport's* axis: `to_client` = "what
/// I (transport) write into; the client's words", `from_client` =
/// "what I (transport) read out of; the client's words eventually
/// echo here via the handler". Reading them as handler-perspective
/// is the historical bug — the names are stable because the
/// transport is the only consumer of this struct outside of test
/// fixtures.
///
/// Lifecycle invariants (cross-reference design §I3 / §I2):
///   * `execute_bidi` returning a `BidiSource` means the open
///     succeeded — the handler has already spawned its long-lived
///     task, both channels are live, and the IPC layer can install
///     the session row atomically.
///   * Exactly one `TerminalBidi` is ever emitted per session_id;
///     whichever cancel path closes a channel first wins, the
///     others observe EOF and no-op.
#[derive(Debug)]
pub struct BidiSource {
    /// Transport WRITE end. `SendBidi` frames push here; the
    /// handler's matching Receiver delivers them.
    pub to_client: mpsc::Sender<Value>,
    /// Transport READ end. The forwarder reads here and emits each
    /// value as `RecvBidi`; the handler's matching Sender is what
    /// produces them.
    pub from_client: mpsc::Receiver<Value>,
}

/// One in-process bidi handler. Per design §D2 the closure runs at
/// open time only: it builds the two channels, spawns its own
/// long-lived `tokio::spawn(...)` loop that owns the session, and
/// returns the `BidiSource` immediately. The dispatcher never
/// blocks waiting for a session loop, mirroring how
/// `register_stream`'s `Live` variant is already shaped.
pub type LocalBidiHandler = Arc<dyn Fn(Value) -> anyhow::Result<BidiSource> + Send + Sync>;

/// Envelope-aware bidi handler. Mirrors `LocalRpcHandlerWithEnvelope`.
pub type LocalBidiHandlerWithEnvelope =
    Arc<dyn Fn(EnvelopeContext, Value) -> anyhow::Result<BidiSource> + Send + Sync>;

/// Resolver consulted on a registry miss. Returns `Some(handler)`
/// when the resolver can synthesize one for the queried ability
/// (e.g. a `<agent>.<verb>` whose TOML was added to the workspace
/// after daemon boot — the dynamic per-agent fallback uses this
/// to discover newly-authored abilities at invoke time without
/// daemon restart). `None` keeps the legacy "not found" semantics.
///
/// One resolver per registry — the daemon owns this slot and uses
/// it for the agent-workspace path. Returning `Send + Sync` so the
/// registry stays clone-friendly on the Arc share.
pub type LocalFallbackResolver = Arc<dyn Fn(&str) -> Option<LocalRpcHandler> + Send + Sync>;

/// Local-ability registry. Keyed by full ability name. v1 shape is
/// a `BTreeMap` for deterministic iteration order; the registry
/// is read-mostly (built once at daemon start, queried per
/// invocation), so RwLock + per-invocation hash is overkill.
///
/// Hot-reload note: the registry itself is built once at boot, but
/// `rpc_fallback` lets a caller (the daemon's per-agent dispatcher)
/// answer lookup misses dynamically. That is the seam new
/// `<agent>.<verb>` abilities authored after boot use to become
/// invokable without a daemon restart — see `chat_ability::register`.
#[derive(Default)]
pub struct LocalAbilityRegistry {
    rpc: BTreeMap<String, LocalRpcHandler>,
    stream: BTreeMap<String, LocalStreamHandler>,
    bidi: BTreeMap<String, LocalBidiHandler>,
    rpc_fallback: Option<LocalFallbackResolver>,
    // ── Envelope-aware variants (PR-DISPATCHER-SUBJECT) ──────
    // Separate maps so legacy `register_*` callers stay on the
    // args-only signature (zero churn) and only abilities that
    // need envelope context opt in via the `_with_envelope`
    // family. Dispatcher consults these FIRST, falling back to
    // the args-only maps on miss; one ability MUST be in exactly
    // one map (registering both forms is rejected at boot via
    // debug_assert).
    rpc_with_env: BTreeMap<String, LocalRpcHandlerWithEnvelope>,
    stream_with_env: BTreeMap<String, LocalStreamHandlerWithEnvelope>,
    bidi_with_env: BTreeMap<String, LocalBidiHandlerWithEnvelope>,
}

impl std::fmt::Debug for LocalAbilityRegistry {
    /// Manual impl because the handler types are `Arc<dyn Fn>`
    /// trait objects which do not implement `Debug`. Surfaces just
    /// the registered ability counts + names per shape — enough for
    /// `OnceLock::set`'s `.expect(..)` to print a useful message
    /// without leaking handler addresses.
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("LocalAbilityRegistry")
            .field("rpc_count", &self.rpc.len())
            .field("stream_count", &self.stream.len())
            .field("bidi_count", &self.bidi.len())
            .field("rpc_with_env_count", &self.rpc_with_env.len())
            .field("stream_with_env_count", &self.stream_with_env.len())
            .field("bidi_with_env_count", &self.bidi_with_env.len())
            .field("has_rpc_fallback", &self.rpc_fallback.is_some())
            .finish()
    }
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

    /// Register a bidi handler under `ability`. Same single-writer
    /// model as `register_rpc` / `register_stream`.
    ///
    /// Per design §D2 the handler closure runs only once per session
    /// (at OpenBidi time): it must build the two `mpsc` channels,
    /// `tokio::spawn` its own session loop, and return the
    /// `BidiSource` immediately. Returning the source is the
    /// "session opened" signal — anything that can fail the open
    /// (registry lookup, validation, channel construction) must
    /// surface as `Err` from the closure so §I3 holds: a failed
    /// open never produces a half-live session.
    pub fn register_bidi(&mut self, ability: impl Into<String>, handler: LocalBidiHandler) {
        self.bidi.insert(ability.into(), handler);
    }

    /// Register an envelope-aware RPC handler. Used by abilities
    /// that need access to the AXIOM 7-tuple `subject` (per
    /// **INV-SUBJECT-ENVELOPE**) — typically media abilities
    /// resolving a `subject = resource_uri` to a local resource
    /// table entry. The handler closure signature is
    /// `Fn(EnvelopeContext, Value) -> Result<Value>`; the dispatcher
    /// passes the resolved `InvocationTarget.subject` in the
    /// context. Mutually exclusive with `register_rpc` per ability
    /// — registering both is a startup bug (caller picks one
    /// shape per ability).
    pub fn register_rpc_with_envelope(
        &mut self,
        ability: impl Into<String>,
        handler: LocalRpcHandlerWithEnvelope,
    ) {
        self.rpc_with_env.insert(ability.into(), handler);
    }

    /// Envelope-aware stream variant. See `register_rpc_with_envelope`
    /// for the rationale.
    pub fn register_stream_with_envelope(
        &mut self,
        ability: impl Into<String>,
        handler: LocalStreamHandlerWithEnvelope,
    ) {
        self.stream_with_env.insert(ability.into(), handler);
    }

    /// Envelope-aware bidi variant. See `register_rpc_with_envelope`
    /// for the rationale.
    pub fn register_bidi_with_envelope(
        &mut self,
        ability: impl Into<String>,
        handler: LocalBidiHandlerWithEnvelope,
    ) {
        self.bidi_with_env.insert(ability.into(), handler);
    }

    /// Lookup helper — exposed because PR-ATTACH onwards will need
    /// a way to introspect "what abilities does this daemon
    /// publish?" without reflecting through the dispatcher.
    ///
    /// Returns the union of RPC + stream + bidi ability names,
    /// sorted. Discovery callers should not see the call-mode
    /// distinction (a single ability is currently only registered
    /// under one call mode, but the union here keeps the list
    /// honest if a future ability legitimately exposes both shapes).
    pub fn list_abilities(&self) -> Vec<String> {
        let mut names: Vec<String> = self.rpc.keys().cloned().collect();
        // Envelope-aware variants are part of the same discovery
        // surface — meta.list_abilities should not care which
        // signature an ability registered under.
        for k in self.rpc_with_env.keys() {
            if !names.iter().any(|n| n == k) {
                names.push(k.clone());
            }
        }
        for k in self.stream.keys() {
            if !names.iter().any(|n| n == k) {
                names.push(k.clone());
            }
        }
        for k in self.stream_with_env.keys() {
            if !names.iter().any(|n| n == k) {
                names.push(k.clone());
            }
        }
        for k in self.bidi.keys() {
            if !names.iter().any(|n| n == k) {
                names.push(k.clone());
            }
        }
        for k in self.bidi_with_env.keys() {
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

    /// Owned-clone counterpart that consults the fallback resolver
    /// on a registry miss. Existing call sites that take `&Arc<...>`
    /// keep using `get_rpc`; the dispatcher's execute path uses this
    /// so a `<agent>.<verb>` written to disk post-boot is found via
    /// the fallback without forcing the registry to be mutable.
    pub fn resolve_rpc(&self, ability: &str) -> Option<LocalRpcHandler> {
        if let Some(h) = self.rpc.get(ability) {
            return Some(Arc::clone(h));
        }
        if let Some(resolver) = self.rpc_fallback.as_ref() {
            return resolver(ability);
        }
        None
    }

    /// Install the RPC fallback resolver. Called once by the daemon
    /// boot path after every static handler is in place. Replaces
    /// any prior resolver — single-writer registry semantics still
    /// hold; only the daemon installs this.
    pub fn set_rpc_fallback(&mut self, resolver: LocalFallbackResolver) {
        self.rpc_fallback = Some(resolver);
    }

    /// Returns Some when a stream handler is registered for `ability`.
    pub fn get_stream(&self, ability: &str) -> Option<&LocalStreamHandler> {
        self.stream.get(ability)
    }

    /// Returns Some when a bidi handler is registered for `ability`.
    pub fn get_bidi(&self, ability: &str) -> Option<&LocalBidiHandler> {
        self.bidi.get(ability)
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
            TargetScope::Local => {
                // Per PR-DISPATCHER-SUBJECT: envelope-aware
                // handlers take precedence so an ability that
                // opted into envelope access is never called via
                // the legacy args-only path. The args-only registry
                // is the fallback for legacy abilities.
                if let Some(handler) = self.local.rpc_with_env.get(&target.ability) {
                    let env = EnvelopeContext {
                        subject: target.subject,
                    };
                    return handler(env, target.normalized_args);
                }
                match self.local.resolve_rpc(&target.ability) {
                    Some(handler) => handler(target.normalized_args),
                    None => anyhow::bail!(
                        "no local handler registered for ability {} (loopback path)",
                        target.ability
                    ),
                }
            }
            TargetScope::Remote { node } => {
                // Joint-plan phase 4: TargetScope::Remote no longer
                // routes through GatewayApi (deleted along with
                // NoopGateway's invoke_remote_ability stub). Any
                // caller that still constructs a Remote target
                // should be migrated to
                // `support::federation_invoke::invoke_via_federation_forward`
                // — every CLI surface and EAL dispatcher already
                // did. Surface a typed error rather than silently
                // bouncing to `Local` so a regression that adds a
                // new Remote caller fails loud.
                let _ = node;
                anyhow::bail!(
                    "AbilityDispatcher::execute_rpc no longer accepts \
                     TargetScope::Remote; route through \
                     `federation.forward_invoke` (see \
                     `support::federation_invoke::invoke_via_federation_forward`)."
                )
            }
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
            TargetScope::Local => {
                if let Some(handler) = self.local.stream_with_env.get(&target.ability) {
                    let env = EnvelopeContext {
                        subject: target.subject,
                    };
                    return handler(env, target.normalized_args);
                }
                match self.local.get_stream(&target.ability) {
                    Some(handler) => handler(target.normalized_args),
                    None => anyhow::bail!(
                        "no local stream handler registered for ability {} (loopback path)",
                        target.ability
                    ),
                }
            }
            TargetScope::Remote { .. } => anyhow::bail!(
                "remote stream dispatch not yet wired in v1; \
                 lands once GatewayApi::subscribe_remote_ability is plumbed \
                 to forward into the IPC stream"
            ),
        }
    }

    /// Execute a Bidi-mode `InvocationTarget`. Returns a `BidiSource`
    /// holding both ends of the live session. The caller (IPC
    /// server) installs the session into the per-connection
    /// `BidiRegistry`, spawns the forwarder that pumps `to_client`
    /// into `RecvBidi` envelopes, and routes inbound `SendBidi`
    /// frames into `from_client`.
    ///
    /// Per §I3 atomicity: returning Ok(BidiSource) is the
    /// "session opened" signal. The handler closure has already
    /// spawned its long-lived loop; failure paths in the closure
    /// surface as `Err` here and the IPC layer must NOT install
    /// any session state.
    ///
    /// Remote bidi forwarding through GatewayApi is deferred for
    /// the same reason as remote stream — InvokeBidi over the
    /// federation hop needs Axon-side machinery (C-M5b/c/d) before
    /// it can forward through the IPC connection.
    pub fn execute_bidi(&self, target: InvocationTarget) -> anyhow::Result<BidiSource> {
        if target.call_mode != CallMode::Bidi {
            anyhow::bail!(
                "AbilityDispatcher::execute_bidi called with non-Bidi call_mode \
                 (got {:?}); use execute_rpc or execute_stream instead",
                target.call_mode
            );
        }
        match target.scope {
            TargetScope::Local => {
                if let Some(handler) = self.local.bidi_with_env.get(&target.ability) {
                    let env = EnvelopeContext {
                        subject: target.subject,
                    };
                    return handler(env, target.normalized_args);
                }
                match self.local.get_bidi(&target.ability) {
                    Some(handler) => handler(target.normalized_args),
                    None => anyhow::bail!(
                        "no local bidi handler registered for ability {} (loopback path)",
                        target.ability
                    ),
                }
            }
            TargetScope::Remote { .. } => anyhow::bail!(
                "remote bidi dispatch not yet wired in v1; \
                 lands once GatewayApi exposes a bidi forwarder over \
                 InvokeBidi (tracked by C-M5b/c/d)"
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
            subject: None,
        }
    }

    #[test]
    fn unregistered_local_ability_returns_clear_error() {
        // The error must name the ability so an operator can grep
        // "is observe.health registered?" against the daemon log.
        let dispatcher = AbilityDispatcher::new(empty_registry(), Arc::new(NoopGateway::new()));
        let err = dispatcher.execute_rpc(ping_target_local()).unwrap_err();
        let msg = format!("{err}");
        assert!(
            msg.contains("observe.health"),
            "error must name ability, got: {msg}"
        );
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

    // ── PR-DISPATCHER-SUBJECT envelope-aware handler tests ───

    #[test]
    fn envelope_aware_rpc_handler_receives_subject_from_target() {
        // The load-bearing test for INV-SUBJECT-ENVELOPE positive
        // half: handler registered via register_rpc_with_envelope
        // receives target.subject in EnvelopeContext, NOT via args.
        let mut reg = LocalAbilityRegistry::new();
        reg.register_rpc_with_envelope(
            "media.x.snapshot",
            Arc::new(|env: EnvelopeContext, _args: Value| {
                Ok(json!({
                    "saw_subject": env.subject,
                    "args_subject_was_present": false,
                }))
            }),
        );
        let dispatcher = AbilityDispatcher::new(Arc::new(reg), Arc::new(NoopGateway::new()));
        let target = InvocationTarget {
            scope: TargetScope::Local,
            ability: "media.x.snapshot".into(),
            normalized_args: json!({}),
            call_mode: CallMode::Rpc,
            subject: Some("easynet:///r/acme/resource/01CAM".into()),
        };
        let resp = dispatcher.execute_rpc(target).unwrap();
        assert_eq!(
            resp["saw_subject"],
            json!("easynet:///r/acme/resource/01CAM")
        );
    }

    #[test]
    fn envelope_aware_handler_takes_precedence_over_legacy_handler() {
        // If an ability is mistakenly registered under both shapes
        // (a programming error), the envelope-aware path wins. Pin
        // this so a future refactor that flipped precedence would
        // surface here rather than silently routing media handlers
        // through the args-only path that drops subject.
        let mut reg = LocalAbilityRegistry::new();
        reg.register_rpc(
            "x.dual",
            Arc::new(|_args: Value| Ok(json!({"path": "legacy"}))),
        );
        reg.register_rpc_with_envelope(
            "x.dual",
            Arc::new(|_env: EnvelopeContext, _args: Value| Ok(json!({"path": "envelope"}))),
        );
        let dispatcher = AbilityDispatcher::new(Arc::new(reg), Arc::new(NoopGateway::new()));
        let target = InvocationTarget {
            scope: TargetScope::Local,
            ability: "x.dual".into(),
            normalized_args: json!({}),
            call_mode: CallMode::Rpc,
            subject: None,
        };
        let resp = dispatcher.execute_rpc(target).unwrap();
        assert_eq!(resp, json!({"path": "envelope"}));
    }

    #[test]
    fn envelope_aware_handler_with_none_subject_still_dispatches() {
        // Legacy callers that don't set subject still reach the
        // envelope-aware handler — it just sees subject=None and
        // can decide what to do (fail with resource_not_found or
        // process anyway). The dispatcher does NOT reject the call
        // for missing subject; that's a per-handler policy.
        let mut reg = LocalAbilityRegistry::new();
        reg.register_rpc_with_envelope(
            "x.optional",
            Arc::new(|env: EnvelopeContext, _args: Value| {
                Ok(json!({"subject_was_none": env.subject.is_none()}))
            }),
        );
        let dispatcher = AbilityDispatcher::new(Arc::new(reg), Arc::new(NoopGateway::new()));
        let target = InvocationTarget {
            scope: TargetScope::Local,
            ability: "x.optional".into(),
            normalized_args: json!({}),
            call_mode: CallMode::Rpc,
            subject: None,
        };
        let resp = dispatcher.execute_rpc(target).unwrap();
        assert_eq!(resp, json!({"subject_was_none": true}));
    }

    #[test]
    fn envelope_aware_stream_handler_receives_subject() {
        let mut reg = LocalAbilityRegistry::new();
        reg.register_stream_with_envelope(
            "x.subscribe",
            Arc::new(|env: EnvelopeContext, _args: Value| {
                let frame = json!({"subject_seen": env.subject});
                Ok(StreamSource::Snapshot(vec![frame]))
            }),
        );
        let dispatcher = AbilityDispatcher::new(Arc::new(reg), Arc::new(NoopGateway::new()));
        let target = InvocationTarget {
            scope: TargetScope::Local,
            ability: "x.subscribe".into(),
            normalized_args: json!({}),
            call_mode: CallMode::Stream,
            subject: Some("easynet:///r/x/resource/01MIC".into()),
        };
        let src = dispatcher.execute_stream(target).unwrap();
        match src {
            StreamSource::Snapshot(frames) => {
                assert_eq!(frames.len(), 1);
                assert_eq!(
                    frames[0]["subject_seen"],
                    json!("easynet:///r/x/resource/01MIC")
                );
            }
            other => panic!("expected Snapshot; got {other:?}"),
        }
    }

    #[test]
    fn list_abilities_includes_envelope_aware_registrations() {
        // Discovery must see env-aware handlers — meta.list_abilities
        // and gen-ability-tomls iterate this list, and a handler
        // registered only via register_rpc_with_envelope MUST appear.
        let mut reg = LocalAbilityRegistry::new();
        reg.register_rpc_with_envelope("x.env_only", Arc::new(|_env, _args| Ok(json!({}))));
        let names = reg.list_abilities();
        assert!(
            names.iter().any(|n| n == "x.env_only"),
            "envelope-aware ability missing from list_abilities: {names:?}"
        );
    }

    #[test]
    fn remote_target_returns_unified_path_redirect() {
        // Joint-plan phase 4: `TargetScope::Remote` no longer
        // routes through GatewayApi (deleted along with
        // NoopGateway's invoke_remote_ability stub). Cross-device
        // dispatch flows through
        // `support::federation_invoke::invoke_via_federation_forward`
        // instead. The dispatcher surfaces a typed error
        // pointing the caller at the new path so a stale Remote
        // construction fails loud instead of silently bouncing
        // to Local.
        let dispatcher = AbilityDispatcher::new(empty_registry(), Arc::new(NoopGateway::new()));
        let target = InvocationTarget {
            scope: TargetScope::Remote {
                node: NodeId::new("peer"),
            },
            ability: "observe.health".into(),
            normalized_args: json!({}),
            call_mode: CallMode::Rpc,
            subject: None,
        };
        let err = dispatcher.execute_rpc(target).unwrap_err();
        let msg = format!("{err}");
        assert!(
            msg.contains("federation.forward_invoke") || msg.contains("federation_invoke"),
            "error must redirect to the unified path, got: {msg}"
        );
    }

    #[test]
    fn stream_call_mode_rejected_at_rpc_path() {
        // A handler asking the RPC executor to dispatch a stream
        // mode is calling the wrong method. Returning a clear
        // error catches the misuse at the call site instead of
        // silently degrading to an RPC return.
        let dispatcher = AbilityDispatcher::new(empty_registry(), Arc::new(NoopGateway::new()));
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
        let dispatcher = AbilityDispatcher::new(empty_registry(), Arc::new(NoopGateway::new()));
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
        let dispatcher = AbilityDispatcher::new(empty_registry(), Arc::new(NoopGateway::new()));
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

    // ── register_bidi / get_bidi ─────────────────────────────────

    /// Build a trivial bidi handler that immediately constructs both
    /// channels and returns a `BidiSource` without spawning a real
    /// session loop. Sufficient for registry-level tests where we
    /// only need to observe whether the dispatcher reached the
    /// closure. Production handlers spawn a tokio task that owns
    /// the loop — that path is tested at the IPC layer (commit 4).
    fn trivial_bidi_handler() -> LocalBidiHandler {
        Arc::new(|_args: Value| {
            let (_to_handler_tx, from_client) = mpsc::channel::<Value>(BIDI_CHANNEL_BOUND);
            let (to_client, _to_client_rx) = mpsc::channel::<Value>(BIDI_CHANNEL_BOUND);
            Ok(BidiSource {
                from_client,
                to_client,
            })
        })
    }

    #[test]
    fn register_bidi_makes_ability_dispatchable() {
        // Symmetric to `register_rpc` / `register_stream`: after
        // registration, a get_bidi lookup must surface the handler.
        // Pin the round-trip so a typo (e.g. inserting into
        // `self.stream` instead of `self.bidi`) trips a test.
        let mut reg = LocalAbilityRegistry::new();
        assert!(reg.get_bidi("fleet.session_attach").is_none());
        reg.register_bidi("fleet.session_attach", trivial_bidi_handler());
        assert!(reg.get_bidi("fleet.session_attach").is_some());
        // Negative: not visible on the other call modes.
        assert!(reg.get_rpc("fleet.session_attach").is_none());
        assert!(reg.get_stream("fleet.session_attach").is_none());
    }

    #[test]
    fn list_abilities_includes_bidi_keys_in_sorted_union() {
        // §A12 / §1.3 discovery surfaces (and the future
        // meta.list_abilities ability) project this list verbatim,
        // so a missing call mode would silently hide bidi-only
        // abilities from clients.
        let mut reg = LocalAbilityRegistry::new();
        reg.register_rpc("observe.health", Arc::new(|_| Ok(Value::Null)));
        reg.register_stream(
            "permission.subscribe",
            Arc::new(|_| Ok(StreamSource::Snapshot(vec![]))),
        );
        reg.register_bidi("fleet.session_attach", trivial_bidi_handler());
        assert_eq!(
            reg.list_abilities(),
            vec![
                "fleet.session_attach",
                "observe.health",
                "permission.subscribe",
            ],
        );
    }

    #[test]
    fn execute_bidi_rejects_non_bidi_call_mode() {
        // Symmetric to the rejections execute_rpc / execute_stream
        // perform in commit 1. A misroute that sends an RPC target
        // through the bidi executor would silently allocate channels
        // and never receive a frame; the bail catches that at the
        // call site.
        let dispatcher = AbilityDispatcher::new(empty_registry(), Arc::new(NoopGateway::new()));
        let t = ping_target_local(); // call_mode = Rpc
        let err = dispatcher.execute_bidi(t).unwrap_err();
        assert!(format!("{err}").contains("Bidi"));
    }

    #[test]
    fn execute_bidi_returns_handler_source_on_local_target() {
        // The dispatcher must reach the registered handler and
        // surface its BidiSource verbatim. We assert by sending one
        // frame from the test (acting as IPC server) to the handler-
        // facing receiver; if the dispatcher returned the wrong
        // half the recv would never arrive.
        let mut reg = LocalAbilityRegistry::new();

        // A handler that owns its own loop reading from_client and
        // echoing into to_client. Spawned inside the closure per §D2.
        reg.register_bidi(
            "fleet.echo",
            Arc::new(|_args: Value| {
                let (client_to_handler_tx, mut client_to_handler_rx) =
                    mpsc::channel::<Value>(BIDI_CHANNEL_BOUND);
                let (handler_to_client_tx, handler_to_client_rx) =
                    mpsc::channel::<Value>(BIDI_CHANNEL_BOUND);
                // Forwarder side of the BidiSource is what we hand
                // back to the caller — it sees the *handler input*
                // sender (so it can push frames in) and the handler
                // output receiver (so it can pump them out). The
                // handler keeps the opposite ends.
                tokio::spawn(async move {
                    while let Some(v) = client_to_handler_rx.recv().await {
                        if handler_to_client_tx.send(v).await.is_err() {
                            break;
                        }
                    }
                });
                Ok(BidiSource {
                    from_client: handler_to_client_rx, // misnamed for test brevity
                    to_client: client_to_handler_tx,
                })
            }),
        );
        let dispatcher = AbilityDispatcher::new(Arc::new(reg), Arc::new(NoopGateway::new()));
        let target = InvocationTarget {
            scope: TargetScope::Local,
            ability: "fleet.echo".into(),
            normalized_args: json!({}),
            call_mode: CallMode::Bidi,
            subject: None,
        };

        let rt = tokio::runtime::Builder::new_current_thread()
            .enable_all()
            .build()
            .unwrap();
        rt.block_on(async {
            let mut src = dispatcher.execute_bidi(target).expect("execute_bidi ok");
            // Push one frame in via to_client (the "handler input"
            // half on this test fixture) and pull the echo back out
            // via from_client. End-to-end through the spawned loop.
            src.to_client.send(json!({"hello": 1})).await.unwrap();
            let echoed = src.from_client.recv().await.expect("echo arrives");
            assert_eq!(echoed, json!({"hello": 1}));
        });
    }

    #[test]
    fn execute_bidi_unregistered_ability_returns_clear_error() {
        // Mirror unregistered_local_ability_returns_clear_error for
        // bidi. The error must name the ability so an operator can
        // grep for it.
        let dispatcher = AbilityDispatcher::new(empty_registry(), Arc::new(NoopGateway::new()));
        let target = InvocationTarget {
            scope: TargetScope::Local,
            ability: "fleet.session_attach".into(),
            normalized_args: json!({}),
            call_mode: CallMode::Bidi,
            subject: None,
        };
        let err = dispatcher.execute_bidi(target).unwrap_err();
        let msg = format!("{err}");
        assert!(msg.contains("fleet.session_attach"), "names ability: {msg}");
        assert!(msg.contains("bidi"), "indicates bidi mode: {msg}");
    }

    #[test]
    fn execute_bidi_handler_failure_propagates_no_session_artifacts() {
        // §I3 atomicity: a handler whose construction fails must
        // surface as Err from execute_bidi, with no half-open
        // BidiSource leaking out. There is no "partial source" to
        // assert against — the success type is `BidiSource`, so the
        // type system prevents that — but we do pin that the error
        // message preserves the handler's reason rather than being
        // swallowed by a generic dispatcher message.
        let mut reg = LocalAbilityRegistry::new();
        reg.register_bidi(
            "fleet.bad",
            Arc::new(|_| anyhow::bail!("intentional handler failure: precondition foo missing")),
        );
        let dispatcher = AbilityDispatcher::new(Arc::new(reg), Arc::new(NoopGateway::new()));
        let target = InvocationTarget {
            scope: TargetScope::Local,
            ability: "fleet.bad".into(),
            normalized_args: json!({}),
            call_mode: CallMode::Bidi,
            subject: None,
        };
        let err = dispatcher.execute_bidi(target).unwrap_err();
        assert!(format!("{err}").contains("precondition foo missing"));
    }

    #[test]
    fn execute_bidi_remote_target_bails_until_gateway_supports_it() {
        // Remote bidi forwarding is deferred (C-M5b/c/d). The bail
        // here keeps a misroute from silently degrading to a local
        // lookup or panicking on a missing gateway method; pin it
        // so a later refactor can't drop the guard.
        let dispatcher = AbilityDispatcher::new(empty_registry(), Arc::new(NoopGateway::new()));
        let target = InvocationTarget {
            scope: TargetScope::Remote {
                node: NodeId::new("01PEER"),
            },
            ability: "fleet.session_attach".into(),
            normalized_args: json!({}),
            call_mode: CallMode::Bidi,
            subject: None,
        };
        let err = dispatcher.execute_bidi(target).unwrap_err();
        assert!(format!("{err}").to_lowercase().contains("remote"));
    }

    #[test]
    fn bidi_channel_bound_matches_writer_queue_size() {
        // Pin the constant. Per §D1 the bidi channel bound is the
        // same as the per-connection IPC writer queue — they're set
        // together so a saturated session cannot exceed the writer's
        // backlog. A change to one without the other would create
        // an asymmetry that's invisible until a sustained burst.
        assert_eq!(BIDI_CHANNEL_BOUND, 256);
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
