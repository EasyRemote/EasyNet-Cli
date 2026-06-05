// EasyNet CLI — Cross-realm directory federation types (RFC-N3)
// ===============================================================
//
// File: src/services/federation_directory.rs
// Description: Runtime projection helpers for the cross-realm
//              directory federation surface introduced by PR-N3
//              (`pr-drafts/PR-N3-spec-cross-realm-directory-v2.md`).
//              The wire shapes themselves are Axon SDK exports.
//
//              PR-N3 originally landed these as CLI-local serde
//              structs. F-07 de-forks that contract:
//              `DirectoryEntry` and `DirectoryEvent` are imported
//              from `easynet-axon`; this module keeps only
//              projection, merge, and runtime-view mechanics.
//
// Why a new module
// ----------------
// `federation_wrappers.rs` hosts the original PR-1 `federation.*`
// ability surface (`AgentSummary`, `JoinResponse`, etc.) which
// represents *presence* — "is this URA online right now". The
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

type RealmDirectoryMap = BTreeMap<String, Arc<DirectoryView>>;
type SharedRealmDirectoryMap = Arc<RealmDirectoryMap>;

pub use easynet_axon::federation_directory::{
    DirectoryAgentSummary, DirectoryEntry, DirectoryEvent, ListUserDevicesRequest,
    ListUserDevicesResponse, SigningAuthority,
};

// ── PresenceEvent → DirectoryEvent adapter (PR-N3 N3-streaming-1) ──

/// Project a single presence-registry URA into a `DirectoryEntry`
/// suitable for legacy discover/list projections. The projection is
/// pure — given a URA string and an
/// `is_active` flag, returns a deterministic entry shape.
///
/// `origin_realm` is `None` because the local hub speaks for its
/// own realm (the §2.4 chokepoint stamps it on the receive side
/// when this entry crosses to a peer). `display_name` /
/// `hub_endpoint` / `last_seen_unix_ms` are `None` in the
/// presence-only projection — the registry knows URAs and online
/// state, nothing richer. Future enrichment (joining device-
/// pairing rows for display_name / last-seen) is N3-6 backend-Go
/// territory.
///
/// `node_id` is parsed from the URA tail. Canonical v4.1.4 device
/// URAs use `/device/<node>`. Non-canonical URAs (which should not
/// appear in the registry, but defensive handling matters) get
/// `node_id = agent_ura.clone()` so downstream consumers always
/// have a non-empty key.
#[cfg(feature = "axon-pb")]
#[must_use]
pub fn presence_ura_to_directory_entry(agent_ura: &str, is_active: bool) -> DirectoryEntry {
    DirectoryEntry {
        agent_ura: agent_ura.to_string(),
        node_id: agent_ura_to_node_id(agent_ura),
        display_name: None,
        status: if is_active { "active" } else { "stale" }.to_string(),
        origin_realm: None,
        hub_endpoint: None,
        last_seen_unix_ms: None,
    }
}

#[cfg(feature = "axon-pb")]
#[must_use]
pub fn presence_ura_to_directory_agent_summary(
    agent_ura: &str,
    is_active: bool,
) -> DirectoryAgentSummary {
    DirectoryAgentSummary {
        agent_ura: agent_ura.to_string(),
        signing_authority: SigningAuthority::SelfSigned,
        status: if is_active { "active" } else { "stale" }.to_string(),
        ability_count: 0,
    }
}

#[cfg(feature = "axon-pb")]
#[must_use]
pub fn presence_uras_to_directory_snapshot<I>(uras: I, snapshot_unix_ms: i64) -> DirectoryEvent
where
    I: IntoIterator,
    I::Item: AsRef<str>,
{
    let agents = uras
        .into_iter()
        .map(|ura| presence_ura_to_directory_agent_summary(ura.as_ref(), true))
        .collect();
    DirectoryEvent::Snapshot {
        agents,
        snapshot_unix_ms,
    }
}

#[cfg(feature = "axon-pb")]
#[must_use]
pub fn directory_agent_summary_to_entry(
    agent: &DirectoryAgentSummary,
    peer_realm: &str,
) -> DirectoryEntry {
    DirectoryEntry {
        agent_ura: agent.agent_ura.clone(),
        node_id: agent_ura_to_node_id(&agent.agent_ura),
        display_name: None,
        status: agent.status.clone(),
        origin_realm: Some(peer_realm.to_string()),
        hub_endpoint: None,
        last_seen_unix_ms: None,
    }
}

#[cfg(feature = "axon-pb")]
#[must_use]
fn agent_ura_to_node_id(agent_ura: &str) -> String {
    match crate::ura::parse_ura(agent_ura) {
        Ok(parsed) if parsed.kind == crate::ura::URAKind::Device => parsed.device_id,
        _ => agent_ura.to_string(),
    }
}

#[cfg(feature = "axon-pb")]
#[must_use]
pub fn now_unix_ms() -> i64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_millis() as i64)
        .unwrap_or(0)
}

/// Convert a single `PresenceEvent` into the corresponding
/// `DirectoryEvent`. `Online` projects to `Upsert`; `Offline`
/// projects to `Remove` with the registry's `OfflineReason`
/// stringified into the `reason` field for operator audit.
///
/// **Spec v2 §3.3 broadcast pump**. The hub's
/// `subscribe_directory_v2` server stream wraps a per-subscriber
/// `broadcast::Receiver<PresenceEvent>` with this adapter so the
/// outbound frames carry the `DirectoryEvent` wire shape rather
/// than the legacy `AgentSummary` shape.
#[cfg(feature = "axon-pb")]
#[must_use]
pub fn presence_event_to_directory_event(
    event: &crate::services::presence_registry::PresenceEvent,
) -> DirectoryEvent {
    presence_event_to_directory_event_at(event, now_unix_ms())
}

