// EasyNet CLI — Cross-realm directory federation types (RFC-N3)
// ===============================================================
//
// File: src/services/federation_directory.rs
// Description: Wire shapes for the cross-realm directory
//              federation surface introduced by PR-N3
//              (`pr-drafts/PR-N3-spec-cross-realm-directory-v2.md`).
//
//              This commit (N3-1) lands `DirectoryEntry` only.
//              `DirectoryEvent` (the event-stream tagged enum) +
//              the `subscribe_directory` long-stream FSM upgrade
//              live in N3-2; the per-peer `RemoteDirectoryClient`
//              + `SharedFederatedDirectoryView` cell live in N3-3.
//
// Why a new module
// ----------------
// `federation_wrappers.rs` hosts the original PR-1 `federation.*`
// ability surface (`AgentSummary`, `JoinResponse`, etc.) which
// represents *presence* — "is this URI online right now". The
// RFC-N3 surface represents the *cross-realm directory* — a
// federated, mutually-subscribed view of every paired device on
// every trusted peer hub, with origin-realm provenance carried in
// the wire bytes. Mixing the two in one file would conflate two
// different audit boundaries (presence is per-stream-lifetime;
// directory entries persist across reconnects and are subject to
// the §2.4 origin_realm rewrite chokepoint).
//
// Author: Silan.Hu <silan.hu@u.nus.edu>
// Copyright (c) 2026 EasyNet. All rights reserved.

use std::collections::BTreeMap;
use std::sync::{Arc, RwLock};

use serde::{Deserialize, Serialize};

/// One entry in the federated directory view.
///
/// **Spec v2 §2.1**. Schema-B fields (`origin_realm`,
/// `hub_endpoint`, `last_seen_unix_ms`) ride `#[serde(default)]`
/// so a legacy reader (PR-N1 commit 8/N consumer that only knows
/// `agent_uri`/`node_id`/`display_name`/`status`) deserialises
/// new bytes unchanged. New readers project the optional fields
/// when present.
///
/// `origin_realm` carries the provenance of the entry. **None**
/// ⇔ the entry was constructed by the hub serving this view
/// (i.e. it speaks for its own realm). **Some(realm)** ⇔ the
/// entry was received from a peer hub's `subscribe_directory`
/// stream and the receiving hub stamped the peer's realm into
/// this field at the merge boundary. Wire-tampering is blocked
/// by the §2.4 rewrite chokepoint — the receiving hub
/// **overwrites** this field with the peer's authenticated realm
/// regardless of what the peer's bytes claimed, so a malicious
/// peer cannot pretend its entries originate elsewhere.
///
/// `hub_endpoint` is the hub URL/address that owns this device.
/// Useful for backend `listDevices` views and for the CLI to
/// render which hub a remote device is paired against. The
/// daemon-side `<self>.discover` path is allowed to leave this
/// `None` for local entries (local readers already know the
/// daemon's own endpoint).
///
/// `last_seen_unix_ms` is the epoch-ms timestamp of the last
/// heartbeat the *origin* hub observed for this device. Local
/// entries fill from `PresenceRegistry`; cross-realm entries
/// reflect the peer's reported value verbatim — no clock
/// translation, the peer's clock and ours are assumed
/// approximately synchronised (NTP-coordinated production
/// machines).
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct DirectoryEntry {
    /// Canonical agent URI. Always realm-prefixed
    /// (`easynet:///r/<realm>/agent/<id>` per PR-7 §5.1).
    pub agent_uri: String,
    /// Stable node id within the realm. Matches the device's
    /// `credentials.json::node_id`.
    pub node_id: String,
    /// Operator-set display name. `None` ⇒ CLI renders
    /// `node_id` as a fallback.
    #[serde(default)]
    pub display_name: Option<String>,
    /// `"active"` | `"stale"` | `"draining"`. Stale = last
    /// heartbeat older than the realm's keepalive deadline;
    /// draining = the device announced shutdown but has not
    /// yet been removed from the registry.
    pub status: String,
    /// Realm of origin. See struct docs for the rewrite
    /// chokepoint that authenticates this field.
    #[serde(default)]
    pub origin_realm: Option<String>,
    /// Hub endpoint that owns the device.
    #[serde(default)]
    pub hub_endpoint: Option<String>,
    /// Last-heartbeat epoch-ms.
    #[serde(default)]
    pub last_seen_unix_ms: Option<i64>,
}

// ── DirectoryEvent (PR-N3 N3-2) ────────────────────────────────────

/// Frames the `subscribe_directory` server-stream emits to a
/// subscriber.
///
/// **Spec v2 §2.2**. Tagged with `#[serde(tag = "type",
/// rename_all = "snake_case")]` so JSON consumers see the
/// ergonomic `{"type": "upsert", "entry": {...}}` shape. The
/// first frame on every connection MUST be `Snapshot`; thereafter
/// only `Upsert`, `Remove`, or `Heartbeat`. A second `Snapshot`
/// mid-stream is a protocol violation and the receiver MUST drop
/// the connection (see `SubscriberFsm`).
///
/// `Heartbeat` is the keepalive frame sent every 30s when no
/// real event has been emitted. Receivers MUST tolerate it (drop
/// on the floor); senders MUST emit it so the receiver's idle-
/// timeout watcher can distinguish "stream alive but quiet" from
/// "stream silently dead" and trigger a reconnect when the
/// keepalive itself stops.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum DirectoryEvent {
    /// First frame on every subscribe stream. Receiver replaces
    /// its peer-keyed view wholesale.
    Snapshot { entries: Vec<DirectoryEntry> },
    /// Incremental: one entry added or status-changed.
    Upsert { entry: DirectoryEntry },
    /// Incremental: entry removed.
    Remove { agent_uri: String, reason: String },
    /// Keepalive. Sender emits every 30s when no other frame
    /// has been emitted in window.
    Heartbeat { sent_at_unix_ms: i64 },
}

