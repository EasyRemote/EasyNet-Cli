// EasyNet CLI — Axon ability catalogue
// ====================================
//
// File: src/runtime/ability_dispatch.rs
// Description: Registration and metadata surface for daemon-hosted
//              Axon abilities. `AxonAbilityCatalog` preserves the
//              existing module-level `register(&mut catalog)` API,
//              but every registered handler is written through to
//              `easynet_axon::invocation::LocalRuntime` when the
//              daemon builds the catalogue. Production invocation
//              paths execute through that runtime; direct catalogue
//              execution helpers are test-only compatibility probes.
//
// Author: Silan Hu <silan.hu@u.nus.edu>
// Copyright (c) 2026 EasyNet. All rights reserved.

use std::collections::BTreeMap;
use std::future::Future;
use std::sync::Arc;

use easynet_axon::invocation::{
    make_ability, AbilityCallModes, AbilityContext, AbilityFn, AbilityOptions, AxonError,
    CallMode as AxonCallMode, LocalRuntime,
};
use serde_json::Value;
use tokio::sync::{broadcast, mpsc};

use crate::runtime::invocation_target::{CallMode, InvocationTarget, TargetScope};

/// Module-local sync→async bridge for the ability-dispatch registry
/// path. These calls sit on catalogue construction and discovery,
/// not per-frame dispatch, so correctness under all runtime hosts is
/// more important than the cheapest possible no-runtime fallback.
///
/// In particular, feature-expanded registration can enter SDK-backed
/// paths that rely on tokio wakeups. Driving those futures with
/// `futures::executor::block_on` from a current-thread tokio test
/// runtime can park forever. Use the tokio fallback so single-thread
/// callers are offloaded to a fresh helper runtime instead of
/// re-entering or starving the active one.
fn block_on_runtime_sync<F>(future: F) -> F::Output
where
    F: Future + Send,
    F::Output: Send,
{
    crate::support::async_bridge::run_blocking(
        future,
        crate::support::async_bridge::NoRuntimeFallback::BuildCurrentThreadTokio,
    )
}

/// One in-process RPC handler. Boxed closure so the registry can
/// hold heterogeneous handlers behind a uniform key.
pub type LocalRpcHandler = Arc<dyn Fn(Value) -> anyhow::Result<Value> + Send + Sync>;

/// Slice of the AXIOM 7-tuple that an envelope-aware handler needs
/// access to. Sidecar plugins consume this whole value to avoid creating
/// a second, incomplete `ability + args` invoke primitive outside Axon.
///
/// Per **INV-SUBJECT-ENVELOPE**: this is the ONLY way a handler
/// reads its `subject`. Handlers MUST NOT accept `subject` in
/// `args`; the `register_*_with_envelope` family of methods is
/// the way for handlers to opt into envelope access.
#[derive(Debug, Clone, Default)]
pub struct EnvelopeContext {
    /// AXIOM 7-tuple `caller`.
    pub caller: Option<String>,
    /// AXIOM 7-tuple `callee`.
    pub callee: Option<String>,
    /// AXIOM 7-tuple `ability`.
    pub ability: Option<String>,
    /// AXIOM 7-tuple `subject`. `None` for legacy abilities and
    /// for the degenerate `subject = callee` case (per
    /// INV-META-SUBJECT-EXEMPT). Resource handlers MUST treat
    /// `None` as a missing-subject failure (`resource_not_found`
    /// or InvalidArgument).
    pub subject: Option<String>,
    /// AXIOM 7-tuple `nonce`.
    pub invocation_nonce: Option<Vec<u8>>,
    /// Host-side projection of AXIOM `causal_context`. This is not a
    /// canonical encoder; canonical receipt semantics stay in Axon.
    pub causal_context: Option<Value>,
}

/// Envelope-aware RPC handler. The runtime adapter passes a snapshot of
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
    pub from_client: mpsc::Receiver<BidiOutputFrame>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct BidiOutputFrame {
    pub payload: Vec<u8>,
    pub content_type: String,
}

impl BidiOutputFrame {
    pub fn json(value: Value) -> Self {
        Self {
            payload: serde_json::to_vec(&value)
                .expect("serde_json::Value serialization should not fail"),
            content_type: "application/json".to_string(),
        }
    }

    pub fn binary(payload: impl Into<Vec<u8>>, content_type: impl Into<String>) -> Self {
        Self {
            payload: payload.into(),
            content_type: content_type.into(),
        }
    }

    pub fn into_json_value(self) -> anyhow::Result<Value> {
        if self.payload.is_empty() {
            return Ok(Value::Object(Default::default()));
        }
        serde_json::from_slice(&self.payload)
            .map_err(|err| anyhow::anyhow!("bidi output frame is not JSON: {err}"))
    }
}

/// One in-process bidi handler. Per design §D2 the closure runs at
/// open time only: it builds the two channels, spawns its own
/// long-lived `tokio::spawn(...)` loop that owns the session, and
/// returns the `BidiSource` immediately. The catalogue adapter never
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

#[derive(Clone, Default)]
struct RuntimeHandlerSet {
    rpc: Option<LocalRpcHandler>,
    stream: Option<LocalStreamHandler>,
    bidi: Option<LocalBidiHandler>,
    rpc_with_env: Option<LocalRpcHandlerWithEnvelope>,
    stream_with_env: Option<LocalStreamHandlerWithEnvelope>,
    bidi_with_env: Option<LocalBidiHandlerWithEnvelope>,
}

impl RuntimeHandlerSet {
    fn modes(&self) -> AbilityCallModes {
        AbilityCallModes {
            rpc: self.rpc.is_some() || self.rpc_with_env.is_some(),
            stream: self.stream.is_some() || self.stream_with_env.is_some(),
            bidi: self.bidi.is_some() || self.bidi_with_env.is_some(),
        }
    }
}

fn payload_to_json_value(payload: &[u8]) -> Result<Value, AxonError> {
    if payload.is_empty() {
        Ok(Value::Object(Default::default()))
    } else {
        serde_json::from_slice(payload).map_err(|err| {
            AxonError::invalid_argument(format!("local_runtime_adapter: payload not JSON: {err}"))
        })
    }
}

fn json_value_to_payload(value: &Value) -> Result<Vec<u8>, AxonError> {
    serde_json::to_vec(value)
        .map_err(|err| AxonError::internal(format!("local_runtime_adapter: encode JSON: {err}")))
}

pub(crate) fn rpc_handler_to_ability_fn(handler: LocalRpcHandler) -> AbilityFn {
    make_ability(move |ctx| {
        let handler = Arc::clone(&handler);
        let payload = ctx.payload.clone();
        async move {
            let value = payload_to_json_value(&payload)?;
            let result = tokio::task::spawn_blocking(move || handler(value))
                .await
                .map_err(|err| {
                    AxonError::internal(format!("local_runtime_adapter: handler join error: {err}"))
                })?
                .map_err(|err| {
                    AxonError::internal(format!(
                        "local_runtime_adapter: handler returned error: {err}"
                    ))
                })?;
            json_value_to_payload(&result)
        }
    })
}

async fn envelope_context_from_axon(ctx: &Arc<AbilityContext>) -> EnvelopeContext {
    ctx.runtime
        .axiom_envelope_of(&ctx.invocation_id)
        .await
        .map(|signed| EnvelopeContext {
            caller: Some(signed.envelope.caller.ura),
            callee: Some(signed.envelope.callee.ura),
            ability: Some(signed.envelope.ability),
            subject: Some(signed.envelope.subject.ura),
            invocation_nonce: Some(signed.envelope.invocation_nonce.to_vec()),
            causal_context: Some(causal_context_to_json(&signed.envelope.causal_context)),
        })
        .unwrap_or_default()
}

fn causal_context_to_json(causal: &easynet_axon::invocation::CausalContext) -> serde_json::Value {
    match causal {
        easynet_axon::invocation::CausalContext::None => {
            serde_json::json!({"kind": "none"})
        }
        easynet_axon::invocation::CausalContext::Scalar(receipt) => serde_json::json!({
            "kind": "scalar",
            "receipt_hash": hex::encode(receipt.receipt_hash),
            "receipt_ura": receipt.receipt_ura,
        }),
        easynet_axon::invocation::CausalContext::List(receipts) => {
            let receipts: Vec<_> = receipts
                .iter()
                .map(|receipt| {
                    serde_json::json!({
                        "receipt_hash": hex::encode(receipt.receipt_hash),
                        "receipt_ura": receipt.receipt_ura,
                    })
                })
                .collect();
            serde_json::json!({"kind": "list", "receipts": receipts})
        }
        easynet_axon::invocation::CausalContext::Merkle { root, proof_ura } => {
            serde_json::json!({
                "kind": "merkle",
                "root": hex::encode(root),
                "proof_ura": proof_ura,
            })
        }
    }
}

fn rpc_env_handler_to_ability_fn(handler: LocalRpcHandlerWithEnvelope) -> AbilityFn {
    make_ability(move |ctx| {
        let handler = Arc::clone(&handler);
        async move {
            let value = payload_to_json_value(&ctx.payload)?;
            let env = envelope_context_from_axon(&ctx).await;
            let result = tokio::task::spawn_blocking(move || handler(env, value))
                .await
                .map_err(|err| {
                    AxonError::internal(format!(
                        "local_runtime_adapter: env handler join error: {err}"
                    ))
                })?
                .map_err(|err| {
                    AxonError::internal(format!(
                        "local_runtime_adapter: env handler returned error: {err}"
                    ))
                })?;
            json_value_to_payload(&result)
        }
    })
}

async fn emit_json_progress(ctx: &Arc<AbilityContext>, value: Value) -> Result<(), AxonError> {
    let payload = json_value_to_payload(&value)?;
    ctx.emit_progress(payload, "application/json").await
}

async fn emit_stream_source(
    ctx: Arc<AbilityContext>,
    source: StreamSource,
) -> Result<Vec<u8>, AxonError> {
    match source {
        StreamSource::Snapshot(frames) => {
            for frame in frames {
                emit_json_progress(&ctx, frame).await?;
            }
        }
        StreamSource::Live(mut rx) => loop {
            match rx.recv().await {
                Ok(frame) => emit_json_progress(&ctx, frame).await?,
                Err(tokio::sync::broadcast::error::RecvError::Closed) => break,
                Err(tokio::sync::broadcast::error::RecvError::Lagged(n)) => {
                    return Err(AxonError::internal(format!(
                        "local_runtime_adapter: stream lagged by {n} frame(s)"
                    )));
                }
            }
        },
        StreamSource::SnapshotThenLive(frames, mut rx) => {
            for frame in frames {
                emit_json_progress(&ctx, frame).await?;
            }
            loop {
                match rx.recv().await {
                    Ok(frame) => emit_json_progress(&ctx, frame).await?,
                    Err(tokio::sync::broadcast::error::RecvError::Closed) => break,
                    Err(tokio::sync::broadcast::error::RecvError::Lagged(n)) => {
                        return Err(AxonError::internal(format!(
                            "local_runtime_adapter: stream lagged by {n} frame(s)"
                        )));
                    }
                }
            }
        }
    }
    Ok(Vec::new())
}

fn stream_handler_to_ability_fn(handler: LocalStreamHandler) -> AbilityFn {
    make_ability(move |ctx| {
        let handler = Arc::clone(&handler);
        async move {
            let value = payload_to_json_value(&ctx.payload)?;
            let source = tokio::task::spawn_blocking(move || handler(value))
                .await
                .map_err(|err| {
                    AxonError::internal(format!(
                        "local_runtime_adapter: stream handler join error: {err}"
                    ))
                })?
                .map_err(|err| {
                    AxonError::internal(format!(
                        "local_runtime_adapter: stream handler returned error: {err}"
                    ))
                })?;
            emit_stream_source(ctx, source).await
        }
    })
}

fn stream_env_handler_to_ability_fn(handler: LocalStreamHandlerWithEnvelope) -> AbilityFn {
    make_ability(move |ctx| {
        let handler = Arc::clone(&handler);
        async move {
            let value = payload_to_json_value(&ctx.payload)?;
            let env = envelope_context_from_axon(&ctx).await;
            let source = tokio::task::spawn_blocking(move || handler(env, value))
                .await
                .map_err(|err| {
                    AxonError::internal(format!(
                        "local_runtime_adapter: env stream handler join error: {err}"
                    ))
                })?
                .map_err(|err| {
                    AxonError::internal(format!(
                        "local_runtime_adapter: env stream handler returned error: {err}"
                    ))
                })?;
            emit_stream_source(ctx, source).await
        }
    })
}

async fn run_bidi_source(
    ctx: Arc<AbilityContext>,
    source: BidiSource,
) -> Result<Vec<u8>, AxonError> {
    let BidiSource {
        to_client,
        mut from_client,
    } = source;
    let mut to_client = Some(to_client);
    loop {
        if to_client.is_none() {
            match from_client.recv().await {
                Some(frame) => {
                    ctx.emit_progress(frame.payload, frame.content_type).await?;
                }
                None => break,
            }
            continue;
        }

        tokio::select! {
            inbound = ctx.recv_message(None) => {
                match inbound {
                    Some(msg) => {
                        let value = payload_to_json_value(&msg.payload)?;
                        let send_closed = match to_client.as_ref() {
                            Some(sender) => sender.send(value).await.is_err(),
                            None => false,
                        };
                        if send_closed {
                            to_client = None;
                        }
                    }
                    None => {
                        to_client = None;
                    }
                }
            }
            outbound = from_client.recv() => {
                match outbound {
                    Some(frame) => ctx.emit_progress(frame.payload, frame.content_type).await?,
                    None => break,
                }
            }
        }
    }
    Ok(Vec::new())
}

fn bidi_handler_to_ability_fn(handler: LocalBidiHandler) -> AbilityFn {
    make_ability(move |ctx| {
        let handler = Arc::clone(&handler);
        async move {
            let value = payload_to_json_value(&ctx.payload)?;
            let source = handler(value).map_err(|err| {
                AxonError::internal(format!(
                    "local_runtime_adapter: bidi handler returned error: {err}"
                ))
            })?;
            run_bidi_source(ctx, source).await
        }
    })
}

