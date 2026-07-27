// EasyNet CLI - daemon Invocation presence state
// =================================================
//
// File: src/daemon/invocation/state/presence.rs
// Description: In-memory, sharded, broadcast-equipped registry of
//              live `session.open` reverse channels keyed by
//              caller URA. This is the hub-side source of session liveness.
//
// Why this module exists
// ----------------------
// The pre-RFC-003 daemon learnt liveness from periodic
// `federation.heartbeat` unary calls — a polling model that drifted
// out of sync with the actual transport (devices could be reachable
// while heartbeats failed and vice versa). RFC-003 collapses
// liveness onto the transport itself: a device is *alive* exactly
// when its `session.open` `InvokeBidi` stream is open. When the
// stream closes for any reason, the registry drops the entry and
// emits an `Offline` event consumed by directory projections, dispatch
// correlation, and audit logging.
//
// This module is the single canonical home for that state. PR-1
// spec §3 (`pr-drafts/PR-0-spec-daemon-invocation-server.md`) pins
// the surface; PR-2 will populate it from the real `session.open`
// accept handler; PR-1 lands the structure plus its tests so PR-2
// reviewers can read it standing still.
//
// Public surface
// --------------
// - `PresenceRegistry` — the registry itself, owned by the daemon
//   `DaemonInvocationService` via `Arc`
// - `DispatchSender` — per-device queue for typed session frames
// - `PresenceEvent` — what subscribers receive; either `Online` or
//   `Offline`, with the URA plus (for Offline) a typed reason
// - `OfflineReason` — disjoint reasons a session is removed
// - Capacity constants (`PRESENCE_EVENT_CHANNEL_CAPACITY`,
//   `DISPATCH_CHANNEL_CAPACITY`, `presence_registry_shards()`)
//   exposed at module top so call sites and operators can refer to
//   them by name; rationale lives next to each definition
//
// Invariants
// ----------
// 1. **URA = key**. The registry keys on the *caller-claimed*
//    `easynet:///r/{tenant_id}/agent/{node_id}` URA from the
//    EnvelopeOpen first frame's signed envelope. There is no
//    hub-minted `agent/a-X` shadow identity in the new architecture
//    (spec §5.1 URA scheme migration).
// 2. **Lifecycle = liveness**. A device is "online" exactly when
//    `by_ura.contains_key(ura)` is true. Removal MUST emit an
//    `Offline` event before the entry is gone from the map; the
//    method order in `remove` enforces this so an observer that
//    races a snapshot against an event sees consistent state.
// 3. **Single live entry per URA**. `insert` returning the prior
//    sender means one device displaced the other; the displaced
//    device receives no notification because its mpsc is dropped
//    (the sender end goes away), which the receiver task observes
//    as `None` and treats as `OfflineReason::StreamClosed`.
// 4. **Bounded backpressure**. The per-device mpsc has capacity
//    `DISPATCH_CHANNEL_CAPACITY = 256`. A slow consumer cannot
//    accumulate frames in the hub's memory; dispatch paths handle
//    `try_send` failure
//    by removing the slow device with `OfflineReason::SendFailed`,
//    which collapses backpressure into a presence event.
// 5. **Lossy broadcast**. The events broadcast channel has capacity
//    `PRESENCE_EVENT_CHANNEL_CAPACITY = 1024`. Slow subscribers
//    that lag receive a `RecvError::Lagged(n)` and re-snapshot the
//    registry — cheaper than guaranteeing every subscriber sees
//    every transition.
//
// Author: Silan Hu <silan.hu@u.nus.edu>
// Copyright (c) 2026 EasyNet. All rights reserved.

use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::Arc;

use dashmap::DashMap;
use tokio::sync::{broadcast, mpsc};

/// Stable terminal reason when no live session owns the selected target.
pub const DISPATCH_TARGET_OFFLINE_REASON: &str = "target_offline";

/// Stable retryable reason when the selected session's bounded queue is full.
pub const DISPATCH_TARGET_BUSY_REASON: &str = "target_busy_retry";

/// Capacity of the cross-cutting `events` broadcast channel.
///
/// Sized for ~17 events/s sustained (1000 devices reconnecting at 1
/// per minute). 1024 entries buffer ~60 seconds of subscriber stall
/// before subscribers receive `RecvError::Lagged` and recover via
/// `snapshot()`. Re-snapshot is cheap: O(N_devices) read on a
/// shard-locked DashMap.
pub const PRESENCE_EVENT_CHANNEL_CAPACITY: usize = 1024;

/// Capacity of the per-device dispatch mpsc channel.
///
/// Matches the MVP's `STREAM_CHANNEL_CAPACITY`. Bounded so a slow
/// device cannot accumulate frames in hub memory; senders treat
/// `try_send`-full as `OfflineReason::SendFailed` and surface
/// presence-state transition rather than block.
pub const DISPATCH_CHANNEL_CAPACITY: usize = 256;

/// Number of DashMap shards.
///
/// Function rather than `const` because `num_cpus::get` reads OS
/// state at call time. The target is `num_cpus * 4`, rounded up to
/// the nearest power of two (DashMap requires a power-of-two shard
/// count). On a 10-core host the target is 40, the actual value 64;
/// on a 4-core host the target is 16, the actual value 16.
///
/// Tuning rationale follows DashMap's own recommendation: enough
/// shards that sub-millisecond P99 lookups hold even with thousands
/// of concurrent inserts/removes, not so many that memory overhead
/// dominates.
#[must_use]
pub fn presence_registry_shards() -> usize {
    let target = num_cpus::get().saturating_mul(4).max(1);
    target.next_power_of_two()
}

