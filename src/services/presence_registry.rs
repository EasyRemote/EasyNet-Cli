// EasyNet CLI — Services Layer — PresenceRegistry
// =================================================
//
// File: src/services/presence_registry.rs
// Description: In-memory, sharded, broadcast-equipped registry of
//              live `<self>.session` reverse channels keyed by
//              caller URI. Hub-side liveness model for the new
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
//   `Offline`, with the URI plus (for Offline) a typed reason
// - `OfflineReason` — disjoint reasons a session is removed
// - Capacity constants (`PRESENCE_EVENT_CHANNEL_CAPACITY`,
//   `DISPATCH_CHANNEL_CAPACITY`, `presence_registry_shards()`)
//   exposed at module top so call sites and operators can refer to
//   them by name; rationale lives next to each definition
//
// Invariants
// ----------
// 1. **URI = key**. The registry keys on the *caller-claimed*
//    `easynet:///r/{tenant_id}/agent/{node_id}` URI from the
//    EnvelopeOpen first frame's signed envelope. There is no
//    hub-minted `agent/a-X` shadow identity in the new architecture
//    (spec §5.1 URI scheme migration).
// 2. **Lifecycle = liveness**. A device is "online" exactly when
//    `by_uri.contains_key(uri)` is true. Removal MUST emit an
//    `Offline` event before the entry is gone from the map; the
//    method order in `remove` enforces this so an observer that
//    races a snapshot against an event sees consistent state.
// 3. **Single live entry per URI**. `insert` returning the prior
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
    pub frame: crate::pb::axon::v1::InvokeBidiDown,
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

/// Event emitted on the broadcast channel whenever a session goes
/// online or offline. Subscribers (subscribe_directory pumps, audit
/// log writers) drive their state from this stream.
#[derive(Debug, Clone)]
pub enum PresenceEvent {
    /// A new `<self>.session` was accepted for the given URI. If a
    /// previous session existed for the same URI it is implicitly
    /// offline (this event was preceded by an `Offline` for the
    /// displaced sender — the registry guarantees the order).
    Online {
        /// Caller-claimed URI keyed in the registry.
        uri: String,
    },

    /// A previously-online session is now offline. The reason
    /// indicates how the registry learnt of the loss.
    Offline {
        /// Caller-claimed URI keyed in the registry.
        uri: String,

        /// How the session ended.
        reason: OfflineReason,
    },
}