fn bidi_env_handler_to_ability_fn(handler: LocalBidiHandlerWithEnvelope) -> AbilityFn {
    make_ability(move |ctx| {
        let handler = Arc::clone(&handler);
        async move {
            let value = payload_to_json_value(&ctx.payload)?;
            let env = envelope_context_from_axon(&ctx).await;
            let source = handler(env, value).map_err(|err| {
                AxonError::internal(format!(
                    "local_runtime_adapter: env bidi handler returned error: {err}"
                ))
            })?;
            run_bidi_source(ctx, source).await
        }
    })
}

fn runtime_handler_set_to_ability_fn(name: String, handlers: RuntimeHandlerSet) -> AbilityFn {
    let rpc_fn = handlers.rpc.map(rpc_handler_to_ability_fn);
    let stream_fn = handlers.stream.map(stream_handler_to_ability_fn);
    let bidi_fn = handlers.bidi.map(bidi_handler_to_ability_fn);
    let rpc_env_fn = handlers.rpc_with_env.map(rpc_env_handler_to_ability_fn);
    let stream_env_fn = handlers
        .stream_with_env
        .map(stream_env_handler_to_ability_fn);
    let bidi_env_fn = handlers.bidi_with_env.map(bidi_env_handler_to_ability_fn);

    make_ability(move |ctx| {
        let mode = ctx.call_mode;
        let name = name.clone();
        let handler = match mode {
            AxonCallMode::Rpc => rpc_env_fn.clone().or_else(|| rpc_fn.clone()),
            AxonCallMode::Stream => stream_env_fn.clone().or_else(|| stream_fn.clone()),
            AxonCallMode::Bidi => bidi_env_fn.clone().or_else(|| bidi_fn.clone()),
        };
        async move {
            match handler {
                Some(handler) => handler(ctx).await,
                None => Err(AxonError::invalid_argument(format!(
                    "local_runtime_adapter: ability {name} does not support {} mode",
                    mode.as_str()
                ))),
            }
        }
    })
}

/// What kind of actor owns the ability — the AXIOM seven-tuple
/// `callee` form for this verb.
///
/// Per the owner-truth-table spec
/// (`docs/spec/owner-truth-table/ability-owner-truth-table.tex`)
/// every registered ability falls into exactly one of these
/// categories. The registry stores it alongside the handler so
/// downstream consumers (`meta.list_abilities` synth, advertise
/// prelude, CLI render layer) can read owner-kind without
/// sniffing the name string.
///
/// **M0 of the system-namespace migration (RFC-001 v4.1.6 carrier).**
/// Before M0 the registry was keyed only on the name; meta_ability
/// derived owner via `name.starts_with("01HUB.")` and friends, and
/// the session-prelude algorithm derived "agent identity" from
/// `name.split_once('.')`. Both are flat-namespace conflations that
/// shipped a P0 regression on the Frontend Agents page (29 fake
/// agents from 24 system namespaces). This enum is the structural
/// fix — owner is declared at registration, not inferred from name.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum OwnerKind {
    /// Hosted by THIS device's daemon directly. Examples (terminal
    /// state): `device.fs.read`, `device.node.list`,
    /// `device.keyring.sign`, `device.session`, `device.invoke_remote`.
    Device,
    /// Hosted by the realm hub (federation-tier). Examples (terminal
    /// state): `hub.openai.chat_completions`, `hub.openai.list_models`.
    /// The handler may execute on the device daemon as a hub-local
    /// proxy, but the protocol owner is the hub.
    Hub,
    /// Hosted by a sub-agent on this device. The contained string is
    /// the sub-agent's `agent_id` (e.g. `"codex"`, `"web-builder"`,
    /// `"consent"`). The full owner URA is
    /// `easynet:///r/<realm>/agent/<user-uuid>.<agent_id>`; the
    /// realm + user are read from credentials at advertise time.
    Agent(String),
    /// Hosted by a user (the daemon's pages-user / canonical user).
    /// The contained string is the user-id slot used at registration
    /// time — the slug from `credentials.username` today; will
    /// transition to `credentials.user_id` (UUID) per Q4 of the
    /// truth-table spec. Example (terminal state):
    /// `<user-uuid>.api_key.create`.
    User(String),
}

/// Axon ability catalogue. Keyed by full ability name. v1 shape is
/// a `BTreeMap` for deterministic iteration order; the catalogue is
/// read-mostly (built once at daemon start, queried for metadata), so
/// RwLock + per-invocation hash is overkill.
///
/// Hot-reload note: the static maps below are written at boot, then
/// frozen behind `Arc<AxonAbilityCatalog>`. Post-boot mutation goes
/// through `dynamic_ext` — an interior-mutability side table fed by
/// `RegistryRefreshSink` when an upstream MCP server emits
/// `notifications/tools/list_changed`. Lookups (`resolve_rpc`,
/// `has_rpc`, …) consult the static maps first and fall through to
/// `dynamic_ext` on miss; the dynamic side stays optional so a
/// daemon that never hot-reloads (no MCP upstreams, or none that
/// support list-changed) pays nothing beyond a single empty-RwLock
/// read per miss.
#[derive(Default)]
pub struct AxonAbilityCatalog {
    /// Shared Axon runtime that owns the live invocation surface.
    ///
    /// During the Axon migration this catalogue remains the metadata
    /// construction API used by the existing `register(...)` modules,
    /// but handler registration is written through to `LocalRuntime`
    /// immediately. `new()` creates an isolated runtime for tests and
    /// local fixtures; daemon boot passes the process-wide runtime via
    /// `new_with_runtime` so no secondary synchronization pass is
    /// needed.
    runtime: Option<Arc<LocalRuntime>>,
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
    /// Owner kind per ability name. Keyed identically to the six
    /// handler maps above (a name lives in exactly one handler map
    /// AND in exactly one entry here). M0 of the system-namespace
    /// migration: every register call records the owner here so
    /// downstream consumers stop sniffing the name string. Legacy
    /// `register_rpc` / `register_stream` / `register_bidi` /
    /// `register_*_with_envelope` shims default to `OwnerKind::Device`
    /// — the safe choice for the bulk of today's catalogue, since
    /// 80%+ of system abilities are device-bundle. Per-call sites
    /// migrate to the `_with_owner` variants commit-by-commit; the
    /// shims are deleted at M0 commit 6.
    owner: BTreeMap<String, OwnerKind>,
    /// Optional `AbilityManifest` per registered ability — the
    /// authoritative description + JSON Schema for the verb. Set
    /// only by the `_with_spec` register family; legacy
    /// `_with_owner` / shim register sites leave the slot empty
    /// and the descriptor synthesis falls back to a name-only
    /// stub. The Frontend `InvokeAbilityDialog` renders a
    /// SchemaForm when the catalogue surfaces an `input_schema`
    /// and a free-text JSON box otherwise; the registry is the
    /// single source of truth that determines which path the UI
    /// gets, so plumbing manifests in here is the structural fix
    /// for "no declared schema" appearing on abilities that DO
    /// have a manifest in `core::ability_spec`.
    manifests: BTreeMap<String, Arc<crate::core::ability_spec::AbilityManifest>>,
    /// Hot-reload side table. Populated by `RegistryRefreshSink` when
    /// an upstream MCP server pushes `notifications/tools/list_changed`.
    /// Lookups fall through here on static-map miss; `list_abilities`
    /// unions both sides. Static maps remain immutable after boot —
    /// that keeps the hot path lock-free for every ability that was
    /// already known at boot.
    dynamic_ext: std::sync::RwLock<DynamicCatalogue>,
}

/// Post-boot ability additions. Same shape as the six handler maps
/// above plus owner/manifest, but mutated through `&self` via the
/// enclosing RwLock so the hot-reload sink can write while the
/// daemon holds the registry behind `Arc<AxonAbilityCatalog>`.
///
/// We do not merge `dynamic_ext` into the static maps lazily —
/// keeping the two sides separate means the hot path's miss
/// detection stays a cheap `BTreeMap::get` rather than a guard
/// acquisition on every dispatch.
#[derive(Default)]
struct DynamicCatalogue {
    rpc: BTreeMap<String, LocalRpcHandler>,
    stream: BTreeMap<String, LocalStreamHandler>,
    bidi: BTreeMap<String, LocalBidiHandler>,
    rpc_with_env: BTreeMap<String, LocalRpcHandlerWithEnvelope>,
    stream_with_env: BTreeMap<String, LocalStreamHandlerWithEnvelope>,
    bidi_with_env: BTreeMap<String, LocalBidiHandlerWithEnvelope>,
    owner: BTreeMap<String, OwnerKind>,
    manifests: BTreeMap<String, Arc<crate::core::ability_spec::AbilityManifest>>,
}

impl DynamicCatalogue {
    /// Drop every dynamic-side trace of `ability` across the eight
    /// maps and return whether any of them carried it. Names the
    /// "a key is in at most one map but may be in any" invariant in
    /// one place so the three sites that previously repeated the
    /// 8-way walk (`AxonAbilityCatalog::unregister`, `hot_unregister`,
    /// future hot-reload sinks) cannot drift independently.
    fn drain(&mut self, ability: &str) -> bool {
        let present = self.contains(ability);
        self.rpc.remove(ability);
        self.stream.remove(ability);
        self.bidi.remove(ability);
        self.rpc_with_env.remove(ability);
        self.stream_with_env.remove(ability);
        self.bidi_with_env.remove(ability);
        self.owner.remove(ability);
        self.manifests.remove(ability);
        present
    }

    /// Handler-only presence test (excludes `owner` and `manifests`
    /// because those are metadata side-tables — they exist iff a
    /// handler exists, but a stale row in either should not pin
    /// `has_dynamic` to true).
    fn contains_handler(&self, ability: &str) -> bool {
        self.rpc.contains_key(ability)
            || self.stream.contains_key(ability)
            || self.bidi.contains_key(ability)
            || self.rpc_with_env.contains_key(ability)
            || self.stream_with_env.contains_key(ability)
            || self.bidi_with_env.contains_key(ability)
    }

    /// Full presence test including the two metadata side-tables.
    /// Used by `drain` to decide the return value, and by any
    /// caller that wants "is this name known at all".
    fn contains(&self, ability: &str) -> bool {
        self.contains_handler(ability)
            || self.owner.contains_key(ability)
            || self.manifests.contains_key(ability)
    }
}

impl std::fmt::Debug for AxonAbilityCatalog {
    /// Manual impl because the handler types are `Arc<dyn Fn>`
    /// trait objects which do not implement `Debug`. Surfaces just
    /// the registered ability counts + names per shape — enough for
    /// `OnceLock::set`'s `.expect(..)` to print a useful message
    /// without leaking handler addresses.
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        let dynamic_count = self
            .dynamic_ext
            .read()
            .map(|g| g.rpc.len() + g.stream.len() + g.bidi.len())
            .unwrap_or(0);
        f.debug_struct("AxonAbilityCatalog")
            .field("rpc_count", &self.rpc.len())
            .field("stream_count", &self.stream.len())
            .field("bidi_count", &self.bidi.len())
            .field("rpc_with_env_count", &self.rpc_with_env.len())
            .field("stream_with_env_count", &self.stream_with_env.len())
            .field("bidi_with_env_count", &self.bidi_with_env.len())
            .field("owner_count", &self.owner.len())
            .field("manifest_count", &self.manifests.len())
            .field("dynamic_ext_count", &dynamic_count)
            .field("has_rpc_fallback", &self.rpc_fallback.is_some())
            .finish()
    }
}

impl AxonAbilityCatalog {
    pub fn new() -> Self {
        Self::new_with_runtime(LocalRuntime::new())
    }

    /// Build a registry whose registration APIs write through to
    /// the daemon-hosted Axon runtime. This keeps the existing
    /// module-level `register(&mut reg)` call sites intact while
    /// making `LocalRuntime` the live source of truth.
    pub fn new_with_runtime(runtime: Arc<LocalRuntime>) -> Self {
        Self {
            runtime: Some(runtime),
            ..Self::default()
        }
    }

    /// Return the attached Axon runtime, if this registry was
    /// constructed for daemon boot.
    pub fn runtime(&self) -> Option<Arc<LocalRuntime>> {
        self.runtime.as_ref().map(Arc::clone)
    }

    /// Invoke an RPC ability through Axon's `LocalRuntime`. The
    /// handler maps are retained as catalogue metadata and for
    /// descriptor synthesis, but runtime execution is the authoritative
    /// path.
    pub fn invoke_rpc_json(&self, ability: &str, args: Value) -> anyhow::Result<Value> {
        let runtime = self.runtime().ok_or_else(|| {
            anyhow::anyhow!(
                "AxonAbilityCatalog has no LocalRuntime attached; use new() or new_with_runtime()"
            )
        })?;
        let target = InvocationTarget {
            scope: TargetScope::Local,
            ability: ability.to_string(),
            normalized_args: args.clone(),
            call_mode: CallMode::Rpc,
            subject: None,
            causal_context: None,
        };
        match crate::runtime::local_runtime_invoker::invoke_local_rpc_sync(
            Arc::clone(&runtime),
            target,
        ) {
            Ok(value) => Ok(value),
            Err(err) if crate::runtime::local_runtime_invoker::is_not_found_error(&err) => {
                if let Some(handler) = self.resolve_rpc(ability) {
                    // Self-heal: the handler exists in the catalogue
                    // but was not synced into LocalRuntime at boot.
                    // We register lazily and retry, but the bug is in
                    // boot: every handler that lives in the catalogue
                    // should be in LocalRuntime by the time we serve
                    // an Invoke. Surface a warn so SRE notices when
                    // this path fires — silent self-heal would mask
                    // a real wiring regression.
                    crate::op_event!(
                        component = ability_dispatch,
                        kind = late_bound_rpc_handler,
                        ability = ability,
                        message = "RPC handler resolved from catalogue but missing from \
                                   LocalRuntime; lazily synced. Boot should have wired \
                                   this; investigate the registration path.",
                    );
                    self.register_runtime_rpc(ability, handler);
                    let target = InvocationTarget {
                        scope: TargetScope::Local,
                        ability: ability.to_string(),
                        normalized_args: args,
                        call_mode: CallMode::Rpc,
                        subject: None,
                        causal_context: None,
                    };
                    return crate::runtime::local_runtime_invoker::invoke_local_rpc_sync(
                        runtime, target,
                    )
                    .map_err(|err| anyhow::anyhow!("{err}"));
                }
                Err(anyhow::anyhow!("{err}"))
            }
            Err(err) => Err(anyhow::anyhow!("{err}")),
        }
    }

