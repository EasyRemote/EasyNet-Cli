// EasyNet CLI — cross-hub gRPC outbound dialer (PR-N1 commit 4/N)
// =================================================================
//
// File: src/services/federation_client/cross_hub_dial.rs
//
// PR-N1 commit 4/N — adds timeout + per-peer circuit-breaker on
// top of the schema-B + TLS-pinned dial shipped by commit 2/N
// (`ca081bc`). The handler `dispatch_federation_forward_invoke`
// rewrite shipped by commit 3a/N + 3b/N (`b3a06f4` + `57a42df`)
// already routes cross-tenant calls through this dialer; commit
// 4/N hardens the dial path so a slow / dead peer cannot stall
// the local hub indefinitely.
//
// Forward-invoke shape after this commit:
//
//   forward_invoke(target_hub, req)
//     ├─ trust_anchor.lookup_peer_hub(target_hub)?  ← schema-B gate
//     ├─ check_breaker_open(target_hub)?            ← commit 4/N
//     ├─ resolve_peer_channel(target_hub) → cached or new
//     │     TLS-pinned tonic Channel
//     ├─ tokio::time::timeout(forward_invoke_timeout,
//     │     InvocationClient::new(channel).invoke(req)).await
//     └─ on success → record_breaker_success(target_hub)
//        on failure → record_breaker_failure(target_hub) + return
//
// Breaker state machine (per peer, `Arc<DashMap<HubUri, BreakerState>>`):
//
//   Closed:                   normal operation; failures count toward
//                             threshold; success resets counter.
//   Open(opened_at):          fail-fast `CircuitOpen`; no dial
//                             attempted. After `breaker_reset_window`
//                             elapses, transitions to HalfOpen on
//                             next call.
//   HalfOpen:                 single trial dial allowed; success →
//                             Closed; failure → Open(now).
//
// What is NOT in this commit
// --------------------------
// - mTLS outbound identity. PR-N2 cross-realm admission territory.
// - DaemonConfig wiring of `federation_timeout_ms` /
//   `circuit_breaker_threshold`. The dialer accepts the values via
//   the `with_breaker_*` builders so tests can drive them; boot
//   wiring lands alongside the cross-hub e2e in commit 5/N (or a
//   pre-e2e operator-config commit). Defaults match the spec
//   (forward 30s, dial 10s, threshold 3 fails, window 60s).
//   trait.
//
// Author: Silan.Hu <silan.hu@u.nus.edu>
// Copyright (c) 2026 EasyNet. All rights reserved.

use std::sync::Arc;
use std::time::{Duration, Instant};

use async_trait::async_trait;
use dashmap::DashMap;
use tonic::transport::{Certificate, Channel, ClientTlsConfig, Endpoint};

use crate::pb::axon::v1::invocation_client::InvocationClient;
use crate::pb::axon::v1::{InvokeRequest, InvokeResponse};
use crate::services::realm_trust_anchor::RealmTrustAnchor;
use crate::services::trust_anchor_cell::SharedTrustAnchor;

/// PR-N1 commit 9/N: how the dialer reads the trust anchor on
/// every dial. Two flavours:
///
/// - `Snapshot(Arc<RealmTrustAnchor>)` — the legacy boot-time
///   snapshot wired by commits 2/N–6/N. SIGHUP-triggered
///   reloads to `realm-trust.toml` do NOT propagate; operators
///   must restart the daemon for the dialer to pick up new
///   federation peer entries. Tests under `cross_hub_dial::tests`
///   construct the dialer with this flavour.
///
/// - `Live(SharedTrustAnchor)` — the SIGHUP-aware cell PR-7's
///   `register_device_pubkey` already wires for the admission
///   facade. `lookup_peer_hub` snapshots the cell on every dial,
///   so a cell `replace` (driven by SIGHUP reload or pairing
///   flow) is visible to the next federation dispatch within
///   the cell's per-RPC snapshot cost (~50ms per
///   `perf-notes/PR-N1-commit-6-perf-cross-pass-by-xiaowen.md`).
///   Production `start_axon_serve_sidecar` wires this flavour.
///
/// 晓雯 letter 67 attack round 4 catch: the boot-time snapshot
/// pinned the daemon's federation-peer view at boot, blocking
/// CTO's iterate-config-without-restart cadence. 凉冰 LB-37
/// ratify ship-now of this enum so the same `Arc<dyn
/// FederationClient>` surface stays — only the read-source
/// changes.
#[derive(Clone)]
enum TrustSource {
    Snapshot(Arc<RealmTrustAnchor>),
    Live(SharedTrustAnchor),
}

impl TrustSource {
    /// Take a stable view of the trust anchor for one dial. For
    /// the snapshot flavour this is a `Arc::clone` (cheap); for
    /// the live flavour this acquires the cell's `RwLock::read()`,
    /// clones the inner `Arc`, and releases the lock before the
    /// caller's lookup runs (mirrors the admission gate's
    /// per-call pattern).
    fn snapshot(&self) -> Arc<RealmTrustAnchor> {
        match self {
            TrustSource::Snapshot(anchor) => Arc::clone(anchor),
            TrustSource::Live(cell) => cell.snapshot(),
        }
    }
}

/// Default per-call timeout for `forward_invoke`. Spec §commit 4/N:
/// 30s. The end-to-end caller (the hub-side admission gate) has its
/// own admission deadline; 30s is generous enough to absorb a
/// reasonable peer admission round-trip without stranding the
/// caller on a dead peer.
pub const DEFAULT_FORWARD_INVOKE_TIMEOUT: Duration = Duration::from_secs(30);

/// Default failure threshold before the breaker opens. Spec
/// §commit 4/N: 3 consecutive failures.
pub const DEFAULT_BREAKER_FAILURE_THRESHOLD: u32 = 3;

/// Default reset window — once the breaker is `Open(at)`, the next
/// call after `now() - at >= window` transitions to `HalfOpen` for
/// a single trial dial. Spec §commit 4/N: 60s.
pub const DEFAULT_BREAKER_RESET_WINDOW: Duration = Duration::from_secs(60);

/// Per-peer breaker state. Pure data — no I/O happens inside the
/// state transitions; the dialer reads + updates the entry under
/// a `DashMap` lock.
#[derive(Clone, Debug)]
enum BreakerState {
    /// Normal operation. `consecutive_failures` is the running
    /// count toward the threshold; reset to 0 on each success.
    Closed { consecutive_failures: u32 },
    /// Fail-fast. `opened_at` is the instant the breaker opened;
    /// the next call after `opened_at + reset_window` transitions
    /// to `HalfOpen`.
    Open { opened_at: Instant },
    /// One trial dial allowed. Success → `Closed { 0 }`; failure
    /// → `Open { opened_at: now() }`. Concurrent callers hitting
    /// `HalfOpen` race on the trial — the loser's call also gets
    /// dispatched (a small overshoot is acceptable; the alternative
    /// would be a per-peer mutex which adds contention to the hot
    /// path).
    HalfOpen,
}

