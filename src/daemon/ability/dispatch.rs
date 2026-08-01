// EasyNet CLI — Axon ability catalogue
// ====================================
//
// File: src/daemon/ability/dispatch.rs
// Description: Registration and metadata surface for daemon-hosted
//              Axon abilities. `AxonAbilityCatalog` preserves the
//              existing module-level `register(&mut catalog)` API,
//              but every registered handler is written through to
//              `axon_sdk::invocation::LocalRuntime` when the
//              daemon builds the catalogue. Production invocation
//              paths execute through that runtime; direct catalogue
//              execution helpers are test-only compatibility probes.
//
// Author: Silan Hu <silan.hu@u.nus.edu>
// Copyright (c) 2026 EasyNet. All rights reserved.

use std::collections::{BTreeMap, BTreeSet};
use std::future::Future;
use std::sync::{Arc, OnceLock};

use anyhow::Context as _;
use axon_sdk::invocation::{
    make_ability, AbilityCallModes, AbilityContext, AbilityFn, AbilityOptions, AxonError,
    CallMode as AxonCallMode, LocalRuntime,
};
use serde_json::Value;
use tokio::sync::{broadcast, mpsc};

use crate::daemon::ability::{
    public_route_ability_from_descriptor_ref, AbilityControlPlaneAuthorityModeLookupError,
    AbilityControlPlaneError, AbilityControlPlaneLookupError, AbilityControlPlaneRecord,
    AbilityControlPlaneRegistration, AbilityControlPlaneRegistry, AbilityDescriptor,
    AbilityImplSource, AuthorityScope, CallMode as DescriptorCallMode,
    HostedAgentDelegationContext, HostedAgentDelegationEnvelopeBinding, ReceiptSemantics,
    RuntimeEnv, HOSTED_AGENT_DELEGATION_METADATA_KEY,
};
#[cfg(test)]
use crate::daemon::invocation::routing::target::TargetScope;
use crate::daemon::invocation::routing::target::{CallMode, InvocationTarget};
use crate::daemon::persistence::agent_aggregate::{
    AgentAggregateRepository, AgentAggregateSnapshotLoadError, HostedLlmAgentIdentity,
};

/// Module-local sync→async bridge for the ability-dispatch registry
/// path. These calls sit on catalogue construction and discovery,
/// not per-frame dispatch, so correctness under all runtime hosts is
/// more important than the cheapest possible no-runtime policy.
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
        crate::support::async_bridge::SyncBridgeRuntimePolicy::BuildCurrentThreadTokio,
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
    runtime_invocation_context: Option<RuntimeInvocationContext>,
    hosted_agent_delegation: Option<HostedAgentDelegationContext>,
    session_authority:
        Option<crate::daemon::invocation::admission::authority_metadata::SessionAuthorityPayload>,
}

#[derive(Clone)]
struct RuntimeInvocationContext {
    context: Arc<AbilityContext>,
    runtime: Arc<LocalRuntime>,
    derived_admission: Option<
        Arc<
            dyn crate::daemon::execution::mission::invocation_gateway::MissionChildAdmissionProvider,
        >,
    >,
}

impl std::fmt::Debug for RuntimeInvocationContext {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter
            .debug_struct("RuntimeInvocationContext")
            .field("invocation_id", &self.context.invocation_id)
            .finish_non_exhaustive()
    }
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
    fn from_axon(signature: &axon_sdk::invocation::CallerSignature) -> Self {
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
            runtime_invocation_context: None,
            hosted_agent_delegation: None,
            session_authority: None,
        })
    }

    /// Attach the runtime-minted invocation context used for canonical child
    /// dispatch. Only the Axon adapter can construct this capability; handler
    /// arguments and persisted envelope projections cannot recreate it.
    #[must_use]
    fn with_runtime_invocation_context(
        mut self,
        context: Arc<AbilityContext>,
        runtime: Arc<LocalRuntime>,
        derived_admission: Option<
            Arc<
                dyn crate::daemon::execution::mission::invocation_gateway::MissionChildAdmissionProvider,
            >,
        >,
    ) -> Self {
        self.runtime_invocation_context = Some(RuntimeInvocationContext {
            context,
            runtime,
            derived_admission,
        });
        self
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

    /// Attach the post-admission session authority projection. The adapter
    /// owns construction; handlers receive only typed binding facts.
    #[must_use]
    pub(crate) fn with_session_authority(
        mut self,
        session_authority: Option<
            crate::daemon::invocation::admission::authority_metadata::SessionAuthorityPayload,
        >,
    ) -> Self {
        self.session_authority = session_authority;
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
        Self::for_test_targeted_ability(caller, "easynet:///r/test/device/local", ability, subject)
    }

    /// Build a deterministic complete context with an explicit callee.
    /// Authority-sensitive handlers use this fixture so Device/Hub tests do
    /// not silently inherit the historical test Device target.
    #[cfg(test)]
    pub fn for_test_targeted_ability(
        caller: impl Into<String>,
        callee: impl Into<String>,
        ability: impl Into<String>,
        subject: impl Into<String>,
    ) -> Self {
        let caller = caller.into();
        let callee = callee.into();
        let ability = ability.into();
        let subject = subject.into();
        Self::new(EnvelopeContextParts {
            invocation_id: "test-invocation".to_string(),
            caller,
            callee,
            ability,
            subject,
            invocation_nonce: vec![0xA5; 16],
            causal_context: serde_json::json!({"form": "none"}),
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

    /// Return the runtime-minted capability for deriving a proven child
    /// invocation from this admitted parent.
    pub(crate) fn runtime_invocation_context(&self) -> Option<Arc<AbilityContext>> {
        self.runtime_invocation_context
            .as_ref()
            .map(|capability| Arc::clone(&capability.context))
    }

    /// Return the exact runtime that admitted this handler invocation.
    ///
    /// Child dispatch must use this instance. Rebuilding a runtime or loading
    /// another signer would create a second invocation authority.
    pub(crate) fn shared_runtime(&self) -> Option<Arc<LocalRuntime>> {
        self.runtime_invocation_context
            .as_ref()
            .map(|capability| Arc::clone(&capability.runtime))
    }

    /// Return the daemon runtime-admission capability bound to the exact runtime
    /// host that admitted this invocation.
    pub(crate) fn derived_invocation_admission(
        &self,
    ) -> Option<
        Arc<
            dyn crate::daemon::execution::mission::invocation_gateway::MissionChildAdmissionProvider,
        >,
    >{
        self.runtime_invocation_context
            .as_ref()
            .and_then(|capability| capability.derived_admission.as_ref().map(Arc::clone))
    }

    #[must_use]
    pub fn hosted_agent_delegation(&self) -> Option<&HostedAgentDelegationContext> {
        self.hosted_agent_delegation.as_ref()
    }

    #[must_use]
    pub(crate) fn session_authority(
        &self,
    ) -> Option<&crate::daemon::invocation::admission::authority_metadata::SessionAuthorityPayload>
    {
        self.session_authority.as_ref()
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
    manifest: Option<&'a crate::daemon::ability::manifest::AbilityManifest>,
    call_mode: DescriptorCallMode,
    admission_action: crate::daemon::ability::descriptors::AdmissionAction,
    receipt_semantics: ReceiptSemantics,
    implementation: ControlPlaneImplementation,
}

struct ResolvedControlPlaneRegistration<'a> {
    ability: &'a str,
    authority_scope: AuthorityScope,
    manifest: Option<&'a crate::daemon::ability::manifest::AbilityManifest>,
    call_mode: DescriptorCallMode,
    admission_action: crate::daemon::ability::descriptors::AdmissionAction,
    receipt_semantics: ReceiptSemantics,
    implementation: ControlPlaneImplementation,
    owner_label: String,
}

fn manifest_admission_action(
    manifest: Option<&crate::daemon::ability::manifest::AbilityManifest>,
) -> anyhow::Result<crate::daemon::ability::descriptors::AdmissionAction> {
    let raw = manifest
        .and_then(crate::daemon::ability::manifest::AbilityManifest::admission_action)
        .ok_or_else(|| {
            anyhow::anyhow!("canonical ability registration requires explicit admission_action")
        })?;
    serde_json::from_value(serde_json::Value::String(raw.to_string()))
        .map_err(|error| anyhow::anyhow!("invalid canonical admission_action {raw:?}: {error}"))
}

pub struct ControlPlaneAuthorityRebind<'a> {
    pub ability: &'a str,
    pub authority_scope: AuthorityScope,
    pub manifest: Option<&'a crate::daemon::ability::manifest::AbilityManifest>,
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
///   * `Finite(rx)` — bounded, backpressured finite production. The
///     producer may surface a terminal error through the channel. Used
///     for disk-backed or computed results that must not be materialised
///     as one in-memory snapshot and must not use lossy broadcast.
///
/// The `From` impls let handlers return either a `Vec<Value>` or a
/// `broadcast::Receiver<Value>` directly via `.into()`.
#[derive(Debug)]
pub enum StreamSource {
    Snapshot(Vec<Value>),
    Live(broadcast::Receiver<Value>),
    SnapshotThenLive(Vec<Value>, broadcast::Receiver<Value>),
    Finite(tokio::sync::mpsc::Receiver<anyhow::Result<Value>>),
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
            StreamSource::Finite(_) => Vec::new(),
        }
    }
}

/// One in-process stream handler. Returns an eager snapshot, a live
/// broadcast, or a bounded finite producer — see `StreamSource` for the
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
        crate::daemon::identity::local_invocation::system_verifying_key().map_err(|error| {
            AxonError::internal(format!(
                "hosted_agent_delegation: daemon-local signer unavailable: {error}"
            ))
        })?,
    )
    .map(Some)
    .map_err(|err| AxonError::invalid_argument(format!("hosted_agent_delegation: {err}")))
}

#[derive(Clone)]
struct RuntimeHandlerContext {
    runtime: Arc<LocalRuntime>,
    derived_admission: Arc<
        OnceLock<
            Arc<
                dyn crate::daemon::execution::mission::invocation_gateway::MissionChildAdmissionProvider,
            >,
        >,
    >,
}