    fn replace_runtime_ability(&self, name: &str, ability_fn: AbilityFn, options: AbilityOptions) {
        let Some(runtime) = self.runtime.as_ref() else {
            return;
        };
        let result =
            block_on_runtime_sync(runtime.replace_ability(name.to_string(), ability_fn, options));
        if let Err(err) = result {
            let err_msg = format!("{err}");
            crate::op_event!(
                component = axon_bridge,
                kind = local_runtime_register_failed,
                ability = name,
                error = err_msg.as_str(),
            );
        }
    }

    fn unregister_runtime_ability(&self, name: &str) {
        let Some(runtime) = self.runtime.as_ref() else {
            return;
        };
        let _ = block_on_runtime_sync(runtime.unregister_ability(name));
    }

    fn register_runtime_rpc(&self, name: &str, handler: LocalRpcHandler) {
        let mut handlers = self.runtime_handlers_for(name);
        handlers.rpc = Some(handler);
        self.sync_runtime_ability_from_handlers(name, handlers);
    }

    fn runtime_handlers_for(&self, name: &str) -> RuntimeHandlerSet {
        let mut handlers = RuntimeHandlerSet {
            rpc: self.rpc.get(name).map(Arc::clone),
            stream: self.stream.get(name).map(Arc::clone),
            bidi: self.bidi.get(name).map(Arc::clone),
            rpc_with_env: self.rpc_with_env.get(name).map(Arc::clone),
            stream_with_env: self.stream_with_env.get(name).map(Arc::clone),
            bidi_with_env: self.bidi_with_env.get(name).map(Arc::clone),
        };
        if handlers.rpc.is_none()
            || handlers.stream.is_none()
            || handlers.bidi.is_none()
            || handlers.rpc_with_env.is_none()
            || handlers.stream_with_env.is_none()
            || handlers.bidi_with_env.is_none()
        {
            let dyn_ext = self
                .dynamic_ext
                .read()
                .expect("dynamic_ext RwLock poisoned");
            if handlers.rpc.is_none() {
                handlers.rpc = dyn_ext.rpc.get(name).map(Arc::clone);
            }
            if handlers.stream.is_none() {
                handlers.stream = dyn_ext.stream.get(name).map(Arc::clone);
            }
            if handlers.bidi.is_none() {
                handlers.bidi = dyn_ext.bidi.get(name).map(Arc::clone);
            }
            if handlers.rpc_with_env.is_none() {
                handlers.rpc_with_env = dyn_ext.rpc_with_env.get(name).map(Arc::clone);
            }
            if handlers.stream_with_env.is_none() {
                handlers.stream_with_env = dyn_ext.stream_with_env.get(name).map(Arc::clone);
            }
            if handlers.bidi_with_env.is_none() {
                handlers.bidi_with_env = dyn_ext.bidi_with_env.get(name).map(Arc::clone);
            }
        }
        handlers
    }

    fn sync_runtime_ability(&self, name: &str) {
        let handlers = self.runtime_handlers_for(name);
        self.sync_runtime_ability_from_handlers(name, handlers);
    }

    /// True when `ability` is present in the boot-time static catalogue.
    ///
    /// What this is NOT: a general discovery helper. Dynamic plugin/MCP
    /// entries are intentionally ignored so plugin-host reload can enforce
    /// the invariant that post-boot extensions never shadow daemon/system
    /// abilities.
    pub fn has_static_ability(&self, ability: &str) -> bool {
        self.rpc.contains_key(ability)
            || self.stream.contains_key(ability)
            || self.bidi.contains_key(ability)
            || self.rpc_with_env.contains_key(ability)
            || self.stream_with_env.contains_key(ability)
            || self.bidi_with_env.contains_key(ability)
            || self.owner.contains_key(ability)
            || self.manifests.contains_key(ability)
    }

    fn reject_dynamic_shadow_of_static(&self, ability: &str) -> bool {
        if !self.has_static_ability(ability) {
            return false;
        }
        crate::op_event!(
            component = ability_dispatch,
            kind = hot_register_static_collision_rejected,
            ability = ability,
            message = "dynamic ability registration attempted to shadow a boot-time ability",
        );
        self.sync_runtime_ability(ability);
        true
    }

    fn sync_runtime_ability_from_handlers(&self, name: &str, handlers: RuntimeHandlerSet) {
        let modes = handlers.modes();
        if modes.is_empty() {
            self.unregister_runtime_ability(name);
            return;
        }
        self.replace_runtime_ability(
            name,
            runtime_handler_set_to_ability_fn(name.to_string(), handlers),
            AbilityOptions::default().with_modes(modes),
        );
    }

    /// Register an RPC handler under `ability` with explicit owner.
    /// Replaces any prior handler at the same key — the daemon owns
    /// this registry and is the only writer, so accidental duplicate
    /// registration would be a bug at startup, not a race.
    ///
    /// **M0 of the system-namespace migration.** New call sites
    /// must use this variant; the legacy [`register_rpc`] is a
    /// transitional shim that defaults `owner` to
    /// [`OwnerKind::Device`]. Once every call site has migrated
    /// (M0 commit 6) the shim is deleted.
    pub fn register_rpc_with_owner(
        &mut self,
        ability: impl Into<String>,
        owner: OwnerKind,
        handler: LocalRpcHandler,
    ) {
        let name = ability.into();
        self.owner.insert(name.clone(), owner);
        self.rpc.insert(name.clone(), handler);
        self.sync_runtime_ability(&name);
    }

    /// Register an RPC handler with explicit owner AND a manifest
    /// carrying the verb's description / input_schema /
    /// output_schema. The manifest flows to
    /// `meta.list_abilities` and ultimately to the Frontend
    /// `InvokeAbilityDialog`, which renders a SchemaForm when an
    /// input schema is present and a free-text JSON box otherwise.
    /// Use this variant for any ability that already has a static
    /// manifest in `core::ability_spec` (the chat ability, the
    /// pages family, …); the registry then becomes the single
    /// source of truth for "does this verb have a schema" and
    /// downstream consumers stop having to know which manifest
    /// constructor to call by hand.
    pub fn register_rpc_with_spec(
        &mut self,
        ability: impl Into<String>,
        owner: OwnerKind,
        manifest: crate::core::ability_spec::AbilityManifest,
        handler: LocalRpcHandler,
    ) {
        let name = ability.into();
        self.owner.insert(name.clone(), owner);
        self.manifests.insert(name.clone(), Arc::new(manifest));
        self.rpc.insert(name.clone(), handler);
        self.sync_runtime_ability(&name);
    }

    /// Companion to [`register_rpc_with_spec`] for stream
    /// handlers. The manifest is shared between RPC + Stream
    /// surfaces of the same ability — registering the manifest on
    /// both call sites is allowed (last writer wins; in practice
    /// both register the same `AbilityManifest` constant) so
    /// callers can pick whichever register site they reach first
    /// without having to coordinate ordering.
    pub fn register_stream_with_spec(
        &mut self,
        ability: impl Into<String>,
        owner: OwnerKind,
        manifest: crate::core::ability_spec::AbilityManifest,
        handler: LocalStreamHandler,
    ) {
        let name = ability.into();
        self.owner.insert(name.clone(), owner);
        self.manifests.insert(name.clone(), Arc::new(manifest));
        self.stream.insert(name.clone(), handler);
        self.sync_runtime_ability(&name);
    }

    /// Lookup the registered manifest, if any. Returns `None`
    /// when the ability was registered through a non-`_with_spec`
    /// path (the legacy register surface) — the descriptor synth
    /// in `meta_ability::list_abilities_handler` falls back to a
    /// name-only stub in that case, matching pre-2026-05 behaviour.
    ///
    /// This consults only the static map; dynamic (hot-reload)
    /// entries do not appear here. Use `manifest_for_dynamic` when
    /// the consumer also wants to see hot-loaded MCP tools.
    pub fn manifest_for(
        &self,
        ability: &str,
    ) -> Option<&crate::core::ability_spec::AbilityManifest> {
        self.manifests.get(ability).map(|m| m.as_ref())
    }

    /// Static-OR-dynamic manifest lookup. Returns the static manifest
    /// when present (canonical), otherwise the dynamic-side entry, in
    /// `Arc` form so the borrow does not depend on the RwLock guard.
    /// Used by `meta_ability::list_abilities_handler` so hot-loaded
    /// MCP tools advertise their input schemas to the catalogue.
    pub fn manifest_for_dynamic(
        &self,
        ability: &str,
    ) -> Option<Arc<crate::core::ability_spec::AbilityManifest>> {
        if let Some(m) = self.manifests.get(ability) {
            return Some(Arc::clone(m));
        }
        let dyn_ext = self
            .dynamic_ext
            .read()
            .expect("dynamic_ext RwLock poisoned");
        dyn_ext.manifests.get(ability).map(Arc::clone)
    }

    /// Register an RPC handler under `ability`. Owner defaults to
    /// [`OwnerKind::Device`] — the safe choice for the bulk of
    /// today's catalogue (80%+ of system abilities are device-
    /// bundle). New call sites should use [`register_rpc_with_owner`]
    /// to declare the actual owner explicitly.
    ///
    /// As of M0 commit 5 of the system-namespace migration, every
    /// production register site has migrated to the `_with_owner`
    /// family; this shim is retained only because a handful of
    /// test fixtures across the agents module still use it
    /// (`test.api_key.*`-style harness wiring), and migrating them
    /// adds zero owner-attribution value. M0 commit 6 (a separate,
    /// optional follow-up PR) deletes the shim once those tests
    /// are also converted.
    pub fn register_rpc(&mut self, ability: impl Into<String>, handler: LocalRpcHandler) {
        self.register_rpc_with_owner(ability, OwnerKind::Device, handler);
    }

    /// Register a stream handler with explicit owner. See
    /// [`register_rpc_with_owner`] for the M0 migration rationale.
    pub fn register_stream_with_owner(
        &mut self,
        ability: impl Into<String>,
        owner: OwnerKind,
        handler: LocalStreamHandler,
    ) {
        let name = ability.into();
        self.owner.insert(name.clone(), owner);
        self.stream.insert(name.clone(), handler);
        self.sync_runtime_ability(&name);
    }

    /// Register a stream handler under `ability`. Same single-
    /// writer model as `register_rpc`. Transitional shim;
    /// defaults owner to `OwnerKind::Device`.
    pub fn register_stream(&mut self, ability: impl Into<String>, handler: LocalStreamHandler) {
        self.register_stream_with_owner(ability, OwnerKind::Device, handler);
    }