impl Default for BreakerState {
    fn default() -> Self {
        Self::Closed {
            consecutive_failures: 0,
        }
    }
}

/// Canonical hub URI string used as the federation peer key. We
/// intentionally do not introduce a newtype wrapper around
/// `String` for v1 — the URI is parsed by `tonic::transport::
/// Endpoint::from_shared` at dial time, and additional structure
/// at the type level would foreclose future URI-shape evolution
/// (DEC-N3 §"hub URI carrier").
///
/// Examples:
///   "https://hub-a.example.com:50443"
///   "https://10.0.0.7:50443"
pub type HubUri = String;

/// Outcome of a cross-hub `forward_invoke` attempt.
///
/// Each variant is a wire-stable identifier — audit pipelines and
/// metrics consumers grep on these values, so renaming any is a
/// protocol-level change that requires an RFC amendment.
#[derive(Debug, thiserror::Error)]
pub enum FederationClientError {
    /// The peer hub URI is not present in the local
    /// `RealmTrustAnchor` with the federation role + non-empty
    /// origin tenant id (DEC-N1 schema-B). Fail-closed:
    /// admission's federated trust set is the only authority on
    /// which peers we may dial.
    #[error("federation peer `{0}` is not in the realm trust anchor; cross-hub dial refused")]
    PeerNotTrusted(HubUri),

    /// `tonic::transport::Channel::connect` failed (TCP, TLS,
    /// HTTP/2 handshake — anything below the gRPC layer). The
    /// message carries the underlying tonic error verbatim so
    /// operators can grep without losing diagnostic detail.
    #[error("federation dial to `{hub}` failed: {detail}")]
    DialFailed { hub: HubUri, detail: String },

    /// The cross-hub channel exceeded the configured timeout
    /// (PR-N1 spec INV-4: 30s for `forward_invoke`, 10s for
    /// dial). Maps onto `target_offline` in the wire response so
    /// callers fall back to local cache / retry policy.
    #[error("federation channel to `{0}` timed out")]
    ChannelTimeout(HubUri),

    /// The peer hub returned a `tonic::Status` from the inner
    /// `Invoke`. Wrapping rather than collapsing preserves the
    /// peer's error code so the local caller can replay the
    /// peer's reject reason verbatim (e.g.
    /// `AXON_CALLER_SIGNATURE_INVALID` from cross-realm
    /// admission).
    #[error("federation peer `{hub}` returned: {status}")]
    InnerInvokeFailed { hub: HubUri, status: String },

    /// Circuit-breaker open — the peer has had ≥ 3 consecutive
    /// failures within the breaker window and we refuse new
    /// dials until half-open elapses. Avoids hammering an
    /// unreachable peer. Implementation lands in PR-N1 commit
    /// 4/N; commit 1/N reserves the variant.
    #[error("federation circuit-breaker open for `{0}`")]
    CircuitOpen(HubUri),
}

/// Abstract surface every `federation.forward_invoke` cross-hub
/// dispatcher consumes. Trait shape mirrors
/// `daemon_grpc::Client::Invoke` so audit pipelines and tests can
/// swap in mocks without touching call sites.
#[async_trait]
pub trait FederationClient: Send + Sync {
    /// Forward an `InvokeRequest` to `target_hub` and return its
    /// response. Implementations MUST:
    ///
    /// 1. Look up `target_hub` in the trust anchor. Reject with
    ///    `PeerNotTrusted` if the entry is missing or its
    ///    `origin_tenant_id` is `None` (DEC-N1).
    /// 2. Re-use a cached `tonic::transport::Channel` per peer
    ///    (PR-N1 spec INV-5) — fresh channel per call would
    ///    burn TLS handshakes.
    /// 3. NOT retry on `forward_invoke`. The user-facing call
    ///    has its own idempotency assumptions; only dial-level
    ///    transient failures are retried (PR-N1 commit 4/N).
    async fn forward_invoke(
        &self,
        target_hub: &HubUri,
        request: InvokeRequest,
    ) -> Result<InvokeResponse, FederationClientError>;
}

/// tonic-backed concrete implementation. Holds:
/// - `trust_source` — how the peer trust gate reads the
///   `RealmTrustAnchor`. PR-N1 commit 9/N: either a boot-time
///   snapshot (legacy / test fixtures) or a live `SharedTrustAnchor`
///   cell (production), via the `TrustSource` enum above. The
///   peer trust gate (`lookup_peer_hub`) calls
///   `trust_source.snapshot()` on every dial so a SIGHUP-driven
///   reload of `realm-trust.toml` is visible to the next
///   federation dispatch without requiring a daemon restart.
/// - `channels` — `Arc<DashMap<HubUri, Channel>>` peer-channel
///   cache (PR-N1 spec INV-5). Lock-free — the hot path reads the
///   map on every cross-hub call so `RwLock<HashMap>` would be a
///   regression vs. `PresenceRegistry`'s existing pattern.
///
/// Constructed once per daemon process at boot (alongside the
/// inbound `start_axon_serve_sidecar` listener) and cloned
/// cheaply into per-RPC dispatch tasks.
#[derive(Clone)]
pub struct CrossHubDialer {
    trust_source: TrustSource,
    channels: Arc<DashMap<HubUri, Channel>>,
    /// **PR-N1 commit 4/N**. Per-peer breaker state. Lock-free
    /// `DashMap` matches the channel cache shape so admission +
    /// breaker contention stay symmetric on the hot path.
    breaker_state: Arc<DashMap<HubUri, BreakerState>>,
    /// **PR-N1 commit 4/N**. Per-call timeout for the inner
    /// `InvocationClient::invoke`. Wraps with
    /// `tokio::time::timeout`; expiration surfaces as
    /// `FederationClientError::ChannelTimeout`.
    forward_invoke_timeout: Duration,
    /// **PR-N1 commit 4/N**. Consecutive-failure threshold before
    /// the breaker opens. Default 3 per spec; tests inject 1 so
    /// they can exercise the open transition without dispatching
    /// 3 stub-failure calls each.
    breaker_failure_threshold: u32,
    /// **PR-N1 commit 4/N**. Window the breaker holds Open before
    /// allowing a trial HalfOpen dial. Default 60s per spec; tests
    /// inject 100ms so the auto-reset path is exercisable in unit
    /// time.
    breaker_reset_window: Duration,
}

impl std::fmt::Debug for CrossHubDialer {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("CrossHubDialer")
            .field("cached_peers", &self.channels.len())
            .field("tracked_peers_breaker_state", &self.breaker_state.len())
            .field("forward_invoke_timeout", &self.forward_invoke_timeout)
            .field("breaker_failure_threshold", &self.breaker_failure_threshold)
            .field("breaker_reset_window", &self.breaker_reset_window)
            .finish_non_exhaustive()
    }
}