#[cfg(feature = "axon-pb")]
#[must_use]
pub fn presence_event_to_directory_event_at(
    event: &crate::services::presence_registry::PresenceEvent,
    unix_ms: i64,
) -> DirectoryEvent {
    use crate::services::presence_registry::PresenceEvent;
    match event {
        PresenceEvent::Online { ura } => DirectoryEvent::AgentAdvertised {
            agent_ura: ura.clone(),
            signing_authority: SigningAuthority::SelfSigned,
            replaced_prior: false,
            unix_ms,
        },
        PresenceEvent::Offline { ura, reason } => DirectoryEvent::AgentRevoked {
            agent_ura: ura.clone(),
            was_active: true,
            // `OfflineReason::as_wire_str` is the single source of
            // truth for the snake_case label; both this projection
            // and the op-event `reason=` field share it so an SRE
            // pipeline grepping `reason=stream_closed` matches in
            // both surfaces.
            reason: reason.as_wire_str().to_string(),
            unix_ms,
        },
    }
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
    Backoff {
        delay_ms: u64,
    },
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
                Err(FsmError::ProtocolViolation("second Snapshot mid-stream"))
            }
            (SubscriberState::Pumping, DirectoryEvent::AgentAdvertised { .. })
            | (SubscriberState::Pumping, DirectoryEvent::AbilitiesAdvertised { .. })
            | (SubscriberState::Pumping, DirectoryEvent::AgentRevoked { .. }) => {
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

/// A peer hub's directory projection, keyed by agent_ura so
/// consumers can look up by URA in O(log n).
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
    /// agent_ura → entry. BTreeMap for deterministic iteration
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

    /// Lookup an entry by URA. `None` ⇔ not in this peer's view.
    #[must_use]
    pub fn lookup(&self, agent_ura: &str) -> Option<&DirectoryEntry> {
        self.entries.get(agent_ura)
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
            DirectoryEvent::Snapshot { agents, .. } => {
                self.entries.clear();
                for raw in agents {
                    let entry = directory_agent_summary_to_entry(raw, &self.peer_realm);
                    self.entries.insert(entry.agent_ura.clone(), entry);
                }
            }
            DirectoryEvent::AgentAdvertised {
                agent_ura,
                signing_authority,
                ..
            } => {
                let summary = DirectoryAgentSummary {
                    agent_ura: agent_ura.clone(),
                    signing_authority: signing_authority.clone(),
                    status: "active".to_string(),
                    ability_count: 0,
                };
                let entry = directory_agent_summary_to_entry(&summary, &self.peer_realm);
                self.entries.insert(entry.agent_ura.clone(), entry);
            }
            DirectoryEvent::AbilitiesAdvertised { .. } => {
                // `DirectoryEntry` has no ability-count field. The
                // stream contract says ability advertisements follow
                // an agent advertisement; richer details are resolved
                // through `federation.resolve`, not invented here.
            }
            DirectoryEvent::AgentRevoked { agent_ura, .. } => {
                self.entries.remove(agent_ura);
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

    /// Replace this view from `federation.discover` rows. This is
    /// intentionally separate from `apply_frame`: discover returns
    /// `DirectoryEntry` rows, while the v2 stream now emits
    /// `DirectoryAgentSummary` rows.
    pub fn replace_entries<I>(&mut self, entries: I)
    where
        I: IntoIterator<Item = DirectoryEntry>,
    {
        self.entries.clear();
        for raw in entries {
            let entry = self.rewrite_origin(raw);
            self.entries.insert(entry.agent_ura.clone(), entry);
        }
    }

    fn rewrite_origin(&self, mut entry: DirectoryEntry) -> DirectoryEntry {
        entry.origin_realm = Some(self.peer_realm.clone());
        entry
    }

    /// **PR-N3 N3-streaming-12**. Mark every entry in the view
    /// as `status = "stale"`. The streaming supervisor calls
    /// this after a stream-end before publishing to the cell;
    /// readers see the peer's last-known entries flagged as
    /// possibly-out-of-date until the next successful
    /// Snapshot flips them back to `"active"` (or removes
    /// them).
    ///
    /// Spec §2.1 status enum: `"active" | "stale" |
    /// "draining"`. The streaming wire never directly emits
    /// "stale" — that's purely a receiver-side annotation
    /// driven by transport-level disconnect signal.
    pub fn mark_all_stale(&mut self) {
        for entry in self.entries.values_mut() {
            entry.status = "stale".to_string();
        }
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
    inner: Arc<RwLock<SharedRealmDirectoryMap>>,
    /// **PR-N3 N3-streaming-10**. Peers currently being kept
    /// up-to-date by the streaming supervisor. The poll task
    /// (N3-3.1 fallback) skips entries in this set so the two
    /// transports never race to publish into the same realm
    /// slot. Streaming is the authoritative source whenever
    /// the supervisor's stream is open; on stream-end the
    /// supervisor removes its realm + the poll task picks up
    /// the slack until the next reconnect.
    ///
    /// Wrapped in its own `RwLock` rather than folded into the
    /// directory map's RwLock so streamed-set reads (the poll
    /// task does this on every iteration) don't compete with
    /// directory writes (the supervisor does this on every
    /// applied frame). Two locks, two contended paths, no
    /// cross-blocking.
    streamed_peers: Arc<RwLock<std::collections::BTreeSet<String>>>,
}

impl SharedFederatedDirectoryView {
    #[must_use]
    pub fn new(initial: BTreeMap<String, Arc<DirectoryView>>) -> Self {
        Self {
            inner: Arc::new(RwLock::new(Arc::new(initial))),
            streamed_peers: Arc::new(RwLock::new(std::collections::BTreeSet::new())),
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

    /// **PR-N3 N3-streaming-10**. Mark a peer realm as actively
    /// streamed. The streaming supervisor calls this after a
    /// successful subscribe-stream open. The poll task
    /// (`poll_once`) skips peers in the set so the two
    /// transports never race to publish into the same realm
    /// slot.
    pub fn mark_streamed(&self, peer_realm: &str) {
        self.streamed_peers
            .write()
            .expect("rwlock poisoned")
            .insert(peer_realm.to_string());
    }

    /// **PR-N3 N3-streaming-10**. Unmark a peer realm.
    /// Streaming supervisor calls this on every stream-end so
    /// the poll task can pick up the slack until the next
    /// reconnect.
    pub fn unmark_streamed(&self, peer_realm: &str) {
        self.streamed_peers
            .write()
            .expect("rwlock poisoned")
            .remove(peer_realm);
    }

    /// **PR-N3 N3-streaming-10**. Is this peer realm currently
    /// being kept fresh by the streaming supervisor? Used by
    /// `poll_once` to skip peers whose stream is alive.
    #[must_use]
    pub fn is_streamed(&self, peer_realm: &str) -> bool {
        self.streamed_peers
            .read()
            .expect("rwlock poisoned")
            .contains(peer_realm)
    }
}

impl Default for SharedFederatedDirectoryView {
    fn default() -> Self {
        Self::new(BTreeMap::new())
    }
}

// ── Tier-3 fan-out (PR-N3 N3-3 / N3-4) ─────────────────────────────

/// Look up `query_ura` across every federated peer's directory
/// view. Returns the matching `DirectoryEntry` with the peer's
/// `origin_realm` already stamped (the §2.4 chokepoint runs on
/// the write side via `DirectoryView::apply_frame`, so reads are
/// just lookup).
///
/// **Spec v2 §3.2 + DEC-N4 §2.3**. The fan-out semantics:
/// - Iterate peers in lex order on `peer_realm` so tie-break is
///   deterministic when two peers both claim a hit.
/// - First-success-wins: the lowest-realm peer that has the URA
///   returns its entry. The directory is a *projection*; if the
///   same URA appeared on multiple peers it would indicate a
///   misconfiguration anyway (each device has exactly one
///   origin hub by construction).
/// - Dedupe by `agent_ura` is implicit because we return on
///   first hit.
///
/// Returns `None` when no peer has the URA. The caller (e.g. the
/// `<self>.discover` Tier-3 arm or backend `listDevices`)
/// projects the entry into its surface shape.
#[must_use]
pub fn lookup_in_federated_view(
    cell: &SharedFederatedDirectoryView,
    query_ura: &str,
) -> Option<DirectoryEntry> {
    let snapshot = cell.snapshot();
    // BTreeMap iteration is naturally lex-sorted on the key
    // (peer_realm), which gives the spec's deterministic tie-
    // break for free.
    for (_peer_realm, view) in snapshot.iter() {
        if let Some(entry) = view.lookup(query_ura) {
            return Some(entry.clone());
        }
    }
    None
}

/// Project the entire federated directory into a flat
/// `Vec<DirectoryEntry>` for surfaces that want to enumerate
/// every reachable device — `easynet device list`,
/// `<self>.discover` with no specific URA filter, the backend
/// `listDevices` aggregation. Each entry already carries its
/// `origin_realm` per §2.4.
///
/// Iteration order: peers in lex order on `peer_realm`, entries
/// in lex order on `agent_ura` (BTreeMap value iteration). Stable
/// across invocations on the same snapshot so the CLI prints
/// in a deterministic order.
#[must_use]
pub fn flatten_federated_view(cell: &SharedFederatedDirectoryView) -> Vec<DirectoryEntry> {
    let snapshot = cell.snapshot();
    let mut out = Vec::new();
    for (_peer_realm, view) in snapshot.iter() {
        for entry in view.entries.values() {
            out.push(entry.clone());
        }
    }
    out
}

// ── Directory poll task (PR-N3 N3-3.1) ─────────────────────────────

/// Outcome of a single poll cycle. Returned by `poll_once`
/// rather than logged inside so the boot-time spawn task can
/// surface a structured trace, and the unit tests can assert
/// per-peer success/failure without scraping stderr.
#[cfg(feature = "axon-pb")]
#[derive(Debug, Default)]
pub struct PollOutcome {
    /// Peers whose discover call succeeded; their realms.
    pub successful_peers: Vec<String>,
    /// Peers whose discover call failed; (realm, error string).
    pub failed_peers: Vec<(String, String)>,
}

/// Run one round of cross-realm directory polling against every
/// federated peer in the supplied `peers_snapshot`, writing the
/// resulting per-peer `DirectoryView` projections into the
/// `directory_cell`.
///
/// **PR-N3 commit N3-3.1**. The polling-based integration that
/// turns N3-3's data-plane scaffold into a real working chain.
/// Called periodically from a tokio task spawned at daemon
/// boot; calling cadence drives the
/// "new peer appears in discover within ~5s" acceptance from
/// PR-N3 spec §八 scenario (4).
///
/// Per peer the task:
///   1. Builds an `InvokeRequest` for `federation.discover` with
///      a loopback envelope (bypass admission via the daemon's
///      own URA; the peer accepts). Future signed-envelope
///      version uses PR-N5's audit-bound caller binding.
///   2. Dials the peer's hub via the supplied `FederationClient`.
///   3. Parses the `DiscoverResponse`, projects each entry into
///      the peer's `DirectoryView` (the §2.4 origin_realm
///      rewrite stamps the peer's authenticated realm).
///   4. Writes the new view into the `directory_cell`. Other
///      peers' views in the cell are preserved verbatim — the
///      replace is per-peer, not whole-map.
///
/// Errors per peer surface in `PollOutcome.failed_peers`; one
/// peer's failure does not abort the round. Spec §3.1 backoff
/// schedule lives in the FSM-driven streaming variant (which
/// supersedes this poll task whenever it lands); the poll task
/// just retries on the next interval.
#[cfg(feature = "axon-pb")]
pub async fn poll_once(
    federation_client: &dyn crate::services::federation_client::FederationClient,
    peers_snapshot: &std::collections::BTreeMap<String, String>,
    daemon_ura: Option<&str>,
    directory_cell: &SharedFederatedDirectoryView,
) -> PollOutcome {
    use easynet_axon::pb::axon::v1::{AgentIdentity as PbAgentIdentity, Envelope, InvokeRequest};

    let mut outcome = PollOutcome::default();
    // Start from the current cell so per-peer replaces preserve
    // entries from peers we don't poll this round (eg. removed
    // from federated_peers between snapshot fetch and now).
    let mut next_map: std::collections::BTreeMap<String, Arc<DirectoryView>> =
        (*directory_cell.snapshot()).clone();

    for (peer_realm, peer_hub_endpoint) in peers_snapshot.iter() {
        // **PR-N3 N3-streaming-10**: skip peers whose stream
        // is currently open. The streaming supervisor is the
        // authoritative source of truth for those peers; the
        // poll task is the fallback for peers without v2
        // support or peers in reconnect-backoff. Skipping
        // here prevents a stale poll snapshot from
        // overwriting a fresh stream-emitted Upsert/Remove.
        if directory_cell.is_streamed(peer_realm) {
            outcome.successful_peers.push(peer_realm.clone());
            continue;
        }

        // Build a discover request with the daemon's own URA as
        // caller so the peer's loopback bypass / hub-trust check
        // admits (caller-side strict signing lands in N3-3.2 with
        // the cross-realm CallerBinding from PR-N5 audit chain).
        let envelope = daemon_ura.map(|ura| Envelope {
            caller: Some(PbAgentIdentity {
                ura: ura.to_string(),
                profile: "easynet-strict-v2".to_string(),
            }),
            ..Envelope::default()
        });
        let request = InvokeRequest {
            envelope,
            function_name:
                crate::services::invocation_transport::federation_wrappers::ABILITY_FEDERATION_DISCOVER
                    .to_string(),
            arguments: br#"{}"#.to_vec(),
            ..InvokeRequest::default()
        };

        match federation_client
            .forward_invoke(peer_hub_endpoint, request)
            .await
        {
            Ok(response) => {
                let parsed: Result<
                    crate::services::invocation_transport::federation_wrappers::DiscoverResponse,
                    _,
                > = serde_json::from_slice(&response.result);
                match parsed {
                    Ok(discover) => {
                        let mut view = DirectoryView::new(peer_realm.clone());
                        view.replace_entries(discover.entries);
                        next_map.insert(peer_realm.clone(), Arc::new(view));
                        outcome.successful_peers.push(peer_realm.clone());
                    }
                    Err(err) => {
                        outcome
                            .failed_peers
                            .push((peer_realm.clone(), format!("response parse failed: {err}")));
                    }
                }
            }
            Err(err) => {
                outcome
                    .failed_peers
                    .push((peer_realm.clone(), format!("dial failed: {err:?}")));
            }
        }
    }

    directory_cell.replace(next_map);
    outcome
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
    fsm: SubscriberFsm,
    view: DirectoryView,
}

impl RemoteDirectoryClient {
    #[must_use]
    pub fn new(peer_realm: String) -> Self {
        let view = DirectoryView::new(peer_realm.clone());
        Self {
            peer_realm,
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

    /// **PR-N3 N3-streaming-2**. Publish the current per-peer
    /// `DirectoryView` into the supplied
    /// `SharedFederatedDirectoryView` cell. Read-modify-write
    /// of the cell's snapshot map: replace this peer's slot
    /// with the fresh view, preserve all other peers'
    /// existing slots verbatim. Atomic publish via the cell's
    /// `replace` so concurrent readers either see the
    /// pre-update or post-update map, never a mid-write
    /// state.
    ///
    /// The streaming consumer (a per-peer tokio task that
    /// drives the FSM via `apply_event` for each inbound
    /// frame) calls this after every successful Pumping
    /// transition so the daemon-wide cell reflects the peer
    /// state with sub-second latency.
    pub fn publish_to_cell(&self, cell: &SharedFederatedDirectoryView) {
        let current = cell.snapshot();
        let mut next: BTreeMap<String, Arc<DirectoryView>> = (*current).clone();
        next.insert(self.peer_realm.clone(), Arc::new(self.view.clone()));
        cell.replace(next);
    }

    /// **PR-N3 N3-streaming-12**. Mark every entry in this
    /// peer's local view as `status = "stale"` and publish to
    /// the cell. The supervisor calls this after a stream-end
    /// (StreamEnded / IdleTimeout / ProtocolViolation) so
    /// readers see the peer's last-known entries flagged
    /// possibly-out-of-date until the next successful stream
    /// reconnect's Snapshot replaces the view wholesale.
    ///
    /// Idempotent: calling on an already-stale view is a no-op
    /// at the wire level (same status string), so a supervisor
    /// stuck in backoff cycles doesn't churn the cell.
    pub fn mark_stale_and_publish(&mut self, cell: &SharedFederatedDirectoryView) {
        self.view.mark_all_stale();
        self.publish_to_cell(cell);
    }
}

/// **PR-N3 N3-streaming-4**. Run the per-peer streaming
/// supervisor loop. Opens a `subscribe_directory_v2` stream
/// against the peer, drives `consume_directory_event_stream`,
/// reconnects on stream-end with FSM backoff. Exits when:
///   - the cancel signal fires (operator removed the peer via
///     SIGHUP, or daemon shutdown).
///   - the FSM rejects a frame as a protocol violation; the
///     supervisor tears down the stream + reconnects after
///     backoff (a misbehaving peer's stream churn does not
///     escalate into a busy-loop because backoff is doubled
///     on every redial).
///
/// The supervisor itself never returns a value — errors are
/// surfaced via stderr trace. Production callers spawn this
/// as a tokio task and never await it; the task lives for the
/// daemon's lifetime (or the peer's entry in
/// `SharedFederatedPeers`, whichever ends first).
#[cfg(feature = "axon-pb")]
pub async fn run_per_peer_supervisor(
    peer_realm: String,
    peer_hub_endpoint: String,
    caller_ura: String,
    federation_client: std::sync::Arc<dyn crate::services::federation_client::FederationClient>,
    cell: SharedFederatedDirectoryView,
    cancel: tokio::sync::oneshot::Receiver<()>,
) {
    // Default production cadence: 60s receiver-side idle
    // timeout (spec §2.3 = two missed 30s heartbeat windows).
    run_per_peer_supervisor_with_idle_timeout(
        peer_realm,
        peer_hub_endpoint,
        caller_ura,
        federation_client,
        cell,
        cancel,
        60_000,
    )
    .await
}

/// **PR-N3 N3-streaming-9**. Reconcile a per-peer supervisor
/// map against a fresh `SharedFederatedPeers` snapshot. Pure
/// data shaping — the function:
///
///   - Iterates `snapshot.iter()` and for any peer realm not
///     yet in `active`, calls `spawn(realm, hub_endpoint)` to start
///     a fresh supervisor; stores the returned cancel sender
///     in `active`.
///   - Iterates `active.keys()` and for any realm no longer in
///     the snapshot, fires `cancel_tx.send(())` and removes the
///     entry from `active`.
///
/// Returns `(spawned, cancelled)` realm vectors for caller
/// observability (eprintln traces in production, test assertions
/// in unit tests). Decoupling the reconcile-step from the
/// 2s-interval driver makes the contract testable without a
/// tokio scheduler race.
///
/// `spawn` is a closure rather than a trait so callers can
/// capture whatever per-peer state they own (federation_client
/// Arc, directory cell, etc) without contorting through a
/// generic parameter list. Production calls
/// `tokio::spawn(run_per_peer_supervisor(...))` and returns the
/// `cancel_tx`; tests record the call args + return a fresh
/// oneshot::Sender that the test then awaits to confirm
/// cancellation fired.
pub fn reconcile_streaming_supervisors<F>(
    snapshot: &std::collections::BTreeMap<String, String>,
    active: &mut std::collections::BTreeMap<String, tokio::sync::oneshot::Sender<()>>,
    mut spawn: F,
) -> (Vec<String>, Vec<String>)
where
    F: FnMut(&str, &str) -> tokio::sync::oneshot::Sender<()>,
{
    let mut spawned = Vec::new();
    let mut cancelled = Vec::new();

    // Spawn supervisors for newly-added peers.
    for (peer_realm, peer_hub_endpoint) in snapshot.iter() {
        if active.contains_key(peer_realm) {
            continue;
        }
        let cancel_tx = spawn(peer_realm, peer_hub_endpoint);
        active.insert(peer_realm.clone(), cancel_tx);
        spawned.push(peer_realm.clone());
    }

    // Cancel supervisors for peers no longer in the cell.
    let removed: Vec<String> = active
        .keys()
        .filter(|realm| !snapshot.contains_key(realm.as_str()))
        .cloned()
        .collect();
    for realm in removed {
        if let Some(cancel_tx) = active.remove(&realm) {
            let _ = cancel_tx.send(());
            cancelled.push(realm);
        }
    }

    (spawned, cancelled)
}

/// **PR-N3 N3-streaming-8**. Variant of
/// `run_per_peer_supervisor` with a tunable idle-timeout
/// window. Production callers stick with the default
/// `run_per_peer_supervisor` (60 000ms); integration tests
/// pass a sub-second value to drive the IdleTimeout reconnect
/// path in real time.
pub async fn run_per_peer_supervisor_with_idle_timeout(
    peer_realm: String,
    peer_hub_endpoint: String,
    caller_ura: String,
    federation_client: std::sync::Arc<dyn crate::services::federation_client::FederationClient>,
    cell: SharedFederatedDirectoryView,
    mut cancel: tokio::sync::oneshot::Receiver<()>,
    idle_timeout_ms: u64,
) {
    use easynet_axon::pb::axon::v1::{
        AgentIdentity, Envelope, InvokeServerStreamRequest, SubjectIdentity,
    };
    use rand::RngCore;

    let mut client = RemoteDirectoryClient::new(peer_realm.clone());
    loop {
        // Honour cancel before doing anything expensive.
        if cancel.try_recv().is_ok() {
            return;
        }

        // Build a request with a populated envelope. The peer's
        // `dispatch_invoke_stream` admission rejects with
        // `InvalidArgument: InvokeStream request missing
        // envelope` if either the envelope or its caller /
        // callee / subject / nonce fields are absent. We mirror
        // the same shape the CLI bridge uses for forward_invoke:
        // caller URA = this daemon's own URA, callee + subject =
        // the peer's hub URA as the address being subscribed to,
        // and a fresh 16-byte invocation nonce per dial. The
        // CrossHubDialer applies its own trust gate (TLS pin)
        // before the request reaches the peer's admission, so
        // the peer's strict admission is defence-in-depth.
        let mut nonce = vec![0u8; 16];
        rand::rngs::OsRng.fill_bytes(&mut nonce);
        // URA v4.1.4: peer hub is the realm-singleton; no sub-id tail.
        let peer_ura_for_envelope = crate::ura::hub_ura(&peer_realm);
        // v4.1.5 §A.URA-7 — `subject ∈ {user, device, resource}`.
        // Pre-fix this site set `subject = peer_ura_for_envelope` (the
        // peer hub URA), which violates the constraint (hub is not a
        // legal subject kind). The natural legal subject for "this
        // daemon subscribes to peer hub's directory" is the local
        // daemon's own device URA (which equals `caller_ura`); peer
        // admission's strict 4-step verify still cross-checks the
        // signature against the trust anchor entry for this device.
        let envelope = Envelope {
            caller: Some(AgentIdentity {
                ura: caller_ura.clone(),
                ..AgentIdentity::default()
            }),
            callee: Some(AgentIdentity {
                ura: peer_ura_for_envelope,
                ..AgentIdentity::default()
            }),
            subject: Some(SubjectIdentity {
                ura: caller_ura.clone(),
                ..SubjectIdentity::default()
            }),
            invocation_nonce: nonce,
            ..Envelope::default()
        };
        let request = InvokeServerStreamRequest {
            envelope: Some(envelope),
            function_name: crate::services::invocation_transport::federation_wrappers
                ::ABILITY_FEDERATION_SUBSCRIBE_DIRECTORY_V2
                .to_string(),
            ..InvokeServerStreamRequest::default()
        };

        match federation_client
            .subscribe_directory_v2(&peer_hub_endpoint, request)
            .await
        {
            Ok(stream) => {
                client.on_dial_ok();
                // **PR-N3 N3-streaming-10**: claim authoritative
                // ownership of this peer's directory slot while
                // the stream is open. The poll task skips peers
                // in this set so the two transports never race
                // to publish.
                cell.mark_streamed(&peer_realm);
                // PR-N3 N3-streaming-7 + N3-streaming-8: enforce
                // the spec §2.3 idle-timeout. Production cadence
                // is 60s (= two missed 30s heartbeat windows);
                // integration tests pass a smaller window via
                // `run_per_peer_supervisor_with_idle_timeout`.
                let consume = consume_directory_event_stream_with_idle_timeout(
                    &mut client,
                    &cell,
                    stream,
                    idle_timeout_ms,
                );
                tokio::select! {
                    outcome = consume => {
                        match outcome {
                            ConsumeOutcome::StreamEnded => {
                                // Peer closed; fall through to
                                // reconnect with backoff.
                            }
                            ConsumeOutcome::ProtocolViolation(reason) => {
                                // `peer_realm: String` — pass verbatim so SRE
                                // pipelines see `peer_realm=tenant-a`, not the
                                // double-quoted `peer_realm="tenant-a"` Debug form.
                                // op_event!'s formatter auto-quotes values
                                // containing whitespace; bare strings pass through.
                                crate::op_event!(
                                    component = federation_directory,
                                    kind = subscribe_directory_v2_protocol_violation,
                                    peer_realm = peer_realm,
                                    error = reason,
                                    message = "tearing down + reconnecting",
                                );
                            }
                            ConsumeOutcome::IdleTimeout => {
                                crate::op_event!(
                                    component = federation_directory,
                                    kind = subscribe_directory_v2_idle_timeout,
                                    peer_realm = peer_realm,
                                    idle_timeout_ms = idle_timeout_ms,
                                    message = "reconnecting",
                                );
                            }
                        }
                    }
                    _ = &mut cancel => {
                        // Always release the stream-claim on
                        // exit, even via cancel — otherwise a
                        // peer-removal SIGHUP would leave a
                        // stale claim that blocks the poll task
                        // from picking up the realm in a
                        // re-add cycle. We do NOT mark stale
                        // on cancel: a peer-removed via SIGHUP
                        // means the operator no longer wants
                        // the entries at all (the next watcher
                        // pass drops them).
                        cell.unmark_streamed(&peer_realm);
                        return;
                    }
                }
                // Stream ended (any outcome except cancel) —
                // release the claim so the poll task picks up
                // the slack until the next reconnect succeeds.
                // Also flip every entry in the peer's local
                // view to status = "stale" + republish so
                // readers see freshness annotation per spec
                // §2.1 (PR-N3 N3-streaming-12). The view stays
                // populated so a brief disconnect doesn't
                // erase the entries; the next successful
                // Snapshot replaces them wholesale.
                cell.unmark_streamed(&peer_realm);
                client.mark_stale_and_publish(&cell);
            }
            Err(err) => {
                let err_msg = format!("{err}");
                crate::op_event!(
                    component = federation_directory,
                    kind = subscribe_directory_v2_dial_failed,
                    peer_realm = peer_realm,
                    peer_hub_endpoint = peer_hub_endpoint,
                    error = err_msg,
                    message = "backing off",
                );
                let backoff_ms = client.on_dial_err();
                tokio::select! {
                    _ = tokio::time::sleep(std::time::Duration::from_millis(backoff_ms)) => {}
                    _ = &mut cancel => {
                        return;
                    }
                }
                continue;
            }
        }

        // Stream-end backoff (post-Pumping disconnect).
        let backoff_ms = client.on_dial_err();
        crate::op_event!(
            component = federation_directory,
            kind = subscribe_directory_v2_stream_ended_reconnecting,
            peer_realm = peer_realm,
            backoff_ms = backoff_ms,
        );
        tokio::select! {
            _ = tokio::time::sleep(std::time::Duration::from_millis(backoff_ms)) => {}
            _ = &mut cancel => {
                return;
            }
        }
    }
}

/// **PR-N3 N3-streaming-2**. Drive a `RemoteDirectoryClient`
/// from a stream of inbound `DirectoryEvent` frames, publishing
/// the resulting view into the cell after each frame applied.
/// Returns when the stream ends (peer closed, error) or when an
/// FSM protocol violation aborts the consume.
///
/// The caller (the per-peer tokio task) is responsible for
/// reconnecting on return — backoff via `client.on_dial_err()`
/// before re-dialling. This function does NOT loop; it consumes
/// one stream's lifetime, then yields control so the caller can
/// decide reconnect strategy.
///
/// Errors return the `FsmError::ProtocolViolation` reason; the
/// caller maps this to a tear-down + back-off in the per-peer
/// supervisor task. `Ok(())` means the stream ended gracefully.
pub async fn consume_directory_event_stream<S>(
    client: &mut RemoteDirectoryClient,
    cell: &SharedFederatedDirectoryView,
    mut stream: S,
) -> Result<(), FsmError>
where
    S: futures::Stream<Item = DirectoryEvent> + Unpin,
{
    use futures::StreamExt;
    while let Some(event) = stream.next().await {
        client.apply_event(&event)?;
        // After every applied frame, republish the peer's
        // slot so downstream readers see the update. Heartbeat
        // is a no-op for the view but still publishes — same-
        // shape republish is cheap (Arc clones + one map
        // write) and keeps the contract uniform.
        client.publish_to_cell(cell);
    }
    client.on_stream_end();
    Ok(())
}

/// Outcome of `consume_directory_event_stream_with_idle_timeout`.
/// Distinct from `Result<(), FsmError>` because we need three
/// terminal states, not two: graceful end vs. protocol
/// violation vs. idle timeout. The supervisor reconnects with
/// FSM backoff in all three cases; the variant lets it log
/// the reason without grepping a `Display` string.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ConsumeOutcome {
    /// Stream ended cleanly (peer closed gracefully).
    StreamEnded,
    /// FSM rejected an inbound frame (Snapshot mid-stream,
    /// Upsert before Snapshot, etc).
    ProtocolViolation(&'static str),
    /// No frame received within the idle-timeout window
    /// (peer alive but silent). Spec §2.3:
    /// "60s no frame → Disconnected (treat as dead)". The
    /// peer's heartbeat keepalive should arrive every 30s
    /// (PR-N3 N3-streaming-6); two missed ticks ⇒ dead.
    IdleTimeout,
}

/// **PR-N3 N3-streaming-7**. Drive a `RemoteDirectoryClient`
/// from an upstream stream of `DirectoryEvent` frames, with a
/// receiver-side idle-timeout watcher per spec §2.3. Variant of
/// `consume_directory_event_stream` that races each
/// `stream.next()` against a `tokio::time::sleep` of
/// `idle_timeout_ms`; if the sleep wins, the FSM transitions to
/// Disconnected and we return `ConsumeOutcome::IdleTimeout`.
///
/// Cadence: 60s production per spec §2.3 (= two missed
/// 30s heartbeat windows). Tunable for tests via the parameter.
///
/// Frame applied → idle timer resets (next select! call
/// reinitialises sleep).
pub async fn consume_directory_event_stream_with_idle_timeout<S>(
    client: &mut RemoteDirectoryClient,
    cell: &SharedFederatedDirectoryView,
    mut stream: S,
    idle_timeout_ms: u64,
) -> ConsumeOutcome
where
    S: futures::Stream<Item = DirectoryEvent> + Unpin,
{
    use futures::StreamExt;
    let timeout = std::time::Duration::from_millis(idle_timeout_ms);
    loop {
        tokio::select! {
            next = stream.next() => {
                match next {
                    Some(event) => {
                        if let Err(FsmError::ProtocolViolation(reason)) =
                            client.apply_event(&event)
                        {
                            return ConsumeOutcome::ProtocolViolation(reason);
                        }
                        client.publish_to_cell(cell);
                    }
                    None => {
                        client.on_stream_end();
                        return ConsumeOutcome::StreamEnded;
                    }
                }
            }
            _ = tokio::time::sleep(timeout) => {
                client.on_idle_timeout();
                return ConsumeOutcome::IdleTimeout;
            }
        }
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
            "agent_ura": "easynet:///r/realm-a/device/device-A",
            "node_id": "node-1",
            "display_name": "silan-laptop",
            "status": "active"
        }"#
    }

    fn full_entry_json() -> &'static str {
        r#"{
            "agent_ura": "easynet:///r/realm-a/device/device-A",
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
        assert_eq!(entry.agent_ura, "easynet:///r/realm-a/device/device-A");
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
        assert_eq!(parsed["agent_ura"], "easynet:///r/realm-a/device/device-A");
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
            agent_ura: "easynet:///r/realm-a/device/local-1".to_string(),
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
            agent_ura: "easynet:///r/realm-a/device/device-A".to_string(),
            node_id: "node-1".to_string(),
            display_name: Some("silan-laptop".to_string()),
            status: "active".to_string(),
            origin_realm: Some("realm-a".to_string()),
            hub_endpoint: Some("https://hub-a.example:50443".to_string()),
            last_seen_unix_ms: Some(1_714_492_800_000),
        }
    }

    fn agent_summary_from_entry(entry: &DirectoryEntry) -> DirectoryAgentSummary {
        DirectoryAgentSummary {
            agent_ura: entry.agent_ura.clone(),
            signing_authority: SigningAuthority::SelfSigned,
            status: entry.status.clone(),
            ability_count: 0,
        }
    }

    fn snapshot_event(entries: Vec<DirectoryEntry>) -> DirectoryEvent {
        DirectoryEvent::Snapshot {
            agents: entries.iter().map(agent_summary_from_entry).collect(),
            snapshot_unix_ms: 1_714_492_800_000,
        }
    }

    fn empty_snapshot_event() -> DirectoryEvent {
        snapshot_event(Vec::new())
    }

    fn advertised_event(entry: DirectoryEntry) -> DirectoryEvent {
        DirectoryEvent::AgentAdvertised {
            agent_ura: entry.agent_ura,
            signing_authority: SigningAuthority::SelfSigned,
            replaced_prior: false,
            unix_ms: 1_714_492_800_000,
        }
    }

    fn revoked_event(agent_ura: &str, reason: &str) -> DirectoryEvent {
        DirectoryEvent::AgentRevoked {
            agent_ura: agent_ura.to_string(),
            was_active: true,
            reason: reason.to_string(),
            unix_ms: 1_714_492_800_000,
        }
    }

    fn heartbeat_event(unix_ms: i64) -> DirectoryEvent {
        DirectoryEvent::Heartbeat { unix_ms }
    }

    #[test]
    fn directory_event_snapshot_serialises_with_type_tag() {
        let evt = snapshot_event(vec![sample_entry()]);
        let bytes = serde_json::to_vec(&evt).expect("serialise snapshot");
        let parsed: serde_json::Value = serde_json::from_slice(&bytes).expect("re-parse");
        assert_eq!(parsed["type"], "snapshot");
        assert_eq!(parsed["agents"][0]["agent_ura"], sample_entry().agent_ura);
        assert_eq!(
            parsed["snapshot_unix_ms"],
            serde_json::json!(1_714_492_800_000_i64)
        );
    }

    #[test]
    fn directory_event_delta_heartbeat_serialise_with_type_tag() {
        let advertised_bytes = serde_json::to_vec(&advertised_event(sample_entry())).unwrap();
        let advertised: serde_json::Value = serde_json::from_slice(&advertised_bytes).unwrap();
        assert_eq!(advertised["type"], "agent_advertised");
        assert_eq!(
            advertised["agent_ura"],
            "easynet:///r/realm-a/device/device-A"
        );

        let abilities_bytes = serde_json::to_vec(&DirectoryEvent::AbilitiesAdvertised {
            agent_ura: "easynet:///r/realm-a/device/device-A".to_string(),
            count: 3,
            unix_ms: 1_714_492_800_000,
        })
        .unwrap();
        let abilities: serde_json::Value = serde_json::from_slice(&abilities_bytes).unwrap();
        assert_eq!(abilities["type"], "abilities_advertised");
        assert_eq!(abilities["count"], 3);

        let revoked_bytes = serde_json::to_vec(&revoked_event(
            "easynet:///r/realm-a/device/dropped",
            "shutdown",
        ))
        .unwrap();
        let revoked: serde_json::Value = serde_json::from_slice(&revoked_bytes).unwrap();
        assert_eq!(revoked["type"], "agent_revoked");
        assert_eq!(revoked["reason"], "shutdown");

        let hb_bytes = serde_json::to_vec(&heartbeat_event(1_714_492_800_000)).unwrap();
        let hb: serde_json::Value = serde_json::from_slice(&hb_bytes).unwrap();
        assert_eq!(hb["type"], "heartbeat");
        assert_eq!(hb["unix_ms"], 1_714_492_800_000_i64);
    }

    #[test]
    fn directory_event_round_trips_through_serde() {
        let original = advertised_event(sample_entry());
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

        fsm.on_frame(&empty_snapshot_event()).expect("snapshot ok");
        assert!(matches!(fsm.state(), &SubscriberState::Pumping));
    }

    #[test]
    fn fsm_second_snapshot_mid_stream_is_protocol_violation() {
        // Spec §2.3: a second Snapshot frame after the first
        // promotes-to-Pumping is a protocol violation; receiver
        // MUST drop the connection.
        let mut fsm = SubscriberFsm::new();
        fsm.on_dial_ok();
        fsm.on_frame(&empty_snapshot_event())
            .expect("first snapshot ok");
        let err = fsm
            .on_frame(&empty_snapshot_event())
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
            .on_frame(&advertised_event(sample_entry()))
            .expect_err("upsert before snapshot must reject");
        assert!(matches!(err, FsmError::ProtocolViolation(_)));
        assert!(matches!(fsm.state(), &SubscriberState::Disconnected));
    }

    #[test]
    fn fsm_pumping_accepts_upsert_remove_heartbeat() {
        let mut fsm = SubscriberFsm::new();
        fsm.on_dial_ok();
        fsm.on_frame(&empty_snapshot_event()).expect("snapshot");
        for evt in [
            advertised_event(sample_entry()),
            DirectoryEvent::AbilitiesAdvertised {
                agent_ura: "easynet:///r/realm-a/device/device-A".to_string(),
                count: 2,
                unix_ms: 1_714_492_800_000,
            },
            revoked_event("easynet:///r/realm-a/device/x", "drop"),
            heartbeat_event(1_714_492_800_000),
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
        fsm.on_frame(&empty_snapshot_event()).unwrap();
        fsm.on_frame(&advertised_event(sample_entry())).unwrap();
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
        fsm.on_frame(&empty_snapshot_event()).unwrap();
        fsm.on_frame(&heartbeat_event(1_714_492_800_000)).unwrap();
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
        fsm.on_frame(&empty_snapshot_event()).unwrap();
        fsm.on_frame(&advertised_event(sample_entry())).unwrap();

        fsm.on_idle_timeout();
        assert!(matches!(fsm.state(), &SubscriberState::Disconnected));
    }

    #[test]
    fn round_trip_through_serde_preserves_field_equality() {
        // PartialEq derive lets us assert byte-stable round-
        // trips for testing receivers that compare entries to
        // detect changes between subscribe-stream snapshots.
        let original = DirectoryEntry {
            agent_ura: "easynet:///r/realm-b/device/peer-device".to_string(),
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

    fn entry_with_claimed_origin(ura: &str, claimed: Option<&str>) -> DirectoryEntry {
        DirectoryEntry {
            agent_ura: ura.to_string(),
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
        view.replace_entries(vec![entry_with_claimed_origin(
            "easynet:///r/realm-b/device/peer-device",
            Some("trusted-bank"),
        )]);
        let stamped = view
            .lookup("easynet:///r/realm-b/device/peer-device")
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
        view.apply_frame(&advertised_event(entry_with_claimed_origin(
            "easynet:///r/realm-b/device/peer-device",
            Some("realm-c"),
        )));
        let stamped = view
            .lookup("easynet:///r/realm-b/device/peer-device")
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
        view.apply_frame(&advertised_event(entry_with_claimed_origin(
            "easynet:///r/realm-b/device/peer-device",
            None,
        )));
        let stamped = view
            .lookup("easynet:///r/realm-b/device/peer-device")
            .expect("entry stored");
        assert_eq!(stamped.origin_realm.as_deref(), Some("realm-b"));
    }

    #[test]
    fn apply_remove_drops_entry_from_view() {
        let mut view = DirectoryView::new("realm-b".to_string());
        view.apply_frame(&snapshot_event(vec![entry_with_claimed_origin(
            "easynet:///r/realm-b/device/peer-device",
            None,
        )]));
        assert!(view
            .lookup("easynet:///r/realm-b/device/peer-device")
            .is_some());
        view.apply_frame(&revoked_event(
            "easynet:///r/realm-b/device/peer-device",
            "shutdown",
        ));
        assert!(view
            .lookup("easynet:///r/realm-b/device/peer-device")
            .is_none());
    }

    #[test]
    fn apply_snapshot_replaces_view_wholesale() {
        // Spec §2.2: receiver replaces its peer-keyed view
        // wholesale on Snapshot. Old entries that aren't in the
        // new snapshot disappear.
        let mut view = DirectoryView::new("realm-b".to_string());
        view.apply_frame(&advertised_event(entry_with_claimed_origin(
            "easynet:///r/realm-b/device/old",
            None,
        )));
        view.apply_frame(&snapshot_event(vec![entry_with_claimed_origin(
            "easynet:///r/realm-b/device/new",
            None,
        )]));
        assert!(view.lookup("easynet:///r/realm-b/device/old").is_none());
        assert!(view.lookup("easynet:///r/realm-b/device/new").is_some());
    }

    #[test]
    fn apply_heartbeat_is_noop_for_view() {
        let mut view = DirectoryView::new("realm-b".to_string());
        view.apply_frame(&advertised_event(entry_with_claimed_origin(
            "easynet:///r/realm-b/device/peer",
            None,
        )));
        let before = view.entries.clone();
        view.apply_frame(&heartbeat_event(1_714_500_000_000));
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
        let client = RemoteDirectoryClient::new("realm-b".to_string());
        assert_eq!(client.peer_realm(), "realm-b");
        assert!(matches!(client.fsm_state(), &SubscriberState::Disconnected));
        assert!(client.view_snapshot().entries.is_empty());
    }

    #[test]
    fn remote_directory_client_apply_event_drives_fsm_and_view_together() {
        let mut client = RemoteDirectoryClient::new("realm-b".to_string());
        client.on_dial_ok();
        client
            .apply_event(&snapshot_event(vec![entry_with_claimed_origin(
                "easynet:///r/realm-b/device/peer",
                Some("trusted-bank"), // spoofed; rewrite chokepoint catches
            )]))
            .expect("snapshot accepted");
        assert!(matches!(client.fsm_state(), &SubscriberState::Pumping));
        let stamped = client
            .view_snapshot()
            .lookup("easynet:///r/realm-b/device/peer")
            .expect("entry stored");
        assert_eq!(
            stamped.origin_realm.as_deref(),
            Some("realm-b"),
            "RemoteDirectoryClient must enforce §2.4 origin_realm rewrite"
        );
    }

    #[test]
    fn remote_directory_client_apply_event_protocol_violation_does_not_mutate_view() {
        let mut client = RemoteDirectoryClient::new("realm-b".to_string());
        client.on_dial_ok();
        // Upsert before Snapshot → ProtocolViolation. The FSM
        // drops to Disconnected; the view MUST stay empty so the
        // peer cannot inject entries by sending Upserts before
        // the mandatory Snapshot.
        let err = client
            .apply_event(&advertised_event(entry_with_claimed_origin(
                "easynet:///r/realm-b/device/sneaky",
                None,
            )))
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
        let mut client = RemoteDirectoryClient::new("realm-b".to_string());
        assert_eq!(client.on_dial_err(), 2_000);
        assert_eq!(client.on_dial_err(), 4_000);
        assert_eq!(client.on_dial_err(), 8_000);
    }

    // ── PR-N3 N3-streaming-4 — run_per_peer_supervisor ──

    #[cfg(feature = "axon-pb")]
    mod supervisor_tests {
        use super::*;
        use crate::services::federation_client::{
            DirectoryEventStream, FederationClient, FederationClientError, HubUri,
        };
        use async_trait::async_trait;
        use easynet_axon::pb::axon::v1::{
            InvokeRequest, InvokeResponse, InvokeServerStreamRequest,
        };
        use std::sync::{Arc, Mutex};

        /// Mock that delivers a canned event sequence on the
        /// first subscribe call, then signals via `served`
        /// when called. Subsequent calls fail to dial — the
        /// supervisor will back off + retry until cancel.
        struct OneShotStreamingClient {
            events: Mutex<Option<Vec<DirectoryEvent>>>,
            served: Mutex<bool>,
        }

        #[async_trait]
        impl FederationClient for OneShotStreamingClient {
            async fn forward_invoke(
                &self,
                _target_hub: &HubUri,
                _request: InvokeRequest,
            ) -> Result<InvokeResponse, FederationClientError> {
                Err(FederationClientError::Unimplemented("not used in test"))
            }

            async fn subscribe_directory_v2(
                &self,
                _target_hub: &HubUri,
                _request: InvokeServerStreamRequest,
            ) -> Result<DirectoryEventStream, FederationClientError> {
                let payload = self.events.lock().unwrap().take();
                match payload {
                    Some(events) => {
                        *self.served.lock().unwrap() = true;
                        Ok(Box::pin(futures::stream::iter(events)))
                    }
                    None => Err(FederationClientError::DialFailed {
                        hub: "in-process".to_string(),
                        detail: "test fixture: stream already served once".to_string(),
                    }),
                }
            }
        }

        /// Mock that returns a `pending` stream every time
        /// `subscribe_directory_v2` is called. Each call
        /// records into a counter so the test can assert the
        /// supervisor reconnected.
        struct StalledStreamingClient {
            dial_count: Arc<Mutex<u32>>,
        }

        #[async_trait]
        impl FederationClient for StalledStreamingClient {
            async fn forward_invoke(
                &self,
                _target_hub: &HubUri,
                _request: InvokeRequest,
            ) -> Result<InvokeResponse, FederationClientError> {
                Err(FederationClientError::Unimplemented("not used in test"))
            }

            async fn subscribe_directory_v2(
                &self,
                _target_hub: &HubUri,
                _request: InvokeServerStreamRequest,
            ) -> Result<DirectoryEventStream, FederationClientError> {
                *self.dial_count.lock().unwrap() += 1;
                Ok(Box::pin(futures::stream::pending::<DirectoryEvent>()))
            }
        }

        struct CaptureSubscribeRequestClient {
            function_names: Arc<Mutex<Vec<String>>>,
        }

        #[async_trait]
        impl FederationClient for CaptureSubscribeRequestClient {
            async fn forward_invoke(
                &self,
                _target_hub: &HubUri,
                _request: InvokeRequest,
            ) -> Result<InvokeResponse, FederationClientError> {
                Err(FederationClientError::Unimplemented("not used in test"))
            }

            async fn subscribe_directory_v2(
                &self,
                _target_hub: &HubUri,
                request: InvokeServerStreamRequest,
            ) -> Result<DirectoryEventStream, FederationClientError> {
                self.function_names
                    .lock()
                    .unwrap()
                    .push(request.function_name);
                Ok(Box::pin(futures::stream::pending::<DirectoryEvent>()))
            }
        }

        #[tokio::test(flavor = "multi_thread", worker_threads = 2)]
        async fn supervisor_idle_timeout_drives_reconnect() {
            // PR-N3 N3-streaming-8. Peer accepts the dial but
            // never yields a frame. The receiver-side idle
            // timeout (50ms in this test) fires; the supervisor
            // logs IdleTimeout, sleeps the FSM backoff, dials
            // again. Within the test's 1s budget we should
            // observe at least 2 dials — proves the reconnect
            // path through to the next subscribe call.
            let cell = SharedFederatedDirectoryView::default();
            let dial_count = Arc::new(Mutex::new(0u32));
            let client = Arc::new(StalledStreamingClient {
                dial_count: dial_count.clone(),
            });
            let (cancel_tx, cancel_rx) = tokio::sync::oneshot::channel();

            let cell_for_task = cell.clone();
            let client_for_task: Arc<dyn FederationClient> = client.clone();
            let task = tokio::spawn(async move {
                run_per_peer_supervisor_with_idle_timeout(
                    "realm-b".to_string(),
                    "https://hub-b.example:50443".to_string(),
                    "easynet:///r/realm-a/hub".to_string(),
                    client_for_task,
                    cell_for_task,
                    cancel_rx,
                    50, // 50ms idle timeout
                )
                .await;
            });

            // Wait long enough for at least two dial cycles:
            // 50ms idle + ≥1s FSM backoff (first redial) +
            // 50ms idle + 2s backoff... realistically we need
            // ~2.5s to see the second dial. Cap at 3s so a
            // regression surfaces as a test timeout.
            for _ in 0..120 {
                if *dial_count.lock().unwrap() >= 2 {
                    break;
                }
                tokio::time::sleep(std::time::Duration::from_millis(25)).await;
            }
            let final_count = *dial_count.lock().unwrap();
            assert!(
                final_count >= 2,
                "expected ≥ 2 dials within 3s window (idle + backoff cycle); got {final_count}"
            );

            // Cancel the supervisor and confirm shutdown.
            let _ = cancel_tx.send(());
            let result = tokio::time::timeout(std::time::Duration::from_secs(5), task).await;
            assert!(result.is_ok(), "supervisor must honour cancel within 5s");
        }

        #[tokio::test(flavor = "multi_thread", worker_threads = 2)]
        async fn supervisor_dials_v2_directory_ability() {
            let cell = SharedFederatedDirectoryView::default();
            let function_names = Arc::new(Mutex::new(Vec::<String>::new()));
            let client = Arc::new(CaptureSubscribeRequestClient {
                function_names: function_names.clone(),
            });
            let (cancel_tx, cancel_rx) = tokio::sync::oneshot::channel();

            let cell_for_task = cell.clone();
            let client_for_task: Arc<dyn FederationClient> = client.clone();
            let task = tokio::spawn(async move {
                run_per_peer_supervisor_with_idle_timeout(
                    "realm-b".to_string(),
                    "https://hub-b.example:50443".to_string(),
                    "easynet:///r/realm-a/hub".to_string(),
                    client_for_task,
                    cell_for_task,
                    cancel_rx,
                    50,
                )
                .await;
            });

            for _ in 0..40 {
                if !function_names.lock().unwrap().is_empty() {
                    break;
                }
                tokio::time::sleep(std::time::Duration::from_millis(25)).await;
            }
            let _ = cancel_tx.send(());
            let result = tokio::time::timeout(std::time::Duration::from_secs(5), task).await;
            assert!(result.is_ok(), "supervisor must honour cancel within 5s");

            let names = function_names.lock().unwrap();
            assert!(
                names.iter().any(|name| {
                    name
                        == crate::services::invocation_transport::federation_wrappers
                            ::ABILITY_FEDERATION_SUBSCRIBE_DIRECTORY_V2
                }),
                "supervisor must dial the v2 stream ability; got {names:?}",
            );
            assert!(
                names.iter().all(|name| {
                    name
                        != crate::services::invocation_transport::federation_wrappers
                            ::ABILITY_FEDERATION_SUBSCRIBE_DIRECTORY
                }),
                "supervisor must not dial the legacy v1 stream ability; got {names:?}",
            );
        }

        // ── PR-N3 N3-streaming-9 — reconcile_streaming_supervisors ──

        #[test]
        fn reconcile_spawns_for_new_peers() {
            // Empty active map + 2-peer snapshot ⇒ 2 spawns,
            // 0 cancels. Spawn closure records the calls.
            let mut active: std::collections::BTreeMap<String, tokio::sync::oneshot::Sender<()>> =
                std::collections::BTreeMap::new();
            let mut snapshot = std::collections::BTreeMap::new();
            snapshot.insert(
                "realm-b".to_string(),
                "https://hub-b.example:50443".to_string(),
            );
            snapshot.insert(
                "realm-c".to_string(),
                "https://hub-c.example:50443".to_string(),
            );

            let mut spawn_calls: Vec<(String, String)> = Vec::new();
            let (spawned, cancelled) = reconcile_streaming_supervisors(
                &snapshot,
                &mut active,
                |peer_realm, peer_hub_endpoint| {
                    spawn_calls.push((peer_realm.to_string(), peer_hub_endpoint.to_string()));
                    let (tx, _rx) = tokio::sync::oneshot::channel();
                    tx
                },
            );
            assert_eq!(spawned.len(), 2);
            assert!(spawned.contains(&"realm-b".to_string()));
            assert!(spawned.contains(&"realm-c".to_string()));
            assert!(cancelled.is_empty());
            assert_eq!(spawn_calls.len(), 2);
            assert!(active.contains_key("realm-b"));
            assert!(active.contains_key("realm-c"));
        }

        #[test]
        fn reconcile_skips_peers_already_active() {
            // Pre-populate active with realm-b. Snapshot still
            // has realm-b + new realm-c. Only realm-c spawns.
            let mut active: std::collections::BTreeMap<String, tokio::sync::oneshot::Sender<()>> =
                std::collections::BTreeMap::new();
            let (existing_tx, _existing_rx) = tokio::sync::oneshot::channel();
            active.insert("realm-b".to_string(), existing_tx);

            let mut snapshot = std::collections::BTreeMap::new();
            snapshot.insert(
                "realm-b".to_string(),
                "https://hub-b.example:50443".to_string(),
            );
            snapshot.insert(
                "realm-c".to_string(),
                "https://hub-c.example:50443".to_string(),
            );

            let mut spawn_calls = 0u32;
            let (spawned, cancelled) =
                reconcile_streaming_supervisors(&snapshot, &mut active, |_, _| {
                    spawn_calls += 1;
                    let (tx, _rx) = tokio::sync::oneshot::channel();
                    tx
                });
            assert_eq!(spawned, vec!["realm-c".to_string()]);
            assert!(cancelled.is_empty());
            assert_eq!(spawn_calls, 1, "must NOT respawn an already-active peer");
        }

        #[test]
        fn reconcile_cancels_peers_no_longer_in_snapshot() {
            // Pre-populate active with realm-b. Empty snapshot
            // (peer was removed via SIGHUP). reconcile fires
            // cancel + drops the entry from active. The cancel
            // receiver should observe the signal — proves a
            // real `oneshot::send` not a no-op.
            let (cancel_tx, mut cancel_rx) = tokio::sync::oneshot::channel();
            let mut active = std::collections::BTreeMap::new();
            active.insert("realm-b".to_string(), cancel_tx);

            let snapshot = std::collections::BTreeMap::new();
            let (spawned, cancelled) =
                reconcile_streaming_supervisors(&snapshot, &mut active, |_, _| {
                    panic!("must NOT spawn for empty snapshot");
                });
            assert!(spawned.is_empty());
            assert_eq!(cancelled, vec!["realm-b".to_string()]);
            assert!(!active.contains_key("realm-b"));
            // The supervisor side received the cancel signal.
            assert!(cancel_rx.try_recv().is_ok());
        }

        #[test]
        fn reconcile_handles_simultaneous_add_and_drop() {
            // Realm-b active; snapshot replaces it with realm-c.
            // reconcile spawns realm-c + cancels realm-b in
            // one pass.
            let (existing_tx, mut existing_rx) = tokio::sync::oneshot::channel();
            let mut active = std::collections::BTreeMap::new();
            active.insert("realm-b".to_string(), existing_tx);

            let mut snapshot = std::collections::BTreeMap::new();
            snapshot.insert(
                "realm-c".to_string(),
                "https://hub-c.example:50443".to_string(),
            );

            let (spawned, cancelled) =
                reconcile_streaming_supervisors(&snapshot, &mut active, |_, _| {
                    let (tx, _rx) = tokio::sync::oneshot::channel();
                    tx
                });
            assert_eq!(spawned, vec!["realm-c".to_string()]);
            assert_eq!(cancelled, vec!["realm-b".to_string()]);
            assert!(active.contains_key("realm-c"));
            assert!(!active.contains_key("realm-b"));
            assert!(existing_rx.try_recv().is_ok());
        }

        #[test]
        fn reconcile_no_op_when_active_matches_snapshot() {
            // Active and snapshot identical ⇒ no spawn, no
            // cancel.
            let (existing_tx, _existing_rx) = tokio::sync::oneshot::channel();
            let mut active = std::collections::BTreeMap::new();
            active.insert("realm-b".to_string(), existing_tx);

            let mut snapshot = std::collections::BTreeMap::new();
            snapshot.insert(
                "realm-b".to_string(),
                "https://hub-b.example:50443".to_string(),
            );

            let (spawned, cancelled) =
                reconcile_streaming_supervisors(&snapshot, &mut active, |_, _| {
                    panic!("must NOT spawn");
                });
            assert!(spawned.is_empty());
            assert!(cancelled.is_empty());
            assert!(active.contains_key("realm-b"));
        }

        #[tokio::test(flavor = "multi_thread", worker_threads = 2)]
        async fn supervisor_consumes_one_stream_then_yields_to_cancel() {
            let cell = SharedFederatedDirectoryView::default();
            let client = Arc::new(OneShotStreamingClient {
                events: Mutex::new(Some(vec![snapshot_event(vec![entry_with_claimed_origin(
                    "easynet:///r/realm-b/device/peer",
                    Some("trusted-bank"), // chokepoint stamps realm-b
                )])])),
                served: Mutex::new(false),
            });
            let (cancel_tx, cancel_rx) = tokio::sync::oneshot::channel();

            let cell_for_task = cell.clone();
            let client_for_task: Arc<dyn FederationClient> = client.clone();
            let task = tokio::spawn(async move {
                run_per_peer_supervisor(
                    "realm-b".to_string(),
                    "https://hub-b.example:50443".to_string(),
                    "easynet:///r/realm-a/hub".to_string(),
                    client_for_task,
                    cell_for_task,
                    cancel_rx,
                )
                .await;
            });

            // Wait briefly for the supervisor to consume the
            // canned stream.
            for _ in 0..40 {
                if *client.served.lock().unwrap() {
                    if let Some(view) = cell.snapshot().get("realm-b") {
                        if view.lookup("easynet:///r/realm-b/device/peer").is_some() {
                            break;
                        }
                    }
                }
                tokio::time::sleep(std::time::Duration::from_millis(25)).await;
            }
            assert!(
                *client.served.lock().unwrap(),
                "supervisor must have consumed the canned stream"
            );

            // Cell reflects the stamped entry.
            let snap = cell.snapshot();
            let entry = snap
                .get("realm-b")
                .expect("realm-b view")
                .lookup("easynet:///r/realm-b/device/peer")
                .expect("entry");
            assert_eq!(
                entry.origin_realm.as_deref(),
                Some("realm-b"),
                "§2.4 chokepoint must run through the supervisor's apply path"
            );

            // Cancel and assert task ends within a bounded
            // window. The supervisor should be inside its
            // backoff-sleep when cancel fires.
            let _ = cancel_tx.send(());
            let result = tokio::time::timeout(std::time::Duration::from_secs(5), task).await;
            assert!(
                result.is_ok(),
                "supervisor must honour cancel within timeout"
            );
        }

        // ── PR-N3 N3-streaming-11 — streamed-marker lifecycle ──

        #[tokio::test(flavor = "multi_thread", worker_threads = 2)]
        async fn supervisor_marks_streamed_while_stream_open_unmarks_on_close() {
            // Stream delivers a Snapshot then ends. Within the
            // brief window between dial-ok and stream-end, the
            // cell.is_streamed("realm-b") MUST be true. After
            // the stream ends and the supervisor enters its
            // reconnect-backoff sleep, the marker MUST be
            // false (poll task can pick up the slack).
            //
            // Verifying mid-stream and post-close is racy with
            // pure futures::iter (the stream completes
            // synchronously). Use a delayed stream: yield the
            // Snapshot, then await a small sleep before the
            // None terminator so the test can poll
            // is_streamed during the open window.
            use futures::StreamExt;

            struct DelayedStreamingClient {
                served: Arc<Mutex<bool>>,
            }

            #[async_trait]
            impl FederationClient for DelayedStreamingClient {
                async fn forward_invoke(
                    &self,
                    _target_hub: &HubUri,
                    _request: InvokeRequest,
                ) -> Result<InvokeResponse, FederationClientError> {
                    Err(FederationClientError::Unimplemented("not used"))
                }

                async fn subscribe_directory_v2(
                    &self,
                    _target_hub: &HubUri,
                    _request: InvokeServerStreamRequest,
                ) -> Result<DirectoryEventStream, FederationClientError> {
                    if *self.served.lock().unwrap() {
                        // Subsequent dials hang briefly so the
                        // test can observe the unmark window
                        // between stream-end and re-dial.
                        return Ok(Box::pin(futures::stream::pending::<DirectoryEvent>()));
                    }
                    *self.served.lock().unwrap() = true;
                    let snapshot = futures::stream::once(async { empty_snapshot_event() });
                    // Hold the stream open ~200ms before EOF so
                    // the test has a window to poll is_streamed.
                    let hold = futures::stream::once(async {
                        tokio::time::sleep(std::time::Duration::from_millis(200)).await;
                        heartbeat_event(1_000)
                    });
                    Ok(Box::pin(snapshot.chain(hold)))
                }
            }

            let cell = SharedFederatedDirectoryView::default();
            let served = Arc::new(Mutex::new(false));
            let client: Arc<dyn FederationClient> = Arc::new(DelayedStreamingClient {
                served: served.clone(),
            });
            let (cancel_tx, cancel_rx) = tokio::sync::oneshot::channel();

            let cell_for_task = cell.clone();
            let task = tokio::spawn(async move {
                run_per_peer_supervisor(
                    "realm-b".to_string(),
                    "https://hub-b.example:50443".to_string(),
                    "easynet:///r/realm-a/hub".to_string(),
                    client,
                    cell_for_task,
                    cancel_rx,
                )
                .await;
            });

            // Wait for the first dial to complete; mid-stream
            // is_streamed must be true. The Snapshot frame +
            // 200ms hold gives a generous observation window.
            let mut saw_streamed = false;
            for _ in 0..40 {
                if *served.lock().unwrap() && cell.is_streamed("realm-b") {
                    saw_streamed = true;
                    break;
                }
                tokio::time::sleep(std::time::Duration::from_millis(15)).await;
            }
            assert!(
                saw_streamed,
                "supervisor must mark realm-b streamed during the open window"
            );

            // Cancel before the supervisor can redial. The
            // marker should be cleared on cancel-path exit so
            // a re-add cycle isn't blocked by a stale claim.
            let _ = cancel_tx.send(());
            let _ = tokio::time::timeout(std::time::Duration::from_secs(5), task).await;
            assert!(
                !cell.is_streamed("realm-b"),
                "supervisor must unmark realm-b on cancel-path exit"
            );
        }
    }

    // ── PR-N3 N3-streaming-2 — consume_directory_event_stream ──

    // ── PR-N3 N3-streaming-12 — mark-stale-and-publish ──

    #[test]
    fn mark_all_stale_flips_every_entry_status() {
        // Pure DirectoryView API. View has 3 active entries;
        // mark_all_stale flips each.
        let mut view = DirectoryView::new("realm-b".to_string());
        view.replace_entries(vec![
            entry_with_claimed_origin("easynet:///r/realm-b/device/a", None),
            entry_with_claimed_origin("easynet:///r/realm-b/device/b", None),
            entry_with_claimed_origin("easynet:///r/realm-b/device/c", None),
        ]);
        for entry in view.entries.values() {
            assert_eq!(entry.status, "active", "fixture is active");
        }
        view.mark_all_stale();
        for entry in view.entries.values() {
            assert_eq!(entry.status, "stale", "every entry must flip to stale");
        }
    }

    #[test]
    fn mark_stale_and_publish_writes_stale_view_to_cell() {
        // Client with a populated view; mark_stale_and_publish
        // flips locally + publishes; cell snapshot reflects
        // the stale annotation.
        let cell = SharedFederatedDirectoryView::default();
        let mut client = RemoteDirectoryClient::new("realm-b".to_string());
        client.on_dial_ok();
        client
            .apply_event(&snapshot_event(vec![entry_with_claimed_origin(
                "easynet:///r/realm-b/device/peer",
                None,
            )]))
            .unwrap();
        client.publish_to_cell(&cell);
        // Sanity: cell shows the entry as active.
        let snap1 = cell.snapshot();
        assert_eq!(
            snap1
                .get("realm-b")
                .and_then(|v| v.lookup("easynet:///r/realm-b/device/peer"))
                .map(|e| e.status.as_str()),
            Some("active"),
        );

        // Disconnect → mark_stale_and_publish → cell shows
        // status="stale".
        client.mark_stale_and_publish(&cell);
        let snap2 = cell.snapshot();
        assert_eq!(
            snap2
                .get("realm-b")
                .and_then(|v| v.lookup("easynet:///r/realm-b/device/peer"))
                .map(|e| e.status.as_str()),
            Some("stale"),
            "post-disconnect publish must flip status to stale",
        );
    }

    #[test]
    fn mark_stale_and_publish_idempotent_on_already_stale_view() {
        // Calling twice in a row produces the same wire bytes.
        // Idempotency guard against a supervisor stuck in
        // reconnect cycles churning the cell.
        let cell = SharedFederatedDirectoryView::default();
        let mut client = RemoteDirectoryClient::new("realm-b".to_string());
        client.on_dial_ok();
        client
            .apply_event(&snapshot_event(vec![entry_with_claimed_origin(
                "easynet:///r/realm-b/device/peer",
                None,
            )]))
            .unwrap();
        client.mark_stale_and_publish(&cell);
        let snap_a = cell.snapshot();
        client.mark_stale_and_publish(&cell);
        let snap_b = cell.snapshot();
        // Both snapshots show the same status.
        let status_a = snap_a
            .get("realm-b")
            .unwrap()
            .lookup("easynet:///r/realm-b/device/peer")
            .unwrap()
            .status
            .clone();
        let status_b = snap_b
            .get("realm-b")
            .unwrap()
            .lookup("easynet:///r/realm-b/device/peer")
            .unwrap()
            .status
            .clone();
        assert_eq!(status_a, status_b);
        assert_eq!(status_a, "stale");
    }

    #[test]
    fn publish_to_cell_replaces_only_this_peers_slot() {
        let cell = SharedFederatedDirectoryView::default();
        // Pre-populate realm-c (a different peer).
        let mut realm_c_view = DirectoryView::new("realm-c".to_string());
        realm_c_view.apply_frame(&advertised_event(entry_with_claimed_origin(
            "easynet:///r/realm-c/device/keep",
            None,
        )));
        let mut prior = BTreeMap::new();
        prior.insert("realm-c".to_string(), Arc::new(realm_c_view));
        cell.replace(prior);

        // realm-b client publishes its (still empty) view; the
        // realm-c slot must remain intact.
        let client = RemoteDirectoryClient::new("realm-b".to_string());
        client.publish_to_cell(&cell);

        let snap = cell.snapshot();
        assert!(
            snap.get("realm-c")
                .and_then(|v| v.lookup("easynet:///r/realm-c/device/keep"))
                .is_some(),
            "publishing realm-b's view must not clobber realm-c"
        );
        assert!(snap.contains_key("realm-b"));
    }

    #[tokio::test(flavor = "multi_thread", worker_threads = 2)]
    async fn consume_directory_event_stream_drives_fsm_and_publishes_view() {
        // Simulate the wire-side stream as a sequence of three
        // events: Snapshot, AgentAdvertised, AgentRevoked. Consumer drives the
        // FSM, the view, AND publishes to the cell after each.
        let mut client = RemoteDirectoryClient::new("realm-b".to_string());
        client.on_dial_ok();
        let cell = SharedFederatedDirectoryView::default();

        let events = vec![
            snapshot_event(vec![entry_with_claimed_origin(
                "easynet:///r/realm-b/device/initial",
                Some("trusted-bank"), // chokepoint stamps realm-b
            )]),
            advertised_event(entry_with_claimed_origin(
                "easynet:///r/realm-b/device/added",
                None,
            )),
            revoked_event("easynet:///r/realm-b/device/initial", "stream_closed"),
        ];
        let stream = futures::stream::iter(events);

        consume_directory_event_stream(&mut client, &cell, stream)
            .await
            .expect("stream consumed gracefully");

        // After the stream ended naturally, the FSM should be
        // Disconnected (on_stream_end fired).
        assert!(matches!(client.fsm_state(), &SubscriberState::Disconnected));

        // Cell reflects the final state: `added` is present,
        // `initial` was removed, origin_realm stamped to realm-b
        // (the §2.4 chokepoint, even on the removed-then-snap
        // path).
        let snap = cell.snapshot();
        let view = snap.get("realm-b").expect("realm-b view present");
        assert!(view.lookup("easynet:///r/realm-b/device/initial").is_none());
        let added = view
            .lookup("easynet:///r/realm-b/device/added")
            .expect("added still present");
        assert_eq!(added.origin_realm.as_deref(), Some("realm-b"));
    }

    #[tokio::test(flavor = "multi_thread", worker_threads = 2)]
    async fn consume_with_idle_timeout_returns_idle_when_stream_silent() {
        // PR-N3 N3-streaming-7. Stream produces nothing within
        // the timeout window → consumer returns IdleTimeout +
        // FSM transitions to Disconnected so the supervisor's
        // outer loop reconnects with backoff.
        let mut client = RemoteDirectoryClient::new("realm-b".to_string());
        client.on_dial_ok();
        let cell = SharedFederatedDirectoryView::default();

        // `futures::stream::pending()` never yields — perfect
        // model of a silent peer.
        let stream = futures::stream::pending::<DirectoryEvent>();
        let outcome = consume_directory_event_stream_with_idle_timeout(
            &mut client,
            &cell,
            stream,
            50, // 50ms — keep test fast.
        )
        .await;

        assert_eq!(outcome, ConsumeOutcome::IdleTimeout);
        assert!(matches!(client.fsm_state(), &SubscriberState::Disconnected));
    }

    #[tokio::test(flavor = "multi_thread", worker_threads = 2)]
    async fn consume_with_idle_timeout_returns_stream_ended_on_natural_close() {
        // Stream yields a Snapshot then ends — natural close
        // → StreamEnded outcome (not IdleTimeout).
        let mut client = RemoteDirectoryClient::new("realm-b".to_string());
        client.on_dial_ok();
        let cell = SharedFederatedDirectoryView::default();

        let stream = futures::stream::iter(vec![snapshot_event(vec![entry_with_claimed_origin(
            "easynet:///r/realm-b/device/x",
            None,
        )])]);
        let outcome = consume_directory_event_stream_with_idle_timeout(
            &mut client,
            &cell,
            stream,
            5_000, // 5s — way bigger than the test runtime.
        )
        .await;

        assert_eq!(outcome, ConsumeOutcome::StreamEnded);
        // Cell got the entry stamped + published.
        let snap = cell.snapshot();
        assert!(snap
            .get("realm-b")
            .and_then(|v| v.lookup("easynet:///r/realm-b/device/x"))
            .is_some());
    }

    #[tokio::test(flavor = "multi_thread", worker_threads = 2)]
    async fn consume_with_idle_timeout_resets_on_each_received_frame() {
        // Frames arrive every 30ms; idle timeout is 50ms. The
        // reset-on-receive contract means the timeout never
        // fires before the stream ends naturally. Without the
        // per-loop sleep recreate, each iteration would
        // accumulate elapsed-since-stream-start and trip
        // around the 2nd or 3rd frame.
        use futures::StreamExt;
        let mut client = RemoteDirectoryClient::new("realm-b".to_string());
        client.on_dial_ok();
        let cell = SharedFederatedDirectoryView::default();

        // 5 frames at 30ms cadence = 150ms total runtime; idle
        // timeout 50ms only trips if the reset is broken.
        // First frame must be Snapshot per FSM contract; the
        // remaining four are Heartbeats which exercise the
        // reset-on-receive without changing the view.
        let stream = futures::stream::unfold(0, |i| async move {
            if i >= 5 {
                return None;
            }
            tokio::time::sleep(std::time::Duration::from_millis(30)).await;
            let event = if i == 0 {
                empty_snapshot_event()
            } else {
                heartbeat_event(1_000 + i * 30)
            };
            Some((event, i + 1))
        })
        .boxed();

        let outcome =
            consume_directory_event_stream_with_idle_timeout(&mut client, &cell, stream, 50).await;
        assert_eq!(
            outcome,
            ConsumeOutcome::StreamEnded,
            "30ms-cadence stream must NOT trip the 50ms idle timeout"
        );
    }

    #[tokio::test(flavor = "multi_thread", worker_threads = 2)]
    async fn consume_with_idle_timeout_protocol_violation_aborts() {
        // FSM rejects an AgentAdvertised before Snapshot → consumer
        // returns ProtocolViolation, distinct from IdleTimeout.
        let mut client = RemoteDirectoryClient::new("realm-b".to_string());
        client.on_dial_ok();
        let cell = SharedFederatedDirectoryView::default();

        let stream = futures::stream::iter(vec![advertised_event(entry_with_claimed_origin(
            "easynet:///r/realm-b/device/sneaky",
            None,
        ))]);
        let outcome =
            consume_directory_event_stream_with_idle_timeout(&mut client, &cell, stream, 5_000)
                .await;
        assert!(matches!(outcome, ConsumeOutcome::ProtocolViolation(_)));
    }

    #[tokio::test(flavor = "multi_thread", worker_threads = 2)]
    async fn consume_directory_event_stream_protocol_violation_aborts() {
        // FSM rejects an AgentAdvertised before the mandatory Snapshot
        // → ProtocolViolation propagates as the consume's
        // error return, so the per-peer task tears down +
        // reconnects with backoff.
        let mut client = RemoteDirectoryClient::new("realm-b".to_string());
        client.on_dial_ok();
        let cell = SharedFederatedDirectoryView::default();

        let events = vec![advertised_event(entry_with_claimed_origin(
            "easynet:///r/realm-b/device/sneaky",
            None,
        ))];
        let stream = futures::stream::iter(events);

        let err = consume_directory_event_stream(&mut client, &cell, stream)
            .await
            .expect_err("AgentAdvertised before Snapshot must reject");
        assert!(matches!(err, FsmError::ProtocolViolation(_)));
        // View stays empty — the violation aborted before any
        // mutation could leak.
        let snap = cell.snapshot();
        if let Some(view) = snap.get("realm-b") {
            assert!(
                view.entries.is_empty(),
                "view must stay empty on protocol violation"
            );
        }
    }

    // ── Tier-3 fan-out (N3-4) ─────────────────────────────

    fn populated_cell_with_two_peers() -> SharedFederatedDirectoryView {
        // realm-b has device-X; realm-c has device-Y. Sorted
        // iteration gives realm-b first.
        let mut realm_b = DirectoryView::new("realm-b".to_string());
        realm_b.replace_entries(vec![entry_with_claimed_origin(
            "easynet:///r/realm-b/device/device-X",
            None,
        )]);
        let mut realm_c = DirectoryView::new("realm-c".to_string());
        realm_c.replace_entries(vec![entry_with_claimed_origin(
            "easynet:///r/realm-c/device/device-Y",
            None,
        )]);
        let mut peers = BTreeMap::new();
        peers.insert("realm-b".to_string(), Arc::new(realm_b));
        peers.insert("realm-c".to_string(), Arc::new(realm_c));
        SharedFederatedDirectoryView::new(peers)
    }

    #[test]
    fn lookup_in_federated_view_returns_hit_with_origin_realm_stamped() {
        let cell = populated_cell_with_two_peers();
        let entry =
            lookup_in_federated_view(&cell, "easynet:///r/realm-b/device/device-X").expect("hit");
        assert_eq!(entry.agent_ura, "easynet:///r/realm-b/device/device-X");
        assert_eq!(entry.origin_realm.as_deref(), Some("realm-b"));
    }

    #[test]
    fn lookup_in_federated_view_returns_none_when_not_found() {
        let cell = populated_cell_with_two_peers();
        assert!(lookup_in_federated_view(&cell, "easynet:///r/realm-x/device/missing").is_none());
    }

    #[test]
    fn lookup_in_federated_view_lex_tie_break_on_peer_realm() {
        // Two peers both claim the same URA (would be a real
        // misconfiguration in production, but the spec says the
        // tie-break is "lex order on peer_realm" so we pick the
        // earliest realm). BTreeMap iteration gives us this for
        // free, but pin the contract with a test.
        let mut realm_b = DirectoryView::new("realm-b".to_string());
        realm_b.apply_frame(&advertised_event(entry_with_claimed_origin(
            "easynet:///r/shared/device/dup",
            None,
        )));
        let mut realm_c = DirectoryView::new("realm-c".to_string());
        realm_c.apply_frame(&advertised_event(entry_with_claimed_origin(
            "easynet:///r/shared/device/dup",
            None,
        )));
        let mut peers = BTreeMap::new();
        peers.insert("realm-c".to_string(), Arc::new(realm_c));
        peers.insert("realm-b".to_string(), Arc::new(realm_b));
        let cell = SharedFederatedDirectoryView::new(peers);

        let entry = lookup_in_federated_view(&cell, "easynet:///r/shared/device/dup").expect("hit");
        // realm-b < realm-c, so realm-b wins.
        assert_eq!(entry.origin_realm.as_deref(), Some("realm-b"));
    }

    #[test]
    fn flatten_federated_view_returns_all_entries_in_lex_order() {
        let cell = populated_cell_with_two_peers();
        let entries = flatten_federated_view(&cell);
        assert_eq!(entries.len(), 2);
        // realm-b is iterated before realm-c by BTreeMap key
        // order; within each realm there's only one entry so
        // the inner order is moot.
        assert_eq!(entries[0].agent_ura, "easynet:///r/realm-b/device/device-X");
        assert_eq!(entries[0].origin_realm.as_deref(), Some("realm-b"));
        assert_eq!(entries[1].agent_ura, "easynet:///r/realm-c/device/device-Y");
        assert_eq!(entries[1].origin_realm.as_deref(), Some("realm-c"));
    }

    #[test]
    fn flatten_federated_view_is_empty_when_no_peers() {
        let cell = SharedFederatedDirectoryView::default();
        assert!(flatten_federated_view(&cell).is_empty());
    }

    // ── PresenceEvent → DirectoryEvent adapter (N3-streaming-1) ──

    #[cfg(feature = "axon-pb")]
    mod presence_adapter_tests {
        use super::*;
        use crate::services::presence_registry::{OfflineReason, PresenceEvent};

        #[test]
        fn presence_ura_to_directory_entry_extracts_node_id_from_canonical_shape() {
            let entry =
                presence_ura_to_directory_entry("easynet:///r/realm-a/device/device-X", true);
            assert_eq!(entry.agent_ura, "easynet:///r/realm-a/device/device-X");
            assert_eq!(entry.node_id, "device-X");
            assert_eq!(entry.status, "active");
            // Local hub speaks for own realm; chokepoint stamps
            // origin_realm on the receive side.
            assert_eq!(entry.origin_realm, None);
            assert_eq!(entry.display_name, None);
            assert_eq!(entry.hub_endpoint, None);
            assert_eq!(entry.last_seen_unix_ms, None);
        }

        #[test]
        fn presence_ura_to_directory_entry_inactive_marks_status_stale() {
            let entry =
                presence_ura_to_directory_entry("easynet:///r/realm-a/device/device-X", false);
            assert_eq!(entry.status, "stale");
        }

        #[test]
        fn presence_ura_to_directory_entry_treats_legacy_agent_shape_as_non_canonical() {
            let entry =
                presence_ura_to_directory_entry("easynet:///r/realm-a/agent/device-X", true);
            assert_eq!(entry.agent_ura, "easynet:///r/realm-a/agent/device-X");
            assert_eq!(entry.node_id, "easynet:///r/realm-a/agent/device-X");
        }

        #[test]
        fn presence_ura_to_directory_entry_falls_back_when_ura_non_canonical() {
            // Defensive — registry should never hold these, but
            // the adapter must produce a non-empty node_id
            // anyway so downstream code never sees an empty key.
            let entry = presence_ura_to_directory_entry("not-canonical", true);
            assert_eq!(entry.node_id, "not-canonical");
            assert_eq!(entry.agent_ura, "not-canonical");
        }

        #[test]
        fn presence_event_online_projects_to_agent_advertised() {
            let evt = presence_event_to_directory_event_at(
                &PresenceEvent::Online {
                    ura: "easynet:///r/realm-a/device/x".to_string(),
                },
                1_714_492_800_000,
            );
            match evt {
                DirectoryEvent::AgentAdvertised {
                    agent_ura,
                    signing_authority,
                    replaced_prior,
                    unix_ms,
                } => {
                    assert_eq!(agent_ura, "easynet:///r/realm-a/device/x");
                    assert_eq!(signing_authority, SigningAuthority::SelfSigned);
                    assert!(!replaced_prior);
                    assert_eq!(unix_ms, 1_714_492_800_000);
                }
                _ => panic!("expected AgentAdvertised; got {evt:?}"),
            }
        }

        #[test]
        fn presence_event_offline_projects_to_agent_revoked_with_reason_string() {
            let cases = [
                (OfflineReason::StreamClosed, "stream_closed"),
                (OfflineReason::StreamReset, "stream_reset"),
                (OfflineReason::SendFailed, "send_failed"),
                (OfflineReason::AdminRevoked, "admin_revoked"),
            ];
            for (reason, expected_str) in cases {
                let evt = presence_event_to_directory_event_at(
                    &PresenceEvent::Offline {
                        ura: "easynet:///r/realm-a/device/x".to_string(),
                        reason,
                    },
                    1_714_492_800_000,
                );
                match evt {
                    DirectoryEvent::AgentRevoked {
                        agent_ura,
                        was_active,
                        reason,
                        unix_ms,
                    } => {
                        assert_eq!(agent_ura, "easynet:///r/realm-a/device/x");
                        assert!(was_active);
                        assert_eq!(reason, expected_str);
                        assert_eq!(unix_ms, 1_714_492_800_000);
                    }
                    other => panic!("expected AgentRevoked for {reason:?}; got {other:?}"),
                }
            }
        }
    }

    // ── PollOnce integration (N3-3.1) ─────────────────────────

    #[cfg(feature = "axon-pb")]
    mod poll_tests {
        use super::*;
        use crate::services::federation_client::{FederationClient, FederationClientError, HubUri};
        use async_trait::async_trait;
        use easynet_axon::pb::axon::v1::{InvokeRequest, InvokeResponse};
        use std::sync::Mutex;

        /// Mock FederationClient. Returns canned discover
        /// responses keyed by target_hub endpoint.
        struct CannedClient {
            responses: Mutex<std::collections::BTreeMap<String, Vec<u8>>>,
        }

        #[async_trait]
        impl FederationClient for CannedClient {
            async fn forward_invoke(
                &self,
                target_hub: &HubUri,
                _request: InvokeRequest,
            ) -> Result<InvokeResponse, FederationClientError> {
                let bytes = self
                    .responses
                    .lock()
                    .unwrap()
                    .get(target_hub)
                    .cloned()
                    .unwrap_or_default();
                Ok(InvokeResponse {
                    result: bytes,
                    ..Default::default()
                })
            }
        }

        struct DialFailedClient;
        #[async_trait]
        impl FederationClient for DialFailedClient {
            async fn forward_invoke(
                &self,
                target_hub: &HubUri,
                _request: InvokeRequest,
            ) -> Result<InvokeResponse, FederationClientError> {
                Err(FederationClientError::DialFailed {
                    hub: target_hub.clone(),
                    detail: "test-injected".to_string(),
                })
            }
        }

        fn build_canned_response(entries: Vec<DirectoryEntry>) -> Vec<u8> {
            let resp =
                crate::services::invocation_transport::federation_wrappers::DiscoverResponse {
                    entries,
                };
            serde_json::to_vec(&resp).unwrap()
        }

        #[tokio::test(flavor = "multi_thread", worker_threads = 2)]
        async fn poll_once_writes_per_peer_view_into_cell() {
            let client = CannedClient {
                responses: Mutex::new(
                    [(
                        "https://hub-b.example:50443".to_string(),
                        build_canned_response(vec![entry_with_claimed_origin(
                            "easynet:///r/realm-b/device/peer-device",
                            // peer claims wrong origin_realm; rewrite gate must fix it
                            Some("trusted-bank"),
                        )]),
                    )]
                    .into_iter()
                    .collect(),
                ),
            };
            let mut peers = std::collections::BTreeMap::new();
            peers.insert(
                "realm-b".to_string(),
                "https://hub-b.example:50443".to_string(),
            );
            let cell = SharedFederatedDirectoryView::default();

            let outcome = poll_once(&client, &peers, Some("easynet:///r/realm-a/hub"), &cell).await;

            assert_eq!(outcome.successful_peers, vec!["realm-b".to_string()]);
            assert!(outcome.failed_peers.is_empty());

            let snap = cell.snapshot();
            let realm_b_view = snap.get("realm-b").expect("realm-b in cell");
            let entry = realm_b_view
                .lookup("easynet:///r/realm-b/device/peer-device")
                .expect("entry in view");
            // §2.4 chokepoint: receiving hub stamps peer's
            // authenticated realm regardless of peer's claim.
            assert_eq!(
                entry.origin_realm.as_deref(),
                Some("realm-b"),
                "poll_once must enforce §2.4 origin_realm rewrite"
            );
        }

        #[tokio::test(flavor = "multi_thread", worker_threads = 2)]
        async fn poll_once_dial_failure_records_in_outcome_and_preserves_old_view() {
            let cell = SharedFederatedDirectoryView::default();
            // Pre-populate realm-b's view (simulating an earlier
            // successful poll). The next round fails to dial;
            // the prior view MUST stay intact (no flicker).
            let mut prior_view = DirectoryView::new("realm-b".to_string());
            prior_view.replace_entries(vec![entry_with_claimed_origin(
                "easynet:///r/realm-b/device/persisted",
                None,
            )]);
            let mut prior_map = std::collections::BTreeMap::new();
            prior_map.insert("realm-b".to_string(), Arc::new(prior_view));
            cell.replace(prior_map);

            let mut peers = std::collections::BTreeMap::new();
            peers.insert(
                "realm-b".to_string(),
                "https://hub-b.example:50443".to_string(),
            );

            let outcome = poll_once(&DialFailedClient, &peers, None, &cell).await;

            assert!(outcome.successful_peers.is_empty());
            assert_eq!(outcome.failed_peers.len(), 1);
            assert_eq!(outcome.failed_peers[0].0, "realm-b");

            let snap = cell.snapshot();
            assert!(
                snap.get("realm-b")
                    .expect("realm-b view preserved")
                    .lookup("easynet:///r/realm-b/device/persisted")
                    .is_some(),
                "dial failure MUST NOT clear the previously-cached view"
            );
        }

        #[tokio::test(flavor = "multi_thread", worker_threads = 2)]
        async fn poll_once_skips_peers_marked_streamed() {
            // PR-N3 N3-streaming-10. The streaming supervisor
            // marks realm-b as streamed via cell.mark_streamed.
            // The poll task must skip realm-b on every
            // subsequent round so it cannot overwrite a fresh
            // stream-emitted entry with a stale poll snapshot.
            //
            // Pre-populate the cell with a fresh entry the
            // streaming supervisor would have written; the poll
            // mock returns an empty discover response. If the
            // poll task didn't skip, it would overwrite the
            // realm-b view with the empty response and the
            // entry would disappear. Skipping preserves the
            // entry.
            let cell = SharedFederatedDirectoryView::default();
            let mut realm_b_view = DirectoryView::new("realm-b".to_string());
            realm_b_view.replace_entries(vec![entry_with_claimed_origin(
                "easynet:///r/realm-b/device/streamed",
                None,
            )]);
            let mut prior_map = std::collections::BTreeMap::new();
            prior_map.insert("realm-b".to_string(), Arc::new(realm_b_view));
            cell.replace(prior_map);
            cell.mark_streamed("realm-b");

            // Mock returns an empty DiscoverResponse — if
            // poll_once didn't skip, this would clear the view.
            let client = CannedClient {
                responses: Mutex::new(
                    [(
                        "https://hub-b.example:50443".to_string(),
                        build_canned_response(vec![]),
                    )]
                    .into_iter()
                    .collect(),
                ),
            };
            let mut peers = std::collections::BTreeMap::new();
            peers.insert(
                "realm-b".to_string(),
                "https://hub-b.example:50443".to_string(),
            );

            let outcome = poll_once(&client, &peers, None, &cell).await;
            // realm-b is in successful_peers (no error) but
            // the poll didn't actually dial or replace.
            assert_eq!(outcome.successful_peers, vec!["realm-b".to_string()]);
            assert!(outcome.failed_peers.is_empty());

            // The pre-populated entry survives.
            let snap = cell.snapshot();
            assert!(
                snap.get("realm-b")
                    .and_then(|v| v.lookup("easynet:///r/realm-b/device/streamed"))
                    .is_some(),
                "streamed peer's entry MUST NOT be cleared by a concurrent poll"
            );
        }

        #[test]
        fn streamed_marker_round_trips_via_cell_api() {
            // Pure-data sanity test for mark/unmark/is_streamed.
            let cell = SharedFederatedDirectoryView::default();
            assert!(!cell.is_streamed("realm-b"));
            cell.mark_streamed("realm-b");
            assert!(cell.is_streamed("realm-b"));
            assert!(!cell.is_streamed("realm-c"));
            cell.unmark_streamed("realm-b");
            assert!(!cell.is_streamed("realm-b"));
        }

        #[tokio::test(flavor = "multi_thread", worker_threads = 2)]
        async fn poll_once_with_empty_peers_clears_nothing() {
            let cell = SharedFederatedDirectoryView::default();
            // Pre-populate realm-b. Empty peers map ⇒ no dials,
            // no replaces — the existing view stays.
            let mut prior = DirectoryView::new("realm-b".to_string());
            prior.apply_frame(&advertised_event(entry_with_claimed_origin(
                "easynet:///r/realm-b/device/x",
                None,
            )));
            let mut prior_map = std::collections::BTreeMap::new();
            prior_map.insert("realm-b".to_string(), Arc::new(prior));
            cell.replace(prior_map);

            let outcome = poll_once(
                &CannedClient {
                    responses: Mutex::new(std::collections::BTreeMap::new()),
                },
                &std::collections::BTreeMap::new(),
                None,
                &cell,
            )
            .await;
            assert!(outcome.successful_peers.is_empty());
            assert!(outcome.failed_peers.is_empty());

            // Cell still holds the pre-poll view.
            let snap = cell.snapshot();
            assert!(
                snap.get("realm-b")
                    .is_some_and(|v| v.lookup("easynet:///r/realm-b/device/x").is_some()),
                "empty peers map must not clear existing views"
            );
        }
    }

    #[test]
    fn shared_federated_directory_view_replace_publishes_atomically() {
        let cell = SharedFederatedDirectoryView::default();
        // Take snapshot 1 BEFORE replace.
        let snap1 = cell.snapshot();
        assert!(snap1.is_empty());

        let mut next = BTreeMap::new();
        let mut peer_view = DirectoryView::new("realm-b".to_string());
        peer_view.apply_frame(&advertised_event(entry_with_claimed_origin(
            "easynet:///r/realm-b/device/peer",
            None,
        )));
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
            .lookup("easynet:///r/realm-b/device/peer")
            .is_some());
    }
}
