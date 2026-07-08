// EasyNet CLI — Axon ability catalogue
// ====================================
//
// File: src/daemon/ability/dispatch.rs
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

use std::collections::{BTreeMap, BTreeSet};
use std::future::Future;
use std::sync::Arc;

use anyhow::Context as _;
use easynet_axon::invocation::{
    make_ability, AbilityCallModes, AbilityContext, AbilityFn, AbilityOptions, AxonError,
    CallMode as AxonCallMode, LocalRuntime,
};
use serde_json::Value;
use tokio::sync::{broadcast, mpsc};

use crate::daemon::ability::{
    public_route_ability_from_descriptor_ref, AbilityControlPlaneAuthorityModeLookupError,
    AbilityControlPlaneError, AbilityControlPlaneKey, AbilityControlPlaneLookupError,
    AbilityControlPlaneRecord, AbilityControlPlaneRegistration, AbilityControlPlaneRegistry,
    AbilityImplSource, AuthorityScope, CallMode as DescriptorCallMode,
    HostedAgentDelegationContext, HostedAgentDelegationEnvelopeBinding, RuntimeEnv,
    HOSTED_AGENT_DELEGATION_METADATA_KEY,
};
use crate::daemon::invocation::routing::target::{CallMode, InvocationTarget, TargetScope};

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

/// Error raised when constructing an envelope-aware handler context.
///
/// What this is NOT: an Axon admission error. Axon has already accepted and
/// stored the invocation envelope before this projection is built. These
/// variants mean the daemon is trying to project an incomplete or malformed
/// runtime envelope into product handlers, which is a host boundary bug.
#[derive(Debug, thiserror::Error)]
pub enum EnvelopeContextError {
    #[error("EnvelopeContext.{field} must not be empty")]
    EmptyField { field: &'static str },
    #[error("EnvelopeContext.invocation_nonce must be exactly 16 bytes, got {got}")]
    InvalidNonceLength { got: usize },
}

/// Complete AXIOM invocation context exposed to envelope-aware handlers.
///
/// Invariant 1: every instance contains caller, callee, ability, subject,
/// nonce, causal context, and runtime invocation id. There is no default
/// constructor and no public field mutation path; handlers cannot observe a
/// half-populated tuple.
///
/// Invariant 2: `subject` is an explicit envelope field. The valid self-target
/// case is represented as `subject == callee`, never as `None` plus an
/// interpretation rule.
///
/// Invariant 3: hosted-agent delegation is optional EasyNet product metadata,
/// not an eighth Invocation tuple parameter. It is attached only after the
/// signed envelope tuple has been projected.
#[derive(Debug, Clone)]
pub struct EnvelopeContext {
    invocation_id: String,
    caller: String,
    callee: String,
    ability: String,
    subject: String,
    invocation_nonce: Vec<u8>,
    causal_context: Value,
    caller_signature: EnvelopeCallerSignature,
    hosted_agent_delegation: Option<HostedAgentDelegationContext>,
}

pub struct EnvelopeContextParts {
    pub invocation_id: String,
    pub caller: String,
    pub callee: String,
    pub ability: String,
    pub subject: String,
    pub invocation_nonce: Vec<u8>,
    pub causal_context: Value,
    pub caller_signature: EnvelopeCallerSignature,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct EnvelopeCallerSignature {
    algorithm: String,
    key_id_hint: String,
    signature: Vec<u8>,
}

impl EnvelopeCallerSignature {
    fn from_axon(signature: &easynet_axon::invocation::CallerSignature) -> Self {
        Self {
            algorithm: signature.algorithm.clone(),
            key_id_hint: signature.key_id_hint.clone(),
            signature: signature.signature.clone(),
        }
    }

    #[cfg(test)]
    fn deterministic_test() -> Self {
        Self {
            algorithm: "ed25519".to_string(),
            key_id_hint: "test-envelope-key".to_string(),
            signature: vec![0x5E; 64],
        }
    }

    #[must_use]
    pub fn algorithm(&self) -> &str {
        &self.algorithm
    }

    #[must_use]
    pub fn key_id_hint(&self) -> &str {
        &self.key_id_hint
    }

    #[must_use]
    pub fn signature(&self) -> &[u8] {
        &self.signature
    }
}

impl EnvelopeContext {
    /// Build a complete handler context from an Axon runtime envelope.
    ///
    /// The constructor validates tuple completeness but does not perform
    /// admission or receipt verification; those remain Axon responsibilities.
    pub fn new(parts: EnvelopeContextParts) -> Result<Self, EnvelopeContextError> {
        let invocation_id = required_context_field("invocation_id", parts.invocation_id)?;
        let caller = required_context_field("caller", parts.caller)?;
        let callee = required_context_field("callee", parts.callee)?;
        let ability = required_context_field("ability", parts.ability)?;
        let subject = required_context_field("subject", parts.subject)?;
        let invocation_nonce = parts.invocation_nonce;
        if invocation_nonce.len() != 16 {
            return Err(EnvelopeContextError::InvalidNonceLength {
                got: invocation_nonce.len(),
            });
        }
        Ok(Self {
            invocation_id,
            caller,
            callee,
            ability,
            subject,
            invocation_nonce,
            causal_context: parts.causal_context,
            caller_signature: parts.caller_signature,
            hosted_agent_delegation: None,
        })
    }

    /// Attach verified hosted-agent delegation metadata to this context.
    #[must_use]
    pub fn with_hosted_agent_delegation(
        mut self,
        hosted_agent_delegation: Option<HostedAgentDelegationContext>,
    ) -> Self {
        self.hosted_agent_delegation = hosted_agent_delegation;
        self
    }

    /// Replace the projected causal context while preserving tuple completeness.
    #[must_use]
    pub fn with_causal_context(mut self, causal_context: Value) -> Self {
        self.causal_context = causal_context;
        self
    }

    /// Build a deterministic complete context for unit tests.
    ///
    /// Tests must still state the subject they mean; there is intentionally no
    /// "empty context" helper because production cannot dispatch one.
    #[cfg(test)]
    pub fn for_test(caller: impl Into<String>, subject: impl Into<String>) -> Self {
        Self::for_test_ability(caller, "test.ability", subject)
    }

    #[cfg(test)]
    pub fn for_test_ability(
        caller: impl Into<String>,
        ability: impl Into<String>,
        subject: impl Into<String>,
    ) -> Self {
        let caller = caller.into();
        let ability = ability.into();
        let subject = subject.into();
        Self::new(EnvelopeContextParts {
            invocation_id: "test-invocation".to_string(),
            caller,
            callee: "easynet:///r/test/device/local".to_string(),
            ability,
            subject,
            invocation_nonce: vec![0xA5; 16],
            causal_context: serde_json::json!({"kind": "none"}),
            caller_signature: EnvelopeCallerSignature::deterministic_test(),
        })
        .expect("test EnvelopeContext must be complete")
    }

    #[must_use]
    pub fn invocation_id(&self) -> &str {
        &self.invocation_id
    }

    #[must_use]
    pub fn caller(&self) -> &str {
        &self.caller
    }

    #[must_use]
    pub fn callee(&self) -> &str {
        &self.callee
    }

    #[must_use]
    pub fn ability(&self) -> &str {
        &self.ability
    }

    #[must_use]
    pub fn subject(&self) -> &str {
        &self.subject
    }

    #[must_use]
    pub fn invocation_nonce(&self) -> &[u8] {
        &self.invocation_nonce
    }

    #[must_use]
    pub fn causal_context(&self) -> &Value {
        &self.causal_context
    }

    #[must_use]
    pub fn caller_signature(&self) -> &EnvelopeCallerSignature {
        &self.caller_signature
    }

    #[must_use]
    pub fn hosted_agent_delegation(&self) -> Option<&HostedAgentDelegationContext> {
        self.hosted_agent_delegation.as_ref()
    }
}

fn required_context_field(
    field: &'static str,
    value: String,
) -> Result<String, EnvelopeContextError> {
    let value = value.trim().to_string();
    if value.is_empty() {
        return Err(EnvelopeContextError::EmptyField { field });
    }
    Ok(value)
}

/// Executable implementation facts written with one control-plane
/// registration.
///
/// Invariant 1: the descriptor/authority/implementation record is written
/// exactly once for a registration attempt. Dynamic plugin and MCP abilities
/// must not first masquerade as native daemon code and then patch the record.
///
/// Invariant 2: `impl_content_hash` is optional but, when present, belongs to
/// the implementation binding, not to discovery metadata or handler maps.
#[derive(Debug, Clone)]
pub struct ControlPlaneImplementation {
    impl_source: AbilityImplSource,
    runtime_env: RuntimeEnv,
    impl_content_hash: Option<String>,
}

impl ControlPlaneImplementation {
    /// Build an implementation binding for a non-native registration path.
    #[must_use]
    pub fn new(impl_source: AbilityImplSource, runtime_env: RuntimeEnv) -> Self {
        Self {
            impl_source,
            runtime_env,
            impl_content_hash: None,
        }
    }

    /// Build the canonical binding for daemon-native registrations.
    #[must_use]
    pub fn native_daemon() -> Self {
        Self::new(AbilityImplSource::NativeDaemon, RuntimeEnv::daemon_native())
    }

    /// Attach an immutable implementation content hash.
    #[must_use]
    pub fn with_content_hash(mut self, impl_content_hash: impl Into<String>) -> Self {
        self.impl_content_hash = Some(impl_content_hash.into());
        self
    }
}

struct ControlPlaneRegistrationRequest<'a> {
    ability: &'a str,
    owner: &'a OwnerKind,
    manifest: Option<&'a crate::core::ability::spec::AbilityManifest>,
    call_mode: DescriptorCallMode,
    implementation: ControlPlaneImplementation,
}

struct ResolvedControlPlaneRegistration<'a> {
    ability: &'a str,
    authority_scope: AuthorityScope,
    manifest: Option<&'a crate::core::ability::spec::AbilityManifest>,
    call_mode: DescriptorCallMode,
    implementation: ControlPlaneImplementation,
    owner_label: String,
}

pub struct ControlPlaneAuthorityRebind<'a> {
    pub ability: &'a str,
    pub authority_scope: AuthorityScope,
    pub manifest: Option<&'a crate::core::ability::spec::AbilityManifest>,
    pub call_mode: DescriptorCallMode,
    pub implementation: ControlPlaneImplementation,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RuntimeBindingFacts {
    pub descriptor_version: String,
    pub call_mode: DescriptorCallMode,
    pub schema_hash: String,
    pub descriptor_hash: Option<String>,
    pub implementation_source: String,
    pub implementation_content_hash: Option<String>,
    pub runtime_env: String,
    pub authority_owner_projection: String,
    pub authority_root: String,
    pub governs_advertise: bool,
    pub governs_invoke: bool,
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
/// (daemon/control/server.rs) so a single saturated session
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
    fn is_empty(&self) -> bool {
        self.rpc.is_none()
            && self.stream.is_none()
            && self.bidi.is_none()
            && self.rpc_with_env.is_none()
            && self.stream_with_env.is_none()
            && self.bidi_with_env.is_none()
    }

    fn modes(&self) -> AbilityCallModes {
        AbilityCallModes {
            rpc: self.rpc.is_some() || self.rpc_with_env.is_some(),
            stream: self.stream.is_some() || self.stream_with_env.is_some(),
            bidi: self.bidi.is_some() || self.bidi_with_env.is_some(),
        }
    }

    fn counts(&self) -> ExecutionIndexCounts {
        ExecutionIndexCounts {
            rpc: usize::from(self.rpc.is_some()),
            stream: usize::from(self.stream.is_some()),
            bidi: usize::from(self.bidi.is_some()),
            rpc_with_env: usize::from(self.rpc_with_env.is_some()),
            stream_with_env: usize::from(self.stream_with_env.is_some()),
            bidi_with_env: usize::from(self.bidi_with_env.is_some()),
        }
    }

    fn slots(&self) -> Vec<HandlerSlotKind> {
        let mut slots = Vec::new();
        if self.rpc.is_some() {
            slots.push(HandlerSlotKind::Rpc);
        }
        if self.stream.is_some() {
            slots.push(HandlerSlotKind::Stream);
        }
        if self.bidi.is_some() {
            slots.push(HandlerSlotKind::Bidi);
        }
        if self.rpc_with_env.is_some() {
            slots.push(HandlerSlotKind::RpcWithEnvelope);
        }
        if self.stream_with_env.is_some() {
            slots.push(HandlerSlotKind::StreamWithEnvelope);
        }
        if self.bidi_with_env.is_some() {
            slots.push(HandlerSlotKind::BidiWithEnvelope);
        }
        slots
    }

    fn remove_mode(&mut self, call_mode: DescriptorCallMode) -> bool {
        match call_mode {
            DescriptorCallMode::Rpc => {
                let removed = self.rpc.is_some() || self.rpc_with_env.is_some();
                self.rpc = None;
                self.rpc_with_env = None;
                removed
            }
            DescriptorCallMode::Stream => {
                let removed = self.stream.is_some() || self.stream_with_env.is_some();
                self.stream = None;
                self.stream_with_env = None;
                removed
            }
            DescriptorCallMode::Bidi => {
                let removed = self.bidi.is_some() || self.bidi_with_env.is_some();
                self.bidi = None;
                self.bidi_with_env = None;
                removed
            }
        }
    }

    fn install_static(&mut self, handler: StaticRegistrationHandler) {
        match handler {
            StaticRegistrationHandler::Rpc(handler) => self.rpc = Some(handler),
            StaticRegistrationHandler::Stream(handler) => self.stream = Some(handler),
            StaticRegistrationHandler::Bidi(handler) => self.bidi = Some(handler),
            StaticRegistrationHandler::RpcWithEnvelope(handler) => {
                self.rpc_with_env = Some(handler)
            }
            StaticRegistrationHandler::StreamWithEnvelope(handler) => {
                self.stream_with_env = Some(handler);
            }
            StaticRegistrationHandler::BidiWithEnvelope(handler) => {
                self.bidi_with_env = Some(handler);
            }
        }
    }

    fn install_dynamic(&mut self, handler: DynamicRegistrationHandler) {
        match handler {
            DynamicRegistrationHandler::Rpc(handler) => self.rpc = Some(handler),
            DynamicRegistrationHandler::Stream(handler) => self.stream = Some(handler),
            DynamicRegistrationHandler::RpcWithEnvelope(handler) => {
                self.rpc_with_env = Some(handler);
            }
            DynamicRegistrationHandler::StreamWithEnvelope(handler) => {
                self.stream_with_env = Some(handler);
            }
            DynamicRegistrationHandler::BidiWithEnvelope(handler) => {
                self.bidi_with_env = Some(handler);
            }
        }
    }

    fn fill_missing_from(&mut self, other: &Self) {
        if self.rpc.is_none() {
            self.rpc = other.rpc.as_ref().map(Arc::clone);
        }
        if self.stream.is_none() {
            self.stream = other.stream.as_ref().map(Arc::clone);
        }
        if self.bidi.is_none() {
            self.bidi = other.bidi.as_ref().map(Arc::clone);
        }
        if self.rpc_with_env.is_none() {
            self.rpc_with_env = other.rpc_with_env.as_ref().map(Arc::clone);
        }
        if self.stream_with_env.is_none() {
            self.stream_with_env = other.stream_with_env.as_ref().map(Arc::clone);
        }
        if self.bidi_with_env.is_none() {
            self.bidi_with_env = other.bidi_with_env.as_ref().map(Arc::clone);
        }
    }

    fn resolve_rpc(&self) -> Option<LocalRpcHandler> {
        self.rpc.as_ref().map(Arc::clone)
    }

    fn resolve_stream(&self) -> Option<LocalStreamHandler> {
        self.stream.as_ref().map(Arc::clone)
    }

    fn resolve_stream_with_env(&self) -> Option<LocalStreamHandlerWithEnvelope> {
        self.stream_with_env.as_ref().map(Arc::clone)
    }

    fn resolve_bidi(&self) -> Option<LocalBidiHandler> {
        self.bidi.as_ref().map(Arc::clone)
    }

    fn resolve_bidi_with_env(&self) -> Option<LocalBidiHandlerWithEnvelope> {
        self.bidi_with_env.as_ref().map(Arc::clone)
    }

    fn resolve_rpc_with_env(&self) -> Option<LocalRpcHandlerWithEnvelope> {
        self.rpc_with_env.as_ref().map(Arc::clone)
    }
}

fn payload_to_json_value(payload: &[u8]) -> Result<Value, Box<AxonError>> {
    if payload.is_empty() {
        Ok(Value::Object(Default::default()))
    } else {
        serde_json::from_slice(payload).map_err(|err| {
            Box::new(AxonError::invalid_argument(format!(
                "local_runtime_adapter: payload not JSON: {err}"
            )))
        })
    }
}

// Err boxed (clippy result_large_err): keeps the hot Ok path from
// carrying the ≥144 B AxonError variant.
fn json_value_to_payload(value: &Value) -> Result<Vec<u8>, Box<AxonError>> {
    serde_json::to_vec(value).map_err(|err| {
        Box::new(AxonError::internal(format!(
            "local_runtime_adapter: encode JSON: {err}"
        )))
    })
}

pub(crate) fn rpc_handler_to_ability_fn(handler: LocalRpcHandler) -> AbilityFn {
    make_ability(move |ctx| {
        let handler = Arc::clone(&handler);
        let payload = ctx.payload.clone();
        async move {
            let value = payload_to_json_value(&payload).map_err(|e| *e)?;
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
            json_value_to_payload(&result).map_err(|e| *e)
        }
    })
}

fn parse_hosted_agent_delegation_context(
    metadata: &std::collections::HashMap<String, String>,
    envelope: &HostedAgentDelegationEnvelopeBinding,
) -> Result<Option<HostedAgentDelegationContext>, AxonError> {
    let Some(raw) = metadata.get(HOSTED_AGENT_DELEGATION_METADATA_KEY) else {
        return Ok(None);
    };
    HostedAgentDelegationContext::from_signed_metadata(
        raw,
        envelope,
        crate::daemon::identity::local_invocation::system_verifying_key(),
    )
    .map(Some)
    .map_err(|err| AxonError::invalid_argument(format!("hosted_agent_delegation: {err}")))
}

async fn envelope_context_from_axon(
    ctx: &Arc<AbilityContext>,
) -> Result<EnvelopeContext, AxonError> {
    let signed = ctx
        .runtime
        .axiom_envelope_of(&ctx.invocation_id)
        .await
        .ok_or_else(|| {
            AxonError::internal(format!(
                "local_runtime_adapter: missing Axon envelope for invocation {}",
                ctx.invocation_id
            ))
        })?;
    let envelope = signed.envelope;
    let caller = envelope.caller.ura;
    let callee = envelope.callee.ura;
    let envelope_subject = envelope.subject.ura;
    let invocation_nonce = envelope.invocation_nonce;
    let invocation_nonce_hex = hex::encode(invocation_nonce.as_slice());
    let ability = envelope.ability;
    let hosted_agent_route_ability =
        public_route_ability_from_descriptor_ref(&ability).map_err(|err| {
            AxonError::invalid_argument(format!(
                "hosted_agent_delegation envelope route ability: {err}"
            ))
        })?;
    let caller_signature = EnvelopeCallerSignature::from_axon(&signed.signature);
    let hosted_agent_envelope = HostedAgentDelegationEnvelopeBinding::new(
        &caller,
        &callee,
        &envelope_subject,
        &invocation_nonce_hex,
        &hosted_agent_route_ability,
    )
    .map_err(|err| {
        AxonError::invalid_argument(format!("hosted_agent_delegation envelope binding: {err}"))
    })?;
    let hosted_agent_delegation =
        parse_hosted_agent_delegation_context(&ctx.request_metadata, &hosted_agent_envelope)?;
    EnvelopeContext::new(EnvelopeContextParts {
        invocation_id: ctx.invocation_id.clone(),
        caller,
        callee,
        ability,
        subject: envelope_subject,
        invocation_nonce: invocation_nonce.to_vec(),
        causal_context: causal_context_to_json(&envelope.causal_context),
        caller_signature,
    })
    .map(|context| context.with_hosted_agent_delegation(hosted_agent_delegation))
    .map_err(|err| {
        AxonError::internal(format!(
            "local_runtime_adapter: incomplete Axon envelope projection: {err}"
        ))
    })
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
            let value = payload_to_json_value(&ctx.payload).map_err(|e| *e)?;
            let env = envelope_context_from_axon(&ctx).await?;
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
            json_value_to_payload(&result).map_err(|e| *e)
        }
    })
}

