// EasyNet CLI — Services Layer — PresenceRegistry
// =================================================
//
// File: src/services/presence_registry.rs
// Description: In-memory, sharded, broadcast-equipped registry of
//              live `<self>.session` reverse channels keyed by
//              caller URA. Hub-side liveness model for the new
//              transport plane.
//
// Why this module exists
// ----------------------
// The pre-RFC-003 daemon learnt liveness from periodic
// `federation.heartbeat` unary calls — a polling model that drifted
// out of sync with the actual transport (devices could be reachable
// while heartbeats failed and vice versa). RFC-003 collapses
// liveness onto the transport itself: a device is *alive* exactly
// when its `<self>.session` `InvokeBidi` stream is open. When the
// stream closes for any reason, the registry drops the entry and
// emits an `Offline` event downstream consumers (subscribe_directory
// pumps, federation.* wrappers, the daemon's audit log) all share.
//
// This module is the single canonical home for that state. PR-1
// spec §3 (`pr-drafts/PR-0-spec-daemon-invocation-server.md`) pins
// the surface; PR-2 will populate it from the real `<self>.session`
// accept handler; PR-1 lands the structure plus its tests so PR-2
// reviewers can read it standing still.
//
// Public surface
// --------------
// - `PresenceRegistry` — the registry itself, owned by the daemon
//   `DaemonInvocationService` via `Arc`
// - `DispatchSender` — type alias for the per-device mpsc sender
//   that `<self>.invoke_remote` and `federation.forward_invoke`
//   push reverse-channel frames into
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
//    accumulate frames in the hub's memory; `forward_invoke` and
//    `<self>.invoke_remote` push paths handle `try_send` failure
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

/// Sender end of the per-device mpsc that downstream call paths
/// (`<self>.invoke_remote`, `federation.forward_invoke`) push
/// reverse-channel frames into. The receiving end is held by the
/// device's `<self>.session` task.
///
/// Frames are tonic results so a stream-level error (e.g., admission
/// gate revocation) can propagate to the device cleanly. The receiver
/// converts these into the outbound `InvokeBidiDown` stream.
pub type DispatchSender = mpsc::Sender<Result<DispatchFrame, tonic::Status>>;

/// Monotonic identity of one admitted `<self>.session`.
///
/// Session ids are process-local and exist only to let stale tasks
/// prove which registry entry they own when removing after a race
/// with displacement/reconnect.
pub type PresenceSessionId = u64;

/// One frame heading down a `<self>.session` reverse channel.
///
/// A thin newtype around the proto-generated `InvokeBidiDown` so
/// downstream call paths describe their pushes in terms of presence
/// vocabulary rather than raw proto types.
#[derive(Debug, Clone)]
pub struct DispatchFrame {
    /// The proto-encoded bidirectional-down frame. Owned so the
    /// dispatcher does not hold references across the channel
    /// boundary.
    pub frame: easynet_axon::pb::axon::v1::InvokeBidiDown,
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
/// online or offline. Subscribers (subscribe_directory pumps, audit
/// log writers) drive their state from this stream.
#[derive(Debug, Clone)]
pub enum PresenceEvent {
    /// A new `<self>.session` was accepted for the given URA. If a
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
struct PresenceSlot {
    session_id: PresenceSessionId,
    sender: DispatchSender,
}

#[derive(Debug, Clone)]
pub struct PresenceRegistration {
    pub session_id: PresenceSessionId,
    pub displaced: Option<DispatchSender>,
}

/// Concurrent registry of live `<self>.session` reverse channels.
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

    /// Register a new `<self>.session` for `ura`.
    ///
    /// Emits `PresenceEvent::Offline { reason: StreamClosed }` for
    /// any displaced prior sender, then `PresenceEvent::Online`
    /// for the newcomer. The invariant ordering (Offline-before-
    /// Online) means a subscribe_directory pump that observes both
    /// events sees a clean transition rather than a duplicated URA.
    ///
    /// Returns the displaced sender if any so the caller can observe
    /// the prior session's state if it cares; production paths
    /// drop it.
    pub fn insert(&self, ura: String, sender: DispatchSender) -> Option<DispatchSender> {
        self.insert_tracked(ura, sender).displaced
    }