// ── Subscriber FSM (PR-N3 N3-2) ────────────────────────────────────

/// Per-peer subscribe-stream state machine.
///
/// **Spec v2 §2.3**. The state transitions are:
///
/// ```text
///   Disconnected ──dial──>  Connecting
///   Connecting   ──ok──>    Snapshotting
///   Connecting   ──fail──>  Backoff(t = min(t*2, 60s))
///   Snapshotting ──Snapshot frame─> Pumping
///   Pumping      ──Upsert/Remove/Heartbeat─> Pumping
///   Pumping      ──stream end─> Disconnected
///   Pumping      ──60s no frame─> Disconnected (treat as dead)
///   Backoff(t)   ──t expires─>   Connecting
/// ```
///
/// The FSM is pure data — it owns no clock and no I/O. The
/// per-peer tokio task that drives it calls `on_dial_ok`,
/// `on_dial_err`, `on_frame`, `on_idle_timeout`, and
/// `on_stream_end` from real I/O outcomes; backoff scheduling
/// reads `next_backoff_ms` and sleeps externally. This isolation
/// is what makes the FSM unit-testable without a tokio runtime.
pub struct SubscriberFsm {
    state: SubscriberState,
    /// Most recent backoff floor in milliseconds. Cap is 60_000;
    /// floor is 1_000. Doubles on each consecutive dial failure;
    /// resets to 1_000 on the first non-Heartbeat frame in a
    /// Pumping window.
    backoff_ms: u64,
}

/// Public view of the FSM's current state. Consumers that need
/// to observe progress (eg. an admin dashboard rendering "peer
/// X is connected" / "peer X is in 8s backoff") match on this.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum SubscriberState {
    Disconnected,
    /// Dial in flight; waiting on TCP/TLS + initial gRPC frame.
    Connecting,
    /// Connection up; waiting for the mandatory Snapshot frame.
    Snapshotting,
    /// Snapshot received; pumping incremental frames.
    Pumping,
    /// Last dial failed; sleep `delay_ms` then retry.
    Backoff { delay_ms: u64 },
}

/// Errors the FSM emits when an inbound frame violates the
/// stream contract. The per-peer task maps these to a clean
/// disconnect + Backoff transition.
#[derive(Debug, Clone)]
pub enum FsmError {
    /// A frame arrived that the current state cannot accept
    /// (eg. Snapshot mid-Pumping, or Upsert before Snapshot).
    /// The receiver MUST drop the connection.
    ProtocolViolation(&'static str),
}

const BACKOFF_FLOOR_MS: u64 = 1_000;
const BACKOFF_CEILING_MS: u64 = 60_000;

impl SubscriberFsm {
    #[must_use]
    pub fn new() -> Self {
        Self {
            state: SubscriberState::Disconnected,
            backoff_ms: BACKOFF_FLOOR_MS,
        }
    }

    pub fn state(&self) -> &SubscriberState {
        &self.state
    }

    /// Backoff value the per-peer task should sleep before the
    /// next dial attempt. Always within `[BACKOFF_FLOOR_MS,
    /// BACKOFF_CEILING_MS]`.
    pub fn next_backoff_ms(&self) -> u64 {
        self.backoff_ms
    }

    /// Successful dial: TCP/TLS up, gRPC stream open. Awaiting
    /// the mandatory Snapshot frame.
    pub fn on_dial_ok(&mut self) {
        self.state = SubscriberState::Snapshotting;
    }

    /// Dial failed (TCP, TLS handshake, gRPC open, anything
    /// before the first frame). Sets up the next backoff window.
    pub fn on_dial_err(&mut self) {
        let next_delay = self.backoff_ms.saturating_mul(2).min(BACKOFF_CEILING_MS);
        self.backoff_ms = next_delay.max(BACKOFF_FLOOR_MS);
        self.state = SubscriberState::Backoff {
            delay_ms: self.backoff_ms,
        };
    }

    /// Stream ended without error. Drop to Disconnected; the
    /// per-peer task drives the reconnect.
    pub fn on_stream_end(&mut self) {
        self.state = SubscriberState::Disconnected;
    }

    /// 60s elapsed without any frame (including Heartbeat). The
    /// peer is silently dead; tear down + reconnect.
    pub fn on_idle_timeout(&mut self) {
        self.state = SubscriberState::Disconnected;
    }