/// Sender end of the per-device mpsc carrying reverse-channel frames. The
/// receiving end is held by the
/// device's `session.open` task.
///
/// Frames are tonic results so a stream-level error (e.g., admission
/// gate revocation) can propagate to the device cleanly. The receiver
/// converts these into the outbound `InvokeBidiDown` stream.
pub type DispatchSender = mpsc::Sender<Result<DispatchFrame, tonic::Status>>;

/// Monotonic identity of one admitted `session.open`.
///
/// Session ids are process-local and exist only to let stale tasks
/// prove which registry entry they own when removing after a race
/// with displacement/reconnect.
pub type PresenceSessionId = u64;

/// Session down-frame scheduling class.
///
/// The reverse channel carries both data-plane ability replies and
/// control-plane frames that unblock runtime admission work such as
/// session Request/RequestResult trust sync. Those control frames must
/// not sit behind a burst of large payload replies for unrelated
/// call_ids; otherwise admission can time out while the answer is
/// already queued. Priority is transport-local scheduling metadata, not
/// an Invocation tuple field.
#[derive(Debug, Clone, Copy, Eq, PartialEq)]
pub enum DispatchPriority {
    Normal,
    Control,
}

/// One frame heading down a `session.open` reverse channel.
///
/// A thin newtype around the proto-generated `InvokeBidiDown` so
/// downstream call paths describe their pushes in terms of presence
/// vocabulary rather than raw proto types.
#[derive(Debug, Clone)]
pub struct DispatchFrame {
    /// The proto-encoded bidirectional-down frame. Owned so the
    /// dispatcher does not hold references across the channel
    /// boundary.
    pub frame: axon_sdk::pb::axon::v1::InvokeBidiDown,
    pub priority: DispatchPriority,
}

impl DispatchFrame {
    #[must_use]
    pub fn normal(frame: axon_sdk::pb::axon::v1::InvokeBidiDown) -> Self {
        Self {
            frame,
            priority: DispatchPriority::Normal,
        }
    }

    #[must_use]
    pub fn control(frame: axon_sdk::pb::axon::v1::InvokeBidiDown) -> Self {
        Self {
            frame,
            priority: DispatchPriority::Control,
        }
    }

    #[must_use]
    pub fn is_control(&self) -> bool {
        self.priority == DispatchPriority::Control
    }
}

/// Reason a session was removed from the registry. Distinct variants
/// so consumers (audit log, presence event subscribers) can filter
/// or alert on specific failure modes.
#[derive(Debug, Clone, Copy, Eq, PartialEq)]
pub enum OfflineReason {
    /// Device closed the stream gracefully (CloseSend on the bidi
    /// up channel, or transport-level FIN observed).
    StreamClosed,

    /// Stream was reset abruptly (RST_STREAM, transport error,
    /// network failure on the device side, SIGKILL of the device
    /// process). Distinguished from `StreamClosed` because reset
    /// often warrants a warning log line where graceful close does
    /// not.
    StreamReset,

    /// `try_send` on the per-device mpsc failed because the channel
    /// was full and the device was not draining frames. Treated as
    /// liveness failure: a healthy device must consume the bounded
    /// channel.
    SendFailed,

    /// Operator (or `federation.revoke` ability handler) explicitly
    /// removed the device. Triggered by `force_revoke`.
    AdminRevoked,
}

impl OfflineReason {
    /// Stable snake_case wire label for op-event fields and audit
    /// records. Pinned so a `grep kind=presence_offline_cancel
    /// reason=stream_closed` stays stable across Rust toolchain
    /// upgrades that could in principle alter Debug rendering.
    #[must_use]
    pub fn as_wire_str(self) -> &'static str {
        match self {
            OfflineReason::StreamClosed => "stream_closed",
            OfflineReason::StreamReset => "stream_reset",
            OfflineReason::SendFailed => "send_failed",
            OfflineReason::AdminRevoked => "admin_revoked",
        }
    }
}

impl std::fmt::Display for OfflineReason {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(self.as_wire_str())
    }
}

/// Event emitted on the broadcast channel whenever a session goes
/// online or offline. Subscribers (subscribe_directory_v2 pumps, audit
/// log writers) drive their state from this stream.
#[derive(Debug, Clone)]
pub enum PresenceEvent {
    /// A new `session.open` was accepted for the given URA. If a
    /// previous session existed for the same URA it is implicitly
    /// offline (this event was preceded by an `Offline` for the
    /// displaced sender — the registry guarantees the order).
    Online {
        /// Caller-claimed URA keyed in the registry.
        ura: String,
    },

    /// A previously-online session is now offline. The reason
    /// indicates how the registry learnt of the loss.
    Offline {
        /// Caller-claimed URA keyed in the registry.
        ura: String,

        /// How the session ended.
        reason: OfflineReason,
    },
}

#[derive(Debug, Clone)]
enum PresenceSlot {
    Dispatch(PresenceDispatchSlot),
    ResolveOnly(PresenceResolveOnlySlot),
}