impl CrossHubDialer {
    /// Construct a dialer holding a boot-time `Arc<RealmTrustAnchor>`
    /// snapshot. SIGHUP-triggered trust-anchor reloads do NOT
    /// propagate; this constructor is the legacy / test-fixture
    /// flavour. Production daemons use [`with_trust_anchor_cell`]
    /// (PR-N1 commit 9/N) instead so the hot-reload cadence
    /// 晓雯 letter 67 attack round 4 raised actually works.
    ///
    /// PR-N1 commit 4/N adds breaker + timeout fields; their
    /// defaults match the PR-N1 spec (`30s` per-call timeout, `3`
    /// consecutive failures opens the breaker, `60s` open window).
    /// Tests override via the `with_*` builders below.
    #[must_use]
    pub fn new(trust_anchor: Arc<RealmTrustAnchor>) -> Self {
        Self::from_trust_source(TrustSource::Snapshot(trust_anchor))
    }

    /// **PR-N1 commit 9/N**. Construct a dialer whose peer trust
    /// gate snapshots the supplied `SharedTrustAnchor` cell on
    /// every dial. SIGHUP-driven `realm-trust.toml` reloads are
    /// visible to the next federation dispatch without
    /// reconstructing the dialer or restarting the daemon.
    ///
    /// `services/axon_serve/boot.rs::start_axon_serve_sidecar`
    /// uses this constructor in `Hub` / `Both` modes so operators
    /// editing the federation peer set (adding `[[trusted_agent]]
    /// role = "hub"` blocks with the schema-B `origin_tenant_id` /
    /// `hub_uri` / `tls_ca_pem_path` fields) only need
    /// `kill -HUP <daemon_pid>` — no restart, no in-flight
    /// invoke loss.
    #[must_use]
    pub fn with_trust_anchor_cell(cell: SharedTrustAnchor) -> Self {
        Self::from_trust_source(TrustSource::Live(cell))
    }

    fn from_trust_source(trust_source: TrustSource) -> Self {
        Self {
            trust_source,
            channels: Arc::new(DashMap::new()),
            breaker_state: Arc::new(DashMap::new()),
            forward_invoke_timeout: DEFAULT_FORWARD_INVOKE_TIMEOUT,
            breaker_failure_threshold: DEFAULT_BREAKER_FAILURE_THRESHOLD,
            breaker_reset_window: DEFAULT_BREAKER_RESET_WINDOW,
        }
    }

    /// **PR-N1 commit 4/N**. Override the per-call timeout. Tests
    /// inject `Duration::from_millis(50)` to drive the
    /// `ChannelTimeout` path without sleeping a real 30s; production
    /// daemons accept the default unless `DaemonConfig::
    /// federation_timeout_ms` overrides at boot.
    #[must_use]
    pub fn with_forward_invoke_timeout(mut self, timeout: Duration) -> Self {
        self.forward_invoke_timeout = timeout;
        self
    }

    /// **PR-N1 commit 4/N**. Override the consecutive-failure
    /// threshold that opens the breaker. Tests inject `1` so a
    /// single stub-failure call exercises the open transition.
    #[must_use]
    pub fn with_breaker_failure_threshold(mut self, threshold: u32) -> Self {
        self.breaker_failure_threshold = threshold;
        self
    }

    /// **PR-N1 commit 4/N**. Override the breaker open-state
    /// window. Tests inject `Duration::from_millis(50)` so the
    /// auto-reset path is exercisable in unit-test time.
    #[must_use]
    pub fn with_breaker_reset_window(mut self, window: Duration) -> Self {
        self.breaker_reset_window = window;
        self
    }

    /// Number of cached peer channels. Test/observability only.
    #[must_use]
    pub fn cached_peer_count(&self) -> usize {
        self.channels.len()
    }

    /// **PR-N1 commit 4/N**. Read-only inspection of a peer's
    /// current breaker state. Test/observability only —
    /// production callers do not branch on this directly; they
    /// see the typed `CircuitOpen` error from `forward_invoke`.
    /// Returns `None` when the peer has never been dialed (no
    /// breaker entry tracked yet, semantically equivalent to
    /// `Closed { 0 }`).
    fn breaker_is_closed(&self, target_hub: &HubUri) -> bool {
        match self.breaker_state.get(target_hub).map(|e| e.value().clone()) {
            None => true,
            Some(BreakerState::Closed { .. }) => true,
            Some(BreakerState::Open { .. }) | Some(BreakerState::HalfOpen) => false,
        }
    }

    /// **PR-N1 commit 4/N**. Read the breaker state and decide
    /// whether to dispatch the call. Returns `Ok(())` when the
    /// dial may proceed (Closed or HalfOpen), or
    /// `CircuitOpen` when the breaker is Open within its reset
    /// window. The Open → HalfOpen transition happens here as a
    /// side effect of the read so HalfOpen behaviour is
    /// consistent across concurrent callers.
    fn check_and_advance_breaker(
        &self,
        target_hub: &HubUri,
    ) -> Result<(), FederationClientError> {
        // `entry()` ensures we get exclusive access for the
        // read-modify-write transition. `or_default()` materialises
        // a `Closed { 0 }` entry on first dial — saves a
        // separate `insert` later.
        let mut entry = self.breaker_state.entry(target_hub.clone()).or_default();
        match &*entry {
            BreakerState::Closed { .. } | BreakerState::HalfOpen => Ok(()),
            BreakerState::Open { opened_at } => {
                if opened_at.elapsed() >= self.breaker_reset_window {
                    *entry = BreakerState::HalfOpen;
                    Ok(())
                } else {
                    Err(FederationClientError::CircuitOpen(target_hub.clone()))
                }
            }
        }
    }

    /// **PR-N1 commit 4/N**. Record a successful dial outcome.
    /// Closed → reset the failure counter; HalfOpen → Closed.
    fn record_breaker_success(&self, target_hub: &HubUri) {
        let mut entry = self.breaker_state.entry(target_hub.clone()).or_default();
        *entry = BreakerState::Closed {
            consecutive_failures: 0,
        };
    }

    /// **PR-N1 commit 4/N**. Record a failure. Closed: bump the
    /// counter, transitioning to Open if the threshold is met.
    /// HalfOpen → Open (the trial dial failed). Open: idempotent.
    fn record_breaker_failure(&self, target_hub: &HubUri) {
        let mut entry = self.breaker_state.entry(target_hub.clone()).or_default();
        let next = match &*entry {
            BreakerState::Closed {
                consecutive_failures,
            } => {
                let bumped = consecutive_failures.saturating_add(1);
                if bumped >= self.breaker_failure_threshold {
                    BreakerState::Open {
                        opened_at: Instant::now(),
                    }
                } else {
                    BreakerState::Closed {
                        consecutive_failures: bumped,
                    }
                }
            }
            BreakerState::HalfOpen => BreakerState::Open {
                opened_at: Instant::now(),
            },
            BreakerState::Open { opened_at } => BreakerState::Open {
                opened_at: *opened_at,
            },
        };
        *entry = next;
    }