    /// Process an inbound frame.
    ///
    /// Returns `Err(ProtocolViolation)` when the frame violates
    /// the contract (eg. Snapshot-mid-Pumping); the receiver
    /// MUST drop the connection in that case (the FSM's state
    /// has already been set to Disconnected). Returns `Ok(())`
    /// for all valid frames; the FSM transitions internally and
    /// reading `state()` after the call shows the new state.
    pub fn on_frame(&mut self, event: &DirectoryEvent) -> Result<(), FsmError> {
        match (&self.state, event) {
            (SubscriberState::Snapshotting, DirectoryEvent::Snapshot { .. }) => {
                self.state = SubscriberState::Pumping;
                Ok(())
            }
            (SubscriberState::Snapshotting, _) => {
                self.state = SubscriberState::Disconnected;
                Err(FsmError::ProtocolViolation(
                    "expected Snapshot frame; got incremental",
                ))
            }
            (SubscriberState::Pumping, DirectoryEvent::Snapshot { .. }) => {
                self.state = SubscriberState::Disconnected;
                Err(FsmError::ProtocolViolation(
                    "second Snapshot mid-stream",
                ))
            }
            (SubscriberState::Pumping, DirectoryEvent::Upsert { .. })
            | (SubscriberState::Pumping, DirectoryEvent::Remove { .. }) => {
                // Spec §2.3: backoff resets on the first non-
                // Heartbeat frame in a Pumping window.
                self.backoff_ms = BACKOFF_FLOOR_MS;
                Ok(())
            }
            (SubscriberState::Pumping, DirectoryEvent::Heartbeat { .. }) => {
                // Heartbeat alone is liveness; not evidence the
                // peer can serve real data. Backoff stays.
                Ok(())
            }
            // Frames arriving while Disconnected / Connecting /
            // Backoff are not theoretically possible if the per-
            // peer task drives the FSM correctly, but we surface
            // them as protocol violations to make incorrect
            // drivers loud.
            _ => {
                self.state = SubscriberState::Disconnected;
                Err(FsmError::ProtocolViolation(
                    "frame in disconnected/connecting/backoff state",
                ))
            }
        }
    }
}

impl Default for SubscriberFsm {
    fn default() -> Self {
        Self::new()
    }
}

// ── DirectoryView (PR-N3 N3-3) ─────────────────────────────────────

/// A peer hub's directory projection, keyed by agent_uri so
/// consumers can look up by URI in O(log n).
///
/// The receiving daemon's RemoteDirectoryClient owns one per
/// federated peer; reads are snapshot-cheap via the
/// SharedFederatedDirectoryView cell.
///
/// Entries in the view always carry `origin_realm =
/// Some(peer_realm)`. The §2.4 rewrite chokepoint
/// (`apply_frame`) stamps this on every entry before insertion,
/// regardless of what the peer's wire bytes claimed — so a
/// downstream consumer can rely on `origin_realm` reflecting the
/// peer's authenticated identity, not whatever the peer wrote.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct DirectoryView {
    /// Realm of the peer this view represents.
    pub peer_realm: String,
    /// agent_uri → entry. BTreeMap for deterministic iteration
    /// when projecting into a snapshot for consumers.
    pub entries: BTreeMap<String, DirectoryEntry>,
}

impl DirectoryView {
    #[must_use]
    pub fn new(peer_realm: String) -> Self {
        Self {
            peer_realm,
            entries: BTreeMap::new(),
        }
    }

    /// Lookup an entry by URI. `None` ⇔ not in this peer's view.
    #[must_use]
    pub fn lookup(&self, agent_uri: &str) -> Option<&DirectoryEntry> {
        self.entries.get(agent_uri)
    }

    /// **§2.4 origin_realm rewrite chokepoint.** Apply an
    /// inbound `DirectoryEvent` to this view, **stamping**
    /// `entry.origin_realm = Some(peer_realm)` on every Snapshot
    /// or Upsert entry before insertion. The peer's wire bytes'
    /// `origin_realm` field is overwritten regardless of what
    /// they claimed; combined with PR-N2's signing-key gate (the
    /// peer's signing key is bound to its own realm by DEC-N1
    /// §2.4 admission), cross-realm spoofing is blocked at two
    /// layers.
    pub fn apply_frame(&mut self, event: &DirectoryEvent) {
        match event {
            DirectoryEvent::Snapshot { entries } => {
                self.entries.clear();
                for raw in entries {
                    let entry = self.rewrite_origin(raw.clone());
                    self.entries.insert(entry.agent_uri.clone(), entry);
                }
            }
            DirectoryEvent::Upsert { entry } => {
                let entry = self.rewrite_origin(entry.clone());
                self.entries.insert(entry.agent_uri.clone(), entry);
            }
            DirectoryEvent::Remove { agent_uri, .. } => {
                self.entries.remove(agent_uri);
            }
            DirectoryEvent::Heartbeat { .. } => {
                // Keepalive is content-free for the view; the
                // RemoteDirectoryClient consumes it for liveness
                // only and never reaches apply_frame with a
                // Heartbeat in the steady state. This arm exists
                // so a stray Heartbeat-after-decode is a no-op
                // rather than a panic.
            }
        }
    }

    fn rewrite_origin(&self, mut entry: DirectoryEntry) -> DirectoryEntry {
        entry.origin_realm = Some(self.peer_realm.clone());
        entry
    }
}

// ── SharedFederatedDirectoryView (PR-N3 N3-3) ──────────────────────

/// A reload-friendly cell holding the daemon's current
/// federated-directory map keyed by peer realm.
///
/// Mirrors `SharedTrustAnchor` (commit 9/N) and
/// `SharedFederatedPeers` (commit 10/N). The inner
/// `Arc<BTreeMap<...>>` is the snapshot; readers
/// (`<self>.discover` Tier 3 fan-out, future `listDevices`
/// aggregation) call `.snapshot()` for an `Arc` clone that stays
/// stable for the duration of one read even if a per-peer
/// RemoteDirectoryClient task replaces the map mid-RPC.
#[derive(Clone, Debug)]
pub struct SharedFederatedDirectoryView {
    inner: Arc<RwLock<Arc<BTreeMap<String, Arc<DirectoryView>>>>>,
}