#[derive(Debug, Clone)]
struct PresenceDispatchSlot {
    session_id: PresenceSessionId,
    sender: DispatchSender,
    /// Canonical carrier contract the device declared on frame 0 (DEC-F004).
    contract: SessionContract,
    /// Trust evidence attached to the admission decision that
    /// created this live slot. This is not a second liveness map:
    /// it is metadata on the one canonical presence row.
    trust: SessionTrustContext,
}

#[derive(Debug, Clone)]
struct PresenceResolveOnlySlot {
    session_id: PresenceSessionId,
}

impl PresenceSlot {
    fn session_id(&self) -> PresenceSessionId {
        match self {
            Self::Dispatch(slot) => slot.session_id,
            Self::ResolveOnly(slot) => slot.session_id,
        }
    }

    fn sender(&self) -> Option<DispatchSender> {
        match self {
            Self::Dispatch(slot) => Some(slot.sender.clone()),
            Self::ResolveOnly(_) => None,
        }
    }

    fn claimant_boot_nonce(&self) -> Option<Vec<u8>> {
        match self {
            Self::Dispatch(slot) => Some(slot.contract.claimant_boot_nonce.clone()),
            Self::ResolveOnly(_) => None,
        }
    }

    fn dispatch_session(&self) -> Option<PresenceDispatchSession> {
        match self {
            Self::Dispatch(slot) => Some(PresenceDispatchSession {
                session_id: slot.session_id,
                sender: slot.sender.clone(),
                contract_version: slot.contract.version,
            }),
            Self::ResolveOnly(_) => None,
        }
    }

    fn matches_admitted_key(&self, public_key_b64: &str) -> bool {
        match self {
            Self::Dispatch(slot) => slot.trust.matches_admitted_key(public_key_b64),
            Self::ResolveOnly(_) => false,
        }
    }
}

/// Frame-0 session negotiation facts (DEC-F004 / mini-RFC §2): the
/// dispatch contract version the claimant declared plus its per-boot
/// claimant fingerprint (T1.2). One value object so registration
/// sites cannot pass half the negotiation.
pub const CANONICAL_SESSION_CARRIER_VERSION: u32 = 1;

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SessionContract {
    pub version: u32,
    pub claimant_boot_nonce: Vec<u8>,
}

impl SessionContract {
    pub fn new(version: u32, claimant_boot_nonce: Vec<u8>) -> Self {
        Self {
            version,
            claimant_boot_nonce,
        }
    }
}

/// Runtime-trust evidence for one admitted `session.open`.
///
/// User URAs can carry multiple active public keys. The admission
/// gate pins the presented key before accepting a user session; the
/// presence slot stores that admitted key so a later
/// `identity.revoke_user_pubkey` disconnects only a session that was
/// actually admitted by the revoked key.
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct SessionTrustContext {
    admitted_public_key_b64: Option<String>,
}

impl SessionTrustContext {
    #[must_use]
    pub fn user_pubkey(public_key_b64: impl Into<String>) -> Self {
        let public_key_b64 = public_key_b64.into().trim().to_string();
        if public_key_b64.is_empty() {
            return Self::default();
        }
        Self {
            admitted_public_key_b64: Some(public_key_b64),
        }
    }

    #[must_use]
    pub fn admitted_public_key_b64(&self) -> Option<&str> {
        self.admitted_public_key_b64.as_deref()
    }

    #[must_use]
    fn matches_admitted_key(&self, public_key_b64: &str) -> bool {
        self.admitted_public_key_b64()
            .is_some_and(|admitted| admitted == public_key_b64.trim())
    }
}

#[derive(Debug, Clone)]
pub struct PresenceRegistration {
    pub session_id: PresenceSessionId,
    pub displaced: Option<DispatchSender>,
    /// T1.2: the prior claimant's boot nonce when this registration
    /// displaced a live slot. Empty nonce means the prior did not publish a
    /// claimant fingerprint. `None` = nothing was displaced. A displacement whose
    /// nonce differs from the new claimant's is a claimant conflict
    /// (two processes fighting over one URA), not a same-device
    /// restart.
    pub displaced_claimant_nonce: Option<Vec<u8>>,
}

/// Atomic snapshot of one live reverse-dispatch session.
///
/// Sender identity and carrier version must come from the same presence slot.
/// Reading them through separate registry lookups can combine a displaced
/// sender with its replacement's negotiated contract.
#[derive(Clone, Debug)]
pub struct PresenceDispatchSession {
    pub session_id: PresenceSessionId,
    pub sender: DispatchSender,
    pub contract_version: u32,
}

/// Concurrent registry of live `session.open` reverse channels.
///
/// The registry is the single owner of the mapping
/// `device URA -> DispatchSender` and the canonical source of
/// presence transition events for every consumer. Construct one
/// per daemon process and pass it around by `Arc`.
#[derive(Debug)]
pub struct PresenceRegistry {
    /// Sharded map keyed by caller-claimed URA. Outer `Arc` makes
    /// the entire registry cheap to clone for handler tasks.
    by_ura: Arc<DashMap<String, PresenceSlot>>,

    /// Monotonic session id allocator.
    next_session_id: AtomicU64,

    /// Cross-cutting transitions broadcast. New subscribers receive
    /// only events from `subscribe_events`-time onward; bootstrap
    /// is via `snapshot`.
    events: broadcast::Sender<PresenceEvent>,
}