    /// Resolve a `tonic::transport::Channel` for `target_hub`,
    /// reusing a cached channel when the peer has been dialed
    /// before. The trust-anchor entry the caller already verified
    /// supplies the operator-pinned CA path; the channel is built
    /// with `ClientTlsConfig::new().ca_certificate(...)`. There is
    /// no system-CA fallback by design (DEC-N1: trust set is
    /// authoritative).
    ///
    /// Cache semantics: `DashMap::entry::or_try_insert_with` is
    /// the natural fit but tonic's `Channel` is `Clone` so a
    /// straightforward "miss → build → insert" works. A second
    /// concurrent miss may build a duplicate channel that loses
    /// the insert race; the loser is dropped with no observable
    /// harm (TLS handshake cost is the lone waste — bounded by
    /// the number of concurrent first-cross-hub calls per peer
    /// at boot).
    fn resolve_peer_channel(
        &self,
        target_hub: &HubUri,
        ca_pem_path: &std::path::Path,
    ) -> Result<Channel, FederationClientError> {
        if let Some(cached) = self.channels.get(target_hub) {
            return Ok(cached.clone());
        }

        let ca_pem = std::fs::read(ca_pem_path).map_err(|err| {
            FederationClientError::DialFailed {
                hub: target_hub.clone(),
                detail: format!(
                    "read tls_ca_pem_path `{}`: {err}",
                    ca_pem_path.display()
                ),
            }
        })?;
        let ca = Certificate::from_pem(&ca_pem);
        let tls = ClientTlsConfig::new().ca_certificate(ca);

        let endpoint = Endpoint::from_shared(target_hub.clone())
            .map_err(|err| FederationClientError::DialFailed {
                hub: target_hub.clone(),
                detail: format!("invalid hub_uri `{target_hub}`: {err}"),
            })?
            .tls_config(tls)
            .map_err(|err| FederationClientError::DialFailed {
                hub: target_hub.clone(),
                detail: format!("apply tls_config: {err}"),
            })?;

        // `connect_lazy` defers the real TCP+TLS handshake until
        // the first RPC. Two reasons we prefer it here:
        //  1. Boot does not stall on unreachable peers — the
        //     cross-hub call surfaces the dial error as the
        //     RPC's failure, not as a daemon startup failure.
        //  2. The `Endpoint` is single-use; `connect()` would
        //     consume `self` and force re-building the TLS
        //     config for every retry. `connect_lazy` returns a
        //     Channel that retries internally on a per-RPC
        //     basis, matching the trait's "no retry on
        //     forward_invoke" contract.
        let channel = endpoint.connect_lazy();
        self.channels.insert(target_hub.clone(), channel.clone());
        Ok(channel)
    }
}

#[async_trait]
impl FederationClient for CrossHubDialer {
    async fn forward_invoke(
        &self,
        target_hub: &HubUri,
        request: InvokeRequest,
    ) -> Result<InvokeResponse, FederationClientError> {
        // ── 1. Trust gate ────────────────────────────────────
        // `lookup_peer_hub` enforces the schema-B contract:
        // role == Hub AND origin_tenant_id.is_some() AND
        // hub_uri == target_hub. A peer that fails any of those
        // is `PeerNotTrusted`. We additionally require
        // `tls_ca_pem_path.is_some()` since DEC-N1 forbids the
        // dialer from falling back to system CAs.
        //
        // PR-N1 commit 9/N: snapshot the trust source per-dial
        // so a SIGHUP-driven `realm-trust.toml` reload (PR-7
        // mechanism) is visible to the next dispatch without a
        // daemon restart. The snapshot is one `Arc::clone`
        // (legacy `Snapshot` source) or one `RwLock::read()` +
        // `Arc::clone` (production `Live` source) — both cheap
        // enough that hot-path latency stays inside the budget
        // 晓雯 LB-31 §3.3 ratified.
        let trust_snapshot = self.trust_source.snapshot();
        let entry = trust_snapshot
            .lookup_peer_hub(target_hub)
            .ok_or_else(|| FederationClientError::PeerNotTrusted(target_hub.clone()))?;
        let ca_path = entry
            .tls_ca_pem_path
            .as_deref()
            .ok_or_else(|| FederationClientError::PeerNotTrusted(target_hub.clone()))?;

        // ── 2. Breaker gate (PR-N1 commit 4/N) ───────────────
        // Open + within reset window → fail-fast `CircuitOpen`.
        // Open + past reset window → Open transitions to HalfOpen,
        // this call is the trial dial. Closed → proceed normally.
        self.check_and_advance_breaker(target_hub)?;

        // ── 3. Resolve channel (cached or fresh TLS-pinned) ──
        let channel = match self.resolve_peer_channel(target_hub, ca_path) {
            Ok(channel) => channel,
            Err(err) => {
                // A channel-build failure is a peer-reachability
                // event (cert read error, malformed hub URI). It
                // counts toward the breaker so a typo'd
                // `tls_ca_pem_path` can't pin the breaker open
                // forever — the next operator save flushes the
                // bad entry and the breaker auto-resets.
                self.record_breaker_failure(target_hub);
                return Err(err);
            }
        };

        // ── 4. Inner Invoke wrapped in timeout (commit 4/N) ──
        let mut client = InvocationClient::new(channel);
        let outcome =
            tokio::time::timeout(self.forward_invoke_timeout, client.invoke(request)).await;

        match outcome {
            Ok(Ok(response)) => {
                self.record_breaker_success(target_hub);
                Ok(response.into_inner())
            }
            Ok(Err(status)) => {
                self.record_breaker_failure(target_hub);
                Err(FederationClientError::InnerInvokeFailed {
                    hub: target_hub.clone(),
                    status: format!("code={:?} message={}", status.code(), status.message()),
                })
            }
            Err(_elapsed) => {
                self.record_breaker_failure(target_hub);
                Err(FederationClientError::ChannelTimeout(target_hub.clone()))
            }
        }
    }
}

#[cfg(test)]
mod tests {
    //! PR-N1 commit 2/N tests:
    //! - Trust gate: peer not in anchor / wrong role / missing
    //!   `origin_tenant_id` / missing `tls_ca_pem_path` all surface
    //!   as `PeerNotTrusted` (DEC-N1 schema-B).
    //! - Channel cache: a configured peer dialed twice produces a
    //!   single cache entry; second call reuses the channel.
    //! - TLS plumbing: a malformed or unreadable CA path surfaces
    //!   as a typed `DialFailed`, never a panic.
    //! - Real cross-hub round-trip with a self-signed CA + tonic
    //!   server is deferred to PR-N1 commit 5/N's e2e suite, which
    //!   spawns 2 daemons; this module's unit tests focus on the
    //!   gate + cache + plumbing surface.