impl SharedFederatedDirectoryView {
    #[must_use]
    pub fn new(initial: BTreeMap<String, Arc<DirectoryView>>) -> Self {
        Self {
            inner: Arc::new(RwLock::new(Arc::new(initial))),
        }
    }

    /// Cheap-to-clone snapshot of the current directory map.
    /// Readers hold the returned `Arc` for the duration of a
    /// single fan-out; mid-fan-out replaces are not visible to
    /// the in-flight reader by construction.
    #[must_use]
    pub fn snapshot(&self) -> Arc<BTreeMap<String, Arc<DirectoryView>>> {
        Arc::clone(&self.inner.read().expect("rwlock poisoned"))
    }

    /// Atomic publish of a new map. The writer (per-peer task
    /// finishing a Snapshot apply, or the SIGHUP-driven
    /// federated_peers reload adding a new peer) calls this to
    /// republish; readers in flight keep the prior `Arc`.
    pub fn replace(&self, next: BTreeMap<String, Arc<DirectoryView>>) {
        let mut guard = self.inner.write().expect("rwlock poisoned");
        *guard = Arc::new(next);
    }
}

impl Default for SharedFederatedDirectoryView {
    fn default() -> Self {
        Self::new(BTreeMap::new())
    }
}

// ── RemoteDirectoryClient (PR-N3 N3-3 scaffold) ────────────────────

/// Per-peer remote directory subscriber.
///
/// **Spec v2 §3.1**. One instance per entry in
/// `SharedFederatedPeers`. Owns its own `SubscriberFsm` and a
/// shared handle to the daemon-wide `SharedFederatedDirectoryView`
/// cell; received frames feed into the cell so daemon-wide
/// readers (`<self>.discover` Tier 3 in N3-4, `listDevices`
/// aggregation in N3-6) see this peer's contribution.
///
/// This commit (N3-3) lands the **scaffold**: the struct, the
/// constructor, and the `apply_event` method that drives the
/// FSM + DirectoryView together with the §2.4 rewrite chokepoint.
/// The real tonic subscribe-stream dial that produces those
/// events lives in a follow-up commit (N3-3.1) that integrates
/// with `services::federation_client::CrossHubDialer`. The split
/// keeps the data-plane logic — which is the actual security
/// boundary — unit-testable without a tokio + tonic harness.
pub struct RemoteDirectoryClient {
    peer_realm: String,
    #[allow(dead_code)]
    peer_hub_uri: String,
    fsm: SubscriberFsm,
    view: DirectoryView,
}

impl RemoteDirectoryClient {
    #[must_use]
    pub fn new(peer_realm: String, peer_hub_uri: String) -> Self {
        let view = DirectoryView::new(peer_realm.clone());
        Self {
            peer_realm,
            peer_hub_uri,
            fsm: SubscriberFsm::new(),
            view,
        }
    }

    pub fn peer_realm(&self) -> &str {
        &self.peer_realm
    }

    pub fn fsm_state(&self) -> &SubscriberState {
        self.fsm.state()
    }

    pub fn view_snapshot(&self) -> &DirectoryView {
        &self.view
    }

    /// Process a single inbound frame against both the FSM (for
    /// state-machine correctness) and the view (for the actual
    /// directory state). Errors surface FSM protocol violations;
    /// the per-peer task that drives this MUST tear down + re-
    /// dial when an error returns.
    pub fn apply_event(&mut self, event: &DirectoryEvent) -> Result<(), FsmError> {
        self.fsm.on_frame(event)?;
        // FSM accepted the frame; it's safe to apply to the
        // view. Heartbeat is the only frame that doesn't mutate
        // the view (apply_frame's Heartbeat arm is a no-op).
        self.view.apply_frame(event);
        Ok(())
    }

    /// Successful dial transition. Mirrors `SubscriberFsm::on_dial_ok`.
    pub fn on_dial_ok(&mut self) {
        self.fsm.on_dial_ok();
    }

    /// Dial failure transition. Returns the next backoff in ms
    /// the caller should sleep before re-dial.
    pub fn on_dial_err(&mut self) -> u64 {
        self.fsm.on_dial_err();
        self.fsm.next_backoff_ms()
    }

    /// 60s no-frame timeout. The per-peer task that owns the
    /// stream calls this when its idle watcher fires; the FSM
    /// drops to Disconnected so the caller's outer loop re-dials.
    pub fn on_idle_timeout(&mut self) {
        self.fsm.on_idle_timeout();
    }