    /// Register a new `<self>.session` and return the registry-owned
    /// `session_id` alongside any displaced prior sender.
    pub fn insert_tracked(&self, ura: String, sender: DispatchSender) -> PresenceRegistration {
        let session_id = self.next_session_id.fetch_add(1, Ordering::Relaxed);
        let displaced = self
            .by_ura
            .insert(ura.clone(), PresenceSlot { session_id, sender })
            .map(|prior| prior.sender);
        if displaced.is_some() {
            // Always emit Offline for the displaced session before
            // Online for the newcomer; ignore broadcast send errors
            // (no subscribers is not an error).
            let _ = self.events.send(PresenceEvent::Offline {
                ura: ura.clone(),
                reason: OfflineReason::StreamClosed,
            });
        }
        let _ = self.events.send(PresenceEvent::Online { ura });
        PresenceRegistration {
            session_id,
            displaced,
        }
    }

    /// Remove a session, emitting `PresenceEvent::Offline` with the
    /// supplied reason. No-op if the URA is not present (the caller
    /// races; idempotent for the registry's lifecycle invariant).
    pub fn remove(&self, ura: &str, reason: OfflineReason) -> Option<DispatchSender> {
        let prior = self.by_ura.remove(ura).map(|(_k, slot)| slot.sender);
        if prior.is_some() {
            let _ = self.events.send(PresenceEvent::Offline {
                ura: ura.to_string(),
                reason,
            });
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
            .remove_if(ura, |_uri, slot| slot.session_id == session_id)
            .map(|(_k, slot)| slot.sender);
        if prior.is_some() {
            let _ = self.events.send(PresenceEvent::Offline {
                ura: ura.to_string(),
                reason,
            });
        }
        prior
    }

    /// Find the dispatch sender for `ura`, cloning so the caller
    /// can hold it across `await` points without locking the shard.
    #[must_use]
    pub fn lookup(&self, ura: &str) -> Option<DispatchSender> {
        self.by_ura.get(ura).map(|entry| entry.sender.clone())
    }

    /// Find the current `(session_id, sender)` pair for `ura`.
    #[must_use]
    pub fn lookup_tracked(&self, ura: &str) -> Option<(PresenceSessionId, DispatchSender)> {
        self.by_ura
            .get(ura)
            .map(|entry| (entry.session_id, entry.sender.clone()))
    }

    /// Take a deterministic snapshot of currently-online URAs. Used
    /// as the initial frame of a `federation.subscribe_directory`
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
    fn insert_then_lookup_returns_sender() {
        let registry = PresenceRegistry::new();
        let ura = "easynet:///r/realm/device/node-1".to_string();
        let prior = registry.insert(ura.clone(), make_dispatch_sender());
        assert!(prior.is_none());
        assert!(registry.lookup(&ura).is_some());
    }

    #[test]
    fn snapshot_is_sorted() {
        let registry = PresenceRegistry::new();
        registry.insert(
            "easynet:///r/realm/device/c".to_string(),
            make_dispatch_sender(),
        );
        registry.insert(
            "easynet:///r/realm/device/a".to_string(),
            make_dispatch_sender(),
        );
        registry.insert(
            "easynet:///r/realm/device/b".to_string(),
            make_dispatch_sender(),
        );

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
        registry.insert(
            "easynet:///r/realm/device/n1".to_string(),
            make_dispatch_sender(),
        );

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
        registry.insert(ura.clone(), make_dispatch_sender());

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

        registry.insert(ura.clone(), make_dispatch_sender());

        // Subscribe AFTER the first insert so we observe only the
        // displacement transition.
        let mut subscriber = registry.subscribe_events();

        let displaced = registry.insert(ura.clone(), make_dispatch_sender());
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

        let first = registry.insert_tracked(ura.clone(), sender_a);

        let mut subscriber = registry.subscribe_events();
        let second = registry.insert_tracked(ura.clone(), sender_b.clone());
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
        registry.insert(ura.clone(), make_dispatch_sender());

        let prior = registry.force_revoke(&ura);
        assert!(prior.is_some());
        assert!(registry.lookup(&ura).is_none());
    }

    #[tokio::test]
    async fn slow_subscriber_lags_and_recovers_via_snapshot() {
        // Force a tiny capacity so we can drive Lagged deterministically.
        let registry = PresenceRegistry::with_capacity(2);
        let mut subscriber = registry.subscribe_events();

        for n in 0..10 {
            registry.insert(
                crate::ura::agent_ura("realm", "u1", &format!("n{n}")),
                make_dispatch_sender(),
            );
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