    use super::*;
    use crate::pb::axon::v1::{InvocationState, ResponseHeader};
    use crate::services::realm_trust_anchor::{TrustedAgent, TrustedAgentRole};
    use std::collections::HashMap;
    use std::path::PathBuf;
    use std::sync::Mutex;

    /// Test-only canned-response client. Lookup key is
    /// `(target_hub, request.function_name)` so a single mock
    /// can answer different abilities differently. Calls not in
    /// the canned set return `FederationClientError::DialFailed`
    /// — the same variant the real skeleton would return, so
    /// the dispatcher under test cannot tell apart "no canned
    /// response set" from "real dialer not yet wired".
    pub(super) struct MockFederationClient {
        canned: Mutex<HashMap<(HubUri, String), InvokeResponse>>,
    }

    impl MockFederationClient {
        pub(super) fn new() -> Self {
            Self {
                canned: Mutex::new(HashMap::new()),
            }
        }

        pub(super) fn insert(
            &self,
            target_hub: HubUri,
            function_name: &str,
            response: InvokeResponse,
        ) {
            self.canned
                .lock()
                .expect("mock canned map poisoned")
                .insert((target_hub, function_name.to_string()), response);
        }
    }

    #[async_trait]
    impl FederationClient for MockFederationClient {
        async fn forward_invoke(
            &self,
            target_hub: &HubUri,
            request: InvokeRequest,
        ) -> Result<InvokeResponse, FederationClientError> {
            let key = (target_hub.clone(), request.function_name);
            self.canned
                .lock()
                .expect("mock canned map poisoned")
                .get(&key)
                .cloned()
                .ok_or_else(|| FederationClientError::DialFailed {
                    hub: target_hub.clone(),
                    detail: "MockFederationClient: no canned response".to_string(),
                })
        }
    }

    fn empty_anchor() -> Arc<RealmTrustAnchor> {
        Arc::new(RealmTrustAnchor::default())
    }

    fn sample_request(function_name: &str) -> InvokeRequest {
        InvokeRequest {
            function_name: function_name.to_string(),
            ..InvokeRequest::default()
        }
    }

    fn sample_response() -> InvokeResponse {
        InvokeResponse {
            header: Some(ResponseHeader {
                status: "completed".to_string(),
                ..ResponseHeader::default()
            }),
            state: InvocationState::Completed as i32,
            ..InvokeResponse::default()
        }
    }

    /// Build a fully-populated federation peer entry. Each test
    /// then mutates one field to exercise a specific reject reason.
    fn fed_peer_entry(target_hub: &str, ca_path: PathBuf) -> TrustedAgent {
        TrustedAgent {
            agent_uri: "easynet:///r/peer-realm/agent/peer-hub".to_string(),
            public_key_b64: "AAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAA=".to_string(),
            role: TrustedAgentRole::Hub,
            added_at_unix_ms: 1_714_492_800_000,
            origin_tenant_id: Some("peer-realm".to_string()),
            hub_uri: Some(target_hub.to_string()),
            tls_ca_pem_path: Some(ca_path),
        }
    }

    /// Build a 1-entry trust anchor wrapping the given trusted
    /// agent. Each gate-test uses this to drive `lookup_peer_hub`.
    fn anchor_with(entry: TrustedAgent) -> Arc<RealmTrustAnchor> {
        Arc::new(RealmTrustAnchor::from_entries(vec![entry]).expect("anchor"))
    }

    #[tokio::test]
    async fn peer_not_trusted_when_anchor_empty() {
        // No federation peer in the trust set ⇒ the dialer must
        // reject with PeerNotTrusted, never reach the dial
        // primitive. Empty anchor is the most common operator
        // mis-configuration on a fresh hub-mode daemon.
        let dialer = CrossHubDialer::new(empty_anchor());
        let target = "https://peer-hub.example:50443".to_string();
        let err = dialer
            .forward_invoke(&target, sample_request("test.echo"))
            .await
            .expect_err("empty anchor must reject");
        match err {
            FederationClientError::PeerNotTrusted(hub) => assert_eq!(hub, target),
            other => panic!("expected PeerNotTrusted, got: {other:?}"),
        }
        assert_eq!(dialer.cached_peer_count(), 0);
    }

    #[tokio::test]
    async fn peer_not_trusted_when_role_is_not_hub() {
        // A Backend-role entry whose `hub_uri` matches the target
        // is still rejected. `lookup_peer_hub` filters on role +
        // origin_tenant_id, so a misconfigured TOML that put a
        // backend's URL into hub_uri does not accidentally make
        // it dialable.
        let target = "https://peer-hub.example:50443".to_string();
        let mut entry = fed_peer_entry(&target, PathBuf::from("/dev/null"));
        entry.role = TrustedAgentRole::Backend;
        let anchor = anchor_with(entry);

        let dialer = CrossHubDialer::new(anchor);
        let err = dialer
            .forward_invoke(&target, sample_request("test.echo"))
            .await
            .expect_err("non-hub role must reject");
        assert!(matches!(
            err,
            FederationClientError::PeerNotTrusted(_)
        ));
    }

    #[tokio::test]
    async fn peer_not_trusted_when_origin_tenant_id_missing() {
        // A legacy hub entry written before PR-N1 schema-B has
        // `origin_tenant_id = None`. The dialer must fail closed —
        // a hub entry without a tenant tag is structurally
        // unsuitable for cross-realm admission key resolution
        // (PR-N2's prerequisite).
        let target = "https://peer-hub.example:50443".to_string();
        let mut entry = fed_peer_entry(&target, PathBuf::from("/dev/null"));
        entry.origin_tenant_id = None;
        let anchor = anchor_with(entry);

        let dialer = CrossHubDialer::new(anchor);
        let err = dialer
            .forward_invoke(&target, sample_request("test.echo"))
            .await
            .expect_err("missing origin_tenant_id must reject");
        assert!(matches!(
            err,
            FederationClientError::PeerNotTrusted(_)
        ));
    }

    #[tokio::test]
    async fn peer_not_trusted_when_tls_ca_pem_path_missing() {
        // `tls_ca_pem_path = None` is DEC-N1's "no system-CA
        // fallback" rule: the dialer refuses to touch the network
        // without an operator-pinned CA, even if every other gate
        // field is set.
        let target = "https://peer-hub.example:50443".to_string();
        let mut entry = fed_peer_entry(&target, PathBuf::from("/dev/null"));
        entry.tls_ca_pem_path = None;
        let anchor = anchor_with(entry);

        let dialer = CrossHubDialer::new(anchor);
        let err = dialer
            .forward_invoke(&target, sample_request("test.echo"))
            .await
            .expect_err("missing tls_ca_pem_path must reject");
        assert!(matches!(
            err,
            FederationClientError::PeerNotTrusted(_)
        ));
    }