impl PresenceRegistry {
    /// Construct a registry with default tunings
    /// (`PRESENCE_EVENT_CHANNEL_CAPACITY`,
    /// `presence_registry_shards()`).
    #[must_use]
    pub fn new() -> Self {
        Self::with_capacity(PRESENCE_EVENT_CHANNEL_CAPACITY)
    }

    /// Construct a registry with an explicit broadcast capacity.
    /// Production daemons use the default; tests and benchmarks
    /// may want a smaller capacity to drive `Lagged` paths
    /// deterministically.
    #[must_use]
    pub fn with_capacity(event_capacity: usize) -> Self {
        let (tx, _rx) = broadcast::channel(event_capacity);
        Self {
            by_ura: Arc::new(DashMap::with_shard_amount(presence_registry_shards())),
            next_session_id: AtomicU64::new(1),
            events: tx,
        }
    }

    /// Register a resolve-only presence row for `ura`.
    ///
    /// This is intentionally not a dispatch session. It exists for
    /// daemon-owned directory visibility such as device-mode self presence,
    /// where invocation execution must stay on LocalRuntime.
    pub fn insert_resolve_only(&self, ura: String) -> Result<PresenceRegistration, String> {
        validate_presence_principal_ura(&ura)?;
        let session_id = self.next_session_id.fetch_add(1, Ordering::Relaxed);
        let prior = self.by_ura.insert(
            ura.clone(),
            PresenceSlot::ResolveOnly(PresenceResolveOnlySlot { session_id }),
        );
        let displaced = prior.as_ref().and_then(PresenceSlot::sender);
        if prior.is_some() {
            self.emit_offline(&ura, OfflineReason::StreamClosed);
        }
        self.emit_online(ura);
        Ok(PresenceRegistration {
            session_id,
            displaced,
            displaced_claimant_nonce: prior.and_then(|slot| slot.claimant_boot_nonce()),
        })
    }

    /// Test/fixture-only dispatch registration for callers that do not exercise
    /// frame-0 negotiation.
    ///
    /// Production session.open paths must call `insert_negotiated*` with the
    /// carrier contract obtained from the admitted runtime context.
    #[cfg(any(test, feature = "demo-fixture"))]
    pub fn insert(
        &self,
        ura: String,
        sender: DispatchSender,
    ) -> Result<Option<DispatchSender>, String> {
        Ok(self.insert_fixture_dispatch(ura, sender)?.displaced)
    }

    /// Test/fixture-only dispatch registration returning the registry-owned
    /// `session_id` alongside any displaced prior sender.
    #[cfg(any(test, feature = "demo-fixture"))]
    pub fn insert_tracked(
        &self,
        ura: String,
        sender: DispatchSender,
    ) -> Result<PresenceRegistration, String> {
        self.insert_fixture_dispatch(ura, sender)
    }

    #[cfg(any(test, feature = "demo-fixture"))]
    pub fn insert_fixture_dispatch(
        &self,
        ura: String,
        sender: DispatchSender,
    ) -> Result<PresenceRegistration, String> {
        self.insert_negotiated(
            ura,
            sender,
            SessionContract::new(CANONICAL_SESSION_CARRIER_VERSION, vec![0; 16]),
        )
    }

    /// Register a new `session.open` carrying the frame-0 carrier
    /// negotiation facts (DEC-F004). The slot remembers the declared
    /// contract so the hub's dispatch write path can pick the frame
    /// encoding per device, and the prior claimant's fingerprint is
    /// surfaced for T1.2 conflict classification.
    pub fn insert_negotiated(
        &self,
        ura: String,
        sender: DispatchSender,
        contract: SessionContract,
    ) -> Result<PresenceRegistration, String> {
        self.insert_negotiated_with_trust(ura, sender, contract, SessionTrustContext::default())
    }

    /// Register a negotiated `session.open` together with the
    /// trust evidence admitted for that session.
    pub fn insert_negotiated_with_trust(
        &self,
        ura: String,
        sender: DispatchSender,
        contract: SessionContract,
        trust: SessionTrustContext,
    ) -> Result<PresenceRegistration, String> {
        validate_presence_principal_ura(&ura)?;
        validate_dispatch_session_contract(&contract)?;
        let session_id = self.next_session_id.fetch_add(1, Ordering::Relaxed);
        let prior = self.by_ura.insert(
            ura.clone(),
            PresenceSlot::Dispatch(PresenceDispatchSlot {
                session_id,
                sender,
                contract,
                trust,
            }),
        );
        let displaced_claimant_nonce = prior.as_ref().and_then(PresenceSlot::claimant_boot_nonce);
        let displaced = prior.as_ref().and_then(PresenceSlot::sender);
        if prior.is_some() {
            self.emit_offline(&ura, OfflineReason::StreamClosed);
        }
        self.emit_online(ura);
        Ok(PresenceRegistration {
            session_id,
            displaced,
            displaced_claimant_nonce,
        })
    }

    /// Remove a session, emitting `PresenceEvent::Offline` with the
    /// supplied reason. No-op if the URA is not present (the caller
    /// races; idempotent for the registry's lifecycle invariant).
    pub fn remove(&self, ura: &str, reason: OfflineReason) -> Option<DispatchSender> {
        let removed = self.by_ura.remove(ura).map(|(_k, slot)| slot);
        let prior = removed.as_ref().and_then(PresenceSlot::sender);
        if removed.is_some() {
            self.emit_offline(ura, reason);
        }
        prior
    }