async fn emit_json_progress(ctx: &Arc<AbilityContext>, value: Value) -> Result<(), AxonError> {
    let payload = json_value_to_payload(&value).map_err(|e| *e)?;
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
            let value = payload_to_json_value(&ctx.payload).map_err(|e| *e)?;
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
            let value = payload_to_json_value(&ctx.payload).map_err(|e| *e)?;
            let env = envelope_context_from_axon(&ctx).await?;
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

/// Project an envelope-aware stream handler into the `(AbilityFn,
/// AbilityOptions)` pair a direct `LocalRuntime::replace_ability` call
/// needs, with `modes.stream = true` so locality and dispatch treat it
/// as a server-stream. This is for lower-level registrars that already
/// own their control-plane transaction and need only the Axon runtime
/// adapter pair.
pub(crate) fn stream_env_ability_with_options(
    handler: LocalStreamHandlerWithEnvelope,
) -> (AbilityFn, AbilityOptions) {
    let modes = AbilityCallModes {
        rpc: false,
        stream: true,
        bidi: false,
    };
    (
        stream_env_handler_to_ability_fn(handler),
        AbilityOptions::default().with_modes(modes),
    )
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
                        let value = payload_to_json_value(&msg.payload).map_err(|e| *e)?;
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
            let value = payload_to_json_value(&ctx.payload).map_err(|e| *e)?;
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
            let value = payload_to_json_value(&ctx.payload).map_err(|e| *e)?;
            let env = envelope_context_from_axon(&ctx).await?;
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
    /// Governed by this daemon's device authority.
    ///
    /// The physical device is only the hosting substrate: disk, PTY,
    /// keyring, screen, microphone, and local daemon process. The
    /// device-profile Agent is the control-plane projection that
    /// advertises these abilities; the authority binding remains
    /// anchored to the device URA.
    ///
    /// Examples: `fs.read`, `terminal.create`, `session.list`,
    /// `node.list`, `skill.list`, `device.keyring.sign`.
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
    /// Hosted by the user's account agent. Axon Ability URA ownership
    /// has Hub, Device, and Agent branches, but no raw User branch; the
    /// contained string is therefore projected to
    /// `agent/<user-id>.account` at the protocol boundary while the
    /// product owner projection remains `user:<id>`.
    User(String),
}

impl OwnerKind {
    fn authority_scope(
        &self,
        context: &AbilityAuthorityContext,
    ) -> Result<AuthorityScope, AbilityControlPlaneError> {
        match self {
            OwnerKind::Device => {
                AuthorityScope::new("device", context.device_authority_root.clone())
            }
            OwnerKind::Hub => AuthorityScope::new("hub", context.hub_authority_root.clone()),
            OwnerKind::Agent(agent_id) => AuthorityScope::new(
                format!("agent:{agent_id}"),
                context.agent_authority_root(agent_id),
            ),
            OwnerKind::User(user_id) => AuthorityScope::new(
                format!("user:{user_id}"),
                context.user_authority_root(user_id),
            ),
        }
    }
}

/// Inverse of [`OwnerKind::authority_scope`]'s `owner_projection` encoding:
/// reconstruct the `OwnerKind` from the canonical projection string a
/// control-plane record stores (`device` / `hub` / `agent:<id>` /
/// `user:<id>`). Kept adjacent to the forward mapping so the two cannot
/// drift. Returns `None` for an unrecognized projection rather than
/// guessing — an owner the registry never wrote is not an owner.
fn owner_kind_from_projection(owner_projection: &str) -> Option<OwnerKind> {
    match owner_projection {
        "device" => Some(OwnerKind::Device),
        "hub" => Some(OwnerKind::Hub),
        other => {
            if let Some(agent_id) = other.strip_prefix("agent:") {
                Some(OwnerKind::Agent(agent_id.to_string()))
            } else {
                other
                    .strip_prefix("user:")
                    .map(|user_id| OwnerKind::User(user_id.to_string()))
            }
        }
    }
}

/// Process-local authority roots used when projecting owner kinds into
/// descriptor authority records and Axon `LocalRuntime` ability keys.
///
/// Production registries build this from the local daemon environment.
/// Embedded/test daemons can inject the concrete device URA they serve so
/// registration, control-plane lookup, and carrier-v1 dispatch all resolve
/// the same owner identity instead of drifting through global config.
#[derive(Debug, Clone)]
pub struct AbilityAuthorityContext {
    device_authority_root: String,
    hub_authority_root: String,
    source: AbilityAuthoritySource,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum AbilityAuthoritySource {
    LocalEnvironment,
    FixedDevice,
}

impl Default for AbilityAuthorityContext {
    fn default() -> Self {
        Self::from_local_environment()
    }
}

impl AbilityAuthorityContext {
    pub fn from_local_environment() -> Self {
        Self {
            device_authority_root: local_device_authority_root(),
            hub_authority_root: local_hub_authority_root(),
            source: AbilityAuthoritySource::LocalEnvironment,
        }
    }

    pub fn for_device_authority_root(
        device_authority_root: impl Into<String>,
    ) -> Result<Self, AbilityControlPlaneError> {
        let device_authority_root = device_authority_root.into();
        let parsed = crate::core::ura::parse_ura(&device_authority_root).map_err(|error| {
            AbilityControlPlaneError::InvalidDeviceAuthorityRoot {
                authority_root: device_authority_root.clone(),
                reason: error.to_string(),
            }
        })?;
        if parsed.kind != crate::core::ura::URAKind::Device {
            return Err(AbilityControlPlaneError::InvalidDeviceAuthorityRoot {
                authority_root: device_authority_root,
                reason: format!("expected /device/ URA, got {:?}", parsed.kind),
            });
        }
        Ok(Self {
            device_authority_root,
            hub_authority_root: crate::core::ura::hub_ura(&parsed.realm),
            source: AbilityAuthoritySource::FixedDevice,
        })
    }

    fn agent_authority_root(&self, agent_id: &str) -> String {
        if self.source == AbilityAuthoritySource::FixedDevice {
            return self.device_scoped_agent_authority_root(agent_id);
        }
        if let Ok(local_agents) = crate::daemon::persistence::local_agents::load() {
            if let Ok(Some(entry)) =
                crate::daemon::persistence::local_agents::lookup_hosted_agent_by_name(
                    &local_agents,
                    agent_id,
                )
            {
                return entry.agent_ura.clone();
            }
        }
        match crate::daemon::persistence::config::load_credentials() {
            Ok(creds) => match creds.user_id() {
                Ok(user_id) => crate::core::ura::agent_ura(&creds.realm, user_id, agent_id),
                Err(_) => {
                    crate::core::ura::device_agent_ura(&creds.realm, &creds.node_id, agent_id)
                }
            },
            Err(_) => self.device_scoped_agent_authority_root(agent_id),
        }
    }

    fn user_authority_root(&self, user_id: &str) -> String {
        if self.source == AbilityAuthoritySource::FixedDevice {
            return self.device_scoped_user_authority_root(user_id);
        }
        let realm = crate::daemon::persistence::config::load_credentials()
            .ok()
            .map(|creds| creds.realm)
            .or_else(|| {
                crate::core::ura::parse_ura(&self.device_authority_root)
                    .ok()
                    .map(|ura| ura.realm)
            })
            .unwrap_or_else(|| {
                crate::daemon::identity::local_invocation::UNPAIRED_LOCAL_REALM.to_string()
            });
        crate::core::ura::agent_ura(&realm, user_id, "account")
    }

    fn device_scoped_agent_authority_root(&self, agent_id: &str) -> String {
        let parsed = crate::core::ura::parse_ura(&self.device_authority_root).ok();
        let realm = parsed
            .as_ref()
            .map(|ura| ura.realm.as_str())
            .unwrap_or(crate::daemon::identity::local_invocation::UNPAIRED_LOCAL_REALM);
        let device_id = parsed
            .as_ref()
            .and_then(|ura| ura.device_id())
            .unwrap_or(crate::daemon::identity::local_invocation::UNPAIRED_LOCAL_DEVICE_ID);
        crate::core::ura::device_agent_ura(realm, device_id, agent_id)
    }

    fn device_scoped_user_authority_root(&self, user_id: &str) -> String {
        let parsed = crate::core::ura::parse_ura(&self.device_authority_root).ok();
        let realm = parsed
            .as_ref()
            .map(|ura| ura.realm.as_str())
            .unwrap_or(crate::daemon::identity::local_invocation::UNPAIRED_LOCAL_REALM);
        crate::core::ura::agent_ura(realm, user_id, "account")
    }
}

fn local_device_authority_root() -> String {
    crate::daemon::identity::local_invocation::local_device_ura()
}

fn local_hub_authority_root() -> String {
    let realm = crate::daemon::persistence::config::load_credentials()
        .ok()
        .map(|creds| creds.realm)
        .unwrap_or_else(|| {
            crate::daemon::identity::local_invocation::UNPAIRED_LOCAL_REALM.to_string()
        });
    crate::core::ura::hub_ura(&realm)
}

fn local_runtime_ability_key_for_authority(
    authority_root: &str,
    ability: &str,
) -> anyhow::Result<String> {
    crate::daemon::axon_bridge::descriptor_ref::ability_ura_for_wire(authority_root, ability)
        .map_err(|err| anyhow::anyhow!("{err}"))
}

#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord)]
struct ControlPlaneAbilityKey {
    authority_root: String,
    ability: String,
}

impl ControlPlaneAbilityKey {
    fn new(authority_root: impl Into<String>, ability: impl Into<String>) -> Self {
        Self {
            authority_root: authority_root.into(),
            ability: ability.into(),
        }
    }

    fn from_record(record: &AbilityControlPlaneRecord) -> Self {
        Self::new(
            record.authority().scope().authority_root(),
            record.descriptor().name(),
        )
    }

    fn authority_root(&self) -> &str {
        &self.authority_root
    }

    fn ability(&self) -> &str {
        &self.ability
    }

    fn runtime_key(&self) -> anyhow::Result<String> {
        local_runtime_ability_key_for_authority(&self.authority_root, &self.ability)
    }

    fn for_mode(&self, call_mode: DescriptorCallMode) -> ControlPlaneModeKey {
        ControlPlaneModeKey {
            key: self.clone(),
            call_mode,
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
struct ControlPlaneModeKey {
    key: ControlPlaneAbilityKey,
    call_mode: DescriptorCallMode,
}

impl ControlPlaneModeKey {
    fn from_record(record: &AbilityControlPlaneRecord) -> Self {
        ControlPlaneAbilityKey::from_record(record).for_mode(record.descriptor().call_mode())
    }

    fn ability(&self) -> &str {
        self.key.ability()
    }

    fn authority_root(&self) -> &str {
        self.key.authority_root()
    }

    fn call_mode(&self) -> DescriptorCallMode {
        self.call_mode
    }
}

#[derive(Debug, Clone)]
pub struct AbilityCatalogSnapshotRow {
    pub name: String,
    pub owner: Option<OwnerKind>,
    pub manifest: Option<Arc<crate::core::ability::spec::AbilityManifest>>,
}

#[derive(Debug, Clone)]
struct AbilityCatalogSnapshotBuilder {
    owner: Option<OwnerKind>,
    ambiguous_owner: bool,
    manifest: Option<Arc<crate::core::ability::spec::AbilityManifest>>,
    ambiguous_manifest: bool,
}

impl AbilityCatalogSnapshotBuilder {
    fn new(owner: Option<OwnerKind>) -> Self {
        Self {
            owner,
            ambiguous_owner: false,
            manifest: None,
            ambiguous_manifest: false,
        }
    }

    fn observe_owner(&mut self, next: Option<OwnerKind>) {
        if self.ambiguous_owner {
            return;
        }
        match (&self.owner, next) {
            (None, owner) => self.owner = owner,
            (Some(current), Some(next)) if *current == next => {}
            (Some(_), Some(_)) | (Some(_), None) => {
                self.owner = None;
                self.ambiguous_owner = true;
            }
        }
    }

    fn observe_manifest(&mut self, next: Option<Arc<crate::core::ability::spec::AbilityManifest>>) {
        let Some(next) = next else {
            return;
        };
        match self.manifest.as_ref() {
            None if !self.ambiguous_manifest => self.manifest = Some(next),
            Some(current) if current.descriptor_version() == next.descriptor_version() => {}
            Some(_) | None => {
                self.manifest = None;
                self.ambiguous_manifest = true;
            }
        }
    }

    fn into_row(self, name: String) -> AbilityCatalogSnapshotRow {
        AbilityCatalogSnapshotRow {
            name,
            owner: if self.ambiguous_owner {
                None
            } else {
                self.owner
            },
            manifest: if self.ambiguous_manifest {
                None
            } else {
                self.manifest
            },
        }
    }
}

/// Axon ability catalogue. Keyed by full ability name. v1 shape is
/// a `BTreeMap` for deterministic iteration order; the catalogue is
/// read-mostly (built once at daemon start, queried for metadata), so
/// RwLock + per-invocation hash is overkill.
///
/// Hot-reload note: the single execution index below is written at boot and
/// post-boot. Each row is keyed by the authority-scoped runtime ability key
/// (`authority_root`, `ability`) and tagged with its lifecycle origin
/// (`Static` or `Dynamic`). `RegistryRefreshSink` can mutate dynamic rows
/// through `&self`, while boot-time rows remain immutable by origin guard.
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
    // ── Execution index (SPEC §9.1.A) ────────────────────────
    // This is a PURE EXECUTION INDEX, not a metadata store. Canonical owner /
    // authority / manifest / call-mode truth lives ONLY in `control_plane`,
    // keyed by `AbilityControlPlaneKey(authority_root, ability, version,
    // call_mode)`. The execution index is keyed by the runtime aggregation key
    // `ControlPlaneAbilityKey(authority_root, ability)` because one Axon
    // LocalRuntime ability may expose several call modes under the same
    // authority.
    execution_index: std::sync::RwLock<ExecutionIndex>,
    /// Authority roots for projecting owner kinds into protocol and runtime
    /// identities. This is a catalog property, not a global lookup, so an
    /// embedded daemon's runtime surface is bound to the device identity it
    /// actually serves.
    authority_context: AbilityAuthorityContext,
    /// Descriptor/authority/implementation binding records keyed by the
    /// typed control-plane key. This is the canonical owner / authority /
    /// manifest / call-mode truth (SPEC §9.1.A); the handler maps above are
    /// an execution index only. The former `owner` / `manifests` side tables
    /// have been removed in favour of `control_plane_owner` /
    /// `control_plane_manifest` read-throughs.
    control_plane: std::sync::RwLock<AbilityControlPlaneRegistry>,
    /// Manifest bodies, keyed by the SAME `AbilityControlPlaneKey` as the
    /// control-plane record they belong to (SPEC §9.1.A Step 3). The
    /// control-plane registry is deliberately hash-pure — it stores
    /// `schema_hash`, not the schema body — so the manifest body (needed by
    /// `meta.list_abilities` to surface `input_schema`/`output_schema`) is
    /// the catalog-owned facet of the record, committed in lockstep with it.
    ///
    /// Keying by the control-plane key (not a bare ability `String`) makes
    /// this the manifest facet of the one record rather than a parallel
    /// truth: a multi-call-mode ability has one manifest row per mode, and a
    /// missing record cannot have a manifest fall through (acceptance test
    /// 5). There is no String-keyed manifest fallback.
    control_plane_manifests: std::sync::RwLock<
        BTreeMap<AbilityControlPlaneKey, Arc<crate::core::ability::spec::AbilityManifest>>,
    >,
    /// Serializes post-boot dynamic catalogue transactions.
    ///
    /// Invariant 1: dynamic execution-index rows, dynamic control-plane
    /// records, and `LocalRuntime` replacement are one transaction from the
    /// caller's point of view. No second hot-register or hot-unregister may
    /// observe or overwrite the middle of that transaction.
    ///
    /// Invariant 2: static boot registration never takes this mutex; static
    /// rows are built before the catalogue is shared through `Arc`.
    dynamic_txn: std::sync::Mutex<()>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum ExecutionOrigin {
    Static,
    Dynamic,
}

#[derive(Debug, Clone, Copy, Default)]
struct ExecutionIndexCounts {
    rpc: usize,
    stream: usize,
    bidi: usize,
    rpc_with_env: usize,
    stream_with_env: usize,
    bidi_with_env: usize,
}

impl ExecutionIndexCounts {
    fn total(self) -> usize {
        self.rpc
            + self.stream
            + self.bidi
            + self.rpc_with_env
            + self.stream_with_env
            + self.bidi_with_env
    }
}

#[derive(Clone)]
struct ExecutionIndexEntry {
    origin: ExecutionOrigin,
    handlers: RuntimeHandlerSet,
}

/// Authority-keyed execution index for all daemon ability handlers.
///
/// The index is deliberately not a metadata table. It stores only executable
/// closures and their lifecycle origin. The authority key mirrors the Axon
/// runtime binding boundary: one `(authority_root, ability)` row can carry RPC,
/// stream, and bidi slots, while the control-plane registry remains the source
/// for descriptor version, schema hash, owner, manifest, and call-mode proofs.
#[derive(Default)]
struct ExecutionIndex {
    entries: BTreeMap<ControlPlaneAbilityKey, ExecutionIndexEntry>,
}

impl ExecutionIndex {
    fn counts(&self, origin: ExecutionOrigin) -> ExecutionIndexCounts {
        let mut counts = ExecutionIndexCounts::default();
        for entry in self.entries.values().filter(|entry| entry.origin == origin) {
            let entry_counts = entry.handlers.counts();
            counts.rpc += entry_counts.rpc;
            counts.stream += entry_counts.stream;
            counts.bidi += entry_counts.bidi;
            counts.rpc_with_env += entry_counts.rpc_with_env;
            counts.stream_with_env += entry_counts.stream_with_env;
            counts.bidi_with_env += entry_counts.bidi_with_env;
        }
        counts
    }

    fn dynamic_snapshot(&self, key: &ControlPlaneAbilityKey) -> DynamicAbilitySnapshot {
        self.entries
            .get(key)
            .filter(|entry| entry.origin == ExecutionOrigin::Dynamic)
            .map(|entry| DynamicAbilitySnapshot::from_handlers(entry.handlers.clone()))
            .unwrap_or_default()
    }

    fn restore_dynamic(&mut self, key: ControlPlaneAbilityKey, snapshot: DynamicAbilitySnapshot) {
        self.drain_dynamic(&key);
        if snapshot.has_handlers() {
            self.entries.insert(
                key,
                ExecutionIndexEntry {
                    origin: ExecutionOrigin::Dynamic,
                    handlers: snapshot.into_handlers(),
                },
            );
        }
    }

    fn install_static(&mut self, key: ControlPlaneAbilityKey, handler: StaticRegistrationHandler) {
        let entry = self
            .entries
            .entry(key)
            .or_insert_with(|| ExecutionIndexEntry {
                origin: ExecutionOrigin::Static,
                handlers: RuntimeHandlerSet::default(),
            });
        assert_eq!(
            entry.origin,
            ExecutionOrigin::Static,
            "static registration attempted to overwrite a dynamic execution row"
        );
        entry.handlers.install_static(handler);
    }

    fn install_dynamic(&mut self, key: ControlPlaneAbilityKey, registration: DynamicRegistration) {
        let DynamicRegistration {
            ability: _,
            owner: _,
            authority_scope: _,
            manifest: _,
            implementation: _,
            handler,
        } = registration;
        let call_mode = handler.call_mode();
        let entry = self
            .entries
            .entry(key)
            .or_insert_with(|| ExecutionIndexEntry {
                origin: ExecutionOrigin::Dynamic,
                handlers: RuntimeHandlerSet::default(),
            });
        assert_eq!(
            entry.origin,
            ExecutionOrigin::Dynamic,
            "dynamic registration attempted to overwrite a static execution row"
        );
        entry.handlers.remove_mode(call_mode);
        entry.handlers.install_dynamic(handler);
    }

    fn drain_static(&mut self, key: &ControlPlaneAbilityKey) -> bool {
        self.drain_origin(key, ExecutionOrigin::Static)
    }

    fn drain_dynamic(&mut self, key: &ControlPlaneAbilityKey) -> bool {
        self.drain_origin(key, ExecutionOrigin::Dynamic)
    }

    fn drain_origin(&mut self, key: &ControlPlaneAbilityKey, origin: ExecutionOrigin) -> bool {
        let present = self
            .entries
            .get(key)
            .map(|entry| entry.origin == origin && !entry.handlers.is_empty())
            .unwrap_or(false);
        if present {
            self.entries.remove(key);
        }
        present
    }

    fn contains_origin_handler_by_name(&self, ability: &str, origin: ExecutionOrigin) -> bool {
        self.entries.iter().any(|(key, entry)| {
            key.ability() == ability && entry.origin == origin && !entry.handlers.is_empty()
        })
    }

    fn origin_key_by_ability(
        &self,
        ability: &str,
        origin: ExecutionOrigin,
    ) -> anyhow::Result<Option<ControlPlaneAbilityKey>> {
        let keys = self
            .entries
            .iter()
            .filter(|(key, entry)| {
                key.ability() == ability && entry.origin == origin && !entry.handlers.is_empty()
            })
            .map(|(key, _)| key.clone())
            .collect::<Vec<_>>();
        match keys.as_slice() {
            [] => Ok(None),
            [key] => Ok(Some(key.clone())),
            _ => anyhow::bail!(
                "ability {ability:?} has multiple {origin:?} execution authority keys {keys:?}"
            ),
        }
    }

    fn slots(&self, key: &ControlPlaneAbilityKey) -> Vec<HandlerSlotKind> {
        self.entries
            .get(key)
            .map(|entry| entry.handlers.slots())
            .unwrap_or_default()
    }

    fn names(&self, origin: ExecutionOrigin) -> Vec<String> {
        let mut names = BTreeSet::new();
        for (key, entry) in &self.entries {
            if entry.origin == origin && !entry.handlers.is_empty() {
                names.insert(key.ability().to_string());
            }
        }
        names.into_iter().collect()
    }

    fn handlers_for_ability(&self, ability: &str) -> RuntimeHandlerSet {
        let mut handlers = RuntimeHandlerSet::default();
        for (key, entry) in self.entries.iter().filter(|(key, entry)| {
            key.ability() == ability && entry.origin == ExecutionOrigin::Static
        }) {
            let _ = key;
            handlers.fill_missing_from(&entry.handlers);
        }
        for (key, entry) in self.entries.iter().filter(|(key, entry)| {
            key.ability() == ability && entry.origin == ExecutionOrigin::Dynamic
        }) {
            let _ = key;
            handlers.fill_missing_from(&entry.handlers);
        }
        handlers
    }

    fn has_rpc(&self, ability: &str) -> bool {
        let handlers = self.handlers_for_ability(ability);
        handlers.rpc.is_some() || handlers.rpc_with_env.is_some()
    }

    fn has_stream(&self, ability: &str) -> bool {
        let handlers = self.handlers_for_ability(ability);
        handlers.stream.is_some() || handlers.stream_with_env.is_some()
    }

    fn has_bidi(&self, ability: &str) -> bool {
        let handlers = self.handlers_for_ability(ability);
        handlers.bidi.is_some() || handlers.bidi_with_env.is_some()
    }

    fn has_any_handler(&self, ability: &str) -> bool {
        !self.handlers_for_ability(ability).is_empty()
    }

    fn extend_rpc_names(&self, names: &mut BTreeSet<String>) {
        for (key, entry) in &self.entries {
            if entry.handlers.rpc.is_some() || entry.handlers.rpc_with_env.is_some() {
                names.insert(key.ability().to_string());
            }
        }
    }

    fn resolve_rpc(&self, ability: &str) -> Option<LocalRpcHandler> {
        self.handlers_for_ability(ability).resolve_rpc()
    }

    fn resolve_stream(&self, ability: &str) -> Option<LocalStreamHandler> {
        self.handlers_for_ability(ability).resolve_stream()
    }

    fn resolve_stream_with_env(&self, ability: &str) -> Option<LocalStreamHandlerWithEnvelope> {
        self.handlers_for_ability(ability).resolve_stream_with_env()
    }

    fn resolve_bidi(&self, ability: &str) -> Option<LocalBidiHandler> {
        self.handlers_for_ability(ability).resolve_bidi()
    }

    fn resolve_bidi_with_env(&self, ability: &str) -> Option<LocalBidiHandlerWithEnvelope> {
        self.handlers_for_ability(ability).resolve_bidi_with_env()
    }

    fn resolve_rpc_with_env(&self, ability: &str) -> Option<LocalRpcHandlerWithEnvelope> {
        self.handlers_for_ability(ability).resolve_rpc_with_env()
    }
}

#[derive(Clone, Default)]
struct DynamicAbilitySnapshot {
    rpc: Option<LocalRpcHandler>,
    stream: Option<LocalStreamHandler>,
    bidi: Option<LocalBidiHandler>,
    rpc_with_env: Option<LocalRpcHandlerWithEnvelope>,
    stream_with_env: Option<LocalStreamHandlerWithEnvelope>,
    bidi_with_env: Option<LocalBidiHandlerWithEnvelope>,
}

impl DynamicAbilitySnapshot {
    fn from_handlers(handlers: RuntimeHandlerSet) -> Self {
        Self {
            rpc: handlers.rpc,
            stream: handlers.stream,
            bidi: handlers.bidi,
            rpc_with_env: handlers.rpc_with_env,
            stream_with_env: handlers.stream_with_env,
            bidi_with_env: handlers.bidi_with_env,
        }
    }

    fn into_handlers(self) -> RuntimeHandlerSet {
        RuntimeHandlerSet {
            rpc: self.rpc,
            stream: self.stream,
            bidi: self.bidi,
            rpc_with_env: self.rpc_with_env,
            stream_with_env: self.stream_with_env,
            bidi_with_env: self.bidi_with_env,
        }
    }

    fn has_handlers(&self) -> bool {
        self.rpc.is_some()
            || self.stream.is_some()
            || self.bidi.is_some()
            || self.rpc_with_env.is_some()
            || self.stream_with_env.is_some()
            || self.bidi_with_env.is_some()
    }

    fn slots(&self) -> Vec<HandlerSlotKind> {
        let mut slots = Vec::new();
        if self.rpc.is_some() {
            slots.push(HandlerSlotKind::Rpc);
        }
        if self.stream.is_some() {
            slots.push(HandlerSlotKind::Stream);
        }
        if self.bidi.is_some() {
            slots.push(HandlerSlotKind::Bidi);
        }
        if self.rpc_with_env.is_some() {
            slots.push(HandlerSlotKind::RpcWithEnvelope);
        }
        if self.stream_with_env.is_some() {
            slots.push(HandlerSlotKind::StreamWithEnvelope);
        }
        if self.bidi_with_env.is_some() {
            slots.push(HandlerSlotKind::BidiWithEnvelope);
        }
        slots
    }

    fn conflicting_family_switches(&self, target_slot: HandlerSlotKind) -> Vec<HandlerSlotKind> {
        self.slots()
            .into_iter()
            .filter(|existing| *existing != target_slot && target_slot.conflicts_with(*existing))
            .collect()
    }
}

enum DynamicRegistrationHandler {
    Rpc(LocalRpcHandler),
    Stream(LocalStreamHandler),
    RpcWithEnvelope(LocalRpcHandlerWithEnvelope),
    StreamWithEnvelope(LocalStreamHandlerWithEnvelope),
    BidiWithEnvelope(LocalBidiHandlerWithEnvelope),
}

impl DynamicRegistrationHandler {
    fn call_mode(&self) -> DescriptorCallMode {
        match self {
            Self::Rpc(_) | Self::RpcWithEnvelope(_) => DescriptorCallMode::Rpc,
            Self::Stream(_) | Self::StreamWithEnvelope(_) => DescriptorCallMode::Stream,
            Self::BidiWithEnvelope(_) => DescriptorCallMode::Bidi,
        }
    }

    fn slot(&self) -> HandlerSlotKind {
        match self {
            Self::Rpc(_) => HandlerSlotKind::Rpc,
            Self::Stream(_) => HandlerSlotKind::Stream,
            Self::RpcWithEnvelope(_) => HandlerSlotKind::RpcWithEnvelope,
            Self::StreamWithEnvelope(_) => HandlerSlotKind::StreamWithEnvelope,
            Self::BidiWithEnvelope(_) => HandlerSlotKind::BidiWithEnvelope,
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum HandlerSlotKind {
    Rpc,
    Stream,
    Bidi,
    RpcWithEnvelope,
    StreamWithEnvelope,
    BidiWithEnvelope,
}

impl HandlerSlotKind {
    fn call_mode(self) -> DescriptorCallMode {
        match self {
            Self::Rpc | Self::RpcWithEnvelope => DescriptorCallMode::Rpc,
            Self::Stream | Self::StreamWithEnvelope => DescriptorCallMode::Stream,
            Self::Bidi | Self::BidiWithEnvelope => DescriptorCallMode::Bidi,
        }
    }

    fn label(self) -> &'static str {
        match self {
            Self::Rpc => "rpc",
            Self::Stream => "stream",
            Self::Bidi => "bidi",
            Self::RpcWithEnvelope => "rpc_with_env",
            Self::StreamWithEnvelope => "stream_with_env",
            Self::BidiWithEnvelope => "bidi_with_env",
        }
    }

    fn conflicts_with(self, existing: Self) -> bool {
        self.call_mode() == existing.call_mode()
    }
}

enum StaticRegistrationHandler {
    Rpc(LocalRpcHandler),
    Stream(LocalStreamHandler),
    Bidi(LocalBidiHandler),
    RpcWithEnvelope(LocalRpcHandlerWithEnvelope),
    StreamWithEnvelope(LocalStreamHandlerWithEnvelope),
    BidiWithEnvelope(LocalBidiHandlerWithEnvelope),
}

impl StaticRegistrationHandler {
    fn call_mode(&self) -> DescriptorCallMode {
        match self {
            Self::Rpc(_) | Self::RpcWithEnvelope(_) => DescriptorCallMode::Rpc,
            Self::Stream(_) | Self::StreamWithEnvelope(_) => DescriptorCallMode::Stream,
            Self::Bidi(_) | Self::BidiWithEnvelope(_) => DescriptorCallMode::Bidi,
        }
    }

    fn slot(&self) -> HandlerSlotKind {
        match self {
            Self::Rpc(_) => HandlerSlotKind::Rpc,
            Self::Stream(_) => HandlerSlotKind::Stream,
            Self::Bidi(_) => HandlerSlotKind::Bidi,
            Self::RpcWithEnvelope(_) => HandlerSlotKind::RpcWithEnvelope,
            Self::StreamWithEnvelope(_) => HandlerSlotKind::StreamWithEnvelope,
            Self::BidiWithEnvelope(_) => HandlerSlotKind::BidiWithEnvelope,
        }
    }
}

struct StaticRegistration {
    ability: String,
    owner: OwnerKind,
    authority_scope: Option<AuthorityScope>,
    manifest: Option<Arc<crate::core::ability::spec::AbilityManifest>>,
    implementation: ControlPlaneImplementation,
    handler: StaticRegistrationHandler,
}

impl StaticRegistration {
    fn new(
        ability: impl Into<String>,
        owner: OwnerKind,
        handler: StaticRegistrationHandler,
    ) -> Self {
        Self {
            ability: ability.into(),
            owner,
            authority_scope: None,
            manifest: None,
            implementation: ControlPlaneImplementation::native_daemon(),
            handler,
        }
    }

    fn with_manifest(mut self, manifest: crate::core::ability::spec::AbilityManifest) -> Self {
        self.manifest = Some(Arc::new(manifest));
        self
    }

    fn with_implementation(mut self, implementation: ControlPlaneImplementation) -> Self {
        self.implementation = implementation;
        self
    }

    fn with_authority_scope(mut self, authority_scope: AuthorityScope) -> Self {
        self.authority_scope = Some(authority_scope);
        self
    }

    fn commit(self, catalog: &mut AxonAbilityCatalog) -> anyhow::Result<()> {
        let Self {
            ability,
            owner,
            authority_scope,
            manifest,
            implementation,
            handler,
        } = self;
        let call_mode = handler.call_mode();
        let target_slot = handler.slot();
        let authority_scope = match authority_scope {
            Some(authority_scope) => authority_scope,
            None => catalog.resolve_authority_scope_for_owner(&ability, &owner)?,
        };
        let execution_key = ControlPlaneAbilityKey::new(authority_scope.authority_root(), &ability);
        catalog.assert_static_handler_slot_available(&execution_key, target_slot);
        catalog.register_control_plane_with_scope_result(
            &ability,
            authority_scope.clone(),
            manifest.as_ref().map(Arc::as_ref),
            call_mode,
            &implementation,
        )?;
        catalog
            .execution_index
            .write()
            .expect("execution_index RwLock poisoned")
            .install_static(execution_key, handler);
        catalog.sync_static_runtime_ability_or_panic(&ability);
        Ok(())
    }
}

struct DynamicRegistration {
    ability: String,
    owner: OwnerKind,
    authority_scope: Option<AuthorityScope>,
    manifest: Option<Arc<crate::core::ability::spec::AbilityManifest>>,
    implementation: ControlPlaneImplementation,
    handler: DynamicRegistrationHandler,
}

impl DynamicRegistration {
    fn new(
        ability: impl Into<String>,
        owner: OwnerKind,
        manifest: Option<Arc<crate::core::ability::spec::AbilityManifest>>,
        implementation: ControlPlaneImplementation,
        handler: DynamicRegistrationHandler,
    ) -> Self {
        Self {
            ability: ability.into(),
            owner,
            authority_scope: None,
            manifest,
            implementation,
            handler,
        }
    }

    fn rpc(ability: impl Into<String>, owner: OwnerKind, handler: LocalRpcHandler) -> Self {
        Self::new(
            ability,
            owner,
            None,
            ControlPlaneImplementation::native_daemon(),
            DynamicRegistrationHandler::Rpc(handler),
        )
    }

    fn rpc_with_spec(
        ability: impl Into<String>,
        owner: OwnerKind,
        manifest: crate::core::ability::spec::AbilityManifest,
        handler: LocalRpcHandler,
    ) -> Self {
        Self::new(
            ability,
            owner,
            Some(Arc::new(manifest)),
            ControlPlaneImplementation::native_daemon(),
            DynamicRegistrationHandler::Rpc(handler),
        )
    }

    fn stream_with_spec_and_impl(
        ability: impl Into<String>,
        owner: OwnerKind,
        manifest: crate::core::ability::spec::AbilityManifest,
        handler: LocalStreamHandler,
        implementation: ControlPlaneImplementation,
    ) -> Self {
        Self::new(
            ability,
            owner,
            Some(Arc::new(manifest)),
            implementation,
            DynamicRegistrationHandler::Stream(handler),
        )
    }

    fn rpc_with_envelope(
        ability: impl Into<String>,
        owner: OwnerKind,
        handler: LocalRpcHandlerWithEnvelope,
    ) -> Self {
        Self::new(
            ability,
            owner,
            None,
            ControlPlaneImplementation::native_daemon(),
            DynamicRegistrationHandler::RpcWithEnvelope(handler),
        )
    }

    fn rpc_with_envelope_and_spec_and_impl(
        ability: impl Into<String>,
        owner: OwnerKind,
        manifest: crate::core::ability::spec::AbilityManifest,
        handler: LocalRpcHandlerWithEnvelope,
        implementation: ControlPlaneImplementation,
    ) -> Self {
        Self::new(
            ability,
            owner,
            Some(Arc::new(manifest)),
            implementation,
            DynamicRegistrationHandler::RpcWithEnvelope(handler),
        )
    }

    fn stream_with_envelope(
        ability: impl Into<String>,
        owner: OwnerKind,
        handler: LocalStreamHandlerWithEnvelope,
    ) -> Self {
        Self::new(
            ability,
            owner,
            None,
            ControlPlaneImplementation::native_daemon(),
            DynamicRegistrationHandler::StreamWithEnvelope(handler),
        )
    }

    fn stream_with_envelope_and_spec_and_impl(
        ability: impl Into<String>,
        owner: OwnerKind,
        manifest: crate::core::ability::spec::AbilityManifest,
        handler: LocalStreamHandlerWithEnvelope,
        implementation: ControlPlaneImplementation,
    ) -> Self {
        Self::new(
            ability,
            owner,
            Some(Arc::new(manifest)),
            implementation,
            DynamicRegistrationHandler::StreamWithEnvelope(handler),
        )
    }

    fn bidi_with_envelope(
        ability: impl Into<String>,
        owner: OwnerKind,
        handler: LocalBidiHandlerWithEnvelope,
    ) -> Self {
        Self::new(
            ability,
            owner,
            None,
            ControlPlaneImplementation::native_daemon(),
            DynamicRegistrationHandler::BidiWithEnvelope(handler),
        )
    }

    fn bidi_with_envelope_and_spec_and_impl(
        ability: impl Into<String>,
        owner: OwnerKind,
        manifest: crate::core::ability::spec::AbilityManifest,
        handler: LocalBidiHandlerWithEnvelope,
        implementation: ControlPlaneImplementation,
    ) -> Self {
        Self::new(
            ability,
            owner,
            Some(Arc::new(manifest)),
            implementation,
            DynamicRegistrationHandler::BidiWithEnvelope(handler),
        )
    }

    fn ability(&self) -> &str {
        &self.ability
    }

    fn with_authority_scope(mut self, authority_scope: AuthorityScope) -> Self {
        self.authority_scope = Some(authority_scope);
        self
    }

    fn call_mode(&self) -> DescriptorCallMode {
        self.handler.call_mode()
    }

    fn manifest_ref(&self) -> Option<&crate::core::ability::spec::AbilityManifest> {
        self.manifest.as_ref().map(Arc::as_ref)
    }

    fn commit(self, catalog: &AxonAbilityCatalog) -> anyhow::Result<()> {
        let ability = self.ability().to_string();
        let call_mode = self.call_mode();
        let _dynamic_txn_guard = catalog
            .dynamic_txn
            .lock()
            .expect("dynamic_txn mutex poisoned");
        if catalog.reject_dynamic_shadow_of_static(&ability) {
            anyhow::bail!("dynamic ability {ability:?} shadows a static ability");
        }
        let authority_scope = match self.authority_scope.clone() {
            Some(authority_scope) => authority_scope,
            None => catalog.resolve_authority_scope_for_owner(&ability, &self.owner)?,
        };
        let predicted_execution_key =
            ControlPlaneAbilityKey::new(authority_scope.authority_root(), &ability);
        if let Some(prior_key) = catalog.dynamic_control_plane_key(&ability)? {
            if prior_key != predicted_execution_key {
                let prior_owner = catalog
                    .control_plane_owner(&ability)
                    .map(|owner| format!("{owner:?}"))
                    .unwrap_or_else(|| prior_key.authority_root().to_string());
                anyhow::bail!(
                    "dynamic ability {ability:?} is already owned by {prior_owner}; \
                     owner changes require hot_unregister before re-registering as {:?}",
                    self.owner
                );
            }
        }
        let prior_dynamic = catalog
            .execution_index
            .read()
            .expect("execution_index RwLock poisoned")
            .dynamic_snapshot(&predicted_execution_key);
        let target_slot = self.handler.slot();
        // Reject a hot re-registration that changes the owner: the prior
        // owner now comes from the control-plane record (the canonical owner
        // source since §9.1.A), not a dynamic execution snapshot. An owner change
        // requires an explicit hot_unregister first.
        if prior_dynamic.has_handlers() {
            if let Some(prior_owner) = catalog.control_plane_owner(&ability) {
                if prior_owner != self.owner {
                    anyhow::bail!(
                        "dynamic ability {ability:?} is already owned by {prior_owner:?}; owner changes require hot_unregister before re-registering as {:?}",
                        self.owner
                    );
                }
            }
        }
        let conflicting_slots = prior_dynamic.conflicting_family_switches(target_slot);
        if !conflicting_slots.is_empty() {
            let existing = conflicting_slots
                .iter()
                .map(|slot| slot.label())
                .collect::<Vec<_>>()
                .join(", ");
            anyhow::bail!(
                "dynamic ability {ability:?} already has handler family slot(s) {existing} for {:?}; \
                 hot_unregister before switching handler family to {}",
                target_slot.call_mode(),
                target_slot.label()
            );
        }
        let predicted_control_plane_key =
            ControlPlaneAbilityKey::new(authority_scope.authority_root(), &ability)
                .for_mode(call_mode);
        let control_plane_txn = catalog.begin_control_plane_authority_mode_transaction(
            predicted_control_plane_key.authority_root(),
            predicted_control_plane_key.ability(),
            predicted_control_plane_key.call_mode(),
        );
        let control_plane_key = catalog.register_dynamic_control_plane_with_scope_result(
            &ability,
            authority_scope.clone(),
            self.manifest_ref(),
            call_mode,
            &self.implementation,
        )?;
        debug_assert_eq!(control_plane_key, predicted_control_plane_key);
        let mut txn = DynamicRegistrationTxn::after_control_plane(
            catalog,
            control_plane_key,
            prior_dynamic,
            control_plane_txn,
        );
        {
            let mut execution_index = catalog
                .execution_index
                .write()
                .expect("execution_index RwLock poisoned");
            execution_index.install_dynamic(predicted_execution_key, self);
        }
        txn.mark_execution_index_committed()?;
        txn.sync_runtime_or_rollback()
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum ControlPlaneAuthorityModeTxnPhase {
    Active,
    Committed,
    RolledBack,
}

/// RAII transaction for one `(authority_root, ability, call_mode)` slice of the
/// daemon control plane.
///
/// This object keeps table internals behind `AxonAbilityCatalog`. Callers that
/// need to overwrite a descriptor/authority/implementation projection can begin
/// a transaction, perform their write, and call `commit` only after the outer
/// capability state machine reaches its own durable terminal state. Dropping an
/// active transaction restores the exact prior records.
pub struct ControlPlaneAuthorityModeTxn<'a> {
    catalog: &'a AxonAbilityCatalog,
    authority_root: String,
    ability: String,
    call_mode: DescriptorCallMode,
    prior_records: Option<Vec<AbilityControlPlaneRecord>>,
    /// Manifest-store entries for the affected `(authority_root, ability,
    /// call_mode)` slice, captured at `begin`. On rollback they are restored
    /// in lockstep with `prior_records` so a failed dynamic registration
    /// cannot strand a manifest body keyed by a control-plane key whose
    /// record was reverted (SPEC §9.1.A Step 3 / Step 7 follow-up).
    prior_manifests: Option<
        Vec<(
            AbilityControlPlaneKey,
            Arc<crate::core::ability::spec::AbilityManifest>,
        )>,
    >,
    phase: ControlPlaneAuthorityModeTxnPhase,
}

impl<'a> ControlPlaneAuthorityModeTxn<'a> {
    fn begin(
        catalog: &'a AxonAbilityCatalog,
        authority_root: &str,
        ability: &str,
        call_mode: DescriptorCallMode,
    ) -> Self {
        let prior_records = catalog
            .control_plane
            .read()
            .expect("control_plane RwLock poisoned")
            .records_for_authority_mode(authority_root, ability, call_mode);
        // Capture the manifest facet for the same slice so rollback can
        // restore it. Keyed by each prior record's control-plane key.
        let prior_manifests = {
            let manifests = catalog
                .control_plane_manifests
                .read()
                .expect("control_plane_manifests RwLock poisoned");
            prior_records
                .iter()
                .filter_map(|record| {
                    let key = AbilityControlPlaneKey::for_authority(record.authority());
                    manifests.get(&key).map(|m| (key, Arc::clone(m)))
                })
                .collect::<Vec<_>>()
        };
        Self {
            catalog,
            authority_root: authority_root.to_string(),
            ability: ability.to_string(),
            call_mode,
            prior_records: Some(prior_records),
            prior_manifests: Some(prior_manifests),
            phase: ControlPlaneAuthorityModeTxnPhase::Active,
        }
    }

    pub fn commit(mut self) {
        self.prior_records = None;
        self.prior_manifests = None;
        self.phase = ControlPlaneAuthorityModeTxnPhase::Committed;
    }

    pub fn rollback(&mut self) -> anyhow::Result<()> {
        if self.phase != ControlPlaneAuthorityModeTxnPhase::Active {
            return Ok(());
        }
        if let Some(records) = self.prior_records.take() {
            self.catalog
                .control_plane
                .write()
                .expect("control_plane RwLock poisoned")
                .restore_authority_mode_records(
                    &self.authority_root,
                    &self.ability,
                    self.call_mode,
                    records,
                )?;
        }
        // Restore the manifest facet to its pre-transaction state: drop any
        // entry the failed registration added for this slice, then re-insert
        // the captured snapshot. This prevents a rolled-back dynamic register
        // from stranding a manifest body whose control-plane record was just
        // reverted (SPEC §9.1.A Step 7 follow-up).
        if let Some(snapshot) = self.prior_manifests.take() {
            let mut manifests = self
                .catalog
                .control_plane_manifests
                .write()
                .expect("control_plane_manifests RwLock poisoned");
            manifests.retain(|key, _| {
                !(key.authority_root() == self.authority_root
                    && key.ability() == self.ability
                    && key.call_mode() == self.call_mode)
            });
            for (key, manifest) in snapshot {
                manifests.insert(key, manifest);
            }
        }
        self.phase = ControlPlaneAuthorityModeTxnPhase::RolledBack;
        Ok(())
    }
}

impl Drop for ControlPlaneAuthorityModeTxn<'_> {
    fn drop(&mut self) {
        let _ = self.rollback();
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum DynamicRegistrationPhase {
    ControlPlaneAccepted,
    ExecutionIndexCommitted,
    RuntimeSynced,
    RolledBack,
}

struct DynamicRegistrationTxn<'a> {
    catalog: &'a AxonAbilityCatalog,
    control_plane_key: ControlPlaneModeKey,
    prior_dynamic: Option<DynamicAbilitySnapshot>,
    control_plane_txn: Option<ControlPlaneAuthorityModeTxn<'a>>,
    phase: DynamicRegistrationPhase,
}

impl<'a> DynamicRegistrationTxn<'a> {
    fn after_control_plane(
        catalog: &'a AxonAbilityCatalog,
        control_plane_key: ControlPlaneModeKey,
        prior_dynamic: DynamicAbilitySnapshot,
        control_plane_txn: ControlPlaneAuthorityModeTxn<'a>,
    ) -> Self {
        Self {
            catalog,
            control_plane_key,
            prior_dynamic: Some(prior_dynamic),
            control_plane_txn: Some(control_plane_txn),
            phase: DynamicRegistrationPhase::ControlPlaneAccepted,
        }
    }

    fn mark_execution_index_committed(&mut self) -> anyhow::Result<()> {
        if self.phase != DynamicRegistrationPhase::ControlPlaneAccepted {
            let phase = self.phase;
            self.rollback();
            anyhow::bail!(
                "dynamic ability {:?} cannot commit execution index from phase {:?}",
                self.control_plane_key.ability(),
                phase
            );
        }
        self.phase = DynamicRegistrationPhase::ExecutionIndexCommitted;
        Ok(())
    }

    fn sync_runtime_or_rollback(&mut self) -> anyhow::Result<()> {
        if self.phase != DynamicRegistrationPhase::ExecutionIndexCommitted {
            let phase = self.phase;
            self.rollback();
            anyhow::bail!(
                "dynamic ability {:?} cannot sync runtime from phase {:?}",
                self.control_plane_key.ability(),
                phase
            );
        }
        match self
            .catalog
            .sync_runtime_ability(self.control_plane_key.ability())
        {
            Ok(()) => {
                self.phase = DynamicRegistrationPhase::RuntimeSynced;
                if let Some(control_plane_txn) = self.control_plane_txn.take() {
                    control_plane_txn.commit();
                }
                Ok(())
            }
            Err(error) => {
                self.rollback();
                Err(error)
            }
        }
    }

    fn rollback(&mut self) {
        if matches!(
            self.phase,
            DynamicRegistrationPhase::RuntimeSynced | DynamicRegistrationPhase::RolledBack
        ) {
            return;
        }
        if let Some(snapshot) = self.prior_dynamic.take() {
            let mut execution_index = self
                .catalog
                .execution_index
                .write()
                .expect("execution_index RwLock poisoned");
            execution_index.restore_dynamic(self.control_plane_key.key.clone(), snapshot);
        }
        if let Some(mut control_plane_txn) = self.control_plane_txn.take() {
            control_plane_txn
                .rollback()
                .expect("control-plane rollback snapshot must preserve table keys");
        }
        self.phase = DynamicRegistrationPhase::RolledBack;
    }
}

impl Drop for DynamicRegistrationTxn<'_> {
    fn drop(&mut self) {
        self.rollback();
    }
}

impl std::fmt::Debug for AxonAbilityCatalog {
    /// Manual impl because the handler types are `Arc<dyn Fn>`
    /// trait objects which do not implement `Debug`. Surfaces just
    /// the registered ability counts + names per shape — enough for
    /// `OnceLock::set`'s `.expect(..)` to print a useful message
    /// without leaking handler addresses.
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        let (static_counts, dynamic_count) = self
            .execution_index
            .read()
            .map(|index| {
                (
                    index.counts(ExecutionOrigin::Static),
                    index.counts(ExecutionOrigin::Dynamic).total(),
                )
            })
            .unwrap_or_default();
        let control_plane_count = self
            .control_plane
            .read()
            .map(|registry| registry.names().len())
            .unwrap_or(0);
        f.debug_struct("AxonAbilityCatalog")
            .field("rpc_count", &static_counts.rpc)
            .field("stream_count", &static_counts.stream)
            .field("bidi_count", &static_counts.bidi)
            .field("rpc_with_env_count", &static_counts.rpc_with_env)
            .field("stream_with_env_count", &static_counts.stream_with_env)
            .field("bidi_with_env_count", &static_counts.bidi_with_env)
            .field("control_plane_count", &control_plane_count)
            .field("dynamic_execution_count", &dynamic_count)
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
        Self::new_with_runtime_and_authority_context(
            runtime,
            AbilityAuthorityContext::from_local_environment(),
        )
    }

    pub fn new_with_runtime_and_authority_context(
        runtime: Arc<LocalRuntime>,
        authority_context: AbilityAuthorityContext,
    ) -> Self {
        Self {
            runtime: Some(runtime),
            authority_context,
            ..Self::default()
        }
    }

    /// Return the attached Axon runtime, if this registry was
    /// constructed for daemon boot.
    pub fn runtime(&self) -> Option<Arc<LocalRuntime>> {
        self.runtime.as_ref().map(Arc::clone)
    }

    fn register_control_plane(
        &self,
        request: ControlPlaneRegistrationRequest<'_>,
    ) -> anyhow::Result<AbilityControlPlaneRecord> {
        let owner_label = format!("{:?}", request.owner);
        let authority_scope = match request.owner.authority_scope(&self.authority_context) {
            Ok(scope) => scope,
            Err(error) => {
                let error_message = error.to_string();
                crate::op_event!(
                    component = ability_dispatch,
                    kind = control_plane_register_rejected,
                    ability = request.ability,
                    owner = owner_label,
                    error = error_message,
                    message = "ability control-plane record rejected before registry write",
                );
                return Err(anyhow::anyhow!(
                    "ability {:?} control-plane owner scope rejected: {error}",
                    request.ability
                ));
            }
        };
        self.write_control_plane_record(ResolvedControlPlaneRegistration {
            ability: request.ability,
            authority_scope,
            manifest: request.manifest,
            call_mode: request.call_mode,
            implementation: request.implementation,
            owner_label,
        })
    }

    fn write_control_plane_record(
        &self,
        request: ResolvedControlPlaneRegistration<'_>,
    ) -> anyhow::Result<AbilityControlPlaneRecord> {
        let mut registration = AbilityControlPlaneRegistration::new(
            request.ability.to_string(),
            request.call_mode,
            request.manifest,
            request.authority_scope,
            request.implementation.runtime_env,
            request.implementation.impl_source,
        );
        if let Some(impl_content_hash) = request.implementation.impl_content_hash {
            registration = registration.with_impl_content_hash(impl_content_hash);
        }
        let result = self
            .control_plane
            .write()
            .expect("control_plane RwLock poisoned")
            .register_registration(registration);
        match result {
            Ok(record) => {
                // SPEC §9.1.A Step 3: dual-write the manifest body into the
                // control-plane-keyed store, in lockstep with the record and
                // through this single choke point (both static and dynamic
                // commits flow here). Keyed by the record's own
                // `AbilityControlPlaneKey`, so the manifest is the record's
                // facet by key-identity, not a parallel String-keyed truth.
                // Absent manifest means "no declared schema" — recorded as a
                // key removal so a re-registration without a manifest cannot
                // leave a stale body behind.
                let key = AbilityControlPlaneKey::for_authority(record.authority());
                if let Some(manifest) = request.manifest {
                    self.control_plane_manifests
                        .write()
                        .expect("control_plane_manifests RwLock poisoned")
                        .insert(key, Arc::new(manifest.clone()));
                } else {
                    self.control_plane_manifests
                        .write()
                        .expect("control_plane_manifests RwLock poisoned")
                        .remove(&key);
                }
                Ok(record)
            }
            Err(error) => {
                let error_message = error.to_string();
                crate::op_event!(
                    component = ability_dispatch,
                    kind = control_plane_register_rejected,
                    ability = request.ability,
                    owner = request.owner_label,
                    error = error_message,
                    message = "ability control-plane record rejected by typed constructor",
                );
                Err(anyhow::anyhow!(
                    "ability {:?} control-plane typed constructor rejected: {error}",
                    request.ability
                ))
            }
        }
    }

    fn register_static(&mut self, registration: StaticRegistration) -> anyhow::Result<()> {
        registration.commit(self)
    }

    fn register_static_or_panic(&mut self, registration: StaticRegistration) {
        let ability = registration.ability.clone();
        self.register_static(registration)
            .unwrap_or_else(|error| panic!("static registration failed for {ability:?}: {error}"));
    }

    fn register_control_plane_with_scope_result(
        &self,
        ability: &str,
        authority_scope: AuthorityScope,
        manifest: Option<&crate::core::ability::spec::AbilityManifest>,
        call_mode: DescriptorCallMode,
        implementation: &ControlPlaneImplementation,
    ) -> anyhow::Result<ControlPlaneModeKey> {
        let owner_label = format!(
            "{}@{}",
            authority_scope.owner_projection(),
            authority_scope.authority_root()
        );
        let record = self.write_control_plane_record(ResolvedControlPlaneRegistration {
            ability,
            authority_scope,
            manifest,
            call_mode,
            implementation: implementation.clone(),
            owner_label,
        })?;
        Ok(ControlPlaneModeKey::from_record(&record))
    }

    fn register_dynamic_control_plane_with_scope_result(
        &self,
        ability: &str,
        authority_scope: AuthorityScope,
        manifest: Option<&crate::core::ability::spec::AbilityManifest>,
        call_mode: DescriptorCallMode,
        implementation: &ControlPlaneImplementation,
    ) -> anyhow::Result<ControlPlaneModeKey> {
        match self.register_control_plane_with_scope_result(
            ability,
            authority_scope,
            manifest,
            call_mode,
            implementation,
        ) {
            Ok(control_plane_key) => Ok(control_plane_key),
            Err(error) => {
                let error_message = error.to_string();
                crate::op_event!(
                    component = ability_dispatch,
                    kind = hot_register_control_plane_rejected,
                    ability = ability,
                    error = error_message.as_str(),
                    message = "dynamic ability rejected by control-plane validation",
                );
                Err(error)
            }
        }
    }

    /// Test-only diagnostic for mode-agnostic lookup semantics.
    ///
    /// Product code must use [`Self::control_plane_record_for_mode`] or
    /// [`Self::control_plane_record_for_authority_mode`]. An ability name is
    /// not a complete control-plane key once the same descriptor publishes
    /// multiple call modes or authority roots.
    #[cfg(test)]
    pub(crate) fn control_plane_record(
        &self,
        ability: &str,
    ) -> Result<Option<AbilityControlPlaneRecord>, AbilityControlPlaneLookupError> {
        self.control_plane
            .read()
            .expect("control_plane RwLock poisoned")
            .get(ability)
    }

    /// Return the split descriptor/authority/implementation record
    /// for the concrete execution mode. Same ability name + same
    /// descriptor version may legitimately publish multiple modes
    /// (`agent.chat` RPC and Stream), so protocol-facing callers
    /// should use this API whenever the requested mode is known.
    pub fn control_plane_record_for_mode(
        &self,
        ability: &str,
        call_mode: DescriptorCallMode,
    ) -> Result<Option<AbilityControlPlaneRecord>, AbilityControlPlaneLookupError> {
        self.control_plane
            .read()
            .expect("control_plane RwLock poisoned")
            .get_for_mode(ability, call_mode)
    }

    pub fn control_plane_record_for_version_mode(
        &self,
        ability: &str,
        descriptor_version: &str,
        call_mode: DescriptorCallMode,
    ) -> Result<Option<AbilityControlPlaneRecord>, AbilityControlPlaneLookupError> {
        self.control_plane
            .read()
            .expect("control_plane RwLock poisoned")
            .get_version_for_mode(ability, descriptor_version, call_mode)
    }

    /// Resolve the registered descriptor version for `ability` in
    /// `call_mode`, as the wire-facing dispatch paths must stamp it onto
    /// the descriptor-bound envelope and the receipt proof facts.
    ///
    /// Returns `None` when the ability has no control-plane record for the
    /// mode. There is deliberately no default-version fallback here: a
    /// dispatch that reaches the wire with no registered descriptor is a
    /// registration gap, and stamping a fabricated `1.0.0` would forge a
    /// proof fact. Callers decide how to surface the absence.
    pub fn registered_descriptor_version_for_mode(
        &self,
        ability: &str,
        call_mode: DescriptorCallMode,
    ) -> Result<Option<String>, AbilityControlPlaneLookupError> {
        Ok(self
            .control_plane_record_for_mode(ability, call_mode)?
            .map(|record| record.descriptor().version().as_str().to_string()))
    }

    pub fn control_plane_record_for_authority_mode(
        &self,
        authority_root: &str,
        ability: &str,
        call_mode: DescriptorCallMode,
    ) -> Result<Option<AbilityControlPlaneRecord>, AbilityControlPlaneAuthorityModeLookupError>
    {
        self.control_plane
            .read()
            .expect("control_plane RwLock poisoned")
            .get_for_authority_mode(authority_root, ability, call_mode)
    }

    pub fn begin_control_plane_authority_mode_transaction(
        &self,
        authority_root: &str,
        ability: &str,
        call_mode: DescriptorCallMode,
    ) -> ControlPlaneAuthorityModeTxn<'_> {
        ControlPlaneAuthorityModeTxn::begin(self, authority_root, ability, call_mode)
    }

    pub fn rebind_control_plane_record(
        &self,
        ability: &str,
        owner: &OwnerKind,
        manifest: Option<&crate::core::ability::spec::AbilityManifest>,
        call_mode: DescriptorCallMode,
        impl_source: AbilityImplSource,
        runtime_env: RuntimeEnv,
    ) -> anyhow::Result<()> {
        self.register_control_plane(ControlPlaneRegistrationRequest {
            ability,
            owner,
            manifest,
            call_mode,
            implementation: ControlPlaneImplementation::new(impl_source, runtime_env),
        })
        .map(|_| ())
    }

    pub fn rebind_control_plane_record_with_authority_scope(
        &self,
        request: ControlPlaneAuthorityRebind<'_>,
    ) -> anyhow::Result<AbilityControlPlaneRecord> {
        let owner_label = format!(
            "{}@{}",
            request.authority_scope.owner_projection(),
            request.authority_scope.authority_root()
        );
        self.write_control_plane_record(ResolvedControlPlaneRegistration {
            ability: request.ability,
            authority_scope: request.authority_scope,
            manifest: request.manifest,
            call_mode: request.call_mode,
            implementation: request.implementation,
            owner_label,
        })
    }

    pub fn remove_control_plane_record_for_authority_mode(
        &self,
        authority_root: &str,
        ability: &str,
        call_mode: DescriptorCallMode,
    ) -> bool {
        self.remove_control_plane_for_authority_mode(authority_root, ability, call_mode)
    }

    fn remove_control_plane_for_authority(&self, authority_root: &str, ability: &str) -> bool {
        let records_removed = self
            .control_plane
            .write()
            .expect("control_plane RwLock poisoned")
            .remove_for_authority(authority_root, ability);
        let manifests_removed =
            self.remove_control_plane_manifests_for_authority(authority_root, ability);
        records_removed || manifests_removed
    }

    fn remove_control_plane_for_authority_mode(
        &self,
        authority_root: &str,
        ability: &str,
        call_mode: DescriptorCallMode,
    ) -> bool {
        let records_removed = self
            .control_plane
            .write()
            .expect("control_plane RwLock poisoned")
            .remove_for_authority_mode(authority_root, ability, call_mode);
        let manifests_removed = self.remove_control_plane_manifests_for_authority_mode(
            authority_root,
            ability,
            call_mode,
        );
        records_removed || manifests_removed
    }

    fn remove_control_plane_manifests_for_authority(
        &self,
        authority_root: &str,
        ability: &str,
    ) -> bool {
        let mut manifests = self
            .control_plane_manifests
            .write()
            .expect("control_plane_manifests RwLock poisoned");
        let before = manifests.len();
        manifests
            .retain(|key, _| !(key.authority_root() == authority_root && key.ability() == ability));
        manifests.len() != before
    }

    fn remove_control_plane_manifests_for_authority_mode(
        &self,
        authority_root: &str,
        ability: &str,
        call_mode: DescriptorCallMode,
    ) -> bool {
        let mut manifests = self
            .control_plane_manifests
            .write()
            .expect("control_plane_manifests RwLock poisoned");
        let before = manifests.len();
        manifests.retain(|key, _| {
            !(key.authority_root() == authority_root
                && key.ability() == ability
                && key.call_mode() == call_mode)
        });
        manifests.len() != before
    }

    /// Return daemon-local runtime binding facts for this ability.
    ///
    /// This is deliberately not an Axon proof payload. Axon owns canonical
    /// receipt proof normalization; this projection only explains which
    /// daemon registration supplied the handler and authority metadata.
    pub fn runtime_binding_facts_for(
        &self,
        ability: &str,
    ) -> Result<Vec<RuntimeBindingFacts>, AbilityControlPlaneLookupError> {
        let mut facts = Vec::new();
        for call_mode in [
            DescriptorCallMode::Rpc,
            DescriptorCallMode::Stream,
            DescriptorCallMode::Bidi,
        ] {
            if let Some(mode_facts) = self.runtime_binding_facts_for_mode(ability, call_mode)? {
                facts.push(mode_facts);
            }
        }
        Ok(facts)
    }

    pub fn runtime_binding_facts_for_mode(
        &self,
        ability: &str,
        call_mode: DescriptorCallMode,
    ) -> Result<Option<RuntimeBindingFacts>, AbilityControlPlaneLookupError> {
        let Some(record) = self.control_plane_record_for_mode(ability, call_mode)? else {
            return Ok(None);
        };
        Ok(Some(self.runtime_binding_facts_from_record(record)))
    }

    fn runtime_binding_facts_from_record(
        &self,
        record: AbilityControlPlaneRecord,
    ) -> RuntimeBindingFacts {
        let descriptor = record.descriptor();
        let authority = record.authority();
        let implementation_record = record.implementation();
        let descriptor_version = descriptor.version().to_string();
        let runtime_env = implementation_record.runtime_env().label().to_string();
        let descriptor_hash = Some(descriptor.descriptor_hash().prefixed_hex());
        RuntimeBindingFacts {
            descriptor_version,
            call_mode: descriptor.call_mode(),
            schema_hash: descriptor.schema_hash().prefixed_hex(),
            descriptor_hash,
            implementation_source: implementation_record.source().as_str().to_string(),
            implementation_content_hash: implementation_record.content_hash().map(str::to_string),
            runtime_env,
            authority_owner_projection: authority.scope().owner_projection().to_string(),
            authority_root: authority.scope().authority_root().to_string(),
            governs_advertise: authority.predicate().governs_advertise(),
            governs_invoke: authority.predicate().governs_invoke(),
        }
    }

    fn resolve_authority_scope_for_owner(
        &self,
        ability: &str,
        owner: &OwnerKind,
    ) -> anyhow::Result<AuthorityScope> {
        owner
            .authority_scope(&self.authority_context)
            .map_err(|error| {
                anyhow::anyhow!("ability {ability:?} owner authority scope rejected: {error}")
            })
    }

    /// Resolve the unique authority root that owns `ability` from the
    /// canonical control-plane registry (SPEC §9.1.A — the legacy
    /// `authority_scope` side table is gone). Both static and dynamic
    /// registrations write the same `control_plane`, so a single query
    /// serves both; the static/dynamic key helpers differ only in which
    /// handler set they gate presence on.
    ///
    /// `registry.get()` is deliberately NOT used here: it errors when one
    /// ability name publishes multiple call modes, whereas the legacy table
    /// (and these key helpers) are call-mode-agnostic. `authority_roots_for_ability`
    /// collapses modes and dedups, so a single owner yields exactly one root.
    fn control_plane_authority_root(&self, ability: &str) -> anyhow::Result<Option<String>> {
        let roots = self
            .control_plane
            .read()
            .expect("control_plane RwLock poisoned")
            .authority_roots_for_ability(ability);
        match roots.as_slice() {
            [] => Ok(None),
            [root] => Ok(Some(root.clone())),
            _ => anyhow::bail!(
                "ability {ability:?} resolves to multiple authority roots {roots:?}; \
                 cannot derive a single authority-scoped control-plane key"
            ),
        }
    }

    fn static_control_plane_key(
        &self,
        ability: &str,
    ) -> anyhow::Result<Option<ControlPlaneAbilityKey>> {
        let execution_key = self
            .execution_index
            .read()
            .expect("execution_index RwLock poisoned")
            .origin_key_by_ability(ability, ExecutionOrigin::Static)?;
        if let Some(key) = execution_key {
            let Some(root) = self.control_plane_authority_root(ability)? else {
                anyhow::bail!(
                    "static ability {ability:?} has handlers but no control-plane authority record"
                );
            };
            if root != key.authority_root() {
                anyhow::bail!(
                    "static ability {ability:?} handler authority {:?} disagrees with control-plane authority {:?}",
                    key.authority_root(),
                    root
                );
            }
            return Ok(Some(key));
        }
        if self.has_static_handler(ability) {
            anyhow::bail!(
                "static ability {ability:?} has handlers but no control-plane authority record"
            );
        }
        Ok(None)
    }

    fn dynamic_control_plane_key(
        &self,
        ability: &str,
    ) -> anyhow::Result<Option<ControlPlaneAbilityKey>> {
        let execution_key = self
            .execution_index
            .read()
            .expect("execution_index RwLock poisoned")
            .origin_key_by_ability(ability, ExecutionOrigin::Dynamic)?;
        if let Some(key) = execution_key {
            let Some(root) = self.control_plane_authority_root(ability)? else {
                anyhow::bail!(
                    "dynamic ability {ability:?} has handlers but no control-plane authority record"
                );
            };
            if root != key.authority_root() {
                anyhow::bail!(
                    "dynamic ability {ability:?} handler authority {:?} disagrees with control-plane authority {:?}",
                    key.authority_root(),
                    root
                );
            }
            return Ok(Some(key));
        }
        if self.has_dynamic(ability) {
            anyhow::bail!(
                "dynamic ability {ability:?} has handlers but no control-plane authority record"
            );
        }
        Ok(None)
    }

    fn handler_control_plane_key(&self, ability: &str) -> anyhow::Result<ControlPlaneAbilityKey> {
        if let Some(key) = self.static_control_plane_key(ability)? {
            return Ok(key);
        }
        if let Some(key) = self.dynamic_control_plane_key(ability)? {
            return Ok(key);
        }
        anyhow::bail!(
            "ability {ability:?} has handlers but no authority table entry; cannot derive authority-scoped runtime key"
        )
    }

    fn runtime_ability_key_for_mode(
        &self,
        ability: &str,
        call_mode: DescriptorCallMode,
    ) -> anyhow::Result<Option<String>> {
        let Some(record) = self.control_plane_record_for_mode(ability, call_mode)? else {
            return Ok(None);
        };
        Ok(Some(local_runtime_ability_key_for_authority(
            record.authority().scope().authority_root(),
            ability,
        )?))
    }

    fn bind_invocation_target_to_control_plane(
        &self,
        mut target: InvocationTarget,
        call_mode: DescriptorCallMode,
    ) -> anyhow::Result<InvocationTarget> {
        if crate::core::ura::AbilitySelector::parse(&target.ability).is_ok() {
            return Ok(target);
        }
        let Some(record) = self.control_plane_record_for_mode(&target.ability, call_mode)? else {
            // Carry the canonical `unknown_ability:<name>` token so a
            // control-plane miss classifies as NOT_FOUND and reads as the
            // same "unknown ability" condition everywhere, rather than a
            // bespoke phrase.
            anyhow::bail!(
                "unknown_ability:{} is not registered in the control plane for {:?}",
                target.ability,
                call_mode
            );
        };
        target.ability = local_runtime_ability_key_for_authority(
            record.authority().scope().authority_root(),
            &target.ability,
        )?;
        Ok(target)
    }

    /// Invoke an RPC ability through Axon's `LocalRuntime` as a local-system
    /// call. Use `invoke_rpc_target_json` when the caller has an envelope
    /// subject or causal context; this shorthand is only for daemon-internal
    /// calls that genuinely have no upstream invocation envelope.
    pub fn invoke_rpc_json(&self, ability: &str, args: Value) -> anyhow::Result<Value> {
        let target = InvocationTarget {
            scope: TargetScope::Local,
            ability: ability.to_string(),
            normalized_args: args,
            call_mode: CallMode::Rpc,
            subject: None,
            causal_context: None,
        };
        self.invoke_rpc_target_json(target)
    }

    /// Invoke an RPC ability with an explicit resolved target. This is the
    /// required path for in-process forwarders that know the AXIOM subject or
    /// causal context.
    pub fn invoke_rpc_target_json(&self, target: InvocationTarget) -> anyhow::Result<Value> {
        if target.call_mode != CallMode::Rpc {
            anyhow::bail!(
                "invoke_rpc_target_json requires RPC call mode, got {:?}",
                target.call_mode
            );
        }
        let runtime = self.runtime().ok_or_else(|| {
            anyhow::anyhow!(
                "AxonAbilityCatalog has no LocalRuntime attached; use new() or new_with_runtime()"
            )
        })?;
        let target = self
            .bind_invocation_target_to_control_plane(target, DescriptorCallMode::Rpc)
            .map_err(|err| anyhow::anyhow!("{err}; local Axon runtime loopback path"))?;
        crate::daemon::invocation::dispatch::local_runtime_invoker::invoke_local_rpc_sync(
            runtime, target,
        )
        .map_err(|err| anyhow::anyhow!("{err}"))
    }

    fn replace_runtime_ability(
        &self,
        control_plane_key: &ControlPlaneAbilityKey,
        ability_fn: AbilityFn,
        options: AbilityOptions,
    ) -> anyhow::Result<()> {
        let Some(runtime) = self.runtime.as_ref() else {
            return Ok(());
        };
        let name = control_plane_key.ability();
        let runtime_key = control_plane_key.runtime_key()?;
        let result = block_on_runtime_sync(runtime.replace_ability(
            runtime_key.clone(),
            ability_fn,
            options,
        ));
        if let Err(err) = result {
            let err_msg = format!("{err}");
            crate::op_event!(
                component = axon_bridge,
                kind = local_runtime_register_failed,
                ability = name,
                runtime_key = runtime_key.as_str(),
                error = err_msg.as_str(),
            );
            return Err(anyhow::anyhow!(
                "LocalRuntime rejected ability {name:?} as {runtime_key:?}: {err_msg}"
            ));
        }
        Ok(())
    }

    fn unregister_runtime_ability(&self, name: &str) -> anyhow::Result<()> {
        let Some(runtime) = self.runtime.as_ref() else {
            return Ok(());
        };
        let runtime_key = self.handler_control_plane_key(name)?.runtime_key()?;
        self.unregister_runtime_ability_by_key(runtime, name, &runtime_key)
    }

    fn unregister_runtime_ability_by_key(
        &self,
        runtime: &Arc<LocalRuntime>,
        name: &str,
        runtime_key: &str,
    ) -> anyhow::Result<()> {
        let _removed = block_on_runtime_sync(runtime.unregister_ability(runtime_key));
        crate::op_event!(
            component = axon_bridge,
            kind = local_runtime_unregister,
            ability = name,
            runtime_key = runtime_key,
        );
        Ok(())
    }

    fn runtime_handlers_for(&self, name: &str) -> RuntimeHandlerSet {
        self.execution_index
            .read()
            .expect("execution_index RwLock poisoned")
            .handlers_for_ability(name)
    }

    fn sync_runtime_ability(&self, name: &str) -> anyhow::Result<()> {
        let handlers = self.runtime_handlers_for(name);
        self.sync_runtime_ability_from_handlers(name, handlers)
    }

    fn sync_static_runtime_ability_or_panic(&self, name: &str) {
        self.sync_runtime_ability(name).unwrap_or_else(|error| {
            panic!("static ability {name:?} failed to sync into LocalRuntime: {error}")
        });
    }

    /// True when `ability` is present in the boot-time static catalogue.
    ///
    /// What this is NOT: a general discovery helper. Dynamic plugin/MCP
    /// entries are intentionally ignored so plugin-host reload can enforce
    /// the invariant that post-boot extensions never shadow daemon/system
    /// abilities.
    pub fn has_static_ability(&self, ability: &str) -> bool {
        self.has_static_handler(ability)
    }

    /// Snapshot the boot-time static ability names once for bounded
    /// catalogue validation.
    ///
    /// Plugin reload and other fan-in validators should consume this read
    /// model instead of calling [`Self::has_static_ability`] per candidate:
    /// the execution index is authority-keyed, so per-name probes would
    /// otherwise rescan that index repeatedly.
    pub fn static_ability_names(&self) -> Vec<String> {
        self.execution_index
            .read()
            .expect("execution_index RwLock poisoned")
            .names(ExecutionOrigin::Static)
    }

    fn has_static_handler(&self, ability: &str) -> bool {
        self.execution_index
            .read()
            .expect("execution_index RwLock poisoned")
            .contains_origin_handler_by_name(ability, ExecutionOrigin::Static)
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
        if let Err(error) = self.sync_runtime_ability(ability) {
            let error_message = error.to_string();
            crate::op_event!(
                component = ability_dispatch,
                kind = static_collision_resync_failed,
                ability = ability,
                error = error_message.as_str(),
                message = "static ability collision was rejected, but restoring the static runtime binding failed",
            );
        }
        true
    }

    fn sync_runtime_ability_from_handlers(
        &self,
        name: &str,
        handlers: RuntimeHandlerSet,
    ) -> anyhow::Result<()> {
        let modes = handlers.modes();
        if modes.is_empty() {
            return self.unregister_runtime_ability(name);
        }
        let control_plane_key = self.handler_control_plane_key(name)?;
        let options = self.runtime_options_for(&control_plane_key, modes)?;
        self.replace_runtime_ability(
            &control_plane_key,
            runtime_handler_set_to_ability_fn(name.to_string(), handlers),
            options,
        )
    }

    fn runtime_options_for(
        &self,
        control_plane_key: &ControlPlaneAbilityKey,
        modes: AbilityCallModes,
    ) -> anyhow::Result<AbilityOptions> {
        let mut options = AbilityOptions::default().with_modes(modes);
        if modes.rpc {
            options = self.bind_runtime_proof_for_mode(
                control_plane_key,
                options,
                DescriptorCallMode::Rpc,
                AxonCallMode::Rpc,
            )?;
        }
        if modes.stream {
            options = self.bind_runtime_proof_for_mode(
                control_plane_key,
                options,
                DescriptorCallMode::Stream,
                AxonCallMode::Stream,
            )?;
        }
        if modes.bidi {
            options = self.bind_runtime_proof_for_mode(
                control_plane_key,
                options,
                DescriptorCallMode::Bidi,
                AxonCallMode::Bidi,
            )?;
        }
        Ok(options)
    }

    fn bind_runtime_proof_for_mode(
        &self,
        control_plane_key: &ControlPlaneAbilityKey,
        options: AbilityOptions,
        descriptor_mode: DescriptorCallMode,
        axon_mode: AxonCallMode,
    ) -> anyhow::Result<AbilityOptions> {
        let record = self
            .control_plane_record_for_authority_mode(
                control_plane_key.authority_root(),
                control_plane_key.ability(),
                descriptor_mode,
            )?
            .ok_or_else(|| {
                anyhow::anyhow!(
                    "ability {:?} under authority {:?} has a {:?} runtime handler without a unique control-plane record",
                    control_plane_key.ability(),
                    control_plane_key.authority_root(),
                    descriptor_mode
                )
            })?;
        Ok(options.with_mode_descriptor_proof(
            axon_mode,
            record.descriptor().version().as_str(),
            record.descriptor().schema_hash().0,
            record.implementation().impl_hash(),
        ))
    }

    fn assert_static_handler_slot_available(
        &self,
        key: &ControlPlaneAbilityKey,
        target_slot: HandlerSlotKind,
    ) {
        let conflicts = self
            .handler_slots(key)
            .into_iter()
            .filter(|existing| target_slot.conflicts_with(*existing))
            .map(HandlerSlotKind::label)
            .collect::<Vec<_>>();
        assert!(
            conflicts.is_empty(),
            "ability {:?} under authority {:?} is already registered in static handler slot(s) {}; \
             one call mode must have exactly one handler family",
            key.ability(),
            key.authority_root(),
            conflicts.join(", ")
        );
    }

    fn handler_slots(&self, key: &ControlPlaneAbilityKey) -> Vec<HandlerSlotKind> {
        self.execution_index
            .read()
            .expect("execution_index RwLock poisoned")
            .slots(key)
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
        self.register_static_or_panic(StaticRegistration::new(
            ability,
            owner,
            StaticRegistrationHandler::Rpc(handler),
        ));
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
        manifest: crate::core::ability::spec::AbilityManifest,
        handler: LocalRpcHandler,
    ) {
        self.register_static_or_panic(
            StaticRegistration::new(ability, owner, StaticRegistrationHandler::Rpc(handler))
                .with_manifest(manifest),
        );
    }

    pub fn register_rpc_with_spec_impl_and_authority_scope(
        &mut self,
        ability: impl Into<String>,
        owner: OwnerKind,
        authority_scope: AuthorityScope,
        manifest: crate::core::ability::spec::AbilityManifest,
        handler: LocalRpcHandler,
        implementation: ControlPlaneImplementation,
    ) -> anyhow::Result<()> {
        self.register_static(
            StaticRegistration::new(ability, owner, StaticRegistrationHandler::Rpc(handler))
                .with_authority_scope(authority_scope)
                .with_manifest(manifest)
                .with_implementation(implementation),
        )
    }

    /// Companion to [`register_rpc_with_spec`] for stream
    /// handlers. The manifest is shared between RPC + Stream
    /// surfaces of the same ability. The control-plane record is
    /// keyed by `(ability, descriptor_version, call_mode)`, so both
    /// modes remain visible instead of overwriting one another.
    pub fn register_stream_with_spec(
        &mut self,
        ability: impl Into<String>,
        owner: OwnerKind,
        manifest: crate::core::ability::spec::AbilityManifest,
        handler: LocalStreamHandler,
    ) {
        self.register_static_or_panic(
            StaticRegistration::new(ability, owner, StaticRegistrationHandler::Stream(handler))
                .with_manifest(manifest),
        );
    }

    pub fn register_stream_with_spec_and_impl(
        &mut self,
        ability: impl Into<String>,
        owner: OwnerKind,
        manifest: crate::core::ability::spec::AbilityManifest,
        handler: LocalStreamHandler,
        implementation: ControlPlaneImplementation,
    ) -> anyhow::Result<()> {
        self.register_static(
            StaticRegistration::new(ability, owner, StaticRegistrationHandler::Stream(handler))
                .with_manifest(manifest)
                .with_implementation(implementation),
        )
    }

    pub fn register_stream_with_spec_impl_and_authority_scope(
        &mut self,
        ability: impl Into<String>,
        owner: OwnerKind,
        authority_scope: AuthorityScope,
        manifest: crate::core::ability::spec::AbilityManifest,
        handler: LocalStreamHandler,
        implementation: ControlPlaneImplementation,
    ) -> anyhow::Result<()> {
        self.register_static(
            StaticRegistration::new(ability, owner, StaticRegistrationHandler::Stream(handler))
                .with_authority_scope(authority_scope)
                .with_manifest(manifest)
                .with_implementation(implementation),
        )
    }

    /// Control-plane read-through for an ability's manifest body, derived
    /// from the control-plane-keyed manifest facet (SPEC §9.1.A Step 3).
    ///
    /// The ability is resolved to its canonical control-plane record first
    /// (ambiguous multi-mode names yield `None`, never an arbitrary pick),
    /// then the manifest facet is looked up by that record's key. A record
    /// with no manifest yields `None` — there is no fall-through to the
    /// former String-keyed manifest table, so a missing facet stays missing
    /// (acceptance test 5).
    pub fn control_plane_manifest(
        &self,
        ability: &str,
    ) -> Option<Arc<crate::core::ability::spec::AbilityManifest>> {
        let record = self
            .control_plane
            .read()
            .expect("control_plane RwLock poisoned")
            .get(ability)
            .ok()
            .flatten()?;
        let key = AbilityControlPlaneKey::for_authority(record.authority());
        self.control_plane_manifests
            .read()
            .expect("control_plane_manifests RwLock poisoned")
            .get(&key)
            .map(Arc::clone)
    }

    /// Register an RPC handler under `ability`. Owner defaults to
    /// [`OwnerKind::Device`] — the safe choice for daemon-local host
    /// abilities advertised by the device-profile Agent under device
    /// authority. New call sites should use [`register_rpc_with_owner`]
    /// to declare the actual authority projection explicitly.
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
        self.register_static_or_panic(StaticRegistration::new(
            ability,
            owner,
            StaticRegistrationHandler::Stream(handler),
        ));
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
        self.register_static_or_panic(StaticRegistration::new(
            ability,
            owner,
            StaticRegistrationHandler::Bidi(handler),
        ));
    }

    /// Register a bidi handler with explicit owner and manifest.
    ///
    /// Mirrors [`Self::register_rpc_with_spec`] and
    /// [`Self::register_stream_with_spec`] for abilities whose data plane is
    /// bidirectional but whose discovery contract is still a normal
    /// `AbilityManifest`.
    pub fn register_bidi_with_spec(
        &mut self,
        ability: impl Into<String>,
        owner: OwnerKind,
        manifest: crate::core::ability::spec::AbilityManifest,
        handler: LocalBidiHandler,
    ) {
        self.register_static_or_panic(
            StaticRegistration::new(ability, owner, StaticRegistrationHandler::Bidi(handler))
                .with_manifest(manifest),
        );
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
        self.register_static_or_panic(StaticRegistration::new(
            ability,
            owner,
            StaticRegistrationHandler::RpcWithEnvelope(handler),
        ));
    }

    /// Register an envelope-aware RPC handler with explicit owner and manifest.
    ///
    /// Plugin handlers need the AXIOM envelope (`subject`, caller context, and
    /// causal metadata) and also need to publish the package descriptor schema
    /// through `meta.list_abilities`. This method keeps those two contracts in
    /// one registration path instead of forcing plugin host code to write the
    /// handler index and manifest facet separately.
    pub fn register_rpc_with_envelope_and_spec(
        &mut self,
        ability: impl Into<String>,
        owner: OwnerKind,
        manifest: crate::core::ability::spec::AbilityManifest,
        handler: LocalRpcHandlerWithEnvelope,
    ) {
        self.register_static_or_panic(
            StaticRegistration::new(
                ability,
                owner,
                StaticRegistrationHandler::RpcWithEnvelope(handler),
            )
            .with_manifest(manifest),
        );
    }

    pub fn register_rpc_with_envelope_and_spec_and_impl(
        &mut self,
        ability: impl Into<String>,
        owner: OwnerKind,
        manifest: crate::core::ability::spec::AbilityManifest,
        handler: LocalRpcHandlerWithEnvelope,
        implementation: ControlPlaneImplementation,
    ) -> anyhow::Result<()> {
        StaticRegistration::new(
            ability,
            owner,
            StaticRegistrationHandler::RpcWithEnvelope(handler),
        )
        .with_manifest(manifest)
        .with_implementation(implementation)
        .commit(self)
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
        self.register_static_or_panic(StaticRegistration::new(
            ability,
            owner,
            StaticRegistrationHandler::StreamWithEnvelope(handler),
        ));
    }

    /// Register an envelope-aware stream handler with explicit owner and
    /// registry manifest.
    pub fn register_stream_with_envelope_and_spec(
        &mut self,
        ability: impl Into<String>,
        owner: OwnerKind,
        manifest: crate::core::ability::spec::AbilityManifest,
        handler: LocalStreamHandlerWithEnvelope,
    ) {
        self.register_static_or_panic(
            StaticRegistration::new(
                ability,
                owner,
                StaticRegistrationHandler::StreamWithEnvelope(handler),
            )
            .with_manifest(manifest),
        );
    }

    pub fn register_stream_with_envelope_and_spec_and_impl(
        &mut self,
        ability: impl Into<String>,
        owner: OwnerKind,
        manifest: crate::core::ability::spec::AbilityManifest,
        handler: LocalStreamHandlerWithEnvelope,
        implementation: ControlPlaneImplementation,
    ) -> anyhow::Result<()> {
        StaticRegistration::new(
            ability,
            owner,
            StaticRegistrationHandler::StreamWithEnvelope(handler),
        )
        .with_manifest(manifest)
        .with_implementation(implementation)
        .commit(self)
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
        self.register_static_or_panic(StaticRegistration::new(
            ability,
            owner,
            StaticRegistrationHandler::BidiWithEnvelope(handler),
        ));
    }

    /// Register an envelope-aware bidi handler with explicit owner and
    /// registry manifest.
    pub fn register_bidi_with_envelope_and_spec(
        &mut self,
        ability: impl Into<String>,
        owner: OwnerKind,
        manifest: crate::core::ability::spec::AbilityManifest,
        handler: LocalBidiHandlerWithEnvelope,
    ) {
        self.register_static_or_panic(
            StaticRegistration::new(
                ability,
                owner,
                StaticRegistrationHandler::BidiWithEnvelope(handler),
            )
            .with_manifest(manifest),
        );
    }

    pub fn register_bidi_with_envelope_and_spec_and_impl(
        &mut self,
        ability: impl Into<String>,
        owner: OwnerKind,
        manifest: crate::core::ability::spec::AbilityManifest,
        handler: LocalBidiHandlerWithEnvelope,
        implementation: ControlPlaneImplementation,
    ) -> anyhow::Result<()> {
        StaticRegistration::new(
            ability,
            owner,
            StaticRegistrationHandler::BidiWithEnvelope(handler),
        )
        .with_manifest(manifest)
        .with_implementation(implementation)
        .commit(self)
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

    /// Control-plane read-through for an ability's `OwnerKind`, derived from
    /// the canonical control-plane record rather than the legacy `owner`
    /// side table (SPEC §9.1.A: handler maps and side tables become an
    /// execution index; owner/authority/manifest truth lives in the
    /// control-plane registry).
    ///
    /// The `OwnerKind` is reconstructed from the record's authority
    /// `owner_projection` — the exact string `OwnerKind::authority_scope`
    /// wrote at registration (`device` / `hub` / `agent:<id>` /
    /// `user:<id>`). This is the precise inverse of the registration
    /// mapping: no Ability-URA round-trip and no owner-class policy (the
    /// MCP reflective path's System-Agent rejection does not apply to
    /// general ownership). A missing record yields `None` rather than
    /// falling back to a legacy owner table.
    pub fn control_plane_owner(&self, ability: &str) -> Option<OwnerKind> {
        let record = self
            .control_plane
            .read()
            .expect("control_plane RwLock poisoned")
            .get(ability)
            .ok()
            .flatten()?;
        owner_kind_from_projection(record.authority().scope().owner_projection())
    }

    /// Test fixture: make `ability` resolve to no owner by removing its
    /// control-plane record, so `control_plane_owner` returns `None` (the
    /// catalog simulates an ability whose ownership can no longer be
    /// stamped). Clears the control-plane authority truth, not a legacy
    /// side table.
    #[cfg(test)]
    pub(crate) fn clear_owner_for_test(&mut self, ability: &str) {
        self.control_plane
            .write()
            .expect("control_plane RwLock poisoned")
            .remove_for_ability(ability);
    }

    /// Remove every static trace of `ability` from the registry: its
    /// authority-keyed execution row, control-plane records, and control-plane
    /// manifest facets. Returns `true` if the ability was present, `false` if
    /// it was already absent.
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
    pub fn unregister(&mut self, ability: &str) -> anyhow::Result<bool> {
        if !self.has_static_ability(ability) {
            return Ok(false);
        }
        let control_plane_key = self.static_control_plane_key(ability)?.ok_or_else(|| {
            anyhow::anyhow!(
                "static ability {ability:?} has handler state but no control-plane authority record; cannot unregister authority-scoped control-plane facts"
            )
        })?;
        let runtime_key = control_plane_key.runtime_key()?;
        let present = self.drain_static(&control_plane_key);
        if present {
            self.remove_control_plane_for_authority(
                control_plane_key.authority_root(),
                control_plane_key.ability(),
            );
            if let Some(runtime) = self.runtime.as_ref() {
                self.unregister_runtime_ability_by_key(runtime, ability, &runtime_key)
                    .with_context(|| {
                        format!("static ability {ability:?} failed to unregister from LocalRuntime")
                    })?;
            }
        }
        Ok(present)
    }

    /// Drop the static execution row for `key` and return whether it carried
    /// handlers. Control-plane and runtime cleanup remain the caller's
    /// transaction responsibility.
    fn drain_static(&mut self, key: &ControlPlaneAbilityKey) -> bool {
        self.execution_index
            .write()
            .expect("execution_index RwLock poisoned")
            .drain_static(key)
    }

    // ── Hot-reload dynamic execution rows ────────────────────────────
    //
    // The methods below are the `&self` mutation surface used by
    // `RegistryRefreshSink` after boot. They may only write rows tagged
    // `ExecutionOrigin::Dynamic` in the single execution index. Metadata
    // reads come from the control-plane registry and its keyed manifest facet,
    // not from dynamic metadata snapshots.

    /// Hot-register an RPC handler with explicit owner + manifest as a dynamic
    /// execution row. Used by `RegistryRefreshSink` when a
    /// freshly-listed upstream MCP tool needs to become invokable
    /// without a daemon restart. Replaces any prior dynamic entry at
    /// the same key (same write-replaces-write semantics as the
    /// static `register_rpc_with_spec`).
    pub fn hot_register_rpc_with_spec(
        &self,
        ability: impl Into<String>,
        owner: OwnerKind,
        manifest: crate::core::ability::spec::AbilityManifest,
        handler: LocalRpcHandler,
    ) -> anyhow::Result<()> {
        DynamicRegistration::rpc_with_spec(ability, owner, manifest, handler).commit(self)
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
    ) -> anyhow::Result<()> {
        DynamicRegistration::rpc(ability, owner, handler).commit(self)
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
        manifest: crate::core::ability::spec::AbilityManifest,
        handler: LocalStreamHandler,
    ) -> anyhow::Result<()> {
        self.hot_register_stream_with_spec_and_impl(
            ability,
            owner,
            manifest,
            handler,
            ControlPlaneImplementation::native_daemon(),
        )
    }

    pub fn hot_register_stream_with_spec_and_impl(
        &self,
        ability: impl Into<String>,
        owner: OwnerKind,
        manifest: crate::core::ability::spec::AbilityManifest,
        handler: LocalStreamHandler,
        implementation: ControlPlaneImplementation,
    ) -> anyhow::Result<()> {
        DynamicRegistration::stream_with_spec_and_impl(
            ability,
            owner,
            manifest,
            handler,
            implementation,
        )
        .commit(self)
    }

    pub fn hot_register_stream_with_spec_impl_and_authority_scope(
        &self,
        ability: impl Into<String>,
        owner: OwnerKind,
        authority_scope: AuthorityScope,
        manifest: crate::core::ability::spec::AbilityManifest,
        handler: LocalStreamHandler,
        implementation: ControlPlaneImplementation,
    ) -> anyhow::Result<()> {
        DynamicRegistration::stream_with_spec_and_impl(
            ability,
            owner,
            manifest,
            handler,
            implementation,
        )
        .with_authority_scope(authority_scope)
        .commit(self)
    }

    /// Hot-register an envelope-aware RPC handler in the dynamic execution
    /// index. Plugin hot-load uses this path so sidecar/declarative handlers
    /// receive the same AXIOM envelope context as boot-registered handlers.
    pub fn hot_register_rpc_with_envelope(
        &self,
        ability: impl Into<String>,
        owner: OwnerKind,
        handler: LocalRpcHandlerWithEnvelope,
    ) -> anyhow::Result<()> {
        DynamicRegistration::rpc_with_envelope(ability, owner, handler).commit(self)
    }

    /// Hot-register an envelope-aware RPC handler with explicit owner and
    /// registry manifest in the dynamic execution row.
    pub fn hot_register_rpc_with_envelope_and_spec(
        &self,
        ability: impl Into<String>,
        owner: OwnerKind,
        manifest: crate::core::ability::spec::AbilityManifest,
        handler: LocalRpcHandlerWithEnvelope,
    ) -> anyhow::Result<()> {
        self.hot_register_rpc_with_envelope_and_spec_and_impl(
            ability,
            owner,
            manifest,
            handler,
            ControlPlaneImplementation::native_daemon(),
        )
    }

    pub fn hot_register_rpc_with_envelope_and_spec_and_impl(
        &self,
        ability: impl Into<String>,
        owner: OwnerKind,
        manifest: crate::core::ability::spec::AbilityManifest,
        handler: LocalRpcHandlerWithEnvelope,
        implementation: ControlPlaneImplementation,
    ) -> anyhow::Result<()> {
        DynamicRegistration::rpc_with_envelope_and_spec_and_impl(
            ability,
            owner,
            manifest,
            handler,
            implementation,
        )
        .commit(self)
    }

    /// Hot-register an envelope-aware stream handler as a dynamic execution row.
    pub fn hot_register_stream_with_envelope(
        &self,
        ability: impl Into<String>,
        owner: OwnerKind,
        handler: LocalStreamHandlerWithEnvelope,
    ) -> anyhow::Result<()> {
        DynamicRegistration::stream_with_envelope(ability, owner, handler).commit(self)
    }

    /// Hot-register an envelope-aware stream handler with explicit owner and
    /// registry manifest in the dynamic execution row.
    pub fn hot_register_stream_with_envelope_and_spec(
        &self,
        ability: impl Into<String>,
        owner: OwnerKind,
        manifest: crate::core::ability::spec::AbilityManifest,
        handler: LocalStreamHandlerWithEnvelope,
    ) -> anyhow::Result<()> {
        self.hot_register_stream_with_envelope_and_spec_and_impl(
            ability,
            owner,
            manifest,
            handler,
            ControlPlaneImplementation::native_daemon(),
        )
    }

    pub fn hot_register_stream_with_envelope_and_spec_and_impl(
        &self,
        ability: impl Into<String>,
        owner: OwnerKind,
        manifest: crate::core::ability::spec::AbilityManifest,
        handler: LocalStreamHandlerWithEnvelope,
        implementation: ControlPlaneImplementation,
    ) -> anyhow::Result<()> {
        DynamicRegistration::stream_with_envelope_and_spec_and_impl(
            ability,
            owner,
            manifest,
            handler,
            implementation,
        )
        .commit(self)
    }

    /// Hot-register an envelope-aware bidi handler as a dynamic execution row.
    pub fn hot_register_bidi_with_envelope(
        &self,
        ability: impl Into<String>,
        owner: OwnerKind,
        handler: LocalBidiHandlerWithEnvelope,
    ) -> anyhow::Result<()> {
        DynamicRegistration::bidi_with_envelope(ability, owner, handler).commit(self)
    }

    /// Hot-register an envelope-aware bidi handler with explicit owner and
    /// registry manifest in the dynamic execution row.
    pub fn hot_register_bidi_with_envelope_and_spec(
        &self,
        ability: impl Into<String>,
        owner: OwnerKind,
        manifest: crate::core::ability::spec::AbilityManifest,
        handler: LocalBidiHandlerWithEnvelope,
    ) -> anyhow::Result<()> {
        self.hot_register_bidi_with_envelope_and_spec_and_impl(
            ability,
            owner,
            manifest,
            handler,
            ControlPlaneImplementation::native_daemon(),
        )
    }

    pub fn hot_register_bidi_with_envelope_and_spec_and_impl(
        &self,
        ability: impl Into<String>,
        owner: OwnerKind,
        manifest: crate::core::ability::spec::AbilityManifest,
        handler: LocalBidiHandlerWithEnvelope,
        implementation: ControlPlaneImplementation,
    ) -> anyhow::Result<()> {
        DynamicRegistration::bidi_with_envelope_and_spec_and_impl(
            ability,
            owner,
            manifest,
            handler,
            implementation,
        )
        .commit(self)
    }

    /// Remove the dynamic execution row for `ability` under its recorded
    /// authority root. Static entries are not touched.
    pub fn hot_unregister(&self, ability: &str) -> anyhow::Result<bool> {
        let _dynamic_txn_guard = self.dynamic_txn.lock().expect("dynamic_txn mutex poisoned");
        let Some(control_plane_key) = self.dynamic_control_plane_key(ability)? else {
            return Ok(false);
        };
        let runtime_key = control_plane_key.runtime_key()?;
        {
            let mut execution_index = self
                .execution_index
                .write()
                .expect("execution_index RwLock poisoned");
            if !execution_index.drain_dynamic(&control_plane_key) {
                return Ok(false);
            }
        }
        self.remove_control_plane_for_authority(
            control_plane_key.authority_root(),
            control_plane_key.ability(),
        );
        if let Some(runtime) = self.runtime.as_ref() {
            self.unregister_runtime_ability_by_key(runtime, ability, &runtime_key)?;
        }
        Ok(true)
    }

    /// Remove a post-boot ability from the dynamic side and from
    /// LocalRuntime.
    ///
    /// Static boot abilities are immutable catalogue facts. This API is the
    /// dynamic package/MCP removal path; if a static name reaches it, the call
    /// is rejected as a no-op so boot metadata, control-plane records, and
    /// LocalRuntime cannot be desynchronised.
    pub fn hot_remove_runtime_ability(&self, ability: &str) -> anyhow::Result<bool> {
        if self.has_static_ability(ability) {
            return Ok(false);
        }
        self.hot_unregister(ability)
    }

    /// True iff the dynamic side currently holds an entry for
    /// `ability` in any of its handler maps. Companion check for
    /// hot-reload diagnostics; the boot-time `has_rpc`/`has_stream`/
    /// `has_bidi` lookups already consult this internally via the
    /// fall-through paths.
    pub fn has_dynamic(&self, ability: &str) -> bool {
        self.execution_index
            .read()
            .expect("execution_index RwLock poisoned")
            .contains_origin_handler_by_name(ability, ExecutionOrigin::Dynamic)
    }

    /// List the names currently held as dynamic execution rows.
    /// Used by `list_abilities` to union dynamic with static
    /// without exposing the lock guard; useful on its own for
    /// hot-reload diagnostics.
    pub fn list_dynamic_abilities(&self) -> Vec<String> {
        self.execution_index
            .read()
            .expect("execution_index RwLock poisoned")
            .names(ExecutionOrigin::Dynamic)
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
        // SPEC §9.1.A acceptance test 4 + item 7: the catalogue read surface
        // comes from the canonical control-plane registry, NOT a union of the
        // six String-keyed handler maps. Every static and dynamic
        // registration writes its control-plane record through the single
        // `write_control_plane_record` choke point, so `control_plane.names()`
        // is the authoritative, de-duplicated set of ability names across all
        // call modes and authority roots — the handler maps are a pure
        // execution index and no longer the discovery source of truth.
        self.control_plane
            .read()
            .expect("control_plane RwLock poisoned")
            .names()
    }

    /// Deterministic catalogue read-model: public ability name -> registered
    /// descriptor call modes.
    ///
    /// This is deliberately a control-plane snapshot, not a runtime probe.
    /// Catalogue renderers (`meta.list_abilities`, MCP/A2A projections, CLI
    /// list views) need to know whether an ability is unary, stream, or bidi,
    /// but asking `LocalRuntime::ability_options` once per ability/mode turns
    /// a pure list operation into an async fan-out. The control-plane registry
    /// already owns the validated descriptor rows; read those rows once and
    /// let callers derive advisory hints from the snapshot.
    pub fn call_modes_by_ability(&self) -> BTreeMap<String, BTreeSet<DescriptorCallMode>> {
        let mut modes = BTreeMap::<String, BTreeSet<DescriptorCallMode>>::new();
        for (key, _record) in self
            .control_plane
            .read()
            .expect("control_plane RwLock poisoned")
            .records()
        {
            modes
                .entry(key.ability().to_string())
                .or_default()
                .insert(key.call_mode());
        }
        modes
    }

    /// Complete catalogue read model keyed by public ability name.
    ///
    /// This is the owner/manifest companion to [`Self::call_modes_by_ability`].
    /// Read paths that render descriptors should consume this snapshot instead
    /// of calling `control_plane_owner` and `control_plane_manifest` per name:
    /// those APIs preserve precise single-record lookup semantics for dispatch
    /// and tests, while catalogue projection needs one bounded scan.
    pub fn ability_catalog_snapshot(&self) -> Vec<AbilityCatalogSnapshotRow> {
        let control_plane = self
            .control_plane
            .read()
            .expect("control_plane RwLock poisoned");
        let manifests = self
            .control_plane_manifests
            .read()
            .expect("control_plane_manifests RwLock poisoned");
        let mut rows: BTreeMap<String, AbilityCatalogSnapshotBuilder> = BTreeMap::new();

        for (key, record) in control_plane.records() {
            let owner = owner_kind_from_projection(record.authority().scope().owner_projection());
            let manifest = manifests.get(&key).map(Arc::clone);
            rows.entry(key.ability().to_string())
                .and_modify(|row| {
                    row.observe_owner(owner.clone());
                    row.observe_manifest(manifest.clone());
                })
                .or_insert_with(|| {
                    let mut row = AbilityCatalogSnapshotBuilder::new(owner);
                    row.observe_manifest(manifest);
                    row
                });
        }

        rows.into_iter()
            .map(|(name, row)| row.into_row(name))
            .collect()
    }

    /// Returns Some when an RPC handler is registered for `ability`.
    pub fn get_rpc(&self, ability: &str) -> Option<LocalRpcHandler> {
        self.resolve_rpc(ability)
    }

    /// True iff an RPC-mode handler is registered for `ability`,
    /// including the envelope-aware variant. Consults the dynamic
    /// execution index so hot-loaded MCP tools count as registered.
    pub fn has_rpc(&self, ability: &str) -> bool {
        if let Some(runtime) = self.runtime() {
            if self
                .runtime_ability_key_for_mode(ability, DescriptorCallMode::Rpc)
                .ok()
                .flatten()
                .and_then(|runtime_key| {
                    block_on_runtime_sync(runtime.ability_options(&runtime_key))
                })
                .map(|options| options.modes.rpc)
                .unwrap_or(false)
            {
                return true;
            }
        }
        self.execution_index
            .read()
            .expect("execution_index RwLock poisoned")
            .has_rpc(ability)
    }

    /// List all registered RPC ability names. Dynamic names must
    /// already be materialised in the runtime or execution index; the
    /// catalogue no longer synthesises fallback handlers on lookup miss.
    pub fn list_rpc_names(&self) -> Vec<String> {
        let mut names = BTreeSet::new();
        self.execution_index
            .read()
            .expect("execution_index RwLock poisoned")
            .extend_rpc_names(&mut names);
        names.into_iter().collect()
    }

    /// True iff any local handler is registered for `ability` in the
    /// catalogue's execution index.
    ///
    /// Unlike [`Self::has_rpc`], [`Self::has_stream`], and [`Self::has_bidi`],
    /// this is a pure catalogue check and never probes `LocalRuntime`.
    /// Use it for metadata/collision decisions where the caller only needs
    /// to know whether the public name is already occupied.
    pub fn has_registered_handler(&self, ability: &str) -> bool {
        self.execution_index
            .read()
            .expect("execution_index RwLock poisoned")
            .has_any_handler(ability)
    }

    /// Owned-clone counterpart that consults the unified execution index.
    /// Misses stay misses; post-boot abilities
    /// must be explicitly registered into `LocalRuntime` and the execution
    /// index.
    pub fn resolve_rpc(&self, ability: &str) -> Option<LocalRpcHandler> {
        self.execution_index
            .read()
            .expect("execution_index RwLock poisoned")
            .resolve_rpc(ability)
    }

    /// Owned-clone counterpart of `get_stream` that also consults
    /// the unified execution index. The dispatcher's `execute_stream`
    /// path uses this so hot-loaded MCP tools that register as
    /// streams (today: none — MCP `tools/call` is RPC-shaped, but
    /// the surface exists for symmetry and for future MCP server
    /// extensions) are dispatchable without mutating static lifecycle rows.
    pub fn resolve_stream(&self, ability: &str) -> Option<LocalStreamHandler> {
        self.execution_index
            .read()
            .expect("execution_index RwLock poisoned")
            .resolve_stream(ability)
    }

    /// Companion to `resolve_stream` for the envelope-aware variant.
    pub fn resolve_stream_with_env(&self, ability: &str) -> Option<LocalStreamHandlerWithEnvelope> {
        self.execution_index
            .read()
            .expect("execution_index RwLock poisoned")
            .resolve_stream_with_env(ability)
    }

    /// Owned-clone counterpart of `get_bidi` that also consults the
    /// unified execution index.
    pub fn resolve_bidi(&self, ability: &str) -> Option<LocalBidiHandler> {
        self.execution_index
            .read()
            .expect("execution_index RwLock poisoned")
            .resolve_bidi(ability)
    }

    /// Companion to `resolve_bidi` for the envelope-aware variant.
    pub fn resolve_bidi_with_env(&self, ability: &str) -> Option<LocalBidiHandlerWithEnvelope> {
        self.execution_index
            .read()
            .expect("execution_index RwLock poisoned")
            .resolve_bidi_with_env(ability)
    }

    /// Owned-clone counterpart of `rpc_with_env.get` that also
    /// consults the dynamic execution index. Dispatcher uses this in
    /// `execute_rpc` to keep the envelope-aware precedence rule
    /// (envelope handler beats args-only) honest for hot-loaded
    /// abilities too.
    pub fn resolve_rpc_with_env(&self, ability: &str) -> Option<LocalRpcHandlerWithEnvelope> {
        self.execution_index
            .read()
            .expect("execution_index RwLock poisoned")
            .resolve_rpc_with_env(ability)
    }

    /// Returns Some when a stream handler is registered for `ability`.
    pub fn get_stream(&self, ability: &str) -> Option<LocalStreamHandler> {
        self.resolve_stream(ability)
    }

    /// True iff a server-stream handler is registered for `ability`,
    /// including the envelope-aware variant. Consults the dynamic
    /// execution index.
    pub fn has_stream(&self, ability: &str) -> bool {
        if let Some(runtime) = self.runtime() {
            if self
                .runtime_ability_key_for_mode(ability, DescriptorCallMode::Stream)
                .ok()
                .flatten()
                .and_then(|runtime_key| {
                    block_on_runtime_sync(runtime.ability_options(&runtime_key))
                })
                .map(|options| options.modes.stream)
                .unwrap_or(false)
            {
                return true;
            }
        }
        self.execution_index
            .read()
            .expect("execution_index RwLock poisoned")
            .has_stream(ability)
    }

    /// Returns Some when a bidi handler is registered for `ability`.
    pub fn get_bidi(&self, ability: &str) -> Option<LocalBidiHandler> {
        self.resolve_bidi(ability)
    }

    /// True iff a bidirectional-stream handler is registered for
    /// `ability`, including the envelope-aware variant. Consults
    /// the unified execution index.
    pub fn has_bidi(&self, ability: &str) -> bool {
        if let Some(runtime) = self.runtime() {
            if self
                .runtime_ability_key_for_mode(ability, DescriptorCallMode::Bidi)
                .ok()
                .flatten()
                .and_then(|runtime_key| {
                    block_on_runtime_sync(runtime.ability_options(&runtime_key))
                })
                .map(|options| options.modes.bidi)
                .unwrap_or(false)
            {
                return true;
            }
        }
        self.execution_index
            .read()
            .expect("execution_index RwLock poisoned")
            .has_bidi(ability)
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
        let target = self
            .bind_invocation_target_to_control_plane(target, DescriptorCallMode::Rpc)
            .map_err(|err| anyhow::anyhow!("{err}; local Axon runtime loopback path"))?;
        crate::daemon::invocation::dispatch::local_runtime_invoker::invoke_local_rpc_sync(
            runtime, target,
        )
        .map_err(|err| {
            if crate::daemon::invocation::dispatch::local_runtime_invoker::is_not_found_error(&err)
            {
                anyhow::anyhow!("{err}; local Axon runtime loopback path")
            } else {
                anyhow::anyhow!("{err}")
            }
        })
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
        let target = self
            .bind_invocation_target_to_control_plane(target, DescriptorCallMode::Stream)
            .map_err(|err| {
                anyhow::anyhow!(
                    "no local stream handler registered for ability {ability} (local Axon runtime): {err}"
                )
            })?;
        runtime_stream_source(runtime, target).map_err(|err| {
            let msg = err.to_string();
            if crate::daemon::invocation::dispatch::local_runtime_invoker::is_not_found_error(&msg)
            {
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
        let target = self
            .bind_invocation_target_to_control_plane(target, DescriptorCallMode::Bidi)
            .map_err(|err| {
                anyhow::anyhow!(
                    "no local bidi handler registered for ability {ability} (local Axon runtime): {err}"
                )
            })?;
        runtime_bidi_source(runtime, target).map_err(|err| {
            let msg = err.to_string();
            if crate::daemon::invocation::dispatch::local_runtime_invoker::is_not_found_error(&msg)
            {
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
                    match crate::daemon::invocation::dispatch::local_runtime_invoker::open_local_stream(runtime, target)
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
                                match crate::daemon::invocation::dispatch::local_runtime_invoker::ability_frame_to_json(
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
                            crate::daemon::invocation::dispatch::local_runtime_invoker::ability_frame_to_json(&frame)
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
                    match crate::daemon::invocation::dispatch::local_runtime_invoker::open_local_bidi(runtime, target)
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
                            match crate::daemon::invocation::dispatch::local_runtime_invoker::ability_frame_to_json(
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
                                match crate::daemon::invocation::dispatch::local_runtime_invoker::ability_frame_to_json(
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
    use crate::core::domain::NodeId;
    use crate::daemon::federation::gateway_api::PeerInfo;
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
    fn fixed_authority_context_rejects_non_device_ura() {
        let err = AbilityAuthorityContext::for_device_authority_root("easynet:///r/acme/hub")
            .unwrap_err();

        assert!(matches!(
            err,
            AbilityControlPlaneError::InvalidDeviceAuthorityRoot { .. }
        ));
        assert!(
            err.to_string().contains("device URA"),
            "error should explain the authority root shape: {err}"
        );
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
                    "saw_subject": env.subject(),
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
    #[should_panic(expected = "one call mode must have exactly one handler family")]
    fn duplicate_rpc_handler_family_fails_at_registration() {
        // A programming error must fail at boot instead of silently
        // choosing whichever handler family the dispatcher checks first.
        let mut reg = AxonAbilityCatalog::new();
        reg.register_rpc(
            "x.dual",
            Arc::new(|_args: Value| Ok(json!({"path": "legacy"}))),
        );
        reg.register_rpc_with_envelope(
            "x.dual",
            Arc::new(|_env: EnvelopeContext, _args: Value| Ok(json!({"path": "envelope"}))),
        );
    }

    #[test]
    fn envelope_aware_handler_without_subject_still_dispatches() {
        // Callers that do not set an explicit resource subject still carry a
        // complete AXIOM tuple. The local runtime sets subject=callee for root
        // calls, and envelope-aware handlers must see that value instead of an
        // erased `None`.
        let mut reg = AxonAbilityCatalog::new();
        reg.register_rpc_with_envelope(
            "x.optional",
            Arc::new(|env: EnvelopeContext, _args: Value| {
                Ok(json!({
                    "subject_present": true,
                    "subject_eq_callee": env.subject() == env.callee(),
                }))
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
        assert_eq!(
            resp,
            json!({"subject_present": true, "subject_eq_callee": true})
        );
    }

    #[test]
    fn envelope_aware_stream_handler_receives_subject() {
        // Signed descriptor-bound server-streaming dispatches through the
        // Axon stream API (which carries a receipt), so `execute_stream`
        // runs the env-aware handler and surfaces the envelope subject in
        // the frame. Stream stays its own call mode — it is NOT collapsed
        // onto bidi.
        let mut reg = AxonAbilityCatalog::new();
        reg.register_stream_with_envelope(
            "x.subscribe",
            Arc::new(|env: EnvelopeContext, _args: Value| {
                let frame = json!({"subject_seen": env.subject()});
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
        // `daemon::invocation::routing::federation_invoke::invoke_via_federation_forward`
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

    /// SPEC §9.1.A acceptance test 4: `list_abilities` (the catalogue read
    /// surface behind `meta.list_abilities`) reads the canonical control-plane
    /// registry, NOT the union of the six String-keyed handler maps. We prove
    /// the source by removing only the control-plane record (the handler stays
    /// in its map) and asserting the ability disappears from the listing — a
    /// map-union implementation would still list it.
    #[test]
    fn list_abilities_reads_control_plane_not_handler_maps() {
        let mut reg = AxonAbilityCatalog::new();
        reg.register_rpc_with_owner("fs.read", OwnerKind::Device, ok_handler());
        assert!(reg.list_abilities().contains(&"fs.read".to_string()));
        // The handler is still installed in the rpc map, but its control-plane
        // record is gone.
        reg.clear_owner_for_test("fs.read");
        assert!(
            reg.has_rpc("fs.read"),
            "handler map still holds the closure"
        );
        assert!(
            !reg.list_abilities().contains(&"fs.read".to_string()),
            "list_abilities must read control-plane, not the handler map union"
        );
    }

    #[test]
    fn call_modes_by_ability_reads_control_plane_rows_once() {
        let mut reg = AxonAbilityCatalog::new();
        reg.register_rpc_with_owner("agent.chat", OwnerKind::Agent("agent".into()), ok_handler());
        reg.register_stream_with_owner(
            "agent.chat",
            OwnerKind::Agent("agent".into()),
            Arc::new(|_| Ok(StreamSource::Snapshot(vec![]))),
        );
        reg.register_bidi_with_owner("terminal.attach", OwnerKind::Device, trivial_bidi_handler());

        let modes = reg.call_modes_by_ability();
        assert_eq!(
            modes.get("agent.chat"),
            Some(&BTreeSet::from([
                DescriptorCallMode::Rpc,
                DescriptorCallMode::Stream
            ])),
            "same-name multi-mode abilities must be represented without probing LocalRuntime"
        );
        assert_eq!(
            modes.get("terminal.attach"),
            Some(&BTreeSet::from([DescriptorCallMode::Bidi])),
            "bidi-only rows must remain visible to catalogue hint projection"
        );
    }

    #[test]
    fn ability_catalog_snapshot_projects_owner_and_manifest_in_one_pass() {
        let mut reg = AxonAbilityCatalog::new();
        let manifest = crate::core::ability::spec::AbilityManifest::new(
            "read",
            "read a local file",
            json!({"type": "object"}),
        )
        .unwrap();
        reg.register_rpc_with_spec("fs.read", OwnerKind::Device, manifest, ok_handler());
        reg.register_bidi_with_owner("terminal.attach", OwnerKind::Device, trivial_bidi_handler());

        let rows = reg.ability_catalog_snapshot();
        let fs = rows
            .iter()
            .find(|row| row.name == "fs.read")
            .expect("fs.read row");
        assert_eq!(fs.owner, Some(OwnerKind::Device));
        assert_eq!(
            fs.manifest.as_ref().map(|m| m.description()),
            Some("read a local file")
        );
        let terminal = rows
            .iter()
            .find(|row| row.name == "terminal.attach")
            .expect("terminal.attach row");
        assert_eq!(terminal.owner, Some(OwnerKind::Device));
        assert!(
            terminal.manifest.is_none(),
            "handler-only registrations must not fabricate manifest bodies"
        );
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
        assert!(reg.get_bidi("terminal.attach").is_none());
        reg.register_bidi("terminal.attach", trivial_bidi_handler());
        assert!(reg.get_bidi("terminal.attach").is_some());
        // Negative: not visible on the other call modes.
        assert!(reg.get_rpc("terminal.attach").is_none());
        assert!(reg.get_stream("terminal.attach").is_none());
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
        reg.register_bidi("terminal.attach", trivial_bidi_handler());
        assert_eq!(
            reg.list_abilities(),
            vec!["observe.health", "permission.subscribe", "terminal.attach",],
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
            ability: "terminal.attach".into(),
            normalized_args: json!({}),
            call_mode: CallMode::Bidi,
            subject: None,
            causal_context: None,
        };
        let err = dispatcher.execute_bidi(target).unwrap_err();
        let msg = format!("{err}");
        assert!(msg.contains("terminal.attach"), "names ability: {msg}");
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
            ability: "terminal.attach".into(),
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
    fn register_rpc_writes_control_plane_record() {
        let mut reg = AxonAbilityCatalog::new();
        reg.register_rpc_with_owner("fs.read", OwnerKind::Device, ok_handler());

        let record = reg
            .control_plane_record_for_mode("fs.read", DescriptorCallMode::Rpc)
            .expect("control-plane lookup is unambiguous")
            .expect("control-plane record");
        assert_eq!(record.descriptor().name(), "fs.read");
        assert_eq!(record.descriptor().version().as_str(), "1.0.0");
        assert_eq!(record.descriptor().call_mode(), DescriptorCallMode::Rpc);
        assert!(record.authority().predicate().governs_advertise());
        assert!(record.authority().predicate().governs_invoke());
        assert_eq!(record.authority().scope().owner_projection(), "device");
        assert_eq!(
            *record.implementation().source(),
            AbilityImplSource::NativeDaemon
        );
        assert_eq!(
            record.implementation().runtime_env().label(),
            RuntimeEnv::daemon_native().label()
        );
        assert_ne!(record.descriptor().schema_hash().0, [0u8; 32]);
        assert_ne!(record.implementation().impl_hash(), [0u8; 32]);
    }

    /// SPEC §9.1.A: `control_plane_owner` must reconstruct the exact
    /// `OwnerKind` registration stored, for every owner kind. The legacy
    /// `owner` side table this once cross-checked against is now deleted, so
    /// the test asserts the registration-time truth directly. Covers the
    /// `.chat` special-case that regressed when the owner reconstruction
    /// took the MCP URA path (which rejected it to `None`).
    #[test]
    fn control_plane_owner_reconstructs_registered_owner_kind() {
        let mut reg = AxonAbilityCatalog::new();
        reg.register_rpc_with_owner("fs.read", OwnerKind::Device, ok_handler());
        reg.register_rpc_with_owner("hub.openai.list_models", OwnerKind::Hub, ok_handler());
        reg.register_rpc_with_owner(
            "codex.weather",
            OwnerKind::Agent("codex".to_string()),
            ok_handler(),
        );
        reg.register_rpc_with_owner(
            "codex.chat",
            OwnerKind::Agent("codex".to_string()),
            ok_handler(),
        );

        for (ability, expected) in [
            ("fs.read", OwnerKind::Device),
            ("hub.openai.list_models", OwnerKind::Hub),
            ("codex.weather", OwnerKind::Agent("codex".to_string())),
            ("codex.chat", OwnerKind::Agent("codex".to_string())),
        ] {
            assert_eq!(
                reg.control_plane_owner(ability),
                Some(expected),
                "control_plane_owner must reconstruct the registered owner for {ability:?}"
            );
        }
        assert_eq!(reg.control_plane_owner("not.registered"), None);
    }

    #[test]
    fn runtime_registration_binds_control_plane_proof_facts() {
        let mut reg = AxonAbilityCatalog::new();
        reg.register_rpc_with_owner("fs.read", OwnerKind::Device, ok_handler());

        let record = reg
            .control_plane_record_for_mode("fs.read", DescriptorCallMode::Rpc)
            .expect("control-plane lookup is unambiguous")
            .expect("control-plane record");
        let runtime = reg.runtime().expect("registry owns LocalRuntime");
        let runtime_key = reg
            .handler_control_plane_key("fs.read")
            .expect("handler authority key")
            .runtime_key()
            .expect("runtime key");
        let options =
            block_on_runtime_sync(runtime.ability_options(&runtime_key)).expect("runtime options");
        let proof = options.proof_for_mode(AxonCallMode::Rpc);

        assert_eq!(
            proof.descriptor_version,
            record.descriptor().version().as_str()
        );
        assert_eq!(proof.schema_hash, record.descriptor().schema_hash().0);
        assert_eq!(proof.impl_hash, record.implementation().impl_hash());
        assert!(!proof.is_unbound());
    }

    #[test]
    fn local_runtime_key_strips_agent_owner_prefix() {
        let pages_agent = crate::core::ura::agent_ura("localhost", "dev", "pages");
        let runtime_key = local_runtime_ability_key_for_authority(&pages_agent, "project_list")
            .expect("runtime key");

        assert_eq!(
            runtime_key,
            "easynet:///r/localhost/ability/dev.pages.project_list"
        );
    }

    #[test]
    fn local_runtime_key_preserves_device_domain_prefix() {
        let device = crate::core::ura::device_ura("localhost", "dev-a");
        let runtime_key = local_runtime_ability_key_for_authority(&device, "device.inspect")
            .expect("runtime key");

        assert_eq!(
            runtime_key,
            "easynet:///r/localhost/ability/device.dev-a.device.inspect"
        );
    }

    #[test]
    fn runtime_registration_binds_manifest_descriptor_version() {
        let mut reg = AxonAbilityCatalog::new();
        let manifest = crate::core::ability::spec::AbilityManifest::new(
            "read",
            "read a local file",
            json!({"type": "object"}),
        )
        .unwrap()
        .with_descriptor_version("2.0.0")
        .unwrap();
        reg.register_rpc_with_spec("fs.read", OwnerKind::Device, manifest, ok_handler());

        let control_plane_key = reg
            .handler_control_plane_key("fs.read")
            .expect("handler authority key");
        let record = reg
            .control_plane_record_for_authority_mode(
                control_plane_key.authority_root(),
                control_plane_key.ability(),
                DescriptorCallMode::Rpc,
            )
            .expect("authority/mode lookup is unambiguous")
            .expect("control-plane record");
        let runtime = reg.runtime().expect("registry owns LocalRuntime");
        let runtime_key = control_plane_key.runtime_key().expect("runtime key");
        let options =
            block_on_runtime_sync(runtime.ability_options(&runtime_key)).expect("runtime options");
        let proof = options.proof_for_mode(AxonCallMode::Rpc);

        assert_eq!(record.descriptor().version().as_str(), "2.0.0");
        assert_eq!(proof.descriptor_version, "2.0.0");
        assert_eq!(proof.schema_hash, record.descriptor().schema_hash().0);
        assert_eq!(proof.impl_hash, record.implementation().impl_hash());
    }

    /// SPEC §9.1.A Step 3: the control-plane-keyed manifest store carries
    /// the registered manifest body (the schema `meta.list_abilities`
    /// surfaces), and reports absence for an ability registered without a
    /// manifest. The legacy String-keyed table this once cross-checked is
    /// now deleted, so the test asserts the registration-time truth.
    #[test]
    fn control_plane_manifest_carries_registered_body_and_reports_absence() {
        let mut reg = AxonAbilityCatalog::new();
        let manifest = crate::core::ability::spec::AbilityManifest::new(
            "read",
            "read a local file",
            json!({"type": "object", "properties": {"path": {"type": "string"}}}),
        )
        .unwrap();
        reg.register_rpc_with_spec("fs.read", OwnerKind::Device, manifest.clone(), ok_handler());
        // An ability registered without a manifest must surface no body.
        reg.register_rpc_with_owner("admin.status", OwnerKind::Device, ok_handler());

        // Has-manifest: the schema body survives into the control-plane store.
        let control_plane = reg.control_plane_manifest("fs.read");
        assert_eq!(
            control_plane
                .as_ref()
                .expect("manifest present")
                .input_schema(),
            manifest.input_schema(),
            "input_schema body must survive into the control-plane store"
        );
        assert_eq!(
            control_plane.expect("manifest present").description(),
            manifest.description()
        );

        // No-manifest: the control-plane store reports absence.
        assert!(reg.control_plane_manifest("admin.status").is_none());
    }

    #[test]
    fn control_plane_rebind_without_manifest_removes_stale_manifest_facet() {
        let mut reg = AxonAbilityCatalog::new();
        let manifest = crate::core::ability::spec::AbilityManifest::new(
            "read",
            "read a local file",
            json!({"type": "object", "properties": {"path": {"type": "string"}}}),
        )
        .unwrap();
        reg.register_rpc_with_spec("fs.read", OwnerKind::Device, manifest, ok_handler());
        assert!(
            reg.control_plane_manifest("fs.read").is_some(),
            "test setup must publish a manifest facet"
        );

        reg.rebind_control_plane_record(
            "fs.read",
            &OwnerKind::Device,
            None,
            DescriptorCallMode::Rpc,
            AbilityImplSource::NativeDaemon,
            RuntimeEnv::daemon_native(),
        )
        .expect("control-plane rebind without manifest succeeds");

        assert!(
            reg.control_plane_manifest("fs.read").is_none(),
            "manifest facet must follow the accepted control-plane record; \
             a rebind without manifest must not retain the old schema body"
        );
    }

    #[test]
    fn remove_control_plane_record_for_authority_mode_removes_manifest_facet() {
        let mut reg = AxonAbilityCatalog::new();
        let manifest = crate::core::ability::spec::AbilityManifest::new(
            "read",
            "read a local file",
            json!({"type": "object", "properties": {"path": {"type": "string"}}}),
        )
        .unwrap();
        reg.register_rpc_with_spec("fs.read", OwnerKind::Device, manifest, ok_handler());
        let record = reg
            .control_plane_record_for_mode("fs.read", DescriptorCallMode::Rpc)
            .expect("control-plane lookup succeeds")
            .expect("control-plane record");
        let manifest_key = AbilityControlPlaneKey::for_authority(record.authority());
        assert!(
            reg.control_plane_manifests
                .read()
                .expect("manifest store lock")
                .contains_key(&manifest_key),
            "test setup must publish a manifest facet"
        );

        assert!(reg.remove_control_plane_record_for_authority_mode(
            manifest_key.authority_root(),
            manifest_key.ability(),
            manifest_key.call_mode(),
        ));

        assert!(
            !reg.control_plane_manifests
                .read()
                .expect("manifest store lock")
                .contains_key(&manifest_key),
            "authority-mode removal must delete the manifest facet keyed by the removed record"
        );
    }

    #[test]
    fn control_plane_keeps_rpc_and_stream_records_for_same_ability() {
        let mut reg = AxonAbilityCatalog::new_with_runtime_and_authority_context(
            LocalRuntime::new(),
            AbilityAuthorityContext::for_device_authority_root("easynet:///r/localhost/device/dev")
                .expect("test device URA is a valid device authority root"),
        );
        let stream_handler: LocalStreamHandler =
            Arc::new(|_args| Ok(StreamSource::Snapshot(vec![])));

        reg.register_rpc_with_owner(
            "agent.chat",
            OwnerKind::Agent("agent".to_string()),
            ok_handler(),
        );
        reg.register_stream_with_owner(
            "agent.chat",
            OwnerKind::Agent("agent".to_string()),
            stream_handler,
        );

        let rpc = reg
            .control_plane_record_for_mode("agent.chat", DescriptorCallMode::Rpc)
            .expect("rpc control-plane lookup is unambiguous")
            .expect("rpc control-plane record");
        let stream = reg
            .control_plane_record_for_mode("agent.chat", DescriptorCallMode::Stream)
            .expect("stream control-plane lookup is unambiguous")
            .expect("stream control-plane record");

        assert_eq!(rpc.descriptor().call_mode(), DescriptorCallMode::Rpc);
        assert_eq!(stream.descriptor().call_mode(), DescriptorCallMode::Stream);
        let err = reg
            .control_plane_record("agent.chat")
            .expect_err("mode-agnostic lookup must not collapse same ability/version modes");
        assert_eq!(err.matches.len(), 2);

        let runtime = reg.runtime().expect("registry owns LocalRuntime");
        let rpc_key = reg
            .handler_control_plane_key("agent.chat")
            .expect("handler authority key")
            .runtime_key()
            .expect("runtime rpc key");
        let options =
            block_on_runtime_sync(runtime.ability_options(&rpc_key)).expect("runtime options");
        let rpc_proof = options.proof_for_mode(AxonCallMode::Rpc);
        let stream_proof = options.proof_for_mode(AxonCallMode::Stream);

        assert_eq!(
            rpc_proof.descriptor_version,
            rpc.descriptor().version().as_str()
        );
        assert_eq!(rpc_proof.schema_hash, rpc.descriptor().schema_hash().0);
        assert_eq!(rpc_proof.impl_hash, rpc.implementation().impl_hash());
        assert_eq!(
            stream_proof.descriptor_version,
            stream.descriptor().version().as_str()
        );
        assert_eq!(
            stream_proof.schema_hash,
            stream.descriptor().schema_hash().0
        );
        assert_eq!(stream_proof.impl_hash, stream.implementation().impl_hash());
        assert_ne!(
            rpc_proof.impl_hash, stream_proof.impl_hash,
            "per-mode runtime proof bindings must not collapse RPC and Stream records"
        );
    }

    #[test]
    fn runtime_binding_facts_describe_daemon_binding_only() {
        let mut reg = AxonAbilityCatalog::new();
        reg.register_rpc_with_owner("fs.read", OwnerKind::Device, ok_handler());

        let record = reg
            .control_plane_record_for_mode("fs.read", DescriptorCallMode::Rpc)
            .expect("control-plane lookup is unambiguous")
            .expect("control-plane record");
        let facts = reg
            .runtime_binding_facts_for_mode("fs.read", DescriptorCallMode::Rpc)
            .expect("runtime binding lookup is unambiguous")
            .expect("runtime binding facts");

        assert_eq!(
            facts.descriptor_version,
            record.descriptor().version().to_string()
        );
        assert_eq!(facts.call_mode, DescriptorCallMode::Rpc);
        assert_eq!(
            facts.schema_hash,
            record.descriptor().schema_hash().prefixed_hex()
        );
        let descriptor_hash = record.descriptor().descriptor_hash().prefixed_hex();
        assert_eq!(
            facts.descriptor_hash.as_deref(),
            Some(descriptor_hash.as_str())
        );
        assert_eq!(facts.implementation_source, "native_daemon");
        assert_eq!(
            facts.runtime_env,
            record.implementation().runtime_env().label()
        );
        assert_eq!(facts.authority_owner_projection, "device");
        assert!(facts.governs_advertise);
        assert!(facts.governs_invoke);
    }

    #[test]
    fn runtime_binding_facts_return_every_registered_mode() {
        let mut reg = AxonAbilityCatalog::new_with_runtime_and_authority_context(
            LocalRuntime::new(),
            AbilityAuthorityContext::for_device_authority_root("easynet:///r/localhost/device/dev")
                .expect("test device URA is a valid device authority root"),
        );
        let stream_handler: LocalStreamHandler =
            Arc::new(|_args| Ok(StreamSource::Snapshot(vec![])));
        reg.register_rpc_with_owner(
            "agent.chat",
            OwnerKind::Agent("agent".to_string()),
            ok_handler(),
        );
        reg.register_stream_with_owner(
            "agent.chat",
            OwnerKind::Agent("agent".to_string()),
            stream_handler,
        );

        let facts = reg
            .runtime_binding_facts_for("agent.chat")
            .expect("runtime binding lookups are unambiguous");
        let modes = facts
            .iter()
            .map(|facts| facts.call_mode)
            .collect::<BTreeSet<_>>();
        assert_eq!(facts.len(), 2);
        assert_eq!(
            modes,
            BTreeSet::from([DescriptorCallMode::Rpc, DescriptorCallMode::Stream])
        );
    }

    #[test]
    fn owner_round_trips_for_representative_samples_via_register_with_owner() {
        // Pin the contract: every ability registered via the
        // `_with_owner` family round-trips through `lookup_owner`
        // with the exact OwnerKind variant the call site declared.
        // No name-string sniffing — the registry is the source of
        // truth for owner kind. M0 of the system-namespace migration.
        let mut reg = AxonAbilityCatalog::new();
        reg.register_rpc_with_owner("fs.read", OwnerKind::Device, ok_handler());
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

        assert_eq!(reg.control_plane_owner("fs.read"), Some(OwnerKind::Device));
        assert_eq!(
            reg.control_plane_owner("hub.openai.chat_completions"),
            Some(OwnerKind::Hub)
        );
        assert_eq!(
            reg.control_plane_owner("consent.decide"),
            Some(OwnerKind::Agent("consent".to_string()))
        );
        assert_eq!(
            reg.control_plane_owner("00000000-0000-0000-0000-000000000001.api_key.create"),
            Some(OwnerKind::User(
                "00000000-0000-0000-0000-000000000001".to_string()
            ))
        );
        // Unregistered ability returns None — synth paths can use
        // this to detect "not in our local registry" without falling
        // back to name-string sniffing.
        assert_eq!(reg.control_plane_owner("not.registered"), None);
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
            reg.control_plane_owner("legacy.shim.smoke"),
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

        assert_eq!(reg.control_plane_owner("a.rpc"), Some(OwnerKind::Hub));
        assert_eq!(
            reg.control_plane_owner("a.stream"),
            Some(OwnerKind::Agent("codex".to_string()))
        );
        assert_eq!(
            reg.control_plane_owner("a.bidi"),
            Some(OwnerKind::User("u-1".to_string()))
        );
        assert_eq!(
            reg.control_plane_owner("a.rpc.env"),
            Some(OwnerKind::Device)
        );
        assert_eq!(
            reg.control_plane_owner("a.stream.env"),
            Some(OwnerKind::Hub)
        );
        assert_eq!(
            reg.control_plane_owner("a.bidi.env"),
            Some(OwnerKind::Agent("web-builder".to_string()))
        );
    }

    // ── M1: dual-name (aliased) registration ────────────────────────

    #[test]
    fn device_owner_prefix_names_rejected_from_public_dispatch_surface() {
        // RFC-005 cleanup: device-owned system ability names are owner-local
        // names such as `fs.read`. Owner-prefixed names are not a public
        // catalogue or dispatch surface.
        let mut reg = AxonAbilityCatalog::new();
        reg.register_rpc_with_owner(
            "fs.read",
            OwnerKind::Device,
            Arc::new(|_args: Value| Ok(json!({"ok": true}))),
        );
        let dispatcher = Arc::new(reg);

        // Canonical works.
        let mut t_can = ping_target_local();
        t_can.ability = "fs.read".into();
        let r = dispatcher.execute_rpc(t_can).unwrap();
        assert_eq!(r, json!({"ok": true}));

        // Owner-prefixed public alias is gone.
        let mut t_leg = ping_target_local();
        t_leg.ability = "device.fs.read".into();
        let err = dispatcher.execute_rpc(t_leg).unwrap_err();
        let msg = format!("{err}");
        assert!(
            msg.contains("device.fs.read"),
            "AbilityNotFound message must name the rejected ability; got: {msg}"
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
            reg.control_plane_owner("hub.openai.chat_completions"),
            Some(OwnerKind::Hub)
        );
        // Post-M3 legacy lookup returns None (alias retired).
        assert_eq!(
            reg.control_plane_owner("01HUB.openai.chat_completions"),
            None,
            "post-M3 legacy name must not be in the owner table"
        );
    }

    #[test]
    fn catalogue_lists_owner_local_names_only() {
        // RFC-005: `list_abilities()` returns public owner-local names. It must
        // not synthesize a duplicated `device.*` owner prefix.
        let mut reg = AxonAbilityCatalog::new();
        reg.register_rpc_with_owner(
            "shell.run",
            OwnerKind::Device,
            Arc::new(|_args: Value| Ok(json!({}))),
        );
        let names = reg.list_abilities();
        assert!(names.iter().any(|n| n == "shell.run"));
        assert!(
            !names.iter().any(|n| n == "device.shell.run"),
            "device owner prefix must not appear in list_abilities()"
        );
    }

    #[test]
    #[should_panic(expected = "already registered")]
    fn duplicate_canonical_registration_is_rejected() {
        let mut reg = AxonAbilityCatalog::new();
        reg.register_rpc_with_owner(
            "device.x.foo",
            OwnerKind::Device,
            Arc::new(|_| Ok(json!({"who": "first"}))),
        );
        reg.register_rpc_with_owner(
            "device.x.foo",
            OwnerKind::Device,
            Arc::new(|_| Ok(json!({"who": "second"}))),
        );
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
                reg.control_plane_owner(n),
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
                reg.control_plane_owner(legacy),
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
        assert_eq!(
            reg.control_plane_owner("doomed.tool"),
            Some(OwnerKind::Device)
        );
        let was_present = reg
            .unregister("doomed.tool")
            .expect("static RPC unregister succeeds");
        assert!(
            was_present,
            "unregister must report the ability was present"
        );
        assert!(!reg.has_rpc("doomed.tool"));
        assert_eq!(reg.control_plane_owner("doomed.tool"), None);
    }

    #[test]
    fn unregister_idempotent_on_missing_ability() {
        let mut reg = AxonAbilityCatalog::new();
        // Returns false but does not panic — the contract callers
        // (B4 list_changed refresh diff) rely on for the
        // "tool went away mid-sync" race.
        assert!(!reg
            .unregister("never-was-there")
            .expect("missing unregister is idempotent"));
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
        reg.unregister("doomed.stream")
            .expect("static stream unregister succeeds");
        reg.unregister("doomed.bidi")
            .expect("static bidi unregister succeeds");
        assert!(!reg.has_stream("doomed.stream"));
        assert!(!reg.has_bidi("doomed.bidi"));
    }

    // ── Hot-reload dynamic execution rows ────────────────────────────

    #[test]
    fn hot_register_rpc_is_visible_to_resolve_rpc_and_has_rpc() {
        // The whole reason for dynamic execution rows to exist: a sink can
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
        )
        .expect("dynamic RPC registers");

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
        let manifest = crate::core::ability::spec::AbilityManifest::new(
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
        )
        .expect("dynamic RPC with manifest registers");

        // The hot-registered manifest is visible through the control-plane
        // store: the commit choke point dual-writes static and dynamic
        // registrations alike, so there is no static-vs-dynamic split to
        // probe any more (§9.1.A — the legacy String-keyed manifest table
        // that once distinguished them is gone).
        let m = reg
            .control_plane_manifest("mcp_wikipedia__search")
            .expect("hot-registered manifest visible in control-plane");
        assert_eq!(m.description(), "Search Wikipedia.");
    }

    #[test]
    fn hot_register_stream_with_explicit_impl_writes_control_plane_once() {
        let reg = Arc::new(AxonAbilityCatalog::new());
        let manifest = crate::core::ability::spec::AbilityManifest::new(
            "search",
            "Search Wikipedia.",
            serde_json::json!({"type": "object"}),
        )
        .unwrap();

        reg.hot_register_stream_with_spec_and_impl(
            "mcp_wikipedia__search",
            OwnerKind::Agent("mcp".to_string()),
            manifest,
            Arc::new(|_args| Ok(StreamSource::Snapshot(Vec::new()))),
            ControlPlaneImplementation::new(AbilityImplSource::Mcp, RuntimeEnv::mcp("wikipedia")),
        )
        .expect("dynamic MCP stream registers");

        let record = reg
            .control_plane_record_for_mode("mcp_wikipedia__search", DescriptorCallMode::Stream)
            .expect("stream control-plane lookup is unambiguous")
            .expect("stream control-plane record");
        assert_eq!(*record.implementation().source(), AbilityImplSource::Mcp);
        assert_eq!(
            record.implementation().runtime_env().label(),
            RuntimeEnv::mcp("wikipedia").label()
        );
    }

    #[test]
    fn hot_register_preserves_prior_dynamic_call_modes() {
        let reg = Arc::new(AxonAbilityCatalog::new());
        reg.hot_register_rpc(
            "plugin.mode_shift",
            OwnerKind::Device,
            Arc::new(|_args| Ok(serde_json::json!({"mode": "rpc"}))),
        )
        .expect("dynamic RPC registers");
        assert!(reg.has_rpc("plugin.mode_shift"));
        assert!(!reg.has_stream("plugin.mode_shift"));

        let manifest = crate::core::ability::spec::AbilityManifest::new(
            "mode_shift",
            "Mode-shift test ability.",
            serde_json::json!({"type": "object"}),
        )
        .unwrap();
        reg.hot_register_stream_with_spec(
            "plugin.mode_shift",
            OwnerKind::Device,
            manifest,
            Arc::new(|_args| Ok(StreamSource::Snapshot(Vec::new()))),
        )
        .expect("dynamic stream registers");

        assert!(
            reg.has_rpc("plugin.mode_shift"),
            "hot-registering a stream handler must preserve the existing dynamic RPC mode"
        );
        assert!(reg.has_stream("plugin.mode_shift"));
        assert!(
            reg.control_plane_record_for_mode("plugin.mode_shift", DescriptorCallMode::Rpc)
                .expect("RPC control-plane lookup is unambiguous")
                .is_some(),
            "RPC control-plane record must survive stream registration"
        );
        assert!(
            reg.control_plane_record_for_mode("plugin.mode_shift", DescriptorCallMode::Stream)
                .expect("stream control-plane lookup is unambiguous")
                .is_some(),
            "stream control-plane record must be present"
        );
        assert_eq!(
            reg.list_dynamic_abilities(),
            vec!["plugin.mode_shift".to_string()]
        );
    }

    #[test]
    fn hot_register_rejects_dynamic_owner_migration_without_unregister() {
        let reg = Arc::new(AxonAbilityCatalog::new());
        reg.hot_register_rpc(
            "plugin.owner_shift",
            OwnerKind::Device,
            Arc::new(|_args| Ok(serde_json::json!({"owner": "device"}))),
        )
        .expect("initial dynamic RPC registers");

        let err = reg
            .hot_register_rpc(
                "plugin.owner_shift",
                OwnerKind::Agent("mcp".to_string()),
                Arc::new(|_args| Ok(serde_json::json!({"owner": "agent"}))),
            )
            .expect_err("in-place dynamic owner migration is rejected");
        assert!(
            err.to_string()
                .contains("owner changes require hot_unregister"),
            "{err}"
        );

        assert_eq!(
            reg.control_plane_owner("plugin.owner_shift"),
            Some(OwnerKind::Device)
        );
        let out = reg
            .invoke_rpc_json("plugin.owner_shift", serde_json::json!({}))
            .expect("old runtime binding remains invokable");
        assert_eq!(out, serde_json::json!({"owner": "device"}));
    }

    #[test]
    fn hot_register_replaces_same_dynamic_handler_family() {
        let reg = Arc::new(AxonAbilityCatalog::new());
        reg.hot_register_rpc(
            "plugin.reload",
            OwnerKind::Device,
            Arc::new(|_args| Ok(serde_json::json!({"version": "old"}))),
        )
        .expect("initial dynamic RPC registers");

        reg.hot_register_rpc(
            "plugin.reload",
            OwnerKind::Device,
            Arc::new(|_args| Ok(serde_json::json!({"version": "new"}))),
        )
        .expect("same dynamic handler family can be replaced");

        let out = reg
            .invoke_rpc_json("plugin.reload", serde_json::json!({}))
            .expect("reloaded runtime binding remains invokable");
        assert_eq!(out, serde_json::json!({"version": "new"}));
        assert!(
            reg.resolve_rpc_with_env("plugin.reload").is_none(),
            "same-family replacement must not create an envelope handler"
        );
    }

    #[test]
    fn hot_register_rejects_dynamic_handler_family_switch_without_unregister() {
        let reg = Arc::new(AxonAbilityCatalog::new());
        reg.hot_register_rpc(
            "plugin.family_shift",
            OwnerKind::Device,
            Arc::new(|_args| Ok(serde_json::json!({"family": "rpc"}))),
        )
        .expect("initial dynamic RPC registers");

        let err = reg
            .hot_register_rpc_with_envelope(
                "plugin.family_shift",
                OwnerKind::Device,
                Arc::new(|_env, _args| Ok(serde_json::json!({"family": "rpc_with_env"}))),
            )
            .expect_err("in-place dynamic handler family switch is rejected");
        assert!(err.to_string().contains("handler family"), "{err}");

        assert!(
            reg.resolve_rpc_with_env("plugin.family_shift").is_none(),
            "rejected family switch must not install an envelope handler"
        );
        let out = reg
            .invoke_rpc_json("plugin.family_shift", serde_json::json!({}))
            .expect("old runtime binding remains invokable");
        assert_eq!(out, serde_json::json!({"family": "rpc"}));
    }

    #[test]
    fn control_plane_authority_mode_transaction_restores_prior_slice() {
        let catalog = AxonAbilityCatalog::new();
        let old_manifest = crate::core::ability::spec::AbilityManifest::new(
            "txn",
            "Old transactional descriptor.",
            serde_json::json!({"type": "object", "properties": {"old": {"type": "boolean"}}}),
        )
        .unwrap();
        let authority_scope = catalog
            .resolve_authority_scope_for_owner("plugin.txn", &OwnerKind::Device)
            .expect("device owner resolves authority scope");
        let control_plane_key =
            ControlPlaneAbilityKey::new(authority_scope.authority_root(), "plugin.txn")
                .for_mode(DescriptorCallMode::Rpc);
        catalog
            .register_dynamic_control_plane_with_scope_result(
                "plugin.txn",
                authority_scope.clone(),
                Some(&old_manifest),
                DescriptorCallMode::Rpc,
                &ControlPlaneImplementation::native_daemon(),
            )
            .expect("old control-plane record writes");
        let old_schema_hash = catalog
            .control_plane_record_for_mode("plugin.txn", DescriptorCallMode::Rpc)
            .expect("old lookup succeeds")
            .expect("old record exists")
            .descriptor()
            .schema_hash();

        let mut txn = catalog.begin_control_plane_authority_mode_transaction(
            control_plane_key.authority_root(),
            control_plane_key.ability(),
            control_plane_key.call_mode(),
        );
        let new_manifest = crate::core::ability::spec::AbilityManifest::new(
            "txn",
            "New transactional descriptor.",
            serde_json::json!({"type": "object", "properties": {"new": {"type": "boolean"}}}),
        )
        .unwrap();
        catalog
            .register_dynamic_control_plane_with_scope_result(
                "plugin.txn",
                authority_scope,
                Some(&new_manifest),
                DescriptorCallMode::Rpc,
                &ControlPlaneImplementation::native_daemon(),
            )
            .expect("new control-plane record writes");
        assert_ne!(
            catalog
                .control_plane_record_for_mode("plugin.txn", DescriptorCallMode::Rpc)
                .expect("new lookup succeeds")
                .expect("new record exists")
                .descriptor()
                .schema_hash(),
            old_schema_hash,
            "test must exercise an actual overwrite"
        );

        txn.rollback().expect("rollback restores prior slice");

        assert_eq!(
            catalog
                .control_plane_record_for_mode("plugin.txn", DescriptorCallMode::Rpc)
                .expect("restored lookup succeeds")
                .expect("restored record exists")
                .descriptor()
                .schema_hash(),
            old_schema_hash,
            "rollback must restore old descriptor facts"
        );
    }

    #[test]
    fn dynamic_registration_rollback_restores_prior_snapshot() {
        let catalog = AxonAbilityCatalog::new();
        let old_manifest = crate::core::ability::spec::AbilityManifest::new(
            "rollback",
            "Old rollback handler.",
            serde_json::json!({"type": "object", "properties": {"old": {"type": "boolean"}}}),
        )
        .unwrap();
        catalog
            .hot_register_rpc_with_spec(
                "plugin.rollback",
                OwnerKind::Device,
                old_manifest,
                Arc::new(|_args| Ok(serde_json::json!({"version": "old"}))),
            )
            .expect("old dynamic ability registers");
        let old_schema_hash = catalog
            .control_plane_record_for_mode("plugin.rollback", DescriptorCallMode::Rpc)
            .expect("old control-plane lookup succeeds")
            .expect("old control-plane record exists")
            .descriptor()
            .schema_hash();

        let control_plane_key = catalog
            .dynamic_control_plane_key("plugin.rollback")
            .expect("dynamic authority scope lookup succeeds")
            .expect("dynamic ability has a control-plane key")
            .for_mode(DescriptorCallMode::Rpc);
        let prior_dynamic = catalog
            .execution_index
            .read()
            .expect("execution_index RwLock poisoned")
            .dynamic_snapshot(&control_plane_key.key);
        let control_plane_txn = catalog.begin_control_plane_authority_mode_transaction(
            control_plane_key.authority_root(),
            control_plane_key.ability(),
            control_plane_key.call_mode(),
        );

        let new_manifest = crate::core::ability::spec::AbilityManifest::new(
            "rollback",
            "New rollback handler.",
            serde_json::json!({"type": "object", "properties": {"new": {"type": "boolean"}}}),
        )
        .unwrap();
        let new_authority_scope = catalog
            .resolve_authority_scope_for_owner("plugin.rollback", &OwnerKind::Device)
            .expect("device owner resolves authority scope");
        let written_key = catalog
            .register_dynamic_control_plane_with_scope_result(
                "plugin.rollback",
                new_authority_scope,
                Some(&new_manifest),
                DescriptorCallMode::Rpc,
                &ControlPlaneImplementation::native_daemon(),
            )
            .expect("new dynamic control-plane write succeeds");
        assert_eq!(written_key, control_plane_key);
        let mut txn = DynamicRegistrationTxn::after_control_plane(
            &catalog,
            written_key,
            prior_dynamic,
            control_plane_txn,
        );
        catalog
            .execution_index
            .write()
            .expect("execution_index RwLock poisoned")
            .install_dynamic(
                control_plane_key.key.clone(),
                DynamicRegistration::rpc_with_spec(
                    "plugin.rollback",
                    OwnerKind::Device,
                    new_manifest,
                    Arc::new(|_args| Ok(serde_json::json!({"version": "new"}))),
                ),
            );
        txn.mark_execution_index_committed()
            .expect("execution-index phase is legal");

        txn.rollback();

        let handler = catalog
            .resolve_rpc("plugin.rollback")
            .expect("rollback restores old dynamic handler");
        assert_eq!(
            handler(serde_json::json!({})).expect("old handler runs"),
            serde_json::json!({"version": "old"})
        );
        let restored_schema_hash = catalog
            .control_plane_record_for_mode("plugin.rollback", DescriptorCallMode::Rpc)
            .expect("restored control-plane lookup succeeds")
            .expect("restored control-plane record exists")
            .descriptor()
            .schema_hash();
        assert_eq!(
            restored_schema_hash, old_schema_hash,
            "rollback must restore the previous descriptor proof, not leave the failed write"
        );

        // SPEC §9.1.A Step 7 follow-up: the manifest facet must roll back in
        // lockstep with the record. Before this fix, the new manifest body
        // was stranded in `control_plane_manifests` keyed by the (now
        // restored) control-plane key. After rollback the store must surface
        // the OLD manifest (the "old" schema property), not the failed "new".
        let restored_manifest = catalog
            .control_plane_manifest("plugin.rollback")
            .expect("rollback must restore the prior manifest, not strand the new one");
        let props = restored_manifest
            .input_schema()
            .get("properties")
            .and_then(|p| p.as_object())
            .expect("manifest input_schema has properties");
        assert!(
            props.contains_key("old") && !props.contains_key("new"),
            "manifest store must restore the OLD manifest after rollback; got {:?}",
            restored_manifest.input_schema()
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
            "fs.read",
            OwnerKind::Device,
            Arc::new(|_args| Ok(serde_json::Value::Null)),
        );
        let reg = Arc::new(reg);

        reg.hot_register_rpc(
            "mcp_wikipedia__search",
            OwnerKind::Agent("mcp".to_string()),
            Arc::new(|_args| Ok(serde_json::Value::Null)),
        )
        .expect("dynamic RPC registers");
        assert!(reg.has_rpc("mcp_wikipedia__search"));
        assert!(reg.has_rpc("fs.read"));

        let removed = reg
            .hot_unregister("mcp_wikipedia__search")
            .expect("dynamic RPC unregisters");
        assert!(removed, "hot_unregister reports the entry was present");
        assert!(!reg.has_rpc("mcp_wikipedia__search"));
        // Static entry untouched.
        assert!(reg.has_rpc("fs.read"));

        // Calling hot_unregister on a static name is a silent no-op
        // (returns false) — the static side is the boot-time truth.
        let static_removed = reg
            .hot_unregister("fs.read")
            .expect("static hot-unregister is a no-op");
        assert!(
            !static_removed,
            "hot_unregister does not touch the static execution row"
        );
        assert!(reg.has_rpc("fs.read"));
    }

    #[test]
    fn hot_unregister_removes_dynamic_control_plane_manifest_facet() {
        let reg = Arc::new(AxonAbilityCatalog::new());
        let manifest = crate::core::ability::spec::AbilityManifest::new(
            "search",
            "Search a hot MCP index.",
            json!({"type": "object", "properties": {"query": {"type": "string"}}}),
        )
        .expect("valid manifest");
        reg.hot_register_rpc_with_spec(
            "mcp_wikipedia__search",
            OwnerKind::Agent("mcp".to_string()),
            manifest,
            Arc::new(|_args| Ok(serde_json::Value::Null)),
        )
        .expect("dynamic RPC registers");
        let record = reg
            .control_plane_record_for_mode("mcp_wikipedia__search", DescriptorCallMode::Rpc)
            .expect("control-plane lookup succeeds")
            .expect("control-plane record");
        let manifest_key = AbilityControlPlaneKey::for_authority(record.authority());
        assert!(
            reg.control_plane_manifests
                .read()
                .expect("manifest store lock")
                .contains_key(&manifest_key),
            "test setup must publish a dynamic manifest facet"
        );

        assert!(reg
            .hot_unregister("mcp_wikipedia__search")
            .expect("dynamic RPC unregisters"));

        assert!(
            !reg.control_plane_manifests
                .read()
                .expect("manifest store lock")
                .contains_key(&manifest_key),
            "hot_unregister must remove the manifest facet with the dynamic control-plane record"
        );
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
            "fs.read",
            OwnerKind::Device,
            Arc::new(|_args| Ok(serde_json::Value::Null)),
        );
        let reg = Arc::new(reg);
        reg.hot_register_rpc(
            "mcp_wikipedia__search",
            OwnerKind::Agent("mcp".to_string()),
            Arc::new(|_args| Ok(serde_json::Value::Null)),
        )
        .expect("dynamic RPC registers");
        let names = reg.list_abilities();
        assert!(names.contains(&"fs.read".to_string()));
        assert!(names.contains(&"mcp_wikipedia__search".to_string()));
        // Sorted so the catalogue surface is stable across calls.
        let mut sorted = names.clone();
        sorted.sort();
        assert_eq!(names, sorted);
    }

    #[test]
    fn static_ability_names_excludes_dynamic_rows() {
        let mut reg = AxonAbilityCatalog::new();
        reg.register_rpc_with_owner(
            "fs.read",
            OwnerKind::Device,
            Arc::new(|_args| Ok(serde_json::Value::Null)),
        );
        let reg = Arc::new(reg);
        reg.hot_register_rpc(
            "plugin.echo",
            OwnerKind::Device,
            Arc::new(|_args| Ok(serde_json::Value::Null)),
        )
        .expect("dynamic RPC registers");

        let names = reg.static_ability_names();

        assert!(names.contains(&"fs.read".to_string()));
        assert!(
            !names.contains(&"plugin.echo".to_string()),
            "plugin reload collision checks must compare only against boot-time static abilities"
        );
    }

    #[test]
    fn static_lookup_wins_over_dynamic_on_name_collision() {
        // If an upstream MCP server happens to emit a tool named
        // `fs.read`, the boot-registered system ability must
        // remain canonical. This is a defensive invariant: an
        // operator who deliberately wires such an upstream still
        // gets the system handler, not a 3rd-party reimplementation.
        let mut reg = AxonAbilityCatalog::new();
        reg.register_rpc_with_owner(
            "fs.read",
            OwnerKind::Device,
            Arc::new(|_args| Ok(serde_json::json!({"from": "static"}))),
        );
        let reg = Arc::new(reg);
        let err = reg
            .hot_register_rpc(
                "fs.read",
                OwnerKind::Agent("mcp".to_string()),
                Arc::new(|_args| Ok(serde_json::json!({"from": "dynamic"}))),
            )
            .expect_err("static collision is rejected");
        assert!(
            err.to_string().contains("shadows a static ability"),
            "{err}"
        );
        assert!(
            !reg.has_dynamic("fs.read"),
            "dynamic side must reject attempts to shadow static abilities"
        );
        let handler = reg.resolve_rpc("fs.read").unwrap();
        let out = handler(serde_json::json!({})).unwrap();
        assert_eq!(
            out,
            serde_json::json!({"from": "static"}),
            "static handler must win over dynamic on collision"
        );
        // Owner table reflects the static entry too — synth paths
        // that read `lookup_owner` see Device, not Agent.
        assert_eq!(reg.control_plane_owner("fs.read"), Some(OwnerKind::Device));
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

        let err = reg
            .hot_register_rpc(
                "device.keyring.sign",
                OwnerKind::Agent("malicious-plugin".to_string()),
                Arc::new(|_args| Ok(serde_json::json!({"from": "dynamic-runtime"}))),
            )
            .expect_err("static collision is rejected");
        assert!(
            err.to_string().contains("shadows a static ability"),
            "{err}"
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

    #[test]
    fn hot_remove_runtime_ability_rejects_static_catalog_names() {
        let mut reg = AxonAbilityCatalog::new();
        reg.register_rpc_with_owner(
            "device.keyring.sign",
            OwnerKind::Device,
            Arc::new(|_args| Ok(serde_json::json!({"from": "static-runtime"}))),
        );
        let reg = Arc::new(reg);

        assert!(
            !reg.hot_remove_runtime_ability("device.keyring.sign")
                .expect("static hot-remove is a no-op"),
            "dynamic removal API must reject static catalogue names"
        );
        let out = reg
            .invoke_rpc_json("device.keyring.sign", serde_json::json!({}))
            .expect("static runtime handler remains invokable after rejected hot remove");
        assert_eq!(
            out,
            serde_json::json!({"from": "static-runtime"}),
            "static catalogue and LocalRuntime must stay in sync"
        );
    }
}