    #[tokio::test]
    async fn dial_failed_when_tls_ca_pem_path_unreadable() {
        // A trust entry that names a non-existent file must
        // surface a typed `DialFailed`, not panic. Operators see
        // the underlying io::Error verbatim — the message
        // includes the path so a typo is fixable from the log.
        let target = "https://peer-hub.example:50443".to_string();
        let bogus = PathBuf::from("/tmp/easynet-tls-ca-does-not-exist-xyz");
        let entry = fed_peer_entry(&target, bogus.clone());
        let anchor = anchor_with(entry);

        let dialer = CrossHubDialer::new(anchor);
        let err = dialer
            .forward_invoke(&target, sample_request("test.echo"))
            .await
            .expect_err("unreadable CA must surface as DialFailed");
        match err {
            FederationClientError::DialFailed { hub, detail } => {
                assert_eq!(hub, target);
                assert!(
                    detail.contains("read tls_ca_pem_path"),
                    "DialFailed.detail must cite the read step; got: {detail}"
                );
                assert!(
                    detail.contains(&bogus.display().to_string()),
                    "DialFailed.detail must cite the offending path; got: {detail}"
                );
            }
            other => panic!("expected DialFailed, got: {other:?}"),
        }
    }

    #[tokio::test]
    async fn resolve_peer_channel_caches_per_peer() {
        // Two consecutive forward_invoke calls to the same peer
        // must populate exactly one channel-cache entry. The
        // second call's failure path (peer is unreachable) is
        // expected; we assert only that the cache fills once and
        // stays at one entry.
        let target = "https://127.0.0.1:1".to_string(); // black-hole port
        let dir = tempfile::tempdir().expect("tempdir");
        let ca_path = dir.path().join("ca.pem");
        std::fs::write(&ca_path, SELF_SIGNED_CA_PEM).expect("seed ca");

        let entry = fed_peer_entry(&target, ca_path);
        let anchor = anchor_with(entry);

        let dialer = CrossHubDialer::new(anchor);
        // The first call may surface DialFailed (handshake) or
        // InnerInvokeFailed (channel constructed but RPC dispatch
        // fails) depending on tonic's connect_lazy behaviour. We
        // do not assert the variant — we assert the cache
        // populated.
        let _ = dialer
            .forward_invoke(&target, sample_request("test.echo"))
            .await;
        assert_eq!(
            dialer.cached_peer_count(),
            1,
            "first call must populate the channel cache"
        );

        let _ = dialer
            .forward_invoke(&target, sample_request("test.echo"))
            .await;
        assert_eq!(
            dialer.cached_peer_count(),
            1,
            "second call must reuse the cached channel"
        );
    }

    // ── PR-N1 commit 4/N: timeout + circuit-breaker tests ──

    #[tokio::test]
    async fn channel_timeout_fires_when_inner_invoke_exceeds_deadline() {
        // Drive the timeout path with a 50ms budget against a
        // black-hole port. tonic's `connect_lazy` defers the real
        // connect to RPC time, so the inner invoke blocks on
        // TCP connect and the timeout fires.
        let target = "https://127.0.0.1:1".to_string();
        let dir = tempfile::tempdir().expect("tempdir");
        let ca_path = dir.path().join("ca.pem");
        std::fs::write(&ca_path, SELF_SIGNED_CA_PEM).expect("seed ca");

        let entry = fed_peer_entry(&target, ca_path);
        let anchor = anchor_with(entry);

        let dialer = CrossHubDialer::new(anchor)
            .with_forward_invoke_timeout(Duration::from_millis(50));
        let err = dialer
            .forward_invoke(&target, sample_request("test.echo"))
            .await
            .expect_err("50ms budget against black-hole port must time out");
        match err {
            FederationClientError::ChannelTimeout(hub) => assert_eq!(hub, target),
            // tonic 0.12 may surface the failure as `DialFailed`
            // before the inner invoke timeout fires when the OS
            // refuses TCP synchronously (e.g. `ECONNREFUSED` on
            // localhost:1). Either error variant is a legitimate
            // expression of "peer not reachable inside the
            // budget"; the assertion that matters is "no panic +
            // not a `PeerNotTrusted` false-positive".
            FederationClientError::DialFailed { hub, .. }
            | FederationClientError::InnerInvokeFailed { hub, .. } => {
                assert_eq!(hub, target);
            }
            other => panic!(
                "expected ChannelTimeout / DialFailed / InnerInvokeFailed, got: {other:?}"
            ),
        }
    }

    #[tokio::test]
    async fn breaker_opens_after_threshold_consecutive_failures() {
        // Inject a 1-fail threshold + 50ms timeout so a single
        // failed call flips the breaker. The second call must
        // surface `CircuitOpen` without ever touching the network.
        let target = "https://127.0.0.1:1".to_string();
        let dir = tempfile::tempdir().expect("tempdir");
        let ca_path = dir.path().join("ca.pem");
        std::fs::write(&ca_path, SELF_SIGNED_CA_PEM).expect("seed ca");

        let entry = fed_peer_entry(&target, ca_path);
        let anchor = anchor_with(entry);

        let dialer = CrossHubDialer::new(anchor)
            .with_forward_invoke_timeout(Duration::from_millis(50))
            .with_breaker_failure_threshold(1)
            .with_breaker_reset_window(Duration::from_secs(60));

        // First call fails — breaker transitions to Open.
        let _ = dialer
            .forward_invoke(&target, sample_request("test.echo"))
            .await
            .expect_err("first call should fail");
        assert!(
            !dialer.breaker_is_closed(&target),
            "first failure must open the breaker"
        );

        // Second call surfaces CircuitOpen without touching the
        // network. We verify by timing: a CircuitOpen response
        // should return effectively instantly (< 5ms in practice;
        // the hot path is a single DashMap entry read).
        let started = std::time::Instant::now();
        let err = dialer
            .forward_invoke(&target, sample_request("test.echo"))
            .await
            .expect_err("second call should be CircuitOpen");
        let elapsed = started.elapsed();
        match err {
            FederationClientError::CircuitOpen(hub) => assert_eq!(hub, target),
            other => panic!("expected CircuitOpen, got: {other:?}"),
        }
        assert!(
            elapsed < Duration::from_millis(20),
            "CircuitOpen path must fail-fast; took {elapsed:?}",
        );
    }