    /// Remove a session only if the currently-registered entry for
    /// `ura` still belongs to `session_id`.
    ///
    /// This closes the reconnect/displacement race:
    /// a stale task holding metadata for an old session at `ura`
    /// must not be able to remove a newer replacement session that
    /// has already won a later `insert`.
    pub fn remove_if_session(
        &self,
        ura: &str,
        session_id: PresenceSessionId,
        reason: OfflineReason,
    ) -> Option<DispatchSender> {
        let prior = self
            .by_ura
            .remove_if(ura, |_ura, slot| slot.session_id() == session_id)
            .map(|(_k, slot)| slot);
        let sender = prior.as_ref().and_then(PresenceSlot::sender);
        if prior.is_some() {
            self.emit_offline(ura, reason);
        }
        sender
    }

    /// Find the dispatch sender for `ura`, cloning so the caller
    /// can hold it across `await` points without locking the shard.
    #[must_use]
    pub fn lookup(&self, ura: &str) -> Option<DispatchSender> {
        self.lookup_dispatch_session(ura)
            .map(|session| session.sender)
    }

    /// Snapshot sender identity and negotiated carrier contract from one live
    /// slot. Dispatchers must use this instead of independent sender/version
    /// reads so session displacement cannot create a mixed-generation view.
    #[must_use]
    pub fn lookup_dispatch_session(&self, ura: &str) -> Option<PresenceDispatchSession> {
        self.by_ura
            .get(ura)
            .and_then(|entry| entry.dispatch_session())
    }

    /// Find the current `(session_id, sender)` pair for `ura`.
    #[must_use]
    pub fn lookup_tracked(&self, ura: &str) -> Option<(PresenceSessionId, DispatchSender)> {
        self.lookup_dispatch_session(ura)
            .map(|session| (session.session_id, session.sender))
    }

    /// O(1) liveness check. Hot paths (route resolution runs per
    /// invocation) must use this, never `snapshot().contains(...)`
    /// — `snapshot()` materializes and sorts the whole table.
    #[must_use]
    pub fn contains(&self, ura: &str) -> bool {
        self.by_ura.contains_key(ura)
    }

    /// Cheap online-device count for stats fields (heartbeat runs
    /// per device every 5s — counting via `snapshot()` there was
    /// O(devices²·log) across the fleet).
    #[must_use]
    pub fn online_count(&self) -> usize {
        self.by_ura.len()
    }

    /// Take a deterministic snapshot of currently-online URAs. Used
    /// as the initial frame of a `federation.subscribe_directory_v2`
    /// pump and as the recovery path for a subscriber that received
    /// `Lagged`.
    ///
    /// The order is sorted ascending by URA so byte-identical bytes
    /// land on the wire from byte-identical input states; PR-4 wire
    /// compat tests rely on that determinism.
    #[must_use]
    pub fn snapshot(&self) -> Vec<String> {
        let mut uras: Vec<String> = self
            .by_ura
            .iter()
            .map(|entry| entry.key().clone())
            .collect();
        uras.sort();
        uras
    }

    /// Subscribe to the cross-cutting presence event stream. New
    /// receivers see only events emitted after this call; the
    /// caller should pair `subscribe_events` with a `snapshot` for
    /// bootstrap.
    pub fn subscribe_events(&self) -> broadcast::Receiver<PresenceEvent> {
        self.events.subscribe()
    }

    /// Force-remove a session and emit `Offline` with
    /// `OfflineReason::AdminRevoked`. Surface used by
    /// `federation.revoke` and operator tooling.
    pub fn force_revoke(&self, ura: &str) -> Option<DispatchSender> {
        self.remove(ura, OfflineReason::AdminRevoked)
    }

    /// Force-remove a session only when the live slot was admitted
    /// with `public_key_b64`.
    ///
    /// This is the runtime half of user-key revocation. A missing
    /// slot, a slot with no admitted user key, or a slot
    /// admitted by a different key is a no-op and emits no offline
    /// event.
    pub fn force_revoke_if_admitted_key(
        &self,
        ura: &str,
        public_key_b64: &str,
    ) -> Option<DispatchSender> {
        let prior = self
            .by_ura
            .remove_if(ura, |_ura, slot| slot.matches_admitted_key(public_key_b64))
            .map(|(_k, slot)| slot);
        let sender = prior.as_ref().and_then(PresenceSlot::sender);
        if prior.is_some() {
            self.emit_offline(ura, OfflineReason::AdminRevoked);
        }
        sender
    }

    fn emit_online(&self, ura: String) {
        let _ = self.events.send(PresenceEvent::Online { ura });
    }

    fn emit_offline(&self, ura: &str, reason: OfflineReason) {
        let _ = self.events.send(PresenceEvent::Offline {
            ura: ura.to_string(),
            reason,
        });
    }
}

fn validate_dispatch_session_contract(contract: &SessionContract) -> Result<(), String> {
    if contract.version < CANONICAL_SESSION_CARRIER_VERSION {
        return Err(format!(
            "session contract v{} is retired; v{} or newer is required",
            contract.version, CANONICAL_SESSION_CARRIER_VERSION,
        ));
    }
    if contract.claimant_boot_nonce.len() != 16 {
        return Err(format!(
            "session contract claimant_boot_nonce must be exactly 16 bytes; got {}",
            contract.claimant_boot_nonce.len()
        ));
    }
    Ok(())
}