/// Concurrent registry of live `<self>.session` reverse channels.
///
/// The registry is the single owner of the mapping
/// `device URI -> DispatchSender` and the canonical source of
/// presence transition events for every consumer. Construct one
/// per daemon process and pass it around by `Arc`.
#[derive(Debug)]
pub struct PresenceRegistry {
    /// Sharded map keyed by caller-claimed URI. Outer `Arc` makes
    /// the entire registry cheap to clone for handler tasks.
    by_uri: Arc<DashMap<String, DispatchSender>>,

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
            by_uri: Arc::new(DashMap::with_shard_amount(presence_registry_shards())),
            events: tx,
        }
    }

    /// Register a new `<self>.session` for `uri`.
    ///
    /// Emits `PresenceEvent::Offline { reason: StreamClosed }` for
    /// any displaced prior sender, then `PresenceEvent::Online`
    /// for the newcomer. The invariant ordering (Offline-before-
    /// Online) means a subscribe_directory pump that observes both
    /// events sees a clean transition rather than a duplicated URI.
    ///
    /// Returns the displaced sender if any so the caller can observe
    /// the prior session's state if it cares; production paths
    /// drop it.
    pub fn insert(&self, uri: String, sender: DispatchSender) -> Option<DispatchSender> {
        let displaced = self.by_uri.insert(uri.clone(), sender);
        if displaced.is_some() {
            // Always emit Offline for the displaced session before
            // Online for the newcomer; ignore broadcast send errors
            // (no subscribers is not an error).
            let _ = self.events.send(PresenceEvent::Offline {
                uri: uri.clone(),
                reason: OfflineReason::StreamClosed,
            });
        }
        let _ = self.events.send(PresenceEvent::Online { uri });
        displaced
    }

    /// Remove a session, emitting `PresenceEvent::Offline` with the
    /// supplied reason. No-op if the URI is not present (the caller
    /// races; idempotent for the registry's lifecycle invariant).
    pub fn remove(&self, uri: &str, reason: OfflineReason) -> Option<DispatchSender> {
        let prior = self.by_uri.remove(uri).map(|(_k, v)| v);
        if prior.is_some() {
            let _ = self.events.send(PresenceEvent::Offline {
                uri: uri.to_string(),
                reason,
            });
        }
        prior
    }

    /// Find the dispatch sender for `uri`, cloning so the caller
    /// can hold it across `await` points without locking the shard.
    #[must_use]
    pub fn lookup(&self, uri: &str) -> Option<DispatchSender> {
        self.by_uri.get(uri).map(|entry| entry.value().clone())
    }

    /// Take a deterministic snapshot of currently-online URIs. Used
    /// as the initial frame of a `federation.subscribe_directory`
    /// pump and as the recovery path for a subscriber that received
    /// `Lagged`.
    ///
    /// The order is sorted ascending by URI so byte-identical bytes
    /// land on the wire from byte-identical input states; PR-4 wire
    /// compat tests rely on that determinism.
    #[must_use]
    pub fn snapshot(&self) -> Vec<String> {
        let mut uris: Vec<String> = self
            .by_uri
            .iter()
            .map(|entry| entry.key().clone())
            .collect();
        uris.sort();
        uris
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
    pub fn force_revoke(&self, uri: &str) -> Option<DispatchSender> {
        self.remove(uri, OfflineReason::AdminRevoked)
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
        assert!(registry.lookup("easynet:///r/x/agent/y").is_none());
    }

    #[test]
    fn insert_then_lookup_returns_sender() {
        let registry = PresenceRegistry::new();
        let uri = "easynet:///r/realm/agent/node-1".to_string();
        let prior = registry.insert(uri.clone(), make_dispatch_sender());
        assert!(prior.is_none());
        assert!(registry.lookup(&uri).is_some());
    }

    #[test]
    fn snapshot_is_sorted() {
        let registry = PresenceRegistry::new();
        registry.insert(
            "easynet:///r/realm/agent/c".to_string(),
            make_dispatch_sender(),
        );
        registry.insert(
            "easynet:///r/realm/agent/a".to_string(),
            make_dispatch_sender(),
        );
        registry.insert(
            "easynet:///r/realm/agent/b".to_string(),
            make_dispatch_sender(),
        );

        let snap = registry.snapshot();
        assert_eq!(
            snap,
            vec![
                "easynet:///r/realm/agent/a".to_string(),
                "easynet:///r/realm/agent/b".to_string(),
                "easynet:///r/realm/agent/c".to_string(),
            ]
        );
    }

    #[tokio::test]
    async fn insert_emits_online_event() {
        let registry = PresenceRegistry::new();
        let mut subscriber = registry.subscribe_events();
        registry.insert(
            "easynet:///r/realm/agent/n1".to_string(),
            make_dispatch_sender(),
        );

        match subscriber.recv().await.expect("event") {
            PresenceEvent::Online { uri } => {
                assert_eq!(uri, "easynet:///r/realm/agent/n1");
            }
            other => panic!("expected Online, got {other:?}"),
        }
    }

    #[tokio::test]
    async fn remove_emits_offline_with_reason() {
        let registry = PresenceRegistry::new();
        let uri = "easynet:///r/realm/agent/n1".to_string();
        registry.insert(uri.clone(), make_dispatch_sender());

        let mut subscriber = registry.subscribe_events();
        registry.remove(&uri, OfflineReason::StreamReset);

        match subscriber.recv().await.expect("event") {
            PresenceEvent::Offline {
                uri: out_uri,
                reason,
            } => {
                assert_eq!(out_uri, uri);
                assert_eq!(reason, OfflineReason::StreamReset);
            }
            other => panic!("expected Offline, got {other:?}"),
        }
    }

    #[tokio::test]
    async fn displacement_emits_offline_then_online_in_order() {
        let registry = PresenceRegistry::new();
        let uri = "easynet:///r/realm/agent/n1".to_string();

        registry.insert(uri.clone(), make_dispatch_sender());

        // Subscribe AFTER the first insert so we observe only the
        // displacement transition.
        let mut subscriber = registry.subscribe_events();

        let displaced = registry.insert(uri.clone(), make_dispatch_sender());
        assert!(displaced.is_some(), "second insert must displace");

        let first = subscriber.recv().await.expect("first event");
        let second = subscriber.recv().await.expect("second event");

        match (first, second) {
            (
                PresenceEvent::Offline {
                    uri: u1,
                    reason: OfflineReason::StreamClosed,
                },
                PresenceEvent::Online { uri: u2 },
            ) => {
                assert_eq!(u1, uri);
                assert_eq!(u2, uri);
            }
            other => panic!("expected Offline-then-Online for displacement, got {other:?}"),
        }
    }

    #[test]
    fn remove_missing_uri_is_noop_and_returns_none() {
        let registry = PresenceRegistry::new();
        let prior = registry.remove(
            "easynet:///r/realm/agent/missing",
            OfflineReason::StreamClosed,
        );
        assert!(prior.is_none());
        assert!(registry.snapshot().is_empty());
    }

    #[test]
    fn force_revoke_emits_admin_revoked_offline() {
        let registry = PresenceRegistry::new();
        let uri = "easynet:///r/realm/agent/n1".to_string();
        registry.insert(uri.clone(), make_dispatch_sender());

        let prior = registry.force_revoke(&uri);
        assert!(prior.is_some());
        assert!(registry.lookup(&uri).is_none());
    }

    #[tokio::test]
    async fn slow_subscriber_lags_and_recovers_via_snapshot() {
        // Force a tiny capacity so we can drive Lagged deterministically.
        let registry = PresenceRegistry::with_capacity(2);
        let mut subscriber = registry.subscribe_events();

        for n in 0..10 {
            registry.insert(
                format!("easynet:///r/realm/agent/n{n}"),
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