async fn envelope_context_from_axon(
    ctx: &Arc<AbilityContext>,
    runtime_host: Option<RuntimeHandlerContext>,
) -> Result<EnvelopeContext, AxonError> {
    let signed = ctx.signed_envelope().cloned().ok_or_else(|| {
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
    let session_authority = crate::daemon::invocation::admission::authority_metadata::project_admitted_session_authority(
        &ctx.request_metadata,
    )
    .map_err(|err| AxonError::invalid_argument(format!("session_authority: {err}")))?;
    EnvelopeContext::new(EnvelopeContextParts {
        invocation_id: ctx.invocation_id.clone(),
        caller,
        callee,
        ability,
        subject: envelope_subject,
        invocation_nonce: invocation_nonce.to_vec(),
        causal_context:
            crate::daemon::invocation::causal_context_projection::causal_context_projection(
                &envelope.causal_context,
            ),
        caller_signature,
    })
    .map(|context| {
        let context = match runtime_host {
            Some(runtime_host) => context.with_runtime_invocation_context(
                Arc::clone(ctx),
                runtime_host.runtime,
                runtime_host.derived_admission.get().map(Arc::clone),
            ),
            None => context,
        };
        context
            .with_hosted_agent_delegation(hosted_agent_delegation)
            .with_session_authority(session_authority)
    })
    .map_err(|err| {
        AxonError::internal(format!(
            "local_runtime_adapter: incomplete Axon envelope projection: {err}"
        ))
    })
}

fn rpc_env_handler_to_ability_fn(
    handler: LocalRpcHandlerWithEnvelope,
    runtime_host: RuntimeHandlerContext,
) -> AbilityFn {
    make_ability(move |ctx| {
        let handler = Arc::clone(&handler);
        let runtime_host = runtime_host.clone();
        async move {
            let value = payload_to_json_value(&ctx.payload).map_err(|e| *e)?;
            let env = envelope_context_from_axon(&ctx, Some(runtime_host)).await?;
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
        StreamSource::Finite(mut rx) => {
            while let Some(frame) = rx.recv().await {
                let frame = frame.map_err(|err| {
                    AxonError::internal(format!(
                        "local_runtime_adapter: finite stream producer failed: {err:#}"
                    ))
                })?;
                emit_json_progress(&ctx, frame).await?;
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

fn stream_env_handler_to_ability_fn(
    handler: LocalStreamHandlerWithEnvelope,
    runtime_host: Option<RuntimeHandlerContext>,
) -> AbilityFn {
    make_ability(move |ctx| {
        let handler = Arc::clone(&handler);
        let runtime_host = runtime_host.clone();
        async move {
            let value = payload_to_json_value(&ctx.payload).map_err(|e| *e)?;
            let env = envelope_context_from_axon(&ctx, runtime_host).await?;
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
        stream_env_handler_to_ability_fn(handler, None),
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

fn bidi_env_handler_to_ability_fn(
    handler: LocalBidiHandlerWithEnvelope,
    runtime_host: RuntimeHandlerContext,
) -> AbilityFn {
    make_ability(move |ctx| {
        let handler = Arc::clone(&handler);
        let runtime_host = runtime_host.clone();
        async move {
            let value = payload_to_json_value(&ctx.payload).map_err(|e| *e)?;
            let env = envelope_context_from_axon(&ctx, Some(runtime_host)).await?;
            let source = handler(env, value).map_err(|err| {
                AxonError::internal(format!(
                    "local_runtime_adapter: env bidi handler returned error: {err}"
                ))
            })?;
            run_bidi_source(ctx, source).await
        }
    })
}

fn runtime_handler_set_to_ability_fn(
    name: String,
    handlers: RuntimeHandlerSet,
    runtime: Arc<LocalRuntime>,
    derived_admission: Arc<
        OnceLock<
            Arc<
                dyn crate::daemon::execution::mission::invocation_gateway::MissionChildAdmissionProvider,
            >,
        >,
    >,
) -> AbilityFn {
    let runtime_host = RuntimeHandlerContext {
        runtime,
        derived_admission,
    };
    let rpc_fn = handlers.rpc.map(rpc_handler_to_ability_fn);
    let stream_fn = handlers.stream.map(stream_handler_to_ability_fn);
    let bidi_fn = handlers.bidi.map(bidi_handler_to_ability_fn);
    let rpc_env_fn = handlers
        .rpc_with_env
        .map(|handler| rpc_env_handler_to_ability_fn(handler, runtime_host.clone()));
    let stream_env_fn = handlers
        .stream_with_env
        .map(|handler| stream_env_handler_to_ability_fn(handler, Some(runtime_host.clone())));
    let bidi_env_fn = handlers
        .bidi_with_env
        .map(|handler| bidi_env_handler_to_ability_fn(handler, runtime_host));

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
    /// `node.describe`, `skill.list`, `device.keyring.sign`.
    Device,
    /// Hosted by the realm Authority plane.
    ///
    /// Ability names remain owner-local; the realm Authority URA carries the
    /// ownership fact, so a duplicated `hub.*` prefix is invalid. The
    /// control-plane projection string is `"authority"`.
    RealmAuthority,
    /// Hosted by a sub-agent on this device. The contained string is
    /// the sub-agent's `agent_id` (e.g. `"codex"`, `"web-builder"`,
    /// `"consent"`). The full owner URA is
    /// `easynet:///r/<realm>/agent/<user-uuid>.<agent_id>`; the
    /// realm + user are read from credentials at advertise time.
    Agent(String),
    /// Hosted by the user's account agent. Axon Ability URA ownership
    /// has Authority, Device, and Agent branches, but no raw User branch; the
    /// contained string is therefore projected to
    /// `agent/<user-id>.account` at the protocol boundary while the
    /// product owner projection remains `user:<id>`.
    User(String),
}

/// Canonical authority selected to govern the daemon-process invocation
/// ledger.
///
/// Owner projection and runtime root travel as one value so governance
/// registration cannot select a descriptor owner from the catalog while
/// independently rediscovering the ledger root from ambient product state.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct LedgerGovernanceAuthority {
    owner: OwnerKind,
    runtime_owner_ura: String,
}

impl LedgerGovernanceAuthority {
    pub(crate) fn owner(&self) -> &OwnerKind {
        &self.owner
    }

    pub(crate) fn runtime_owner_ura(&self) -> &str {
        &self.runtime_owner_ura
    }
}

impl OwnerKind {
    fn authority_projection(&self) -> String {
        match self {
            OwnerKind::Device => "device".to_string(),
            OwnerKind::RealmAuthority => "authority".to_string(),
            OwnerKind::Agent(agent_id) => format!("agent:{agent_id}"),
            OwnerKind::User(user_id) => format!("user:{user_id}"),
        }
    }

    fn authority_scope(
        &self,
        context: &AbilityAuthorityContext,
    ) -> Result<AuthorityScope, AbilityControlPlaneError> {
        context.authority_scope_for(self)
    }
}

/// Inverse of [`OwnerKind::authority_scope`]'s `owner_projection` encoding:
/// reconstruct the `OwnerKind` from the canonical projection string a
/// control-plane record stores (`device` / `authority` / `agent:<id>` /
/// `user:<id>`). Kept adjacent to the forward mapping so the two cannot
/// drift. Returns `None` for an unrecognized projection rather than
/// guessing — an owner the registry never wrote is not an owner.
fn owner_kind_from_projection(owner_projection: &str) -> Option<OwnerKind> {
    match owner_projection {
        "device" => Some(OwnerKind::Device),
        "authority" => Some(OwnerKind::RealmAuthority),
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

#[derive(Debug, Clone)]
struct CanonicalDeviceAuthority {
    ura: String,
    realm: String,
    device_id: String,
}

/// Typed failures for the mutable hosted-Agent authority inventory.
///
/// The inventory is deliberately narrower than the general ability authority
/// context: callers cannot insert an arbitrary URA. Enrollment re-reads and
/// validates the durable `agents.json` row plus its `local-agents.json`
/// identity binding, while revocation requires those durable rows to be gone.
#[derive(Debug, thiserror::Error, Clone, PartialEq, Eq)]
pub(crate) enum HotAgentAuthorityInventoryError {
    #[error("this catalog does not own a hot hosted-Agent authority inventory")]
    UnsupportedAuthorityContext,
    #[error("load durable hosted-Agent registry for {agent:?}: {reason}")]
    DurableRegistryUnreadable { agent: String, reason: String },
    #[error("hosted Agent {agent:?} has no durable agents.json row")]
    DurableAgentMissing { agent: String },
    #[error("load hosted-Agent identity registry for {agent:?}: {reason}")]
    IdentityRegistryUnreadable { agent: String, reason: String },
    #[error("hosted Agent {agent:?} has no llm identity row in local-agents.json")]
    IdentityMissing { agent: String },
    #[error("hosted Agent {agent:?} has multiple llm identity rows")]
    IdentityAmbiguous { agent: String },
    #[error("hosted Agent {agent:?} identity {authority_root:?} is invalid: {reason}")]
    IdentityInvalid {
        agent: String,
        authority_root: String,
        reason: String,
    },
    #[error(
        "hosted Agent {agent:?} authority root conflicts with enrolled root: enrolled={enrolled:?}, requested={requested:?}"
    )]
    AuthorityConflict {
        agent: String,
        enrolled: String,
        requested: String,
    },
    #[error("cannot revoke hosted Agent {agent:?}: durable agents.json row still exists")]
    DurableAgentStillPresent { agent: String },
    #[error("cannot revoke hosted Agent {agent:?}: local-agents.json identity still exists")]
    IdentityStillPresent { agent: String },
    #[error("hosted Agent {agent:?} authority root is not enrolled")]
    AuthorityNotEnrolled { agent: String },
    #[error("hosted Agent {agent:?} authority inventory lock is poisoned")]
    InventoryPoisoned { agent: String },
    #[error("hosted Agent {agent:?} authority {counter} counter overflow")]
    CounterOverflow {
        agent: String,
        counter: &'static str,
    },
}

#[derive(Debug, Clone, PartialEq, Eq)]
struct PersistedHotAgentAuthority {
    agent: String,
    authority_root: String,
}

impl PersistedHotAgentAuthority {
    fn load(
        device: &CanonicalDeviceAuthority,
        agent: &str,
    ) -> Result<Self, HotAgentAuthorityInventoryError> {
        let registry_key = crate::core::agent::id::AgentId::parse(agent)
            .map_err(
                |error| HotAgentAuthorityInventoryError::DurableRegistryUnreadable {
                    agent: agent.to_string(),
                    reason: format!("hosted Agent registry key projection failed: {error}"),
                },
            )?
            .to_string();
        let snapshot = AgentAggregateRepository::try_load_snapshot()
            .map_err(|error| hot_agent_authority_snapshot_error(agent, error))?;
        if !snapshot.has_registered_agent(&registry_key) {
            return Err(HotAgentAuthorityInventoryError::DurableAgentMissing {
                agent: agent.to_string(),
            });
        }

        let identity = match snapshot.hosted_llm_agent_identity(agent) {
            HostedLlmAgentIdentity::Present(identity) => identity,
            HostedLlmAgentIdentity::Missing => {
                return Err(HotAgentAuthorityInventoryError::IdentityMissing {
                    agent: agent.to_string(),
                })
            }
            HostedLlmAgentIdentity::Ambiguous => {
                return Err(HotAgentAuthorityInventoryError::IdentityAmbiguous {
                    agent: agent.to_string(),
                })
            }
        };

        let invalid = |reason: String| HotAgentAuthorityInventoryError::IdentityInvalid {
            agent: agent.to_string(),
            authority_root: identity.agent_ura.clone(),
            reason,
        };
        let parsed = crate::core::ura::parse_ura(&identity.agent_ura)
            .map_err(|error| invalid(error.to_string()))?;
        let Some((_owner_user_id, agent_id)) = parsed.agent_ids() else {
            return Err(invalid(
                "expected a user-hosted Agent URA, not a device-sponsored System Agent".to_string(),
            ));
        };
        if parsed.kind != crate::core::ura::URAKind::Agent
            || parsed.realm != device.realm
            || agent_id != agent
        {
            return Err(invalid(format!(
                "expected realm {:?} and agent id {:?}",
                device.realm, agent
            )));
        }

        let expected_signing_authority = format!("hosted_by:{}", device.ura);
        if snapshot.host_device_agent_ura() != device.ura
            || identity.signing_authority != expected_signing_authority
        {
            return Err(invalid(
                "identity is not bound to the catalog's canonical Device authority".to_string(),
            ));
        }

        Ok(Self {
            agent: agent.to_string(),
            authority_root: identity.agent_ura.clone(),
        })
    }
}

fn hot_agent_authority_snapshot_error(
    agent: &str,
    error: AgentAggregateSnapshotLoadError,
) -> HotAgentAuthorityInventoryError {
    match error {
        AgentAggregateSnapshotLoadError::RegistryUnreadable { source } => {
            HotAgentAuthorityInventoryError::DurableRegistryUnreadable {
                agent: agent.to_string(),
                reason: format!("{source:#}"),
            }
        }
        AgentAggregateSnapshotLoadError::IdentityUnreadable { source } => {
            HotAgentAuthorityInventoryError::IdentityRegistryUnreadable {
                agent: agent.to_string(),
                reason: format!("{source:#}"),
            }
        }
    }
}

/// Receipt proving that one authority root was admitted from validated
/// lifecycle state. The fields are private so rollback/revoke cannot be driven
/// by caller-supplied strings.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct HotAgentAuthorityEnrollment {
    agent: String,
    authority_root: String,
    inserted: bool,
    incarnation: u64,
}

impl HotAgentAuthorityEnrollment {
    pub(crate) fn authority_root(&self) -> &str {
        &self.authority_root
    }
}

/// Catalog-owned mutable authority inventory for post-boot hosted Agents.
///
/// Static boot roots and hot enrollments share this single map, so authority
/// admission never consults ambient HOME state during a dynamic registration.
/// The lifecycle proof is checked exactly at the enroll/revoke boundary.
pub trait HostedAgentAuthorityInventory: Send + Sync {
    fn resolve_signing_lease(&self, agent_ura: &str) -> Option<HostedAgentAuthorityLease>;
    fn validate_signing_lease(&self, agent_ura: &str, lease: HostedAgentAuthorityLease) -> bool;
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct HostedAgentAuthorityLease {
    generation: u64,
}

impl HostedAgentAuthorityLease {
    pub fn for_generation(generation: u64) -> Self {
        Self { generation }
    }

    pub fn generation(self) -> u64 {
        self.generation
    }
}

#[derive(Debug)]
struct HotAgentAuthorityInventoryState {
    roots: BTreeMap<String, HotAgentAuthorityEntry>,
    generation: u64,
    next_incarnation: u64,
}

impl HotAgentAuthorityInventoryState {
    fn allocate_incarnation(
        &mut self,
        agent: &str,
    ) -> Result<u64, HotAgentAuthorityInventoryError> {
        let incarnation = self.next_incarnation;
        self.next_incarnation = self.next_incarnation.checked_add(1).ok_or_else(|| {
            HotAgentAuthorityInventoryError::CounterOverflow {
                agent: agent.to_string(),
                counter: "incarnation",
            }
        })?;
        Ok(incarnation)
    }

    fn advance_generation(&mut self, agent: &str) -> Result<(), HotAgentAuthorityInventoryError> {
        self.generation = self.generation.checked_add(1).ok_or_else(|| {
            HotAgentAuthorityInventoryError::CounterOverflow {
                agent: agent.to_string(),
                counter: "generation",
            }
        })?;
        Ok(())
    }
}

#[derive(Debug, Clone)]
struct HotAgentAuthorityEntry {
    authority_root: String,
    incarnation: u64,
}

#[derive(Debug)]
struct HotAgentAuthorityInventory {
    device: CanonicalDeviceAuthority,
    state: std::sync::RwLock<HotAgentAuthorityInventoryState>,
}

impl HostedAgentAuthorityInventory for HotAgentAuthorityInventory {
    fn resolve_signing_lease(&self, agent_ura: &str) -> Option<HostedAgentAuthorityLease> {
        let state = self.state.read().ok()?;
        state
            .roots
            .values()
            .any(|entry| entry.authority_root == agent_ura)
            .then_some(HostedAgentAuthorityLease::for_generation(state.generation))
    }

    fn validate_signing_lease(&self, agent_ura: &str, lease: HostedAgentAuthorityLease) -> bool {
        self.state
            .read()
            .map(|state| {
                state.generation == lease.generation()
                    && state
                        .roots
                        .values()
                        .any(|entry| entry.authority_root == agent_ura)
            })
            .unwrap_or(false)
    }
}

impl HotAgentAuthorityInventory {
    fn new(device: CanonicalDeviceAuthority, roots: BTreeMap<String, String>) -> Arc<Self> {
        let mut next_incarnation = 1_u64;
        let roots = roots
            .into_iter()
            .map(|(agent, authority_root)| {
                let incarnation = next_incarnation;
                next_incarnation = next_incarnation
                    .checked_add(1)
                    .expect("hosted Agent boot authority inventory exceeds u64 incarnations");
                (
                    agent,
                    HotAgentAuthorityEntry {
                        authority_root,
                        incarnation,
                    },
                )
            })
            .collect();
        Arc::new(Self {
            device,
            state: std::sync::RwLock::new(HotAgentAuthorityInventoryState {
                roots,
                generation: 1,
                next_incarnation,
            }),
        })
    }

    fn authority_root(&self, agent: &str) -> Option<String> {
        self.state
            .read()
            .ok()?
            .roots
            .get(agent)
            .map(|entry| entry.authority_root.clone())
    }

    fn declare_static_root(
        &self,
        agent: &str,
        authority_root: &str,
    ) -> Result<(), HotAgentAuthorityInventoryError> {
        let mut state =
            self.state
                .write()
                .map_err(|_| HotAgentAuthorityInventoryError::InventoryPoisoned {
                    agent: agent.to_string(),
                })?;
        match state.roots.get(agent) {
            Some(existing) if existing.authority_root == authority_root => Ok(()),
            Some(existing) => Err(HotAgentAuthorityInventoryError::AuthorityConflict {
                agent: agent.to_string(),
                enrolled: existing.authority_root.clone(),
                requested: authority_root.to_string(),
            }),
            None => {
                state.advance_generation(agent)?;
                let incarnation = state.allocate_incarnation(agent)?;
                state.roots.insert(
                    agent.to_string(),
                    HotAgentAuthorityEntry {
                        authority_root: authority_root.to_string(),
                        incarnation,
                    },
                );
                Ok(())
            }
        }
    }

    fn enroll_persisted(
        &self,
        agent: &str,
    ) -> Result<HotAgentAuthorityEnrollment, HotAgentAuthorityInventoryError> {
        let proof = PersistedHotAgentAuthority::load(&self.device, agent)?;
        let mut state =
            self.state
                .write()
                .map_err(|_| HotAgentAuthorityInventoryError::InventoryPoisoned {
                    agent: agent.to_string(),
                })?;
        let (inserted, incarnation) = match state.roots.get(agent) {
            Some(enrolled) if enrolled.authority_root == proof.authority_root => {
                (false, enrolled.incarnation)
            }
            Some(enrolled) => {
                return Err(HotAgentAuthorityInventoryError::AuthorityConflict {
                    agent: agent.to_string(),
                    enrolled: enrolled.authority_root.clone(),
                    requested: proof.authority_root,
                });
            }
            None => {
                state.advance_generation(agent)?;
                let incarnation = state.allocate_incarnation(agent)?;
                state.roots.insert(
                    agent.to_string(),
                    HotAgentAuthorityEntry {
                        authority_root: proof.authority_root.clone(),
                        incarnation,
                    },
                );
                (true, incarnation)
            }
        };
        Ok(HotAgentAuthorityEnrollment {
            agent: proof.agent,
            authority_root: proof.authority_root,
            inserted,
            incarnation,
        })
    }

    fn rollback_enrollment(
        &self,
        enrollment: &HotAgentAuthorityEnrollment,
    ) -> Result<(), HotAgentAuthorityInventoryError> {
        if !enrollment.inserted {
            return Ok(());
        }
        let mut state =
            self.state
                .write()
                .map_err(|_| HotAgentAuthorityInventoryError::InventoryPoisoned {
                    agent: enrollment.agent.clone(),
                })?;
        match state.roots.get(&enrollment.agent) {
            Some(current)
                if current.authority_root == enrollment.authority_root
                    && current.incarnation == enrollment.incarnation =>
            {
                state.roots.remove(&enrollment.agent);
                state.advance_generation(&enrollment.agent)?;
                Ok(())
            }
            Some(_) => Ok(()),
            None => Ok(()),
        }
    }

    fn revoke_after_durable_removal(
        &self,
        enrollment: &HotAgentAuthorityEnrollment,
    ) -> Result<(), HotAgentAuthorityInventoryError> {
        let snapshot = AgentAggregateRepository::try_load_snapshot()
            .map_err(|error| hot_agent_authority_snapshot_error(&enrollment.agent, error))?;
        if snapshot.has_registered_agent(&enrollment.agent) {
            return Err(HotAgentAuthorityInventoryError::DurableAgentStillPresent {
                agent: enrollment.agent.clone(),
            });
        }
        if snapshot.has_hosted_llm_agent_identity(&enrollment.agent) {
            return Err(HotAgentAuthorityInventoryError::IdentityStillPresent {
                agent: enrollment.agent.clone(),
            });
        }

        let mut state =
            self.state
                .write()
                .map_err(|_| HotAgentAuthorityInventoryError::InventoryPoisoned {
                    agent: enrollment.agent.clone(),
                })?;
        match state.roots.get(&enrollment.agent) {
            Some(current)
                if current.authority_root == enrollment.authority_root
                    && current.incarnation == enrollment.incarnation =>
            {
                state.roots.remove(&enrollment.agent);
                state.advance_generation(&enrollment.agent)?;
                Ok(())
            }
            Some(current) => Err(HotAgentAuthorityInventoryError::AuthorityConflict {
                agent: enrollment.agent.clone(),
                enrolled: current.authority_root.clone(),
                requested: enrollment.authority_root.clone(),
            }),
            None => Err(HotAgentAuthorityInventoryError::AuthorityNotEnrolled {
                agent: enrollment.agent.clone(),
            }),
        }
    }
}

impl CanonicalDeviceAuthority {
    fn parse(ura: String) -> Result<Self, AbilityControlPlaneError> {
        let parsed = crate::core::ura::parse_ura(&ura).map_err(|error| {
            AbilityControlPlaneError::InvalidDeviceAuthorityRoot {
                authority_root: ura.clone(),
                reason: error.to_string(),
            }
        })?;
        if parsed.kind != crate::core::ura::URAKind::Device {
            return Err(AbilityControlPlaneError::InvalidDeviceAuthorityRoot {
                authority_root: ura,
                reason: format!("expected /device/ URA, got {:?}", parsed.kind),
            });
        }
        let device_id = parsed
            .device_id()
            .ok_or_else(|| AbilityControlPlaneError::InvalidDeviceAuthorityRoot {
                authority_root: ura.clone(),
                reason: "device URA is missing device id".to_string(),
            })?
            .to_string();
        Ok(Self {
            ura,
            realm: parsed.realm,
            device_id,
        })
    }
}

#[derive(Debug, Clone)]
struct CanonicalRealmAuthority {
    ura: String,
}

impl CanonicalRealmAuthority {
    fn parse(ura: String) -> Result<Self, AbilityControlPlaneError> {
        let parsed = crate::core::ura::parse_ura(&ura).map_err(|error| {
            AbilityControlPlaneError::InvalidRealmAuthorityRoot {
                authority_root: ura.clone(),
                reason: error.to_string(),
            }
        })?;
        if parsed.kind != crate::core::ura::URAKind::Authority {
            return Err(AbilityControlPlaneError::InvalidRealmAuthorityRoot {
                authority_root: ura,
                reason: format!("expected authority URA, got {:?}", parsed.kind),
            });
        }
        Ok(Self { ura })
    }

    fn for_realm(realm: &str) -> Self {
        Self {
            ura: crate::core::ura::hub_ura(realm),
        }
    }
}

#[derive(Debug, Clone)]
enum DeviceSubordinateAuthoritySource {
    DeviceScoped,
    /// Canonical Agent authority roots resolved by daemon boot from the
    /// hosted-agent lifecycle registry. Keeping this set inside the authority
    /// context makes dynamic registration independent of ambient HOME reads
    /// while still rejecting a same-realm Agent owned by another user.
    ExplicitHostedAgentRoots(Arc<HotAgentAuthorityInventory>),
}

/// Explicit process-local authority state.
///
/// An authority root is present only when this runtime actually hosts that
/// owner plane. In particular, realm-authority mode has no Device root: callers cannot
/// accidentally materialize Device/Agent/User rows under a fabricated owner.
#[derive(Debug, Clone)]
enum AbilityAuthoritySet {
    Device {
        device: CanonicalDeviceAuthority,
        subordinate_source: DeviceSubordinateAuthoritySource,
    },
    RealmAuthority {
        authority: CanonicalRealmAuthority,
    },
    DeviceAndRealmAuthority {
        device: CanonicalDeviceAuthority,
        authority: CanonicalRealmAuthority,
        subordinate_source: DeviceSubordinateAuthoritySource,
    },
}

impl AbilityAuthoritySet {
    fn label(&self) -> &'static str {
        match self {
            Self::Device { .. } => "device",
            Self::RealmAuthority { .. } => "realm-authority",
            Self::DeviceAndRealmAuthority { .. } => "device+realm-authority",
        }
    }

    fn device(&self) -> Option<(&CanonicalDeviceAuthority, &DeviceSubordinateAuthoritySource)> {
        match self {
            Self::Device {
                device,
                subordinate_source,
            }
            | Self::DeviceAndRealmAuthority {
                device,
                subordinate_source,
                ..
            } => Some((device, subordinate_source)),
            Self::RealmAuthority { .. } => None,
        }
    }

    fn device_authority_root(&self) -> Option<&str> {
        self.device().map(|(device, _)| device.ura.as_str())
    }

    fn realm_authority(&self) -> Option<&CanonicalRealmAuthority> {
        match self {
            Self::RealmAuthority { authority }
            | Self::DeviceAndRealmAuthority { authority, .. } => Some(authority),
            Self::Device { .. } => None,
        }
    }
}

/// Process-local authorities used when projecting owner kinds into descriptor
/// records and Axon `LocalRuntime` ability keys.
///
/// The set is an explicit Device / RealmAuthority / DeviceAndRealmAuthority
/// state rather than two always-present strings. Registration can therefore
/// enforce the daemon's hosted owner planes before any control-plane or
/// runtime row is written.
#[derive(Debug, Clone)]
pub struct AbilityAuthorityContext {
    authorities: AbilityAuthoritySet,
    /// Runtime-owned Agent roots declared by static daemon capabilities during
    /// assembly. These are distinct from the persisted hosted-agent
    /// lifecycle inventory: a daemon-native executor such as Pages is
    /// deterministic from boot configuration and must not depend on ambient
    /// local-agent lookup to prove its authority root.
    declared_agent_roots: BTreeMap<String, String>,
}

impl AbilityAuthorityContext {
    pub fn from_local_environment() -> Self {
        let device = CanonicalDeviceAuthority::parse(
            local_device_authority_root().expect("local Device authority must be available"),
        )
        .expect("local Device authority helper must return a canonical Device URA");
        let roots = crate::daemon::persistence::hosted_agent_authority_roots().expect(
            "local hosted-Agent lifecycle state must be readable when building authority context",
        );
        let roots = hosted_agent_roots_for_device(&device, roots)
            .expect("local hosted-Agent lifecycle state must contain canonical authority roots");
        Self {
            authorities: AbilityAuthoritySet::Device {
                subordinate_source: DeviceSubordinateAuthoritySource::ExplicitHostedAgentRoots(
                    HotAgentAuthorityInventory::new(device.clone(), roots),
                ),
                device,
            },
            declared_agent_roots: BTreeMap::new(),
        }
    }

    pub fn for_device_authority_root(
        device_authority_root: impl Into<String>,
    ) -> Result<Self, AbilityControlPlaneError> {
        let device = CanonicalDeviceAuthority::parse(device_authority_root.into())?;
        Ok(Self {
            authorities: AbilityAuthoritySet::Device {
                device,
                subordinate_source: DeviceSubordinateAuthoritySource::DeviceScoped,
            },
            declared_agent_roots: BTreeMap::new(),
        })
    }

    /// Bind a Device runtime to a fixed device authority plus the Agent URAs
    /// it actually hosts. Daemon boot constructs this once from lifecycle
    /// state; later dynamic registrations cannot widen it with an arbitrary
    /// same-realm Agent root.
    pub fn for_device_authority_root_with_hosted_agents(
        device_authority_root: impl Into<String>,
        hosted_agent_uras: impl IntoIterator<Item = String>,
    ) -> Result<Self, AbilityControlPlaneError> {
        let device = CanonicalDeviceAuthority::parse(device_authority_root.into())?;
        let hosted_agent_roots = hosted_agent_roots_for_device(&device, hosted_agent_uras)?;
        Ok(Self {
            authorities: AbilityAuthoritySet::Device {
                subordinate_source: DeviceSubordinateAuthoritySource::ExplicitHostedAgentRoots(
                    HotAgentAuthorityInventory::new(device.clone(), hosted_agent_roots),
                ),
                device,
            },
            declared_agent_roots: BTreeMap::new(),
        })
    }

    /// Bind a combined Device + realm-authority registry. The two authority
    /// roots remain distinct even though one process hosts both runtime roles.
    pub fn for_combined_authority_roots(
        device_authority_root: impl Into<String>,
    ) -> Result<Self, AbilityControlPlaneError> {
        let device = CanonicalDeviceAuthority::parse(device_authority_root.into())?;
        let authority = CanonicalRealmAuthority::for_realm(&device.realm);
        Ok(Self {
            authorities: AbilityAuthoritySet::DeviceAndRealmAuthority {
                device,
                authority,
                subordinate_source: DeviceSubordinateAuthoritySource::DeviceScoped,
            },
            declared_agent_roots: BTreeMap::new(),
        })
    }

    /// Bind a combined Device + realm-authority runtime with the explicit
    /// hosted-Agent authority inventory captured at boot.
    pub fn for_combined_authority_roots_with_hosted_agents(
        device_authority_root: impl Into<String>,
        hosted_agent_uras: impl IntoIterator<Item = String>,
    ) -> Result<Self, AbilityControlPlaneError> {
        let device = CanonicalDeviceAuthority::parse(device_authority_root.into())?;
        let authority = CanonicalRealmAuthority::for_realm(&device.realm);
        let hosted_agent_roots = hosted_agent_roots_for_device(&device, hosted_agent_uras)?;
        Ok(Self {
            authorities: AbilityAuthoritySet::DeviceAndRealmAuthority {
                authority,
                subordinate_source: DeviceSubordinateAuthoritySource::ExplicitHostedAgentRoots(
                    HotAgentAuthorityInventory::new(device.clone(), hosted_agent_roots),
                ),
                device,
            },
            declared_agent_roots: BTreeMap::new(),
        })
    }

    /// Bind an Authority-only registry to the configured realm authority. This
    /// state intentionally has no Device authority and never consults Device
    /// credentials.
    pub fn for_realm_authority_root(
        realm_authority_root: impl Into<String>,
    ) -> Result<Self, AbilityControlPlaneError> {
        let authority = CanonicalRealmAuthority::parse(realm_authority_root.into())?;
        Ok(Self {
            authorities: AbilityAuthoritySet::RealmAuthority { authority },
            declared_agent_roots: BTreeMap::new(),
        })
    }

    /// Declare one daemon-native Agent execution root captured from explicit
    /// boot configuration. The root must belong to this Device authority's
    /// realm (and, for a device-qualified agent, this exact Device). An
    /// Authority-only runtime cannot host such an Agent root.
    pub fn with_declared_agent_authority_root(
        mut self,
        authority_root: impl Into<String>,
    ) -> Result<Self, AbilityControlPlaneError> {
        let authority_root = authority_root.into();
        let (device, _) = self.authorities.device().ok_or_else(|| {
            AbilityControlPlaneError::UnsupportedOwnerForAuthoritySet {
                owner_projection: "agent".to_string(),
                authority_set: self.authorities.label(),
            }
        })?;
        let mut roots = hosted_agent_roots_for_device(device, [authority_root.clone()])?;
        let (agent_id, authority_root) = roots
            .pop_first()
            .expect("one validated hosted agent root must produce one agent id");
        if let Some(existing) = self.declared_agent_roots.get(&agent_id) {
            if existing != &authority_root {
                return Err(AbilityControlPlaneError::InvalidAuthorityRoot { authority_root });
            }
            return Ok(self);
        }
        if let Some(existing) = self.persisted_hosted_agent_root(&agent_id) {
            if existing != authority_root {
                return Err(AbilityControlPlaneError::InvalidAuthorityRoot { authority_root });
            }
        }
        self.declare_agent_authority_root_in_signing_inventory(&agent_id, &authority_root)?;
        self.declared_agent_roots.insert(agent_id, authority_root);
        Ok(self)
    }

    fn declare_agent_authority_root_in_signing_inventory(
        &mut self,
        agent_id: &str,
        authority_root: &str,
    ) -> Result<(), AbilityControlPlaneError> {
        match &mut self.authorities {
            AbilityAuthoritySet::Device {
                device,
                subordinate_source,
            }
            | AbilityAuthoritySet::DeviceAndRealmAuthority {
                device,
                subordinate_source,
                ..
            } => match subordinate_source {
                DeviceSubordinateAuthoritySource::ExplicitHostedAgentRoots(inventory) => inventory
                    .declare_static_root(agent_id, authority_root)
                    .map_err(|_| AbilityControlPlaneError::InvalidAuthorityRoot {
                        authority_root: authority_root.to_string(),
                    }),
                DeviceSubordinateAuthoritySource::DeviceScoped => {
                    *subordinate_source =
                        DeviceSubordinateAuthoritySource::ExplicitHostedAgentRoots(
                            HotAgentAuthorityInventory::new(
                                device.clone(),
                                BTreeMap::from([(
                                    agent_id.to_string(),
                                    authority_root.to_string(),
                                )]),
                            ),
                        );
                    Ok(())
                }
            },
            AbilityAuthoritySet::RealmAuthority { .. } => {
                Err(AbilityControlPlaneError::UnsupportedOwnerForAuthoritySet {
                    owner_projection: format!("agent:{agent_id}"),
                    authority_set: self.authorities.label(),
                })
            }
        }
    }

    pub(crate) fn local_runtime_owners(&self) -> Vec<OwnerKind> {
        match &self.authorities {
            AbilityAuthoritySet::Device { .. } => vec![OwnerKind::Device],
            AbilityAuthoritySet::RealmAuthority { .. } => vec![OwnerKind::RealmAuthority],
            AbilityAuthoritySet::DeviceAndRealmAuthority { .. } => {
                vec![OwnerKind::Device, OwnerKind::RealmAuthority]
            }
        }
    }

    pub(crate) fn ledger_governance_owner(&self) -> OwnerKind {
        match &self.authorities {
            AbilityAuthoritySet::Device { .. }
            | AbilityAuthoritySet::DeviceAndRealmAuthority { .. } => OwnerKind::Device,
            AbilityAuthoritySet::RealmAuthority { .. } => OwnerKind::RealmAuthority,
        }
    }

    fn ledger_governance_authority(&self) -> LedgerGovernanceAuthority {
        let owner = self.ledger_governance_owner();
        let authority_scope = self
            .authority_scope_for(&owner)
            .expect("ledger governance owner must belong to the configured authority set");
        LedgerGovernanceAuthority {
            owner,
            runtime_owner_ura: authority_scope.authority_root().to_string(),
        }
    }

    pub(crate) fn hosts_device_authority(&self) -> bool {
        self.authorities.device().is_some()
    }

    pub(crate) fn owns_device_product_state(&self) -> bool {
        self.authorities.device().is_some_and(|(_, source)| {
            matches!(
                source,
                DeviceSubordinateAuthoritySource::ExplicitHostedAgentRoots(_)
            )
        })
    }

    pub(crate) fn hosts_realm_authority(&self) -> bool {
        self.authorities.realm_authority().is_some()
    }

    fn hot_agent_authority_inventory(&self) -> Option<Arc<HotAgentAuthorityInventory>> {
        let (_, source) = self.authorities.device()?;
        match source {
            DeviceSubordinateAuthoritySource::ExplicitHostedAgentRoots(inventory) => {
                Some(Arc::clone(inventory))
            }
            DeviceSubordinateAuthoritySource::DeviceScoped => None,
        }
    }

    pub fn hosted_agent_signing_inventory(&self) -> Option<Arc<dyn HostedAgentAuthorityInventory>> {
        self.hot_agent_authority_inventory()
            .map(|inventory| inventory as Arc<dyn HostedAgentAuthorityInventory>)
    }

    fn supports_owner(&self, owner: &OwnerKind) -> bool {
        match owner {
            OwnerKind::RealmAuthority => self.authorities.realm_authority().is_some(),
            OwnerKind::Device | OwnerKind::Agent(_) | OwnerKind::User(_) => {
                self.authorities.device().is_some()
            }
        }
    }

    fn ensure_owner_supported(&self, owner: &OwnerKind) -> Result<(), AbilityControlPlaneError> {
        if self.supports_owner(owner) {
            return Ok(());
        }
        Err(AbilityControlPlaneError::UnsupportedOwnerForAuthoritySet {
            owner_projection: owner.authority_projection(),
            authority_set: self.authorities.label(),
        })
    }

    fn ensure_explicit_scope_supported(
        &self,
        owner: &OwnerKind,
        authority_scope: &AuthorityScope,
    ) -> Result<(), AbilityControlPlaneError> {
        self.ensure_owner_supported(owner)?;
        let expected_projection = owner.authority_projection();
        if authority_scope.owner_projection() != expected_projection {
            return Err(
                AbilityControlPlaneError::AuthorityScopeOwnerProjectionMismatch {
                    expected_projection,
                    actual_projection: authority_scope.owner_projection().to_string(),
                },
            );
        }
        if !self.authority_root_is_hosted_for_owner(owner, authority_scope.authority_root()) {
            return Err(AbilityControlPlaneError::AuthorityScopeRootNotHosted {
                owner_projection: expected_projection,
                authority_root: authority_scope.authority_root().to_string(),
                authority_set: self.authorities.label(),
            });
        }
        Ok(())
    }

    fn authority_root_is_hosted_for_owner(&self, owner: &OwnerKind, authority_root: &str) -> bool {
        match owner {
            OwnerKind::Device => self
                .authorities
                .device()
                .is_some_and(|(device, _)| device.ura == authority_root),
            OwnerKind::RealmAuthority => self
                .authorities
                .realm_authority()
                .is_some_and(|authority| authority.ura == authority_root),
            OwnerKind::Agent(agent_id) => {
                self.authorities.device().is_some()
                    && authority_root == self.agent_authority_root(agent_id)
            }
            OwnerKind::User(user_id) => self.user_authority_root_is_hosted(user_id, authority_root),
        }
    }

    fn authority_scope_for(
        &self,
        owner: &OwnerKind,
    ) -> Result<AuthorityScope, AbilityControlPlaneError> {
        self.ensure_owner_supported(owner)?;
        let projection = owner.authority_projection();
        let authority_root = match owner {
            OwnerKind::Device => self
                .authorities
                .device()
                .expect("supported Device owner requires Device authority")
                .0
                .ura
                .clone(),
            OwnerKind::RealmAuthority => self
                .authorities
                .realm_authority()
                .expect("supported RealmAuthority owner requires realm authority")
                .ura
                .clone(),
            OwnerKind::Agent(agent_id) => self.agent_authority_root(agent_id),
            OwnerKind::User(user_id) => self.user_authority_root(user_id),
        };
        AuthorityScope::new(projection, authority_root)
    }

    fn agent_authority_root(&self, agent_id: &str) -> String {
        if let Some(authority_root) = self.declared_agent_roots.get(agent_id) {
            return authority_root.clone();
        }
        let (device, source) = self
            .authorities
            .device()
            .expect("Agent owner support requires Device authority");
        match source {
            DeviceSubordinateAuthoritySource::DeviceScoped => {
                crate::core::ura::device_agent_ura(&device.realm, &device.device_id, agent_id)
            }
            DeviceSubordinateAuthoritySource::ExplicitHostedAgentRoots(roots) => {
                if let Some(hosted) = roots.authority_root(agent_id) {
                    return hosted;
                }
                crate::core::ura::device_agent_ura(&device.realm, &device.device_id, agent_id)
            }
        }
    }

    fn user_authority_root(&self, user_id: &str) -> String {
        let (device, _) = self
            .authorities
            .device()
            .expect("User owner support requires Device authority");
        crate::core::ura::agent_ura(&device.realm, user_id, "account")
    }

    fn persisted_hosted_agent_root(&self, agent_id: &str) -> Option<String> {
        let (_, source) = self.authorities.device()?;
        match source {
            DeviceSubordinateAuthoritySource::ExplicitHostedAgentRoots(roots) => {
                roots.authority_root(agent_id)
            }
            DeviceSubordinateAuthoritySource::DeviceScoped => None,
        }
    }

    fn user_authority_root_is_hosted(&self, user_id: &str, authority_root: &str) -> bool {
        let Some((device, _)) = self.authorities.device() else {
            return false;
        };
        if authority_root == self.user_authority_root(user_id) {
            return true;
        }
        let Ok(parsed) = crate::core::ura::parse_ura(authority_root) else {
            return false;
        };
        let Some((host_user, agent_id)) = parsed.agent_ids() else {
            return false;
        };
        parsed.realm == device.realm
            && host_user == user_id
            && authority_root == self.agent_authority_root(agent_id)
    }
}

fn hosted_agent_roots_for_device(
    device: &CanonicalDeviceAuthority,
    hosted_agent_uras: impl IntoIterator<Item = String>,
) -> Result<BTreeMap<String, String>, AbilityControlPlaneError> {
    let mut roots = BTreeMap::new();
    for authority_root in hosted_agent_uras {
        let parsed = crate::core::ura::parse_ura(&authority_root).map_err(|_| {
            AbilityControlPlaneError::InvalidAuthorityRoot {
                authority_root: authority_root.clone(),
            }
        })?;
        if parsed.kind != crate::core::ura::URAKind::Agent || parsed.realm != device.realm {
            return Err(AbilityControlPlaneError::InvalidAuthorityRoot { authority_root });
        }
        if let Some((device_id, _)) = parsed.device_agent_ids() {
            if device_id != device.device_id {
                return Err(AbilityControlPlaneError::InvalidAuthorityRoot { authority_root });
            }
        }
        let agent_id = parsed
            .agent_ids()
            .or_else(|| parsed.device_agent_ids())
            .map(|(_, agent_id)| agent_id.to_string())
            .ok_or_else(|| AbilityControlPlaneError::InvalidAuthorityRoot {
                authority_root: authority_root.clone(),
            })?;
        if let Some(previous) = roots.insert(agent_id.clone(), authority_root.clone()) {
            if previous != authority_root {
                return Err(AbilityControlPlaneError::InvalidAuthorityRoot { authority_root });
            }
        }
    }
    Ok(roots)
}

fn local_device_authority_root() -> anyhow::Result<String> {
    crate::daemon::identity::local_invocation::local_device_ura()
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
            record.ability(),
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

/// One canonical descriptor row per committed control-plane record.
///
/// The execution `name` remains available for local handler lookup, while all
/// protocol-visible interface, owner, version, transport, receipt, access, and
/// schema facts live in `descriptor`. Rows are deliberately not collapsed by
/// public identity: if an authority publishes distinct versions or call modes,
/// callers observe each committed record instead of an arbitrary winner.
#[derive(Debug, Clone)]
pub struct AuthorityAbilityCatalogSnapshotRow {
    /// Execution registry key. Never serialize this as the public ability name.
    pub name: String,
    pub owner: OwnerKind,
    pub descriptor: AbilityDescriptor,
}

/// Failure to resolve one canonical public descriptor from the committed
/// control-plane catalogue.
///
/// Public ability identity is `(owner, public_name, call_mode)`. The execution
/// registry key is deliberately excluded: Agent-owned handlers use qualified
/// local keys such as `testbot.discover` while their signed public descriptor
/// remains `discover`.
#[derive(Debug, thiserror::Error)]
pub(crate) enum PublicDescriptorLookupError {
    #[error(
        "no public descriptor for owner {owner:?}, ability {public_name:?}, mode {call_mode:?}"
    )]
    Missing {
        owner: OwnerKind,
        public_name: String,
        call_mode: DescriptorCallMode,
    },
    #[error(
        "ambiguous public descriptor for owner {owner:?}, ability {public_name:?}, mode {call_mode:?}"
    )]
    Ambiguous {
        owner: OwnerKind,
        public_name: String,
        call_mode: DescriptorCallMode,
    },
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
pub struct AxonAbilityCatalog {
    /// Shared Axon runtime that owns the live invocation surface.
    ///
    /// During the Axon migration this catalogue remains the metadata
    /// construction API used by the existing `register(...)` modules,
    /// but handler registration is written through to `LocalRuntime`
    /// immediately when a runtime is explicitly attached. `new()` is a
    /// metadata-only catalogue; executable callers must use `new_with_runtime`
    /// with a signer-configured runtime.
    runtime: Option<Arc<LocalRuntime>>,
    /// Late-bound daemon policy capability shared by every envelope-aware
    /// runtime adapter. Catalog assembly precedes `AdmissionFacade` assembly
    /// because the facade validates against this catalog; boot closes that
    /// dependency cycle exactly once before publishing any listener.
    derived_invocation_admission: Arc<
        OnceLock<
            Arc<
                dyn crate::daemon::execution::mission::invocation_gateway::MissionChildAdmissionProvider,
            >,
        >,
    >,
    // ── Execution index (SPEC §9.1.A) ────────────────────────
    // This is a PURE EXECUTION INDEX, not a metadata store. Canonical owner /
    // authority / governed-descriptor / call-mode truth lives ONLY in `control_plane`,
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
    /// Boot-time registrations excluded by the explicit authority set,
    /// aggregated by owner projection. Daemon assembly reports one bounded
    /// summary instead of emitting one event per non-hosted ability.
    static_authority_exclusions: BTreeMap<String, usize>,
    /// Descriptor/authority/implementation binding records keyed by the
    /// typed control-plane key. This is the canonical owner / authority /
    /// interface / call-mode truth (SPEC §9.1.A); the handler maps above are
    /// an execution index only. Import manifests are normalized into the
    /// descriptor at this boundary and are never retained as a second read
    /// model.
    control_plane: std::sync::RwLock<AbilityControlPlaneRegistry>,
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
    /// Post-commit hooks for consumers that derive external read models from
    /// the committed catalog. Hooks are notifications only; they must schedule
    /// their own async work and must not mutate the catalog inline.
    dynamic_publication_hooks: std::sync::RwLock<Vec<Arc<dyn Fn() + Send + Sync>>>,
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
/// for descriptor version, schema hash, owner, and call-mode proofs.
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
            receipt_semantics: _,
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

    fn static_rows_for_ability(
        &self,
        ability: &str,
    ) -> Vec<(ControlPlaneAbilityKey, RuntimeHandlerSet)> {
        self.entries
            .iter()
            .filter(|(key, entry)| {
                key.ability() == ability
                    && entry.origin == ExecutionOrigin::Static
                    && !entry.handlers.is_empty()
            })
            .map(|(key, entry)| (key.clone(), entry.handlers.clone()))
            .collect()
    }

    fn handlers_for_key(&self, key: &ControlPlaneAbilityKey) -> RuntimeHandlerSet {
        self.entries
            .get(key)
            .map(|entry| entry.handlers.clone())
            .unwrap_or_default()
    }

    fn resolve_rpc_for_key(&self, key: &ControlPlaneAbilityKey) -> Option<LocalRpcHandler> {
        self.entries
            .get(key)
            .and_then(|entry| entry.handlers.resolve_rpc())
    }

    fn unique_handler_slot<T>(
        &self,
        ability: &str,
        extract: impl Fn(&RuntimeHandlerSet) -> Option<T>,
    ) -> Option<T> {
        let mut matches = self
            .entries
            .iter()
            .filter(|(key, entry)| key.ability() == ability && !entry.handlers.is_empty())
            .filter_map(|(_, entry)| extract(&entry.handlers));
        let first = matches.next()?;
        if matches.next().is_some() {
            return None;
        }
        Some(first)
    }

    fn unique_mode_registered(&self, ability: &str, call_mode: DescriptorCallMode) -> bool {
        let mut matches = self
            .entries
            .iter()
            .filter(|(key, entry)| key.ability() == ability && !entry.handlers.is_empty())
            .filter(|(_, entry)| {
                let modes = entry.handlers.modes();
                match call_mode {
                    DescriptorCallMode::Rpc => modes.rpc,
                    DescriptorCallMode::Stream => modes.stream,
                    DescriptorCallMode::Bidi => modes.bidi,
                }
            });
        matches.next().is_some() && matches.next().is_none()
    }

    fn has_mode(&self, ability: &str, call_mode: DescriptorCallMode) -> bool {
        self.unique_mode_registered(ability, call_mode)
    }

    fn has_any_handler(&self, ability: &str) -> bool {
        self.entries
            .iter()
            .any(|(key, entry)| key.ability() == ability && !entry.handlers.is_empty())
    }

    fn resolve_rpc(&self, ability: &str) -> Option<LocalRpcHandler> {
        self.unique_handler_slot(ability, RuntimeHandlerSet::resolve_rpc)
    }

    fn resolve_stream(&self, ability: &str) -> Option<LocalStreamHandler> {
        self.unique_handler_slot(ability, RuntimeHandlerSet::resolve_stream)
    }

    fn resolve_stream_with_env(&self, ability: &str) -> Option<LocalStreamHandlerWithEnvelope> {
        self.unique_handler_slot(ability, RuntimeHandlerSet::resolve_stream_with_env)
    }

    fn resolve_bidi(&self, ability: &str) -> Option<LocalBidiHandler> {
        self.unique_handler_slot(ability, RuntimeHandlerSet::resolve_bidi)
    }

    fn resolve_bidi_with_env(&self, ability: &str) -> Option<LocalBidiHandlerWithEnvelope> {
        self.unique_handler_slot(ability, RuntimeHandlerSet::resolve_bidi_with_env)
    }

    fn resolve_rpc_with_env(&self, ability: &str) -> Option<LocalRpcHandlerWithEnvelope> {
        self.unique_handler_slot(ability, RuntimeHandlerSet::resolve_rpc_with_env)
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

#[derive(Debug, Clone, PartialEq, Eq)]
enum StaticRegistrationOutcome {
    Registered,
    ExcludedByAuthoritySet { owner_projection: String },
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
    admission_action: Option<crate::daemon::ability::descriptors::AdmissionAction>,
    authority_scope: Option<AuthorityScope>,
    manifest: Option<Arc<crate::daemon::ability::manifest::AbilityManifest>>,
    receipt_semantics: ReceiptSemantics,
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
            admission_action: None,
            authority_scope: None,
            manifest: None,
            receipt_semantics: ReceiptSemantics::Operational,
            implementation: ControlPlaneImplementation::native_daemon(),
            handler,
        }
    }

    fn with_manifest(
        mut self,
        manifest: crate::daemon::ability::manifest::AbilityManifest,
    ) -> Self {
        self.manifest = Some(Arc::new(manifest));
        self
    }

    fn with_admission_action(
        mut self,
        admission_action: crate::daemon::ability::descriptors::AdmissionAction,
    ) -> Self {
        self.admission_action = Some(admission_action);
        self
    }

    fn with_implementation(mut self, implementation: ControlPlaneImplementation) -> Self {
        self.implementation = implementation;
        self
    }

    fn with_receipt_semantics(mut self, receipt_semantics: ReceiptSemantics) -> Self {
        self.receipt_semantics = receipt_semantics;
        self
    }

    fn with_authority_scope(mut self, authority_scope: AuthorityScope) -> Self {
        self.authority_scope = Some(authority_scope);
        self
    }

    fn commit(self, catalog: &mut AxonAbilityCatalog) -> anyhow::Result<StaticRegistrationOutcome> {
        let Self {
            ability,
            owner,
            admission_action,
            authority_scope,
            mut manifest,
            receipt_semantics,
            implementation,
            handler,
        } = self;
        if catalog
            .authority_context
            .ensure_owner_supported(&owner)
            .is_err()
        {
            return Ok(StaticRegistrationOutcome::ExcludedByAuthoritySet {
                owner_projection: owner.authority_projection(),
            });
        }
        let call_mode = handler.call_mode();
        let target_slot = handler.slot();
        if let Some(action) = admission_action {
            let internal_manifest = match manifest.take() {
                Some(manifest) => manifest.as_ref().clone(),
                None => {
                    if matches!(owner, OwnerKind::Agent(_))
                        && crate::daemon::ability::catalog::try_system_ability_descriptor_path(
                            &ability,
                        )
                        .is_err()
                    {
                        anyhow::bail!(
                            "agent-owned ability {ability:?} requires an explicit manifest; \
                             descriptor publication must not synthesize fallback metadata"
                        );
                    }
                    crate::daemon::ability::manifest::AbilityManifest::new(
                        ability.rsplit('.').next().unwrap_or(&ability),
                        crate::daemon::ability::catalog::try_description_for_owned(&ability)?,
                        crate::daemon::ability::catalog::try_input_schema_for(&ability)?,
                    )?
                }
            };
            manifest = Some(Arc::new(
                internal_manifest.with_admission_action(action.as_str())?,
            ));
        } else {
            if crate::daemon::ability::catalog::try_system_ability_descriptor_path(&ability)
                .is_err()
            {
                let provided = manifest.as_ref().ok_or_else(|| {
                    anyhow::anyhow!(
                        "non-system ability {ability:?} requires a manifest with explicit admission_action"
                    )
                })?;
                manifest_admission_action(Some(provided.as_ref()))?;
            } else {
                let contract = crate::daemon::ability::catalog::system_manifest::canonical_registration_contract(
                &ability,
            )?;
                if contract.call_mode != call_mode {
                    anyhow::bail!(
                    "static ability {ability:?} handler mode {call_mode:?} disagrees with canonical descriptor mode {:?}",
                    contract.call_mode
                );
                }
                let canonical_manifest = match manifest.take() {
                    Some(manifest) => manifest.as_ref().clone(),
                    None => {
                        crate::daemon::ability::catalog::system_manifest::registration_manifest(
                            &ability,
                        )
                        .with_context(|| {
                            format!(
                        "static ability {ability:?} failed to import canonical catalog metadata"
                    )
                        })?
                    }
                };
                manifest = Some(Arc::new(
                    canonical_manifest
                        .with_descriptor_version(contract.descriptor_version)?
                        .with_admission_action(contract.admission_action.as_str())?,
                ));
            }
        }
        let authority_scope = match authority_scope {
            Some(authority_scope) => {
                catalog
                    .authority_context
                    .ensure_explicit_scope_supported(&owner, &authority_scope)
                    .map_err(|error| {
                        anyhow::anyhow!(
                            "static ability {ability:?} explicit authority scope rejected: {error}"
                        )
                    })?;
                authority_scope
            }
            None => catalog.resolve_authority_scope_for_owner(&ability, &owner)?,
        };
        let execution_key = ControlPlaneAbilityKey::new(authority_scope.authority_root(), &ability);
        catalog.assert_static_handler_slot_available(&execution_key, target_slot);
        catalog.register_control_plane_with_scope_and_semantics_result(
            &ability,
            authority_scope.clone(),
            manifest.as_ref().map(Arc::as_ref),
            call_mode,
            receipt_semantics,
            &implementation,
        )?;
        catalog
            .execution_index
            .write()
            .expect("execution_index RwLock poisoned")
            .install_static(execution_key, handler);
        catalog.sync_static_runtime_ability_or_panic(&ability);
        Ok(StaticRegistrationOutcome::Registered)
    }
}

struct DynamicRegistration {
    ability: String,
    owner: OwnerKind,
    authority_scope: Option<AuthorityScope>,
    manifest: Option<Arc<crate::daemon::ability::manifest::AbilityManifest>>,
    receipt_semantics: ReceiptSemantics,
    implementation: ControlPlaneImplementation,
    handler: DynamicRegistrationHandler,
}

impl DynamicRegistration {
    fn new(
        ability: impl Into<String>,
        owner: OwnerKind,
        manifest: Option<Arc<crate::daemon::ability::manifest::AbilityManifest>>,
        implementation: ControlPlaneImplementation,
        handler: DynamicRegistrationHandler,
    ) -> Self {
        Self {
            ability: ability.into(),
            owner,
            authority_scope: None,
            manifest,
            receipt_semantics: ReceiptSemantics::Operational,
            implementation,
            handler,
        }
    }

    fn rpc_with_spec(
        ability: impl Into<String>,
        owner: OwnerKind,
        manifest: crate::daemon::ability::manifest::AbilityManifest,
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
        manifest: crate::daemon::ability::manifest::AbilityManifest,
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

    fn rpc_with_envelope_and_spec_and_impl(
        ability: impl Into<String>,
        owner: OwnerKind,
        manifest: crate::daemon::ability::manifest::AbilityManifest,
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

    fn stream_with_envelope_and_spec_and_impl(
        ability: impl Into<String>,
        owner: OwnerKind,
        manifest: crate::daemon::ability::manifest::AbilityManifest,
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

    fn bidi_with_envelope_and_spec_and_impl(
        ability: impl Into<String>,
        owner: OwnerKind,
        manifest: crate::daemon::ability::manifest::AbilityManifest,
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

    fn manifest_ref(&self) -> Option<&crate::daemon::ability::manifest::AbilityManifest> {
        self.manifest.as_ref().map(Arc::as_ref)
    }

    fn commit(self, catalog: &AxonAbilityCatalog) -> anyhow::Result<()> {
        let ability = self.ability().to_string();
        let call_mode = self.call_mode();
        catalog
            .authority_context
            .ensure_owner_supported(&self.owner)
            .map_err(|error| {
                anyhow::anyhow!(
                    "dynamic ability {ability:?} owner authority is not hosted by this runtime: {error}"
                )
            })?;
        let _dynamic_txn_guard = catalog
            .dynamic_txn
            .lock()
            .expect("dynamic_txn mutex poisoned");
        if catalog.reject_dynamic_shadow_of_static(&ability) {
            anyhow::bail!("dynamic ability {ability:?} shadows a static ability");
        }
        let authority_scope = match self.authority_scope.clone() {
            Some(authority_scope) => {
                catalog
                    .authority_context
                    .ensure_explicit_scope_supported(&self.owner, &authority_scope)
                    .map_err(|error| {
                        anyhow::anyhow!(
                            "dynamic ability {ability:?} explicit authority scope rejected: {error}"
                        )
                    })?;
                authority_scope
            }
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
        let control_plane_key = catalog
            .register_dynamic_control_plane_with_scope_and_semantics_result(
                &ability,
                authority_scope.clone(),
                self.manifest_ref(),
                call_mode,
                self.receipt_semantics.clone(),
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
        let commit_result = txn.sync_runtime_or_rollback();
        drop(_dynamic_txn_guard);
        commit_result?;
        catalog.notify_dynamic_publication_hooks();
        Ok(())
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
        Self {
            catalog,
            authority_root: authority_root.to_string(),
            ability: ability.to_string(),
            call_mode,
            prior_records: Some(prior_records),
            phase: ControlPlaneAuthorityModeTxnPhase::Active,
        }
    }

    pub fn commit(mut self) {
        self.prior_records = None;
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
            .field(
                "derived_invocation_admission_bound",
                &self.derived_invocation_admission.get().is_some(),
            )
            .finish()
    }
}

impl AxonAbilityCatalog {
    pub fn new_metadata_only_with_authority_context(
        authority_context: AbilityAuthorityContext,
    ) -> Self {
        Self {
            runtime: None,
            derived_invocation_admission: Arc::new(OnceLock::new()),
            execution_index: std::sync::RwLock::new(ExecutionIndex::default()),
            authority_context,
            static_authority_exclusions: BTreeMap::new(),
            control_plane: std::sync::RwLock::new(AbilityControlPlaneRegistry::default()),
            dynamic_txn: std::sync::Mutex::new(()),
            dynamic_publication_hooks: std::sync::RwLock::new(Vec::new()),
        }
    }

    #[cfg(test)]
    pub(crate) fn new_test_metadata_for_device_authority(device_ura: &str) -> Self {
        Self::new_metadata_only_with_authority_context(
            AbilityAuthorityContext::for_device_authority_root(device_ura)
                .expect("explicit test Device authority root must be canonical"),
        )
    }

    #[cfg(test)]
    pub(crate) fn new_test_runtime_for_device_authority(
        runtime: Arc<LocalRuntime>,
        device_ura: &str,
    ) -> Self {
        Self::new_with_runtime_and_authority_context(
            runtime,
            AbilityAuthorityContext::for_device_authority_root(device_ura)
                .expect("explicit test Device authority root must be canonical"),
        )
    }

    /// Build a registry whose registration APIs write through to
    /// the daemon-hosted Axon runtime. This keeps the existing
    /// module-level `register(&mut reg)` call sites intact while
    /// making `LocalRuntime` the live source of truth.
    pub fn new_with_runtime(runtime: Arc<LocalRuntime>) -> Self {
        #[cfg(test)]
        {
            Self::new_with_runtime_and_authority_context(
                runtime,
                AbilityAuthorityContext::for_combined_authority_roots(
                    local_device_authority_root().expect("test Device authority must be available"),
                )
                .expect("test Device authority root must be canonical"),
            )
        }
        #[cfg(not(test))]
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
            derived_invocation_admission: Arc::new(OnceLock::new()),
            execution_index: std::sync::RwLock::new(ExecutionIndex::default()),
            authority_context,
            static_authority_exclusions: BTreeMap::new(),
            control_plane: std::sync::RwLock::new(AbilityControlPlaneRegistry::default()),
            dynamic_txn: std::sync::Mutex::new(()),
            dynamic_publication_hooks: std::sync::RwLock::new(Vec::new()),
        }
    }

    /// Register a notification hook fired after a dynamic ability transaction
    /// has committed to the control plane, execution index, and LocalRuntime.
    /// The hook observes only stable catalog state. It is intentionally a
    /// no-argument signal so product-specific publication stays in boot wiring,
    /// not in the canonical catalog abstraction.
    pub(crate) fn register_dynamic_publication_hook(&self, hook: Arc<dyn Fn() + Send + Sync>) {
        self.dynamic_publication_hooks
            .write()
            .expect("dynamic_publication_hooks RwLock poisoned")
            .push(hook);
    }

    pub(crate) fn notify_dynamic_publication_hooks(&self) {
        let hooks = self
            .dynamic_publication_hooks
            .read()
            .expect("dynamic_publication_hooks RwLock poisoned")
            .clone();
        for hook in hooks {
            hook();
        }
    }

    /// Return the attached Axon runtime, if this registry was
    /// constructed for daemon boot.
    pub fn runtime(&self) -> Option<Arc<LocalRuntime>> {
        self.runtime.as_ref().map(Arc::clone)
    }

    /// Close the daemon catalog/admission construction cycle before any
    /// invocation can enter this runtime.
    pub(crate) fn bind_derived_invocation_admission(
        &self,
        admission: Arc<
            dyn crate::daemon::execution::mission::invocation_gateway::MissionChildAdmissionProvider,
        >,
    ) -> anyhow::Result<()> {
        self.derived_invocation_admission
            .set(admission)
            .map_err(|_| {
                anyhow::anyhow!(
                    "daemon derived Invocation admission capability already has an owner"
                )
            })
    }

    pub(crate) fn authority_set_label(&self) -> &'static str {
        self.authority_context.authorities.label()
    }

    /// Admit one post-boot hosted-Agent authority from validated lifecycle
    /// state. The only input is the durable agent name; the authority root is
    /// resolved and verified inside the inventory boundary.
    pub(crate) fn enroll_persisted_hot_agent_authority(
        &self,
        agent: &str,
    ) -> Result<HotAgentAuthorityEnrollment, HotAgentAuthorityInventoryError> {
        self.authority_context
            .hot_agent_authority_inventory()
            .ok_or(HotAgentAuthorityInventoryError::UnsupportedAuthorityContext)?
            .enroll_persisted(agent)
    }

    /// Undo only an enrollment created by the supplied receipt. Existing boot
    /// inventory rows are never removed by a failed replacement attempt.
    pub(crate) fn rollback_hot_agent_authority_enrollment(
        &self,
        enrollment: &HotAgentAuthorityEnrollment,
    ) -> Result<(), HotAgentAuthorityInventoryError> {
        self.authority_context
            .hot_agent_authority_inventory()
            .ok_or(HotAgentAuthorityInventoryError::UnsupportedAuthorityContext)?
            .rollback_enrollment(enrollment)
    }

    /// Revoke a hosted-Agent authority only after both lifecycle stores prove
    /// removal. The opaque enrollment receipt prevents caller-supplied URAs
    /// from deleting unrelated authority roots.
    pub(crate) fn revoke_removed_hot_agent_authority(
        &self,
        enrollment: &HotAgentAuthorityEnrollment,
    ) -> Result<(), HotAgentAuthorityInventoryError> {
        self.authority_context
            .hot_agent_authority_inventory()
            .ok_or(HotAgentAuthorityInventoryError::UnsupportedAuthorityContext)?
            .revoke_after_durable_removal(enrollment)
    }

    #[cfg(test)]
    pub(crate) fn enrolled_hot_agent_authority_root(&self, agent: &str) -> Option<String> {
        self.authority_context
            .hot_agent_authority_inventory()?
            .authority_root(agent)
    }

    pub(crate) fn hosted_device_authority_root(&self) -> Option<&str> {
        self.authority_context.authorities.device_authority_root()
    }

    pub(crate) fn static_authority_exclusion_snapshot(&self) -> BTreeMap<String, usize> {
        self.static_authority_exclusions.clone()
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
            admission_action: request.admission_action,
            receipt_semantics: request.receipt_semantics,
            implementation: request.implementation,
            owner_label,
        })
    }

    fn write_control_plane_record(
        &self,
        request: ResolvedControlPlaneRegistration<'_>,
    ) -> anyhow::Result<AbilityControlPlaneRecord> {
        let authority_root = request.authority_scope.authority_root().to_string();
        let mut registration = AbilityControlPlaneRegistration::new(
            request.ability.to_string(),
            request.call_mode,
            request.admission_action,
            request.manifest,
            request.authority_scope,
            request.implementation.runtime_env,
            request.implementation.impl_source,
        )
        .with_receipt_semantics(request.receipt_semantics)
        .with_descriptor_hints(crate::daemon::ability::catalog::registration_hints(
            &authority_root,
            request.ability,
        ));
        if let Some(impl_content_hash) = request.implementation.impl_content_hash {
            registration = registration.with_impl_content_hash(impl_content_hash);
        }
        let result = self
            .control_plane
            .write()
            .expect("control_plane RwLock poisoned")
            .register_registration(registration);
        match result {
            Ok(record) => Ok(record),
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
        match registration.commit(self)? {
            StaticRegistrationOutcome::Registered => {}
            StaticRegistrationOutcome::ExcludedByAuthoritySet { owner_projection } => {
                *self
                    .static_authority_exclusions
                    .entry(owner_projection)
                    .or_default() += 1;
            }
        }
        Ok(())
    }

    fn register_static_or_panic(&mut self, registration: StaticRegistration) {
        let ability = registration.ability.clone();
        self.register_static(registration)
            .unwrap_or_else(|error| panic!("static registration failed for {ability:?}: {error}"));
    }

    fn register_control_plane_with_scope_and_semantics_result(
        &self,
        ability: &str,
        authority_scope: AuthorityScope,
        manifest: Option<&crate::daemon::ability::manifest::AbilityManifest>,
        call_mode: DescriptorCallMode,
        receipt_semantics: ReceiptSemantics,
        implementation: &ControlPlaneImplementation,
    ) -> anyhow::Result<ControlPlaneModeKey> {
        let admission_action = manifest_admission_action(manifest)?;
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
            admission_action,
            receipt_semantics,
            implementation: implementation.clone(),
            owner_label,
        })?;
        Ok(ControlPlaneModeKey::from_record(&record))
    }

    /// Register a governed descriptor/authority/implementation record without
    /// installing an execution handler in this catalog.
    ///
    /// Daemon Invocation exact routes are served by
    /// `DaemonRouteRuntimeAdapter`, not by `AxonAbilityCatalog`'s local handler
    /// index. They still need the same control-plane proof facts as local
    /// abilities so descriptor-bound LocalRuntime registration and callers that
    /// derive signed descriptor refs resolve one canonical contract.
    pub(crate) fn register_control_plane_descriptor_with_owner(
        &self,
        ability: &str,
        owner: &OwnerKind,
        manifest: &crate::daemon::ability::manifest::AbilityManifest,
        call_mode: DescriptorCallMode,
        receipt_semantics: ReceiptSemantics,
        implementation: &ControlPlaneImplementation,
    ) -> anyhow::Result<()> {
        self.authority_context
            .ensure_owner_supported(owner)
            .map_err(|error| {
                anyhow::anyhow!(
                    "control-plane-only ability {ability:?} owner is not hosted by this authority set: {error}"
                )
            })?;
        let authority_scope = self.resolve_authority_scope_for_owner(ability, owner)?;
        self.register_control_plane_with_scope_and_semantics_result(
            ability,
            authority_scope,
            Some(manifest),
            call_mode,
            receipt_semantics,
            implementation,
        )
        .map(|_| ())
    }

    fn register_dynamic_control_plane_with_scope_and_semantics_result(
        &self,
        ability: &str,
        authority_scope: AuthorityScope,
        manifest: Option<&crate::daemon::ability::manifest::AbilityManifest>,
        call_mode: DescriptorCallMode,
        receipt_semantics: ReceiptSemantics,
        implementation: &ControlPlaneImplementation,
    ) -> anyhow::Result<ControlPlaneModeKey> {
        match self.register_control_plane_with_scope_and_semantics_result(
            ability,
            authority_scope,
            manifest,
            call_mode,
            receipt_semantics,
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

    /// Resolve one canonical governed descriptor by execution registry name.
    ///
    /// Callers without a concrete transport may use this only when the name is
    /// provably single-mode/single-version/single-authority. Ambiguity is an
    /// error rather than an arbitrary descriptor choice.
    pub fn canonical_descriptor_for_ability(
        &self,
        ability: &str,
    ) -> Result<Option<AbilityDescriptor>, AbilityControlPlaneLookupError> {
        self.control_plane
            .read()
            .expect("control_plane RwLock poisoned")
            .get(ability)
            .map(|record| record.map(|record| record.descriptor().clone()))
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
            .map(|record| record.descriptor().version.clone()))
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
        manifest: Option<&crate::daemon::ability::manifest::AbilityManifest>,
        call_mode: DescriptorCallMode,
        impl_source: AbilityImplSource,
        runtime_env: RuntimeEnv,
    ) -> anyhow::Result<()> {
        self.register_control_plane(ControlPlaneRegistrationRequest {
            ability,
            owner,
            manifest,
            call_mode,
            admission_action: manifest_admission_action(manifest)?,
            receipt_semantics: ReceiptSemantics::Operational,
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
            admission_action: manifest_admission_action(request.manifest)?,
            receipt_semantics: ReceiptSemantics::Operational,
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
        self.control_plane
            .write()
            .expect("control_plane RwLock poisoned")
            .remove_for_authority(authority_root, ability)
    }

    fn remove_control_plane_for_authority_mode(
        &self,
        authority_root: &str,
        ability: &str,
        call_mode: DescriptorCallMode,
    ) -> bool {
        self.control_plane
            .write()
            .expect("control_plane RwLock poisoned")
            .remove_for_authority_mode(authority_root, ability, call_mode)
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
        let descriptor_version = descriptor.version.clone();
        let runtime_env = implementation_record.runtime_env().label().to_string();
        let descriptor_hash = Some(descriptor.descriptor_hash_prefixed());
        RuntimeBindingFacts {
            descriptor_version,
            call_mode: descriptor.call_mode(),
            schema_hash: descriptor.schema_hash_prefixed(),
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

    /// Prove that an authority-scoped execution key is backed by exact
    /// control-plane records for every installed handler mode.
    ///
    /// This is intentionally not an ability-name lookup. The canonical owner /
    /// authority / descriptor truth is keyed by `(authority_root, ability,
    /// call_mode)`, so runtime key derivation must validate the same tuple that
    /// will be registered in `LocalRuntime`. Same-name records under another
    /// authority are unrelated facts and must not rescue this execution row.
    fn verify_execution_key_control_plane_modes(
        &self,
        origin: ExecutionOrigin,
        key: &ControlPlaneAbilityKey,
        handlers: &RuntimeHandlerSet,
    ) -> anyhow::Result<()> {
        if handlers.is_empty() {
            anyhow::bail!(
                "{origin:?} ability {:?} under authority {:?} has no execution handlers",
                key.ability(),
                key.authority_root()
            );
        }
        for slot in handlers.slots() {
            let call_mode = slot.call_mode();
            if self
                .control_plane_record_for_authority_mode(
                    key.authority_root(),
                    key.ability(),
                    call_mode,
                )?
                .is_none()
            {
                anyhow::bail!(
                    "{origin:?} ability {:?} under authority {:?} has {} handler state \
                     but no exact control-plane record for {:?}",
                    key.ability(),
                    key.authority_root(),
                    slot.label(),
                    call_mode
                );
            }
        }
        Ok(())
    }

    fn static_control_plane_key(
        &self,
        ability: &str,
    ) -> anyhow::Result<Option<ControlPlaneAbilityKey>> {
        let (execution_key, handlers) = {
            let execution_index = self
                .execution_index
                .read()
                .expect("execution_index RwLock poisoned");
            let execution_key =
                execution_index.origin_key_by_ability(ability, ExecutionOrigin::Static)?;
            let handlers = execution_key
                .as_ref()
                .map(|key| execution_index.handlers_for_key(key))
                .unwrap_or_default();
            (execution_key, handlers)
        };
        if let Some(key) = execution_key {
            self.verify_execution_key_control_plane_modes(
                ExecutionOrigin::Static,
                &key,
                &handlers,
            )?;
            return Ok(Some(key));
        }
        if self.has_static_handler(ability) {
            anyhow::bail!(
                "static ability {ability:?} has handlers but no exact control-plane authority/mode record"
            );
        }
        Ok(None)
    }

    fn dynamic_control_plane_key(
        &self,
        ability: &str,
    ) -> anyhow::Result<Option<ControlPlaneAbilityKey>> {
        let (execution_key, handlers) = {
            let execution_index = self
                .execution_index
                .read()
                .expect("execution_index RwLock poisoned");
            let execution_key =
                execution_index.origin_key_by_ability(ability, ExecutionOrigin::Dynamic)?;
            let handlers = execution_key
                .as_ref()
                .map(|key| execution_index.handlers_for_key(key))
                .unwrap_or_default();
            (execution_key, handlers)
        };
        if let Some(key) = execution_key {
            self.verify_execution_key_control_plane_modes(
                ExecutionOrigin::Dynamic,
                &key,
                &handlers,
            )?;
            return Ok(Some(key));
        }
        if self.has_dynamic(ability) {
            anyhow::bail!(
                "dynamic ability {ability:?} has handlers but no exact control-plane authority/mode record"
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
                "AxonAbilityCatalog has no LocalRuntime attached; use new_with_runtime() with an explicit signing authority"
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

    fn runtime_handlers_for_key(&self, key: &ControlPlaneAbilityKey) -> RuntimeHandlerSet {
        self.execution_index
            .read()
            .expect("execution_index RwLock poisoned")
            .handlers_for_key(key)
    }

    fn sync_runtime_ability(&self, name: &str) -> anyhow::Result<()> {
        let control_plane_key = self.handler_control_plane_key(name)?;
        let handlers = self.runtime_handlers_for_key(&control_plane_key);
        self.sync_runtime_ability_from_handlers(name, &control_plane_key, handlers)
    }

    fn sync_static_runtime_ability_or_panic(&self, name: &str) {
        self.sync_static_runtime_abilities(name)
            .unwrap_or_else(|error| {
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
        if let Err(error) = self.sync_static_runtime_abilities(ability) {
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

    fn sync_static_runtime_abilities(&self, name: &str) -> anyhow::Result<()> {
        let rows = self
            .execution_index
            .read()
            .expect("execution_index RwLock poisoned")
            .static_rows_for_ability(name);
        if rows.is_empty() {
            return self.unregister_runtime_ability(name);
        }
        for (control_plane_key, handlers) in rows {
            self.sync_runtime_ability_from_handlers(name, &control_plane_key, handlers)?;
        }
        Ok(())
    }

    fn sync_runtime_ability_from_handlers(
        &self,
        name: &str,
        control_plane_key: &ControlPlaneAbilityKey,
        handlers: RuntimeHandlerSet,
    ) -> anyhow::Result<()> {
        let modes = handlers.modes();
        if modes.is_empty() {
            let Some(runtime) = self.runtime.as_ref() else {
                return Ok(());
            };
            let runtime_key = control_plane_key.runtime_key()?;
            return self.unregister_runtime_ability_by_key(runtime, name, &runtime_key);
        }
        let options = self.runtime_options_for(control_plane_key, modes)?;
        let Some(runtime) = self.runtime.as_ref().cloned() else {
            return Ok(());
        };
        self.replace_runtime_ability(
            control_plane_key,
            runtime_handler_set_to_ability_fn(
                name.to_string(),
                handlers,
                runtime,
                Arc::clone(&self.derived_invocation_admission),
            ),
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
            record.descriptor().version.as_str(),
            record.descriptor().admission_action().as_str(),
            record.descriptor().descriptor_hash_bytes(),
            record.descriptor().schema_hash_bytes(),
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

    /// Register an RPC handler under `ability` with an explicit owner.
    ///
    /// This is the static registration facade for descriptor-governed local
    /// handlers that do not need an imported manifest. Ownership is declared by
    /// the caller and is written into the control plane through
    /// [`StaticRegistration`]; there is no owner-inference fallback at this
    /// boundary.
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

    /// Register an intentionally non-canonical local route. Public/canonical
    /// handlers must use the descriptor-importing registration methods; this
    /// variant exists for local front doors that are deliberately absent from
    /// descriptor publication.
    pub fn register_rpc_with_owner_and_action(
        &mut self,
        ability: impl Into<String>,
        owner: OwnerKind,
        action: crate::daemon::ability::descriptors::AdmissionAction,
        handler: LocalRpcHandler,
    ) {
        self.register_static_or_panic(
            StaticRegistration::new(ability, owner, StaticRegistrationHandler::Rpc(handler))
                .with_admission_action(action),
        );
    }

    /// Register an RPC handler with explicit owner AND a manifest
    /// carrying the verb's description / input_schema /
    /// output_schema. The manifest flows to
    /// `meta.list_abilities` and ultimately to the Frontend
    /// `InvokeAbilityDialog`, which renders a SchemaForm when an
    /// input schema is present and a free-text JSON box otherwise.
    /// Use this variant for any ability that already has a static
    /// import manifest in `daemon::ability::manifest` (the chat ability, the
    /// pages family, …); the registry then becomes the single
    /// source of truth for "does this verb have a schema" and
    /// downstream consumers stop having to know which import DTO
    /// constructor to call by hand.
    pub fn register_rpc_with_spec(
        &mut self,
        ability: impl Into<String>,
        owner: OwnerKind,
        manifest: crate::daemon::ability::manifest::AbilityManifest,
        handler: LocalRpcHandler,
    ) {
        self.register_rpc_with_spec_and_semantics(
            ability,
            owner,
            manifest,
            ReceiptSemantics::Operational,
            handler,
        );
    }

    pub fn register_rpc_with_spec_and_action(
        &mut self,
        ability: impl Into<String>,
        owner: OwnerKind,
        action: crate::daemon::ability::descriptors::AdmissionAction,
        manifest: crate::daemon::ability::manifest::AbilityManifest,
        handler: LocalRpcHandler,
    ) {
        self.register_static_or_panic(
            StaticRegistration::new(ability, owner, StaticRegistrationHandler::Rpc(handler))
                .with_admission_action(action)
                .with_manifest(manifest),
        );
    }

    pub fn register_rpc_with_spec_and_semantics(
        &mut self,
        ability: impl Into<String>,
        owner: OwnerKind,
        manifest: crate::daemon::ability::manifest::AbilityManifest,
        receipt_semantics: ReceiptSemantics,
        handler: LocalRpcHandler,
    ) {
        self.register_static_or_panic(
            StaticRegistration::new(ability, owner, StaticRegistrationHandler::Rpc(handler))
                .with_manifest(manifest)
                .with_receipt_semantics(receipt_semantics),
        );
    }

    pub fn register_rpc_with_spec_impl_and_authority_scope(
        &mut self,
        ability: impl Into<String>,
        owner: OwnerKind,
        authority_scope: AuthorityScope,
        manifest: crate::daemon::ability::manifest::AbilityManifest,
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
        manifest: crate::daemon::ability::manifest::AbilityManifest,
        handler: LocalStreamHandler,
    ) {
        self.register_stream_with_spec_and_semantics(
            ability,
            owner,
            manifest,
            ReceiptSemantics::Operational,
            handler,
        );
    }

    pub fn register_stream_with_spec_and_semantics(
        &mut self,
        ability: impl Into<String>,
        owner: OwnerKind,
        manifest: crate::daemon::ability::manifest::AbilityManifest,
        receipt_semantics: ReceiptSemantics,
        handler: LocalStreamHandler,
    ) {
        self.register_static_or_panic(
            StaticRegistration::new(ability, owner, StaticRegistrationHandler::Stream(handler))
                .with_manifest(manifest)
                .with_receipt_semantics(receipt_semantics),
        );
    }

    pub fn register_stream_with_spec_and_impl(
        &mut self,
        ability: impl Into<String>,
        owner: OwnerKind,
        manifest: crate::daemon::ability::manifest::AbilityManifest,
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
        manifest: crate::daemon::ability::manifest::AbilityManifest,
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

    /// Register a stream handler under `ability` with an explicit owner.
    /// See [`register_rpc_with_owner`] for the static registration contract.
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

    /// Register a bidi handler under `ability` with an explicit owner.
    /// See [`register_rpc_with_owner`] for the static registration contract.
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
        manifest: crate::daemon::ability::manifest::AbilityManifest,
        handler: LocalBidiHandler,
    ) {
        self.register_bidi_with_spec_and_semantics(
            ability,
            owner,
            manifest,
            ReceiptSemantics::Operational,
            handler,
        );
    }

    pub fn register_bidi_with_spec_and_semantics(
        &mut self,
        ability: impl Into<String>,
        owner: OwnerKind,
        manifest: crate::daemon::ability::manifest::AbilityManifest,
        receipt_semantics: ReceiptSemantics,
        handler: LocalBidiHandler,
    ) {
        self.register_static_or_panic(
            StaticRegistration::new(ability, owner, StaticRegistrationHandler::Bidi(handler))
                .with_manifest(manifest)
                .with_receipt_semantics(receipt_semantics),
        );
    }

    /// Register an envelope-aware RPC handler under `ability` with an explicit
    /// owner. See [`register_rpc_with_owner`] for the static registration
    /// contract.
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
    /// handler index and descriptor projection separately.
    pub fn register_rpc_with_envelope_and_spec(
        &mut self,
        ability: impl Into<String>,
        owner: OwnerKind,
        manifest: crate::daemon::ability::manifest::AbilityManifest,
        handler: LocalRpcHandlerWithEnvelope,
    ) {
        self.register_rpc_with_envelope_and_spec_and_semantics(
            ability,
            owner,
            manifest,
            ReceiptSemantics::Operational,
            handler,
        );
    }

    pub fn register_rpc_with_envelope_spec_and_action(
        &mut self,
        ability: impl Into<String>,
        owner: OwnerKind,
        action: crate::daemon::ability::descriptors::AdmissionAction,
        manifest: crate::daemon::ability::manifest::AbilityManifest,
        handler: LocalRpcHandlerWithEnvelope,
    ) {
        self.register_static_or_panic(
            StaticRegistration::new(
                ability,
                owner,
                StaticRegistrationHandler::RpcWithEnvelope(handler),
            )
            .with_admission_action(action)
            .with_manifest(manifest),
        );
    }

    pub fn register_rpc_with_envelope_and_spec_and_semantics(
        &mut self,
        ability: impl Into<String>,
        owner: OwnerKind,
        manifest: crate::daemon::ability::manifest::AbilityManifest,
        receipt_semantics: ReceiptSemantics,
        handler: LocalRpcHandlerWithEnvelope,
    ) {
        self.register_static_or_panic(
            StaticRegistration::new(
                ability,
                owner,
                StaticRegistrationHandler::RpcWithEnvelope(handler),
            )
            .with_manifest(manifest)
            .with_receipt_semantics(receipt_semantics),
        );
    }

    pub fn register_rpc_with_envelope_and_spec_and_impl(
        &mut self,
        ability: impl Into<String>,
        owner: OwnerKind,
        manifest: crate::daemon::ability::manifest::AbilityManifest,
        handler: LocalRpcHandlerWithEnvelope,
        implementation: ControlPlaneImplementation,
    ) -> anyhow::Result<()> {
        self.register_static(
            StaticRegistration::new(
                ability,
                owner,
                StaticRegistrationHandler::RpcWithEnvelope(handler),
            )
            .with_manifest(manifest)
            .with_implementation(implementation),
        )
    }

    /// Register an envelope-aware stream handler under `ability` with an
    /// explicit owner. See [`register_rpc_with_owner`] for the static
    /// registration contract.
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
        manifest: crate::daemon::ability::manifest::AbilityManifest,
        handler: LocalStreamHandlerWithEnvelope,
    ) {
        self.register_stream_with_envelope_and_spec_and_semantics(
            ability,
            owner,
            manifest,
            ReceiptSemantics::Operational,
            handler,
        );
    }

    pub fn register_stream_with_envelope_and_spec_and_semantics(
        &mut self,
        ability: impl Into<String>,
        owner: OwnerKind,
        manifest: crate::daemon::ability::manifest::AbilityManifest,
        receipt_semantics: ReceiptSemantics,
        handler: LocalStreamHandlerWithEnvelope,
    ) {
        self.register_static_or_panic(
            StaticRegistration::new(
                ability,
                owner,
                StaticRegistrationHandler::StreamWithEnvelope(handler),
            )
            .with_manifest(manifest)
            .with_receipt_semantics(receipt_semantics),
        );
    }

    pub fn register_stream_with_envelope_and_spec_and_impl(
        &mut self,
        ability: impl Into<String>,
        owner: OwnerKind,
        manifest: crate::daemon::ability::manifest::AbilityManifest,
        handler: LocalStreamHandlerWithEnvelope,
        implementation: ControlPlaneImplementation,
    ) -> anyhow::Result<()> {
        self.register_static(
            StaticRegistration::new(
                ability,
                owner,
                StaticRegistrationHandler::StreamWithEnvelope(handler),
            )
            .with_manifest(manifest)
            .with_implementation(implementation),
        )
    }

    /// Register an envelope-aware bidi handler under `ability` with an explicit
    /// owner. See [`register_rpc_with_owner`] for the static registration
    /// contract.
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
        manifest: crate::daemon::ability::manifest::AbilityManifest,
        handler: LocalBidiHandlerWithEnvelope,
    ) {
        self.register_bidi_with_envelope_and_spec_and_semantics(
            ability,
            owner,
            manifest,
            ReceiptSemantics::Operational,
            handler,
        );
    }

    pub fn register_bidi_with_envelope_and_spec_and_semantics(
        &mut self,
        ability: impl Into<String>,
        owner: OwnerKind,
        manifest: crate::daemon::ability::manifest::AbilityManifest,
        receipt_semantics: ReceiptSemantics,
        handler: LocalBidiHandlerWithEnvelope,
    ) {
        self.register_static_or_panic(
            StaticRegistration::new(
                ability,
                owner,
                StaticRegistrationHandler::BidiWithEnvelope(handler),
            )
            .with_manifest(manifest)
            .with_receipt_semantics(receipt_semantics),
        );
    }

    pub fn register_bidi_with_envelope_and_spec_and_impl(
        &mut self,
        ability: impl Into<String>,
        owner: OwnerKind,
        manifest: crate::daemon::ability::manifest::AbilityManifest,
        handler: LocalBidiHandlerWithEnvelope,
        implementation: ControlPlaneImplementation,
    ) -> anyhow::Result<()> {
        self.register_static(
            StaticRegistration::new(
                ability,
                owner,
                StaticRegistrationHandler::BidiWithEnvelope(handler),
            )
            .with_manifest(manifest)
            .with_implementation(implementation),
        )
    }

    /// Control-plane read-through for an ability's `OwnerKind`, derived from
    /// the canonical control-plane record rather than the execution index
    /// (SPEC §9.1.A: handler maps are only executable bindings;
    /// owner/authority/manifest truth lives in the control-plane registry).
    ///
    /// The `OwnerKind` is reconstructed from the record's authority
    /// `owner_projection` — the exact string `OwnerKind::authority_scope`
    /// wrote at registration (`device` / `hub` / `agent:<id>` /
    /// `user:<id>`). This is the precise inverse of the registration
    /// mapping: no Ability-URA round-trip and no owner-class policy (the
    /// MCP reflective path's System-Agent rejection does not apply to
    /// general ownership). A missing record yields `None` rather than guessing
    /// from the ability name or execution handlers.
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
    /// authority-keyed execution row and control-plane records. Returns
    /// `true` if the ability was present, `false` if
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
    // reads come from the control-plane registry's canonical descriptor, not
    // from dynamic metadata snapshots.

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
        manifest: crate::daemon::ability::manifest::AbilityManifest,
        handler: LocalRpcHandler,
    ) -> anyhow::Result<()> {
        DynamicRegistration::rpc_with_spec(ability, owner, manifest, handler).commit(self)
    }

    /// Hot-register an RPC handler under an already-resolved authority root.
    ///
    /// Hosted runtime owners must use this surface: their canonical Agent URA
    /// is lifecycle state, not a value that can be reconstructed from the
    /// hosting device identity. The supplied scope is committed atomically
    /// with the execution row and LocalRuntime binding.
    pub fn hot_register_rpc_with_spec_and_authority_scope(
        &self,
        ability: impl Into<String>,
        owner: OwnerKind,
        authority_scope: AuthorityScope,
        manifest: crate::daemon::ability::manifest::AbilityManifest,
        handler: LocalRpcHandler,
    ) -> anyhow::Result<()> {
        DynamicRegistration::rpc_with_spec(ability, owner, manifest, handler)
            .with_authority_scope(authority_scope)
            .commit(self)
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
        manifest: crate::daemon::ability::manifest::AbilityManifest,
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

    /// Hot-register a stream handler under an already-resolved authority root.
    pub fn hot_register_stream_with_spec_and_authority_scope(
        &self,
        ability: impl Into<String>,
        owner: OwnerKind,
        authority_scope: AuthorityScope,
        manifest: crate::daemon::ability::manifest::AbilityManifest,
        handler: LocalStreamHandler,
    ) -> anyhow::Result<()> {
        self.hot_register_stream_with_spec_impl_and_authority_scope(
            ability,
            owner,
            authority_scope,
            manifest,
            handler,
            ControlPlaneImplementation::native_daemon(),
        )
    }

    pub fn hot_register_stream_with_spec_and_impl(
        &self,
        ability: impl Into<String>,
        owner: OwnerKind,
        manifest: crate::daemon::ability::manifest::AbilityManifest,
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
        manifest: crate::daemon::ability::manifest::AbilityManifest,
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

    /// Hot-register an envelope-aware RPC handler with explicit owner and
    /// registry manifest in the dynamic execution row.
    pub fn hot_register_rpc_with_envelope_and_spec(
        &self,
        ability: impl Into<String>,
        owner: OwnerKind,
        manifest: crate::daemon::ability::manifest::AbilityManifest,
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
        manifest: crate::daemon::ability::manifest::AbilityManifest,
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

    pub fn hot_register_rpc_with_envelope_spec_and_authority_scope(
        &self,
        ability: impl Into<String>,
        owner: OwnerKind,
        authority_scope: AuthorityScope,
        manifest: crate::daemon::ability::manifest::AbilityManifest,
        handler: LocalRpcHandlerWithEnvelope,
    ) -> anyhow::Result<()> {
        DynamicRegistration::rpc_with_envelope_and_spec_and_impl(
            ability,
            owner,
            manifest,
            handler,
            ControlPlaneImplementation::native_daemon(),
        )
        .with_authority_scope(authority_scope)
        .commit(self)
    }

    /// Hot-register an envelope-aware stream handler with explicit owner and
    /// registry manifest in the dynamic execution row.
    pub fn hot_register_stream_with_envelope_and_spec(
        &self,
        ability: impl Into<String>,
        owner: OwnerKind,
        manifest: crate::daemon::ability::manifest::AbilityManifest,
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

    /// Hot-register an envelope-aware stream handler under an
    /// already-resolved authority root.
    pub fn hot_register_stream_with_envelope_and_spec_and_authority_scope(
        &self,
        ability: impl Into<String>,
        owner: OwnerKind,
        authority_scope: AuthorityScope,
        manifest: crate::daemon::ability::manifest::AbilityManifest,
        handler: LocalStreamHandlerWithEnvelope,
    ) -> anyhow::Result<()> {
        DynamicRegistration::stream_with_envelope_and_spec_and_impl(
            ability,
            owner,
            manifest,
            handler,
            ControlPlaneImplementation::native_daemon(),
        )
        .with_authority_scope(authority_scope)
        .commit(self)
    }

    pub fn hot_register_stream_with_envelope_and_spec_and_impl(
        &self,
        ability: impl Into<String>,
        owner: OwnerKind,
        manifest: crate::daemon::ability::manifest::AbilityManifest,
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

    /// Hot-register an envelope-aware bidi handler with explicit owner and
    /// registry manifest in the dynamic execution row.
    pub fn hot_register_bidi_with_envelope_and_spec(
        &self,
        ability: impl Into<String>,
        owner: OwnerKind,
        manifest: crate::daemon::ability::manifest::AbilityManifest,
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
        manifest: crate::daemon::ability::manifest::AbilityManifest,
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

    /// True iff the dynamic side currently holds an execution-index row for
    /// `ability`.
    ///
    /// This is a diagnostic/collision predicate only. Discovery and routeability
    /// read the committed control-plane plus exact mode projections; they must
    /// not union static and dynamic execution rows as a publication source.
    pub fn has_dynamic(&self, ability: &str) -> bool {
        self.execution_index
            .read()
            .expect("execution_index RwLock poisoned")
            .contains_origin_handler_by_name(ability, ExecutionOrigin::Dynamic)
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

    /// Complete canonical catalogue read model.
    ///
    /// Every row is projected directly from one committed control-plane
    /// aggregate. No manifest DTO, static template, owner reconstruction, or
    /// mode/version collapse participates in this read path.
    pub fn authority_ability_catalog_snapshot(&self) -> Vec<AuthorityAbilityCatalogSnapshotRow> {
        self.control_plane
            .read()
            .expect("control_plane RwLock poisoned")
            .records()
            .into_iter()
            .map(|record| {
                let scope = record.authority().scope();
                let owner =
                    owner_kind_from_projection(scope.owner_projection()).unwrap_or_else(|| {
                        panic!(
                            "control-plane authority {:?} stored unknown owner projection {:?}",
                            scope.authority_root(),
                            scope.owner_projection()
                        )
                    });
                AuthorityAbilityCatalogSnapshotRow {
                    name: record.ability().to_string(),
                    owner,
                    descriptor: record.descriptor().clone(),
                }
            })
            .collect()
    }

    /// Atomic owner/root binding for the process-local invocation ledger.
    ///
    /// Governance providers receive this binding during catalog assembly;
    /// request handlers therefore never consult product credentials to infer
    /// the authority that already constructed the catalog.
    pub(crate) fn ledger_governance_authority(&self) -> LedgerGovernanceAuthority {
        self.authority_context.ledger_governance_authority()
    }

    /// Resolve the unique descriptor for a protocol-visible ability identity.
    ///
    /// The catalogue owns this projection so admission, descriptor-ref
    /// construction, and future protocol boundaries cannot independently
    /// reinterpret an execution registry key as a public ability name.
    pub(crate) fn public_descriptor_for_mode(
        &self,
        owner: &OwnerKind,
        public_name: &str,
        call_mode: DescriptorCallMode,
    ) -> Result<AbilityDescriptor, PublicDescriptorLookupError> {
        let mut matches = self
            .authority_ability_catalog_snapshot()
            .into_iter()
            .filter(|row| &row.owner == owner)
            .filter(|row| row.descriptor.public_name() == public_name)
            .filter(|row| row.descriptor.call_mode() == call_mode);
        let descriptor = matches
            .next()
            .ok_or_else(|| PublicDescriptorLookupError::Missing {
                owner: owner.clone(),
                public_name: public_name.to_string(),
                call_mode,
            })?
            .descriptor;
        if matches.next().is_some() {
            return Err(PublicDescriptorLookupError::Ambiguous {
                owner: owner.clone(),
                public_name: public_name.to_string(),
                call_mode,
            });
        }
        Ok(descriptor)
    }

    /// Returns Some when an RPC handler is registered for `ability`.
    pub fn get_rpc(&self, ability: &str) -> Option<LocalRpcHandler> {
        self.resolve_rpc(ability)
    }

    fn routeable_mode_registered(&self, ability: &str, call_mode: DescriptorCallMode) -> bool {
        let has_control_plane_record = self
            .control_plane_record_for_mode(ability, call_mode)
            .ok()
            .flatten()
            .is_some();
        if !has_control_plane_record {
            return false;
        }
        self.execution_index
            .read()
            .expect("execution_index RwLock poisoned")
            .has_mode(ability, call_mode)
    }

    /// True iff an RPC-mode handler and matching control-plane record are
    /// registered for `ability`, including the envelope-aware variant.
    /// `LocalRuntime` installation is verified by explicit runtime
    /// option/proof checks, never as catalogue presence fallback.
    pub fn has_rpc(&self, ability: &str) -> bool {
        self.routeable_mode_registered(ability, DescriptorCallMode::Rpc)
    }

    /// List all committed RPC ability names from the control-plane registry.
    /// Handler maps and `LocalRuntime` ability options are execution details,
    /// not publication sources.
    pub fn list_rpc_names(&self) -> Vec<String> {
        let mut names = BTreeSet::new();
        for record in self
            .control_plane
            .read()
            .expect("control_plane RwLock poisoned")
            .records()
        {
            if record.descriptor().call_mode() == DescriptorCallMode::Rpc {
                names.insert(record.ability().to_string());
            }
        }
        names.into_iter().collect()
    }

    /// True iff any local handler is registered for `ability` in the
    /// catalogue's execution index.
    ///
    /// Use it for metadata/collision decisions where the caller only needs to
    /// know whether the public name is already occupied.
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

    /// Resolve an RPC handler for the exact authority-scoped runtime key.
    ///
    /// This is the production surface for callers that have already resolved a
    /// descriptor under a concrete authority root. It intentionally does not
    /// fall back to a same-name handler under any other authority: same public
    /// ability name across Device, RealmAuthority, User, and Agent roots is a
    /// normal catalogue state, not an ambiguity the execution index may repair.
    pub fn resolve_rpc_for_authority(
        &self,
        authority_root: &str,
        ability: &str,
    ) -> anyhow::Result<Option<LocalRpcHandler>> {
        if self
            .control_plane_record_for_authority_mode(
                authority_root,
                ability,
                DescriptorCallMode::Rpc,
            )?
            .is_none()
        {
            anyhow::bail!(
                "RPC ability {ability:?} is not registered under authority {authority_root:?}"
            );
        }
        let key = ControlPlaneAbilityKey::new(authority_root, ability);
        Ok(self
            .execution_index
            .read()
            .expect("execution_index RwLock poisoned")
            .resolve_rpc_for_key(&key))
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

    /// True iff a server-stream handler and matching control-plane record are
    /// registered for `ability`, including the envelope-aware variant.
    /// `LocalRuntime` installation is verified by explicit runtime
    /// option/proof checks, never as catalogue presence fallback.
    pub fn has_stream(&self, ability: &str) -> bool {
        self.routeable_mode_registered(ability, DescriptorCallMode::Stream)
    }

    /// Returns Some when a bidi handler is registered for `ability`.
    pub fn get_bidi(&self, ability: &str) -> Option<LocalBidiHandler> {
        self.resolve_bidi(ability)
    }

    /// True iff a bidirectional-stream handler and matching control-plane
    /// record are registered for `ability`, including the envelope-aware
    /// variant. `LocalRuntime` installation is verified by explicit runtime
    /// option/proof checks, never as catalogue presence fallback.
    pub fn has_bidi(&self, ability: &str) -> bool {
        self.routeable_mode_registered(ability, DescriptorCallMode::Bidi)
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
                 Axon canonical Invocation::Invoke."
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
        .name(format!("axon-sdk-stream-{}", target.ability))
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
        .name(format!("axon-sdk-bidi-{}", target.ability))
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
                        let frame = axon_sdk::invocation::BidiInputFrame::new(payload)
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
    use serde_json::json;

    fn empty_registry() -> Arc<AxonAbilityCatalog> {
        Arc::new(combined_catalog())
    }

    fn test_runtime() -> Arc<LocalRuntime> {
        crate::daemon::axon_bridge::runtime_factory::build_local_runtime(
            crate::daemon::axon_bridge::runtime_factory::rejecting_test_key_resolver(),
            None,
        )
    }

    fn combined_catalog() -> AxonAbilityCatalog {
        AxonAbilityCatalog::new_with_runtime_and_authority_context(
            test_runtime(),
            AbilityAuthorityContext::for_combined_authority_roots(
                "easynet:///r/localhost/device/test-device",
            )
            .expect("combined test authority context"),
        )
    }

    fn ping_target_local() -> InvocationTarget {
        crate::daemon::invocation::routing::target::SystemInvocationTargetIssuer::local_root(
            "observe.health",
            json!({}),
            CallMode::Rpc,
        )
    }

    fn invoke_test_rpc(
        catalog: &AxonAbilityCatalog,
        ability: &str,
        args: serde_json::Value,
    ) -> anyhow::Result<serde_json::Value> {
        catalog.invoke_rpc_target_json(
            crate::daemon::invocation::routing::target::SystemInvocationTargetIssuer::local_root(
                ability,
                args,
                CallMode::Rpc,
            ),
        )
    }

    fn test_manifest(
        ability: &str,
        description: &str,
        input_schema: serde_json::Value,
    ) -> crate::daemon::ability::manifest::AbilityManifest {
        crate::daemon::ability::manifest::AbilityManifest::new(
            ability.rsplit('.').next().unwrap_or(ability),
            description,
            input_schema,
        )
        .and_then(|manifest| manifest.with_admission_action("invoke"))
        .expect("test ability manifest is well-formed")
    }

    fn runtime_key_for_registered_mode(
        catalog: &AxonAbilityCatalog,
        ability: &str,
        call_mode: DescriptorCallMode,
    ) -> String {
        catalog
            .control_plane_record_for_mode(ability, call_mode)
            .expect("control-plane lookup is unambiguous")
            .expect("control-plane record");
        catalog
            .handler_control_plane_key(ability)
            .expect("handler authority key")
            .runtime_key()
            .expect("runtime key")
    }

    fn hot_register_test_rpc(
        catalog: &AxonAbilityCatalog,
        ability: &str,
        owner: OwnerKind,
        handler: LocalRpcHandler,
    ) -> anyhow::Result<()> {
        catalog.hot_register_rpc_with_spec(
            ability,
            owner,
            test_manifest(
                ability,
                "Test dynamic ability.",
                serde_json::json!({"type": "object"}),
            ),
            handler,
        )
    }

    fn register_test_rpc(
        catalog: &mut AxonAbilityCatalog,
        ability: &str,
        owner: OwnerKind,
        handler: LocalRpcHandler,
    ) {
        catalog.register_rpc_with_spec(
            ability,
            owner,
            test_manifest(
                ability,
                "Test static RPC ability.",
                serde_json::json!({"type": "object"}),
            ),
            handler,
        );
    }

    fn register_test_stream(
        catalog: &mut AxonAbilityCatalog,
        ability: &str,
        owner: OwnerKind,
        handler: LocalStreamHandler,
    ) {
        catalog.register_stream_with_spec(
            ability,
            owner,
            test_manifest(
                ability,
                "Test static stream ability.",
                serde_json::json!({"type": "object"}),
            ),
            handler,
        );
    }

    fn register_test_bidi(
        catalog: &mut AxonAbilityCatalog,
        ability: &str,
        owner: OwnerKind,
        handler: LocalBidiHandler,
    ) {
        catalog.register_bidi_with_spec(
            ability,
            owner,
            test_manifest(
                ability,
                "Test static bidi ability.",
                serde_json::json!({"type": "object"}),
            ),
            handler,
        );
    }

    fn register_test_rpc_env(
        catalog: &mut AxonAbilityCatalog,
        ability: &str,
        owner: OwnerKind,
        handler: LocalRpcHandlerWithEnvelope,
    ) {
        catalog.register_rpc_with_envelope_and_spec(
            ability,
            owner,
            test_manifest(
                ability,
                "Test static envelope RPC ability.",
                serde_json::json!({"type": "object"}),
            ),
            handler,
        );
    }

    fn register_test_stream_env(
        catalog: &mut AxonAbilityCatalog,
        ability: &str,
        owner: OwnerKind,
        handler: LocalStreamHandlerWithEnvelope,
    ) {
        catalog.register_stream_with_envelope_and_spec(
            ability,
            owner,
            test_manifest(
                ability,
                "Test static envelope stream ability.",
                serde_json::json!({"type": "object"}),
            ),
            handler,
        );
    }

    fn register_test_bidi_env(
        catalog: &mut AxonAbilityCatalog,
        ability: &str,
        owner: OwnerKind,
        handler: LocalBidiHandlerWithEnvelope,
    ) {
        catalog.register_bidi_with_envelope_and_spec(
            ability,
            owner,
            test_manifest(
                ability,
                "Test static envelope bidi ability.",
                serde_json::json!({"type": "object"}),
            ),
            handler,
        );
    }

    #[test]
    fn stale_hot_agent_enrollment_rollback_cannot_remove_reenrolled_incarnation() {
        let device =
            CanonicalDeviceAuthority::parse("easynet:///r/acme/device/edge-01".to_string())
                .expect("canonical device");
        let inventory = HotAgentAuthorityInventory::new(device, BTreeMap::new());
        let agent = "alice";
        let root = "easynet:///r/acme/agent/alice.worker";

        let stale = {
            let mut state = inventory.state.write().unwrap();
            let incarnation = state.allocate_incarnation(agent).unwrap();
            state.roots.insert(
                agent.to_string(),
                HotAgentAuthorityEntry {
                    authority_root: root.to_string(),
                    incarnation,
                },
            );
            HotAgentAuthorityEnrollment {
                agent: agent.to_string(),
                authority_root: root.to_string(),
                inserted: true,
                incarnation,
            }
        };

        let current_incarnation = {
            let mut state = inventory.state.write().unwrap();
            state.roots.remove(agent);
            state.advance_generation(agent).unwrap();
            let incarnation = state.allocate_incarnation(agent).unwrap();
            state.roots.insert(
                agent.to_string(),
                HotAgentAuthorityEntry {
                    authority_root: root.to_string(),
                    incarnation,
                },
            );
            incarnation
        };

        inventory
            .rollback_enrollment(&stale)
            .expect("stale rollback is an idempotent no-op");
        let state = inventory.state.read().unwrap();
        let current = state.roots.get(agent).expect("E2 remains enrolled");
        assert_eq!(current.incarnation, current_incarnation);
        assert_eq!(current.authority_root, root);
    }

    #[test]
    fn hot_agent_authority_generation_overflow_fails_closed() {
        let device =
            CanonicalDeviceAuthority::parse("easynet:///r/acme/device/edge-01".to_string())
                .expect("canonical device");
        let inventory = HotAgentAuthorityInventory::new(device, BTreeMap::new());
        let mut state = inventory.state.write().unwrap();
        state.generation = u64::MAX;

        let error = state.advance_generation("alice").unwrap_err();

        assert_eq!(
            error,
            HotAgentAuthorityInventoryError::CounterOverflow {
                agent: "alice".to_string(),
                counter: "generation",
            }
        );
        assert_eq!(state.generation, u64::MAX);
    }

    #[test]
    fn hot_agent_authority_incarnation_overflow_fails_closed() {
        let device =
            CanonicalDeviceAuthority::parse("easynet:///r/acme/device/edge-01".to_string())
                .expect("canonical device");
        let inventory = HotAgentAuthorityInventory::new(device, BTreeMap::new());
        let mut state = inventory.state.write().unwrap();
        state.next_incarnation = u64::MAX;

        let error = state.allocate_incarnation("alice").unwrap_err();

        assert_eq!(
            error,
            HotAgentAuthorityInventoryError::CounterOverflow {
                agent: "alice".to_string(),
                counter: "incarnation",
            }
        );
        assert_eq!(state.next_incarnation, u64::MAX);
    }

    #[test]
    fn fixed_authority_context_rejects_non_device_ura() {
        let err = AbilityAuthorityContext::for_device_authority_root("easynet:///r/acme/authority")
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
    fn fixed_realm_authority_context_uses_configured_realm_without_device_credentials() {
        let hub_ura = crate::core::ura::hub_ura("realm-b");
        let context = AbilityAuthorityContext::for_realm_authority_root(hub_ura.clone())
            .expect("canonical realm authority context");

        let authority_scope = OwnerKind::RealmAuthority
            .authority_scope(&context)
            .expect("realm owner scope");
        assert_eq!(authority_scope.authority_root(), hub_ura);
        assert_eq!(
            context.local_runtime_owners(),
            vec![OwnerKind::RealmAuthority]
        );
        assert_eq!(context.ledger_governance_owner(), OwnerKind::RealmAuthority);

        let err =
            AbilityAuthorityContext::for_realm_authority_root("easynet:///r/realm-b/device/dev-b")
                .expect_err("Device URA must not be accepted as realm authority");
        assert!(matches!(
            err,
            AbilityControlPlaneError::InvalidRealmAuthorityRoot { .. }
        ));

        for unsupported in [
            OwnerKind::Device,
            OwnerKind::Agent("worker".to_string()),
            OwnerKind::User("alice".to_string()),
        ] {
            let error = unsupported
                .authority_scope(&context)
                .expect_err("realm authority set must reject Device-plane owners");
            assert!(matches!(
                error,
                AbilityControlPlaneError::UnsupportedOwnerForAuthoritySet {
                    authority_set: "realm-authority",
                    ..
                }
            ));
        }
    }

    #[test]
    fn fixed_device_context_keeps_device_sponsored_agent_policy_and_rejects_realm_authority() {
        let device_ura = crate::core::ura::device_ura("realm-b", "dev-b");
        let context = AbilityAuthorityContext::for_device_authority_root(&device_ura)
            .expect("fixed Device authority context");

        assert_eq!(context.ledger_governance_owner(), OwnerKind::Device);

        let agent_scope = OwnerKind::Agent("worker".to_string())
            .authority_scope(&context)
            .expect("Device-hosted Agent authority");
        assert_eq!(
            agent_scope.authority_root(),
            crate::core::ura::device_agent_ura("realm-b", "dev-b", "worker")
        );
        let authority_error = OwnerKind::RealmAuthority
            .authority_scope(&context)
            .expect_err("Device authority set must reject RealmAuthority owners");
        assert!(matches!(
            authority_error,
            AbilityControlPlaneError::UnsupportedOwnerForAuthoritySet {
                authority_set: "device",
                ..
            }
        ));
    }

    #[test]
    fn static_registration_excludes_non_hosted_owner_before_control_plane_and_runtime() {
        let hub_ura = crate::core::ura::hub_ura("hub-only");
        let runtime = test_runtime();
        let mut catalog = AxonAbilityCatalog::new_with_runtime_and_authority_context(
            Arc::clone(&runtime),
            AbilityAuthorityContext::for_realm_authority_root(&hub_ura)
                .expect("realm authority context"),
        );

        register_test_rpc(
            &mut catalog,
            "device.test.only",
            OwnerKind::Device,
            ok_handler(),
        );
        register_test_rpc(
            &mut catalog,
            "meta.hub_only",
            OwnerKind::RealmAuthority,
            ok_handler(),
        );

        let rows = catalog.authority_ability_catalog_snapshot();
        assert_eq!(rows.len(), 1);
        assert_eq!(rows[0].owner, OwnerKind::RealmAuthority);
        assert_eq!(rows[0].descriptor.owner_ura, hub_ura);
        assert_eq!(
            catalog.static_authority_exclusion_snapshot(),
            BTreeMap::from([("device".to_string(), 1)])
        );

        let hub_runtime_key = local_runtime_ability_key_for_authority(&hub_ura, "meta.hub_only")
            .expect("RealmAuthority runtime key");
        assert!(block_on_runtime_sync(runtime.ability_options(&hub_runtime_key)).is_some());
        let synthetic_device_key = local_runtime_ability_key_for_authority(
            &crate::core::ura::device_ura("hub-only", "local"),
            "device.test.only",
        )
        .expect("hypothetical Device runtime key");
        assert!(block_on_runtime_sync(runtime.ability_options(&synthetic_device_key)).is_none());
    }

    #[test]
    fn device_registration_excludes_realm_authority_owner_before_control_plane_and_runtime() {
        let device_ura = crate::core::ura::device_ura("device-only", "dev-1");
        let hub_ura = crate::core::ura::hub_ura("device-only");
        let runtime = test_runtime();
        let mut catalog = AxonAbilityCatalog::new_with_runtime_and_authority_context(
            Arc::clone(&runtime),
            AbilityAuthorityContext::for_device_authority_root(&device_ura)
                .expect("Device authority context"),
        );

        register_test_rpc(
            &mut catalog,
            "device.test.only",
            OwnerKind::Device,
            ok_handler(),
        );
        register_test_rpc(
            &mut catalog,
            "meta.hub_only",
            OwnerKind::RealmAuthority,
            ok_handler(),
        );

        let rows = catalog.authority_ability_catalog_snapshot();
        assert_eq!(rows.len(), 1);
        assert_eq!(rows[0].owner, OwnerKind::Device);
        assert_eq!(rows[0].descriptor.owner_ura, device_ura);
        assert_eq!(
            catalog.static_authority_exclusion_snapshot(),
            BTreeMap::from([("authority".to_string(), 1)])
        );

        let device_runtime_key =
            local_runtime_ability_key_for_authority(&device_ura, "device.test.only")
                .expect("Device runtime key");
        assert!(block_on_runtime_sync(runtime.ability_options(&device_runtime_key)).is_some());
        let hub_runtime_key = local_runtime_ability_key_for_authority(&hub_ura, "meta.hub_only")
            .expect("hypothetical RealmAuthority runtime key");
        assert!(block_on_runtime_sync(runtime.ability_options(&hub_runtime_key)).is_none());
    }

    #[test]
    fn dynamic_registration_rejects_non_hosted_owner_without_partial_rows() {
        let hub_ura = crate::core::ura::hub_ura("hub-only");
        let runtime = test_runtime();
        let catalog = AxonAbilityCatalog::new_with_runtime_and_authority_context(
            Arc::clone(&runtime),
            AbilityAuthorityContext::for_realm_authority_root(&hub_ura)
                .expect("realm authority context"),
        );

        let error =
            hot_register_test_rpc(&catalog, "device.dynamic", OwnerKind::Device, ok_handler())
                .expect_err("realm authority set must reject dynamic Device owner");
        assert!(error
            .to_string()
            .contains("authority set \"realm-authority\""));
        assert!(catalog.authority_ability_catalog_snapshot().is_empty());
        assert!(!catalog.has_dynamic("device.dynamic"));

        let runtime_key = local_runtime_ability_key_for_authority(
            &crate::core::ura::device_ura("hub-only", "local"),
            "device.dynamic",
        )
        .expect("hypothetical Device runtime key");
        assert!(block_on_runtime_sync(runtime.ability_options(&runtime_key)).is_none());

        let device_ura = crate::core::ura::device_ura("device-only", "dev-1");
        let device_catalog = AxonAbilityCatalog::new_with_runtime_and_authority_context(
            test_runtime(),
            AbilityAuthorityContext::for_device_authority_root(device_ura)
                .expect("Device authority context"),
        );
        let error = hot_register_test_rpc(
            &device_catalog,
            "hub.dynamic",
            OwnerKind::RealmAuthority,
            ok_handler(),
        )
        .expect_err("Device authority set must reject dynamic RealmAuthority owner");
        assert!(error.to_string().contains("authority set \"device\""));
        assert!(device_catalog
            .authority_ability_catalog_snapshot()
            .is_empty());
        assert!(!device_catalog.has_dynamic("hub.dynamic"));
    }

    #[test]
    fn combined_authority_context_exposes_distinct_device_and_realm_authority_owners() {
        let device_ura = crate::core::ura::device_ura("realm-b", "dev-b");
        let context = AbilityAuthorityContext::for_combined_authority_roots(device_ura.clone())
            .expect("combined authority context");

        assert_eq!(
            context.local_runtime_owners(),
            vec![OwnerKind::Device, OwnerKind::RealmAuthority]
        );
        assert_eq!(context.ledger_governance_owner(), OwnerKind::Device);
        assert_eq!(
            OwnerKind::Device
                .authority_scope(&context)
                .expect("Device scope")
                .authority_root(),
            device_ura
        );
        assert_eq!(
            OwnerKind::RealmAuthority
                .authority_scope(&context)
                .expect("RealmAuthority scope")
                .authority_root(),
            crate::core::ura::hub_ura("realm-b")
        );
    }

    #[test]
    fn public_catalog_constructor_preserves_device_and_realm_authority_registration() {
        let mut catalog = combined_catalog();
        register_test_rpc(
            &mut catalog,
            "device.test.constructor",
            OwnerKind::Device,
            ok_handler(),
        );
        register_test_rpc(
            &mut catalog,
            "meta.constructor",
            OwnerKind::RealmAuthority,
            ok_handler(),
        );

        let rows = catalog.authority_ability_catalog_snapshot();
        assert!(rows.iter().any(|row| row.owner == OwnerKind::Device));
        assert!(rows
            .iter()
            .any(|row| row.owner == OwnerKind::RealmAuthority));
        assert!(catalog.static_authority_exclusion_snapshot().is_empty());
    }

    #[test]
    fn static_explicit_scope_rejects_owner_projection_mismatch() {
        let device_ura = crate::core::ura::device_ura("scope-test", "dev-1");
        let mut catalog = AxonAbilityCatalog::new_with_runtime_and_authority_context(
            test_runtime(),
            AbilityAuthorityContext::for_combined_authority_roots(&device_ura)
                .expect("combined authority context"),
        );
        let device_scope = AuthorityScope::new("device", &device_ura).expect("Device scope");

        let error = catalog
            .register_static(
                StaticRegistration::new(
                    "meta.scope_mismatch",
                    OwnerKind::RealmAuthority,
                    StaticRegistrationHandler::Rpc(ok_handler()),
                )
                .with_manifest(test_manifest(
                    "meta.scope_mismatch",
                    "Scope mismatch test ability.",
                    serde_json::json!({"type": "object"}),
                ))
                .with_authority_scope(device_scope),
            )
            .expect_err("RealmAuthority owner with Device projection must fail closed");
        assert!(error
            .to_string()
            .contains("does not match registration owner"));
        assert!(catalog.authority_ability_catalog_snapshot().is_empty());
    }

    #[test]
    fn dynamic_explicit_scope_rejects_correct_owner_with_foreign_root() {
        let hub_ura = crate::core::ura::hub_ura("scope-test");
        let catalog = AxonAbilityCatalog::new_with_runtime_and_authority_context(
            test_runtime(),
            AbilityAuthorityContext::for_realm_authority_root(&hub_ura)
                .expect("realm authority context"),
        );
        let foreign_scope = AuthorityScope::new("authority", crate::core::ura::hub_ura("foreign"))
            .expect("foreign RealmAuthority scope is structurally valid");

        let error = DynamicRegistration::rpc_with_spec(
            "meta.foreign_scope",
            OwnerKind::RealmAuthority,
            test_manifest(
                "meta.foreign_scope",
                "Foreign scope test ability.",
                serde_json::json!({"type": "object"}),
            ),
            ok_handler(),
        )
        .with_authority_scope(foreign_scope)
        .commit(&catalog)
        .expect_err("foreign RealmAuthority root must fail closed");
        assert!(error.to_string().contains("is not hosted by authority set"));
        assert!(catalog.authority_ability_catalog_snapshot().is_empty());
        assert!(!catalog.has_dynamic("meta.foreign_scope"));
    }

    #[test]
    fn user_owner_may_delegate_to_its_explicitly_hosted_agent_only() {
        let pages_agent = crate::core::ura::agent_ura("scope-test", "user-a", "pages");
        let context = AbilityAuthorityContext::for_device_authority_root_with_hosted_agents(
            crate::core::ura::device_ura("scope-test", "dev-1"),
            vec![pages_agent.clone()],
        )
        .expect("fixed hosted Agent authority context");
        let owner = OwnerKind::User("user-a".to_string());
        let accepted =
            AuthorityScope::new("user:user-a", &pages_agent).expect("user-owned Pages scope");
        context
            .ensure_explicit_scope_supported(&owner, &accepted)
            .expect("user-owned ability may execute on that user's hosted agent");

        let foreign_owner = OwnerKind::User("user-b".to_string());
        let rejected = context
            .ensure_explicit_scope_supported(
                &foreign_owner,
                &AuthorityScope::new("user:user-b", &pages_agent)
                    .expect("structurally valid foreign user scope"),
            )
            .expect_err("a user's ability must not execute on another user's hosted agent");
        assert!(matches!(
            rejected,
            AbilityControlPlaneError::AuthorityScopeRootNotHosted { .. }
        ));
    }

    #[test]
    fn declared_agent_root_cannot_override_persisted_hosted_identity() {
        let persisted_pages = crate::core::ura::agent_ura("scope-test", "user-a", "pages");
        let conflicting_pages = crate::core::ura::agent_ura("scope-test", "user-b", "pages");
        let context = AbilityAuthorityContext::for_device_authority_root_with_hosted_agents(
            crate::core::ura::device_ura("scope-test", "dev-1"),
            vec![persisted_pages],
        )
        .expect("fixed hosted Agent authority context");

        let error = context
            .with_declared_agent_authority_root(conflicting_pages)
            .expect_err("static capability must not replace persisted hosted Agent identity");
        assert!(matches!(
            error,
            AbilityControlPlaneError::InvalidAuthorityRoot { .. }
        ));
    }

    #[test]
    fn agent_scope_rejects_same_realm_and_agent_id_owned_by_another_user() {
        let hosted_agent_ura = crate::core::ura::agent_ura("scope-test", "user-a", "worker");
        let context = AbilityAuthorityContext::for_device_authority_root_with_hosted_agents(
            crate::core::ura::device_ura("scope-test", "dev-1"),
            vec![hosted_agent_ura.clone()],
        )
        .expect("fixed hosted Agent authority context");
        let owner = OwnerKind::Agent("worker".to_string());
        let exact_scope =
            AuthorityScope::new("agent:worker", &hosted_agent_ura).expect("hosted Agent scope");
        context
            .ensure_explicit_scope_supported(&owner, &exact_scope)
            .expect("exact hosted Agent root must be accepted");

        let catalog =
            AxonAbilityCatalog::new_with_runtime_and_authority_context(test_runtime(), context);
        let foreign_user_scope = AuthorityScope::new(
            "agent:worker",
            crate::core::ura::agent_ura("scope-test", "user-b", "worker"),
        )
        .expect("foreign user Agent scope is structurally valid");
        let error = DynamicRegistration::rpc_with_spec(
            "worker.chat",
            owner,
            test_manifest(
                "worker.chat",
                "Foreign Agent scope test ability.",
                serde_json::json!({"type": "object"}),
            ),
            ok_handler(),
        )
        .with_authority_scope(foreign_user_scope)
        .commit(&catalog)
        .expect_err("same realm/agent id under another user must fail closed");
        assert!(error.to_string().contains("is not hosted by authority set"));
        assert!(catalog.authority_ability_catalog_snapshot().is_empty());
    }

    #[test]
    fn hosted_authority_inventory_rejects_foreign_device_sponsored_agent() {
        let error = AbilityAuthorityContext::for_device_authority_root_with_hosted_agents(
            crate::core::ura::device_ura("scope-test", "dev-a"),
            vec![crate::core::ura::device_agent_ura(
                "scope-test",
                "dev-b",
                "terminal",
            )],
        )
        .expect_err("a device may not claim another device's System Agent root");
        assert!(matches!(
            error,
            AbilityControlPlaneError::InvalidAuthorityRoot { .. }
        ));
    }

    #[test]
    fn combined_authority_set_accepts_exact_explicit_device_and_authority_scopes() {
        let device_ura = crate::core::ura::device_ura("scope-test", "dev-1");
        let hub_ura = crate::core::ura::hub_ura("scope-test");
        let mut catalog = AxonAbilityCatalog::new_with_runtime_and_authority_context(
            test_runtime(),
            AbilityAuthorityContext::for_combined_authority_roots(&device_ura)
                .expect("combined authority context"),
        );

        for (owner, projection, authority_root) in [
            (OwnerKind::Device, "device", device_ura.as_str()),
            (OwnerKind::RealmAuthority, "authority", hub_ura.as_str()),
        ] {
            catalog
                .register_static(
                    StaticRegistration::new(
                        "meta.exact_scope",
                        owner,
                        StaticRegistrationHandler::Rpc(ok_handler()),
                    )
                    .with_manifest(test_manifest(
                        "meta.exact_scope",
                        "Exact scope test ability.",
                        serde_json::json!({"type": "object"}),
                    ))
                    .with_authority_scope(
                        AuthorityScope::new(projection, authority_root).expect("exact scope"),
                    ),
                )
                .expect("Both must admit each exact hosted authority scope");
        }

        let rows = catalog.authority_ability_catalog_snapshot();
        assert_eq!(rows.len(), 2);
        assert!(rows
            .iter()
            .any(|row| row.descriptor.owner_ura == device_ura));
        assert!(rows.iter().any(|row| row.descriptor.owner_ura == hub_ura));
    }

    #[test]
    fn owner_kind_projection_rejects_retired_hub_marker() {
        assert_eq!(
            OwnerKind::RealmAuthority.authority_projection(),
            "authority"
        );
        assert_eq!(
            owner_kind_from_projection("authority"),
            Some(OwnerKind::RealmAuthority)
        );
        assert_eq!(owner_kind_from_projection("hub"), None);
    }

    #[test]
    fn authority_catalog_snapshot_preserves_combined_runtime_rows() {
        let device_ura = crate::core::ura::device_ura("realm-b", "dev-b");
        let hub_ura = crate::core::ura::hub_ura("realm-b");
        let context = AbilityAuthorityContext::for_combined_authority_roots(&device_ura)
            .expect("combined authority context");
        let mut catalog =
            AxonAbilityCatalog::new_with_runtime_and_authority_context(test_runtime(), context);
        register_test_rpc(
            &mut catalog,
            "meta.describe",
            OwnerKind::Device,
            Arc::new(|_args| Ok(json!({}))),
        );
        register_test_rpc(
            &mut catalog,
            "meta.describe",
            OwnerKind::RealmAuthority,
            Arc::new(|_args| Ok(json!({}))),
        );

        let rows = catalog.authority_ability_catalog_snapshot();
        assert_eq!(rows.len(), 2, "combined authority rows must not collapse");
        assert!(rows.iter().any(|row| {
            row.owner == OwnerKind::Device
                && row.descriptor.owner_ura == device_ura
                && row.descriptor.canonical_ability_ura().as_deref()
                    == Some(
                        crate::core::ura::device_ability_ura("realm-b", "dev-b", "meta.describe")
                            .as_str(),
                    )
        }));
        assert!(rows.iter().any(|row| {
            row.owner == OwnerKind::RealmAuthority
                && row.descriptor.owner_ura == hub_ura
                && row.descriptor.canonical_ability_ura().as_deref()
                    == Some(crate::core::ura::hub_ability_ura("realm-b", "meta.describe").as_str())
        }));

        let runtime = catalog.runtime().expect("combined LocalRuntime");
        for authority_root in [&device_ura, &hub_ura] {
            let runtime_key =
                local_runtime_ability_key_for_authority(authority_root, "meta.describe")
                    .expect("authority-scoped runtime key");
            assert!(
                block_on_runtime_sync(runtime.ability_options(&runtime_key)).is_some(),
                "combined runtime must retain meta.describe for {authority_root}"
            );
        }

        assert_eq!(
            rows.iter()
                .filter(|row| row.name == "meta.describe")
                .count(),
            2,
            "canonical snapshot must retain one descriptor per authority"
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
        let mut reg = combined_catalog();
        register_test_rpc(
            &mut reg,
            "observe.health",
            OwnerKind::Device,
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
        let mut reg = combined_catalog();
        register_test_rpc_env(
            &mut reg,
            "media.x.snapshot",
            OwnerKind::Device,
            Arc::new(|env: EnvelopeContext, _args: Value| {
                Ok(json!({
                    "saw_subject": env.subject(),
                    "args_subject_was_present": false,
                }))
            }),
        );
        let dispatcher = Arc::new(reg);
        let target = crate::daemon::invocation::routing::target::SystemInvocationTargetIssuer::local_root_for_subject(
            "media.x.snapshot",
            json!({}),
            CallMode::Rpc,
            "easynet:///r/acme/resource/01CAM",
        );
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
        let mut reg = combined_catalog();
        register_test_rpc(
            &mut reg,
            "x.dual",
            OwnerKind::Device,
            Arc::new(|_args: Value| Ok(json!({"path": "legacy"}))),
        );
        register_test_rpc_env(
            &mut reg,
            "x.dual",
            OwnerKind::Device,
            Arc::new(|_env: EnvelopeContext, _args: Value| Ok(json!({"path": "envelope"}))),
        );
    }

    #[test]
    fn envelope_aware_handler_without_subject_still_dispatches() {
        // Callers that do not set an explicit resource subject still carry a
        // complete AXIOM tuple. The local runtime sets subject=callee for root
        // calls, and envelope-aware handlers must see that value instead of an
        // erased `None`.
        let mut reg = combined_catalog();
        register_test_rpc_env(
            &mut reg,
            "x.optional",
            OwnerKind::Device,
            Arc::new(|env: EnvelopeContext, _args: Value| {
                Ok(json!({
                    "subject_present": true,
                    "subject_eq_callee": env.subject() == env.callee(),
                }))
            }),
        );
        let dispatcher = Arc::new(reg);
        let target =
            crate::daemon::invocation::routing::target::SystemInvocationTargetIssuer::local_root(
                "x.optional",
                json!({}),
                CallMode::Rpc,
            );
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
        let mut reg = combined_catalog();
        register_test_stream_env(
            &mut reg,
            "x.subscribe",
            OwnerKind::Device,
            Arc::new(|env: EnvelopeContext, _args: Value| {
                let frame = json!({"subject_seen": env.subject()});
                Ok(StreamSource::Snapshot(vec![frame]))
            }),
        );
        let dispatcher = Arc::new(reg);
        let target = crate::daemon::invocation::routing::target::SystemInvocationTargetIssuer::local_root_for_subject(
            "x.subscribe",
            json!({}),
            CallMode::Stream,
            "easynet:///r/x/resource/01MIC",
        );
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
        let mut reg = combined_catalog();
        register_test_rpc_env(
            &mut reg,
            "x.env_only",
            OwnerKind::Device,
            Arc::new(|_env, _args| Ok(json!({}))),
        );
        let names = reg.list_abilities();
        assert!(
            names.iter().any(|n| n == "x.env_only"),
            "envelope-aware ability missing from list_abilities: {names:?}"
        );
    }

    #[test]
    fn remote_target_returns_unified_path_redirect() {
        // Joint-plan phase 4: `TargetScope::Remote` no longer
        // routes through a dispatcher-owned network stub. Cross-device
        // dispatch flows through
        // `daemon::invocation::routing::remote_invoke::invoke_remote_target`
        // instead. The dispatcher surfaces a typed error
        // pointing the caller at the new path so a stale Remote
        // construction fails loud instead of silently bouncing
        // to Local.
        let dispatcher = empty_registry();
        let target =
            crate::daemon::invocation::routing::target::SystemInvocationTargetIssuer::remote_root(
                NodeId::new("peer"),
                "observe.health",
                json!({}),
                CallMode::Rpc,
            );
        let err = dispatcher.execute_rpc(target).unwrap_err();
        let msg = format!("{err}");
        assert!(
            msg.contains("canonical Invocation::Invoke") || msg.contains("remote_invoke"),
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
        let mut reg = combined_catalog();
        register_test_rpc(
            &mut reg,
            "observe.health",
            OwnerKind::Device,
            Arc::new(|_| Ok(Value::Null)),
        );
        register_test_rpc(
            &mut reg,
            "test.foo",
            OwnerKind::Device,
            Arc::new(|_| Ok(Value::Null)),
        );
        register_test_rpc(
            &mut reg,
            "test.bar",
            OwnerKind::Device,
            Arc::new(|_| Ok(Value::Null)),
        );
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
        let mut reg = combined_catalog();
        register_test_rpc(&mut reg, "fs.read", OwnerKind::Device, ok_handler());
        assert!(reg.list_abilities().contains(&"fs.read".to_string()));
        // The handler is still installed in the rpc map, but its control-plane
        // record is gone.
        reg.clear_owner_for_test("fs.read");
        assert!(
            reg.has_registered_handler("fs.read"),
            "execution index still holds the closure"
        );
        assert!(
            !reg.has_rpc("fs.read"),
            "routeability must require a committed RPC control-plane record"
        );
        assert!(
            !reg.list_abilities().contains(&"fs.read".to_string()),
            "list_abilities must read control-plane, not the handler map union"
        );
    }

    #[test]
    fn routeability_helpers_require_control_plane_records_for_all_modes() {
        let mut reg = combined_catalog();
        register_test_rpc(&mut reg, "doomed.rpc", OwnerKind::Device, ok_handler());
        register_test_stream(
            &mut reg,
            "doomed.stream",
            OwnerKind::Device,
            Arc::new(|_| Ok(StreamSource::Snapshot(Vec::new()))),
        );
        register_test_bidi(
            &mut reg,
            "doomed.bidi",
            OwnerKind::Device,
            trivial_bidi_handler(),
        );
        assert!(reg.has_rpc("doomed.rpc"));
        assert!(reg.has_stream("doomed.stream"));
        assert!(reg.has_bidi("doomed.bidi"));

        for ability in ["doomed.rpc", "doomed.stream", "doomed.bidi"] {
            reg.clear_owner_for_test(ability);
            assert!(
                reg.has_registered_handler(ability),
                "execution index remains installed for {ability}"
            );
        }

        assert!(!reg.has_rpc("doomed.rpc"));
        assert!(!reg.has_stream("doomed.stream"));
        assert!(!reg.has_bidi("doomed.bidi"));
    }

    #[test]
    fn routeability_helpers_do_not_fallback_to_local_runtime_options() {
        let mut reg = combined_catalog();
        register_test_rpc(&mut reg, "runtime.rpc", OwnerKind::Device, ok_handler());
        register_test_stream(
            &mut reg,
            "runtime.stream",
            OwnerKind::Device,
            Arc::new(|_| Ok(StreamSource::Snapshot(Vec::new()))),
        );
        register_test_bidi(
            &mut reg,
            "runtime.bidi",
            OwnerKind::Device,
            trivial_bidi_handler(),
        );
        let runtime = reg.runtime().expect("registry owns LocalRuntime");
        let rpc_key = runtime_key_for_registered_mode(&reg, "runtime.rpc", DescriptorCallMode::Rpc);
        let stream_key =
            runtime_key_for_registered_mode(&reg, "runtime.stream", DescriptorCallMode::Stream);
        let bidi_key =
            runtime_key_for_registered_mode(&reg, "runtime.bidi", DescriptorCallMode::Bidi);

        for (ability, call_mode) in [
            ("runtime.rpc", DescriptorCallMode::Rpc),
            ("runtime.stream", DescriptorCallMode::Stream),
            ("runtime.bidi", DescriptorCallMode::Bidi),
        ] {
            reg.control_plane_record_for_mode(ability, call_mode)
                .expect("control-plane lookup is unambiguous")
                .expect("control-plane record");
            let key = reg
                .handler_control_plane_key(ability)
                .expect("handler authority key");
            assert!(
                reg.execution_index
                    .write()
                    .expect("execution_index RwLock poisoned")
                    .drain_static(&key),
                "static execution row removed for {ability}"
            );
        }

        assert!(block_on_runtime_sync(runtime.ability_options(&rpc_key)).is_some());
        assert!(block_on_runtime_sync(runtime.ability_options(&stream_key)).is_some());
        assert!(block_on_runtime_sync(runtime.ability_options(&bidi_key)).is_some());
        assert!(!reg.has_rpc("runtime.rpc"));
        assert!(!reg.has_stream("runtime.stream"));
        assert!(!reg.has_bidi("runtime.bidi"));
    }

    #[test]
    fn static_runtime_key_validates_exact_authority_mode_record() {
        let mut reg = combined_catalog();
        register_test_rpc(&mut reg, "shared.route", OwnerKind::Device, ok_handler());
        let device_key = reg
            .handler_control_plane_key("shared.route")
            .expect("device RPC handler has an exact control-plane record");

        reg.register_control_plane_descriptor_with_owner(
            "shared.route",
            &OwnerKind::RealmAuthority,
            &test_manifest(
                "shared.route",
                "Same public name under an unrelated RealmAuthority Stream authority.",
                serde_json::json!({"type": "object"}),
            ),
            DescriptorCallMode::Stream,
            ReceiptSemantics::Operational,
            &ControlPlaneImplementation::native_daemon(),
        )
        .expect("unrelated Hub Stream descriptor registers");

        let got = reg
            .handler_control_plane_key("shared.route")
            .expect("runtime key derivation must validate only the device RPC tuple");

        assert_eq!(got, device_key);
    }

    #[test]
    fn static_runtime_key_rejects_unrelated_authority_record_as_rescue_path() {
        let mut reg = combined_catalog();
        register_test_rpc(&mut reg, "shared.missing", OwnerKind::Device, ok_handler());
        let device_key = reg
            .handler_control_plane_key("shared.missing")
            .expect("device RPC handler has an exact control-plane record");
        assert!(reg.remove_control_plane_record_for_authority_mode(
            device_key.authority_root(),
            device_key.ability(),
            DescriptorCallMode::Rpc,
        ));
        reg.register_control_plane_descriptor_with_owner(
            "shared.missing",
            &OwnerKind::RealmAuthority,
            &test_manifest(
                "shared.missing",
                "Unrelated RealmAuthority RPC descriptor must not rescue Device handler state.",
                serde_json::json!({"type": "object"}),
            ),
            DescriptorCallMode::Rpc,
            ReceiptSemantics::Operational,
            &ControlPlaneImplementation::native_daemon(),
        )
        .expect("unrelated RealmAuthority RPC descriptor registers");

        let err = reg
            .handler_control_plane_key("shared.missing")
            .expect_err("device handler must require its own exact authority/mode record");

        assert!(
            err.to_string().contains("no exact control-plane record"),
            "unexpected error: {err}"
        );
    }

    #[test]
    fn dynamic_runtime_key_validates_exact_authority_mode_record() {
        let reg = combined_catalog();
        hot_register_test_rpc(&reg, "dynamic.shared", OwnerKind::Device, ok_handler())
            .expect("dynamic device RPC registers");
        let device_key = reg
            .dynamic_control_plane_key("dynamic.shared")
            .expect("dynamic key lookup succeeds")
            .expect("dynamic device key exists");

        reg.register_control_plane_descriptor_with_owner(
            "dynamic.shared",
            &OwnerKind::RealmAuthority,
            &test_manifest(
                "dynamic.shared",
                "Same public name under an unrelated RealmAuthority Stream authority.",
                serde_json::json!({"type": "object"}),
            ),
            DescriptorCallMode::Stream,
            ReceiptSemantics::Operational,
            &ControlPlaneImplementation::native_daemon(),
        )
        .expect("unrelated Hub Stream descriptor registers");

        let got = reg
            .dynamic_control_plane_key("dynamic.shared")
            .expect("dynamic key lookup succeeds")
            .expect("runtime key derivation must validate only the device RPC tuple");

        assert_eq!(got, device_key);
    }

    #[test]
    fn ability_name_handler_projection_rejects_multi_authority_same_slot() {
        let mut reg = combined_catalog();
        register_test_rpc(
            &mut reg,
            "shared.rpc",
            OwnerKind::Device,
            Arc::new(|_| Ok(serde_json::json!({"owner": "device"}))),
        );
        register_test_rpc(
            &mut reg,
            "shared.rpc",
            OwnerKind::RealmAuthority,
            Arc::new(|_| Ok(serde_json::json!({"owner": "hub"}))),
        );

        assert!(
            reg.has_registered_handler("shared.rpc"),
            "execution index still records same-name handlers"
        );
        assert!(
            !reg.has_rpc("shared.rpc"),
            "same-name same-mode multi-authority handlers must not be routeable by bare ability name"
        );
        assert!(
            reg.resolve_rpc("shared.rpc").is_none(),
            "same-name same-slot handlers must fail closed instead of picking an arbitrary authority"
        );
    }

    #[test]
    fn ability_name_handler_projection_does_not_synthesize_cross_authority_runtime_set() {
        let mut reg = combined_catalog();
        register_test_rpc(
            &mut reg,
            "shared.modes",
            OwnerKind::Device,
            Arc::new(|_| Ok(serde_json::json!({"mode": "rpc"}))),
        );
        register_test_stream(
            &mut reg,
            "shared.modes",
            OwnerKind::RealmAuthority,
            Arc::new(|_| Ok(StreamSource::Snapshot(Vec::new()))),
        );

        assert!(
            reg.has_rpc("shared.modes"),
            "RPC projection is unique for the device authority"
        );
        assert!(
            reg.has_stream("shared.modes"),
            "Stream projection is unique for the hub authority"
        );
        let err = reg
            .handler_control_plane_key("shared.modes")
            .expect_err("cross-authority modes must not collapse into one runtime key");
        assert!(
            err.to_string()
                .contains("multiple Static execution authority keys"),
            "unexpected error: {err}"
        );
    }

    #[test]
    fn list_rpc_names_requires_rpc_control_plane_record() {
        let mut reg = combined_catalog();
        register_test_rpc(
            &mut reg,
            "agent.chat",
            OwnerKind::Agent("agent".to_string()),
            ok_handler(),
        );
        register_test_stream(
            &mut reg,
            "agent.chat",
            OwnerKind::Agent("agent".to_string()),
            Arc::new(|_| Ok(StreamSource::Snapshot(Vec::new()))),
        );
        assert!(reg.list_rpc_names().contains(&"agent.chat".to_string()));

        let rpc_key = reg
            .control_plane_record_for_mode("agent.chat", DescriptorCallMode::Rpc)
            .expect("RPC control-plane lookup is unambiguous")
            .expect("RPC control-plane record")
            .key()
            .clone();
        assert!(reg.remove_control_plane_record_for_authority_mode(
            rpc_key.authority_root(),
            rpc_key.ability(),
            rpc_key.call_mode(),
        ));

        assert!(
            !reg.list_rpc_names().contains(&"agent.chat".to_string()),
            "RPC names must come from committed RPC control-plane rows"
        );
        assert!(
            reg.list_abilities().contains(&"agent.chat".to_string()),
            "stream control-plane row still publishes the ability name"
        );
        assert!(reg.has_stream("agent.chat"));
    }

    #[test]
    fn authority_catalog_snapshot_projects_canonical_descriptors_in_one_pass() {
        let mut reg = combined_catalog();
        let manifest = test_manifest("read", "read a local file", json!({"type": "object"}));
        reg.register_rpc_with_spec("fs.read", OwnerKind::Device, manifest, ok_handler());
        register_test_bidi(
            &mut reg,
            "terminal.attach",
            OwnerKind::Device,
            trivial_bidi_handler(),
        );

        let rows = reg.authority_ability_catalog_snapshot();
        let fs = rows
            .iter()
            .find(|row| row.name == "fs.read")
            .expect("fs.read row");
        assert_eq!(fs.owner, OwnerKind::Device);
        assert_eq!(fs.descriptor.description, "read a local file");
        assert_eq!(fs.descriptor.input_schema(), &json!({"type": "object"}));
        let terminal = rows
            .iter()
            .find(|row| row.name == "terminal.attach")
            .expect("terminal.attach row");
        assert_eq!(terminal.owner, OwnerKind::Device);
        assert_eq!(terminal.descriptor.input_schema()["type"], "object");
        assert!(
            !terminal.descriptor.description.is_empty(),
            "static registrations must import canonical catalog metadata before commit"
        );
    }

    #[test]
    fn static_catalog_import_preserves_observation_public_visibility() {
        let mut reg = combined_catalog();
        reg.register_rpc_with_owner("observe.health", OwnerKind::Device, ok_handler());
        reg.register_rpc_with_owner("admin.status", OwnerKind::Device, ok_handler());

        let observe = reg
            .control_plane_record_for_mode("observe.health", DescriptorCallMode::Rpc)
            .expect("observe lookup")
            .expect("observe descriptor");
        assert_eq!(
            observe.descriptor().visibility,
            crate::daemon::ability::descriptors::Visibility::Public
        );
        assert!(observe.descriptor().hints.read_only);
        assert!(observe.descriptor().hints.idempotent);
        assert_eq!(observe.descriptor().call_mode(), DescriptorCallMode::Rpc);
        let admin = reg
            .control_plane_record_for_mode("admin.status", DescriptorCallMode::Rpc)
            .expect("admin lookup")
            .expect("admin descriptor");
        assert_eq!(
            admin.descriptor().visibility,
            crate::daemon::ability::descriptors::Visibility::Scoped
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
        let mut reg = combined_catalog();
        assert!(reg.get_bidi("terminal.attach").is_none());
        register_test_bidi(
            &mut reg,
            "terminal.attach",
            OwnerKind::Device,
            trivial_bidi_handler(),
        );
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
        let mut reg = combined_catalog();
        register_test_rpc(
            &mut reg,
            "observe.health",
            OwnerKind::Device,
            Arc::new(|_| Ok(Value::Null)),
        );
        register_test_stream(
            &mut reg,
            "permission.subscribe",
            OwnerKind::Device,
            Arc::new(|_| Ok(StreamSource::Snapshot(vec![]))),
        );
        register_test_bidi(
            &mut reg,
            "terminal.attach",
            OwnerKind::Device,
            trivial_bidi_handler(),
        );
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
        let mut reg = combined_catalog();

        // A handler that owns its own loop reading from_client and
        // echoing into to_client. Spawned inside the closure per §D2.
        register_test_bidi(
            &mut reg,
            "device.test.echo",
            OwnerKind::Device,
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
        let target =
            crate::daemon::invocation::routing::target::SystemInvocationTargetIssuer::local_root(
                "device.test.echo",
                json!({}),
                CallMode::Bidi,
            );

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
        let target =
            crate::daemon::invocation::routing::target::SystemInvocationTargetIssuer::local_root(
                "terminal.attach",
                json!({}),
                CallMode::Bidi,
            );
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
        let mut reg = combined_catalog();
        register_test_bidi(
            &mut reg,
            "device.test.bad",
            OwnerKind::Device,
            Arc::new(|_| anyhow::bail!("intentional handler failure: precondition foo missing")),
        );
        let dispatcher = Arc::new(reg);
        let target =
            crate::daemon::invocation::routing::target::SystemInvocationTargetIssuer::local_root(
                "device.test.bad",
                json!({}),
                CallMode::Bidi,
            );
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
        let target =
            crate::daemon::invocation::routing::target::SystemInvocationTargetIssuer::remote_root(
                NodeId::new("01PEER"),
                "terminal.attach",
                json!({}),
                CallMode::Bidi,
            );
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

    // ── M0 commit 1: OwnerKind round-trip ─────────────────────────

    fn ok_handler() -> LocalRpcHandler {
        Arc::new(|_args| Ok(json!({"ok": true})))
    }

    #[test]
    fn register_rpc_writes_control_plane_record() {
        let mut reg = combined_catalog();
        register_test_rpc(&mut reg, "fs.read", OwnerKind::Device, ok_handler());

        let record = reg
            .control_plane_record_for_mode("fs.read", DescriptorCallMode::Rpc)
            .expect("control-plane lookup is unambiguous")
            .expect("control-plane record");
        assert_eq!(record.descriptor().name, "fs.read");
        assert_eq!(record.descriptor().version, "1.0.0");
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
        assert_ne!(record.descriptor().schema_hash_bytes(), [0u8; 32]);
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
        let mut reg = combined_catalog();
        register_test_rpc(&mut reg, "fs.read", OwnerKind::Device, ok_handler());
        register_test_rpc(
            &mut reg,
            "federation.discover",
            OwnerKind::RealmAuthority,
            ok_handler(),
        );
        register_test_rpc(
            &mut reg,
            "codex.weather",
            OwnerKind::Agent("codex".to_string()),
            ok_handler(),
        );
        register_test_rpc(
            &mut reg,
            "codex.chat",
            OwnerKind::Agent("codex".to_string()),
            ok_handler(),
        );

        for (ability, expected) in [
            ("fs.read", OwnerKind::Device),
            ("federation.discover", OwnerKind::RealmAuthority),
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
        let mut reg = combined_catalog();
        register_test_rpc(&mut reg, "fs.read", OwnerKind::Device, ok_handler());

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
        let proof = options
            .proof_for_mode(AxonCallMode::Rpc)
            .expect("registered RPC carries descriptor proof");

        assert_eq!(
            proof.descriptor_version,
            record.descriptor().version.as_str()
        );
        assert_eq!(proof.schema_hash, record.descriptor().schema_hash_bytes());
        assert_eq!(proof.impl_hash, record.implementation().impl_hash());
        assert!(proof.is_bound());
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
        let mut reg = combined_catalog();
        let manifest = test_manifest(
            "versioned",
            "versioned test ability",
            json!({"type": "object"}),
        )
        .with_descriptor_version("2.0.0")
        .unwrap();
        reg.register_rpc_with_spec("test.versioned", OwnerKind::Device, manifest, ok_handler());

        let control_plane_key = reg
            .handler_control_plane_key("test.versioned")
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
        let proof = options
            .proof_for_mode(AxonCallMode::Rpc)
            .expect("versioned RPC carries descriptor proof");

        assert_eq!(record.descriptor().version, "2.0.0");
        assert_eq!(proof.descriptor_version, "2.0.0");
        assert_eq!(proof.schema_hash, record.descriptor().schema_hash_bytes());
        assert_eq!(proof.impl_hash, record.implementation().impl_hash());
    }

    #[test]
    fn control_plane_descriptor_owns_normalized_schema_and_description() {
        let mut reg = combined_catalog();
        let manifest = test_manifest(
            "read",
            "read a local file",
            json!({"type": "object", "properties": {"path": {"type": "string"}}}),
        );
        reg.register_rpc_with_spec("fs.read", OwnerKind::Device, manifest.clone(), ok_handler());
        register_test_rpc(&mut reg, "admin.status", OwnerKind::Device, ok_handler());

        let descriptor = reg
            .control_plane_record_for_mode("fs.read", DescriptorCallMode::Rpc)
            .expect("lookup succeeds")
            .expect("record exists")
            .descriptor()
            .clone();
        assert_eq!(
            descriptor.input_schema(),
            manifest.input_schema(),
            "manifest schema must be normalized into the canonical descriptor"
        );
        assert_eq!(descriptor.description, manifest.description());

        let imported = reg
            .control_plane_record_for_mode("admin.status", DescriptorCallMode::Rpc)
            .expect("lookup succeeds")
            .expect("record exists");
        assert_eq!(imported.descriptor().input_schema()["type"], "object");
        assert!(!imported.descriptor().description.is_empty());
    }

    #[test]
    fn control_plane_rebind_without_manifest_is_rejected() {
        let mut reg = combined_catalog();
        let manifest = test_manifest(
            "read",
            "read a local file",
            json!({"type": "object", "properties": {"path": {"type": "string"}}}),
        );
        reg.register_rpc_with_spec("fs.read", OwnerKind::Device, manifest, ok_handler());
        let original_schema_hash = reg
            .control_plane_record_for_mode("fs.read", DescriptorCallMode::Rpc)
            .expect("lookup succeeds")
            .expect("record exists")
            .descriptor()
            .schema_hash_bytes();

        let err = reg
            .rebind_control_plane_record(
                "fs.read",
                &OwnerKind::Device,
                None,
                DescriptorCallMode::Rpc,
                AbilityImplSource::NativeDaemon,
                RuntimeEnv::daemon_native(),
            )
            .expect_err("control-plane rebind without manifest must fail closed");
        assert!(err.to_string().contains("admission_action"), "{err}");

        let record = reg
            .control_plane_record_for_mode("fs.read", DescriptorCallMode::Rpc)
            .expect("lookup succeeds")
            .expect("record exists");
        assert_eq!(
            record.descriptor().schema_hash_bytes(),
            original_schema_hash,
            "failed rebind must leave the existing descriptor proof intact"
        );
    }

    #[test]
    fn control_plane_rebind_with_manifest_replaces_descriptor_schema() {
        let mut reg = combined_catalog();
        let manifest = test_manifest(
            "read",
            "read a local file",
            json!({"type": "object", "properties": {"path": {"type": "string"}}}),
        );
        reg.register_rpc_with_spec("fs.read", OwnerKind::Device, manifest, ok_handler());
        let rebound_manifest = test_manifest(
            "read",
            "read a local file by descriptor-bound id",
            json!({"type": "object", "properties": {"id": {"type": "string"}}}),
        );

        reg.rebind_control_plane_record(
            "fs.read",
            &OwnerKind::Device,
            Some(&rebound_manifest),
            DescriptorCallMode::Rpc,
            AbilityImplSource::NativeDaemon,
            RuntimeEnv::daemon_native(),
        )
        .expect("control-plane rebind with manifest succeeds");

        let rebound = reg
            .control_plane_record_for_mode("fs.read", DescriptorCallMode::Rpc)
            .expect("lookup succeeds")
            .expect("record exists");
        assert_eq!(
            rebound.descriptor().input_schema(),
            rebound_manifest.input_schema()
        );
        assert_eq!(
            rebound.descriptor().description,
            rebound_manifest.description()
        );
    }

    #[test]
    fn remove_control_plane_record_for_authority_mode_removes_descriptor_row() {
        let mut reg = combined_catalog();
        let manifest = test_manifest(
            "read",
            "read a local file",
            json!({"type": "object", "properties": {"path": {"type": "string"}}}),
        );
        reg.register_rpc_with_spec("fs.read", OwnerKind::Device, manifest, ok_handler());
        let record = reg
            .control_plane_record_for_mode("fs.read", DescriptorCallMode::Rpc)
            .expect("control-plane lookup succeeds")
            .expect("control-plane record");
        let key = record.key().clone();

        assert!(reg.remove_control_plane_record_for_authority_mode(
            key.authority_root(),
            key.ability(),
            key.call_mode(),
        ));
        assert!(
            reg.control_plane_record_for_authority_mode(
                key.authority_root(),
                key.ability(),
                key.call_mode(),
            )
            .expect("lookup remains valid")
            .is_none(),
            "authority-mode removal must delete the canonical descriptor row"
        );
    }

    #[test]
    fn control_plane_keeps_rpc_and_stream_records_for_same_ability() {
        let mut reg = AxonAbilityCatalog::new_with_runtime_and_authority_context(
            test_runtime(),
            AbilityAuthorityContext::for_device_authority_root("easynet:///r/localhost/device/dev")
                .expect("test device URA is a valid device authority root"),
        );
        let stream_handler: LocalStreamHandler =
            Arc::new(|_args| Ok(StreamSource::Snapshot(vec![])));

        register_test_rpc(
            &mut reg,
            "agent.chat",
            OwnerKind::Agent("agent".to_string()),
            ok_handler(),
        );
        register_test_stream(
            &mut reg,
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
        let snapshot = reg.authority_ability_catalog_snapshot();
        assert_eq!(
            snapshot
                .iter()
                .filter(|row| row.name == "agent.chat")
                .count(),
            2,
            "catalogue projection must preserve each committed call-mode record"
        );
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
        let rpc_proof = options
            .proof_for_mode(AxonCallMode::Rpc)
            .expect("RPC mode carries descriptor proof");
        let stream_proof = options
            .proof_for_mode(AxonCallMode::Stream)
            .expect("Stream mode carries descriptor proof");

        assert_eq!(
            rpc_proof.descriptor_version,
            rpc.descriptor().version.as_str()
        );
        assert_eq!(rpc_proof.schema_hash, rpc.descriptor().schema_hash_bytes());
        assert_eq!(rpc_proof.impl_hash, rpc.implementation().impl_hash());
        assert_eq!(
            stream_proof.descriptor_version,
            stream.descriptor().version.as_str()
        );
        assert_eq!(
            stream_proof.schema_hash,
            stream.descriptor().schema_hash_bytes()
        );
        assert_eq!(stream_proof.impl_hash, stream.implementation().impl_hash());
        assert_ne!(
            rpc_proof.impl_hash, stream_proof.impl_hash,
            "per-mode runtime proof bindings must not collapse RPC and Stream records"
        );
    }

    #[test]
    fn runtime_binding_facts_describe_daemon_binding_only() {
        let mut reg = combined_catalog();
        register_test_rpc(&mut reg, "fs.read", OwnerKind::Device, ok_handler());

        let record = reg
            .control_plane_record_for_mode("fs.read", DescriptorCallMode::Rpc)
            .expect("control-plane lookup is unambiguous")
            .expect("control-plane record");
        let facts = reg
            .runtime_binding_facts_for_mode("fs.read", DescriptorCallMode::Rpc)
            .expect("runtime binding lookup is unambiguous")
            .expect("runtime binding facts");

        assert_eq!(facts.descriptor_version, record.descriptor().version);
        assert_eq!(facts.call_mode, DescriptorCallMode::Rpc);
        assert_eq!(
            facts.schema_hash,
            record.descriptor().schema_hash_prefixed()
        );
        let descriptor_hash = record.descriptor().descriptor_hash_prefixed();
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
            test_runtime(),
            AbilityAuthorityContext::for_device_authority_root("easynet:///r/localhost/device/dev")
                .expect("test device URA is a valid device authority root"),
        );
        let stream_handler: LocalStreamHandler =
            Arc::new(|_args| Ok(StreamSource::Snapshot(vec![])));
        register_test_rpc(
            &mut reg,
            "agent.chat",
            OwnerKind::Agent("agent".to_string()),
            ok_handler(),
        );
        register_test_stream(
            &mut reg,
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
        let mut reg = combined_catalog();
        register_test_rpc(&mut reg, "fs.read", OwnerKind::Device, ok_handler());
        register_test_rpc(
            &mut reg,
            "federation.resolve_key",
            OwnerKind::RealmAuthority,
            ok_handler(),
        );
        register_test_rpc(
            &mut reg,
            "consent.decide",
            OwnerKind::Agent("consent".to_string()),
            ok_handler(),
        );
        register_test_rpc(
            &mut reg,
            "00000000-0000-0000-0000-000000000001.api_key.create",
            OwnerKind::User("00000000-0000-0000-0000-000000000001".to_string()),
            ok_handler(),
        );

        assert_eq!(reg.control_plane_owner("fs.read"), Some(OwnerKind::Device));
        assert_eq!(
            reg.control_plane_owner("federation.resolve_key"),
            Some(OwnerKind::RealmAuthority)
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
    fn owner_tracking_works_across_all_six_register_variants() {
        // M0 D4 decision: thread OwnerKind across every register
        // variant (rpc/stream/bidi × with-envelope/without).
        // Without this we'd ship sniffing fallbacks for the half-
        // covered variants — the same flat-namespace bug class
        // that the migration is closing.
        let mut reg = combined_catalog();

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

        register_test_rpc(&mut reg, "a.rpc", OwnerKind::RealmAuthority, ok_handler());
        register_test_stream(
            &mut reg,
            "a.stream",
            OwnerKind::Agent("codex".to_string()),
            stream_handler,
        );
        register_test_bidi(
            &mut reg,
            "a.bidi",
            OwnerKind::User("u-1".to_string()),
            bidi_handler,
        );
        register_test_rpc_env(&mut reg, "a.rpc.env", OwnerKind::Device, rpc_env);
        register_test_stream_env(
            &mut reg,
            "a.stream.env",
            OwnerKind::RealmAuthority,
            stream_env,
        );
        register_test_bidi_env(
            &mut reg,
            "a.bidi.env",
            OwnerKind::Agent("web-builder".to_string()),
            bidi_env,
        );

        assert_eq!(
            reg.control_plane_owner("a.rpc"),
            Some(OwnerKind::RealmAuthority)
        );
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
            Some(OwnerKind::RealmAuthority)
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
        let mut reg = combined_catalog();
        register_test_rpc(
            &mut reg,
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
        let mut reg = combined_catalog();
        register_test_rpc(
            &mut reg,
            "federation.discover",
            OwnerKind::RealmAuthority,
            Arc::new(|_args: Value| Ok(json!({}))),
        );
        // Canonical owner-local lookup returns the realm Authority.
        assert_eq!(
            reg.control_plane_owner("federation.discover"),
            Some(OwnerKind::RealmAuthority)
        );
        // A duplicated Hub alias is not registered.
        assert_eq!(
            reg.control_plane_owner("hub.federation.discover"),
            None,
            "retired Hub aliases must not be in the owner table"
        );
    }

    #[test]
    fn catalogue_lists_owner_local_names_only() {
        // RFC-005: `list_abilities()` returns public owner-local names. It must
        // not synthesize a duplicated `device.*` owner prefix.
        let mut reg = combined_catalog();
        register_test_rpc(
            &mut reg,
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
        let mut reg = combined_catalog();
        register_test_rpc(
            &mut reg,
            "device.x.foo",
            OwnerKind::Device,
            Arc::new(|_| Ok(json!({"who": "first"}))),
        );
        register_test_rpc(
            &mut reg,
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
        let mut reg = combined_catalog();
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

        register_test_rpc(
            &mut reg,
            "device.x.rpc",
            OwnerKind::Device,
            Arc::new(|_| Ok(json!({}))),
        );
        register_test_stream(
            &mut reg,
            "device.x.stream",
            OwnerKind::Device,
            stream_handler,
        );
        register_test_bidi(&mut reg, "device.x.bidi", OwnerKind::Device, bidi_handler);
        register_test_rpc_env(&mut reg, "device.x.rpc.env", OwnerKind::Device, rpc_env);
        register_test_stream_env(
            &mut reg,
            "device.x.stream.env",
            OwnerKind::Device,
            stream_env,
        );
        register_test_bidi_env(&mut reg, "device.x.bidi.env", OwnerKind::Device, bidi_env);

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
        let mut reg = combined_catalog();
        register_test_rpc(
            &mut reg,
            "doomed.tool",
            OwnerKind::Device,
            Arc::new(|_| Ok(json!("v"))),
        );
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
        let mut reg = combined_catalog();
        // Returns false but does not panic — the contract callers
        // (B4 list_changed refresh diff) rely on for the
        // "tool went away mid-sync" race.
        assert!(!reg
            .unregister("never-was-there")
            .expect("missing unregister is idempotent"));
    }

    #[test]
    fn unregister_removes_stream_and_bidi_handlers() {
        let mut reg = combined_catalog();
        register_test_stream(
            &mut reg,
            "doomed.stream",
            OwnerKind::Device,
            Arc::new(|_| Ok(StreamSource::Snapshot(vec![]))),
        );
        register_test_bidi(
            &mut reg,
            "doomed.bidi",
            OwnerKind::Device,
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
        let reg = Arc::new(combined_catalog());
        assert!(!reg.has_rpc("mcp_wikipedia__search"));
        assert!(reg.resolve_rpc("mcp_wikipedia__search").is_none());

        hot_register_test_rpc(
            &reg,
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
    fn hot_register_rpc_with_spec_publishes_canonical_descriptor() {
        let reg = Arc::new(combined_catalog());
        let manifest = test_manifest(
            "search",
            "Search Wikipedia.",
            serde_json::json!({"type": "object"}),
        );
        reg.hot_register_rpc_with_spec(
            "mcp_wikipedia__search",
            OwnerKind::Agent("mcp".to_string()),
            manifest,
            Arc::new(|_args| Ok(serde_json::json!({}))),
        )
        .expect("dynamic RPC with manifest registers");

        let record = reg
            .control_plane_record_for_mode("mcp_wikipedia__search", DescriptorCallMode::Rpc)
            .expect("lookup succeeds")
            .expect("hot-registered descriptor visible in control-plane");
        assert_eq!(record.descriptor().description, "Search Wikipedia.");
        assert_eq!(
            record.descriptor().input_schema(),
            &serde_json::json!({"type": "object"})
        );
    }

    #[test]
    fn hot_register_stream_with_explicit_impl_writes_control_plane_once() {
        let reg = Arc::new(combined_catalog());
        let manifest = test_manifest(
            "search",
            "Search Wikipedia.",
            serde_json::json!({"type": "object"}),
        );

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
        let reg = Arc::new(combined_catalog());
        hot_register_test_rpc(
            &reg,
            "plugin.mode_shift",
            OwnerKind::Device,
            Arc::new(|_args| Ok(serde_json::json!({"mode": "rpc"}))),
        )
        .expect("dynamic RPC registers");
        assert!(reg.has_rpc("plugin.mode_shift"));
        assert!(!reg.has_stream("plugin.mode_shift"));

        let manifest = test_manifest(
            "mode_shift",
            "Mode-shift test ability.",
            serde_json::json!({"type": "object"}),
        );
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
        assert!(
            reg.has_dynamic("plugin.mode_shift"),
            "dynamic execution row remains present after adding a second mode"
        );
    }

    #[test]
    fn hot_register_rejects_dynamic_owner_migration_without_unregister() {
        let reg = Arc::new(combined_catalog());
        hot_register_test_rpc(
            &reg,
            "plugin.owner_shift",
            OwnerKind::Device,
            Arc::new(|_args| Ok(serde_json::json!({"owner": "device"}))),
        )
        .expect("initial dynamic RPC registers");

        let err = reg
            .hot_register_rpc_with_spec(
                "plugin.owner_shift",
                OwnerKind::Agent("mcp".to_string()),
                test_manifest(
                    "plugin.owner_shift",
                    "Owner-shift test ability.",
                    serde_json::json!({"type": "object"}),
                ),
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
        let out = invoke_test_rpc(&reg, "plugin.owner_shift", serde_json::json!({}))
            .expect("old runtime binding remains invokable");
        assert_eq!(out, serde_json::json!({"owner": "device"}));
    }

    #[test]
    fn hot_register_replaces_same_dynamic_handler_family() {
        let reg = Arc::new(combined_catalog());
        hot_register_test_rpc(
            &reg,
            "plugin.reload",
            OwnerKind::Device,
            Arc::new(|_args| Ok(serde_json::json!({"version": "old"}))),
        )
        .expect("initial dynamic RPC registers");

        hot_register_test_rpc(
            &reg,
            "plugin.reload",
            OwnerKind::Device,
            Arc::new(|_args| Ok(serde_json::json!({"version": "new"}))),
        )
        .expect("same dynamic handler family can be replaced");

        let out = invoke_test_rpc(&reg, "plugin.reload", serde_json::json!({}))
            .expect("reloaded runtime binding remains invokable");
        assert_eq!(out, serde_json::json!({"version": "new"}));
        assert!(
            reg.resolve_rpc_with_env("plugin.reload").is_none(),
            "same-family replacement must not create an envelope handler"
        );
    }

    #[test]
    fn hot_register_rejects_dynamic_handler_family_switch_without_unregister() {
        let reg = Arc::new(combined_catalog());
        hot_register_test_rpc(
            &reg,
            "plugin.family_shift",
            OwnerKind::Device,
            Arc::new(|_args| Ok(serde_json::json!({"family": "rpc"}))),
        )
        .expect("initial dynamic RPC registers");

        let err = reg
            .hot_register_rpc_with_envelope_and_spec(
                "plugin.family_shift",
                OwnerKind::Device,
                test_manifest(
                    "plugin.family_shift",
                    "Rejected handler-family switch test ability.",
                    serde_json::json!({"type": "object"}),
                ),
                Arc::new(|_env, _args| Ok(serde_json::json!({"family": "rpc_with_env"}))),
            )
            .expect_err("in-place dynamic handler family switch is rejected");
        assert!(err.to_string().contains("handler family"), "{err}");

        assert!(
            reg.resolve_rpc_with_env("plugin.family_shift").is_none(),
            "rejected family switch must not install an envelope handler"
        );
        let out = invoke_test_rpc(&reg, "plugin.family_shift", serde_json::json!({}))
            .expect("old runtime binding remains invokable");
        assert_eq!(out, serde_json::json!({"family": "rpc"}));
    }

    #[test]
    fn control_plane_authority_mode_transaction_restores_prior_slice() {
        let catalog = combined_catalog();
        let old_manifest = test_manifest(
            "txn",
            "Old transactional descriptor.",
            serde_json::json!({"type": "object", "properties": {"old": {"type": "boolean"}}}),
        );
        let authority_scope = catalog
            .resolve_authority_scope_for_owner("plugin.txn", &OwnerKind::Device)
            .expect("device owner resolves authority scope");
        let control_plane_key =
            ControlPlaneAbilityKey::new(authority_scope.authority_root(), "plugin.txn")
                .for_mode(DescriptorCallMode::Rpc);
        catalog
            .register_dynamic_control_plane_with_scope_and_semantics_result(
                "plugin.txn",
                authority_scope.clone(),
                Some(&old_manifest),
                DescriptorCallMode::Rpc,
                ReceiptSemantics::Operational,
                &ControlPlaneImplementation::native_daemon(),
            )
            .expect("old control-plane record writes");
        let old_schema_hash = catalog
            .control_plane_record_for_mode("plugin.txn", DescriptorCallMode::Rpc)
            .expect("old lookup succeeds")
            .expect("old record exists")
            .descriptor()
            .schema_hash_bytes();

        let mut txn = catalog.begin_control_plane_authority_mode_transaction(
            control_plane_key.authority_root(),
            control_plane_key.ability(),
            control_plane_key.call_mode(),
        );
        let new_manifest = test_manifest(
            "txn",
            "New transactional descriptor.",
            serde_json::json!({"type": "object", "properties": {"new": {"type": "boolean"}}}),
        );
        catalog
            .register_dynamic_control_plane_with_scope_and_semantics_result(
                "plugin.txn",
                authority_scope,
                Some(&new_manifest),
                DescriptorCallMode::Rpc,
                ReceiptSemantics::Operational,
                &ControlPlaneImplementation::native_daemon(),
            )
            .expect("new control-plane record writes");
        assert_ne!(
            catalog
                .control_plane_record_for_mode("plugin.txn", DescriptorCallMode::Rpc)
                .expect("new lookup succeeds")
                .expect("new record exists")
                .descriptor()
                .schema_hash_bytes(),
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
                .schema_hash_bytes(),
            old_schema_hash,
            "rollback must restore old descriptor facts"
        );
    }

    #[test]
    fn dynamic_registration_rollback_restores_prior_snapshot() {
        let catalog = combined_catalog();
        let old_manifest = test_manifest(
            "rollback",
            "Old rollback handler.",
            serde_json::json!({"type": "object", "properties": {"old": {"type": "boolean"}}}),
        );
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
            .schema_hash_bytes();

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

        let new_manifest = test_manifest(
            "rollback",
            "New rollback handler.",
            serde_json::json!({"type": "object", "properties": {"new": {"type": "boolean"}}}),
        );
        let new_authority_scope = catalog
            .resolve_authority_scope_for_owner("plugin.rollback", &OwnerKind::Device)
            .expect("device owner resolves authority scope");
        let written_key = catalog
            .register_dynamic_control_plane_with_scope_and_semantics_result(
                "plugin.rollback",
                new_authority_scope,
                Some(&new_manifest),
                DescriptorCallMode::Rpc,
                ReceiptSemantics::Operational,
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
            .schema_hash_bytes();
        assert_eq!(
            restored_schema_hash, old_schema_hash,
            "rollback must restore the previous descriptor proof, not leave the failed write"
        );

        let restored_record = catalog
            .control_plane_record_for_mode("plugin.rollback", DescriptorCallMode::Rpc)
            .expect("restored descriptor lookup succeeds")
            .expect("rollback must restore the prior descriptor");
        let props = restored_record
            .descriptor()
            .input_schema()
            .get("properties")
            .and_then(|p| p.as_object())
            .expect("descriptor input_schema has properties");
        assert!(
            props.contains_key("old") && !props.contains_key("new"),
            "rollback must restore the old descriptor schema; got {:?}",
            restored_record.descriptor().input_schema()
        );
    }

    #[test]
    fn hot_unregister_removes_dynamic_entry_without_touching_static() {
        // Diff-aware refresh writes `hot_unregister` for tools that
        // disappeared from the upstream catalogue. Static entries
        // (boot-registered system abilities) must never be touched
        // by this surface — if a future bug routes a static name
        // through `hot_unregister`, the static entry survives.
        let mut reg = combined_catalog();
        register_test_rpc(
            &mut reg,
            "fs.read",
            OwnerKind::Device,
            Arc::new(|_args| Ok(serde_json::Value::Null)),
        );
        let reg = Arc::new(reg);

        hot_register_test_rpc(
            &reg,
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
    fn hot_unregister_removes_dynamic_control_plane_descriptor() {
        let reg = Arc::new(combined_catalog());
        let manifest = test_manifest(
            "search",
            "Search a hot MCP index.",
            json!({"type": "object", "properties": {"query": {"type": "string"}}}),
        );
        reg.hot_register_rpc_with_spec(
            "mcp_wikipedia__search",
            OwnerKind::Agent("mcp".to_string()),
            manifest,
            Arc::new(|_args| Ok(serde_json::Value::Null)),
        )
        .expect("dynamic RPC registers");
        assert!(
            reg.authority_ability_catalog_snapshot()
                .iter()
                .any(|row| row.name == "mcp_wikipedia__search"),
            "test setup must publish a canonical dynamic descriptor"
        );

        assert!(reg
            .hot_unregister("mcp_wikipedia__search")
            .expect("dynamic RPC unregisters"));

        assert!(
            !reg.authority_ability_catalog_snapshot()
                .iter()
                .any(|row| row.name == "mcp_wikipedia__search"),
            "hot_unregister must remove the dynamic control-plane descriptor"
        );
    }

    #[test]
    fn list_abilities_unions_static_and_dynamic_names() {
        // `meta.list_abilities` is the catalogue surface backing
        // EasyNet-Frontend / the Codex / Claude Code surface. A
        // freshly reflected MCP tool MUST show up here without a
        // restart — that's the user-visible payoff for the listener
        // + dynamic side combined.
        let mut reg = combined_catalog();
        register_test_rpc(
            &mut reg,
            "fs.read",
            OwnerKind::Device,
            Arc::new(|_args| Ok(serde_json::Value::Null)),
        );
        let reg = Arc::new(reg);
        hot_register_test_rpc(
            &reg,
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
        let mut reg = combined_catalog();
        register_test_rpc(
            &mut reg,
            "fs.read",
            OwnerKind::Device,
            Arc::new(|_args| Ok(serde_json::Value::Null)),
        );
        let reg = Arc::new(reg);
        hot_register_test_rpc(
            &reg,
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
        let mut reg = combined_catalog();
        register_test_rpc(
            &mut reg,
            "fs.read",
            OwnerKind::Device,
            Arc::new(|_args| Ok(serde_json::json!({"from": "static"}))),
        );
        let reg = Arc::new(reg);
        let err = reg
            .hot_register_rpc_with_spec(
                "fs.read",
                OwnerKind::Agent("mcp".to_string()),
                test_manifest(
                    "fs.read",
                    "Collision test ability.",
                    serde_json::json!({"type": "object"}),
                ),
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
        let mut reg = combined_catalog();
        reg.register_rpc_with_spec(
            "device.keyring.sign",
            OwnerKind::Device,
            test_manifest(
                "device.keyring.sign",
                "Static collision test ability.",
                serde_json::json!({"type": "object"}),
            ),
            Arc::new(|_args| Ok(serde_json::json!({"from": "static-runtime"}))),
        );
        let reg = Arc::new(reg);

        let err = reg
            .hot_register_rpc_with_spec(
                "device.keyring.sign",
                OwnerKind::Agent("malicious-plugin".to_string()),
                test_manifest(
                    "device.keyring.sign",
                    "Collision test ability.",
                    serde_json::json!({"type": "object"}),
                ),
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
        let out = invoke_test_rpc(&reg, "device.keyring.sign", serde_json::json!({}))
            .expect("static runtime handler remains invokable");
        assert_eq!(
            out,
            serde_json::json!({"from": "static-runtime"}),
            "LocalRuntime must continue routing to the boot-registered handler"
        );
    }

    #[test]
    fn hot_remove_runtime_ability_rejects_static_catalog_names() {
        let mut reg = combined_catalog();
        reg.register_rpc_with_spec(
            "device.keyring.sign",
            OwnerKind::Device,
            test_manifest(
                "device.keyring.sign",
                "Static removal test ability.",
                serde_json::json!({"type": "object"}),
            ),
            Arc::new(|_args| Ok(serde_json::json!({"from": "static-runtime"}))),
        );
        let reg = Arc::new(reg);

        assert!(
            !reg.hot_remove_runtime_ability("device.keyring.sign")
                .expect("static hot-remove is a no-op"),
            "dynamic removal API must reject static catalogue names"
        );
        let out = invoke_test_rpc(&reg, "device.keyring.sign", serde_json::json!({}))
            .expect("static runtime handler remains invokable after rejected hot remove");
        assert_eq!(
            out,
            serde_json::json!({"from": "static-runtime"}),
            "static catalogue and LocalRuntime must stay in sync"
        );
    }
}