    #[tokio::test]
    async fn breaker_auto_resets_to_halfopen_after_window() {
        // Breaker opens after 1 failure, reset window is 50ms.
        // After sleeping past the window, the next call moves
        // the breaker to HalfOpen and dispatches a trial. That
        // trial fails (peer still unreachable) → Open again.
        let target = "https://127.0.0.1:1".to_string();
        let dir = tempfile::tempdir().expect("tempdir");
        let ca_path = dir.path().join("ca.pem");
        std::fs::write(&ca_path, SELF_SIGNED_CA_PEM).expect("seed ca");

        let entry = fed_peer_entry(&target, ca_path);
        let anchor = anchor_with(entry);

        let dialer = CrossHubDialer::new(anchor)
            .with_forward_invoke_timeout(Duration::from_millis(50))
            .with_breaker_failure_threshold(1)
            .with_breaker_reset_window(Duration::from_millis(50));

        let _ = dialer
            .forward_invoke(&target, sample_request("test.echo"))
            .await
            .expect_err("first call should fail");
        assert!(!dialer.breaker_is_closed(&target));

        // Wait for the reset window to elapse.
        tokio::time::sleep(Duration::from_millis(75)).await;

        // Next call's outcome is the trial dial: NOT CircuitOpen
        // (the window passed). Either ChannelTimeout / DialFailed /
        // InnerInvokeFailed is acceptable — the assertion is that
        // the breaker did not fail-fast.
        let err = dialer
            .forward_invoke(&target, sample_request("test.echo"))
            .await
            .expect_err("trial dial after reset window should attempt the network");
        match err {
            FederationClientError::CircuitOpen(_) => {
                panic!("breaker should have transitioned to HalfOpen, not stayed Open")
            }
            FederationClientError::ChannelTimeout(_)
            | FederationClientError::DialFailed { .. }
            | FederationClientError::InnerInvokeFailed { .. } => {
                // The trial dial fired and failed — expected.
            }
            other => panic!("unexpected error variant: {other:?}"),
        }
    }

    // ── PR-N1 commit 9/N: SIGHUP-aware live trust anchor ──

    #[tokio::test]
    async fn live_trust_anchor_cell_picks_up_replace_without_dialer_rebuild() {
        // 晓雯 letter 67 attack round 4 catch: commit 6/N's
        // boot-time `Arc<RealmTrustAnchor>` snapshot froze the
        // dialer's federation-peer view at boot, so SIGHUP-driven
        // realm-trust.toml reloads required a daemon restart for
        // the dialer to see them. Commit 9/N's
        // `with_trust_anchor_cell` constructor wires the live
        // `SharedTrustAnchor` cell so a `cell.replace(...)` is
        // visible to the next `forward_invoke` dial.
        //
        // This test drives the assertion at the trust-gate layer
        // (no real network) — the dialer rejects the target with
        // `PeerNotTrusted` initially, then we publish a federation
        // peer entry into the cell, and the next call accepts the
        // peer (proceeds past the trust gate to the channel-build
        // step, where it fails for an unrelated reason since the
        // CA path is a non-existent file). The `PeerNotTrusted`
        // → `DialFailed` transition is the bit that proves the
        // live cell update reached the gate.

        let target = "https://peer-hub.example:50443".to_string();
        let dir = tempfile::tempdir().expect("tempdir");
        let ca_path = dir.path().join("ca.pem");
        std::fs::write(&ca_path, SELF_SIGNED_CA_PEM).expect("seed ca");

        // Cell starts empty (no federation peer entries).
        let cell = SharedTrustAnchor::new(Arc::new(RealmTrustAnchor::default()));
        let dialer = CrossHubDialer::with_trust_anchor_cell(cell.clone())
            .with_forward_invoke_timeout(Duration::from_millis(50));

        // First dial: empty cell → PeerNotTrusted.
        let err = dialer
            .forward_invoke(&target, sample_request("test.echo"))
            .await
            .expect_err("empty cell must reject");
        assert!(
            matches!(err, FederationClientError::PeerNotTrusted(_)),
            "empty cell must surface PeerNotTrusted; got: {err:?}"
        );

        // Operator edits realm-trust.toml + SIGHUP. Production
        // pathway calls `cell.replace(new_anchor)`; we simulate
        // that here with a fresh anchor that includes the peer.
        let new_anchor = anchor_with(fed_peer_entry(&target, ca_path));
        cell.replace(new_anchor);

        // Next dial: live cell snapshot picks up the new entry.
        // The trust gate now passes; the call fails at the
        // network layer (black-hole port) — but **not** with
        // PeerNotTrusted, which is the contract this test is
        // pinning.
        let err2 = dialer
            .forward_invoke(&target, sample_request("test.echo"))
            .await
            .expect_err("network-layer failure expected");
        match err2 {
            FederationClientError::PeerNotTrusted(_) => {
                panic!(
                    "live cell update must reach the trust gate; \
                     PeerNotTrusted means the dialer did NOT see the cell.replace"
                )
            }
            FederationClientError::DialFailed { .. }
            | FederationClientError::ChannelTimeout(_)
            | FederationClientError::InnerInvokeFailed { .. } => {
                // Expected — gate passed, network layer refused.
            }
            FederationClientError::CircuitOpen(_) => {
                panic!("breaker should not be open on the second dial")
            }
        }
    }

    #[tokio::test]
    async fn boot_time_snapshot_does_not_pick_up_anchor_replace() {
        // Reverse-direction pin: the legacy `CrossHubDialer::new`
        // constructor takes an `Arc<RealmTrustAnchor>` snapshot
        // and is intentionally NOT cell-aware. A test fixture
        // that mutates the original anchor (impossible — it's
        // captured by-value) cannot affect the snapshot. This
        // test documents that contract so a future refactor of
        // `CrossHubDialer::new` to accept a live source has to
        // update the test (and the operator-facing constructor
        // doc) deliberately.

        let target = "https://peer-hub.example:50443".to_string();
        let empty_anchor = Arc::new(RealmTrustAnchor::default());
        let dialer = CrossHubDialer::new(empty_anchor)
            .with_forward_invoke_timeout(Duration::from_millis(50));

        let err = dialer
            .forward_invoke(&target, sample_request("test.echo"))
            .await
            .expect_err("snapshot must reject indefinitely");
        assert!(matches!(
            err,
            FederationClientError::PeerNotTrusted(_)
        ));

        // No cell to mutate — the snapshot constructor's signature
        // makes hot-reload impossible by construction. Call again
        // and assert the same outcome to pin the contract.
        let err2 = dialer
            .forward_invoke(&target, sample_request("test.echo"))
            .await
            .expect_err("snapshot must reject indefinitely");
        assert!(matches!(
            err2,
            FederationClientError::PeerNotTrusted(_)
        ));
    }

    #[tokio::test]
    async fn breaker_success_resets_failure_counter() {
        // Drive the counter to threshold-1 = 1 failure (threshold
        // = 2 ⇒ 2 consecutive fails open). Then simulate a
        // success by directly calling `record_breaker_success`.
        // The next failure must NOT open the breaker because the
        // counter was reset.
        let target = "https://peer-hub.example:50443".to_string();
        let dir = tempfile::tempdir().expect("tempdir");
        let ca_path = dir.path().join("ca.pem");
        std::fs::write(&ca_path, SELF_SIGNED_CA_PEM).expect("seed ca");

        let entry = fed_peer_entry(&target, ca_path);
        let anchor = anchor_with(entry);

        let dialer = CrossHubDialer::new(anchor).with_breaker_failure_threshold(2);

        dialer.record_breaker_failure(&target);
        // Counter at 1 — still Closed.
        assert!(dialer.breaker_is_closed(&target));

        dialer.record_breaker_success(&target);
        // Counter back to 0.
        assert!(dialer.breaker_is_closed(&target));

        dialer.record_breaker_failure(&target);
        // Only 1 failure since the success — still Closed (would
        // be Open if the counter hadn't reset).
        assert!(
            dialer.breaker_is_closed(&target),
            "success between failures must reset the counter"
        );
    }