    pub fn on_stream_end(&mut self) {
        self.fsm.on_stream_end();
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn legacy_entry_json() -> &'static str {
        // The PR-N1 commit 8/N readers see this exact shape on
        // the wire. Legacy emit drops the schema-B fields
        // entirely; new emit includes them.
        r#"{
            "agent_uri": "easynet:///r/realm-a/agent/device-A",
            "node_id": "node-1",
            "display_name": "silan-laptop",
            "status": "active"
        }"#
    }

    fn full_entry_json() -> &'static str {
        r#"{
            "agent_uri": "easynet:///r/realm-a/agent/device-A",
            "node_id": "node-1",
            "display_name": "silan-laptop",
            "status": "active",
            "origin_realm": "realm-a",
            "hub_endpoint": "https://hub-a.example:50443",
            "last_seen_unix_ms": 1714492800000
        }"#
    }

    #[test]
    fn legacy_entry_deserialises_with_origin_realm_none() {
        // Schema-B forward-compat: a 4-field legacy entry
        // round-trips without errors and the new optional
        // fields surface as None / None / None.
        let entry: DirectoryEntry = serde_json::from_str(legacy_entry_json()).expect("deserialise");
        assert_eq!(entry.agent_uri, "easynet:///r/realm-a/agent/device-A");
        assert_eq!(entry.node_id, "node-1");
        assert_eq!(entry.display_name.as_deref(), Some("silan-laptop"));
        assert_eq!(entry.status, "active");
        assert_eq!(entry.origin_realm, None);
        assert_eq!(entry.hub_endpoint, None);
        assert_eq!(entry.last_seen_unix_ms, None);
    }

    #[test]
    fn full_entry_round_trips_all_fields() {
        let entry: DirectoryEntry = serde_json::from_str(full_entry_json()).expect("deserialise");
        assert_eq!(entry.origin_realm.as_deref(), Some("realm-a"));
        assert_eq!(
            entry.hub_endpoint.as_deref(),
            Some("https://hub-a.example:50443")
        );
        assert_eq!(entry.last_seen_unix_ms, Some(1_714_492_800_000));

        // Re-serialise and confirm the fields persist through
        // the round-trip. Field order is serde-determined; we
        // assert each key/value pair via JSON parse rather than
        // string match to stay byte-format-tolerant.
        let bytes = serde_json::to_vec(&entry).expect("serialise");
        let parsed: serde_json::Value = serde_json::from_slice(&bytes).expect("re-parse");
        assert_eq!(parsed["agent_uri"], "easynet:///r/realm-a/agent/device-A");
        assert_eq!(parsed["origin_realm"], "realm-a");
        assert_eq!(parsed["hub_endpoint"], "https://hub-a.example:50443");
        assert_eq!(parsed["last_seen_unix_ms"], 1_714_492_800_000_i64);
    }

    #[test]
    fn local_entry_serialised_with_none_fields_emits_nulls() {
        // Local entry (origin_realm = None, no hub_endpoint).
        // serde's default Option behaviour emits these as JSON
        // null. Legacy readers ignore unknown fields; null is
        // identically interpretable as "field absent" by the
        // schema-B convention.
        let local = DirectoryEntry {
            agent_uri: "easynet:///r/realm-a/agent/local-1".to_string(),
            node_id: "local-1".to_string(),
            display_name: None,
            status: "active".to_string(),
            origin_realm: None,
            hub_endpoint: None,
            last_seen_unix_ms: None,
        };
        let bytes = serde_json::to_vec(&local).expect("serialise");
        let parsed: serde_json::Value = serde_json::from_slice(&bytes).expect("re-parse");
        assert!(parsed["origin_realm"].is_null());
        assert!(parsed["hub_endpoint"].is_null());
        assert!(parsed["last_seen_unix_ms"].is_null());
    }

    // ── N3-2 DirectoryEvent + subscribe_directory FSM ────────────

    fn sample_entry() -> DirectoryEntry {
        DirectoryEntry {
            agent_uri: "easynet:///r/realm-a/agent/device-A".to_string(),
            node_id: "node-1".to_string(),
            display_name: Some("silan-laptop".to_string()),
            status: "active".to_string(),
            origin_realm: Some("realm-a".to_string()),
            hub_endpoint: Some("https://hub-a.example:50443".to_string()),
            last_seen_unix_ms: Some(1_714_492_800_000),
        }
    }

    #[test]
    fn directory_event_snapshot_serialises_with_type_tag() {
        let evt = DirectoryEvent::Snapshot {
            entries: vec![sample_entry()],
        };
        let bytes = serde_json::to_vec(&evt).expect("serialise snapshot");
        let parsed: serde_json::Value = serde_json::from_slice(&bytes).expect("re-parse");
        assert_eq!(parsed["type"], "snapshot");
        assert_eq!(parsed["entries"][0]["agent_uri"], sample_entry().agent_uri);
    }

    #[test]
    fn directory_event_upsert_remove_heartbeat_serialise_with_type_tag() {
        let upsert_bytes =
            serde_json::to_vec(&DirectoryEvent::Upsert { entry: sample_entry() }).unwrap();
        let upsert: serde_json::Value = serde_json::from_slice(&upsert_bytes).unwrap();
        assert_eq!(upsert["type"], "upsert");

        let remove_bytes = serde_json::to_vec(&DirectoryEvent::Remove {
            agent_uri: "easynet:///r/realm-a/agent/dropped".to_string(),
            reason: "shutdown".to_string(),
        })
        .unwrap();
        let remove: serde_json::Value = serde_json::from_slice(&remove_bytes).unwrap();
        assert_eq!(remove["type"], "remove");
        assert_eq!(remove["reason"], "shutdown");

        let hb_bytes = serde_json::to_vec(&DirectoryEvent::Heartbeat {
            sent_at_unix_ms: 1_714_492_800_000,
        })
        .unwrap();
        let hb: serde_json::Value = serde_json::from_slice(&hb_bytes).unwrap();
        assert_eq!(hb["type"], "heartbeat");
        assert_eq!(hb["sent_at_unix_ms"], 1_714_492_800_000_i64);
    }

    #[test]
    fn directory_event_round_trips_through_serde() {
        let original = DirectoryEvent::Upsert { entry: sample_entry() };
        let bytes = serde_json::to_vec(&original).expect("serialise");
        let restored: DirectoryEvent = serde_json::from_slice(&bytes).expect("deserialise");
        assert_eq!(original, restored);
    }

    // ── FSM ──

    #[test]
    fn fsm_starts_disconnected() {
        let fsm = SubscriberFsm::new();
        assert!(matches!(fsm.state(), &SubscriberState::Disconnected));
        assert_eq!(fsm.next_backoff_ms(), 1000);
    }

    #[test]
    fn fsm_dial_success_then_snapshot_promotes_to_pumping() {
        let mut fsm = SubscriberFsm::new();
        fsm.on_dial_ok();
        assert!(matches!(fsm.state(), &SubscriberState::Snapshotting));

        fsm.on_frame(&DirectoryEvent::Snapshot { entries: vec![] }).expect("snapshot ok");
        assert!(matches!(fsm.state(), &SubscriberState::Pumping));
    }

    #[test]
    fn fsm_second_snapshot_mid_stream_is_protocol_violation() {
        // Spec §2.3: a second Snapshot frame after the first
        // promotes-to-Pumping is a protocol violation; receiver
        // MUST drop the connection.
        let mut fsm = SubscriberFsm::new();
        fsm.on_dial_ok();
        fsm.on_frame(&DirectoryEvent::Snapshot { entries: vec![] }).expect("first snapshot ok");
        let err = fsm
            .on_frame(&DirectoryEvent::Snapshot { entries: vec![] })
            .expect_err("second snapshot must reject");
        assert!(matches!(err, FsmError::ProtocolViolation(_)));
        // Receiver moves to Disconnected so backoff drives a
        // rebuild.
        assert!(matches!(fsm.state(), &SubscriberState::Disconnected));
    }

    #[test]
    fn fsm_upsert_before_snapshot_is_protocol_violation() {
        // Spec §2.3: Snapshot is mandatory and exactly-once at
        // the front of every connection. Any incremental frame
        // arriving while still Snapshotting is a violation.
        let mut fsm = SubscriberFsm::new();
        fsm.on_dial_ok();
        let err = fsm
            .on_frame(&DirectoryEvent::Upsert { entry: sample_entry() })
            .expect_err("upsert before snapshot must reject");
        assert!(matches!(err, FsmError::ProtocolViolation(_)));
        assert!(matches!(fsm.state(), &SubscriberState::Disconnected));
    }

    #[test]
    fn fsm_pumping_accepts_upsert_remove_heartbeat() {
        let mut fsm = SubscriberFsm::new();
        fsm.on_dial_ok();
        fsm.on_frame(&DirectoryEvent::Snapshot { entries: vec![] }).expect("snapshot");
        for evt in [
            DirectoryEvent::Upsert { entry: sample_entry() },
            DirectoryEvent::Remove {
                agent_uri: "easynet:///r/realm-a/agent/x".to_string(),
                reason: "drop".to_string(),
            },
            DirectoryEvent::Heartbeat {
                sent_at_unix_ms: 1_714_492_800_000,
            },
        ] {
            fsm.on_frame(&evt).expect("pumping accepts");
            assert!(matches!(fsm.state(), &SubscriberState::Pumping));
        }
    }

    #[test]
    fn fsm_dial_failure_sets_backoff_and_state() {
        let mut fsm = SubscriberFsm::new();
        fsm.on_dial_err();
        assert!(matches!(fsm.state(), &SubscriberState::Backoff { .. }));
        assert_eq!(fsm.next_backoff_ms(), 2000);
        fsm.on_dial_err();
        assert_eq!(fsm.next_backoff_ms(), 4000);
        // Backoff caps at 60_000ms regardless of how many
        // failures we accumulate.
        for _ in 0..30 {
            fsm.on_dial_err();
        }
        assert_eq!(fsm.next_backoff_ms(), 60_000);
    }

    #[test]
    fn fsm_first_real_frame_resets_backoff() {
        let mut fsm = SubscriberFsm::new();
        fsm.on_dial_err();
        fsm.on_dial_err();
        assert_eq!(fsm.next_backoff_ms(), 4000);

        // After success-then-snapshot-then-real-event we go
        // back to the floor.
        fsm.on_dial_ok();
        fsm.on_frame(&DirectoryEvent::Snapshot { entries: vec![] }).unwrap();
        fsm.on_frame(&DirectoryEvent::Upsert { entry: sample_entry() }).unwrap();
        assert_eq!(fsm.next_backoff_ms(), 1000);
    }

    #[test]
    fn fsm_heartbeat_alone_does_not_reset_backoff() {
        // Spec §2.3 last paragraph: backoff resets "on any
        // successful Pumping transition that received at least
        // one non-Heartbeat frame". Heartbeat alone is liveness;
        // it is not evidence the peer can serve real data.
        let mut fsm = SubscriberFsm::new();
        fsm.on_dial_err();
        fsm.on_dial_err();
        assert_eq!(fsm.next_backoff_ms(), 4000);

        fsm.on_dial_ok();
        fsm.on_frame(&DirectoryEvent::Snapshot { entries: vec![] }).unwrap();
        fsm.on_frame(&DirectoryEvent::Heartbeat {
            sent_at_unix_ms: 1_714_492_800_000,
        })
        .unwrap();
        // Backoff stays at the next-step value.
        assert_eq!(fsm.next_backoff_ms(), 4000);
    }

    #[test]
    fn fsm_idle_timeout_drops_to_disconnected() {
        // Spec §2.3: 60s no-frame triggers reconnect. The FSM
        // surfaces this via on_idle_timeout; the FSM does not
        // own its own clock — the per-peer task drives it.
        let mut fsm = SubscriberFsm::new();
        fsm.on_dial_ok();
        fsm.on_frame(&DirectoryEvent::Snapshot { entries: vec![] }).unwrap();
        fsm.on_frame(&DirectoryEvent::Upsert { entry: sample_entry() }).unwrap();

        fsm.on_idle_timeout();
        assert!(matches!(fsm.state(), &SubscriberState::Disconnected));
    }

    #[test]
    fn round_trip_through_serde_preserves_field_equality() {
        // PartialEq derive lets us assert byte-stable round-
        // trips for testing receivers that compare entries to
        // detect changes between subscribe-stream snapshots.
        let original = DirectoryEntry {
            agent_uri: "easynet:///r/realm-b/agent/peer-device".to_string(),
            node_id: "peer-1".to_string(),
            display_name: Some("silan-phone".to_string()),
            status: "stale".to_string(),
            origin_realm: Some("realm-b".to_string()),
            hub_endpoint: Some("https://hub-b.example:50443".to_string()),
            last_seen_unix_ms: Some(1_714_500_000_000),
        };
        let bytes = serde_json::to_vec(&original).expect("serialise");
        let restored: DirectoryEntry = serde_json::from_slice(&bytes).expect("deserialise");
        assert_eq!(original, restored);
    }

    // ── N3-3 DirectoryView + §2.4 origin_realm rewrite ─────────

    fn entry_with_claimed_origin(uri: &str, claimed: Option<&str>) -> DirectoryEntry {
        DirectoryEntry {
            agent_uri: uri.to_string(),
            node_id: "n".to_string(),
            display_name: None,
            status: "active".to_string(),
            origin_realm: claimed.map(String::from),
            hub_endpoint: None,
            last_seen_unix_ms: None,
        }
    }

    #[test]
    fn apply_snapshot_rewrites_origin_realm_to_peer() {
        // §2.4 rewrite chokepoint. The peer's wire bytes claimed
        // `origin_realm = "trusted-bank"` (clearly malicious —
        // the peer is in realm-b). The receiver MUST overwrite
        // with the peer's authenticated realm before any in-
        // process consumer sees the entry.
        let mut view = DirectoryView::new("realm-b".to_string());
        view.apply_frame(&DirectoryEvent::Snapshot {
            entries: vec![entry_with_claimed_origin(
                "easynet:///r/realm-b/agent/peer-device",
                Some("trusted-bank"),
            )],
        });
        let stamped = view
            .lookup("easynet:///r/realm-b/agent/peer-device")
            .expect("entry stored");
        assert_eq!(
            stamped.origin_realm.as_deref(),
            Some("realm-b"),
            "spoofed origin_realm `trusted-bank` MUST be overwritten with the peer's realm"
        );
    }

    #[test]
    fn apply_upsert_rewrites_origin_realm_to_peer() {
        let mut view = DirectoryView::new("realm-b".to_string());
        view.apply_frame(&DirectoryEvent::Upsert {
            entry: entry_with_claimed_origin(
                "easynet:///r/realm-b/agent/peer-device",
                Some("realm-c"),
            ),
        });
        let stamped = view
            .lookup("easynet:///r/realm-b/agent/peer-device")
            .expect("entry stored");
        assert_eq!(stamped.origin_realm.as_deref(), Some("realm-b"));
    }

    #[test]
    fn apply_upsert_stamps_origin_realm_when_peer_omitted_it() {
        // Peer's bytes had `origin_realm = None` (the legacy
        // schema-A shape). The receiver still stamps the peer's
        // realm so consumers downstream cannot accidentally see
        // a None for a cross-realm entry.
        let mut view = DirectoryView::new("realm-b".to_string());
        view.apply_frame(&DirectoryEvent::Upsert {
            entry: entry_with_claimed_origin(
                "easynet:///r/realm-b/agent/peer-device",
                None,
            ),
        });
        let stamped = view
            .lookup("easynet:///r/realm-b/agent/peer-device")
            .expect("entry stored");
        assert_eq!(stamped.origin_realm.as_deref(), Some("realm-b"));
    }

    #[test]
    fn apply_remove_drops_entry_from_view() {
        let mut view = DirectoryView::new("realm-b".to_string());
        view.apply_frame(&DirectoryEvent::Snapshot {
            entries: vec![entry_with_claimed_origin(
                "easynet:///r/realm-b/agent/peer-device",
                None,
            )],
        });
        assert!(view
            .lookup("easynet:///r/realm-b/agent/peer-device")
            .is_some());
        view.apply_frame(&DirectoryEvent::Remove {
            agent_uri: "easynet:///r/realm-b/agent/peer-device".to_string(),
            reason: "shutdown".to_string(),
        });
        assert!(view
            .lookup("easynet:///r/realm-b/agent/peer-device")
            .is_none());
    }

    #[test]
    fn apply_snapshot_replaces_view_wholesale() {
        // Spec §2.2: receiver replaces its peer-keyed view
        // wholesale on Snapshot. Old entries that aren't in the
        // new snapshot disappear.
        let mut view = DirectoryView::new("realm-b".to_string());
        view.apply_frame(&DirectoryEvent::Upsert {
            entry: entry_with_claimed_origin("easynet:///r/realm-b/agent/old", None),
        });
        view.apply_frame(&DirectoryEvent::Snapshot {
            entries: vec![entry_with_claimed_origin(
                "easynet:///r/realm-b/agent/new",
                None,
            )],
        });
        assert!(view
            .lookup("easynet:///r/realm-b/agent/old")
            .is_none());
        assert!(view
            .lookup("easynet:///r/realm-b/agent/new")
            .is_some());
    }

    #[test]
    fn apply_heartbeat_is_noop_for_view() {
        let mut view = DirectoryView::new("realm-b".to_string());
        view.apply_frame(&DirectoryEvent::Upsert {
            entry: entry_with_claimed_origin("easynet:///r/realm-b/agent/peer", None),
        });
        let before = view.entries.clone();
        view.apply_frame(&DirectoryEvent::Heartbeat {
            sent_at_unix_ms: 1_714_500_000_000,
        });
        assert_eq!(view.entries, before, "heartbeat must not mutate the view");
    }

    // ── SharedFederatedDirectoryView ──────────────────────────

    #[test]
    fn shared_federated_directory_view_starts_empty() {
        let cell = SharedFederatedDirectoryView::default();
        let snap = cell.snapshot();
        assert!(snap.is_empty());
    }

    // ── RemoteDirectoryClient scaffold ──────────────────────

    #[test]
    fn remote_directory_client_starts_disconnected_with_empty_view() {
        let client = RemoteDirectoryClient::new(
            "realm-b".to_string(),
            "https://hub-b.example:50443".to_string(),
        );
        assert_eq!(client.peer_realm(), "realm-b");
        assert!(matches!(client.fsm_state(), &SubscriberState::Disconnected));
        assert!(client.view_snapshot().entries.is_empty());
    }

    #[test]
    fn remote_directory_client_apply_event_drives_fsm_and_view_together() {
        let mut client = RemoteDirectoryClient::new(
            "realm-b".to_string(),
            "https://hub-b.example:50443".to_string(),
        );
        client.on_dial_ok();
        client
            .apply_event(&DirectoryEvent::Snapshot {
                entries: vec![entry_with_claimed_origin(
                    "easynet:///r/realm-b/agent/peer",
                    Some("trusted-bank"), // spoofed; rewrite chokepoint catches
                )],
            })
            .expect("snapshot accepted");
        assert!(matches!(client.fsm_state(), &SubscriberState::Pumping));
        let stamped = client
            .view_snapshot()
            .lookup("easynet:///r/realm-b/agent/peer")
            .expect("entry stored");
        assert_eq!(
            stamped.origin_realm.as_deref(),
            Some("realm-b"),
            "RemoteDirectoryClient must enforce §2.4 origin_realm rewrite"
        );
    }

    #[test]
    fn remote_directory_client_apply_event_protocol_violation_does_not_mutate_view() {
        let mut client = RemoteDirectoryClient::new(
            "realm-b".to_string(),
            "https://hub-b.example:50443".to_string(),
        );
        client.on_dial_ok();
        // Upsert before Snapshot → ProtocolViolation. The FSM
        // drops to Disconnected; the view MUST stay empty so the
        // peer cannot inject entries by sending Upserts before
        // the mandatory Snapshot.
        let err = client
            .apply_event(&DirectoryEvent::Upsert {
                entry: entry_with_claimed_origin(
                    "easynet:///r/realm-b/agent/sneaky",
                    None,
                ),
            })
            .expect_err("upsert before snapshot must reject");
        assert!(matches!(err, FsmError::ProtocolViolation(_)));
        assert!(matches!(client.fsm_state(), &SubscriberState::Disconnected));
        assert!(
            client.view_snapshot().entries.is_empty(),
            "view MUST stay empty when FSM rejects the frame"
        );
    }

    #[test]
    fn remote_directory_client_dial_err_returns_growing_backoff() {
        let mut client = RemoteDirectoryClient::new(
            "realm-b".to_string(),
            "https://hub-b.example:50443".to_string(),
        );
        assert_eq!(client.on_dial_err(), 2_000);
        assert_eq!(client.on_dial_err(), 4_000);
        assert_eq!(client.on_dial_err(), 8_000);
    }

    #[test]
    fn shared_federated_directory_view_replace_publishes_atomically() {
        let cell = SharedFederatedDirectoryView::default();
        // Take snapshot 1 BEFORE replace.
        let snap1 = cell.snapshot();
        assert!(snap1.is_empty());

        let mut next = BTreeMap::new();
        let mut peer_view = DirectoryView::new("realm-b".to_string());
        peer_view.apply_frame(&DirectoryEvent::Upsert {
            entry: entry_with_claimed_origin("easynet:///r/realm-b/agent/peer", None),
        });
        next.insert("realm-b".to_string(), Arc::new(peer_view));
        cell.replace(next);

        // snap1 still observes the empty pre-replace state.
        assert!(snap1.is_empty(), "in-flight reader sees pre-replace state");

        // A fresh snapshot sees the new map.
        let snap2 = cell.snapshot();
        assert_eq!(snap2.len(), 1);
        assert!(snap2
            .get("realm-b")
            .expect("realm-b present")
            .lookup("easynet:///r/realm-b/agent/peer")
            .is_some());
    }
}