fn validate_presence_principal_ura(ura: &str) -> Result<(), String> {
    let trimmed = ura.trim();
    if trimmed.is_empty() {
        return Err("presence key must be a non-empty canonical principal URA".to_string());
    }
    if trimmed != ura {
        return Err(format!(
            "presence key {ura:?} must not carry leading or trailing whitespace"
        ));
    }
    let parsed = crate::core::ura::parse_ura(trimmed)
        .map_err(|error| format!("presence key {ura:?} is not a canonical URA: {error}"))?;
    match parsed.kind {
        crate::core::ura::URAKind::Device
        | crate::core::ura::URAKind::User
        | crate::core::ura::URAKind::Agent => Ok(()),
        other => Err(format!(
            "presence key {ura:?} must be a canonical Device, User, or Agent URA; got {other:?}"
        )),
    }
}

impl Default for PresenceRegistry {
    fn default() -> Self {
        Self::new()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::time::Duration;
    use tokio::sync::broadcast::error::RecvError;

    /// Build a `DispatchSender` whose receiver is dropped immediately.
    /// Tests do not exercise dispatch; only the registry's bookkeeping.
    fn make_dispatch_sender() -> DispatchSender {
        let (tx, _rx) = mpsc::channel(DISPATCH_CHANNEL_CAPACITY);
        tx
    }

    #[test]
    fn new_registry_is_empty_and_snapshot_is_sorted_empty() {
        let registry = PresenceRegistry::new();
        assert!(registry.snapshot().is_empty());
        assert!(registry.lookup("easynet:///r/x/device/y").is_none());
    }

    #[test]
    fn negotiated_insert_remembers_contract_and_surfaces_prior_nonce() {
        let reg = PresenceRegistry::new();
        let (tx1, _rx1) = tokio::sync::mpsc::channel(1);
        let first = reg
            .insert_negotiated(
                "easynet:///r/t/device/d1".into(),
                tx1,
                SessionContract {
                    version: 1,
                    claimant_boot_nonce: vec![1; 16],
                },
            )
            .expect("canonical presence key");
        assert!(first.displaced.is_none());
        assert!(first.displaced_claimant_nonce.is_none());
        assert_eq!(
            reg.lookup_dispatch_session("easynet:///r/t/device/d1")
                .map(|session| session.contract_version),
            Some(1)
        );

        // A different claimant displacing the slot surfaces the prior
        // fingerprint so the accept path can classify the conflict.
        let (tx2, _rx2) = tokio::sync::mpsc::channel(1);
        let second = reg
            .insert_negotiated(
                "easynet:///r/t/device/d1".into(),
                tx2,
                SessionContract {
                    version: CANONICAL_SESSION_CARRIER_VERSION,
                    claimant_boot_nonce: vec![2; 16],
                },
            )
            .expect("canonical presence key");
        assert!(second.displaced.is_some());
        assert_eq!(second.displaced_claimant_nonce, Some(vec![1; 16]));
        assert_eq!(
            reg.lookup_dispatch_session("easynet:///r/t/device/d1")
                .map(|session| session.contract_version),
            Some(CANONICAL_SESSION_CARRIER_VERSION)
        );
    }

    #[test]
    fn negotiated_insert_rejects_missing_claimant_fingerprint() {
        let reg = PresenceRegistry::new();
        let (tx, _rx) = tokio::sync::mpsc::channel(1);
        let error = reg
            .insert_negotiated(
                "easynet:///r/t/device/d1".into(),
                tx,
                SessionContract::new(CANONICAL_SESSION_CARRIER_VERSION, Vec::new()),
            )
            .expect_err("dispatch session must carry claimant fingerprint");
        assert!(error.contains("claimant_boot_nonce"), "{error}");
        assert!(
            reg.snapshot().is_empty(),
            "invalid dispatch contract must fail before presence mutation"
        );
    }

    #[test]
    fn insert_tracked_registers_canonical_contract() {
        let reg = PresenceRegistry::new();
        let (tx, _rx) = tokio::sync::mpsc::channel(1);
        let r = reg
            .insert_tracked("easynet:///r/t/device/d2".into(), tx)
            .expect("canonical presence key");
        assert!(r.displaced_claimant_nonce.is_none());
        assert_eq!(
            reg.lookup_dispatch_session("easynet:///r/t/device/d2")
                .map(|session| session.contract_version),
            Some(CANONICAL_SESSION_CARRIER_VERSION)
        );
    }

    #[tokio::test]
    async fn resolve_only_presence_is_directory_visible_but_not_dispatchable() {
        let registry = PresenceRegistry::new();
        let ura = "easynet:///r/realm/device/self".to_string();
        let mut subscriber = registry.subscribe_events();

        let registration = registry
            .insert_resolve_only(ura.clone())
            .expect("canonical resolve-only presence key");

        assert!(registration.displaced.is_none());
        assert!(registration.displaced_claimant_nonce.is_none());
        assert!(registry.contains(&ura));
        assert_eq!(registry.snapshot(), vec![ura.clone()]);
        assert!(registry.lookup(&ura).is_none());
        assert!(
            registry.lookup_dispatch_session(&ura).is_none(),
            "resolve-only presence must never expose a dispatch session"
        );

        match subscriber.recv().await.expect("online event") {
            PresenceEvent::Online { ura: event_ura } => assert_eq!(event_ura, ura),
            other => panic!("expected Online, got {other:?}"),
        }
    }

    #[tokio::test]
    async fn resolve_only_remove_emits_offline_without_sender() {
        let registry = PresenceRegistry::new();
        let ura = "easynet:///r/realm/device/self".to_string();
        registry
            .insert_resolve_only(ura.clone())
            .expect("canonical resolve-only presence key");
        let mut subscriber = registry.subscribe_events();

        let removed = registry.force_revoke(&ura);

        assert!(removed.is_none());
        assert!(!registry.contains(&ura));
        match subscriber.recv().await.expect("offline event") {
            PresenceEvent::Offline {
                ura: event_ura,
                reason,
            } => {
                assert_eq!(event_ura, ura);
                assert_eq!(reason, OfflineReason::AdminRevoked);
            }
            other => panic!("expected Offline, got {other:?}"),
        }
    }

    #[test]
    fn insert_then_lookup_returns_sender() {
        let registry = PresenceRegistry::new();
        let ura = "easynet:///r/realm/device/node-1".to_string();
        let prior = registry
            .insert(ura.clone(), make_dispatch_sender())
            .expect("canonical presence key");
        assert!(prior.is_none());
        assert!(registry.lookup(&ura).is_some());
    }

    #[test]
    fn snapshot_is_sorted() {
        let registry = PresenceRegistry::new();
        registry
            .insert(
                "easynet:///r/realm/device/c".to_string(),
                make_dispatch_sender(),
            )
            .expect("canonical presence key");
        registry
            .insert(
                "easynet:///r/realm/device/a".to_string(),
                make_dispatch_sender(),
            )
            .expect("canonical presence key");
        registry
            .insert(
                "easynet:///r/realm/device/b".to_string(),
                make_dispatch_sender(),
            )
            .expect("canonical presence key");

        let snap = registry.snapshot();
        assert_eq!(
            snap,
            vec![
                "easynet:///r/realm/device/a".to_string(),
                "easynet:///r/realm/device/b".to_string(),
                "easynet:///r/realm/device/c".to_string(),
            ]
        );
    }

    #[tokio::test]
    async fn insert_emits_online_event() {
        let registry = PresenceRegistry::new();
        let mut subscriber = registry.subscribe_events();
        registry
            .insert(
                "easynet:///r/realm/device/n1".to_string(),
                make_dispatch_sender(),
            )
            .expect("canonical presence key");

        match subscriber.recv().await.expect("event") {
            PresenceEvent::Online { ura } => {
                assert_eq!(ura, "easynet:///r/realm/device/n1");
            }
            other => panic!("expected Online, got {other:?}"),
        }
    }

    #[tokio::test]
    async fn remove_emits_offline_with_reason() {
        let registry = PresenceRegistry::new();
        let ura = "easynet:///r/realm/device/n1".to_string();
        registry
            .insert(ura.clone(), make_dispatch_sender())
            .expect("canonical presence key");

        let mut subscriber = registry.subscribe_events();
        registry.remove(&ura, OfflineReason::StreamReset);

        match subscriber.recv().await.expect("event") {
            PresenceEvent::Offline {
                ura: out_ura,
                reason,
            } => {
                assert_eq!(out_ura, ura);
                assert_eq!(reason, OfflineReason::StreamReset);
            }
            other => panic!("expected Offline, got {other:?}"),
        }
    }

    #[tokio::test]
    async fn displacement_emits_offline_then_online_in_order() {
        let registry = PresenceRegistry::new();
        let ura = "easynet:///r/realm/device/n1".to_string();

        registry
            .insert(ura.clone(), make_dispatch_sender())
            .expect("canonical presence key");

        // Subscribe AFTER the first insert so we observe only the
        // displacement transition.
        let mut subscriber = registry.subscribe_events();

        let displaced = registry
            .insert(ura.clone(), make_dispatch_sender())
            .expect("canonical presence key");
        assert!(displaced.is_some(), "second insert must displace");

        let first = subscriber.recv().await.expect("first event");
        let second = subscriber.recv().await.expect("second event");

        match (first, second) {
            (
                PresenceEvent::Offline {
                    ura: u1,
                    reason: OfflineReason::StreamClosed,
                },
                PresenceEvent::Online { ura: u2 },
            ) => {
                assert_eq!(u1, ura);
                assert_eq!(u2, ura);
            }
            other => panic!("expected Offline-then-Online for displacement, got {other:?}"),
        }
    }

    #[tokio::test]
    async fn stale_sender_cannot_remove_newer_displaced_session() {
        let registry = PresenceRegistry::new();
        let ura = "easynet:///r/realm/device/n1".to_string();
        let sender_a = make_dispatch_sender();
        let sender_b = make_dispatch_sender();

        let first = registry
            .insert_tracked(ura.clone(), sender_a)
            .expect("canonical presence key");

        let mut subscriber = registry.subscribe_events();
        let second = registry
            .insert_tracked(ura.clone(), sender_b.clone())
            .expect("canonical presence key");
        assert!(
            second.displaced.is_some(),
            "second insert must displace first sender"
        );

        // Drain the displacement transition first.
        let _ = subscriber.recv().await.expect("offline displacement event");
        let _ = subscriber.recv().await.expect("online replacement event");

        let removed =
            registry.remove_if_session(&ura, first.session_id, OfflineReason::StreamClosed);
        assert!(
            removed.is_none(),
            "stale session id must not remove the replacement session"
        );

        let (current_session_id, current) = registry
            .lookup_tracked(&ura)
            .expect("replacement session still present");
        assert!(
            current.same_channel(&sender_b),
            "registry must still point at the replacement sender"
        );
        assert_eq!(current_session_id, second.session_id);

        match tokio::time::timeout(Duration::from_millis(50), subscriber.recv()).await {
            Err(_elapsed) => {}
            Ok(other) => panic!("stale remove must not emit a new offline event, got {other:?}"),
        }
    }

    #[test]
    fn remove_missing_ura_is_noop_and_returns_none() {
        let registry = PresenceRegistry::new();
        let prior = registry.remove(
            "easynet:///r/realm/device/missing",
            OfflineReason::StreamClosed,
        );
        assert!(prior.is_none());
        assert!(registry.snapshot().is_empty());
    }

    #[test]
    fn force_revoke_emits_admin_revoked_offline() {
        let registry = PresenceRegistry::new();
        let ura = "easynet:///r/realm/device/n1".to_string();
        registry
            .insert(ura.clone(), make_dispatch_sender())
            .expect("canonical presence key");

        let prior = registry.force_revoke(&ura);
        assert!(prior.is_some());
        assert!(registry.lookup(&ura).is_none());
    }

    #[test]
    fn force_revoke_if_admitted_key_removes_matching_user_slot() {
        let registry = PresenceRegistry::new();
        let ura = "easynet:///r/realm/user/alice".to_string();
        let key = "pubkey-a";
        registry
            .insert_negotiated_with_trust(
                ura.clone(),
                make_dispatch_sender(),
                SessionContract::new(CANONICAL_SESSION_CARRIER_VERSION, vec![1; 16]),
                SessionTrustContext::user_pubkey(key),
            )
            .expect("canonical presence key");

        let prior = registry.force_revoke_if_admitted_key(&ura, key);

        assert!(prior.is_some());
        assert!(registry.lookup(&ura).is_none());
    }

    #[test]
    fn force_revoke_if_admitted_key_keeps_different_key_slot() {
        let registry = PresenceRegistry::new();
        let ura = "easynet:///r/realm/user/alice".to_string();
        registry
            .insert_negotiated_with_trust(
                ura.clone(),
                make_dispatch_sender(),
                SessionContract::new(CANONICAL_SESSION_CARRIER_VERSION, vec![1; 16]),
                SessionTrustContext::user_pubkey("pubkey-b"),
            )
            .expect("canonical presence key");

        let prior = registry.force_revoke_if_admitted_key(&ura, "pubkey-a");

        assert!(prior.is_none());
        assert!(registry.lookup(&ura).is_some());
    }

    #[tokio::test]
    async fn slow_subscriber_lags_and_recovers_via_snapshot() {
        // Force a tiny capacity so we can drive Lagged deterministically.
        let registry = PresenceRegistry::with_capacity(2);
        let mut subscriber = registry.subscribe_events();

        for n in 0..10 {
            registry
                .insert(
                    crate::core::ura::agent_ura("realm", "u1", &format!("n{n}")),
                    make_dispatch_sender(),
                )
                .expect("canonical presence key");
        }

        // The slow subscriber observes lag rather than blocking
        // production paths.
        match subscriber.recv().await {
            Err(RecvError::Lagged(_)) => { /* expected */ }
            other => panic!("expected Lagged, got {other:?}"),
        }

        // Recovery path is to snapshot the registry directly.
        let snap = registry.snapshot();
        assert_eq!(snap.len(), 10);
    }

    #[test]
    fn insert_rejects_malformed_presence_key_before_mutation() {
        let registry = PresenceRegistry::new();

        let error = registry
            .insert("not-a-ura".to_string(), make_dispatch_sender())
            .expect_err("malformed presence key must fail closed");

        assert!(
            error.contains("canonical URA"),
            "unexpected presence validation error: {error}"
        );
        assert!(
            registry.snapshot().is_empty(),
            "malformed presence keys must not mutate live registry state"
        );
    }

    #[test]
    fn insert_rejects_non_principal_presence_key_before_mutation() {
        let registry = PresenceRegistry::new();

        let error = registry
            .insert(
                crate::core::ura::resource_dot_ura("realm", "user.alice", "session/s1"),
                make_dispatch_sender(),
            )
            .expect_err("non-principal presence key must fail closed");

        assert!(
            error.contains("Device, User, or Agent URA"),
            "unexpected presence validation error: {error}"
        );
        assert!(
            registry.snapshot().is_empty(),
            "non-principal presence keys must not mutate live registry state"
        );
    }

    #[test]
    fn capacity_constants_match_spec_section_3_2() {
        // Pin the values directly: spec §3.2 says 1024 / 256, and a
        // future drift in the constants must require updating spec
        // first. The `presence_registry_shards()` value is dynamic
        // (num_cpus*4) so we only assert it is positive.
        assert_eq!(PRESENCE_EVENT_CHANNEL_CAPACITY, 1024);
        assert_eq!(DISPATCH_CHANNEL_CAPACITY, 256);
        assert!(presence_registry_shards() > 0);
    }
}