    /// Self-signed CA used by the cache test. Embedded so the
    /// unit test does not require an external `rcgen` dep or a
    /// runtime cert-generation fixture. Generated once with
    /// `openssl req -x509 -newkey rsa:2048 -nodes` and cited as a
    /// build-time fixture, never validated against a real peer
    /// (the test exercises the trust gate + cache mechanics, not
    /// TLS handshake success — full handshake lives in the e2e
    /// suite at commit 5/N where two daemons share a real CA).
    const SELF_SIGNED_CA_PEM: &str = "\
-----BEGIN CERTIFICATE-----
MIIDIzCCAgugAwIBAgIUWSDP0u/rTbKiKyiecmz54C99DsUwDQYJKoZIhvcNAQEL
BQAwIDEeMBwGA1UEAwwVRWFzeU5ldCBQUi1OMSBUZXN0IENBMCAXDTI2MDQzMDE5
MzExOVoYDzIxMjYwNDA2MTkzMTE5WjAgMR4wHAYDVQQDDBVFYXN5TmV0IFBSLU4x
IFRlc3QgQ0EwggEiMA0GCSqGSIb3DQEBAQUAA4IBDwAwggEKAoIBAQDE5dWIWW5P
y5Kxb7x+5fAhJhCHIGnvIXAjFsXNdxzVBVeVZs+cJRewjNqa18FQMzZ+7BO5o8Z7
aFboujGZJF1DcEP1gq69Z8Wn5qp9n2PEBjtcPygg5AeEC4/4m4rhHs9U5nKOYHhY
dkkNop4VKl/WDGwX44a+mNARrjPPxm+BhWA03cgrcGQne0UGXVDI/SXCoYOaPHbS
bNY9FuhgEtaUPkiAo0U+xHkY0ITJorKGssAApn/k5XExS8SQNrvwZgQjfqLYPkTP
LjwfpJqS/jbPj3cYg7y0IJvTmuskP7JpyMTIM8tzJ4dT1/u1N4fNgCXtwj6r639D
rWD/xMmyz+xVAgMBAAGjUzBRMB0GA1UdDgQWBBTxh0O/FuznnDBiTSHTu3ue+ba1
xzAfBgNVHSMEGDAWgBTxh0O/FuznnDBiTSHTu3ue+ba1xzAPBgNVHRMBAf8EBTAD
AQH/MA0GCSqGSIb3DQEBCwUAA4IBAQCKdj6fwsRArmqVE5WVqqqyQt9Lq2gBLGdI
4jhBRq0l6dwpcTb76B2QncTd6LGsfiWgIOUI1gC0yZpJnFewfBvrflNF3tpwgCUA
n5pEQsCZWFEM6+adkK80AX/TusX+31vb1s6ue5Mkh305YT8orguTFsajF1HpT/12
SxYwtVK19IHR+6r7EBBCBg5D0fpPsH/xFsEWhdKVscezZ/W6m2iSQASUsCqSuQ22
6i81muHeKZjGAV1Tv0GJ7dXH1hVGF3mnQYSgTPMI3A5LWmjIiJY7jDKH2iwDeF6Z
30/lrNkD1+uxFboKf5XC1ySO8OysZ8qee2aV0LiP0hUYPiVMoRxl
-----END CERTIFICATE-----
";

    #[test]
    fn dialer_starts_with_zero_cached_peer_channels() {
        let dialer = CrossHubDialer::new(empty_anchor());
        assert_eq!(dialer.cached_peer_count(), 0);
    }

    #[test]
    fn dialer_clone_shares_channel_cache() {
        // PR-N1 spec INV-5: the channel cache is process-wide,
        // not per-clone. Two clones must observe the same
        // backing DashMap so the eventual commit 2/N TLS-pinned
        // channel inserted on one clone is visible to admission
        // RPCs holding a different clone.
        let dialer_a = CrossHubDialer::new(empty_anchor());
        let dialer_b = dialer_a.clone();
        // We can't insert a real `Channel` here without a tonic
        // endpoint, but we can check the Arc identity by
        // comparing pointer equality on the underlying DashMap.
        assert!(
            Arc::ptr_eq(&dialer_a.channels, &dialer_b.channels),
            "clones must share the channel cache by Arc identity"
        );
    }

    #[tokio::test]
    async fn mock_client_returns_canned_response_when_present() {
        let mock = MockFederationClient::new();
        let target = "https://peer-hub.example:50443".to_string();
        mock.insert(target.clone(), "test.echo", sample_response());

        let resp = mock
            .forward_invoke(&target, sample_request("test.echo"))
            .await
            .expect("canned response delivered");
        assert_eq!(resp.state, InvocationState::Completed as i32);
        assert_eq!(
            resp.header.as_ref().expect("header present").status,
            "completed"
        );
    }

    #[tokio::test]
    async fn mock_client_dial_failed_when_no_canned_response() {
        let mock = MockFederationClient::new();
        let target = "https://peer-hub.example:50443".to_string();
        let err = mock
            .forward_invoke(&target, sample_request("never.canned"))
            .await
            .expect_err("missing canned response must surface as DialFailed");
        match err {
            FederationClientError::DialFailed { hub, .. } => assert_eq!(hub, target),
            other => panic!("expected DialFailed, got: {other:?}"),
        }
    }

    #[tokio::test]
    async fn mock_client_canned_lookup_keyed_by_function_name() {
        // The canned map keys on `(hub, function_name)` so one
        // mock can answer multiple abilities differently. Pin
        // the contract so commit 3/N tests against this mock
        // can rely on the keying.
        let mock = MockFederationClient::new();
        let target = "https://peer-hub.example:50443".to_string();

        let mut completed_resp = sample_response();
        completed_resp.result = b"echo-payload".to_vec();
        let mut other_resp = sample_response();
        other_resp.result = b"other-payload".to_vec();

        mock.insert(target.clone(), "test.echo", completed_resp.clone());
        mock.insert(target.clone(), "test.other", other_resp.clone());

        let r1 = mock
            .forward_invoke(&target, sample_request("test.echo"))
            .await
            .expect("echo canned");
        let r2 = mock
            .forward_invoke(&target, sample_request("test.other"))
            .await
            .expect("other canned");
        assert_eq!(r1.result, b"echo-payload");
        assert_eq!(r2.result, b"other-payload");
    }
}