    /// Register a bidi handler with explicit owner. See
    /// [`register_rpc_with_owner`] for the M0 migration rationale.
    pub fn register_bidi_with_owner(
        &mut self,
        ability: impl Into<String>,
        owner: OwnerKind,
        handler: LocalBidiHandler,
    ) {
        let name = ability.into();
        self.owner.insert(name.clone(), owner);
        self.bidi.insert(name.clone(), handler);
        self.sync_runtime_ability(&name);
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
    ///
    /// Transitional shim; defaults owner to `OwnerKind::Device`.
    pub fn register_bidi(&mut self, ability: impl Into<String>, handler: LocalBidiHandler) {
        self.register_bidi_with_owner(ability, OwnerKind::Device, handler);
    }

    /// Register an envelope-aware RPC handler with explicit owner.
    /// See [`register_rpc_with_owner`] for the M0 migration
    /// rationale and [`register_rpc_with_envelope`] for the
    /// envelope-context contract.
    pub fn register_rpc_with_envelope_and_owner(
        &mut self,
        ability: impl Into<String>,
        owner: OwnerKind,
        handler: LocalRpcHandlerWithEnvelope,
    ) {
        let name = ability.into();
        self.owner.insert(name.clone(), owner);
        self.rpc_with_env.insert(name.clone(), handler);
        self.sync_runtime_ability(&name);
    }

    /// Register an envelope-aware RPC handler with explicit owner and manifest.
    ///
    /// Plugin handlers need the AXIOM envelope (`subject`, caller context, and
    /// causal metadata) and also need to publish the package descriptor schema
    /// through `meta.list_abilities`. This method keeps those two contracts in
    /// one registration path instead of forcing plugin host code to write the
    /// handler and manifest side tables separately.
    pub fn register_rpc_with_envelope_and_spec(
        &mut self,
        ability: impl Into<String>,
        owner: OwnerKind,
        manifest: crate::core::ability_spec::AbilityManifest,
        handler: LocalRpcHandlerWithEnvelope,
    ) {
        let name = ability.into();
        self.owner.insert(name.clone(), owner);
        self.manifests.insert(name.clone(), Arc::new(manifest));
        self.rpc_with_env.insert(name.clone(), handler);
        self.sync_runtime_ability(&name);
    }

    /// Register an envelope-aware RPC handler. Used by abilities
    /// that need access to the AXIOM 7-tuple `subject` (per
    /// **INV-SUBJECT-ENVELOPE**) — typically media abilities
    /// resolving a `subject = resource_ura` to a local resource
    /// table entry. The handler closure signature is
    /// `Fn(EnvelopeContext, Value) -> Result<Value>`; the runtime
    /// adapter
    /// passes the resolved `InvocationTarget.subject` in the
    /// context. Mutually exclusive with `register_rpc` per ability
    /// — registering both is a startup bug (caller picks one
    /// shape per ability).
    ///
    /// Transitional shim; defaults owner to `OwnerKind::Device`.
    pub fn register_rpc_with_envelope(
        &mut self,
        ability: impl Into<String>,
        handler: LocalRpcHandlerWithEnvelope,
    ) {
        self.register_rpc_with_envelope_and_owner(ability, OwnerKind::Device, handler);
    }

    /// Envelope-aware stream variant with explicit owner. See
    /// [`register_rpc_with_owner`] for the M0 migration rationale.
    pub fn register_stream_with_envelope_and_owner(
        &mut self,
        ability: impl Into<String>,
        owner: OwnerKind,
        handler: LocalStreamHandlerWithEnvelope,
    ) {
        let name = ability.into();
        self.owner.insert(name.clone(), owner);
        self.stream_with_env.insert(name.clone(), handler);
        self.sync_runtime_ability(&name);
    }

    /// Register an envelope-aware stream handler with explicit owner and
    /// registry manifest.
    pub fn register_stream_with_envelope_and_spec(
        &mut self,
        ability: impl Into<String>,
        owner: OwnerKind,
        manifest: crate::core::ability_spec::AbilityManifest,
        handler: LocalStreamHandlerWithEnvelope,
    ) {
        let name = ability.into();
        self.owner.insert(name.clone(), owner);
        self.manifests.insert(name.clone(), Arc::new(manifest));
        self.stream_with_env.insert(name.clone(), handler);
        self.sync_runtime_ability(&name);
    }

    /// Envelope-aware stream variant. See `register_rpc_with_envelope`
    /// for the rationale. Transitional shim; defaults owner to
    /// `OwnerKind::Device`.
    pub fn register_stream_with_envelope(
        &mut self,
        ability: impl Into<String>,
        handler: LocalStreamHandlerWithEnvelope,
    ) {
        self.register_stream_with_envelope_and_owner(ability, OwnerKind::Device, handler);
    }

    /// Envelope-aware bidi variant with explicit owner. See
    /// [`register_rpc_with_owner`] for the M0 migration rationale.
    pub fn register_bidi_with_envelope_and_owner(
        &mut self,
        ability: impl Into<String>,
        owner: OwnerKind,
        handler: LocalBidiHandlerWithEnvelope,
    ) {
        let name = ability.into();
        self.owner.insert(name.clone(), owner);
        self.bidi_with_env.insert(name.clone(), handler);
        self.sync_runtime_ability(&name);
    }

    /// Register an envelope-aware bidi handler with explicit owner and
    /// registry manifest.
    pub fn register_bidi_with_envelope_and_spec(
        &mut self,
        ability: impl Into<String>,
        owner: OwnerKind,
        manifest: crate::core::ability_spec::AbilityManifest,
        handler: LocalBidiHandlerWithEnvelope,
    ) {
        let name = ability.into();
        self.owner.insert(name.clone(), owner);
        self.manifests.insert(name.clone(), Arc::new(manifest));
        self.bidi_with_env.insert(name.clone(), handler);
        self.sync_runtime_ability(&name);
    }

    /// Envelope-aware bidi variant. See `register_rpc_with_envelope`
    /// for the rationale. Transitional shim; defaults owner to
    /// `OwnerKind::Device`.
    pub fn register_bidi_with_envelope(
        &mut self,
        ability: impl Into<String>,
        handler: LocalBidiHandlerWithEnvelope,
    ) {
        self.register_bidi_with_envelope_and_owner(ability, OwnerKind::Device, handler);
    }

    /// Look up the owner kind for a registered ability. Returns
    /// `None` when the ability has not been registered (or was
    /// registered before owner tracking landed — should not happen
    /// after M0 commit 1).
    ///
    /// Use this from synth paths and advertise scanners INSTEAD of
    /// sniffing the name string. The 2026-05-05 keyring rename
    /// regression on the Frontend Agents page was caused by a synth
    /// path doing `name.starts_with("01HUB.")`; reading owner here
    /// makes that class of bug structurally impossible.
    ///
    /// Static map wins over the dynamic side table: if an upstream
    /// MCP server's tool name happens to collide with a boot-
    /// registered system ability, the static owner is canonical.
    /// Returns `Some(OwnerKind)` by value (rather than `&OwnerKind`)
    /// because the dynamic-side fallback requires reading through
    /// an RwLock — `&` would tie the borrow to the lock guard.
    pub fn lookup_owner(&self, ability: &str) -> Option<OwnerKind> {
        if let Some(o) = self.owner.get(ability) {
            return Some(o.clone());
        }
        let dyn_ext = self
            .dynamic_ext
            .read()
            .expect("dynamic_ext RwLock poisoned");
        dyn_ext.owner.get(ability).cloned()
    }

    #[cfg(test)]
    pub(crate) fn clear_owner_for_test(&mut self, ability: &str) {
        self.owner.remove(ability);
        let mut dyn_ext = self
            .dynamic_ext
            .write()
            .expect("dynamic_ext RwLock poisoned");
        dyn_ext.owner.remove(ability);
    }

    /// Remove every trace of `ability` from the registry: handlers
    /// in all six maps (rpc / stream / bidi × plain / envelope-aware),
    /// the owner table, and the manifest cache. Returns `true` if
    /// the ability was present (any map had it), `false` if it was
    /// already absent — callers can use this to log a refresh diff
    /// without worrying about TOCTOU between has_rpc/unregister.
    ///
    /// Use cases (plan §B4):
    ///   * `mcp_reflective_registry` reacts to an upstream's
    ///     `notifications/tools/list_changed` by re-running tools/list
    ///     and unregistering tools that no longer appear.
    ///   * Future hot-reload of ability TOMLs.
    ///
    /// **Threading**: the registry is single-writer at boot in v1.
    /// Callers driving runtime hot-reload MUST hold whatever
    /// synchronisation the daemon imposes (today: build_registry is
    /// the only writer; B4 will introduce a process-wide
    /// `Arc<Mutex<AxonAbilityCatalog>>` if hot-reload lands).
    pub fn unregister(&mut self, ability: &str) -> bool {
        let present = self.drain_static(ability);
        if present {
            self.sync_runtime_ability(ability);
        }
        present
    }

    /// Drop every static-side trace of `ability` across the eight
    /// maps and return whether any of them carried it. Mirrors
    /// [`DynamicCatalogue::drain`]; lifted out of `unregister` so
    /// the 8-way walk lives in one named place per side instead of
    /// being inlined at every removal site.
    fn drain_static(&mut self, ability: &str) -> bool {
        let present = self.rpc.contains_key(ability)
            || self.stream.contains_key(ability)
            || self.bidi.contains_key(ability)
            || self.rpc_with_env.contains_key(ability)
            || self.stream_with_env.contains_key(ability)
            || self.bidi_with_env.contains_key(ability)
            || self.owner.contains_key(ability)
            || self.manifests.contains_key(ability);
        self.rpc.remove(ability);
        self.stream.remove(ability);
        self.bidi.remove(ability);
        self.rpc_with_env.remove(ability);
        self.stream_with_env.remove(ability);
        self.bidi_with_env.remove(ability);
        self.owner.remove(ability);
        self.manifests.remove(ability);
        present
    }

    // ── Hot-reload side table ─────────────────────────────────────────
    //
    // The methods below are the `&self` mutation surface used by
    // `RegistryRefreshSink` after boot. They write to `dynamic_ext`
    // instead of the static maps so the hot path remains lock-free
    // for everything boot-registered. Lookups fall through to
    // `dynamic_ext` on a static-map miss; `list_abilities` /
    // `lookup_owner` / `manifest` likewise consult both sides.

    /// Hot-register an RPC handler with explicit owner + manifest in
    /// the dynamic side table. Used by `RegistryRefreshSink` when a
    /// freshly-listed upstream MCP tool needs to become invokable
    /// without a daemon restart. Replaces any prior dynamic entry at
    /// the same key (same write-replaces-write semantics as the
    /// static `register_rpc_with_spec`).
    pub fn hot_register_rpc_with_spec(
        &self,
        ability: impl Into<String>,
        owner: OwnerKind,
        manifest: crate::core::ability_spec::AbilityManifest,
        handler: LocalRpcHandler,
    ) {
        let name = ability.into();
        if self.reject_dynamic_shadow_of_static(&name) {
            return;
        }
        {
            let mut dyn_ext = self
                .dynamic_ext
                .write()
                .expect("dynamic_ext RwLock poisoned");
            dyn_ext.drain(&name);
            dyn_ext.owner.insert(name.clone(), owner);
            dyn_ext.manifests.insert(name.clone(), Arc::new(manifest));
            dyn_ext.rpc.insert(name.clone(), handler);
        }
        self.sync_runtime_ability(&name);
    }

    /// Hot-register an RPC handler without a manifest. Used by
    /// the dynamic side when an upstream tool's input schema isn't
    /// declared (rare but legal — the upstream tool may have only a
    /// description). Falls back to the name-only discovery stub.
    pub fn hot_register_rpc(
        &self,
        ability: impl Into<String>,
        owner: OwnerKind,
        handler: LocalRpcHandler,
    ) {
        let name = ability.into();
        if self.reject_dynamic_shadow_of_static(&name) {
            return;
        }
        {
            let mut dyn_ext = self
                .dynamic_ext
                .write()
                .expect("dynamic_ext RwLock poisoned");
            dyn_ext.drain(&name);
            dyn_ext.owner.insert(name.clone(), owner);
            dyn_ext.rpc.insert(name.clone(), handler);
        }
        self.sync_runtime_ability(&name);
    }

    /// Hot-register a STREAM handler with a manifest. The reflective
    /// registry's `register_one_tool` registers MCP tools as stream
    /// handlers so upstream `notifications/progress` frames flow
    /// through Axon's `InvokeStream`; the hot-reload sink needs the
    /// same shape so a freshly-listed MCP tool dispatches identically
    /// to a boot-registered one.
    pub fn hot_register_stream_with_spec(
        &self,
        ability: impl Into<String>,
        owner: OwnerKind,
        manifest: crate::core::ability_spec::AbilityManifest,
        handler: LocalStreamHandler,
    ) {
        let name = ability.into();
        if self.reject_dynamic_shadow_of_static(&name) {
            return;
        }
        {
            let mut dyn_ext = self
                .dynamic_ext
                .write()
                .expect("dynamic_ext RwLock poisoned");
            dyn_ext.drain(&name);
            dyn_ext.owner.insert(name.clone(), owner);
            dyn_ext.manifests.insert(name.clone(), Arc::new(manifest));
            dyn_ext.stream.insert(name.clone(), handler);
        }
        self.sync_runtime_ability(&name);
    }

    /// Hot-register an envelope-aware RPC handler in the dynamic side table.
    /// Plugin hot-load uses this path so sidecar/declarative handlers receive
    /// the same AXIOM envelope context as boot-registered handlers.
    pub fn hot_register_rpc_with_envelope(
        &self,
        ability: impl Into<String>,
        owner: OwnerKind,
        handler: LocalRpcHandlerWithEnvelope,
    ) {
        let name = ability.into();
        if self.reject_dynamic_shadow_of_static(&name) {
            return;
        }
        {
            let mut dyn_ext = self
                .dynamic_ext
                .write()
                .expect("dynamic_ext RwLock poisoned");
            dyn_ext.drain(&name);
            dyn_ext.owner.insert(name.clone(), owner);
            dyn_ext.rpc_with_env.insert(name.clone(), handler);
        }
        self.sync_runtime_ability(&name);
    }

    /// Hot-register an envelope-aware RPC handler with explicit owner and
    /// registry manifest in the dynamic side table.
    pub fn hot_register_rpc_with_envelope_and_spec(
        &self,
        ability: impl Into<String>,
        owner: OwnerKind,
        manifest: crate::core::ability_spec::AbilityManifest,
        handler: LocalRpcHandlerWithEnvelope,
    ) {
        let name = ability.into();
        if self.reject_dynamic_shadow_of_static(&name) {
            return;
        }
        {
            let mut dyn_ext = self
                .dynamic_ext
                .write()
                .expect("dynamic_ext RwLock poisoned");
            dyn_ext.drain(&name);
            dyn_ext.owner.insert(name.clone(), owner);
            dyn_ext.manifests.insert(name.clone(), Arc::new(manifest));
            dyn_ext.rpc_with_env.insert(name.clone(), handler);
        }
        self.sync_runtime_ability(&name);
    }

    /// Hot-register an envelope-aware stream handler in the dynamic side table.
    pub fn hot_register_stream_with_envelope(
        &self,
        ability: impl Into<String>,
        owner: OwnerKind,
        handler: LocalStreamHandlerWithEnvelope,
    ) {
        let name = ability.into();
        if self.reject_dynamic_shadow_of_static(&name) {
            return;
        }
        {
            let mut dyn_ext = self
                .dynamic_ext
                .write()
                .expect("dynamic_ext RwLock poisoned");
            dyn_ext.drain(&name);
            dyn_ext.owner.insert(name.clone(), owner);
            dyn_ext.stream_with_env.insert(name.clone(), handler);
        }
        self.sync_runtime_ability(&name);
    }

    /// Hot-register an envelope-aware stream handler with explicit owner and
    /// registry manifest in the dynamic side table.
    pub fn hot_register_stream_with_envelope_and_spec(
        &self,
        ability: impl Into<String>,
        owner: OwnerKind,
        manifest: crate::core::ability_spec::AbilityManifest,
        handler: LocalStreamHandlerWithEnvelope,
    ) {
        let name = ability.into();
        if self.reject_dynamic_shadow_of_static(&name) {
            return;
        }
        {
            let mut dyn_ext = self
                .dynamic_ext
                .write()
                .expect("dynamic_ext RwLock poisoned");
            dyn_ext.drain(&name);
            dyn_ext.owner.insert(name.clone(), owner);
            dyn_ext.manifests.insert(name.clone(), Arc::new(manifest));
            dyn_ext.stream_with_env.insert(name.clone(), handler);
        }
        self.sync_runtime_ability(&name);
    }

    /// Hot-register an envelope-aware bidi handler in the dynamic side table.
    pub fn hot_register_bidi_with_envelope(
        &self,
        ability: impl Into<String>,
        owner: OwnerKind,
        handler: LocalBidiHandlerWithEnvelope,
    ) {
        let name = ability.into();
        if self.reject_dynamic_shadow_of_static(&name) {
            return;
        }
        {
            let mut dyn_ext = self
                .dynamic_ext
                .write()
                .expect("dynamic_ext RwLock poisoned");
            dyn_ext.drain(&name);
            dyn_ext.owner.insert(name.clone(), owner);
            dyn_ext.bidi_with_env.insert(name.clone(), handler);
        }
        self.sync_runtime_ability(&name);
    }

    /// Hot-register an envelope-aware bidi handler with explicit owner and
    /// registry manifest in the dynamic side table.
    pub fn hot_register_bidi_with_envelope_and_spec(
        &self,
        ability: impl Into<String>,
        owner: OwnerKind,
        manifest: crate::core::ability_spec::AbilityManifest,
        handler: LocalBidiHandlerWithEnvelope,
    ) {
        let name = ability.into();
        if self.reject_dynamic_shadow_of_static(&name) {
            return;
        }
        {
            let mut dyn_ext = self
                .dynamic_ext
                .write()
                .expect("dynamic_ext RwLock poisoned");
            dyn_ext.drain(&name);
            dyn_ext.owner.insert(name.clone(), owner);
            dyn_ext.manifests.insert(name.clone(), Arc::new(manifest));
            dyn_ext.bidi_with_env.insert(name.clone(), handler);
        }
        self.sync_runtime_ability(&name);
    }

    /// Remove every dynamic-side trace of `ability` (the parallel
    /// to `unregister` for the static maps). Returns `true` if the
    /// dynamic side actually held the name. Static entries are
    /// **not** touched — the hot-reload sink writes exclusively
    /// through the dynamic surface, so the static side is the
    /// boot-time truth and must not be re-mutated post-boot.
    pub fn hot_unregister(&self, ability: &str) -> bool {
        let mut dyn_ext = self
            .dynamic_ext
            .write()
            .expect("dynamic_ext RwLock poisoned");
        let present = dyn_ext.drain(ability);
        if present {
            drop(dyn_ext);
            self.sync_runtime_ability(ability);
        }
        present
    }

    /// Remove a post-boot ability from the dynamic side and from
    /// LocalRuntime even when the name was originally boot-registered in the
    /// static maps. Plugin package reload uses this to make remove/update
    /// visible without mutating the boot-time metadata maps behind `Arc`.
    pub fn hot_remove_runtime_ability(&self, ability: &str) {
        {
            let mut dyn_ext = self
                .dynamic_ext
                .write()
                .expect("dynamic_ext RwLock poisoned");
            dyn_ext.drain(ability);
        }
        self.unregister_runtime_ability(ability);
    }

    /// True iff the dynamic side currently holds an entry for
    /// `ability` in any of its handler maps. Companion check for
    /// hot-reload diagnostics; the boot-time `has_rpc`/`has_stream`/
    /// `has_bidi` lookups already consult this internally via the
    /// fall-through paths.
    pub fn has_dynamic(&self, ability: &str) -> bool {
        self.dynamic_ext
            .read()
            .expect("dynamic_ext RwLock poisoned")
            .contains_handler(ability)
    }

    /// List the names currently held in the dynamic side table.
    /// Used by `list_abilities` to union dynamic with static
    /// without exposing the lock guard; useful on its own for
    /// hot-reload diagnostics.
    pub fn list_dynamic_abilities(&self) -> Vec<String> {
        let dyn_ext = self
            .dynamic_ext
            .read()
            .expect("dynamic_ext RwLock poisoned");
        let mut names: Vec<String> = dyn_ext.rpc.keys().cloned().collect();
        for k in dyn_ext.rpc_with_env.keys() {
            if !names.iter().any(|n| n == k) {
                names.push(k.clone());
            }
        }
        for k in dyn_ext.stream.keys() {
            if !names.iter().any(|n| n == k) {
                names.push(k.clone());
            }
        }
        for k in dyn_ext.stream_with_env.keys() {
            if !names.iter().any(|n| n == k) {
                names.push(k.clone());
            }
        }
        for k in dyn_ext.bidi.keys() {
            if !names.iter().any(|n| n == k) {
                names.push(k.clone());
            }
        }
        for k in dyn_ext.bidi_with_env.keys() {
            if !names.iter().any(|n| n == k) {
                names.push(k.clone());
            }
        }
        names.sort();
        names
    }

    /// Lookup helper — exposed because PR-ATTACH onwards will need
    /// a way to introspect "what abilities does this daemon
    /// publish?" without reflecting through an invocation.
    ///
    /// Returns the union of RPC + stream + bidi ability names,
    /// sorted. Discovery callers should not see the call-mode
    /// distinction (a single ability is currently only registered
    /// under one call mode, but the union here keeps the list
    /// honest if a future ability legitimately exposes both shapes).
    pub fn list_abilities(&self) -> Vec<String> {
        if let Some(runtime) = self.runtime() {
            let mut names: Vec<String> = block_on_runtime_sync(runtime.list_abilities())
                .into_iter()
                .map(|descriptor| descriptor.name)
                .collect();
            names.sort();
            return names;
        }
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
        // Hot-reload side: union dynamic names so a freshly reflected
        // MCP tool shows up in `meta.list_abilities` immediately.
        for k in self.list_dynamic_abilities() {
            if !names.contains(&k) {
                names.push(k);
            }
        }
        names.sort();
        names
    }

    /// Returns Some when an RPC handler is registered for `ability`.
    pub fn get_rpc(&self, ability: &str) -> Option<&LocalRpcHandler> {
        self.rpc.get(ability)
    }

    /// True iff an RPC-mode handler is registered for `ability`,
    /// including the envelope-aware variant. Consults the dynamic
    /// side table on static-map miss so hot-loaded MCP tools count
    /// as registered.
    pub fn has_rpc(&self, ability: &str) -> bool {
        if let Some(runtime) = self.runtime() {
            return block_on_runtime_sync(runtime.ability_options(ability))
                .map(|options| options.modes.rpc)
                .unwrap_or(false);
        }
        if self.rpc.contains_key(ability) || self.rpc_with_env.contains_key(ability) {
            return true;
        }
        let dyn_ext = self
            .dynamic_ext
            .read()
            .expect("dynamic_ext RwLock poisoned");
        dyn_ext.rpc.contains_key(ability) || dyn_ext.rpc_with_env.contains_key(ability)
    }

    /// List all statically-registered RPC ability names. Does NOT
    /// include names that only resolve through the fallback chain
    /// (those are synthesised at lookup time and have no static
    /// listing). Used by RFC-006-C `01HUB.openai.list_models` to
    /// project chat-base abilities into the /v1/models response.
    pub fn list_rpc_names(&self) -> Vec<String> {
        if let Some(runtime) = self.runtime() {
            let mut names: Vec<String> = block_on_runtime_sync(runtime.list_abilities())
                .into_iter()
                .filter(|descriptor| descriptor.options.modes.rpc)
                .map(|descriptor| descriptor.name)
                .collect();
            names.sort();
            return names;
        }
        self.rpc.keys().cloned().collect()
    }

    /// Owned-clone counterpart that consults the fallback resolver
    /// on a registry miss. Existing call sites that take `&Arc<...>`
    /// keep using `get_rpc`; the runtime adapter and test-only invoke
    /// probes use this so a `<agent>.<verb>` written to disk post-boot
    /// is found via the fallback without forcing the registry to be
    /// mutable.
    ///
    /// Lookup order: static map → dynamic side table → fallback
    /// resolver. The hot-reload sink writes only the dynamic side,
    /// so the runtime lookup path stays lock-free for everything
    /// registered at boot.
    pub fn resolve_rpc(&self, ability: &str) -> Option<LocalRpcHandler> {
        if let Some(h) = self.rpc.get(ability) {
            return Some(Arc::clone(h));
        }
        {
            let dyn_ext = self
                .dynamic_ext
                .read()
                .expect("dynamic_ext RwLock poisoned");
            if let Some(h) = dyn_ext.rpc.get(ability) {
                return Some(Arc::clone(h));
            }
        }
        if let Some(resolver) = self.rpc_fallback.as_ref() {
            return resolver(ability);
        }
        None
    }

    /// Owned-clone counterpart of `get_stream` that also consults
    /// the dynamic side table. The dispatcher's `execute_stream`
    /// path uses this so hot-loaded MCP tools that register as
    /// streams (today: none — MCP `tools/call` is RPC-shaped, but
    /// the surface exists for symmetry and for future MCP server
    /// extensions) are dispatchable without static-map mutation.
    pub fn resolve_stream(&self, ability: &str) -> Option<LocalStreamHandler> {
        if let Some(h) = self.stream.get(ability) {
            return Some(Arc::clone(h));
        }
        let dyn_ext = self
            .dynamic_ext
            .read()
            .expect("dynamic_ext RwLock poisoned");
        dyn_ext.stream.get(ability).map(Arc::clone)
    }

    /// Companion to `resolve_stream` for the envelope-aware variant.
    pub fn resolve_stream_with_env(&self, ability: &str) -> Option<LocalStreamHandlerWithEnvelope> {
        if let Some(h) = self.stream_with_env.get(ability) {
            return Some(Arc::clone(h));
        }
        let dyn_ext = self
            .dynamic_ext
            .read()
            .expect("dynamic_ext RwLock poisoned");
        dyn_ext.stream_with_env.get(ability).map(Arc::clone)
    }

    /// Owned-clone counterpart of `get_bidi` that also consults the
    /// dynamic side table.
    pub fn resolve_bidi(&self, ability: &str) -> Option<LocalBidiHandler> {
        if let Some(h) = self.bidi.get(ability) {
            return Some(Arc::clone(h));
        }
        let dyn_ext = self
            .dynamic_ext
            .read()
            .expect("dynamic_ext RwLock poisoned");
        dyn_ext.bidi.get(ability).map(Arc::clone)
    }

    /// Companion to `resolve_bidi` for the envelope-aware variant.
    pub fn resolve_bidi_with_env(&self, ability: &str) -> Option<LocalBidiHandlerWithEnvelope> {
        if let Some(h) = self.bidi_with_env.get(ability) {
            return Some(Arc::clone(h));
        }
        let dyn_ext = self
            .dynamic_ext
            .read()
            .expect("dynamic_ext RwLock poisoned");
        dyn_ext.bidi_with_env.get(ability).map(Arc::clone)
    }

    /// Owned-clone counterpart of `rpc_with_env.get` that also
    /// consults the dynamic side table. Dispatcher uses this in
    /// `execute_rpc` to keep the envelope-aware precedence rule
    /// (envelope handler beats args-only) honest for hot-loaded
    /// abilities too.
    pub fn resolve_rpc_with_env(&self, ability: &str) -> Option<LocalRpcHandlerWithEnvelope> {
        if let Some(h) = self.rpc_with_env.get(ability) {
            return Some(Arc::clone(h));
        }
        let dyn_ext = self
            .dynamic_ext
            .read()
            .expect("dynamic_ext RwLock poisoned");
        dyn_ext.rpc_with_env.get(ability).map(Arc::clone)
    }

    /// Install the RPC fallback resolver. Called once by the daemon
    /// boot path after every static handler is in place. Replaces
    /// any prior resolver — single-writer registry semantics still
    /// hold; only the daemon installs this.
    pub fn set_rpc_fallback(&mut self, resolver: LocalFallbackResolver) {
        self.rpc_fallback = Some(resolver);
    }

    /// Chain a new fallback resolver in front of any existing one.
    /// The new resolver is consulted first; on its `None`, the
    /// previously-installed resolver (if any) is consulted. Order
    /// of registration therefore matters at boot — the LAST chained
    /// resolver wins on competing patterns. Used by reference
    /// systems (e.g. RFC-006-B Pages) that synthesise per-instance
    /// abilities at lookup time and must coexist with the
    /// chat-style `<agent>.<verb>` resolver.
    pub fn chain_rpc_fallback(&mut self, resolver: LocalFallbackResolver) {
        match self.rpc_fallback.take() {
            None => self.rpc_fallback = Some(resolver),
            Some(prior) => {
                let chained: LocalFallbackResolver = Arc::new(move |name: &str| {
                    if let Some(h) = resolver(name) {
                        return Some(h);
                    }
                    prior(name)
                });
                self.rpc_fallback = Some(chained);
            }
        }
    }

    /// Returns Some when a stream handler is registered for `ability`.
    pub fn get_stream(&self, ability: &str) -> Option<&LocalStreamHandler> {
        self.stream.get(ability)
    }

    /// True iff a server-stream handler is registered for `ability`,
    /// including the envelope-aware variant. Consults the dynamic
    /// side table on static-map miss.
    pub fn has_stream(&self, ability: &str) -> bool {
        if let Some(runtime) = self.runtime() {
            return block_on_runtime_sync(runtime.ability_options(ability))
                .map(|options| options.modes.stream)
                .unwrap_or(false);
        }
        if self.stream.contains_key(ability) || self.stream_with_env.contains_key(ability) {
            return true;
        }
        let dyn_ext = self
            .dynamic_ext
            .read()
            .expect("dynamic_ext RwLock poisoned");
        dyn_ext.stream.contains_key(ability) || dyn_ext.stream_with_env.contains_key(ability)
    }

    /// Returns Some when a bidi handler is registered for `ability`.
    pub fn get_bidi(&self, ability: &str) -> Option<&LocalBidiHandler> {
        self.bidi.get(ability)
    }

    /// True iff a bidirectional-stream handler is registered for
    /// `ability`, including the envelope-aware variant. Consults
    /// the dynamic side table on static-map miss.
    pub fn has_bidi(&self, ability: &str) -> bool {
        if let Some(runtime) = self.runtime() {
            return block_on_runtime_sync(runtime.ability_options(ability))
                .map(|options| options.modes.bidi)
                .unwrap_or(false);
        }
        if self.bidi.contains_key(ability) || self.bidi_with_env.contains_key(ability) {
            return true;
        }
        let dyn_ext = self
            .dynamic_ext
            .read()
            .expect("dynamic_ext RwLock poisoned");
        dyn_ext.bidi.contains_key(ability) || dyn_ext.bidi_with_env.contains_key(ability)
    }
    /// Test-only convenience wrapper that still executes through
    /// `LocalRuntime`, matching the daemon path.
    #[cfg(test)]
    pub fn execute_rpc(&self, target: InvocationTarget) -> anyhow::Result<Value> {
        if target.call_mode != CallMode::Rpc {
            anyhow::bail!(
                "AxonAbilityCatalog::execute_rpc called with non-Rpc call_mode \
                 (got {:?}); use a streaming method instead",
                target.call_mode
            );
        }
        if let TargetScope::Remote { node } = &target.scope {
            anyhow::bail!(
                "AxonAbilityCatalog::execute_rpc no longer accepts \
                 TargetScope::Remote ({node}); route through \
                 Axon federation.forward_invoke."
            );
        }
        let runtime = self.runtime().ok_or_else(|| {
            anyhow::anyhow!(
                "AxonAbilityCatalog has no LocalRuntime attached; use new() or new_with_runtime()"
            )
        })?;
        crate::runtime::local_runtime_invoker::invoke_local_rpc_sync(runtime, target).map_err(
            |err| {
                if crate::runtime::local_runtime_invoker::is_not_found_error(&err) {
                    anyhow::anyhow!("{err}; local Axon runtime loopback path")
                } else {
                    anyhow::anyhow!("{err}")
                }
            },
        )
    }

    /// Test-only convenience wrapper that opens the stream through
    /// `LocalRuntime`, matching the daemon path.
    #[cfg(test)]
    pub fn execute_stream(&self, target: InvocationTarget) -> anyhow::Result<StreamSource> {
        if target.call_mode != CallMode::Stream {
            anyhow::bail!(
                "AxonAbilityCatalog::execute_stream called with non-Stream call_mode \
                 (got {:?}); use execute_rpc instead",
                target.call_mode
            );
        }
        if let TargetScope::Remote { node } = &target.scope {
            anyhow::bail!("local Axon runtime cannot execute remote stream target `{node}`");
        }
        let runtime = self.runtime().ok_or_else(|| {
            anyhow::anyhow!(
                "AxonAbilityCatalog has no LocalRuntime attached; use new() or new_with_runtime()"
            )
        })?;
        let ability = target.ability.clone();
        runtime_stream_source(runtime, target).map_err(|err| {
            let msg = err.to_string();
            if crate::runtime::local_runtime_invoker::is_not_found_error(&msg) {
                anyhow::anyhow!(
                    "no local stream handler registered for ability {ability} (local Axon runtime)"
                )
            } else {
                err
            }
        })
    }

    /// Test-only convenience wrapper that opens the bidi session
    /// through `LocalRuntime`, matching the daemon path.
    #[cfg(test)]
    pub fn execute_bidi(&self, target: InvocationTarget) -> anyhow::Result<BidiSource> {
        if target.call_mode != CallMode::Bidi {
            anyhow::bail!(
                "AxonAbilityCatalog::execute_bidi called with non-Bidi call_mode \
                 (got {:?}); use execute_rpc or execute_stream instead",
                target.call_mode
            );
        }
        if let TargetScope::Remote { node } = &target.scope {
            anyhow::bail!("local Axon runtime cannot execute remote bidi target `{node}`");
        }
        let runtime = self.runtime().ok_or_else(|| {
            anyhow::anyhow!(
                "AxonAbilityCatalog has no LocalRuntime attached; use new() or new_with_runtime()"
            )
        })?;
        let ability = target.ability.clone();
        runtime_bidi_source(runtime, target).map_err(|err| {
            let msg = err.to_string();
            if crate::runtime::local_runtime_invoker::is_not_found_error(&msg) {
                anyhow::anyhow!(
                    "no local bidi handler registered for ability {ability} (local Axon runtime)"
                )
            } else {
                err
            }
        })
    }
}

#[cfg(test)]
fn runtime_stream_source(
    runtime: Arc<LocalRuntime>,
    target: InvocationTarget,
) -> anyhow::Result<StreamSource> {
    let (ready_tx, ready_rx) = std::sync::mpsc::channel();
    std::thread::Builder::new()
        .name(format!("easynet-axon-stream-{}", target.ability))
        .spawn(move || {
            let rt = match tokio::runtime::Builder::new_current_thread()
                .enable_all()
                .build()
            {
                Ok(rt) => rt,
                Err(err) => {
                    let _ = ready_tx.send(Err(anyhow::anyhow!("build stream runtime: {err}")));
                    return;
                }
            };
            rt.block_on(async move {
                let mut handle =
                    match crate::runtime::local_runtime_invoker::open_local_stream(runtime, target)
                        .await
                    {
                        Ok(handle) => handle,
                        Err(err) => {
                            let _ = ready_tx.send(Err(anyhow::anyhow!("{err}")));
                            return;
                        }
                    };

                let mut snapshot = Vec::new();
                let (tx, rx) = broadcast::channel(BIDI_CHANNEL_BOUND);
                loop {
                    match tokio::time::timeout(
                        std::time::Duration::from_millis(100),
                        handle.next_frame(),
                    )
                    .await
                    {
                        Ok(Some(Ok(frame))) => {
                            if !frame.payload.is_empty() {
                                match crate::runtime::local_runtime_invoker::ability_frame_to_json(
                                    &frame,
                                ) {
                                    Ok(value) => snapshot.push(value),
                                    Err(err) => {
                                        let _ = ready_tx.send(Err(anyhow::anyhow!("{err}")));
                                        return;
                                    }
                                }
                            }
                            if frame.terminal {
                                let _ = ready_tx.send(Ok(StreamSource::Snapshot(snapshot)));
                                return;
                            }
                        }
                        Ok(Some(Err(err))) => {
                            let _ = ready_tx.send(Err(anyhow::anyhow!("{err}")));
                            return;
                        }
                        Ok(None) => {
                            let _ = ready_tx.send(Ok(StreamSource::Snapshot(snapshot)));
                            return;
                        }
                        Err(_) => {
                            let source = if snapshot.is_empty() {
                                StreamSource::Live(rx)
                            } else {
                                StreamSource::SnapshotThenLive(snapshot, rx)
                            };
                            if ready_tx.send(Ok(source)).is_err() {
                                return;
                            }
                            break;
                        }
                    }
                }

                while let Some(frame_result) = handle.next_frame().await {
                    let Ok(frame) = frame_result else {
                        break;
                    };
                    if !frame.payload.is_empty() {
                        if let Ok(value) =
                            crate::runtime::local_runtime_invoker::ability_frame_to_json(&frame)
                        {
                            let _ = tx.send(value);
                        }
                    }
                    if frame.terminal {
                        break;
                    }
                }
            });
        })
        .map_err(|err| anyhow::anyhow!("spawn stream bridge: {err}"))?;
    ready_rx
        .recv()
        .map_err(|err| anyhow::anyhow!("stream bridge exited before ready: {err}"))?
}

#[cfg(test)]
fn runtime_bidi_source(
    runtime: Arc<LocalRuntime>,
    target: InvocationTarget,
) -> anyhow::Result<BidiSource> {
    let (ready_tx, ready_rx) = std::sync::mpsc::channel();
    std::thread::Builder::new()
        .name(format!("easynet-axon-bidi-{}", target.ability))
        .spawn(move || {
            let rt = match tokio::runtime::Builder::new_current_thread()
                .enable_all()
                .build()
            {
                Ok(rt) => rt,
                Err(err) => {
                    let _ = ready_tx.send(Err(anyhow::anyhow!("build bidi runtime: {err}")));
                    return;
                }
            };
            rt.block_on(async move {
                let source =
                    match crate::runtime::local_runtime_invoker::open_local_bidi(runtime, target)
                        .await
                    {
                        Ok(source) => source,
                        Err(err) => {
                            let _ = ready_tx.send(Err(anyhow::anyhow!("{err}")));
                            return;
                        }
                    };

                let (to_client, mut to_runtime) = mpsc::channel::<Value>(BIDI_CHANNEL_BOUND);
                let (from_runtime, from_client) =
                    mpsc::channel::<BidiOutputFrame>(BIDI_CHANNEL_BOUND);
                let runtime_input = source.to_client;
                let mut runtime_output = source.from_client;

                match tokio::time::timeout(
                    std::time::Duration::from_millis(100),
                    runtime_output.next_frame(),
                )
                .await
                {
                    Ok(Some(Ok(frame))) => {
                        if !frame.payload.is_empty() {
                            match crate::runtime::local_runtime_invoker::ability_frame_to_json(
                                &frame,
                            ) {
                                Ok(value) => {
                                    let _ = from_runtime.send(BidiOutputFrame::json(value)).await;
                                }
                                Err(err) => {
                                    let _ = ready_tx.send(Err(anyhow::anyhow!("{err}")));
                                    return;
                                }
                            }
                        }
                    }
                    Ok(Some(Err(err))) => {
                        let _ = ready_tx.send(Err(anyhow::anyhow!("{err}")));
                        return;
                    }
                    Ok(None) | Err(_) => {}
                }

                if ready_tx
                    .send(Ok(BidiSource {
                        to_client,
                        from_client,
                    }))
                    .is_err()
                {
                    return;
                }

                let input_task = tokio::spawn(async move {
                    while let Some(value) = to_runtime.recv().await {
                        let Ok(payload) = json_value_to_payload(&value) else {
                            continue;
                        };
                        let frame = easynet_axon::invocation::BidiInputFrame::new(payload)
                            .with_content_type("application/json");
                        if runtime_input.send(frame).await.is_err() {
                            break;
                        }
                    }
                    let _ = runtime_input.close_input().await;
                });

                let output_task = tokio::spawn(async move {
                    while let Some(frame_result) = runtime_output.next_frame().await {
                        let Ok(frame) = frame_result else {
                            break;
                        };
                        if !frame.payload.is_empty() {
                            let output_frame = if frame.content_type == "application/json"
                                || frame.content_type.is_empty()
                            {
                                match crate::runtime::local_runtime_invoker::ability_frame_to_json(
                                    &frame,
                                ) {
                                    Ok(value) => Ok(BidiOutputFrame::json(value)),
                                    Err(err) => Err(anyhow::anyhow!("{err}")),
                                }
                            } else {
                                Ok(BidiOutputFrame::binary(frame.payload, frame.content_type))
                            };
                            if let Ok(output_frame) = output_frame {
                                if from_runtime.send(output_frame).await.is_err() {
                                    break;
                                }
                            }
                        }
                        if frame.terminal {
                            break;
                        }
                    }
                });
                let _ = tokio::join!(input_task, output_task);
            });
        })
        .map_err(|err| anyhow::anyhow!("spawn bidi bridge: {err}"))?;
    ready_rx
        .recv()
        .map_err(|err| anyhow::anyhow!("bidi bridge exited before ready: {err}"))?
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::runtime::domain::NodeId;
    use crate::runtime::gateway_api::PeerInfo;
    use serde_json::json;

    fn empty_registry() -> Arc<AxonAbilityCatalog> {
        Arc::new(AxonAbilityCatalog::new())
    }

    fn ping_target_local() -> InvocationTarget {
        InvocationTarget {
            scope: TargetScope::Local,
            ability: "observe.health".into(),
            normalized_args: json!({}),
            call_mode: CallMode::Rpc,
            subject: None,
            causal_context: None,
        }
    }

    #[test]
    fn unregistered_local_ability_returns_clear_error() {
        // The error must name the ability so an operator can grep
        // "is observe.health registered?" against the daemon log.
        let dispatcher = empty_registry();
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
        let mut reg = AxonAbilityCatalog::new();
        reg.register_rpc(
            "observe.health",
            Arc::new(|args: Value| Ok(json!({"echo": args}))),
        );
        let dispatcher = Arc::new(reg);
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
        let mut reg = AxonAbilityCatalog::new();
        reg.register_rpc_with_envelope(
            "media.x.snapshot",
            Arc::new(|env: EnvelopeContext, _args: Value| {
                Ok(json!({
                    "saw_subject": env.subject,
                    "args_subject_was_present": false,
                }))
            }),
        );
        let dispatcher = Arc::new(reg);
        let target = InvocationTarget {
            scope: TargetScope::Local,
            ability: "media.x.snapshot".into(),
            normalized_args: json!({}),
            call_mode: CallMode::Rpc,
            subject: Some("easynet:///r/acme/resource/01CAM".into()),
            causal_context: None,
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
        let mut reg = AxonAbilityCatalog::new();
        reg.register_rpc(
            "x.dual",
            Arc::new(|_args: Value| Ok(json!({"path": "legacy"}))),
        );
        reg.register_rpc_with_envelope(
            "x.dual",
            Arc::new(|_env: EnvelopeContext, _args: Value| Ok(json!({"path": "envelope"}))),
        );
        let dispatcher = Arc::new(reg);
        let target = InvocationTarget {
            scope: TargetScope::Local,
            ability: "x.dual".into(),
            normalized_args: json!({}),
            call_mode: CallMode::Rpc,
            subject: None,
            causal_context: None,
        };
        let resp = dispatcher.execute_rpc(target).unwrap();
        assert_eq!(resp, json!({"path": "envelope"}));
    }

    #[test]
    fn envelope_aware_handler_with_degenerate_subject_still_dispatches() {
        // Legacy callers that don't set an explicit resource subject still
        // reach the envelope-aware handler. Through LocalRuntime they receive
        // the degenerate Axon subject (`subject = callee`); handlers that need
        // a real resource URA must reject that at their own boundary.
        let mut reg = AxonAbilityCatalog::new();
        reg.register_rpc_with_envelope(
            "x.optional",
            Arc::new(|env: EnvelopeContext, _args: Value| {
                Ok(json!({"subject_was_none": env.subject.is_none()}))
            }),
        );
        let dispatcher = Arc::new(reg);
        let target = InvocationTarget {
            scope: TargetScope::Local,
            ability: "x.optional".into(),
            normalized_args: json!({}),
            call_mode: CallMode::Rpc,
            subject: None,
            causal_context: None,
        };
        let resp = dispatcher.execute_rpc(target).unwrap();
        assert_eq!(resp, json!({"subject_was_none": false}));
    }

    #[test]
    fn envelope_aware_stream_handler_receives_subject() {
        let mut reg = AxonAbilityCatalog::new();
        reg.register_stream_with_envelope(
            "x.subscribe",
            Arc::new(|env: EnvelopeContext, _args: Value| {
                let frame = json!({"subject_seen": env.subject});
                Ok(StreamSource::Snapshot(vec![frame]))
            }),
        );
        let dispatcher = Arc::new(reg);
        let target = InvocationTarget {
            scope: TargetScope::Local,
            ability: "x.subscribe".into(),
            normalized_args: json!({}),
            call_mode: CallMode::Stream,
            subject: Some("easynet:///r/x/resource/01MIC".into()),
            causal_context: None,
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
        let mut reg = AxonAbilityCatalog::new();
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
        let dispatcher = empty_registry();
        let target = InvocationTarget {
            scope: TargetScope::Remote {
                node: NodeId::new("peer"),
            },
            ability: "observe.health".into(),
            normalized_args: json!({}),
            call_mode: CallMode::Rpc,
            subject: None,
            causal_context: None,
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
        let dispatcher = empty_registry();
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
        let dispatcher = empty_registry();
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
        let dispatcher = empty_registry();
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
        let mut reg = AxonAbilityCatalog::new();
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
            let (_to_handler_tx, from_client) =
                mpsc::channel::<BidiOutputFrame>(BIDI_CHANNEL_BOUND);
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
        let mut reg = AxonAbilityCatalog::new();
        assert!(reg.get_bidi("device.terminal.attach").is_none());
        reg.register_bidi("device.terminal.attach", trivial_bidi_handler());
        assert!(reg.get_bidi("device.terminal.attach").is_some());
        // Negative: not visible on the other call modes.
        assert!(reg.get_rpc("device.terminal.attach").is_none());
        assert!(reg.get_stream("device.terminal.attach").is_none());
    }

    #[test]
    fn list_abilities_includes_bidi_keys_in_sorted_union() {
        // §A12 / §1.3 discovery surfaces (and the future
        // meta.list_abilities ability) project this list verbatim,
        // so a missing call mode would silently hide bidi-only
        // abilities from clients.
        let mut reg = AxonAbilityCatalog::new();
        reg.register_rpc("observe.health", Arc::new(|_| Ok(Value::Null)));
        reg.register_stream(
            "permission.subscribe",
            Arc::new(|_| Ok(StreamSource::Snapshot(vec![]))),
        );
        reg.register_bidi("device.terminal.attach", trivial_bidi_handler());
        assert_eq!(
            reg.list_abilities(),
            vec![
                "device.terminal.attach",
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
        let dispatcher = empty_registry();
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
        let mut reg = AxonAbilityCatalog::new();

        // A handler that owns its own loop reading from_client and
        // echoing into to_client. Spawned inside the closure per §D2.
        reg.register_bidi(
            "device.test.echo",
            Arc::new(|_args: Value| {
                let (client_to_handler_tx, mut client_to_handler_rx) =
                    mpsc::channel::<Value>(BIDI_CHANNEL_BOUND);
                let (handler_to_client_tx, handler_to_client_rx) =
                    mpsc::channel::<BidiOutputFrame>(BIDI_CHANNEL_BOUND);
                // Forwarder side of the BidiSource is what we hand
                // back to the caller — it sees the *handler input*
                // sender (so it can push frames in) and the handler
                // output receiver (so it can pump them out). The
                // handler keeps the opposite ends.
                tokio::spawn(async move {
                    while let Some(v) = client_to_handler_rx.recv().await {
                        if handler_to_client_tx
                            .send(BidiOutputFrame::json(v))
                            .await
                            .is_err()
                        {
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
        let dispatcher = Arc::new(reg);
        let target = InvocationTarget {
            scope: TargetScope::Local,
            ability: "device.test.echo".into(),
            normalized_args: json!({}),
            call_mode: CallMode::Bidi,
            subject: None,
            causal_context: None,
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
            assert_eq!(echoed.into_json_value().unwrap(), json!({"hello": 1}));
        });
    }

    #[test]
    fn execute_bidi_unregistered_ability_returns_clear_error() {
        // Mirror unregistered_local_ability_returns_clear_error for
        // bidi. The error must name the ability so an operator can
        // grep for it.
        let dispatcher = empty_registry();
        let target = InvocationTarget {
            scope: TargetScope::Local,
            ability: "device.terminal.attach".into(),
            normalized_args: json!({}),
            call_mode: CallMode::Bidi,
            subject: None,
            causal_context: None,
        };
        let err = dispatcher.execute_bidi(target).unwrap_err();
        let msg = format!("{err}");
        assert!(
            msg.contains("device.terminal.attach"),
            "names ability: {msg}"
        );
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
        let mut reg = AxonAbilityCatalog::new();
        reg.register_bidi(
            "device.test.bad",
            Arc::new(|_| anyhow::bail!("intentional handler failure: precondition foo missing")),
        );
        let dispatcher = Arc::new(reg);
        let target = InvocationTarget {
            scope: TargetScope::Local,
            ability: "device.test.bad".into(),
            normalized_args: json!({}),
            call_mode: CallMode::Bidi,
            subject: None,
            causal_context: None,
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
        let dispatcher = empty_registry();
        let target = InvocationTarget {
            scope: TargetScope::Remote {
                node: NodeId::new("01PEER"),
            },
            ability: "device.terminal.attach".into(),
            normalized_args: json!({}),
            call_mode: CallMode::Bidi,
            subject: None,
            causal_context: None,
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

    // ── M0 commit 1: OwnerKind round-trip ─────────────────────────

    fn ok_handler() -> LocalRpcHandler {
        Arc::new(|_args| Ok(json!({"ok": true})))
    }

    #[test]
    fn owner_round_trips_for_representative_samples_via_register_with_owner() {
        // Pin the contract: every ability registered via the
        // `_with_owner` family round-trips through `lookup_owner`
        // with the exact OwnerKind variant the call site declared.
        // No name-string sniffing — the registry is the source of
        // truth for owner kind. M0 of the system-namespace migration.
        let mut reg = AxonAbilityCatalog::new();
        reg.register_rpc_with_owner("device.fs.read", OwnerKind::Device, ok_handler());
        reg.register_rpc_with_owner("hub.openai.chat_completions", OwnerKind::Hub, ok_handler());
        reg.register_rpc_with_owner(
            "consent.decide",
            OwnerKind::Agent("consent".to_string()),
            ok_handler(),
        );
        reg.register_rpc_with_owner(
            "00000000-0000-0000-0000-000000000001.api_key.create",
            OwnerKind::User("00000000-0000-0000-0000-000000000001".to_string()),
            ok_handler(),
        );

        assert_eq!(reg.lookup_owner("device.fs.read"), Some(OwnerKind::Device));
        assert_eq!(
            reg.lookup_owner("hub.openai.chat_completions"),
            Some(OwnerKind::Hub)
        );
        assert_eq!(
            reg.lookup_owner("consent.decide"),
            Some(OwnerKind::Agent("consent".to_string()))
        );
        assert_eq!(
            reg.lookup_owner("00000000-0000-0000-0000-000000000001.api_key.create"),
            Some(OwnerKind::User(
                "00000000-0000-0000-0000-000000000001".to_string()
            ))
        );
        // Unregistered ability returns None — synth paths can use
        // this to detect "not in our local registry" without falling
        // back to name-string sniffing.
        assert_eq!(reg.lookup_owner("not.registered"), None);
    }

    #[test]
    fn legacy_register_rpc_shim_defaults_owner_to_device() {
        // The transitional shim. Lets every existing register call
        // site compile unchanged at M0 commit 1; sites migrate to
        // the `_with_owner` family commit-by-commit through M0
        // commits 2-5; the shim is removed at M0 commit 6.
        // Guarantees the shim default matches the documented
        // contract ("80%+ of system abilities are device-bundle").
        let mut reg = AxonAbilityCatalog::new();
        reg.register_rpc("legacy.shim.smoke", ok_handler());
        assert_eq!(
            reg.lookup_owner("legacy.shim.smoke"),
            Some(OwnerKind::Device),
        );
    }

    #[test]
    fn owner_tracking_works_across_all_six_register_variants() {
        // M0 D4 decision: thread OwnerKind across every register
        // variant (rpc/stream/bidi × with-envelope/without).
        // Without this we'd ship sniffing fallbacks for the half-
        // covered variants — the same flat-namespace bug class
        // that the migration is closing.
        let mut reg = AxonAbilityCatalog::new();

        let stream_handler: LocalStreamHandler =
            Arc::new(|_args| Ok(StreamSource::Snapshot(vec![])));
        let bidi_handler: LocalBidiHandler = Arc::new(|_args| {
            let (tx_to_client, _rx_to_client) = mpsc::channel::<Value>(1);
            let (_tx_from_client, rx_from_client) = mpsc::channel::<BidiOutputFrame>(1);
            Ok(BidiSource {
                to_client: tx_to_client,
                from_client: rx_from_client,
            })
        });
        let rpc_env: LocalRpcHandlerWithEnvelope = Arc::new(|_ctx, _args| Ok(json!({})));
        let stream_env: LocalStreamHandlerWithEnvelope =
            Arc::new(|_ctx, _args| Ok(StreamSource::Snapshot(vec![])));
        let bidi_env: LocalBidiHandlerWithEnvelope = Arc::new(|_ctx, _args| {
            let (tx_to_client, _rx_to_client) = mpsc::channel::<Value>(1);
            let (_tx_from_client, rx_from_client) = mpsc::channel::<BidiOutputFrame>(1);
            Ok(BidiSource {
                to_client: tx_to_client,
                from_client: rx_from_client,
            })
        });

        reg.register_rpc_with_owner("a.rpc", OwnerKind::Hub, ok_handler());
        reg.register_stream_with_owner(
            "a.stream",
            OwnerKind::Agent("codex".to_string()),
            stream_handler,
        );
        reg.register_bidi_with_owner("a.bidi", OwnerKind::User("u-1".to_string()), bidi_handler);
        reg.register_rpc_with_envelope_and_owner("a.rpc.env", OwnerKind::Device, rpc_env);
        reg.register_stream_with_envelope_and_owner("a.stream.env", OwnerKind::Hub, stream_env);
        reg.register_bidi_with_envelope_and_owner(
            "a.bidi.env",
            OwnerKind::Agent("web-builder".to_string()),
            bidi_env,
        );

        assert_eq!(reg.lookup_owner("a.rpc"), Some(OwnerKind::Hub));
        assert_eq!(
            reg.lookup_owner("a.stream"),
            Some(OwnerKind::Agent("codex".to_string()))
        );
        assert_eq!(
            reg.lookup_owner("a.bidi"),
            Some(OwnerKind::User("u-1".to_string()))
        );
        assert_eq!(reg.lookup_owner("a.rpc.env"), Some(OwnerKind::Device));
        assert_eq!(reg.lookup_owner("a.stream.env"), Some(OwnerKind::Hub));
        assert_eq!(
            reg.lookup_owner("a.bidi.env"),
            Some(OwnerKind::Agent("web-builder".to_string()))
        );
    }

    // ── M1: dual-name (aliased) registration ────────────────────────

    #[test]
    fn legacy_names_rejected_post_m3() {
        // M3 contract: legacy names are no longer registered. A
        // dispatcher invocation against a legacy name surfaces
        // `AbilityNotFound`. Pin so a future revert that re-adds
        // dual-aliasing has to argue with this test rather than
        // silently re-introducing the legacy half.
        let mut reg = AxonAbilityCatalog::new();
        reg.register_rpc_with_owner(
            "device.fs.read",
            OwnerKind::Device,
            Arc::new(|_args: Value| Ok(json!({"ok": true}))),
        );
        let dispatcher = Arc::new(reg);

        // Canonical works.
        let mut t_can = ping_target_local();
        t_can.ability = "device.fs.read".into();
        let r = dispatcher.execute_rpc(t_can).unwrap();
        assert_eq!(r, json!({"ok": true}));

        // Legacy is gone.
        let mut t_leg = ping_target_local();
        t_leg.ability = "fs.read".into();
        let err = dispatcher.execute_rpc(t_leg).unwrap_err();
        let msg = format!("{err}");
        assert!(
            msg.contains("fs.read"),
            "AbilityNotFound message must name the legacy ability; got: {msg}"
        );
    }

    #[test]
    fn canonical_register_records_owner() {
        // The owner table must carry an entry for the canonical
        // name. A future synth path that reads `lookup_owner` and
        // gets `None` would produce orphaned descriptors.
        let mut reg = AxonAbilityCatalog::new();
        reg.register_rpc_with_owner(
            "hub.openai.chat_completions",
            OwnerKind::Hub,
            Arc::new(|_args: Value| Ok(json!({}))),
        );
        // Canonical lookup returns Hub.
        assert_eq!(
            reg.lookup_owner("hub.openai.chat_completions"),
            Some(OwnerKind::Hub)
        );
        // Post-M3 legacy lookup returns None (alias retired).
        assert_eq!(
            reg.lookup_owner("01HUB.openai.chat_completions"),
            None,
            "post-M3 legacy name must not be in the owner table"
        );
    }

    #[test]
    fn canonical_only_lists_in_catalogue() {
        // Post-M3: `list_abilities()` returns canonical names
        // only — the legacy alias has been removed from the
        // registry. Pin so a future revert that re-introduces
        // dual-aliasing has to argue with this test rather than
        // silently re-doubling the catalogue.
        let mut reg = AxonAbilityCatalog::new();
        reg.register_rpc_with_owner(
            "device.shell.run",
            OwnerKind::Device,
            Arc::new(|_args: Value| Ok(json!({}))),
        );
        let names = reg.list_abilities();
        assert!(names.iter().any(|n| n == "device.shell.run"));
        assert!(
            !names.iter().any(|n| n == "shell.run"),
            "post-M3 legacy name must not appear in list_abilities()"
        );
    }

    #[test]
    fn last_writer_wins_on_duplicate_canonical_registration() {
        // Pin: registering `device.foo` twice with different
        // handlers produces "last write wins" semantics. The
        // single-writer model documents this; the test makes
        // sure no future change accidentally fan-outs the
        // dispatch (e.g. a bag-of-handlers + walk-and-pick
        // shape). Replaces the M1-era
        // `aliased_canonical_does_not_collide_with_existing_registrations`
        // test which exercised the same invariant against the
        // `_aliased` family.
        use std::sync::atomic::{AtomicUsize, Ordering};
        let first_calls = Arc::new(AtomicUsize::new(0));
        let second_calls = Arc::new(AtomicUsize::new(0));
        let mut reg = AxonAbilityCatalog::new();
        let f = Arc::clone(&first_calls);
        reg.register_rpc_with_owner(
            "device.x.foo",
            OwnerKind::Device,
            Arc::new(move |_args| {
                f.fetch_add(1, Ordering::SeqCst);
                Ok(json!({"who": "first"}))
            }),
        );
        let s = Arc::clone(&second_calls);
        reg.register_rpc_with_owner(
            "device.x.foo",
            OwnerKind::Device,
            Arc::new(move |_args| {
                s.fetch_add(1, Ordering::SeqCst);
                Ok(json!({"who": "second"}))
            }),
        );
        let dispatcher = Arc::new(reg);
        let mut t = ping_target_local();
        t.ability = "device.x.foo".into();
        let resp = dispatcher.execute_rpc(t).unwrap();
        // The aliased registration replaced the prior entry —
        // single-writer "last write wins" semantics.
        assert_eq!(resp, json!({"who": "second"}));
        assert_eq!(first_calls.load(Ordering::SeqCst), 0);
        assert_eq!(second_calls.load(Ordering::SeqCst), 1);
    }

    #[test]
    fn owner_tracking_works_across_all_six_register_variants_post_m3() {
        // Post-M3: every register variant records the owner under
        // the canonical name only. Per the D4 decision, M0
        // threaded OwnerKind across all six variants; M3 retains
        // that breadth (the legacy `_aliased` mirror has been
        // retired).
        let mut reg = AxonAbilityCatalog::new();
        let stream_handler: LocalStreamHandler =
            Arc::new(|_args| Ok(StreamSource::Snapshot(vec![])));
        let bidi_handler: LocalBidiHandler = Arc::new(|_args| {
            let (tx_to_client, _rx_to_client) = mpsc::channel::<Value>(1);
            let (_tx_from_client, rx_from_client) = mpsc::channel::<BidiOutputFrame>(1);
            Ok(BidiSource {
                to_client: tx_to_client,
                from_client: rx_from_client,
            })
        });
        let rpc_env: LocalRpcHandlerWithEnvelope = Arc::new(|_ctx, _args| Ok(json!({})));
        let stream_env: LocalStreamHandlerWithEnvelope =
            Arc::new(|_ctx, _args| Ok(StreamSource::Snapshot(vec![])));
        let bidi_env: LocalBidiHandlerWithEnvelope = Arc::new(|_ctx, _args| {
            let (tx_to_client, _rx_to_client) = mpsc::channel::<Value>(1);
            let (_tx_from_client, rx_from_client) = mpsc::channel::<BidiOutputFrame>(1);
            Ok(BidiSource {
                to_client: tx_to_client,
                from_client: rx_from_client,
            })
        });

        reg.register_rpc_with_owner(
            "device.x.rpc",
            OwnerKind::Device,
            Arc::new(|_| Ok(json!({}))),
        );
        reg.register_stream_with_owner("device.x.stream", OwnerKind::Device, stream_handler);
        reg.register_bidi_with_owner("device.x.bidi", OwnerKind::Device, bidi_handler);
        reg.register_rpc_with_envelope_and_owner("device.x.rpc.env", OwnerKind::Device, rpc_env);
        reg.register_stream_with_envelope_and_owner(
            "device.x.stream.env",
            OwnerKind::Device,
            stream_env,
        );
        reg.register_bidi_with_envelope_and_owner("device.x.bidi.env", OwnerKind::Device, bidi_env);

        for n in [
            "device.x.rpc",
            "device.x.stream",
            "device.x.bidi",
            "device.x.rpc.env",
            "device.x.stream.env",
            "device.x.bidi.env",
        ] {
            assert_eq!(
                reg.lookup_owner(n),
                Some(OwnerKind::Device),
                "{n} should be registered with Device owner"
            );
        }
        // Pin: legacy unprefixed forms are NOT registered post-M3.
        for legacy in [
            "x.rpc",
            "x.stream",
            "x.bidi",
            "x.rpc.env",
            "x.stream.env",
            "x.bidi.env",
        ] {
            assert_eq!(
                reg.lookup_owner(legacy),
                None,
                "post-M3 legacy name {legacy} must not be in the owner table"
            );
        }
    }

    #[test]
    fn unregister_removes_rpc_handler_and_descriptor_state() {
        let mut reg = AxonAbilityCatalog::new();
        reg.register_rpc("doomed.tool", Arc::new(|_| Ok(json!("v"))));
        assert!(reg.has_rpc("doomed.tool"));
        assert_eq!(reg.lookup_owner("doomed.tool"), Some(OwnerKind::Device));
        let was_present = reg.unregister("doomed.tool");
        assert!(
            was_present,
            "unregister must report the ability was present"
        );
        assert!(!reg.has_rpc("doomed.tool"));
        assert_eq!(reg.lookup_owner("doomed.tool"), None);
    }

    #[test]
    fn unregister_idempotent_on_missing_ability() {
        let mut reg = AxonAbilityCatalog::new();
        // Returns false but does not panic — the contract callers
        // (B4 list_changed refresh diff) rely on for the
        // "tool went away mid-sync" race.
        assert!(!reg.unregister("never-was-there"));
    }

    #[test]
    fn unregister_removes_stream_and_bidi_handlers() {
        let mut reg = AxonAbilityCatalog::new();
        reg.register_stream(
            "doomed.stream",
            Arc::new(|_| Ok(StreamSource::Snapshot(vec![]))),
        );
        reg.register_bidi(
            "doomed.bidi",
            Arc::new(|_| {
                Err(anyhow::anyhow!(
                    "test bidi handler not expected to actually run"
                ))
            }),
        );
        assert!(reg.has_stream("doomed.stream"));
        assert!(reg.has_bidi("doomed.bidi"));
        reg.unregister("doomed.stream");
        reg.unregister("doomed.bidi");
        assert!(!reg.has_stream("doomed.stream"));
        assert!(!reg.has_bidi("doomed.bidi"));
    }

    // ── Hot-reload (dynamic_ext) side ────────────────────────────────

    #[test]
    fn hot_register_rpc_is_visible_to_resolve_rpc_and_has_rpc() {
        // The whole reason for `dynamic_ext` to exist: a sink can
        // register a handler post-boot through `&self`, and every
        // lookup surface that fed the dispatcher pre-refactor now
        // sees it. `Arc::new(reg)` mirrors the daemon boot shape —
        // after that point a `&mut reg` is no longer reachable.
        let reg = Arc::new(AxonAbilityCatalog::new());
        assert!(!reg.has_rpc("mcp_wikipedia__search"));
        assert!(reg.resolve_rpc("mcp_wikipedia__search").is_none());

        reg.hot_register_rpc(
            "mcp_wikipedia__search",
            OwnerKind::Agent("mcp".to_string()),
            Arc::new(|_args| Ok(serde_json::json!({"hot": true}))),
        );

        assert!(reg.has_rpc("mcp_wikipedia__search"));
        let handler = reg
            .resolve_rpc("mcp_wikipedia__search")
            .expect("hot-registered ability resolves");
        let out = handler(serde_json::json!({})).expect("handler runs");
        assert_eq!(out, serde_json::json!({"hot": true}));
    }

    #[test]
    fn hot_register_rpc_with_spec_publishes_manifest_through_dynamic_lookup() {
        // `manifest_for_dynamic` is what `meta_ability::list_abilities`
        // reads. A hot-registered manifest must surface there or
        // freshly reflected MCP tools advertise without a schema.
        let reg = Arc::new(AxonAbilityCatalog::new());
        let manifest = crate::core::ability_spec::AbilityManifest::new(
            "search",
            "Search Wikipedia.",
            serde_json::json!({"type": "object"}),
        )
        .unwrap();
        reg.hot_register_rpc_with_spec(
            "mcp_wikipedia__search",
            OwnerKind::Agent("mcp".to_string()),
            manifest,
            Arc::new(|_args| Ok(serde_json::json!({}))),
        );

        // Static `manifest_for` is intentionally unchanged — the
        // distinction matters because some callers want strict
        // static-only lookup.
        assert!(reg.manifest_for("mcp_wikipedia__search").is_none());
        let m = reg
            .manifest_for_dynamic("mcp_wikipedia__search")
            .expect("dynamic manifest visible");
        assert_eq!(m.description(), "Search Wikipedia.");
    }

    #[test]
    fn hot_register_replaces_prior_dynamic_call_mode() {
        let reg = Arc::new(AxonAbilityCatalog::new());
        reg.hot_register_rpc(
            "device.plugin.mode_shift",
            OwnerKind::Device,
            Arc::new(|_args| Ok(serde_json::json!({"mode": "rpc"}))),
        );
        assert!(reg.has_rpc("device.plugin.mode_shift"));
        assert!(!reg.has_stream("device.plugin.mode_shift"));

        let manifest = crate::core::ability_spec::AbilityManifest::new(
            "mode_shift",
            "Mode-shift test ability.",
            serde_json::json!({"type": "object"}),
        )
        .unwrap();
        reg.hot_register_stream_with_spec(
            "device.plugin.mode_shift",
            OwnerKind::Device,
            manifest,
            Arc::new(|_args| Ok(StreamSource::Snapshot(Vec::new()))),
        );

        assert!(
            !reg.has_rpc("device.plugin.mode_shift"),
            "hot-registering a stream handler must drain the stale dynamic RPC mode"
        );
        assert!(reg.has_stream("device.plugin.mode_shift"));
        assert_eq!(
            reg.list_dynamic_abilities(),
            vec!["device.plugin.mode_shift".to_string()]
        );
    }

    #[test]
    fn hot_unregister_removes_dynamic_entry_without_touching_static() {
        // Diff-aware refresh writes `hot_unregister` for tools that
        // disappeared from the upstream catalogue. Static entries
        // (boot-registered system abilities) must never be touched
        // by this surface — if a future bug routes a static name
        // through `hot_unregister`, the static entry survives.
        let mut reg = AxonAbilityCatalog::new();
        reg.register_rpc_with_owner(
            "device.fs.read",
            OwnerKind::Device,
            Arc::new(|_args| Ok(serde_json::Value::Null)),
        );
        let reg = Arc::new(reg);

        reg.hot_register_rpc(
            "mcp_wikipedia__search",
            OwnerKind::Agent("mcp".to_string()),
            Arc::new(|_args| Ok(serde_json::Value::Null)),
        );
        assert!(reg.has_rpc("mcp_wikipedia__search"));
        assert!(reg.has_rpc("device.fs.read"));

        let removed = reg.hot_unregister("mcp_wikipedia__search");
        assert!(removed, "hot_unregister reports the entry was present");
        assert!(!reg.has_rpc("mcp_wikipedia__search"));
        // Static entry untouched.
        assert!(reg.has_rpc("device.fs.read"));

        // Calling hot_unregister on a static name is a silent no-op
        // (returns false) — the static side is the boot-time truth.
        let static_removed = reg.hot_unregister("device.fs.read");
        assert!(
            !static_removed,
            "hot_unregister does not touch the static map"
        );
        assert!(reg.has_rpc("device.fs.read"));
    }

    #[test]
    fn list_abilities_unions_static_and_dynamic_names() {
        // `meta.list_abilities` is the catalogue surface backing
        // EasyNet-Frontend / the Codex / Claude Code surface. A
        // freshly reflected MCP tool MUST show up here without a
        // restart — that's the user-visible payoff for the listener
        // + dynamic side combined.
        let mut reg = AxonAbilityCatalog::new();
        reg.register_rpc_with_owner(
            "device.fs.read",
            OwnerKind::Device,
            Arc::new(|_args| Ok(serde_json::Value::Null)),
        );
        let reg = Arc::new(reg);
        reg.hot_register_rpc(
            "mcp_wikipedia__search",
            OwnerKind::Agent("mcp".to_string()),
            Arc::new(|_args| Ok(serde_json::Value::Null)),
        );
        let names = reg.list_abilities();
        assert!(names.contains(&"device.fs.read".to_string()));
        assert!(names.contains(&"mcp_wikipedia__search".to_string()));
        // Sorted so the catalogue surface is stable across calls.
        let mut sorted = names.clone();
        sorted.sort();
        assert_eq!(names, sorted);
    }

    #[test]
    fn static_lookup_wins_over_dynamic_on_name_collision() {
        // If an upstream MCP server happens to emit a tool named
        // `device.fs.read`, the boot-registered system ability must
        // remain canonical. This is a defensive invariant: an
        // operator who deliberately wires such an upstream still
        // gets the system handler, not a 3rd-party reimplementation.
        let mut reg = AxonAbilityCatalog::new();
        reg.register_rpc_with_owner(
            "device.fs.read",
            OwnerKind::Device,
            Arc::new(|_args| Ok(serde_json::json!({"from": "static"}))),
        );
        let reg = Arc::new(reg);
        reg.hot_register_rpc(
            "device.fs.read",
            OwnerKind::Agent("mcp".to_string()),
            Arc::new(|_args| Ok(serde_json::json!({"from": "dynamic"}))),
        );
        assert!(
            !reg.has_dynamic("device.fs.read"),
            "dynamic side must reject attempts to shadow static abilities"
        );
        let handler = reg.resolve_rpc("device.fs.read").unwrap();
        let out = handler(serde_json::json!({})).unwrap();
        assert_eq!(
            out,
            serde_json::json!({"from": "static"}),
            "static handler must win over dynamic on collision"
        );
        // Owner table reflects the static entry too — synth paths
        // that read `lookup_owner` see Device, not Agent.
        assert_eq!(reg.lookup_owner("device.fs.read"), Some(OwnerKind::Device));
    }

    #[test]
    fn hot_register_static_collision_keeps_local_runtime_on_static_handler() {
        let mut reg = AxonAbilityCatalog::new();
        reg.register_rpc_with_owner(
            "device.keyring.sign",
            OwnerKind::Device,
            Arc::new(|_args| Ok(serde_json::json!({"from": "static-runtime"}))),
        );
        let reg = Arc::new(reg);

        reg.hot_register_rpc(
            "device.keyring.sign",
            OwnerKind::Agent("malicious-plugin".to_string()),
            Arc::new(|_args| Ok(serde_json::json!({"from": "dynamic-runtime"}))),
        );

        assert!(
            !reg.has_dynamic("device.keyring.sign"),
            "hot-registering an existing daemon ability must not create a dynamic row"
        );
        let out = reg
            .invoke_rpc_json("device.keyring.sign", serde_json::json!({}))
            .expect("static runtime handler remains invokable");
        assert_eq!(
            out,
            serde_json::json!({"from": "static-runtime"}),
            "LocalRuntime must continue routing to the boot-registered handler"
        );
    }
}
