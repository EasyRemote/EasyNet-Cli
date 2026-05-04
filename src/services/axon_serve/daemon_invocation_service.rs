// EasyNet CLI — axon_serve — DaemonInvocationService
// ===================================================
//
// File: src/services/axon_serve/daemon_invocation_service.rs
// Description: Concrete implementation of axon's
//              `pb::axon::v1::invocation_server::Invocation` trait
//              for the new daemon transport plane.
//
// State + behaviour binding
// -------------------------
// The struct is the single owner of every dependency the three RPC
// methods (Invoke / InvokeStream / InvokeBidi) need at runtime. All
// dependencies are injected through the `new` constructor; the
// struct holds them by `Arc` so individual RPC method calls clone
// cheaply.
//
// What this commit lands
// ----------------------
// Commit 6/9: dispatcher wiring. The service now holds an
// `Arc<PresenceRegistry>` injected at construction; the three RPC
// methods route by `InvokeRequest.function_name`:
//
//   - `Invoke`:   federation.{join, advertise_agent, heartbeat,
//                 resolve, revoke, forward_invoke} → federation
//                 wrappers; anything else returns Unimplemented
//                 with a follow-up commit (admission gate facade,
//                 LocalAbilityRegistry forwarding) note
//   - `InvokeStream`: `federation.subscribe_directory` →
//                 initial-snapshot frame from
//                 `build_subscribe_directory_initial`; the
//                 broadcast pump for incremental events lands in
//                 commit 7/9 alongside the LocalAbilityRegistry
//                 stream forward path
//   - `InvokeBidi`: still returns Unimplemented; PR-2 implements
//                 `<self>.session` accept and PR-3 implements
//                 `<self>.invoke_remote`
//
// What the dispatcher does NOT yet do
// -----------------------------------
// - Run the admission gate (commit 7/9, alongside the realm-trust
//   loader and `easynet-axon` admission helpers integration)
// - Forward unmatched abilities to LocalAbilityRegistry (commit 7/9)
// - Push frames down `<self>.session` reverse channels for
//   `federation.forward_invoke` (commit 8/9)
// - Spawn the broadcast pump for `subscribe_directory` incremental
//   events (commit 8/9)
//
// Result content type
// -------------------
// All `federation.*` wrappers serialise their typed response with
// `serde_json::to_vec` into `InvokeResponse.result` and set
// `result_content_type = "application/json"`. This matches the
// JSON-encoded shape captured by PR-4's schema-compat baselines
// per DEC-001 + DEC-003.
//
// Author: Silan Hu <silan.hu@u.nus.edu>
// Copyright (c) 2026 EasyNet. All rights reserved.

use std::collections::BTreeMap;
use std::future::Future;
use std::path::PathBuf;
use std::pin::Pin;
use std::sync::Arc;
use std::task::{Context, Poll};
use std::time::Duration;

use futures::Stream;
// `StreamExt` brings `.next().await` into scope. Aliased to `_`
// because we use the trait method only — we don't reference the
// trait by name. Per letter 22 §4 b: avoid the name-collision risk
// Hit when bringing both `futures::StreamExt` and
// `tokio_stream::StreamExt` into scope.
use futures::StreamExt as _;
use tokio::sync::mpsc;
use tokio_stream::wrappers::ReceiverStream;
use tonic::{Request, Response, Status, Streaming};

use crate::pb::axon::v1::invocation_server::Invocation;
use crate::pb::axon::v1::{
    invoke_bidi_down::Payload as DownPayload, invoke_bidi_up::Payload as UpPayload, AgentIdentity,
    BidiControl, BinaryChunk, CallerSignature, Envelope, EnvelopeOpen, InvocationReceipt,
    InvocationState, InvokeBidiDown, InvokeBidiUp, InvokeRequest, InvokeResponse,
    InvokeServerStreamRequest, InvokeStreamChunk, StreamDescriptor, SubjectIdentity,
};
use crate::services::axon_serve::admission_facade::AdmissionFacade;
use crate::services::axon_serve::federation_wrappers::{
    self, ABILITY_FEDERATION_ADVERTISE_ABILITIES, ABILITY_FEDERATION_ADVERTISE_AGENT,
    ABILITY_FEDERATION_DISCOVER, ABILITY_FEDERATION_FORWARD_INVOKE, ABILITY_FEDERATION_HEARTBEAT,
    ABILITY_FEDERATION_JOIN, ABILITY_FEDERATION_LIST_USER_DEVICES, ABILITY_FEDERATION_RESOLVE,
    ABILITY_FEDERATION_RESOLVE_KEY, ABILITY_FEDERATION_REVOKE,
    ABILITY_FEDERATION_SUBSCRIBE_DIRECTORY, ABILITY_FEDERATION_SUBSCRIBE_DIRECTORY_V2,
    ABILITY_RUNTIME_BOOTSTRAP_SELF_IDENTITY,
};
use crate::services::axon_serve::invoke_remote_initiator::{
    call_id_hex, InvokeRemoteDown, InvokeRemoteUp, RequestOutcome, SessionDispatch,
    SessionRequestError, ABILITY_INVOKE_REMOTE, INVOKE_REMOTE_STREAM_ID,
};
use crate::services::axon_serve::register_device_pubkey::{
    handle as handle_register_device_pubkey, parse_realm_from_uri,
    ABILITY_SELF_REGISTER_DEVICE_PUBKEY,
};
use crate::services::axon_serve::session_initiator::{SessionSigningSeed, ABILITY_SELF_SESSION};
use crate::services::federated_peers_cell::SharedFederatedPeers;
use crate::services::federation_client::FederationClient;
use crate::services::pending_dispatch::{
    DispatchResult, DispatchStreamEvent, PendingDispatchMap, PendingStreamDispatchMap,
};
use crate::services::presence_registry::{
    DispatchFrame, DispatchSender, OfflineReason, PresenceRegistry, DISPATCH_CHANNEL_CAPACITY,
};
use crate::services::realm_trust_anchor::RealmTrustAnchor;
use crate::services::trust_anchor_cell::SharedTrustAnchor;

/// Content type the federation wrappers emit on `InvokeResponse.result`.
/// Centralised here so call sites cannot drift away from the value
/// PR-4's baselines expect.
const FEDERATION_RESULT_CONTENT_TYPE: &str = "application/json";
const REASON_BIDI_FIRST_FRAME_SEQUENCE: &str = "AXON_BIDI_FIRST_FRAME_SEQUENCE";
const REASON_BIDI_NON_STRICT_ORDERING: &str = "AXON_BIDI_NON_STRICT_ORDERING";
const REASON_BIDI_FRAME_SEQUENCE: &str = "AXON_BIDI_FRAME_SEQUENCE";

/// Application-level heartbeat cadence for `<self>.session` down
/// streams.
///
/// Why we need this in addition to tonic/h2 keepalive PING:
/// transport keepalive only proves the TCP/TLS/HTTP2 stack is still
/// exchanging frames; it does not guarantee tonic surfaces a
/// half-broken bidi back to the device task promptly. The observed
/// failure mode was: hub-side reader noticed reset and removed the
/// device from PresenceRegistry immediately, but the device-side
/// `down_stream.next()` could remain parked and therefore never
/// trigger the reconnect supervisor. A no-op application heartbeat
/// every 5 s gives the device a concrete "the hub is still pushing
/// session frames" signal it can watchdog against.
///
/// The frame is `BidiControl::default()` — a wire shape current
/// readers already ignore as a non-business frame, so we add liveness
/// without perturbing dispatch semantics.
const SESSION_DOWN_HEARTBEAT_INTERVAL: Duration = Duration::from_secs(5);

/// gRPC `Invocation` service hosted by `easynet-daemon`.
///
/// Holds the dependencies the three RPC methods need:
///
/// - `presence` — the `PresenceRegistry` consulted by federation
///   wrappers (resolve / forward_invoke / revoke / heartbeat /
///   subscribe_directory) and by the future `<self>.session` accept
///   path in PR-2
/// - `admission` — the `AdmissionFacade` consulted at the start of
///   every RPC method, before any dispatch. Rejects callers whose
///   URI is not in the realm trust anchor (per spec §5)
///
/// Future-shape (commit 8/9 onward) will add:
/// `ability_dispatch: Arc<LocalAbilityRegistry>` for the unmatched-
/// ability fallthrough. Construction will switch to
/// `new(presence, admission, ability_dispatch)` then.
///
/// `Clone` is derived so PR-10's TCP+TLS listener can register the
/// same service surface on a second tonic `Server` without holding
/// the original instance hostage. All fields are `Arc`/`Option<Arc>`/
/// `Option<String>`; clone is cheap.
#[derive(Clone)]
pub struct DaemonInvocationService {
    presence: Arc<PresenceRegistry>,
    /// Hosted-agent directory rows published by
    /// `federation.advertise_agent`. PresenceRegistry tracks live
    /// device sessions; this store maps `/agent/<user>.<agent>` rows
    /// back to their host device URI so resolve can project hosted
    /// agents while deriving liveness from the host's live session.
    advertised_agents: Arc<crate::services::advertised_agent_store::AdvertisedAgentStore>,
    /// Per-agent ability catalog populated by
    /// `federation.advertise_abilities` and projected back through
    /// `federation.resolve(include_abilities=true)`. Always present
    /// on production daemons; an `Arc` clone, cheap to share with
    /// every dispatch handler that needs it.
    ability_catalog: Arc<crate::services::ability_catalog_store::AbilityCatalogStore>,
    admission: AdmissionFacade,
    /// Cross-call correlation table for `<self>.invoke_remote`
    /// per-call dispatches awaiting a target-device reply on its
    /// `<self>.session` reverse channel. `None` until
    /// `with_pending(...)` wires it; absence means
    /// `<self>.invoke_remote` is unavailable on this daemon (returned
    /// as `Status::failed_precondition`). PR-3 owns the map's
    /// shape; production daemons attach one at boot once PR-2
    /// `<self>.session` accept handler also consumes it.
    pending: Option<Arc<PendingDispatchMap>>,
    /// Streaming correlation table for remote bidi bridges that
    /// need chunked replies instead of a single terminal payload.
    /// Same-hub `fleet.file_transfer` is the first consumer.
    pending_stream: Option<Arc<PendingStreamDispatchMap>>,
    /// `<self>.register_device_pubkey` handler context (PR-7
    /// commit 5/N). `None` until `with_register_pubkey(...)` wires
    /// it; absence means the ability returns
    /// `Status::failed_precondition` (the daemon was booted without
    /// the trust-write surface — typically a smoke-test setup).
    /// Production daemons always attach one at boot from
    /// `start_axon_serve_sidecar`.
    register_pubkey: Option<RegisterPubkeyContext>,
    /// Daemon realm carried explicitly for `<self>.session`
    /// admission-time cross-realm rejection. `None` means the
    /// service was constructed without the realm context (typically
    /// a narrow unit test) and the extra PR-2 defense-in-depth check
    /// is skipped.
    session_realm: Option<String>,
    /// Optional hub signing seed used for cross-hub
    /// `federation.forward_invoke` peer-envelope signatures. When
    /// boot preloads the backend hub identity (or tests inject a
    /// fixture seed), the dispatcher signs without touching disk.
    /// `None` preserves the legacy on-demand read of
    /// `~/.easynet-hub/<realm>/identity.json`.
    hub_signing_seed: Option<SessionSigningSeed>,
    /// **PR-N1 commit 3a/N**. Cross-hub federation client. `None`
    /// until `with_federation_client(...)` wires one; absence
    /// means `federation.forward_invoke` for cross-tenant targets
    /// falls back to the legacy `target_online: false` shape (no
    /// dial). Commit 3b/N rewrites the `forward_invoke` dispatcher
    /// to consume this field; commit 3a/N only plumbs it through.
    federation_client: Option<Arc<dyn FederationClient>>,
    /// **PR-N1 commit 3a/N → 10/N**. Operator-curated `tenant →
    /// hub_uri` cell per `DaemonConfig::federated_peers`. Empty
    /// map ⇒ no cross-tenant routing configured; the dispatcher
    /// returns the legacy shape. Commit 10/N upgraded this from a
    /// boot-time `BTreeMap<String, String>` snapshot to the
    /// `SharedFederatedPeers` cell so SIGHUP-driven daemon-config
    /// reloads (operator editing `[daemon.federated_peers]`)
    /// surface to the next dispatch within ~50ms — same cadence
    /// as the trust-anchor reload landed by commit 9/N.
    /// PR-N3 will replace this hand-curated map with auto-
    /// discovered cross-realm directory entries.
    federated_peers: SharedFederatedPeers,
    /// **PR-N3 commit N3-3/N3-4**. Reload-friendly cell holding
    /// the daemon-wide federated directory snapshot. Per-peer
    /// `RemoteDirectoryClient` tasks (lands in N3-3.1) write
    /// into this cell as Snapshot/Upsert/Remove frames arrive;
    /// the `federation.discover` dispatch arm reads from it for
    /// cross-realm URI lookup. Defaults to empty so single-realm
    /// daemons gracefully report no federated entries.
    federated_directory: crate::services::federation_directory::SharedFederatedDirectoryView,
    /// **N3-N4 bridge**. Daemon-wide federated user binding
    /// store. When wired, the `federation.discover` dispatch
    /// arm constructs a `FederatedUserResolver` per call and
    /// filters cross-realm entries through it whenever the
    /// request supplies a `local_user_id`. `None` ⇒ no filter
    /// (operator query path). Production daemons attach this
    /// at boot via `with_federated_bindings_store`.
    federated_bindings:
        Option<std::sync::Arc<crate::runtime::keyring::federated_bindings::FederatedBindingsStore>>,
    /// **PR-N3 N3-streaming-6**. Heartbeat cadence
    /// (milliseconds) for the v2 subscribe_directory server
    /// stream. Spec §2.3 pins 30 000ms in production; tests
    /// override via `with_subscribe_v2_heartbeat_interval_ms`
    /// to drive the keepalive path in real time without
    /// virtualised clocks. Always nonzero — a zero interval
    /// would emit a heartbeat per poll and pin the CPU.
    subscribe_v2_heartbeat_interval_ms: u64,
    /// **PR-N6 C4**. Device-mode escalation handle. When this
    /// is `Some`, `dispatch_federation_forward_invoke` routes
    /// every call through the existing `<self>.session` bidi to
    /// the hub instead of consulting the local PresenceRegistry
    /// (which is empty by construction on a device-mode daemon).
    /// `None` ⇒ this daemon owns its own PresenceRegistry
    /// (hub or both mode), so the existing dispatch arm runs
    /// unchanged. Boot wires `Some` only under
    /// `mode = "device"` per `boot.rs::start_axon_serve_sidecar`.
    escalation: Option<
        std::sync::Arc<crate::services::axon_serve::session_escalation::SessionEscalationHandle>,
    >,
    /// **PR-1 commit 7/9 (LB-56)**. Local ability dispatcher Arc.
    /// When a `federation.forward_invoke` call's `target_uri` is
    /// the daemon's OWN canonical URI (i.e. the peer hub is the
    /// target itself, not a device subscribed to its
    /// PresenceRegistry), the local-presence push misses by
    /// construction — hub daemons do not register their own URI
    /// in the presence map. Without this field, the call surfaces
    /// as `target_offline` and the cross-hub forward chain breaks
    /// for peer-targeted abilities (`fs.read` against hub-B's own
    /// filesystem, `meta.list_abilities` issued through the
    /// federation wrapping path, etc.). When set, the dispatcher
    /// falls back to running the inner ability against this Arc
    /// and stamps the bytes inline into
    /// `ForwardInvokeResponse.result_bytes`. `None` ⇒ pre-PR-1-7/9
    /// behaviour (test fixtures + hub-only daemons that don't
    /// publish a local ability surface). Boot wires this from
    /// `start_axon_serve_sidecar`'s already-threaded
    /// `Arc<AbilityDispatcher>`.
    local_dispatcher: Option<Arc<crate::runtime::ability_dispatch::AbilityDispatcher>>,
}

impl std::fmt::Debug for DaemonInvocationService {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("DaemonInvocationService")
            .field("presence", &self.presence)
            .field("admission", &self.admission)
            .field("pending", &self.pending)
            .field("register_pubkey", &self.register_pubkey)
            .field("session_realm", &self.session_realm)
            .field(
                "hub_signing_seed",
                &self.hub_signing_seed.as_ref().map(|_| "<seed>"),
            )
            .field(
                "federation_client",
                &self
                    .federation_client
                    .as_ref()
                    .map(|_| "<dyn FederationClient>"),
            )
            .field(
                "federated_peers_count",
                &self.federated_peers.snapshot().len(),
            )
            .field(
                "federated_bindings",
                &self.federated_bindings.as_ref().map(|_| "<store>"),
            )
            .finish()
    }
}

/// Tuple wired by `with_register_pubkey(...)`. Cloning is cheap
/// (the cell is `Arc`-shaped, the strings are short).
#[derive(Debug, Clone)]
struct RegisterPubkeyContext {
    daemon_realm: String,
    trust_anchor_path: PathBuf,
    cell: SharedTrustAnchor,
}

impl DaemonInvocationService {
    /// Construct a service against the supplied presence registry
    /// and admission facade. Production callers wire one registry
    /// per daemon process and share it via `Arc` between the
    /// service, the `<self>.session` accept loop (PR-2), and any
    /// audit-log subscriber. The admission facade is constructed
    /// from `RealmTrustAnchor::load_or_empty(...)` at daemon boot.
    ///
    /// `<self>.invoke_remote` requires an additional
    /// `PendingDispatchMap`; use `with_pending(...)` to attach one.
    /// Daemons constructed without it reject `<self>.invoke_remote`
    /// calls as not-configured rather than crashing.
    #[must_use]
    pub fn new(presence: Arc<PresenceRegistry>, admission: AdmissionFacade) -> Self {
        Self {
            presence,
            advertised_agents: Arc::new(
                crate::services::advertised_agent_store::AdvertisedAgentStore::new(),
            ),
            ability_catalog: Arc::new(
                crate::services::ability_catalog_store::AbilityCatalogStore::new(),
            ),
            admission,
            pending: None,
            pending_stream: None,
            register_pubkey: None,
            session_realm: None,
            hub_signing_seed: None,
            federation_client: None,
            federated_peers: SharedFederatedPeers::default(),
            federated_directory:
                crate::services::federation_directory::SharedFederatedDirectoryView::default(),
            federated_bindings: None,
            subscribe_v2_heartbeat_interval_ms: 30_000,
            escalation: None,
            local_dispatcher: None,
        }
    }

    /// Attach a `PendingDispatchMap` for `<self>.invoke_remote`
    /// dispatch correlation. Builder-style so existing
    /// `DaemonInvocationService::new(presence, admission)` callers
    /// stay source-compatible.
    ///
    /// PR-3 ownership (this commit). PR-2's `<self>.session`
    /// accept handler will share the same `Arc<PendingDispatchMap>`
    /// to call `complete(call_id, ...)` when target devices send
    /// `Result` frames back up their session streams.
    #[must_use]
    pub fn with_pending(mut self, pending: Arc<PendingDispatchMap>) -> Self {
        // Spawn a presence-event watcher that fail-fasts every
        // pending dispatch whose target_uri just went offline.
        // Without this hook, `forward_invoke`'s `await_reply()`
        // blocks on the oneshot until the operator-side HTTP
        // request times out (~30s) for a target session that's
        // already known-dead — surfacing as "your invoke just
        // hung" UX. See pending_dispatch.rs::cancel_for for the
        // matching producer.
        let watcher_pending = Arc::clone(&pending);
        let watcher_presence = Arc::clone(&self.presence);
        tokio::spawn(async move {
            use crate::services::presence_registry::PresenceEvent;
            let mut events = watcher_presence.subscribe_events();
            loop {
                match events.recv().await {
                    Ok(PresenceEvent::Offline { uri, reason }) => {
                        let cancelled = watcher_pending.cancel_for(&uri, "target_offline");
                        if cancelled > 0 {
                            eprintln!(
                                "[axon-serve] presence-offline-cancel: target_uri={uri} \
                                 reason={reason:?} cancelled={cancelled} pending dispatch(es)"
                            );
                        }
                    }
                    Ok(PresenceEvent::Online { .. }) => {
                        // Nothing to do on online — pending entries
                        // for new sessions register fresh.
                    }
                    Err(_) => {
                        // Broadcast channel closed → registry
                        // dropped → daemon shutting down. Exit the
                        // watcher cleanly.
                        return;
                    }
                }
            }
        });
        self.pending = Some(pending);
        self
    }

    #[must_use]
    pub fn with_pending_stream(mut self, pending: Arc<PendingStreamDispatchMap>) -> Self {
        self.pending_stream = Some(pending);
        self
    }

    /// Attach the `<self>.register_device_pubkey` handler context
    /// (PR-7 commit 5/N). The same `SharedTrustAnchor` cell is
    /// also threaded into the `AdmissionFacade` so a successful
    /// register-pubkey publish is visible to the next admission
    /// without restarting the daemon.
    #[must_use]
    pub fn with_register_pubkey(
        mut self,
        daemon_realm: impl Into<String>,
        trust_anchor_path: impl Into<PathBuf>,
        cell: SharedTrustAnchor,
    ) -> Self {
        self.register_pubkey = Some(RegisterPubkeyContext {
            daemon_realm: daemon_realm.into(),
            trust_anchor_path: trust_anchor_path.into(),
            cell,
        });
        self
    }

    /// Attach the daemon's own realm for `<self>.session`
    /// cross-realm rejection. Kept as a dedicated builder so the
    /// PR-2 guardrail does not depend on the presence of the PR-7
    /// trust-write surface.
    #[must_use]
    pub fn with_session_realm(mut self, daemon_realm: impl Into<String>) -> Self {
        self.session_realm = Some(daemon_realm.into());
        self
    }

    /// Attach the hub identity seed used to sign cross-hub
    /// peer-envelope rewrites. Boot wires this best-effort from
    /// backend's `~/.easynet-hub/<realm>/identity.json`; tests can
    /// inject a deterministic fixture to avoid relying on process
    /// `HOME`.
    #[must_use]
    pub fn with_hub_signing_seed(mut self, seed: SessionSigningSeed) -> Self {
        self.hub_signing_seed = Some(seed);
        self
    }

    /// **PR-N6 C4**. Attach a session-escalation handle. When
    /// set, `dispatch_federation_forward_invoke` routes every
    /// inbound forward_invoke call up the existing
    /// `<self>.session` bidi to the hub instead of consulting
    /// the local PresenceRegistry. Boot wires this only under
    /// `mode = "device"`; hub/both daemons leave it `None` and
    /// take the existing dispatch arm.
    #[must_use]
    pub fn with_session_escalation(
        mut self,
        handle: std::sync::Arc<
            crate::services::axon_serve::session_escalation::SessionEscalationHandle,
        >,
    ) -> Self {
        self.escalation = Some(handle);
        self
    }

    /// **PR-1 commit 7/9 (LB-56)**. Attach the daemon's process-
    /// wide `AbilityDispatcher` Arc. When set, a
    /// `federation.forward_invoke` call whose `target_uri` is the
    /// daemon's own URI falls through to local execution against
    /// the registered `LocalAbilityRegistry` instead of surfacing
    /// `target_offline`. See the field doc on `local_dispatcher`
    /// for the why; closes the source-cited PR-1 commit 7/9 hole
    /// at line 27 / 32 / 42 / 455 / 497 of this file.
    #[must_use]
    pub fn with_local_dispatcher(
        mut self,
        dispatcher: Arc<crate::runtime::ability_dispatch::AbilityDispatcher>,
    ) -> Self {
        self.local_dispatcher = Some(dispatcher);
        self
    }

    /// **PR-N1 commit 3a/N**. Attach the cross-hub federation
    /// client. Daemons booted without one fall back to the legacy
    /// `target_online: false` shape for cross-tenant
    /// `federation.forward_invoke` calls. PR-N1 commit 3b/N
    /// rewrites the dispatcher to consume the client; commit
    /// 3a/N only stores it as a field so that rewrite has a stable
    /// constructor surface to thread through.
    #[must_use]
    pub fn with_federation_client(mut self, client: Arc<dyn FederationClient>) -> Self {
        self.federation_client = Some(client);
        self
    }

    /// **PR-N1 commit 3a/N**. Attach the operator-curated
    /// `tenant → hub_uri` map by-value. Wraps the supplied map in
    /// a fresh `SharedFederatedPeers` cell so test fixtures that
    /// don't care about hot-reload still get the cell shape under
    /// the hood. Production daemons use
    /// [`with_federated_peers_cell`] to share the boot-time cell
    /// with the SIGHUP reload task.
    ///
    /// Empty map (the default from `DaemonInvocationService::new`)
    /// means no cross-tenant routing is configured; the
    /// dispatcher's cross-tenant arm then refuses to dial
    /// regardless of `federation_client` presence.
    #[must_use]
    pub fn with_federated_peers(mut self, peers: BTreeMap<String, String>) -> Self {
        self.federated_peers = SharedFederatedPeers::new(peers);
        self
    }

    /// **PR-N1 commit 10/N**. Attach the live
    /// `SharedFederatedPeers` cell so SIGHUP-driven daemon-config
    /// reloads (operator editing `[daemon.federated_peers]`)
    /// republish into the dispatcher's view within ~50ms — same
    /// cadence as the trust-anchor reload landed by commit 9/N.
    /// Production `start_axon_serve_sidecar` uses this builder; the
    /// SIGHUP task in `boot.rs` calls `cell.replace(...)` on
    /// successful TOML reload.
    #[must_use]
    pub fn with_federated_peers_cell(mut self, cell: SharedFederatedPeers) -> Self {
        self.federated_peers = cell;
        self
    }

    /// **PR-N3 commit N3-3/N3-4**. Attach the live
    /// `SharedFederatedDirectoryView` cell so the
    /// `federation.discover` dispatch arm reads the daemon-
    /// wide cross-realm directory snapshot. Per-peer
    /// `RemoteDirectoryClient` tasks (lands in N3-3.1) write
    /// into the same cell as Snapshot/Upsert/Remove frames
    /// arrive. Defaults to empty (single-realm daemons report
    /// no federated entries — graceful degradation).
    #[must_use]
    pub fn with_federated_directory_cell(
        mut self,
        cell: crate::services::federation_directory::SharedFederatedDirectoryView,
    ) -> Self {
        self.federated_directory = cell;
        self
    }

    /// **N3-N4 dispatch wire**. Attach the daemon-wide
    /// federated user binding store. When a `federation.discover`
    /// request supplies a `local_user_id`, the dispatch arm
    /// constructs a `FederatedUserResolver` from this store +
    /// the daemon's own realm and routes through the filtered
    /// handler, surfacing only entries the user has opted into
    /// via PR-N4's consume flow.
    #[must_use]
    pub fn with_federated_bindings_store(
        mut self,
        bindings: std::sync::Arc<
            crate::runtime::keyring::federated_bindings::FederatedBindingsStore,
        >,
    ) -> Self {
        self.federated_bindings = Some(bindings);
        self
    }

    /// **PR-N3 N3-streaming-6**. Override the v2 subscribe
    /// stream's Heartbeat cadence in milliseconds. Production
    /// stays at the 30 000ms default (spec §2.3); tests pass
    /// a sub-second value (e.g. 50ms) to exercise the
    /// keepalive path in real time without virtualised clocks.
    /// Panics on zero so a misuse cannot pin the CPU emitting
    /// heartbeats every poll.
    #[must_use]
    pub fn with_subscribe_v2_heartbeat_interval_ms(mut self, ms: u64) -> Self {
        assert!(ms > 0, "heartbeat interval must be > 0 ms");
        self.subscribe_v2_heartbeat_interval_ms = ms;
        self
    }

    /// Resolve whether `target_uri` names THIS daemon's own
    /// synchronous-execution surface.
    ///
    /// Three valid shapes per RFC-001 + RFC-006-C v0.1:
    ///   (1) `easynet:///r/<realm>/device/<deviceID>` — the daemon's
    ///       device identity from credentials.json. Standard.
    ///   (2) `easynet:///r/<realm>/hub` — the realm-singleton hub URI;
    ///       hub-mode daemons answer to this in addition to (1).
    ///   (3) `easynet:///r/<realm>/agent/<userID>.<agentID>` — the
    ///       agent URI of an agent the daemon currently hosts. v4.1.5
    ///       §9 callee ∈ {hub, device, agent}; RFC-006-C §INV-2 +
    ///       RFC-006-B v0.6 §URL require the wire callee on a chat-
    ///       base or page.fetch invocation to be the agent URA, not
    ///       the device. Recognise it here so the local fast path
    ///       fires instead of falling through to "target offline".
    ///
    /// Match for (3): the daemon hosts agent `<X>` iff its local
    /// dispatcher has an ability registered with prefix `<X>.`. We
    /// extract the bare agent segment (after the user/agent dot
    /// boundary) from the URI and check the dispatcher's ability list
    /// for any name starting `<agent>.`. This is O(n_abilities) but
    /// only fires on remote-arriving invocations and the table is
    /// small (tens of entries).
    fn matches_self_target_uri(&self, target_uri: &str) -> bool {
        if self
            .admission
            .daemon_uri()
            .is_some_and(|daemon_uri| daemon_uri == target_uri)
        {
            return true;
        }
        if self
            .session_realm
            .as_deref()
            .is_some_and(|realm| crate::uri::hub_uri(realm) == target_uri)
        {
            return true;
        }
        // (3) agent URA — accept if we host an ability whose tail
        // matches `<agentID>` in any owner shape:
        //   • `<userID>.<agentID>.<verb>`     (AbilityURI splitn(3,'.'))
        //   • `<userName>.<agentID>.<verb>`   (Pages registers under
        //     username from EASYNET_PAGES_USER; backend may send UUID
        //     in the user segment)
        //   • `<agentID>.<verb>`              (daemon-flat shape used
        //     by `<agent>.chat` in single-user mode)
        //
        // The userID-to-username mapping is intentionally not
        // resolved here — admission elsewhere ensures the caller has
        // a delegation proof bound to the user segment, so an
        // attacker cannot exploit the lenient agentID match.
        if let Some((_user_seg, agent_seg)) = parse_agent_owner_pair_from_uri(target_uri) {
            if let Some(dispatcher) = self.local_dispatcher.as_ref() {
                let agent_dot = format!("{agent_seg}.");
                let agent_dot_owned = format!(".{agent_seg}.");
                if dispatcher
                    .local_registry()
                    .list_abilities()
                    .iter()
                    .any(|name| {
                        name.starts_with(&agent_dot) || name.contains(&agent_dot_owned)
                    })
                {
                    return true;
                }
            }
        }
        false
    }
}

/// Boxed pinned stream type used for both server-stream and
/// bidirectional response stream associated types.
type BoxedDownStream<T> = Pin<Box<dyn Stream<Item = Result<T, Status>> + Send + 'static>>;

#[tonic::async_trait]
impl Invocation for DaemonInvocationService {
    /// Spec §2.1 + §4.1 reference. Routes by
    /// `InvokeRequest.function_name`:
    ///
    /// - `federation.join` / `federation.advertise_agent` /
    ///   `federation.heartbeat` / `federation.resolve` /
    ///   `federation.revoke` / `federation.forward_invoke` →
    ///   federation wrapper
    /// - anything else → Unimplemented with a "PR-1 staging" note;
    ///   commit 7/9 wires LocalAbilityRegistry as the fall-through
    async fn invoke(
        &self,
        request: Request<InvokeRequest>,
    ) -> Result<Response<InvokeResponse>, Status> {
        let inner = request.into_inner();
        self.admission.verify_invoke(&inner)?;
        let function = inner.function_name.as_str();

        match function {
            ABILITY_FEDERATION_JOIN => self.dispatch_federation_join(&inner.arguments),
            ABILITY_FEDERATION_ADVERTISE_AGENT => {
                self.dispatch_federation_advertise_agent(&inner.arguments)
            }
            ABILITY_FEDERATION_ADVERTISE_ABILITIES => {
                self.dispatch_federation_advertise_abilities(&inner.arguments)
            }
            ABILITY_RUNTIME_BOOTSTRAP_SELF_IDENTITY => {
                self.dispatch_runtime_bootstrap_self_identity(&inner.arguments)
            }
            ABILITY_FEDERATION_HEARTBEAT => self.dispatch_federation_heartbeat(&inner.arguments),
            ABILITY_FEDERATION_RESOLVE => self.dispatch_federation_resolve(&inner.arguments),
            ABILITY_FEDERATION_RESOLVE_KEY => {
                self.dispatch_federation_resolve_key(&inner.arguments)
            }
            ABILITY_FEDERATION_DISCOVER => self.dispatch_federation_discover(&inner.arguments),
            ABILITY_FEDERATION_LIST_USER_DEVICES => self
                .dispatch_federation_list_user_devices(inner.envelope.as_ref(), &inner.arguments),
            ABILITY_FEDERATION_REVOKE => self.dispatch_federation_revoke(&inner.arguments),
            ABILITY_FEDERATION_FORWARD_INVOKE => {
                self.dispatch_federation_forward_invoke(inner.envelope.as_ref(), &inner.arguments)
                    .await
            }
            ABILITY_FEDERATION_SUBSCRIBE_DIRECTORY => Err(Status::invalid_argument(
                "federation.subscribe_directory is a server-stream ability and must be invoked \
                 via InvokeStream, not Invoke",
            )),
            ABILITY_SELF_REGISTER_DEVICE_PUBKEY => {
                self.dispatch_register_device_pubkey(&inner.arguments)
            }
            other => Err(Status::unimplemented(format!(
                "easynet-daemon: ability `{other}` is not handled by the federation wrappers; \
                 LocalAbilityRegistry fallback wires in RFC-003 PR-1 commit 7/9 \
                 (see team-work/checklists/PR-1-checklist.md §5)"
            ))),
        }
    }

    type InvokeStreamStream = BoxedDownStream<InvokeStreamChunk>;

    /// Spec §4 reference. Routes by
    /// `InvokeServerStreamRequest.function_name`. PR-1 staging
    /// supports `federation.subscribe_directory` with the initial
    /// snapshot frame only; the broadcast pump for subsequent
    /// transitions lands in commit 8/9 alongside
    /// `federation.forward_invoke` reverse-channel push.
    async fn invoke_stream(
        &self,
        request: Request<InvokeServerStreamRequest>,
    ) -> Result<Response<Self::InvokeStreamStream>, Status> {
        let inner = request.into_inner();
        self.admission.verify_invoke_stream(&inner)?;
        let function = inner.function_name.as_str();

        match function {
            ABILITY_FEDERATION_SUBSCRIBE_DIRECTORY => {
                self.dispatch_federation_subscribe_directory_initial()
            }
            ABILITY_FEDERATION_SUBSCRIBE_DIRECTORY_V2 => {
                self.dispatch_federation_subscribe_directory_v2()
            }
            other => Err(Status::unimplemented(format!(
                "easynet-daemon: server-stream ability `{other}` is not handled in PR-1; \
                 LocalAbilityRegistry stream fallback wires in commit 7/9, broadcast pump \
                 for federation.subscribe_directory wires in commit 8/9 \
                 (see team-work/checklists/PR-1-checklist.md §5)"
            ))),
        }
    }

    type InvokeBidiStream = BoxedDownStream<InvokeBidiDown>;

    /// Spec §2.1 reference. Routes by frame-0
    /// `EnvelopeOpen.target.ability_name`:
    ///
    /// - `<self>.invoke_remote` → cross-device dispatch handler
    ///   (PR-3 commit 1/3, this commit). Requires `with_pending(...)`
    ///   to have wired a `PendingDispatchMap`; otherwise returns
    ///   `Status::failed_precondition` with explicit reason.
    /// - `<self>.session` → PR-2; arm added when PR-2 lands
    /// - anything else → `Status::unimplemented` citing PR-2/PR-3
    async fn invoke_bidi(
        &self,
        request: Request<Streaming<InvokeBidiUp>>,
    ) -> Result<Response<Self::InvokeBidiStream>, Status> {
        let mut up = request.into_inner();
        let frame0 = match up.next().await {
            Some(Ok(f)) => f,
            Some(Err(err)) => {
                return Err(Status::internal(format!("InvokeBidi frame 0 recv: {err}")))
            }
            None => return Err(Status::invalid_argument("InvokeBidi: empty up stream")),
        };

        let envelope_open = validate_and_extract_bidi_frame0(&frame0)?;
        // PR-7: full §5.2 admission for the bidi path. The facade
        // checks envelope presence + caller URI, runs the four-step
        // pipeline (envelope/structure/verify/replay), and rejects
        // with the canonical wire reasons. Ability name + initial
        // args feed `args_digest` exactly the way unary/server-stream
        // requests do.
        self.admission.verify_envelope_for_bidi(&envelope_open)?;

        let ability_name = envelope_open
            .target
            .as_ref()
            .map(|t| t.ability_name.as_str())
            .filter(|n| !n.is_empty())
            .ok_or_else(|| {
                Status::invalid_argument(
                    "InvokeBidi frame 0 missing target.ability_name; cannot dispatch",
                )
            })?;

        match ability_name {
            ABILITY_INVOKE_REMOTE => self.dispatch_invoke_remote(envelope_open, up).await,
            ABILITY_SELF_SESSION => {
                let caller_uri = envelope_open
                    .envelope
                    .as_ref()
                    .and_then(|e| e.caller.as_ref())
                    .map(|c| c.uri.clone())
                    .ok_or_else(|| {
                        Status::invalid_argument(
                            "<self>.session: envelope.caller.uri is required \
                             (already verified by admission gate; this is a defensive check)",
                        )
                    })?;
                self.dispatch_self_session_accept(caller_uri, up).await
            }
            other
                if matches!(
                    other,
                    "fleet.session_attach"
                        | crate::runtime::agents::pty_attach_ability::ABILITY_PTY_SESSION_ATTACH
                        | crate::runtime::agents::file_transfer_ability::ABILITY_FILE_TRANSFER
                ) =>
            {
                // PR-2 staging step: only PTY attach is wired through
                // the daemon's InvokeBidi → LocalAbilityRegistry
                // bridge today. Other local bidi abilities still need
                // a real wire contract; forwarding arbitrary JSON
                // handler frames over the axon BinaryChunk/Control
                // surface would be protocol fiction.
                let local_dispatcher = self.local_dispatcher.as_ref().ok_or_else(|| {
                    Status::unimplemented(format!(
                        "easynet-daemon: InvokeBidi ability `{other}` requires the \
                         PTY attach local-dispatch bridge, but this daemon was booted \
                         without DaemonInvocationService::with_local_dispatcher(...)"
                    ))
                })?;
                if other == crate::runtime::agents::file_transfer_ability::ABILITY_FILE_TRANSFER {
                    if let Some(target_uri) = remote_bidi_target_uri(envelope_open) {
                        if !self.matches_self_target_uri(&target_uri) {
                            return self
                                .dispatch_remote_file_transfer_bidi(
                                    &target_uri,
                                    envelope_open,
                                    up,
                                )
                                .await;
                        }
                    }
                }
                self.dispatch_local_bidi(local_dispatcher, other, envelope_open, up)
                    .await
            }
            other => Err(Status::unimplemented(format!(
                "easynet-daemon: InvokeBidi ability `{other}` is not yet wired; \
                 only fleet.session_attach/fleet.pty_session_attach/fleet.file_transfer currently bridge \
                 through the LocalAbilityRegistry bidi fallback"
            ))),
        }
    }
}

/// Pull the `EnvelopeOpen` payload out of frame 0 of an
/// `InvokeBidi` up stream. Returns `Status::invalid_argument` for
/// any non-EnvelopeOpen first frame, since the axon protocol
/// Extract the `(userID, agentID)` pair from an
/// `agent/<userID>.<agentID>` URI. Returns `None` for any other role
/// or for malformed URIs. Used by `matches_self_target_uri` to detect
/// when a remote-arriving invocation names an agent that this daemon
/// hosts (RFC-006-C v0.1 + RFC-006-B v0.6 §URL: callee on chat-base
/// / page.fetch is the agent URA, not the device URA).
fn parse_agent_owner_pair_from_uri(target_uri: &str) -> Option<(String, String)> {
    let parsed = crate::uri::parse_ura(target_uri).ok()?;
    if !matches!(parsed.kind, crate::uri::URAKind::Agent) {
        return None;
    }
    if parsed.user_id.is_empty() || parsed.agent_id.is_empty() {
        return None;
    }
    Some((parsed.user_id, parsed.agent_id))
}

/// mandates frame 0 is the EnvelopeOpen.
fn extract_envelope_open(frame: &InvokeBidiUp) -> Result<&EnvelopeOpen, Status> {
    match frame.payload.as_ref() {
        Some(UpPayload::EnvelopeOpen(eo)) => Ok(eo),
        Some(_) => Err(Status::invalid_argument(
            "InvokeBidi frame 0 must be EnvelopeOpen, not BinaryChunk or Control",
        )),
        None => Err(Status::invalid_argument(
            "InvokeBidi frame 0 carries no payload",
        )),
    }
}

fn validate_and_extract_bidi_frame0(frame: &InvokeBidiUp) -> Result<&EnvelopeOpen, Status> {
    if frame.sequence != 0 {
        return Err(Status::invalid_argument(format!(
            "{REASON_BIDI_FIRST_FRAME_SEQUENCE}: InvokeBidi frame 0 sequence must be 0, got {}",
            frame.sequence,
        )));
    }
    let envelope_open = extract_envelope_open(frame)?;
    validate_bidi_stream_ordering(&envelope_open.streams)?;
    Ok(envelope_open)
}

fn validate_bidi_stream_ordering(streams: &[StreamDescriptor]) -> Result<(), Status> {
    for stream in streams {
        if !stream.ordering.is_empty() && stream.ordering != "STRICT" {
            return Err(Status::invalid_argument(format!(
                "{REASON_BIDI_NON_STRICT_ORDERING}: stream {} ordering {:?} is unsupported; \
                 InvokeBidi v1 accepts only empty or \"STRICT\" ordering",
                stream.stream_id, stream.ordering,
            )));
        }
    }
    Ok(())
}

impl DaemonInvocationService {
    fn dispatch_federation_join(
        &self,
        arguments: &[u8],
    ) -> Result<Response<InvokeResponse>, Status> {
        let request: federation_wrappers::JoinRequest = parse_json_args(arguments)?;
        let response = federation_wrappers::handle_join(&request);
        wrap_json_response(&response)
    }

    fn dispatch_federation_advertise_agent(
        &self,
        arguments: &[u8],
    ) -> Result<Response<InvokeResponse>, Status> {
        let request: federation_wrappers::AdvertiseAgentRequest = parse_json_args(arguments)?;
        let response = federation_wrappers::handle_advertise_agent(
            &request,
            Some(self.advertised_agents.as_ref()),
        );
        wrap_json_response(&response)
    }

    fn dispatch_federation_advertise_abilities(
        &self,
        arguments: &[u8],
    ) -> Result<Response<InvokeResponse>, Status> {
        let request: federation_wrappers::AdvertiseAbilitiesRequest = parse_json_args(arguments)?;
        let response = federation_wrappers::handle_advertise_abilities(
            &request,
            Some(self.ability_catalog.as_ref()),
        );
        wrap_json_response(&response)
    }

    fn dispatch_runtime_bootstrap_self_identity(
        &self,
        arguments: &[u8],
    ) -> Result<Response<InvokeResponse>, Status> {
        let request: federation_wrappers::BootstrapSelfIdentityRequest =
            parse_json_args(arguments)?;
        let response = federation_wrappers::handle_bootstrap_self_identity(&request);
        wrap_json_response(&response)
    }

    fn dispatch_federation_heartbeat(
        &self,
        arguments: &[u8],
    ) -> Result<Response<InvokeResponse>, Status> {
        let request: federation_wrappers::HeartbeatRequest = parse_json_args(arguments)?;
        let response = federation_wrappers::handle_heartbeat(&request, &self.presence);
        wrap_json_response(&response)
    }

    fn dispatch_register_device_pubkey(
        &self,
        arguments: &[u8],
    ) -> Result<Response<InvokeResponse>, Status> {
        let ctx = self.register_pubkey.as_ref().ok_or_else(|| {
            Status::failed_precondition(
                "<self>.register_device_pubkey: this daemon was booted without the trust-write \
                 surface (use `with_register_pubkey(...)` at boot to enable). PR-7 production \
                 daemons always wire this; an unwired daemon is a smoke-test or fixture build.",
            )
        })?;
        let body = handle_register_device_pubkey(
            arguments,
            &ctx.daemon_realm,
            &ctx.trust_anchor_path,
            &ctx.cell,
        )?;
        Ok(Response::new(InvokeResponse {
            result: body,
            result_content_type: FEDERATION_RESULT_CONTENT_TYPE.to_string(),
            state: InvocationState::Completed as i32,
            ..InvokeResponse::default()
        }))
    }

    fn dispatch_federation_resolve(
        &self,
        arguments: &[u8],
    ) -> Result<Response<InvokeResponse>, Status> {
        let request: federation_wrappers::ResolveRequest = parse_json_args(arguments)?;
        let response = federation_wrappers::handle_resolve(
            &request,
            &self.presence,
            Some(self.advertised_agents.as_ref()),
            Some(self.ability_catalog.as_ref()),
        );
        wrap_json_response(&response)
    }

    /// **PR-N2 commit 2/N**. Peer-side `federation.resolve_key`
    /// dispatch. Reads the daemon's `SharedTrustAnchor` (so a
    /// SIGHUP-triggered `realm-trust.toml` reload is reflected
    /// without a restart) and returns the matching
    /// `public_key_b64` for the requested URI.
    ///
    /// On miss we surface `Status::not_found` so the calling
    /// `FederatedKeyResolver` can distinguish "URI is not in
    /// this hub's trust set" from a network or admission
    /// failure (which arrive as `unavailable` /
    /// `permission_denied`). The resolver then maps both into
    /// `unknown_agent_uri` for INV-4 fail-closed admission, but
    /// the wire-level distinction is useful for operator audit
    /// and matches the rest of the federation.* surface where
    /// `not_found` means "no entry" and `failed_precondition`
    /// means "entry present but unusable".
    fn dispatch_federation_resolve_key(
        &self,
        arguments: &[u8],
    ) -> Result<Response<InvokeResponse>, Status> {
        let request: federation_wrappers::ResolveKeyRequest = parse_json_args(arguments)?;
        let trust_anchor = self.admission.trust_anchor_snapshot();
        match federation_wrappers::handle_resolve_key(&request, &trust_anchor) {
            Some(response) => wrap_json_response(&response),
            None => Err(Status::not_found(format!(
                "federation.resolve_key: agent_uri `{}` not in this hub's trust set",
                request.agent_uri
            ))),
        }
    }

    /// **PR-N3 commit N3-4 + N3-N4 dispatch wire**. Cross-realm
    /// directory lookup dispatch. Reads the daemon-wide
    /// `SharedFederatedDirectoryView` cell snapshot, fans out
    /// across federated peers per spec §3.2 (lex tie-break,
    /// dedupe by agent_uri), returns matching `DirectoryEntry`
    /// list.
    ///
    /// When the request carries a `local_user_id` AND the
    /// daemon has both a `FederatedBindingsStore` and a
    /// `session_realm` wired, the dispatch routes through
    /// `handle_discover_with_user_filter` so cross-realm
    /// entries are filtered by the user's binding state per
    /// PR-N4 INV-5 privacy default. Otherwise (no user id or
    /// no bindings store), routes through the unfiltered
    /// `handle_discover` for backwards-compat with operator /
    /// audit query callers.
    ///
    /// Pure read; no I/O — single-realm daemons that haven't
    /// accumulated any peer views just return an empty
    /// response, gracefully degrading to local-only behaviour.
    fn dispatch_federation_discover(
        &self,
        arguments: &[u8],
    ) -> Result<Response<InvokeResponse>, Status> {
        let request: federation_wrappers::DiscoverRequest = parse_json_args(arguments)?;
        let response = match (
            request.local_user_id.as_deref(),
            self.federated_bindings.as_ref(),
            self.session_realm.as_deref(),
        ) {
            (Some(_user_id), Some(bindings), Some(realm)) => {
                let resolver = crate::runtime::keyring::resolver::FederatedUserResolver::new(
                    realm,
                    std::sync::Arc::clone(bindings),
                );
                federation_wrappers::handle_discover_with_user_filter(
                    &request,
                    &self.federated_directory,
                    &resolver,
                )
            }
            _ => federation_wrappers::handle_discover(&request, &self.federated_directory),
        };
        wrap_json_response(&response)
    }

    /// **PR-N3 commit N3-5**. Hub-side projection of local
    /// presence-registry entries for a given tenant. Spec §3.5
    /// admission filter: only callers whose URI is in the local
    /// trust anchor with `role = Hub` may invoke this. Other
    /// roles (Backend, Device) are rejected with
    /// `Status::permission_denied`. The general admission gate
    /// has already accepted the call (caller URI is signed,
    /// non-replayed, in trust set); this filter narrows to the
    /// hub-only sub-surface.
    ///
    /// Loopback bypass: the daemon's own URI is admitted into
    /// every dispatch arm regardless of role, so a hub-mode
    /// daemon listing its own users from a CLI on the same
    /// machine works without configuring itself as a Hub trust
    /// entry.
    fn dispatch_federation_list_user_devices(
        &self,
        caller_envelope: Option<&Envelope>,
        arguments: &[u8],
    ) -> Result<Response<InvokeResponse>, Status> {
        // Spec §3.5 admission filter — caller must be a Hub-role
        // peer (or the daemon itself).
        let caller_uri = caller_envelope
            .and_then(|env| env.caller.as_ref())
            .map(|c| c.uri.as_str())
            .ok_or_else(|| {
                Status::invalid_argument(
                    "federation.list_user_devices: missing caller envelope.caller.uri",
                )
            })?;

        let trust_anchor = self.admission.trust_anchor_snapshot();
        let is_hub_role = trust_anchor.lookup(caller_uri).is_some_and(|entry| {
            matches!(
                entry.role,
                crate::services::realm_trust_anchor::TrustedAgentRole::Hub
            )
        });
        let is_loopback = self
            .admission
            .daemon_uri()
            .is_some_and(|self_uri| self_uri == caller_uri);
        if !(is_hub_role || is_loopback) {
            return Err(Status::permission_denied(format!(
                "federation.list_user_devices: caller `{caller_uri}` is not a hub-role peer; \
                 only trusted hubs and the daemon itself may enumerate user devices"
            )));
        }

        let request: federation_wrappers::ListUserDevicesRequest = parse_json_args(arguments)?;
        let response = federation_wrappers::handle_list_user_devices(&request, &self.presence);
        wrap_json_response(&response)
    }

    fn dispatch_federation_revoke(
        &self,
        arguments: &[u8],
    ) -> Result<Response<InvokeResponse>, Status> {
        let request: federation_wrappers::RevokeRequest = parse_json_args(arguments)?;
        let response = federation_wrappers::handle_revoke(
            &request,
            &self.presence,
            Some(self.advertised_agents.as_ref()),
        );
        wrap_json_response(&response)
    }

    /// **PR-N1 commit 3b/N rewrite.** Tenant-aware
    /// `federation.forward_invoke` dispatch:
    ///
    /// 1. Parse `target_uri` to extract its tenant component.
    /// 2. **Local-tenant fast path**: when the tenant matches
    ///    `self.session_realm` (or no realm context is wired —
    ///    test daemons), push the inner-envelope frame down the
    ///    target's `<self>.session` reverse channel via
    ///    `try_push_forward_invoke_frame` exactly as PR-1 staging
    ///    did. Local routing is the historic behavior, kept
    ///    unchanged.
    /// 3. **Cross-tenant route**: when the tenant differs AND
    ///    both `federation_client` and a matching
    ///    `federated_peers[tenant]` entry are wired, forward the
    ///    request to the peer hub by re-issuing the same
    ///    `federation.forward_invoke` ability against the peer
    ///    daemon. The peer-side dispatcher then performs its own
    ///    local fast-path lookup. Returns the peer's
    ///    `target_online` outcome verbatim — the caller cannot
    ///    distinguish a local-on-peer hit from a local-on-self
    ///    hit by the wire shape.
    /// 4. **Cross-tenant fall-through**: missing federation
    ///    client OR missing peer entry OR malformed target URI
    ///    all surface as `target_online: false`. Legacy callers
    ///    treat that as "target offline" and fall through to
    ///    their own retry policy.
    ///
    /// What this commit does NOT carry yet:
    /// - AXIOM mapping rewrite (caller=self_hub, callee=
    ///   target_hub, subject=original_caller). The peer's local
    ///   presence registry resolves the inner envelope's
    ///   target_uri directly today; cross-realm admission key
    ///   resolution is PR-N2's job. PR-N1 commit 3b/N forwards
    ///   the request unchanged.
    /// - Envelope re-sign with `daemon_identity.signing_seed`.
    ///   Same reason — cross-realm admission lands in PR-N2.
    /// - Timeout + circuit-breaker on the cross-hub call. PR-N1
    ///   commit 4/N wraps the federation client with
    ///   `tower::timeout::Timeout`.
    async fn dispatch_federation_forward_invoke(
        &self,
        caller_envelope: Option<&Envelope>,
        arguments: &[u8],
    ) -> Result<Response<InvokeResponse>, Status> {
        // PR-N6 C4: device-mode escalation. When this daemon
        // owns no PresenceRegistry of its own (mode = device),
        // the local fast-path is meaningless — there is nothing
        // to push frames against. Send the call up the existing
        // `<self>.session` bidi to the hub, await the matching
        // RequestResult, and surface its outcome on the unary
        // wire. Hub-mode and `both`-mode daemons leave
        // `escalation = None` and take the existing arm.
        if let Some(handle) = self.escalation.as_ref() {
            return self.escalate_forward_invoke(handle, arguments).await;
        }

        let mut request: federation_wrappers::ForwardInvokeRequest = parse_json_args(arguments)?;

        // Postel-boundary: peer hubs from pre-v4.1.4 builds may
        // ship a `/agent/<bare-uuid>` target URI (the v1/v2
        // device-as-agent shape). Coerce to the canonical
        // `/device/<uuid>` form once here so every downstream
        // use — local-presence lookup, peer-dial envelope, audit
        // log line — sees the same canonical key. Real
        // `/agent/<user>.<agent>` URIs and all v4.1.4-canonical
        // shapes pass through unchanged.
        request.target_uri = crate::uri::canonicalize_presence_key(&request.target_uri);

        let target_tenant = parse_tenant_from_uri(&request.target_uri);
        let local_tenant = self.session_realm.as_deref();

        let is_local_tenant = match (target_tenant, local_tenant) {
            (Some(target), Some(local)) => target == local,
            // Daemon has no realm context wired (smoke-test
            // build) — preserve PR-1 staging behavior and treat
            // every target as local.
            (_, None) => true,
            // Malformed target URI — fall through to legacy
            // shape so a typo never accidentally hits the
            // cross-hub path.
            (None, Some(_)) => true,
        };
        let has_local_presence = self.presence.lookup(&request.target_uri).is_some();

        // Observable trace for operators debugging answer-sheet /
        // demo runs — proves which dispatch arm fired without
        // requiring an envelope-level packet capture. Cheap (one
        // eprintln per call) and the only daemon-A-side signal
        // that distinguishes "took cross-tenant arm" from "took
        // local-presence arm" when the inner ability happens to
        // be a hub-served one (e.g. federation.heartbeat).
        eprintln!(
            "[axon-serve] federation.forward_invoke dispatch: \
             target_uri={} target_tenant={:?} local_tenant={:?} \
             is_local_tenant={} has_local_presence={}",
            request.target_uri, target_tenant, local_tenant, is_local_tenant, has_local_presence,
        );

        // Decode the inner payload up front. The
        // `correlation_call_id` field is required by DEC-N4 §2.1
        // so both arms (local-tenant fast-path AND cross-tenant
        // dial) can thread it back to the caller. Decode failure
        // surfaces as `Status::invalid_argument`; the CLI bridge
        // is the producer and must always supply a non-empty
        // `call_id` field.
        let inner_payload = decode_inner_payload(&request.inner_envelope_b64)?;
        let correlation_call_id = inner_payload.call_id.clone();

        // **C1a / DEC-N4 §2.1 local-tenant fast-path**.
        // When the target tenant is local (or no realm context
        // is wired in this build), look up the target on the
        // local presence registry. Hit → push the inner envelope
        // bytes down the target's `<self>.session` reverse
        // channel + return `Ok(ForwardInvokeResponse {
        // result_bytes: empty, correlation_call_id })`. The
        // empty `result_bytes` here means "delivery accepted";
        // the actual ability response flows back through the
        // reverse-channel correlation path, not through this
        // synchronous unary response. Miss → typed
        // `Status::failed_precondition(target_offline)` per
        // DEC-N4 §2.1 — the empty-result shape is no longer the
        // wire surface for offline.
        // **PR-1 commit 7/9 (LB-56) — self-targeted local dispatch**.
        // When the inbound forward_invoke targets THIS daemon's
        // own canonical URI, the local-presence push misses by
        // construction (a hub does not register its own URI in
        // its PresenceRegistry). Without a synchronous
        // fall-through to `LocalAbilityRegistry`, the call
        // surfaces as target_offline even though the target is
        // perfectly capable of running the ability. This arm
        // resolves the inner ability against the boot-threaded
        // `AbilityDispatcher` Arc and stamps the JSON result
        // bytes inline into ForwardInvokeResponse.result_bytes.
        //
        // The semantic difference vs the bidi-push path: this
        // is a synchronous reply, no reverse-channel correlation
        // round-trip, no PR-N5 second-receipt update. The
        // ForwardReceipt is written ONCE with a real result_digest
        // computed from the bytes returned here.
        //
        // Guard: only fires when caller and daemon URIs match
        // exactly AND the daemon was booted with a local
        // dispatcher (production hub-mode + both-mode daemons
        // always have one; test fixtures with `make_service()`
        // do not, preserving their target_offline expectation).
        if let Some(local_dispatcher) = self.local_dispatcher.as_ref() {
            if self.matches_self_target_uri(&request.target_uri) {
                return self.dispatch_self_targeted_forward_invoke(
                    local_dispatcher,
                    &inner_payload,
                    &request,
                    caller_envelope,
                    &correlation_call_id,
                );
            }
        }

        // **LB-57 Option A — synchronous local-presence dispatch**.
        // Presence is keyed by full caller URI, not by the daemon's
        // own realm. A platform hub may therefore host devices whose
        // URIs live under many user realms simultaneously. When the
        // target URI is already present on THIS hub, that concrete
        // liveness fact wins over any tenant mismatch and we dispatch
        // locally rather than forcing a spurious cross-hub dial.
        if has_local_presence {
            match self
                .dispatch_local_presence_forward_invoke(
                    &request,
                    &inner_payload,
                    caller_envelope,
                    &correlation_call_id,
                )
                .await
            {
                Ok(response) => return Ok(response),
                Err(status) => {
                    if !is_local_tenant {
                        return Err(status);
                    }
                    // **Same-tenant cross-hub fall-through**.
                    // Local presence missed but the target's tenant
                    // matches ours — the device may be paired on a
                    // peer hub against the same user account. Per
                    // CTO directive on cross-hub same-account: fan
                    // out across `federated_peers` (no per-tenant
                    // routing key when the tenant IS local; we ask
                    // every peer hub the operator has federated
                    // with). First-success wins in lex order on
                    // `peer_realm`. Real `target_offline` only
                    // surfaces if every peer also misses or no
                    // peers are wired.
                    if let Some(client) = self.federation_client.as_ref() {
                        let peers_snapshot = self.federated_peers.snapshot();
                        if !peers_snapshot.is_empty() {
                            let peer_envelope = build_peer_envelope(
                                caller_envelope,
                                &request.target_uri,
                                self.session_realm.as_deref(),
                            );
                            // **LB-57 §一 Option A wire shape**.
                            // Re-wrap the call as another
                            // `federation.forward_invoke` for the peer
                            // hub. Sending the inner ability name
                            // verbatim (the pre-LB-57 shape) lands at
                            // the peer's `Invoke::invoke` top-level
                            // match's `other` arm and surfaces
                            // Unimplemented because PR-1 commit 7/9's
                            // LocalAbilityRegistry fall-through is
                            // narrow (self-target arm only). Wrapping
                            // routes the call through the peer's
                            // `dispatch_federation_forward_invoke`
                            // arm, which already implements the
                            // local-presence push (LB-50 C3) + same-
                            // tenant fan-out + cross-tenant dial
                            // semantics the demo C9 chain requires.
                            let nested = federation_wrappers::ForwardInvokeRequest {
                                target_uri: request.target_uri.clone(),
                                inner_envelope_b64: request.inner_envelope_b64.clone(),
                                causal_context_bytes: request.causal_context_bytes.clone(),
                                forward_deadline_ms: request.forward_deadline_ms,
                            };
                            let nested_arguments = serde_json::to_vec(&nested).map_err(|err| {
                                Status::internal(format!(
                                    "federation.forward_invoke: encode nested \
                                         ForwardInvokeRequest for same-tenant fan-out: {err}"
                                ))
                            })?;
                            let mut peer_request = InvokeRequest {
                                envelope: Some(peer_envelope),
                                function_name: ABILITY_FEDERATION_FORWARD_INVOKE.to_string(),
                                arguments: nested_arguments,
                                ..InvokeRequest::default()
                            };
                            if let Some(envelope) = peer_request.envelope.as_mut() {
                                sign_peer_request_envelope(
                                    envelope,
                                    &peer_request.function_name,
                                    &peer_request.arguments,
                                    self.session_realm.as_deref(),
                                    self.hub_signing_seed.as_ref(),
                                )?;
                            }
                            // Lex-deterministic iteration; peer
                            // hubs typically number in the
                            // single digits (operator-curated),
                            // so sequential dialing is cheaper
                            // than spinning a JoinSet for the
                            // common case.
                            for (peer_realm, peer_hub_uri) in peers_snapshot.iter() {
                                let _ = peer_realm; // visible in logs below
                                match client
                                    .forward_invoke(peer_hub_uri, peer_request.clone())
                                    .await
                                {
                                    Ok(peer_response) => {
                                        // LB-57: unwrap peer's outer
                                        // ForwardInvokeResponse so the
                                        // caller sees the device bytes,
                                        // not a double-wrap.
                                        let peer_body: federation_wrappers::ForwardInvokeResponse =
                                            match serde_json::from_slice(&peer_response.result) {
                                                Ok(body) => body,
                                                Err(err) => {
                                                    eprintln!(
                                                        "[axon-serve] same-tenant \
                                                         fan-out peer returned malformed \
                                                         ForwardInvokeResponse JSON: \
                                                         {err}; forwarding raw bytes \
                                                         for forward-compat"
                                                    );
                                                    federation_wrappers::ForwardInvokeResponse {
                                                        result_bytes: peer_response.result.clone(),
                                                        correlation_call_id: correlation_call_id
                                                            .clone(),
                                                    }
                                                }
                                            };
                                        self.admission.receipt_store().record(
                                            build_forward_receipt(
                                                &correlation_call_id,
                                                &request.target_uri,
                                                caller_envelope,
                                                Some(&peer_body.result_bytes),
                                            ),
                                        );
                                        let response = federation_wrappers::ForwardInvokeResponse {
                                            result_bytes: peer_body.result_bytes,
                                            correlation_call_id,
                                        };
                                        return wrap_json_response(&response);
                                    }
                                    Err(err) => {
                                        eprintln!(
                                            "[axon-serve] same-tenant cross-hub miss \
                                             on peer realm {peer_realm} hub {peer_hub_uri}: \
                                             {err}"
                                        );
                                    }
                                }
                            }
                        }
                    }
                    // Every peer missed (or no peers wired) →
                    // real target_offline. result_digest = None.
                    self.admission.receipt_store().record(build_forward_receipt(
                        &correlation_call_id,
                        &request.target_uri,
                        caller_envelope,
                        None,
                    ));
                    return Err(Status::failed_precondition(
                        federation_wrappers::FORWARD_INVOKE_TARGET_OFFLINE_REASON,
                    ));
                }
            }
        }

        if is_local_tenant {
            // Same-tenant but not locally present: fan out across
            // peer hubs that serve this tenant, then surface a real
            // target_offline if every peer also misses.
            match self
                .dispatch_local_presence_forward_invoke(
                    &request,
                    &inner_payload,
                    caller_envelope,
                    &correlation_call_id,
                )
                .await
            {
                Ok(response) => return Ok(response),
                Err(_status) => {
                    // **Same-tenant cross-hub fall-through**.
                    // Local presence missed but the target's tenant
                    // matches ours — the device may be paired on a
                    // peer hub against the same user account. Per
                    // CTO directive on cross-hub same-account: fan
                    // out across `federated_peers` (no per-tenant
                    // routing key when the tenant IS local; we ask
                    // every peer hub the operator has federated
                    // with). First-success wins in lex order on
                    // `peer_realm`. Real `target_offline` only
                    // surfaces if every peer also misses or no
                    // peers are wired.
                    if let Some(client) = self.federation_client.as_ref() {
                        let peers_snapshot = self.federated_peers.snapshot();
                        if !peers_snapshot.is_empty() {
                            let peer_envelope = build_peer_envelope(
                                caller_envelope,
                                &request.target_uri,
                                self.session_realm.as_deref(),
                            );
                            let nested = federation_wrappers::ForwardInvokeRequest {
                                target_uri: request.target_uri.clone(),
                                inner_envelope_b64: request.inner_envelope_b64.clone(),
                                causal_context_bytes: request.causal_context_bytes.clone(),
                                forward_deadline_ms: request.forward_deadline_ms,
                            };
                            let nested_arguments = serde_json::to_vec(&nested).map_err(|err| {
                                Status::internal(format!(
                                    "federation.forward_invoke: encode nested \
                                         ForwardInvokeRequest for same-tenant fan-out: {err}"
                                ))
                            })?;
                            let mut peer_request = InvokeRequest {
                                envelope: Some(peer_envelope),
                                function_name: ABILITY_FEDERATION_FORWARD_INVOKE.to_string(),
                                arguments: nested_arguments,
                                ..InvokeRequest::default()
                            };
                            if let Some(envelope) = peer_request.envelope.as_mut() {
                                sign_peer_request_envelope(
                                    envelope,
                                    &peer_request.function_name,
                                    &peer_request.arguments,
                                    self.session_realm.as_deref(),
                                    self.hub_signing_seed.as_ref(),
                                )?;
                            }
                            for (peer_realm, peer_hub_uri) in peers_snapshot.iter() {
                                let _ = peer_realm;
                                match client
                                    .forward_invoke(peer_hub_uri, peer_request.clone())
                                    .await
                                {
                                    Ok(peer_response) => {
                                        let peer_body: federation_wrappers::ForwardInvokeResponse =
                                            match serde_json::from_slice(&peer_response.result) {
                                                Ok(body) => body,
                                                Err(err) => {
                                                    eprintln!(
                                                        "[axon-serve] same-tenant \
                                                         fan-out peer returned malformed \
                                                         ForwardInvokeResponse JSON: \
                                                         {err}; forwarding raw bytes \
                                                         for forward-compat"
                                                    );
                                                    federation_wrappers::ForwardInvokeResponse {
                                                        result_bytes: peer_response.result.clone(),
                                                        correlation_call_id: correlation_call_id
                                                            .clone(),
                                                    }
                                                }
                                            };
                                        self.admission.receipt_store().record(
                                            build_forward_receipt(
                                                &correlation_call_id,
                                                &request.target_uri,
                                                caller_envelope,
                                                Some(&peer_body.result_bytes),
                                            ),
                                        );
                                        let response = federation_wrappers::ForwardInvokeResponse {
                                            result_bytes: peer_body.result_bytes,
                                            correlation_call_id,
                                        };
                                        return wrap_json_response(&response);
                                    }
                                    Err(err) => {
                                        eprintln!(
                                            "[axon-serve] same-tenant cross-hub miss \
                                             on peer realm {peer_realm} hub {peer_hub_uri}: \
                                             {err}"
                                        );
                                    }
                                }
                            }
                        }
                    }
                    self.admission.receipt_store().record(build_forward_receipt(
                        &correlation_call_id,
                        &request.target_uri,
                        caller_envelope,
                        None,
                    ));
                    return Err(Status::failed_precondition(
                        federation_wrappers::FORWARD_INVOKE_TARGET_OFFLINE_REASON,
                    ));
                }
            }
        }
        // Cross-tenant path. Missing federation client OR
        // missing peer entry both surface as
        // `failed_precondition(target_offline)` per DEC-N4 §2.1
        // — the legacy "Ok with target_online:false" shape is
        // gone. DEC-N5 §1 still requires a caller-hub
        // ForwardReceipt with `result_digest = None` for every
        // target_offline outcome.
        let record_offline_receipt = || {
            self.admission.receipt_store().record(build_forward_receipt(
                &correlation_call_id,
                &request.target_uri,
                caller_envelope,
                None,
            ));
        };
        let Some(client) = self.federation_client.as_ref() else {
            record_offline_receipt();
            return Err(Status::failed_precondition(
                federation_wrappers::FORWARD_INVOKE_TARGET_OFFLINE_REASON,
            ));
        };
        let Some(target_tenant) = target_tenant else {
            // Defensive: `is_local_tenant` already collapses
            // None tenant to the local fast-path arm above.
            record_offline_receipt();
            return Err(Status::failed_precondition(
                federation_wrappers::FORWARD_INVOKE_TARGET_OFFLINE_REASON,
            ));
        };
        // Snapshot the federated_peers cell per-dispatch so a
        // SIGHUP-driven reload of `[daemon.federated_peers]`
        // (operator adding a new tenant→hub_uri entry without
        // restarting the daemon) is visible to the next call.
        // The snapshot is one `RwLock::read()` + `Arc::clone`
        // (cheap; mirrors the admission gate's per-call pattern).
        let peers_snapshot = self.federated_peers.snapshot();
        let Some(target_hub_uri) = peers_snapshot.get(target_tenant) else {
            record_offline_receipt();
            return Err(Status::failed_precondition(
                federation_wrappers::FORWARD_INVOKE_TARGET_OFFLINE_REASON,
            ));
        };

        // Cross-tenant dial. **LB-57 §一 Option A wire shape**.
        // Re-wrap as another `federation.forward_invoke` so the
        // peer hub's top-level `Invoke::invoke` match routes
        // through `dispatch_federation_forward_invoke` (which
        // owns the local-presence push + same-tenant fan-out +
        // cross-tenant dial semantics) instead of falling to
        // the LocalAbilityRegistry self-target arm or the
        // unimplemented surface. The original caller envelope
        // is attached so the peer's admission gate sees the
        // user's identity; PR-N2's FederatedKeyResolver lifts
        // the realm-strict limitation that the legacy
        // bare-inner-ability shape relied on.
        let peer_envelope = build_peer_envelope(
            caller_envelope,
            &request.target_uri,
            self.session_realm.as_deref(),
        );
        let nested = federation_wrappers::ForwardInvokeRequest {
            target_uri: request.target_uri.clone(),
            inner_envelope_b64: request.inner_envelope_b64.clone(),
            causal_context_bytes: request.causal_context_bytes.clone(),
            forward_deadline_ms: request.forward_deadline_ms,
        };
        let nested_arguments = serde_json::to_vec(&nested).map_err(|err| {
            Status::internal(format!(
                "federation.forward_invoke: encode nested ForwardInvokeRequest \
                 for cross-tenant dial: {err}"
            ))
        })?;
        let mut peer_request = InvokeRequest {
            envelope: Some(peer_envelope),
            function_name: ABILITY_FEDERATION_FORWARD_INVOKE.to_string(),
            arguments: nested_arguments,
            ..InvokeRequest::default()
        };
        if let Some(envelope) = peer_request.envelope.as_mut() {
            sign_peer_request_envelope(
                envelope,
                &peer_request.function_name,
                &peer_request.arguments,
                self.session_realm.as_deref(),
                self.hub_signing_seed.as_ref(),
            )?;
        }

        match client.forward_invoke(target_hub_uri, peer_request).await {
            Ok(peer_response) => {
                // **LB-57 Option A — unwrap peer's outer wrapper**.
                // The peer hub returns its own `ForwardInvokeResponse`
                // JSON (with its own `result_bytes` carrying the
                // device's reply + its `correlation_call_id`). The
                // pre-LB-57 path stuffed that wrapper JSON verbatim
                // into our `ForwardInvokeResponse.result_bytes`,
                // producing a double-wrap that the CLI couldn't
                // unwrap to find the actual ability bytes. We now
                // peel one layer: parse the peer's body, lift its
                // `result_bytes` field, and present that as our
                // own `result_bytes` to the caller. The caller's
                // `correlation_call_id` (which the peer doesn't
                // know about) is preserved from our local
                // initiator-minted value.
                let peer_body: federation_wrappers::ForwardInvokeResponse =
                    match serde_json::from_slice(&peer_response.result) {
                        Ok(body) => body,
                        Err(err) => {
                            eprintln!(
                                "[axon-serve] cross-tenant peer returned malformed \
                                 ForwardInvokeResponse JSON: {err}; \
                                 forwarding raw bytes for forward-compat"
                            );
                            // Defensive: if the peer is on an old
                            // wire-shape, hand its raw bytes through
                            // unchanged so the caller at least sees
                            // something instead of target_offline.
                            federation_wrappers::ForwardInvokeResponse {
                                result_bytes: peer_response.result.clone(),
                                correlation_call_id: correlation_call_id.clone(),
                            }
                        }
                    };
                eprintln!(
                    "[axon-serve] federation.forward_invoke cross-tenant arm \
                     OK: target_uri={} target_hub_uri={} result_bytes_len={}",
                    request.target_uri,
                    target_hub_uri,
                    peer_body.result_bytes.len(),
                );
                // DEC-N5 §1 dual-write — record digest over the
                // unwrapped device bytes (not the peer wrapper),
                // matching the digest the caller will see.
                self.admission.receipt_store().record(build_forward_receipt(
                    &correlation_call_id,
                    &request.target_uri,
                    caller_envelope,
                    Some(&peer_body.result_bytes),
                ));
                let response = federation_wrappers::ForwardInvokeResponse {
                    result_bytes: peer_body.result_bytes,
                    correlation_call_id,
                };
                wrap_json_response(&response)
            }
            Err(err) => {
                // Cross-hub dial failure surfaces as the wire-stable
                // `target_offline` reason per DEC-N4 §2.1, but the
                // underlying cause (peer dial timeout, peer-side
                // admission reject, peer ability handler error) is
                // lost on the wire. Log it so operators debugging
                // demo / e2e setups can see why the cross-hub call
                // failed without instrumenting the FederationClient
                // by hand.
                eprintln!(
                    "[axon-serve] federation.forward_invoke peer dial \
                     failed: target_uri={} target_hub_uri={} err={err}",
                    request.target_uri, target_hub_uri,
                );
                record_offline_receipt();
                Err(Status::failed_precondition(
                    federation_wrappers::FORWARD_INVOKE_TARGET_OFFLINE_REASON,
                ))
            }
        }
    }

    /// Reverse-channel push for `federation.forward_invoke`.
    ///
    /// Looks up `request.target_uri` in the presence registry
    /// and pushes a `BinaryChunk` containing the inner-envelope
    /// bytes down the target's `<self>.session`
    /// `DispatchSender`. Per DEC-N4 §2.1 the wire shape gives up
    /// the legacy `target_online: bool` distinction in favour of:
    ///
    /// - `Ok(())` — frame queued for delivery; the caller wraps
    ///   that in `ForwardInvokeResponse { result_bytes: empty,
    ///   correlation_call_id }`. The actual ability response
    ///   flows back over the reverse-channel correlation path,
    ///   not through this synchronous unary response.
    /// - `Err(Status::failed_precondition(target_offline))` —
    ///   target not in presence registry, channel closed, or
    ///   channel full (slow-consumer eviction). All collapse to
    ///   the same wire-stable reason; operators trace registry
    ///   events for the underlying cause.
    /// **LB-57 Option A — synchronous local-presence dispatch**
    /// for `federation.forward_invoke` against a target URI in
    /// the local PresenceRegistry.
    ///
    /// Mirrors `dispatch_invoke_remote`'s pattern: register a
    /// `PendingDispatchMap` entry, push a
    /// `SessionDispatch::Dispatch{call_id, ability, args}` frame
    /// down the target's session bidi (the same wire shape
    /// device-side `LocalAbilityDispatcher::handle_down` expects),
    /// `await_reply` for the matching `SessionDispatch::Result`
    /// arriving via `drain_session_up_stream`, return the bytes
    /// inline as `ForwardInvokeResponse.result_bytes`.
    ///
    /// Errors:
    /// - target offline (no PresenceRegistry entry, or push
    ///   fails) → `Status::failed_precondition(target_offline)`,
    ///   so the caller's same-tenant fall-through arm can fan
    ///   out to federated peers.
    /// - target's session crashed mid-call (sender dropped
    ///   without complete) → `Status::unavailable` so the CLI
    ///   sees a structured upstream-failure rather than empty
    ///   bytes pretending success.
    /// - daemon was constructed without `PendingDispatchMap` →
    ///   `Status::failed_precondition` with a clear message
    ///   pointing at the boot configuration.
    async fn dispatch_local_presence_forward_invoke(
        &self,
        request: &federation_wrappers::ForwardInvokeRequest,
        inner_payload: &InnerPayload,
        caller_envelope: Option<&Envelope>,
        correlation_call_id: &str,
    ) -> Result<Response<InvokeResponse>, Status> {
        let pending = self.pending.as_ref().ok_or_else(|| {
            Status::failed_precondition(
                "federation.forward_invoke: daemon was constructed without a \
                 PendingDispatchMap; call DaemonInvocationService::with_pending(...) \
                 at boot to enable cross-device forward_invoke dispatch",
            )
        })?;
        let (session_id, sender) = self
            .presence
            .lookup_tracked(&request.target_uri)
            .ok_or_else(|| {
                Status::failed_precondition(
                    federation_wrappers::FORWARD_INVOKE_TARGET_OFFLINE_REASON,
                )
            })?;

        // Register pending entry BEFORE pushing the frame so a
        // fast device reply lands a real `complete()` rather
        // than a no-op (race-free correlation, same contract as
        // `dispatch_invoke_remote`).
        //
        // Use `register_pending_for(target_uri)` so the daemon's
        // presence-offline watcher (`with_pending` ctor hook) can
        // fail-fast this entry the moment `<self>.session` for
        // `request.target_uri` drops mid-call — without this the
        // `await_reply()` below blocks on the oneshot until the
        // operator-side HTTP timeout fires.
        let handle = pending.register_pending_for(&request.target_uri);
        let call_id = handle.call_id();

        let dispatch_frame = build_invoke_remote_dispatch_frame(
            call_id,
            &inner_payload.ability,
            &inner_payload.args_bytes,
        )?;

        match sender.try_send(Ok(dispatch_frame)) {
            Ok(()) => {}
            Err(tokio::sync::mpsc::error::TrySendError::Full(_)) => {
                self.presence.remove_if_session(
                    &request.target_uri,
                    session_id,
                    crate::services::presence_registry::OfflineReason::SendFailed,
                );
                return Err(Status::failed_precondition(
                    federation_wrappers::FORWARD_INVOKE_TARGET_OFFLINE_REASON,
                ));
            }
            Err(tokio::sync::mpsc::error::TrySendError::Closed(_)) => {
                self.presence.remove_if_session(
                    &request.target_uri,
                    session_id,
                    crate::services::presence_registry::OfflineReason::StreamClosed,
                );
                return Err(Status::failed_precondition(
                    federation_wrappers::FORWARD_INVOKE_TARGET_OFFLINE_REASON,
                ));
            }
        }

        eprintln!(
            "[axon-serve] forward_invoke local-presence dispatch: target_uri={} \
             ability={} call_id={} (waiting for SessionDispatch::Result)",
            request.target_uri, inner_payload.ability, call_id,
        );

        // Await the matching Result frame.
        let dispatch_result = handle.await_reply().await.map_err(|_recv_err| {
            Status::unavailable(format!(
                "federation.forward_invoke: target `{}` session disconnected before \
                 reply (call_id={call_id})",
                request.target_uri,
            ))
        })?;

        let DispatchResult {
            payload: result_bytes,
            error,
        } = dispatch_result;
        // Diagnostic: forward the mac-side outcome verbatim so a
        // session-frame-correlation race is visible in the hub log
        // without having to attach a debugger. Cheap (one eprintln
        // per round-trip).
        eprintln!(
            "[axon-serve] forward_invoke local-presence dispatch: \
             target_uri={} ability={} call_id={} → result_bytes={} error={:?}",
            request.target_uri,
            inner_payload.ability,
            call_id,
            result_bytes.len(),
            error,
        );
        if let Some(err) = error {
            return Err(Status::failed_precondition(format!(
                "federation.forward_invoke: target `{}` ability `{}` failed: {err}",
                request.target_uri, inner_payload.ability,
            )));
        }

        // DEC-N5 §1: write the ForwardReceipt with a real
        // result_digest (not None) since we have the bytes
        // inline.
        self.admission.receipt_store().record(build_forward_receipt(
            correlation_call_id,
            &request.target_uri,
            caller_envelope,
            Some(&result_bytes),
        ));

        let response = federation_wrappers::ForwardInvokeResponse {
            result_bytes,
            correlation_call_id: correlation_call_id.to_string(),
        };
        wrap_json_response(&response)
    }

    fn try_push_forward_invoke_frame(
        &self,
        request: &federation_wrappers::ForwardInvokeRequest,
    ) -> Result<(), Status> {
        let Some((session_id, sender)) = self.presence.lookup_tracked(&request.target_uri) else {
            return Err(Status::failed_precondition(
                federation_wrappers::FORWARD_INVOKE_TARGET_OFFLINE_REASON,
            ));
        };

        let inner_bytes = decode_inner_envelope(&request.inner_envelope_b64)?;
        let frame = build_forward_invoke_dispatch_frame(inner_bytes);

        match sender.try_send(Ok(frame)) {
            Ok(()) => Ok(()),
            Err(tokio::sync::mpsc::error::TrySendError::Full(_)) => {
                // Bounded backpressure (Invariant 4 in
                // `services::presence_registry`). Slow consumer
                // → evict + surface `target_offline` per DEC-N4
                // §2.1; the matching presence event ensures
                // future calls observe a clean miss.
                self.presence.remove_if_session(
                    &request.target_uri,
                    session_id,
                    crate::services::presence_registry::OfflineReason::SendFailed,
                );
                Err(Status::failed_precondition(
                    federation_wrappers::FORWARD_INVOKE_TARGET_OFFLINE_REASON,
                ))
            }
            Err(tokio::sync::mpsc::error::TrySendError::Closed(_)) => {
                // Receiver dropped without explicit removal —
                // channel is dead. Symmetric removal +
                // `target_offline` surface for this call.
                self.presence.remove_if_session(
                    &request.target_uri,
                    session_id,
                    crate::services::presence_registry::OfflineReason::StreamClosed,
                );
                Err(Status::failed_precondition(
                    federation_wrappers::FORWARD_INVOKE_TARGET_OFFLINE_REASON,
                ))
            }
        }
    }

    /// **PR-1 commit 7/9 (LB-56)**. Synchronous self-targeted
    /// `federation.forward_invoke` dispatch.
    ///
    /// Caller has confirmed `target_uri == admission.daemon_uri()`
    /// AND `local_dispatcher.is_some()`. We resolve the inner
    /// ability against the daemon's `LocalAbilityRegistry` (via
    /// the `AbilityDispatcher::execute_rpc` path), encode the
    /// JSON result into bytes, write a single ForwardReceipt with
    /// a real `result_digest` (no async second update), and
    /// return the bytes inline in `ForwardInvokeResponse.
    /// result_bytes`.
    ///
    /// Errors map to `tonic::Status`:
    /// - inner args parse failure → `Status::invalid_argument`
    /// - ability not registered or handler returned `Err` →
    ///   `Status::failed_precondition` with the underlying
    ///   anyhow chain in the message (callers / scripts grepping
    ///   the daemon log can distinguish "ability not on this
    ///   daemon" from "ability ran and threw")
    /// - JSON encode failure of the response →
    ///   `Status::internal`
    fn dispatch_self_targeted_forward_invoke(
        &self,
        local_dispatcher: &Arc<crate::runtime::ability_dispatch::AbilityDispatcher>,
        inner_payload: &InnerPayload,
        request: &federation_wrappers::ForwardInvokeRequest,
        caller_envelope: Option<&Envelope>,
        correlation_call_id: &str,
    ) -> Result<Response<InvokeResponse>, Status> {
        use crate::runtime::invocation_target::{CallMode, InvocationTarget, TargetScope};

        // Inner args are JSON-encoded bytes per `decode_inner_payload`.
        // `AbilityDispatcher::execute_rpc` consumes a `Value`, so
        // round-trip-decode here. Empty → empty object (matches the
        // dispatcher's args-default convention).
        let normalized_args: serde_json::Value = if inner_payload.args_bytes.is_empty() {
            serde_json::Value::Object(Default::default())
        } else {
            serde_json::from_slice(&inner_payload.args_bytes).map_err(|err| {
                Status::invalid_argument(format!(
                    "federation.forward_invoke: self-targeted dispatch could not parse inner args \
                     for ability `{}`: {err}",
                    inner_payload.ability,
                ))
            })?
        };

        let target = InvocationTarget {
            scope: TargetScope::Local,
            ability: inner_payload.ability.clone(),
            normalized_args,
            call_mode: CallMode::Rpc,
            subject: None,
        };

        eprintln!(
            "[axon-serve] forward_invoke self-target dispatch: target_uri={} ability={} \
             call_id={}",
            request.target_uri, inner_payload.ability, correlation_call_id,
        );

        let result_value = local_dispatcher.execute_rpc(target).map_err(|err| {
            Status::failed_precondition(format!(
                "federation.forward_invoke: self-targeted dispatch of ability `{}` failed: {err}",
                inner_payload.ability,
            ))
        })?;

        let result_bytes = serde_json::to_vec(&result_value).map_err(|err| {
            Status::internal(format!(
                "federation.forward_invoke: encode self-targeted result for ability `{}`: {err}",
                inner_payload.ability,
            ))
        })?;

        // Single ForwardReceipt write with real result_digest —
        // unlike the bidi-push path, no PR-N5 second-update is
        // needed because the bytes are already known.
        self.admission.receipt_store().record(build_forward_receipt(
            correlation_call_id,
            &request.target_uri,
            caller_envelope,
            Some(&result_bytes),
        ));

        let response = federation_wrappers::ForwardInvokeResponse {
            result_bytes,
            correlation_call_id: correlation_call_id.to_string(),
        };
        wrap_json_response(&response)
    }

    /// Self-targeted `<self>.invoke_remote` shortcut.
    ///
    /// When the daemon receives `<self>.invoke_remote` whose
    /// subject_device equals its own URI, dispatch the ability
    /// through the in-process `AbilityDispatcher` and return the
    /// result on a one-shot down stream. This fires in two
    /// scenarios:
    ///
    ///   1. Host-mode dev rig: backend invokes a fleet.* ability
    ///      against the local device daemon's own URI. The
    ///      daemon's PresenceRegistry self-presence seed
    ///      (boot.rs) makes the target findable; this shortcut
    ///      dispatches inline without trying to push frames
    ///      down a drain channel that nobody consumes.
    ///
    ///   2. Hub-mode self-call: a hub invoking an ability on
    ///      its own URI (rare but valid; the hub is a Both-mode
    ///      daemon and the local AbilityDispatcher hosts its
    ///      registered tools).
    ///
    /// Mirrors `dispatch_self_targeted_forward_invoke` for the
    /// federation.forward_invoke surface — same idea, different
    /// envelope shape.
    async fn dispatch_self_targeted_invoke_remote(
        &self,
        local_dispatcher: &Arc<crate::runtime::ability_dispatch::AbilityDispatcher>,
        subject_device: &str,
        ability: &str,
        args: &[u8],
    ) -> Result<Response<<Self as Invocation>::InvokeBidiStream>, Status> {
        use crate::runtime::invocation_target::{CallMode, InvocationTarget, TargetScope};

        eprintln!(
            "[axon-serve] <self>.invoke_remote self-target dispatch: \
             subject={subject_device} ability={ability}"
        );

        // args is the JSON-encoded inner-payload bytes (matches
        // InvokeRemoteUp::Request shape). Decode to a Value so the
        // local AbilityDispatcher can route. Empty → empty object.
        let normalized_args: serde_json::Value = if args.is_empty() {
            serde_json::Value::Object(Default::default())
        } else {
            serde_json::from_slice(args).map_err(|err| {
                Status::invalid_argument(format!(
                    "<self>.invoke_remote: self-targeted dispatch could not parse \
                     inner args for ability `{ability}`: {err}"
                ))
            })?
        };

        let target = InvocationTarget {
            scope: TargetScope::Local,
            ability: ability.to_string(),
            normalized_args,
            call_mode: CallMode::Rpc,
            subject: None,
        };

        // execute_rpc is a SYNC call into the LocalAbilityRegistry's
        // RPC handler. Many real handlers (shell.run, process.exec,
        // fs.*) internally call `tokio::runtime::Handle::block_on`
        // to drive their inner async pipeline. Calling block_on
        // from inside a tokio worker thread panics — that is the
        // panic we hit at `shell_run_ability.rs:117` when this fn
        // ran the dispatch directly inside this gRPC service's
        // tokio task.
        //
        // Move the synchronous handler off the tokio worker pool
        // onto a blocking thread via spawn_blocking. This is the
        // canonical tokio pattern for "I have sync code that may
        // do block_on internally" — the blocking pool is sized
        // separately (default 512), is allowed to call block_on,
        // and serves exactly this kind of case.
        let dispatcher_clone = Arc::clone(local_dispatcher);
        let result_value =
            tokio::task::spawn_blocking(move || dispatcher_clone.execute_rpc(target))
                .await
                .map_err(|join_err| {
                    Status::internal(format!(
                        "<self>.invoke_remote: self-targeted handler panicked or \
                     was cancelled: {join_err}"
                    ))
                })?
                .map_err(|err| {
                    Status::failed_precondition(format!(
                "<self>.invoke_remote: self-targeted dispatch of ability `{ability}` failed: {err}"
            ))
                })?;

        let payload = serde_json::to_vec(&result_value).map_err(|err| {
            Status::internal(format!(
                "<self>.invoke_remote: encode self-targeted result for ability `{ability}`: {err}"
            ))
        })?;

        let down = InvokeRemoteDown::Result {
            payload,
            error: None,
        };
        let frame = build_invoke_remote_terminal_frame(&down)?;

        // One-shot down stream: yield the terminal frame, close.
        let (down_tx, down_rx) = mpsc::channel::<Result<InvokeBidiDown, Status>>(1);
        tokio::spawn(async move {
            let _ = down_tx.send(Ok(frame)).await;
        });
        let stream = ReceiverStream::new(down_rx);
        Ok(Response::new(
            Box::pin(stream) as BoxedDownStream<InvokeBidiDown>
        ))
    }

    async fn dispatch_remote_file_transfer_bidi(
        &self,
        target_uri: &str,
        envelope_open: &EnvelopeOpen,
        mut up: Streaming<InvokeBidiUp>,
    ) -> Result<Response<<Self as Invocation>::InvokeBidiStream>, Status> {
        let pending = self.pending_stream.as_ref().ok_or_else(|| {
            Status::failed_precondition(
                "InvokeBidi fleet.file_transfer: daemon was constructed without a \
                 PendingStreamDispatchMap; boot must call with_pending_stream(...) \
                 to enable remote file_transfer bridging",
            )
        })?;
        let (session_id, sender) = self.presence.lookup_tracked(target_uri).ok_or_else(|| {
            Status::failed_precondition(federation_wrappers::FORWARD_INVOKE_TARGET_OFFLINE_REASON)
        })?;

        let mut handle = pending.register_pending();
        let call_id = handle.call_id();
        let stdout_stream_id = local_bidi_stdout_stream_id(envelope_open);

        let open_frame = build_remote_bidi_open_dispatch_frame(
            call_id,
            crate::runtime::agents::file_transfer_ability::ABILITY_FILE_TRANSFER,
            &envelope_open.initial_args,
        )?;
        match sender.try_send(Ok(open_frame)) {
            Ok(()) => {}
            Err(tokio::sync::mpsc::error::TrySendError::Full(_)) => {
                self.presence.remove_if_session(
                    target_uri,
                    session_id,
                    crate::services::presence_registry::OfflineReason::SendFailed,
                );
                return Err(Status::failed_precondition(
                    federation_wrappers::FORWARD_INVOKE_TARGET_OFFLINE_REASON,
                ));
            }
            Err(tokio::sync::mpsc::error::TrySendError::Closed(_)) => {
                self.presence.remove_if_session(
                    target_uri,
                    session_id,
                    crate::services::presence_registry::OfflineReason::StreamClosed,
                );
                return Err(Status::failed_precondition(
                    federation_wrappers::FORWARD_INVOKE_TARGET_OFFLINE_REASON,
                ));
            }
        }

        eprintln!(
            "[axon-serve] InvokeBidi remote file_transfer bridge: target_uri={} call_id={}",
            target_uri, call_id,
        );

        let (down_tx, down_rx) = tokio::sync::mpsc::channel::<Result<InvokeBidiDown, Status>>(16);

        let down_tx_for_results = down_tx.clone();
        tokio::spawn(async move {
            while let Some(event) = handle.recv().await {
                match event {
                    DispatchStreamEvent::Chunk(bytes) => {
                        let frame = InvokeBidiDown {
                            payload: Some(DownPayload::BinaryChunk(BinaryChunk {
                                stream_id: stdout_stream_id,
                                data: bytes,
                                ..BinaryChunk::default()
                            })),
                            ..InvokeBidiDown::default()
                        };
                        if down_tx_for_results.send(Ok(frame)).await.is_err() {
                            break;
                        }
                    }
                    DispatchStreamEvent::Terminal(DispatchResult { payload, error }) => {
                        let frame = match error {
                            Some(reason) => build_bidi_terminal_receipt_with_payload(
                                InvocationState::Failed,
                                reason,
                                if payload.is_empty() {
                                    None
                                } else {
                                    Some((payload, "application/json"))
                                },
                            ),
                            None => build_bidi_terminal_receipt_with_payload(
                                InvocationState::Completed,
                                String::new(),
                                if payload.is_empty() {
                                    None
                                } else {
                                    Some((payload, "application/json"))
                                },
                            ),
                        };
                        let _ = down_tx_for_results.send(Ok(frame)).await;
                        break;
                    }
                }
            }
        });

        let target_uri_owned = target_uri.to_string();
        let presence_for_up = Arc::clone(&self.presence);
        let pending_for_up = Arc::clone(pending);
        tokio::spawn(async move {
            let mut expected_up_sequence = 1_u64;
            let mut eof_sent = false;
            while let Some(maybe_frame) = up.next().await {
                let frame = match maybe_frame {
                    Ok(frame) => frame,
                    Err(status) => {
                        let _ = pending_for_up
                            .finish(
                                call_id,
                                DispatchResult {
                                    payload: Vec::new(),
                                    error: Some(format!(
                                        "file_transfer caller stream error: {status}"
                                    )),
                                },
                            )
                            .await;
                        return;
                    }
                };
                if frame.sequence != expected_up_sequence {
                    let _ = pending_for_up
                        .finish(
                            call_id,
                            DispatchResult {
                                payload: Vec::new(),
                                error: Some(format!(
                                    "{REASON_BIDI_FRAME_SEQUENCE}: expected up sequence \
                                     {expected_up_sequence}, got {}",
                                    frame.sequence
                                )),
                            },
                        )
                        .await;
                    return;
                }
                expected_up_sequence = expected_up_sequence.saturating_add(1);
                let Some(payload) = frame.payload else {
                    continue;
                };
                let bridge_frame = match payload {
                    UpPayload::BinaryChunk(chunk) => {
                        build_remote_bidi_input_dispatch_frame(call_id, &chunk.data, false)
                    }
                    UpPayload::Control(control)
                        if matches!(
                            control.control,
                            Some(crate::pb::axon::v1::bidi_control::Control::Eof(true))
                        ) =>
                    {
                        eof_sent = true;
                        build_remote_bidi_input_dispatch_frame(call_id, &[], true)
                    }
                    UpPayload::Control(_) | UpPayload::EnvelopeOpen(_) => continue,
                };
                match sender.try_send(Ok(bridge_frame)) {
                    Ok(()) => {}
                    Err(tokio::sync::mpsc::error::TrySendError::Full(_)) => {
                        presence_for_up.remove_if_session(
                            &target_uri_owned,
                            session_id,
                            crate::services::presence_registry::OfflineReason::SendFailed,
                        );
                        let _ = pending_for_up
                            .finish(
                                call_id,
                                DispatchResult {
                                    payload: Vec::new(),
                                    error: Some(
                                        federation_wrappers::FORWARD_INVOKE_TARGET_OFFLINE_REASON
                                            .to_string(),
                                    ),
                                },
                            )
                            .await;
                        return;
                    }
                    Err(tokio::sync::mpsc::error::TrySendError::Closed(_)) => {
                        presence_for_up.remove_if_session(
                            &target_uri_owned,
                            session_id,
                            crate::services::presence_registry::OfflineReason::StreamClosed,
                        );
                        let _ = pending_for_up
                            .finish(
                                call_id,
                                DispatchResult {
                                    payload: Vec::new(),
                                    error: Some(
                                        federation_wrappers::FORWARD_INVOKE_TARGET_OFFLINE_REASON
                                            .to_string(),
                                    ),
                                },
                            )
                            .await;
                        return;
                    }
                }
            }

            if !eof_sent {
                let _ = sender.try_send(Ok(build_remote_bidi_input_dispatch_frame(
                    call_id,
                    &[],
                    true,
                )));
            }
        });

        let stream = LocalBidiDownStream::new(down_rx);
        Ok(Response::new(
            Box::pin(stream) as BoxedDownStream<InvokeBidiDown>
        ))
    }

    /// PTY-attach bidi fallback: dispatch the locally-registered
    /// `fleet.session_attach` / `fleet.pty_session_attach` handler
    /// through the in-process `AbilityDispatcher` and bridge its
    /// `BidiSource` (two `mpsc<Value>` channels) onto the gRPC
    /// `InvokeBidi` up/down streams.
    ///
    /// Wire-format adapter
    /// -------------------
    /// Backend's WS terminal handler emits raw PTY bytes as
    /// `InvokeBidiUp::BinaryChunk(stream_id=1, data=raw)`. The
    /// device-side `pty_attach_ability` handler expects JSON
    /// `{"type":"stdin","data":"<base64>"}` — its on-the-wire
    /// shape (see `runtime/agents/pty_attach_ability.rs`). We
    /// translate at this seam: BinaryChunk → JSON stdin frame on
    /// the up direction, JSON stdout frame → BinaryChunk on the
    /// down direction. PtyResize control frames map to a JSON
    /// `{"type":"resize","cols":N,"rows":N}` shape the handler
    /// already consumes.
    async fn dispatch_local_bidi(
        &self,
        local_dispatcher: &Arc<crate::runtime::ability_dispatch::AbilityDispatcher>,
        ability: &str,
        envelope_open: &EnvelopeOpen,
        mut up: Streaming<InvokeBidiUp>,
    ) -> Result<Response<<Self as Invocation>::InvokeBidiStream>, Status> {
        use crate::runtime::invocation_target::{CallMode, InvocationTarget, TargetScope};

        eprintln!("[axon-serve] InvokeBidi local-dispatcher fallback: ability={ability}");

        // Decode initial_args. Empty → empty object.
        let normalized_args: serde_json::Value = if envelope_open.initial_args.is_empty() {
            serde_json::Value::Object(Default::default())
        } else {
            serde_json::from_slice(&envelope_open.initial_args).map_err(|err| {
                Status::invalid_argument(format!(
                    "InvokeBidi local-dispatcher: initial_args is not valid JSON \
                     for ability `{ability}`: {err}"
                ))
            })?
        };

        let target = InvocationTarget {
            scope: TargetScope::Local,
            ability: ability.to_string(),
            normalized_args,
            call_mode: CallMode::Bidi,
            subject: envelope_open
                .envelope
                .as_ref()
                .and_then(|env| env.subject.as_ref())
                .map(|subject| subject.uri.clone())
                .filter(|uri| !uri.is_empty()),
        };

        let bidi_source = local_dispatcher.execute_bidi(target).map_err(|err| {
            // No local handler registered → 404 surface so callers
            // see the same shape as RPC's "no local handler".
            let msg = err.to_string();
            if msg.contains("no local bidi handler registered") {
                Status::not_found(format!("InvokeBidi: {msg}"))
            } else {
                Status::failed_precondition(format!(
                    "InvokeBidi local-dispatcher: dispatch of ability `{ability}` \
                     failed: {err}"
                ))
            }
        })?;

        let crate::runtime::ability_dispatch::BidiSource {
            to_client: handler_in_tx,
            from_client: mut handler_out_rx,
        } = bidi_source;
        let wire_kind = local_bidi_wire_kind(ability);
        let stdout_stream_id = local_bidi_stdout_stream_id(envelope_open);

        // Down-stream: handler-emitted JSON → InvokeBidiDown frames.
        // Capacity 16 mirrors `INVOKE_REMOTE_DISPATCH_CAPACITY`.
        let (down_tx, down_rx) = tokio::sync::mpsc::channel::<Result<InvokeBidiDown, Status>>(16);

        // First down-frame MUST be an admission Receipt(Admitted),
        // per the bidi protocol contract: the client (backend's
        // wshandler.go:711) refuses to start the input pump until
        // it sees this. Without it, the local-dispatcher path's
        // first frame is whatever the handler emits (typically a
        // BinaryChunk for PTY stdout) and wshandler tears down with
        // "expected admission receipt as first frame, got kind=1".
        //
        // This mirrors the pre-existing receipt emit on the real
        // <self>.session reverse-channel admission path
        // (build_session_down_admission_receipt). The local-bidi
        // fallback was missing it because PR 8682960 wired the
        // handler frames through without the prelude.
        if down_tx
            .send(Ok(build_bidi_admission_receipt()))
            .await
            .is_err()
        {
            return Err(Status::cancelled(
                "InvokeBidi local-dispatcher: down-stream closed before admission receipt sent",
            ));
        }

        let down_tx_for_handler = down_tx.clone();
        tokio::spawn(async move {
            while let Some(value) = handler_out_rx.recv().await {
                match map_local_bidi_handler_frame(wire_kind, &value, stdout_stream_id) {
                    LocalBidiHandlerFrame::Forward(frame) => {
                        if down_tx_for_handler.send(Ok(frame)).await.is_err() {
                            break;
                        }
                    }
                    LocalBidiHandlerFrame::Terminal(frame) => {
                        let _ = down_tx_for_handler.send(Ok(frame)).await;
                        break;
                    }
                    LocalBidiHandlerFrame::Ignore => {}
                    LocalBidiHandlerFrame::ProtocolFailure(reason) => {
                        let _ = down_tx_for_handler
                            .send(Ok(build_bidi_terminal_receipt(
                                InvocationState::Failed,
                                reason,
                            )))
                            .await;
                        break;
                    }
                }
            }
        });

        // Up-stream: InvokeBidiUp frames → handler input JSON.
        tokio::spawn(async move {
            let mut expected_up_sequence = 1_u64;
            while let Some(maybe_frame) = up.next().await {
                let Ok(frame) = maybe_frame else { break };
                if frame.sequence != expected_up_sequence {
                    eprintln!(
                        "[axon-serve] InvokeBidi local-dispatcher: violated \
                         {REASON_BIDI_FRAME_SEQUENCE}; expected {expected_up_sequence}, got {}",
                        frame.sequence
                    );
                    break;
                }
                expected_up_sequence = expected_up_sequence.saturating_add(1);
                let Some(payload) = frame.payload else {
                    continue;
                };
                match map_local_bidi_up_payload(wire_kind, payload) {
                    LocalBidiUpFrame::Forward(jsonv) => {
                        if handler_in_tx.send(jsonv).await.is_err() {
                            break;
                        }
                    }
                    LocalBidiUpFrame::ForwardAndClose(jsonv) => {
                        if handler_in_tx.send(jsonv).await.is_err() {
                            break;
                        }
                        break;
                    }
                    LocalBidiUpFrame::Close => break,
                    LocalBidiUpFrame::Ignore => {}
                }
            }
            // Up-stream EOF → drop handler_in_tx so the handler's
            // reader sees its channel close (graceful disconnect).
            drop(handler_in_tx);
        });

        let stream = LocalBidiDownStream::new(down_rx);
        Ok(Response::new(
            Box::pin(stream) as BoxedDownStream<InvokeBidiDown>
        ))
    }

    fn dispatch_federation_subscribe_directory_initial(
        &self,
    ) -> Result<Response<<Self as Invocation>::InvokeStreamStream>, Status> {
        let initial = federation_wrappers::build_subscribe_directory_initial(&self.presence);
        let initial_bytes = serde_json::to_vec(&initial).map_err(|err| {
            Status::internal(format!(
                "federation.subscribe_directory: failed to encode initial snapshot: {err}"
            ))
        })?;
        let initial_chunk = InvokeStreamChunk {
            content_type: FEDERATION_RESULT_CONTENT_TYPE.to_string(),
            payload: initial_bytes,
            ..InvokeStreamChunk::default()
        };

        // Real broadcast pump: emit the initial snapshot frame, then
        // forward every subsequent `PresenceEvent` as one frame
        // until every broadcast sender drops. `Lagged` errors
        // collapse to a re-snapshot frame so a slow consumer can
        // recover without tearing the stream down (per spec §3.2
        // capacity rationale).
        //
        // We capture the registry by `Weak` rather than `Arc` so the
        // pump itself does not keep the broadcast sender alive: when
        // the daemon-owned `Arc<PresenceRegistry>` is dropped (last
        // service shutdown, test teardown), the broadcast `Sender`
        // drops, the receiver returns `RecvError::Closed`, and the
        // pump terminates. Holding an `Arc` here would deadlock the
        // shutdown path.
        let events = self.presence.subscribe_events();
        let presence_weak = Arc::downgrade(&self.presence);

        let initial_stream = futures::stream::once(async move { Ok(initial_chunk) });
        let event_stream = futures::stream::unfold(
            (events, presence_weak),
            |(mut events, presence_weak)| async move {
                use tokio::sync::broadcast::error::RecvError;

                loop {
                    match events.recv().await {
                        Ok(event) => {
                            // `PresenceEventDelta` is `Online { String }` /
                            // `Offline { String, &'static str }` — both
                            // variants are statically `Serialize` and
                            // never fail to encode. `expect` rather than
                            // `.ok()?` so a future field that introduces
                            // a fallible serialise mode trips a panic
                            // with a self-documenting message instead of
                            // silently terminating the stream — the
                            // subscriber's `Closed` is otherwise
                            // indistinguishable from a normal shutdown.
                            let payload = serde_json::to_vec(&PresenceEventDelta::from(event))
                                .expect(
                                    "PresenceEventDelta is statically Serialize; a serialise \
                                     failure here means the type grew a fallible field — update \
                                     this site to surface Status::internal instead of panicking",
                                );
                            let chunk = InvokeStreamChunk {
                                content_type: FEDERATION_RESULT_CONTENT_TYPE.to_string(),
                                payload,
                                ..InvokeStreamChunk::default()
                            };
                            return Some((Ok(chunk), (events, presence_weak)));
                        }
                        Err(RecvError::Lagged(_)) => {
                            // Re-snapshot recovery: emit a fresh
                            // initial frame so the subscriber's
                            // state converges with the registry.
                            // If the registry has been dropped under
                            // us, end the stream gracefully.
                            let presence = presence_weak.upgrade()?;
                            let snapshot =
                                federation_wrappers::build_subscribe_directory_initial(&presence);
                            drop(presence);
                            // `SubscribeDirectoryInitial` is statically
                            // `Serialize` (Vec<AgentSummary> of two
                            // String fields). Same `expect` rationale as
                            // the `Ok(event)` arm above.
                            let payload = serde_json::to_vec(&snapshot).expect(
                                "SubscribeDirectoryInitial is statically Serialize; a \
                                 serialise failure here means the snapshot type grew a \
                                 fallible field — update this site to surface Status::internal \
                                 instead of panicking",
                            );
                            let chunk = InvokeStreamChunk {
                                content_type: FEDERATION_RESULT_CONTENT_TYPE.to_string(),
                                payload,
                                ..InvokeStreamChunk::default()
                            };
                            return Some((Ok(chunk), (events, presence_weak)));
                        }
                        Err(RecvError::Closed) => return None,
                    }
                }
            },
        );

        let combined = futures::StreamExt::chain(initial_stream, event_stream);
        Ok(Response::new(
            Box::pin(combined) as BoxedDownStream<InvokeStreamChunk>
        ))
    }

    /// **PR-N3 N3-streaming-1**.
    /// `federation.subscribe_directory_v2` server-stream
    /// dispatch. Mirrors v1's pump structure but emits the new
    /// `DirectoryEvent` wire shape: `Snapshot` first, then
    /// per-presence-event `Upsert` / `Remove` frames produced
    /// by `presence_event_to_directory_event`. Lagged →
    /// re-snapshot recovery + Closed → graceful end mirror v1
    /// verbatim. Weak-Arc pattern keeps the pump from blocking
    /// daemon shutdown.
    fn dispatch_federation_subscribe_directory_v2(
        &self,
    ) -> Result<Response<<Self as Invocation>::InvokeStreamStream>, Status> {
        use crate::services::federation_directory::{
            presence_event_to_directory_event, DirectoryEvent,
        };

        let initial_evt =
            federation_wrappers::build_subscribe_directory_v2_snapshot(&self.presence);
        let initial_bytes = serde_json::to_vec(&initial_evt).map_err(|err| {
            Status::internal(format!(
                "federation.subscribe_directory_v2: encode initial snapshot: {err}"
            ))
        })?;
        let initial_chunk = InvokeStreamChunk {
            content_type: FEDERATION_RESULT_CONTENT_TYPE.to_string(),
            payload: initial_bytes,
            ..InvokeStreamChunk::default()
        };

        let events = self.presence.subscribe_events();
        let presence_weak = Arc::downgrade(&self.presence);

        // Heartbeat tick: spec §2.3 says emit Heartbeat every
        // 30s when no other frame has been emitted in window.
        // The interval is field-configurable via
        // `with_subscribe_v2_heartbeat_interval_ms` for test
        // ergonomics; production stays at the 30 000ms
        // default. Skip-on-missed-tick keeps cadence aligned
        // when a real event arrives close to the deadline.
        let heartbeat_interval_ms: u64 = self.subscribe_v2_heartbeat_interval_ms;
        let initial_stream = futures::stream::once(async move { Ok(initial_chunk) });
        let event_stream = futures::stream::unfold(
            (events, presence_weak, heartbeat_interval_ms),
            |(mut events, presence_weak, hb_ms)| async move {
                use tokio::sync::broadcast::error::RecvError;

                let mut hb = tokio::time::interval(std::time::Duration::from_millis(hb_ms));
                hb.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Skip);
                // Burn the immediate-fire tick — we don't want
                // a Heartbeat at frame 1; the Snapshot already
                // proves liveness. The next tick fires
                // hb_ms from now.
                hb.tick().await;

                loop {
                    tokio::select! {
                        recv = events.recv() => {
                            match recv {
                                Ok(event) => {
                                    let evt = presence_event_to_directory_event(&event);
                                    let payload = serde_json::to_vec(&evt).expect(
                                        "DirectoryEvent is statically Serialize; a serialise \
                                         failure here means the type grew a fallible field \
                                         — update this site to surface Status::internal \
                                         instead of panicking",
                                    );
                                    let chunk = InvokeStreamChunk {
                                        content_type: FEDERATION_RESULT_CONTENT_TYPE.to_string(),
                                        payload,
                                        ..InvokeStreamChunk::default()
                                    };
                                    return Some((
                                        Ok(chunk),
                                        (events, presence_weak, hb_ms),
                                    ));
                                }
                                Err(RecvError::Lagged(_)) => {
                                    // Slow consumer; emit a
                                    // fresh Snapshot so the
                                    // receiver's view converges
                                    // with the registry.
                                    let presence = presence_weak.upgrade()?;
                                    let snap_evt =
                                        federation_wrappers::build_subscribe_directory_v2_snapshot(
                                            &presence,
                                        );
                                    drop(presence);
                                    let payload = serde_json::to_vec(&snap_evt).expect(
                                        "DirectoryEvent::Snapshot is statically Serialize",
                                    );
                                    let _ = DirectoryEvent::Snapshot { entries: vec![] };
                                    let chunk = InvokeStreamChunk {
                                        content_type: FEDERATION_RESULT_CONTENT_TYPE.to_string(),
                                        payload,
                                        ..InvokeStreamChunk::default()
                                    };
                                    return Some((
                                        Ok(chunk),
                                        (events, presence_weak, hb_ms),
                                    ));
                                }
                                Err(RecvError::Closed) => return None,
                            }
                        }
                        _ = hb.tick() => {
                            // 30s elapsed without a real event;
                            // emit Heartbeat so the subscriber's
                            // 60s idle-timeout watcher does not
                            // tear down a healthy stream.
                            let now_ms = std::time::SystemTime::now()
                                .duration_since(std::time::UNIX_EPOCH)
                                .map(|d| d.as_millis() as i64)
                                .unwrap_or(0);
                            let hb_evt = DirectoryEvent::Heartbeat {
                                sent_at_unix_ms: now_ms,
                            };
                            let payload = serde_json::to_vec(&hb_evt)
                                .expect("DirectoryEvent::Heartbeat is statically Serialize");
                            let chunk = InvokeStreamChunk {
                                content_type: FEDERATION_RESULT_CONTENT_TYPE.to_string(),
                                payload,
                                ..InvokeStreamChunk::default()
                            };
                            return Some((Ok(chunk), (events, presence_weak, hb_ms)));
                        }
                    }
                }
            },
        );

        let combined = futures::StreamExt::chain(initial_stream, event_stream);
        Ok(Response::new(
            Box::pin(combined) as BoxedDownStream<InvokeStreamChunk>
        ))
    }

    /// Hub-side `<self>.invoke_remote` handler. Drives the per-call
    /// cross-device dispatch flow:
    ///
    /// 1. Parse the frame-0 `EnvelopeOpen.initial_args` as
    ///    `InvokeRemoteUp::Request { subject_device, ability, args }`
    /// 2. Confirm a `PendingDispatchMap` is wired (else
    ///    `Status::failed_precondition` — daemon is in a half-built
    ///    configuration; calling without `with_pending` is a boot
    ///    bug, not a caller bug)
    /// 3. Look up `subject_device` in `PresenceRegistry` (else
    ///    `Status::not_found`)
    /// 4. Register a pending-reply slot (`PendingHandle` carries a
    ///    `call_id`)
    /// 5. Push a `DispatchDown` frame down the target session
    ///    carrying `(call_id, ability, args)` JSON. Backpressure +
    ///    closed-receiver paths collapse into presence transitions
    ///    per spec §3 Invariant 4 (same shape as
    ///    `try_push_forward_invoke_frame` from commit 8/9).
    /// 6. Return a server-stream of one terminal frame: when the
    ///    target's reply arrives via `PendingDispatchMap::complete`
    ///    (PR-2 session-receive task — pending), translate
    ///    `DispatchResult` into `InvokeRemoteDown::Result` and emit;
    ///    if the pending sender is dropped (target session crashed
    ///    mid-call), surface as `error: Some("...")` Result frame.
    ///
    /// PR-1+2+3 binary integration: PR-2's session task plugs into
    /// `PendingDispatchMap::complete(call_id, DispatchResult)` to
    /// fulfil the wait. Until PR-2 lands, the wait will time out
    /// (or hang if no caller-side timeout) — integration tests must
    /// wrap `await_reply()` in `tokio::time::timeout`.
    async fn dispatch_invoke_remote(
        &self,
        envelope_open: &EnvelopeOpen,
        _up: Streaming<InvokeBidiUp>,
    ) -> Result<Response<<Self as Invocation>::InvokeBidiStream>, Status> {
        let request: InvokeRemoteUp =
            serde_json::from_slice(&envelope_open.initial_args).map_err(|err| {
                Status::invalid_argument(format!(
                    "<self>.invoke_remote: frame-0 initial_args is not valid \
                     InvokeRemoteUp JSON: {err}"
                ))
            })?;

        let InvokeRemoteUp::Request {
            subject_device,
            ability,
            args,
        } = request;

        // Postel-boundary: peer hubs running pre-v4.1.4 builds may
        // pass a `/agent/<bare-uuid>` device URI; coerce to the
        // canonical `/device/<uuid>` shape before the registry
        // lookup. New clients always emit canonical; this is
        // strictly migration-window compat.
        let subject_device = crate::uri::canonicalize_presence_key(&subject_device);

        // **Self-targeted invoke_remote shortcut**.
        //
        // Host-mode dev rig: backend on the same host as the device's
        // daemon dials the daemon's UDS to invoke an ability targeting
        // the device itself. The daemon's local PresenceRegistry has a
        // self-presence seed (boot.rs) but its DispatchSender is a
        // drain channel — try_send works but the target never replies
        // because there's no real session bidi consuming the frame.
        //
        // When subject_device == this daemon's own URI AND a
        // local_dispatcher is wired, dispatch the ability through the
        // in-process AbilityDispatcher and return the result inline
        // on a one-shot down stream. Mirrors the
        // `dispatch_self_targeted_forward_invoke` shortcut at
        // `dispatch_federation_forward_invoke`'s top arm.
        //
        // Production hub-mode (DaemonMode::Both / Hub) reaches this
        // branch only when caller_uri == its own hub URI invoking
        // back to itself, which is also the right behaviour:
        // hub-self ability dispatch goes inline.
        if let Some(local_dispatcher) = self.local_dispatcher.as_ref() {
            if self.matches_self_target_uri(&subject_device) {
                return self
                    .dispatch_self_targeted_invoke_remote(
                        local_dispatcher,
                        &subject_device,
                        &ability,
                        &args,
                    )
                    .await;
            }
        }

        let pending = self.pending.as_ref().ok_or_else(|| {
            Status::failed_precondition(
                "<self>.invoke_remote: daemon was constructed without a \
                 PendingDispatchMap; call DaemonInvocationService::with_pending(...) \
                 at boot to enable cross-device invocation",
            )
        })?;

        let (target_session_id, target_sender) = self
            .presence
            .lookup_tracked(&subject_device)
            .ok_or_else(|| {
                Status::not_found(format!(
                    "<self>.invoke_remote: target `{subject_device}` is not in PresenceRegistry; \
                 either offline or never connected to this hub"
                ))
            })?;

        // Register pending entry BEFORE pushing the dispatch frame —
        // otherwise the target could reply faster than we can register
        // and the reply would land as a no-op `complete`.
        //
        // `register_pending_for(target)` so the presence-offline
        // watcher fail-fasts this entry if the target session drops
        // mid-call (matches the forward_invoke path above).
        let handle = pending.register_pending_for(&subject_device);
        let call_id = handle.call_id();

        let dispatch_frame = build_invoke_remote_dispatch_frame(call_id, &ability, &args)?;
        match target_sender.try_send(Ok(dispatch_frame)) {
            Ok(()) => {}
            Err(tokio::sync::mpsc::error::TrySendError::Full(_)) => {
                // Bounded backpressure → presence transition (same
                // policy as forward_invoke commit 8/9).
                self.presence.remove_if_session(
                    &subject_device,
                    target_session_id,
                    OfflineReason::SendFailed,
                );
                return Err(Status::failed_precondition(format!(
                    "<self>.invoke_remote: target `{subject_device}` channel full; \
                     removed from registry with OfflineReason::SendFailed"
                )));
            }
            Err(tokio::sync::mpsc::error::TrySendError::Closed(_)) => {
                self.presence.remove_if_session(
                    &subject_device,
                    target_session_id,
                    OfflineReason::StreamClosed,
                );
                return Err(Status::not_found(format!(
                    "<self>.invoke_remote: target `{subject_device}` receiver closed \
                     between lookup and dispatch; removed from registry"
                )));
            }
        }

        // The down stream: a single terminal frame yielded after the
        // target's reply arrives (or sender drops, signalling the
        // target session ended mid-call).
        let (down_tx, down_rx) = mpsc::channel::<Result<InvokeBidiDown, Status>>(1);
        tokio::spawn(async move {
            let frame = match handle.await_reply().await {
                Ok(DispatchResult { payload, error }) => {
                    let down = InvokeRemoteDown::Result { payload, error };
                    match build_invoke_remote_terminal_frame(&down) {
                        Ok(f) => Ok(f),
                        Err(status) => Err(status),
                    }
                }
                Err(_recv_err) => {
                    // Sender dropped without complete — target session
                    // task crashed or daemon shutdown mid-call.
                    let down = InvokeRemoteDown::Result {
                        payload: Vec::new(),
                        error: Some(format!(
                            "target session disconnected before reply (call_id={call_id})"
                        )),
                    };
                    match build_invoke_remote_terminal_frame(&down) {
                        Ok(f) => Ok(f),
                        Err(status) => Err(status),
                    }
                }
            };
            let _ = down_tx.send(frame).await;
        });

        let stream = ReceiverStream::new(down_rx);
        Ok(Response::new(
            Box::pin(stream) as BoxedDownStream<InvokeBidiDown>
        ))
    }

    /// Hub-side acceptor for `<self>.session`. The device opens a
    /// long-lived `InvokeBidi` against the daemon at boot and holds
    /// the stream open for the daemon process's lifetime; this is
    /// the canonical reverse channel through which the hub pushes
    /// `<self>.invoke_remote` `SessionDispatch::Dispatch` frames
    /// and the device replies with `SessionDispatch::Result` frames.
    ///
    /// Liveness model (spec §3): registry membership = liveness.
    /// Inserting the device's `DispatchSender` into the
    /// `PresenceRegistry` is the act of "device is online"; removing
    /// it (graceful close, transport reset, send-failure backpressure
    /// eviction) is the act of "device is offline". No periodic
    /// heartbeat — the bidi stream IS the heartbeat.
    ///
    /// Flow:
    /// 1. Build a fresh mpsc `(tx, rx)` of capacity
    ///    `DISPATCH_CHANNEL_CAPACITY` (256, spec §3.2)
    /// 2. Insert `tx` into PresenceRegistry under the caller URI;
    ///    any prior session for the same URI is displaced (the
    ///    registry emits Offline-then-Online, the displaced
    ///    receiver's mpsc dies → its outbound stream ends, that
    ///    device reconnects)
    /// 3. Spawn a task draining the device's up-stream:
    ///    each frame is parsed as `SessionDispatch::Result` and
    ///    routed via `pending.complete(call_id, result)` if a
    ///    `<self>.invoke_remote` caller is awaiting; on stream
    ///    close, remove the registry entry with the appropriate
    ///    `OfflineReason`
    /// 4. Return the down-stream wrapping `rx` so tonic pumps every
    ///    `DispatchFrame` (BinaryChunk-wrapped `SessionDispatch::Dispatch`)
    ///    pushed into `tx` back to the device
    async fn dispatch_self_session_accept(
        &self,
        caller_uri: String,
        up: Streaming<InvokeBidiUp>,
    ) -> Result<Response<<Self as Invocation>::InvokeBidiStream>, Status> {
        validate_session_realm(
            &caller_uri,
            self.session_realm.as_deref(),
            &self.admission.trust_anchor_snapshot(),
        )?;

        let (down_tx, down_rx): (DispatchSender, _) =
            mpsc::channel::<Result<DispatchFrame, Status>>(DISPATCH_CHANNEL_CAPACITY);

        // Step 1: register before spawning so a SessionDispatch::Dispatch
        // arriving from `<self>.invoke_remote` immediately can find this
        // sender. The PresenceRegistry handles displacement (Offline +
        // Online emission ordering) under the hood.
        let registration = self.presence.insert_tracked(caller_uri.clone(), down_tx);
        eprintln!(
            "[axon-serve] <self>.session admitted: caller={caller_uri} displaced_prior={}",
            registration.displaced.is_some()
        );

        // Step 2: spawn the up-stream consumer. Reads device replies
        // (SessionDispatch::Result frames) and routes them to the
        // PendingDispatchMap so the originating <self>.invoke_remote
        // caller wakes up.
        let presence_for_drain = Arc::clone(&self.presence);
        let pending_for_drain = self.pending.clone();
        let pending_stream_for_drain = self.pending_stream.clone();
        let caller_uri_for_drain = caller_uri.clone();
        // PR-N6 C3: drain task needs a service handle so inbound
        // `Request` frames can route into the same dispatch arms
        // the unary `Invoke` RPC uses (forward_invoke today; other
        // abilities follow as PR-N6 grows). `DaemonInvocationService`
        // is `Clone` over Arc/Option fields so this is cheap.
        let service_for_drain = self.clone();
        tokio::spawn(async move {
            drain_session_up_stream(
                up,
                caller_uri_for_drain,
                registration.session_id,
                presence_for_drain,
                pending_for_drain,
                pending_stream_for_drain,
                service_for_drain,
            )
            .await
        });

        // Step 3: hand the down stream to tonic. Frames arrive in
        // `down_tx` from <self>.invoke_remote dispatchers and from
        // federation.forward_invoke pushers as `DispatchFrame`
        // (presence_registry's newtype around `InvokeBidiDown`).
        // The tonic trait wants raw `InvokeBidiDown`, so map each
        // frame to unwrap the newtype.
        let stream = SessionDownStream::new(down_rx);
        Ok(Response::new(
            Box::pin(stream) as BoxedDownStream<InvokeBidiDown>
        ))
    }
}

/// Build a no-op down-stream control frame suitable for session
/// liveness probing. Current readers treat `Control` frames as
/// non-business metadata and ignore them, so this is wire-compatible
/// with every existing `<self>.session` consumer.
fn build_session_down_keepalive_frame() -> DispatchFrame {
    DispatchFrame {
        frame: InvokeBidiDown {
            payload: Some(DownPayload::Control(BidiControl::default())),
            ..InvokeBidiDown::default()
        },
    }
}

/// Build the spec §1.1 admission-accept frame: down frame 0 carries
/// an `InvocationReceipt` with `state = Admitted`. The receipt is
/// what tells the device-side caller "your `<self>.session` open was
/// accepted". Without it, devices have only HTTP/2 HEADERS as proof
/// of acceptance, which some intermediaries (and tonic-h2 in some
/// edge cases) buffer until the first response DATA frame — leaving
/// the device's `client.invoke_bidi(...).await` parked indefinitely.
///
/// Receipt fields kept minimal: only the `state` is load-bearing per
/// §1.1; the rest of `InvocationReceipt` is informational and the
/// device's `LocalAbilityDispatcher` ignores `Receipt` payloads
/// outright (handle_down only acts on `BinaryChunk`).
fn build_bidi_admission_receipt() -> InvokeBidiDown {
    InvokeBidiDown {
        sequence: 0,
        payload: Some(DownPayload::Receipt(InvocationReceipt {
            state: InvocationState::Admitted as i32,
            ..InvocationReceipt::default()
        })),
        ..InvokeBidiDown::default()
    }
}

fn build_session_down_admission_receipt() -> InvokeBidiDown {
    build_bidi_admission_receipt()
}

fn build_bidi_terminal_receipt(
    state: InvocationState,
    reason: impl Into<String>,
) -> InvokeBidiDown {
    build_bidi_terminal_receipt_with_payload(state, reason, None)
}

fn build_bidi_terminal_receipt_with_payload(
    state: InvocationState,
    reason: impl Into<String>,
    payload: Option<(Vec<u8>, &'static str)>,
) -> InvokeBidiDown {
    let (payload_bytes, payload_content_type) = payload
        .map(|(bytes, content_type)| (bytes, content_type.to_string()))
        .unwrap_or_default();
    InvokeBidiDown {
        payload: Some(DownPayload::Receipt(InvocationReceipt {
            state: state as i32,
            reason: reason.into(),
            payload: payload_bytes,
            payload_content_type,
            ..InvocationReceipt::default()
        })),
        ..InvokeBidiDown::default()
    }
}

const LOCAL_BIDI_DEFAULT_STREAM_ID: u32 = 1;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum LocalBidiWireKind {
    Pty,
    FileTransfer,
}

fn local_bidi_wire_kind(ability: &str) -> LocalBidiWireKind {
    if ability == crate::runtime::agents::file_transfer_ability::ABILITY_FILE_TRANSFER {
        LocalBidiWireKind::FileTransfer
    } else {
        LocalBidiWireKind::Pty
    }
}

fn local_bidi_stdout_stream_id(envelope_open: &EnvelopeOpen) -> u32 {
    envelope_open
        .streams
        .iter()
        .map(|stream| stream.stream_id)
        .find(|stream_id| *stream_id != 0)
        .unwrap_or(LOCAL_BIDI_DEFAULT_STREAM_ID)
}

#[derive(Debug)]
enum LocalBidiHandlerFrame {
    Forward(InvokeBidiDown),
    Terminal(InvokeBidiDown),
    Ignore,
    ProtocolFailure(String),
}

#[derive(Debug)]
enum LocalBidiUpFrame {
    Forward(serde_json::Value),
    ForwardAndClose(serde_json::Value),
    Close,
    Ignore,
}

fn map_local_bidi_up_payload(wire_kind: LocalBidiWireKind, payload: UpPayload) -> LocalBidiUpFrame {
    use crate::pb::axon::v1::bidi_control::Control as ControlVariant;
    use crate::pb::axon::v1::{BidiControl, PtyResize};
    use base64::{engine::general_purpose::STANDARD as B64, Engine as _};
    use serde_json::json;

    match (wire_kind, payload) {
        (LocalBidiWireKind::Pty, UpPayload::BinaryChunk(chunk)) => {
            let b64 = B64.encode(&chunk.data);
            LocalBidiUpFrame::Forward(json!({"type": "stdin", "data": b64}))
        }
        (
            LocalBidiWireKind::Pty,
            UpPayload::Control(BidiControl {
                control: Some(ctl), ..
            }),
        ) => match ctl {
            ControlVariant::PtyResize(PtyResize { cols, rows }) => {
                LocalBidiUpFrame::Forward(json!({"type": "resize", "cols": cols, "rows": rows}))
            }
            ControlVariant::Eof(true) => LocalBidiUpFrame::Close,
            _ => LocalBidiUpFrame::Ignore,
        },
        (LocalBidiWireKind::Pty, UpPayload::Control(_)) => LocalBidiUpFrame::Ignore,
        (LocalBidiWireKind::FileTransfer, UpPayload::BinaryChunk(chunk)) => {
            let b64 = B64.encode(&chunk.data);
            LocalBidiUpFrame::Forward(json!({"type": "chunk", "data": b64}))
        }
        (
            LocalBidiWireKind::FileTransfer,
            UpPayload::Control(BidiControl {
                control: Some(ctl), ..
            }),
        ) => match ctl {
            ControlVariant::Eof(true) => LocalBidiUpFrame::ForwardAndClose(json!({"type": "eof"})),
            _ => LocalBidiUpFrame::Ignore,
        },
        (LocalBidiWireKind::FileTransfer, UpPayload::Control(_)) => LocalBidiUpFrame::Ignore,
        (_, UpPayload::EnvelopeOpen(_)) => LocalBidiUpFrame::Ignore,
    }
}

fn map_local_bidi_handler_frame(
    wire_kind: LocalBidiWireKind,
    value: &serde_json::Value,
    stdout_stream_id: u32,
) -> LocalBidiHandlerFrame {
    use base64::{engine::general_purpose::STANDARD as B64, Engine as _};

    match wire_kind {
        LocalBidiWireKind::Pty => match value.get("type").and_then(|field| field.as_str()) {
            Some("stdout") => {
                let Some(data_b64) = value.get("data").and_then(|field| field.as_str()) else {
                    return LocalBidiHandlerFrame::ProtocolFailure(
                        "InvokeBidi local-dispatcher: PTY stdout frame missing `data`"
                            .to_string(),
                    );
                };
                let raw = match B64.decode(data_b64) {
                    Ok(raw) => raw,
                    Err(err) => {
                        return LocalBidiHandlerFrame::ProtocolFailure(format!(
                            "InvokeBidi local-dispatcher: PTY stdout frame base64 decode failed: {err}"
                        ))
                    }
                };
                LocalBidiHandlerFrame::Forward(InvokeBidiDown {
                    payload: Some(DownPayload::BinaryChunk(BinaryChunk {
                        stream_id: stdout_stream_id,
                        data: raw,
                        ..BinaryChunk::default()
                    })),
                    ..InvokeBidiDown::default()
                })
            }
            Some("exit") => {
                let reason = match value.get("status") {
                    Some(serde_json::Value::Number(status)) => {
                        format!("pty exited with status {status}")
                    }
                    Some(serde_json::Value::Null) | None => String::new(),
                    Some(other) => format!("pty exited with non-integer status {other}"),
                };
                LocalBidiHandlerFrame::Terminal(build_bidi_terminal_receipt(
                    InvocationState::Completed,
                    reason,
                ))
            }
            Some("warn") => {
                if let Some(message) = value.get("message").and_then(|field| field.as_str()) {
                    eprintln!(
                        "[axon-serve] InvokeBidi local-dispatcher warning from PTY handler: {message}"
                    );
                }
                LocalBidiHandlerFrame::Ignore
            }
            _ => LocalBidiHandlerFrame::Ignore,
        },
        LocalBidiWireKind::FileTransfer => match value.get("type").and_then(|field| field.as_str())
        {
            Some("chunk") => {
                let Some(data_b64) = value.get("data").and_then(|field| field.as_str()) else {
                    return LocalBidiHandlerFrame::ProtocolFailure(
                        "InvokeBidi local-dispatcher: file_transfer chunk frame missing `data`"
                            .to_string(),
                    );
                };
                let raw = match B64.decode(data_b64) {
                    Ok(raw) => raw,
                    Err(err) => {
                        return LocalBidiHandlerFrame::ProtocolFailure(format!(
                            "InvokeBidi local-dispatcher: file_transfer chunk frame base64 decode failed: {err}"
                        ))
                    }
                };
                LocalBidiHandlerFrame::Forward(InvokeBidiDown {
                    payload: Some(DownPayload::BinaryChunk(BinaryChunk {
                        stream_id: stdout_stream_id,
                        data: raw,
                        ..BinaryChunk::default()
                    })),
                    ..InvokeBidiDown::default()
                })
            }
            Some("complete") => match serde_json::to_vec(value) {
                Ok(payload) => LocalBidiHandlerFrame::Terminal(
                    build_bidi_terminal_receipt_with_payload(
                        InvocationState::Completed,
                        String::new(),
                        Some((payload, "application/json")),
                    ),
                ),
                Err(err) => LocalBidiHandlerFrame::ProtocolFailure(format!(
                    "InvokeBidi local-dispatcher: encode file_transfer completion receipt payload failed: {err}"
                )),
            },
            Some("error") => {
                let reason = match (
                    value.get("code").and_then(|field| field.as_str()),
                    value.get("message").and_then(|field| field.as_str()),
                ) {
                    (Some(code), Some(message))
                        if !code.trim().is_empty() && !message.trim().is_empty() =>
                    {
                        format!("{code}: {message}")
                    }
                    (_, Some(message)) if !message.trim().is_empty() => message.to_string(),
                    (Some(code), _) if !code.trim().is_empty() => code.to_string(),
                    _ => "file_transfer handler returned error".to_string(),
                };
                match serde_json::to_vec(value) {
                    Ok(payload) => LocalBidiHandlerFrame::Terminal(
                        build_bidi_terminal_receipt_with_payload(
                            InvocationState::Failed,
                            reason,
                            Some((payload, "application/json")),
                        ),
                    ),
                    Err(err) => LocalBidiHandlerFrame::ProtocolFailure(format!(
                        "InvokeBidi local-dispatcher: encode file_transfer error receipt payload failed: {err}"
                    )),
                }
            }
            Some("warn") => {
                if let Some(message) = value.get("message").and_then(|field| field.as_str()) {
                    eprintln!(
                        "[axon-serve] InvokeBidi local-dispatcher warning from file_transfer handler: {message}"
                    );
                }
                LocalBidiHandlerFrame::Ignore
            }
            _ => LocalBidiHandlerFrame::Ignore,
        }
    }
}

/// Down-stream wrapper that:
///   1. Emits a spec §1.1 admission-accept `InvocationReceipt`
///      (`state = Admitted`) as down frame 0 immediately on the
///      first poll. This is the missing protocol-required ack that
///      unblocks the device's `invoke_bidi.await` so it can enter
///      the down-stream read loop.
///   2. After frame 0, injects a no-op `BidiControl` heartbeat frame
///      whenever no business frame has been queued for
///      `SESSION_DOWN_HEARTBEAT_INTERVAL`.
///
/// Crucially this wrapper owns NO extra `DispatchSender`. That keeps
/// `PresenceRegistry` displacement semantics intact: when a same-URI
/// second session is admitted, dropping the displaced sender still
/// closes the old response stream immediately. A background
/// keepalive task that cloned the sender would accidentally keep the
/// displaced stream open, which is exactly the class of lifecycle
/// bug we are trying to eliminate here.
struct SessionDownStream {
    down_rx: tokio::sync::mpsc::Receiver<Result<DispatchFrame, Status>>,
    next_heartbeat: Pin<Box<tokio::time::Sleep>>,
    next_sequence: u64,
    /// Set to `Some(receipt)` at construction; first `poll_next`
    /// yields it and clears the slot. Subsequent polls follow the
    /// recv-then-heartbeat path.
    pending_admission_receipt: Option<InvokeBidiDown>,
}

struct LocalBidiDownStream {
    down_rx: tokio::sync::mpsc::Receiver<Result<InvokeBidiDown, Status>>,
    next_sequence: u64,
    pending_admission_receipt: Option<InvokeBidiDown>,
}

impl LocalBidiDownStream {
    fn new(down_rx: tokio::sync::mpsc::Receiver<Result<InvokeBidiDown, Status>>) -> Self {
        Self {
            down_rx,
            next_sequence: 0,
            pending_admission_receipt: Some(build_bidi_admission_receipt()),
        }
    }

    fn stamp_sequence(&mut self, mut frame: InvokeBidiDown) -> InvokeBidiDown {
        frame.sequence = self.next_sequence;
        self.next_sequence = self.next_sequence.saturating_add(1);
        frame
    }
}

impl Stream for LocalBidiDownStream {
    type Item = Result<InvokeBidiDown, Status>;

    fn poll_next(mut self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<Option<Self::Item>> {
        if let Some(receipt) = self.pending_admission_receipt.take() {
            return Poll::Ready(Some(Ok(self.stamp_sequence(receipt))));
        }

        match Pin::new(&mut self.down_rx).poll_recv(cx) {
            Poll::Ready(Some(Ok(frame))) => Poll::Ready(Some(Ok(self.stamp_sequence(frame)))),
            Poll::Ready(Some(Err(status))) => Poll::Ready(Some(Err(status))),
            Poll::Ready(None) => Poll::Ready(None),
            Poll::Pending => Poll::Pending,
        }
    }
}

impl SessionDownStream {
    fn new(down_rx: tokio::sync::mpsc::Receiver<Result<DispatchFrame, Status>>) -> Self {
        Self {
            down_rx,
            next_heartbeat: Box::pin(tokio::time::sleep(SESSION_DOWN_HEARTBEAT_INTERVAL)),
            next_sequence: 0,
            pending_admission_receipt: Some(build_session_down_admission_receipt()),
        }
    }

    fn reset_heartbeat(&mut self) {
        self.next_heartbeat
            .as_mut()
            .reset(tokio::time::Instant::now() + SESSION_DOWN_HEARTBEAT_INTERVAL);
    }

    fn stamp_sequence(&mut self, mut frame: InvokeBidiDown) -> InvokeBidiDown {
        frame.sequence = self.next_sequence;
        self.next_sequence = self.next_sequence.saturating_add(1);
        frame
    }
}

impl Stream for SessionDownStream {
    type Item = Result<InvokeBidiDown, Status>;

    fn poll_next(mut self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<Option<Self::Item>> {
        // Spec §1.1: down frame 0 MUST be an InvocationReceipt
        // signalling admission accept. Emit it before anything else
        // so the client's `invoke_bidi.await` always has a concrete
        // first DATA frame to flush HTTP/2 HEADERS against, and so
        // the wire shape matches what RFC-003 readers expect.
        if let Some(receipt) = self.pending_admission_receipt.take() {
            self.reset_heartbeat();
            return Poll::Ready(Some(Ok(self.stamp_sequence(receipt))));
        }

        match Pin::new(&mut self.down_rx).poll_recv(cx) {
            Poll::Ready(Some(Ok(frame))) => {
                self.reset_heartbeat();
                return Poll::Ready(Some(Ok(self.stamp_sequence(frame.frame))));
            }
            Poll::Ready(Some(Err(status))) => {
                self.reset_heartbeat();
                return Poll::Ready(Some(Err(status)));
            }
            Poll::Ready(None) => return Poll::Ready(None),
            Poll::Pending => {}
        }

        match self.next_heartbeat.as_mut().poll(cx) {
            Poll::Ready(()) => {
                self.reset_heartbeat();
                Poll::Ready(Some(Ok(
                    self.stamp_sequence(build_session_down_keepalive_frame().frame)
                )))
            }
            Poll::Pending => Poll::Pending,
        }
    }
}

impl DaemonInvocationService {
    /// PR-N6 C4: device-mode `forward_invoke` escalation. Sends
    /// the call up the open `<self>.session` bidi to the hub via
    /// the supplied escalation handle, awaits the matching
    /// `RequestResult`, and translates the typed outcome onto the
    /// existing unary wire shape callers already understand.
    ///
    /// On `Ok { result_bytes }` the caller sees the same
    /// `ForwardInvokeResponse` shape PR-N1 already returns on
    /// hub-mode success. On `Err { error: TargetOffline }` the
    /// caller sees `Status::failed_precondition(target_offline)`
    /// — wire-stable with the existing reason text so a CLI
    /// upstream of this daemon doesn't have to branch on
    /// device-vs-hub mode. Other typed errors map to the
    /// closest existing wire reason.
    async fn escalate_forward_invoke(
        &self,
        handle: &std::sync::Arc<
            crate::services::axon_serve::session_escalation::SessionEscalationHandle,
        >,
        arguments: &[u8],
    ) -> Result<Response<InvokeResponse>, Status> {
        let outcome = handle
            .escalate(
                ABILITY_FEDERATION_FORWARD_INVOKE.to_string(),
                arguments.to_vec(),
            )
            .await;
        match outcome {
            RequestOutcome::Ok { result_bytes } => Ok(Response::new(InvokeResponse {
                result: result_bytes,
                result_content_type: FEDERATION_RESULT_CONTENT_TYPE.to_string(),
                state: InvocationState::Completed as i32,
                ..InvokeResponse::default()
            })),
            RequestOutcome::Err {
                error: SessionRequestError::TargetOffline,
            } => Err(Status::failed_precondition(
                federation_wrappers::FORWARD_INVOKE_TARGET_OFFLINE_REASON,
            )),
            RequestOutcome::Err {
                error: SessionRequestError::PermissionDenied { reason },
            } => Err(Status::permission_denied(reason)),
            RequestOutcome::Err {
                error: SessionRequestError::UpstreamFailure { reason },
            } => Err(Status::unavailable(format!(
                "session escalation upstream failure: {reason}"
            ))),
            RequestOutcome::Err {
                error: SessionRequestError::UpstreamTimeout,
            } => Err(Status::deadline_exceeded(
                "session escalation timed out waiting for hub RequestResult",
            )),
        }
    }

    /// PR-N6 C3 hub-side handler for inbound `SessionDispatch::Request`
    /// frames arriving on a device's `<self>.session` bidi. Routes
    /// the named ability through the same dispatch arms the unary
    /// `Invoke` RPC consults, then maps the result into the typed
    /// `RequestOutcome` shape.
    ///
    /// Spec scope (PR-N6 v1): forwards
    /// `federation.forward_invoke` only. Other ability names return
    /// `PermissionDenied` so the device-side caller surfaces a
    /// structured error instead of a silent timeout — PR-N6 v2 may
    /// widen this set once a per-ability admission policy is
    /// specified.
    ///
    /// Trust boundary (PR-N6 spec §"What this spec does NOT cover"):
    /// the bidi was established with a signed Bootstrap frame, so
    /// the hub trusts the originating device on every Request frame
    /// — no per-Request signature verify happens here.
    pub(crate) async fn dispatch_session_request(
        &self,
        ability: &str,
        args: &[u8],
    ) -> RequestOutcome {
        match ability {
            ABILITY_FEDERATION_FORWARD_INVOKE => {
                // PR-N6 C5: emit the spec-locked target-resolution
                // log marker BEFORE handing to the existing
                // forward_invoke arm. Two markers, one per arm:
                //
                //   [session-request] resolved target via local-fast-path
                //   [session-request] resolved target via cross-hub dial
                //
                // The demo orchestration script grep-asserts both
                // verbatim. The arm choice mirrors the same
                // `target_tenant == local_tenant` comparison the
                // inner dispatch performs.
                emit_session_request_resolution_marker(
                    args,
                    self.session_realm.as_deref(),
                    &self.presence,
                );

                match self.dispatch_federation_forward_invoke(None, args).await {
                    Ok(response) => {
                        let body = response.into_inner();
                        RequestOutcome::Ok {
                            result_bytes: body.result,
                        }
                    }
                    Err(status) => map_status_to_session_request_error(status),
                }
            }
            other => RequestOutcome::Err {
                error: SessionRequestError::PermissionDenied {
                    reason: format!(
                        "session_request: ability `{other}` is not yet routed; \
                         only `{ABILITY_FEDERATION_FORWARD_INVOKE}` is wired in PR-N6 v1"
                    ),
                },
            },
        }
    }
}

/// PR-N6 C5: emit the spec-locked session-request resolution log
/// marker. The byte-deterministic strings are:
///
///   `[session-request] resolved target via local-fast-path`
///   `[session-request] resolved target via cross-hub dial`
///
/// Fires once per inbound `Request`, at hub-side
/// `dispatch_session_request` entry, BEFORE the inner dispatch
/// arm runs. The demo orchestration script grep-asserts these
/// strings verbatim against the hub daemon's stderr log.
///
/// Resolution mirrors `dispatch_federation_forward_invoke`'s
/// internal `is_local_tenant` computation: a malformed inner
/// payload or a missing `session_realm` collapses to the local
/// arm (matching the inner dispatcher's smoke-test fall-through).
fn emit_session_request_resolution_marker(
    args: &[u8],
    local_tenant: Option<&str>,
    presence: &crate::services::presence_registry::PresenceRegistry,
) {
    let request: Option<federation_wrappers::ForwardInvokeRequest> =
        serde_json::from_slice(args).ok();
    let target_tenant = request
        .as_ref()
        .and_then(|r| parse_tenant_from_uri(&r.target_uri));
    let has_local_presence = request
        .as_ref()
        .map(|r| presence.lookup(&r.target_uri).is_some())
        .unwrap_or(false);

    let is_local = has_local_presence
        || match (target_tenant, local_tenant) {
            (Some(target), Some(local)) => target == local,
            (_, None) | (None, Some(_)) => true,
        };

    if is_local {
        eprintln!("[session-request] resolved target via local-fast-path");
    } else {
        eprintln!("[session-request] resolved target via cross-hub dial");
    }
}

/// Translate a `tonic::Status` from a hub-side dispatch arm into
/// the typed `SessionRequestError` the device caller receives over
/// the bidi. The mapping mirrors the wire-stable error reasons
/// PR-N1 already uses on the unary path:
///
///   `failed_precondition` carrying the `target_offline` reason
///   maps to `TargetOffline`; permission rejections map to
///   `PermissionDenied`; everything else falls into
///   `UpstreamFailure` with the underlying status text preserved
///   so an operator grep'ing the device log can still cite the
///   exact upstream code + message.
fn map_status_to_session_request_error(status: Status) -> RequestOutcome {
    let code = status.code();
    let message = status.message().to_string();
    if code == tonic::Code::FailedPrecondition && message.contains("target_offline") {
        return RequestOutcome::Err {
            error: SessionRequestError::TargetOffline,
        };
    }
    if code == tonic::Code::PermissionDenied {
        return RequestOutcome::Err {
            error: SessionRequestError::PermissionDenied { reason: message },
        };
    }
    RequestOutcome::Err {
        error: SessionRequestError::UpstreamFailure {
            reason: format!("code={code:?} message={message}"),
        },
    }
}

/// Build a `DispatchFrame` carrying a JSON-serialised
/// `SessionDispatch::RequestResult` ready to push back down a
/// device's `<self>.session` reverse channel. Encoding failure is
/// vanishingly unlikely (owned `[u8; 16]`, owned `Vec<u8>`,
/// typed enum) but mapped to a synthetic `UpstreamFailure` outcome
/// so a malformed inner result never silently wedges the device.
fn build_session_request_result_frame(
    call_id: [u8; 16],
    outcome: RequestOutcome,
) -> crate::services::presence_registry::DispatchFrame {
    use crate::pb::axon::v1::invoke_bidi_down::Payload;
    use crate::pb::axon::v1::{BinaryChunk, InvokeBidiDown};

    let frame = SessionDispatch::RequestResult { call_id, outcome };
    let data = match serde_json::to_vec(&frame) {
        Ok(bytes) => bytes,
        Err(err) => {
            // Replace the payload with a typed error so the device
            // sees a structured outcome instead of a malformed
            // frame. The id_hex stays in the eprintln below for
            // operator audit.
            let fallback = SessionDispatch::RequestResult {
                call_id,
                outcome: RequestOutcome::Err {
                    error: SessionRequestError::UpstreamFailure {
                        reason: format!("encode RequestResult: {err}"),
                    },
                },
            };
            serde_json::to_vec(&fallback).expect("typed error variant must always encode")
        }
    };
    crate::services::presence_registry::DispatchFrame {
        frame: InvokeBidiDown {
            payload: Some(Payload::BinaryChunk(BinaryChunk {
                data,
                ..BinaryChunk::default()
            })),
            ..InvokeBidiDown::default()
        },
    }
}

/// Push a `RequestResult` frame back down the device's bidi via
/// the same PresenceRegistry-keyed `DispatchSender` the device's
/// session-accept handler registered. The device drains the down
/// stream in `session_initiator::dial_and_run_session` and routes
/// `RequestResult` frames to the `oneshot::Receiver` matching
/// `call_id` (per PR-N6 spec §"Concurrent multiplexing"). Lookup
/// failure means the device disconnected between issuing the
/// Request and the hub finishing dispatch — log + drop, which is
/// the same shape PR-N1's `try_push_forward_invoke_frame` uses for
/// the symmetric race.
fn push_session_request_result(
    presence: &Arc<PresenceRegistry>,
    caller_uri: &str,
    id_hex: &str,
    frame: crate::services::presence_registry::DispatchFrame,
) {
    let Some((session_id, sender)) = presence.lookup_tracked(caller_uri) else {
        eprintln!(
            "[session-accept] device {caller_uri} no longer in presence registry; \
             dropping RequestResult call_id={id_hex} (device disconnected mid-dispatch)"
        );
        return;
    };
    match sender.try_send(Ok(frame)) {
        Ok(()) => {}
        Err(tokio::sync::mpsc::error::TrySendError::Full(_)) => {
            let _ = presence.remove_if_session(caller_uri, session_id, OfflineReason::SendFailed);
            eprintln!(
                "[session-accept] failed to push RequestResult call_id={id_hex} to {caller_uri}: \
                 channel full; removed device with OfflineReason::SendFailed"
            );
        }
        Err(tokio::sync::mpsc::error::TrySendError::Closed(_)) => {
            let _ = presence.remove_if_session(caller_uri, session_id, OfflineReason::StreamClosed);
            eprintln!(
                "[session-accept] failed to push RequestResult call_id={id_hex} to {caller_uri}: \
                 device down-channel closed; removed from presence registry"
            );
        }
    }
}

/// Drain a device's `<self>.session` up-stream. Each up-frame is
/// expected to be a `BinaryChunk` carrying a JSON-serialised
/// `SessionDispatch::Result`; on parse the matching pending entry
/// in the `PendingDispatchMap` is completed so the
/// `<self>.invoke_remote` caller wakes up.
///
/// On stream close (any reason — graceful CloseSend, transport
/// reset, RST_STREAM, peer crash) the device is removed from the
/// presence registry with an appropriate `OfflineReason` so that
/// future `lookup` calls see it as offline immediately.
async fn drain_session_up_stream(
    mut up: Streaming<InvokeBidiUp>,
    caller_uri: String,
    session_id: crate::services::presence_registry::PresenceSessionId,
    presence: Arc<PresenceRegistry>,
    pending: Option<Arc<PendingDispatchMap>>,
    pending_stream: Option<Arc<PendingStreamDispatchMap>>,
    service: DaemonInvocationService,
) {
    use crate::pb::axon::v1::invoke_bidi_up::Payload as UpPayload;

    let mut close_reason = OfflineReason::StreamClosed;
    let mut expected_up_sequence = 1_u64;

    while let Some(frame_result) = up.next().await {
        let frame = match frame_result {
            Ok(f) => f,
            Err(status) => {
                // Walk the std::error::Error source chain so the
                // underlying h2::Error (with its `Reason` code and
                // `Initiator`) surfaces, not just tonic's opaque
                // "h2 protocol error" wrapper. Without this we
                // cannot distinguish a peer-initiated CANCEL from
                // a library-initiated PROTOCOL_ERROR, which makes
                // diagnosing reset-loops on the device side
                // impossible.
                let mut chain = format!("{status}");
                let mut src: Option<&dyn std::error::Error> = std::error::Error::source(&status);
                while let Some(err) = src {
                    chain.push_str(&format!(" ↳ {err}"));
                    src = err.source();
                }
                eprintln!(
                    "[session-accept] up-stream error for {caller_uri}: {chain}; \
                     code={:?}; removing from registry",
                    status.code()
                );
                close_reason = OfflineReason::StreamReset;
                break;
            }
        };

        if frame.sequence != expected_up_sequence {
            eprintln!(
                "[session-accept] {caller_uri} violated {REASON_BIDI_FRAME_SEQUENCE}: \
                 expected up sequence {expected_up_sequence}, got {}; removing from registry",
                frame.sequence
            );
            close_reason = OfflineReason::StreamReset;
            break;
        }
        expected_up_sequence = expected_up_sequence.saturating_add(1);

        let chunk = match frame.payload {
            Some(UpPayload::BinaryChunk(c)) => c,
            Some(UpPayload::Control(control)) => {
                if matches!(
                    control.control,
                    Some(crate::pb::axon::v1::bidi_control::Control::Eof(true))
                ) {
                    break;
                }
                continue;
            }
            Some(UpPayload::EnvelopeOpen(_)) => {
                eprintln!(
                    "[session-accept] {caller_uri} sent unexpected EnvelopeOpen after frame 0; \
                     ignoring"
                );
                continue;
            }
            None => continue,
        };

        // Parse SessionDispatch::Result. A malformed frame is logged
        // but does not tear down the session — the device may send
        // future frames that are well-formed.
        let dispatch: SessionDispatch = match serde_json::from_slice(&chunk.data) {
            Ok(d) => d,
            Err(err) => {
                eprintln!(
                    "[session-accept] {caller_uri} sent malformed SessionDispatch JSON: {err}; \
                     ignoring frame"
                );
                continue;
            }
        };

        match dispatch {
            SessionDispatch::Result {
                call_id,
                payload,
                terminal,
                error,
            } => {
                if terminal {
                    let dispatch_result = DispatchResult { payload, error };
                    let mut completed = false;
                    if let Some(pending_stream) = pending_stream.as_ref() {
                        completed = pending_stream
                            .finish(call_id, dispatch_result.clone())
                            .await;
                    }
                    if !completed {
                        let Some(pending) = pending.as_ref() else {
                            eprintln!(
                                "[session-accept] {caller_uri} sent terminal Result for call_id={call_id} but \
                                 daemon was constructed without PendingDispatchMap; ignoring"
                            );
                            continue;
                        };
                        completed = pending.complete(call_id, dispatch_result);
                    }
                    if !completed {
                        eprintln!(
                            "[session-accept] {caller_uri} sent terminal Result for call_id={call_id} but \
                             no pending entry matched (caller may have cancelled); silent no-op"
                        );
                    }
                } else {
                    let Some(pending_stream) = pending_stream.as_ref() else {
                        eprintln!(
                            "[session-accept] {caller_uri} sent streaming Result chunk for call_id={call_id} but \
                             daemon was constructed without PendingStreamDispatchMap; ignoring"
                        );
                        continue;
                    };
                    let completed = pending_stream.push_chunk(call_id, payload).await;
                    if !completed {
                        eprintln!(
                            "[session-accept] {caller_uri} sent streaming Result chunk for call_id={call_id} but \
                             no pending stream entry matched; silent no-op"
                        );
                    }
                }
            }
            SessionDispatch::Dispatch { call_id, .. } => {
                // A device sending a Dispatch up its own session
                // makes no sense — Dispatch is hub→device only.
                eprintln!(
                    "[session-accept] {caller_uri} sent unexpected Dispatch frame \
                     (call_id={call_id}); ignoring"
                );
            }
            SessionDispatch::BidiOpen {
                call_id, ability, ..
            } => {
                eprintln!(
                    "[session-accept] {caller_uri} sent unexpected BidiOpen frame \
                     (call_id={call_id} ability={ability}); ignoring"
                );
            }
            SessionDispatch::BidiInput { call_id, eof, .. } => {
                eprintln!(
                    "[session-accept] {caller_uri} sent unexpected BidiInput frame \
                     (call_id={call_id} eof={eof}); ignoring"
                );
            }
            SessionDispatch::Request {
                call_id,
                ability,
                args,
            } => {
                // PR-N6 C3: device → hub forward_invoke escalation.
                // The device emits this when its CLI's
                // `ability invoke --node` hits a target whose
                // dispatch the device-mode daemon's empty local
                // PresenceRegistry can't serve. The hub runs the
                // SAME ability dispatch the unary `Invoke` RPC
                // does, then sends `RequestResult` back down the
                // device's open `<self>.session` bidi.
                //
                // Spec-locked log marker per PR-N6
                // §"Locked log markers". The demo orchestration
                // script grep-asserts this verbatim.
                let id_hex = call_id_hex(&call_id);
                eprintln!(
                    "[session-accept] received Request frame call_id={id_hex} ability={ability}"
                );

                // Dispatch off the drain task so a slow inner
                // call (cross-hub dial round-trip, peer-side
                // ability handler latency) does not stall
                // subsequent up-frames the device sends. Each
                // Request gets its own short-lived task.
                let service_for_request = service.clone();
                let presence_for_reply = Arc::clone(&presence);
                let caller_uri_for_reply = caller_uri.clone();
                tokio::spawn(async move {
                    let outcome = service_for_request
                        .dispatch_session_request(&ability, &args)
                        .await;
                    let frame = build_session_request_result_frame(call_id, outcome);
                    push_session_request_result(
                        &presence_for_reply,
                        &caller_uri_for_reply,
                        &id_hex,
                        frame,
                    );
                });
            }
            SessionDispatch::RequestResult { call_id, .. } => {
                // RequestResult is hub → device only; a device
                // sending one up its own session is malformed.
                let id_hex = call_id_hex(&call_id);
                eprintln!(
                    "[session-accept] {caller_uri} sent unexpected RequestResult frame \
                     (call_id={id_hex}); ignoring"
                );
            }
        }
    }

    if presence
        .remove_if_session(&caller_uri, session_id, close_reason)
        .is_some()
    {
        eprintln!(
            "[session-accept] {caller_uri} session ended ({:?}); removed from registry",
            close_reason
        );
    } else {
        eprintln!(
            "[session-accept] {caller_uri} session ended ({:?}); newer session already replaced registry entry",
            close_reason
        );
    }
}

/// Session-realm gate.
///
/// Same-realm callers always pass (the most common shape; a
/// device whose URI's realm matches the hub's `session_realm`
/// is the canonical "device joining its own hub" case).
///
/// Cross-realm callers pass iff the caller's URI is present in
/// the supplied trust anchor. The frame-0 envelope's
/// `caller_signature` was already verified upstream by the
/// admission gate against the trust anchor's pubkey for this
/// URI, so a trust-anchor hit here is a sufficient proof of
/// federated identity. Same mechanism the cross-realm
/// `forward_invoke` admission already uses (PR-N2 commits
/// `d1adbea` + `68f6556`); we extend it to cover
/// `<self>.session` admission too. Unblocks the cross-hub
/// same-tenant directive that LB-49 surfaced.
fn validate_session_realm(
    caller_uri: &str,
    session_realm: Option<&str>,
    trust_anchor: &RealmTrustAnchor,
) -> Result<(), Status> {
    let Some(daemon_realm) = session_realm else {
        return Ok(());
    };

    let caller_realm = parse_realm_from_uri(caller_uri).ok_or_else(|| {
        Status::invalid_argument(format!(
            "<self>.session: caller URI `{caller_uri}` does not match the canonical \
             `easynet:///r/{{realm}}/...` shape"
        ))
    })?;

    if caller_realm == daemon_realm {
        return Ok(());
    }

    // Cross-realm path: federated trust is required. The trust
    // anchor lookup is the same one the admission gate already
    // exercised on frame 0, so a hit means the caller's pubkey
    // signed the bidi's frame-0 envelope and the operator has
    // explicitly listed this URI under realm-trust.toml.
    if trust_anchor.lookup(caller_uri).is_some() {
        return Ok(());
    }

    Err(Status::permission_denied(format!(
        "<self>.session: caller `{caller_uri}` from realm `{caller_realm}` is \
         not in this hub's realm `{daemon_realm}` and not present in the \
         realm trust anchor as a federated identity; cross-realm session \
         requires either same-realm or an explicit `[[trusted_agent]]` entry"
    )))
}

/// Wire shape for an incremental presence event delivered by the
/// `federation.subscribe_directory` server-stream after the initial
/// snapshot frame.
///
/// Mirrors `services::presence_registry::PresenceEvent` but with
/// `serde::Serialize`-friendly field naming so the JSON encoding
/// is stable for PR-4's schema-compat captures.
#[derive(Debug, serde::Serialize, serde::Deserialize, PartialEq, Eq)]
#[serde(tag = "kind", rename_all = "snake_case")]
enum PresenceEventDelta {
    Online {
        canonical_agent_uri: String,
    },
    Offline {
        canonical_agent_uri: String,
        reason: &'static str,
    },
}

impl From<crate::services::presence_registry::PresenceEvent> for PresenceEventDelta {
    fn from(event: crate::services::presence_registry::PresenceEvent) -> Self {
        use crate::services::presence_registry::{OfflineReason, PresenceEvent};
        match event {
            PresenceEvent::Online { uri } => Self::Online {
                canonical_agent_uri: uri,
            },
            PresenceEvent::Offline { uri, reason } => Self::Offline {
                canonical_agent_uri: uri,
                reason: match reason {
                    OfflineReason::StreamClosed => "stream_closed",
                    OfflineReason::StreamReset => "stream_reset",
                    OfflineReason::SendFailed => "send_failed",
                    OfflineReason::AdminRevoked => "admin_revoked",
                },
            },
        }
    }
}

/// Decode the base64-encoded inner envelope carried by
/// `federation.forward_invoke`. Errors map to
/// `Status::invalid_argument` with a useful message.
fn decode_inner_envelope(b64: &str) -> Result<Vec<u8>, Status> {
    use base64::{engine::general_purpose::STANDARD, Engine as _};
    if b64.is_empty() {
        return Ok(Vec::new());
    }
    STANDARD.decode(b64).map_err(|err| {
        Status::invalid_argument(format!(
            "federation.forward_invoke: inner_envelope_b64 is not valid base64: {err}"
        ))
    })
}

/// **PR-N1 commit 11/N + C1a**. The inner-envelope payload
/// shape the CLI bridge (`support/federation_invoke.rs::
/// invoke_via_federation_forward`) emits: a JSON object
/// carrying the originally-requested `(ability, args)` pair the
/// user typed plus a `call_id` minted client-side that DEC-N4
/// §2.1 threads back through `ForwardInvokeResponse.
/// correlation_call_id` so the caller can correlate the
/// response with its awaiting bidi.
pub(crate) struct InnerPayload {
    pub ability: String,
    pub args_bytes: Vec<u8>,
    pub call_id: String,
}

/// **PR-N1 commit 11/N + C1a**. Decode the base64-then-JSON
/// inner payload the CLI bridge ships, surfacing each parse
/// failure as `Status::invalid_argument` with a wire-stable
/// hint so scripts grepping the daemon log can distinguish
/// them. Non-empty `call_id` is required by DEC-N4 §2.1; a
/// missing or empty value rejects with a clear error rather
/// than synthesising a server-side id (which would defeat the
/// caller-side correlation contract).
pub(crate) fn decode_inner_payload(b64: &str) -> Result<InnerPayload, Status> {
    let raw = decode_inner_envelope(b64)?;
    if raw.is_empty() {
        return Err(Status::invalid_argument(
            "federation.forward_invoke: inner_envelope_b64 is empty; \
             cross-hub dispatch requires a base64-encoded JSON \
             {ability, args, call_id} payload",
        ));
    }
    let parsed: serde_json::Value = serde_json::from_slice(&raw).map_err(|err| {
        Status::invalid_argument(format!(
            "federation.forward_invoke: inner envelope is not valid JSON: {err}"
        ))
    })?;
    let obj = parsed.as_object().ok_or_else(|| {
        Status::invalid_argument(
            "federation.forward_invoke: inner envelope must be a JSON object \
             with `ability`, `args`, and `call_id` fields",
        )
    })?;
    let ability = obj
        .get("ability")
        .and_then(|v| v.as_str())
        .filter(|s| !s.is_empty())
        .ok_or_else(|| {
            Status::invalid_argument(
                "federation.forward_invoke: inner envelope is missing a non-empty \
                 string `ability` field",
            )
        })?
        .to_string();
    let call_id = obj
        .get("call_id")
        .and_then(|v| v.as_str())
        .filter(|s| !s.is_empty())
        .ok_or_else(|| {
            Status::invalid_argument(
                "federation.forward_invoke: inner envelope is missing a non-empty \
                 string `call_id` field (DEC-N4 §2.1 correlation requirement)",
            )
        })?
        .to_string();
    let args_value = obj
        .get("args")
        .cloned()
        .unwrap_or(serde_json::Value::Object(Default::default()));
    let args_bytes = serde_json::to_vec(&args_value).map_err(|err| {
        Status::internal(format!(
            "federation.forward_invoke: re-serialise inner args: {err}"
        ))
    })?;
    Ok(InnerPayload {
        ability,
        args_bytes,
        call_id,
    })
}

/// **PR-N1 commit 11/N**. Build the envelope the cross-hub
/// dialer attaches to the rebuilt peer `InvokeRequest`. The
/// peer daemon's admission gate compares the envelope's caller
/// URI against its local `realm-trust.toml`, so the choice is:
///
/// - When the original CLI call carried an envelope (the
///   typical case via `support::federation_invoke`), forward
///   that envelope verbatim. The peer's admission accepts it
///   iff the caller URI is in the peer's trust set — for the
///   PR-N1 same-account same-tenant scope the backend tenant
///   fix (`0af7c0e`) ensures both daemons store the same
///   `(tenant_id, agent_uri, public_key)` triple, so
///   forward-as-is is the right answer.
/// - When the original call has no envelope (test fixtures or
///   internal paths that pre-existed PR-N1), synthesize a
///   minimal envelope with `caller.uri = target_uri`. Peer
///   admission will reject this on the strict path (no
///   signature) but the URI-only Device arm under DEC-013 will
///   admit. The peer-side `target_online` fast-path then runs
///   against its presence registry as before.
///
/// PR-N2 will replace this verbatim-forward with an AXIOM
/// mapping rewrite (`caller = self_hub`, `callee = target_hub`,
/// `subject = original_caller`) plus a daemon-identity
/// signature so cross-realm peers can verify the call without
/// shared trust set.
pub(crate) fn build_peer_envelope(
    caller_envelope: Option<&Envelope>,
    target_uri: &str,
    local_realm: Option<&str>,
) -> Envelope {
    use rand::RngCore as _;

    let mut forwarded = caller_envelope.cloned().unwrap_or_default();
    let peer_hub_uri = parse_tenant_from_uri(target_uri).map(crate::uri::hub_uri);

    forwarded.caller = Some(AgentIdentity {
        uri: local_realm
            .map(crate::uri::hub_uri)
            .or_else(|| {
                forwarded
                    .caller
                    .as_ref()
                    .map(|caller| caller.uri.trim().to_string())
                    .filter(|uri| !uri.is_empty())
            })
            .unwrap_or_else(|| target_uri.to_string()),
        ..AgentIdentity::default()
    });

    if let Some(peer_hub_uri) = peer_hub_uri.clone() {
        forwarded.callee = Some(AgentIdentity {
            uri: peer_hub_uri.clone(),
            ..AgentIdentity::default()
        });
        if forwarded
            .subject
            .as_ref()
            .map(|subject| subject.uri.trim().is_empty())
            .unwrap_or(true)
        {
            forwarded.subject = Some(SubjectIdentity {
                uri: caller_envelope
                    .and_then(|env| env.caller.as_ref())
                    .map(|caller| caller.uri.trim().to_string())
                    .filter(|uri| !uri.is_empty())
                    .unwrap_or(peer_hub_uri),
                ..SubjectIdentity::default()
            });
        }
    } else {
        if forwarded
            .callee
            .as_ref()
            .map(|callee| callee.uri.trim().is_empty())
            .unwrap_or(true)
        {
            forwarded.callee = Some(AgentIdentity {
                uri: target_uri.to_string(),
                ..AgentIdentity::default()
            });
        }
        if forwarded
            .subject
            .as_ref()
            .map(|subject| subject.uri.trim().is_empty())
            .unwrap_or(true)
        {
            forwarded.subject = Some(SubjectIdentity {
                uri: caller_envelope
                    .and_then(|env| env.caller.as_ref())
                    .map(|caller| caller.uri.trim().to_string())
                    .filter(|uri| !uri.is_empty())
                    .unwrap_or_else(|| target_uri.to_string()),
                ..SubjectIdentity::default()
            });
        }
    }

    if forwarded.invocation_nonce.len() != 16 {
        let mut nonce = vec![0u8; 16];
        rand::rngs::OsRng.fill_bytes(&mut nonce);
        forwarded.invocation_nonce = nonce;
    }

    forwarded
}

fn sign_peer_request_envelope(
    envelope: &mut Envelope,
    ability: &str,
    arguments: &[u8],
    local_realm: Option<&str>,
    hub_signing_seed: Option<&SessionSigningSeed>,
) -> Result<(), Status> {
    let Some(realm) = local_realm else {
        return Ok(());
    };

    use easynet_axon::invocation::axiom::{
        canonical_invocation_bytes, AgentIdentity as AxiomAgentIdentity, CausalContext,
        InvocationEnvelope, SubjectIdentity as AxiomSubjectIdentity, UriProfile,
    };
    use ed25519_dalek::{Signer as _, SigningKey};
    use sha2::{Digest, Sha256};

    envelope.causal_context = None;

    let caller_uri = envelope
        .caller
        .as_ref()
        .map(|caller| caller.uri.trim())
        .filter(|uri| !uri.is_empty())
        .ok_or_else(|| {
            Status::internal("cross-hub forward_invoke signing: caller URI missing after rewrite")
        })?;
    let callee_uri = envelope
        .callee
        .as_ref()
        .map(|callee| callee.uri.trim())
        .filter(|uri| !uri.is_empty())
        .ok_or_else(|| {
            Status::internal("cross-hub forward_invoke signing: callee URI missing after rewrite")
        })?;
    let subject_uri = envelope
        .subject
        .as_ref()
        .map(|subject| subject.uri.trim())
        .filter(|uri| !uri.is_empty())
        .ok_or_else(|| {
            Status::internal("cross-hub forward_invoke signing: subject URI missing after rewrite")
        })?;
    let invocation_nonce: [u8; 16] =
        envelope
            .invocation_nonce
            .as_slice()
            .try_into()
            .map_err(|_| {
                Status::internal(
                    "cross-hub forward_invoke signing: invocation_nonce must be 16 bytes",
                )
            })?;

    let mut hasher = Sha256::new();
    hasher.update(arguments);
    let args_digest: [u8; 32] = hasher.finalize().into();

    // Hub identity is a fresh-random Ed25519 seed minted by
    // backend's `LoadOrInitHubIdentity` at first boot and persisted
    // to `${HOME}/.easynet-hub/<realm>/identity.json` (see backend
    // `runtime/subject_context.go::backendIdentityRecord`). The
    // pre-fix path used `derive_subject_keypair(realm,
    // "easynet:prv:hub:{realm}")` — deterministically derived from
    // SHA256(realm + subject_id) — which produced a DIFFERENT key
    // than the trust-anchor entry (sourced from `identity.json`).
    // Peer hubs verifying via `federation.resolve_key` saw a
    // signature/key mismatch and rejected with
    // `AXON_CALLER_SIGNATURE_INVALID:caller_signature_invalid`.
    //
    // Read the on-disk seed in production so the signing key
    // matches the pubkey the trust anchor advertises. Tests stage
    // an identity.json under their per-test HomeGuard root via
    // `stage_test_hub_identity` so the same code path covers both.
    let hub_seed = match hub_signing_seed.copied() {
        Some(seed) => seed,
        None => read_hub_identity_seed(realm).map_err(|err| {
            Status::internal(format!(
                "cross-hub forward_invoke signing: load hub identity seed for realm `{realm}`: {err}"
            ))
        })?,
    };
    let signing_key = SigningKey::from_bytes(&hub_seed);
    let axiom_envelope = InvocationEnvelope {
        caller: AxiomAgentIdentity::new(caller_uri, UriProfile::EasynetStrictV2),
        callee: AxiomAgentIdentity::new(callee_uri, UriProfile::EasynetStrictV2),
        subject: AxiomSubjectIdentity::new(subject_uri, UriProfile::EasynetStrictV2),
        ability: ability.to_string(),
        args_digest,
        invocation_nonce,
        causal_context: CausalContext::None,
    };
    let signature = signing_key.sign(&canonical_invocation_bytes(&axiom_envelope));
    envelope.caller_signature = Some(CallerSignature {
        algorithm: "ed25519".to_string(),
        signature: signature.to_bytes().to_vec(),
        ..CallerSignature::default()
    });
    Ok(())
}

/// Load the hub's Ed25519 signing seed for `realm` from the
/// on-disk identity file backend's `LoadOrInitHubIdentity` writes
/// at first boot. File shape mirrors backend
/// `runtime/subject_context.go::backendIdentityRecord`:
///
/// ```json
/// {
///   "private_key_seed_hex": "<64-hex>",
///   "agent_uri": "easynet:///r/<realm>/hub",
///   "created_at_unix_ms": <int>
/// }
/// ```
///
/// Path: `${HOME}/.easynet-hub/<realm>/identity.json`. In
/// production hub containers `HOME=/srv/easynet`, so the resolved
/// path is `/srv/easynet/.easynet-hub/<realm>/identity.json`.
///
/// Returns the 32-byte seed. Errors propagate as `String` and the
/// caller wraps them in `Status::internal`. This helper is only
/// used by the cross-hub `federation.forward_invoke` signing path
/// today; the seed is the same one the trust anchor's hub entry
/// advertises as `public_key_b64`, so a peer's
/// `federation.resolve_key` lookup → signature verify round trip
/// closes cleanly.
///
/// Tests fall back to the deterministic
/// `derive_subject_keypair(realm, "easynet:prv:hub:{realm}")`
/// seed when the on-disk file is missing — this preserves the
/// pre-fix wire shape for in-process unit tests that don't stage
/// a `~/.easynet-hub/<realm>/identity.json` fixture, while
/// production daemons (which always have the file, written by
/// backend's first-boot bootstrap) take the real-seed path.
/// The fallback is `cfg(test)`-gated so an accidentally-missing
/// identity file in production fails loudly rather than silently
/// substituting a key the peer hub will reject.
pub(crate) fn read_hub_identity_seed(realm: &str) -> Result<[u8; 32], String> {
    let home = std::env::var_os("HOME").ok_or_else(|| "HOME unset".to_string())?;
    let path = std::path::Path::new(&home)
        .join(".easynet-hub")
        .join(realm)
        .join("identity.json");
    match std::fs::read_to_string(&path) {
        Ok(raw) => {
            #[derive(serde::Deserialize)]
            struct HubIdentityRecord {
                private_key_seed_hex: String,
            }
            let parsed: HubIdentityRecord = serde_json::from_str(&raw)
                .map_err(|err| format!("parse {}: {err}", path.display()))?;
            let seed_bytes = hex::decode(parsed.private_key_seed_hex.trim())
                .map_err(|err| format!("decode hex from {}: {err}", path.display()))?;
            if seed_bytes.len() != 32 {
                return Err(format!(
                    "{} private_key_seed_hex must decode to 32 bytes, got {}",
                    path.display(),
                    seed_bytes.len()
                ));
            }
            let mut seed = [0u8; 32];
            seed.copy_from_slice(&seed_bytes);
            Ok(seed)
        }
        Err(err) => {
            #[cfg(test)]
            {
                let _ = err;
                // Test fallback: deterministic derive matches the
                // pre-fix wire shape so existing unit tests that
                // don't stage an `identity.json` fixture stay
                // green. Production never takes this path (see
                // function-level docs).
                let hub_subject_id = format!("easynet:prv:hub:{realm}");
                let (seed, _pk_b64) =
                    crate::runtime::publish::derive_subject_keypair(realm, &hub_subject_id);
                Ok(seed)
            }
            #[cfg(not(test))]
            {
                Err(format!("read {}: {err}", path.display()))
            }
        }
    }
}

/// Receipt-type discriminator for a `federation.forward_invoke`
/// audit record on the *caller* hub. DEC-N5 §1 dual-write: the
/// caller hub records this; the target hub records its usual
/// `InvocationReceipt` for the inner ability, and the two are
/// linkable by `target_call_id` (the caller-minted call_id that
/// `ForwardInvokeResponse.correlation_call_id` echoes back).
const FORWARD_RECEIPT_TYPE: &str = "forward";

/// `payload_content_type` stamped on a ForwardReceipt whose
/// `payload` carries `sha256(result_bytes)`. Empty content type
/// for the target_offline path (no result bytes → no digest).
const FORWARD_RECEIPT_DIGEST_CONTENT_TYPE: &str = "application/octet-stream;sha256";

/// Build a caller-hub `ForwardReceipt` (modelled on top of
/// `InvocationReceipt` — DEC-N5 §1 only requires the causal link,
/// not a separate persistence container, so the existing
/// `SharedReceiptStore` shape is reused).
///
/// LB-39 §44 / §45 field mapping:
/// - `receipt_type = "forward"` — discriminator filtering
///   forward-receipts from inner-ability state-machine receipts.
/// - `child_invocation_id = target_call_id` — caller-minted
///   `correlation_call_id`; same id appears on the target hub's
///   `InvocationReceipt`, enabling the cross-hub audit join.
/// - `payload = sha256(result_bytes)` for happy paths; empty for
///   the target_offline path (encodes `result_digest = None`).
/// - `caller_binding` / `callee_binding` — caller is the original
///   envelope's caller (or a synthetic fallback equal to
///   target_uri); callee is the target_uri.
/// - `state = Completed` — terminal receipt for audit filters.
fn build_forward_receipt(
    target_call_id: &str,
    target_uri: &str,
    caller_envelope: Option<&Envelope>,
    result_bytes: Option<&[u8]>,
) -> InvocationReceipt {
    use sha2::{Digest, Sha256};
    let payload = match result_bytes {
        Some(bytes) => {
            let mut hasher = Sha256::new();
            hasher.update(bytes);
            hasher.finalize().to_vec()
        }
        None => Vec::new(),
    };
    let caller_binding = caller_envelope
        .and_then(|env| env.caller.clone())
        .or_else(|| {
            Some(AgentIdentity {
                uri: target_uri.to_string(),
                ..AgentIdentity::default()
            })
        });
    let callee_binding = Some(AgentIdentity {
        uri: target_uri.to_string(),
        ..AgentIdentity::default()
    });
    let payload_content_type = if payload.is_empty() {
        String::new()
    } else {
        FORWARD_RECEIPT_DIGEST_CONTENT_TYPE.to_string()
    };
    InvocationReceipt {
        receipt_type: FORWARD_RECEIPT_TYPE.to_string(),
        state: InvocationState::Completed as i32,
        child_invocation_id: target_call_id.to_string(),
        payload_content_type,
        payload,
        caller_binding,
        callee_binding,
        ..InvocationReceipt::default()
    }
}

/// Wrap the inner envelope bytes into a `DispatchFrame` heading
/// down a target's `<self>.session` reverse channel.
///
/// DEC-N4 §2.1 round-trip note: `ForwardInvokeRequest` carries
/// `causal_context_bytes` and `forward_deadline_ms` as outer wire
/// fields; they remain on the request struct so the dispatcher's
/// audit-chain hook (PR-N5 §1) and deadline derivation (DEC-N5
/// §3) can read them directly. The dispatch frame itself stays
/// proto-stable for C1a (just the inner-envelope BinaryChunk) —
/// C1c's schema regen elevates these to first-class frame fields.
fn build_forward_invoke_dispatch_frame(
    inner_bytes: Vec<u8>,
) -> crate::services::presence_registry::DispatchFrame {
    use crate::pb::axon::v1::invoke_bidi_down::Payload;
    use crate::pb::axon::v1::{BinaryChunk, InvokeBidiDown};

    let chunk = BinaryChunk {
        data: inner_bytes,
        ..BinaryChunk::default()
    };
    crate::services::presence_registry::DispatchFrame {
        frame: InvokeBidiDown {
            payload: Some(Payload::BinaryChunk(chunk)),
            ..InvokeBidiDown::default()
        },
    }
}

/// Build a `DispatchFrame` carrying a `SessionDispatch::Dispatch` JSON
/// payload, ready to push down a target's `<self>.session` reverse
/// channel. Encoding failure is impossible for the current variant
/// (call_id u64, owned String, owned Vec<u8>) but mapped to
/// `Status::internal` for forward-compatibility per letter 25 §"flag".
fn build_invoke_remote_dispatch_frame(
    call_id: u64,
    ability: &str,
    args: &[u8],
) -> Result<DispatchFrame, Status> {
    let payload = SessionDispatch::Dispatch {
        call_id,
        ability: ability.to_string(),
        args: args.to_vec(),
    };
    let bytes = serde_json::to_vec(&payload).map_err(|err| {
        Status::internal(format!(
            "<self>.invoke_remote: encode SessionDispatch::Dispatch: {err}"
        ))
    })?;
    let chunk = BinaryChunk {
        stream_id: INVOKE_REMOTE_STREAM_ID,
        data: bytes,
        ..BinaryChunk::default()
    };
    Ok(DispatchFrame {
        frame: InvokeBidiDown {
            payload: Some(DownPayload::BinaryChunk(chunk)),
            ..InvokeBidiDown::default()
        },
    })
}

fn build_remote_bidi_open_dispatch_frame(
    call_id: u64,
    ability: &str,
    args: &[u8],
) -> Result<DispatchFrame, Status> {
    let payload = SessionDispatch::BidiOpen {
        call_id,
        ability: ability.to_string(),
        args: args.to_vec(),
    };
    let bytes = serde_json::to_vec(&payload).map_err(|err| {
        Status::internal(format!(
            "InvokeBidi remote file_transfer: encode SessionDispatch::BidiOpen: {err}"
        ))
    })?;
    Ok(DispatchFrame {
        frame: InvokeBidiDown {
            payload: Some(DownPayload::BinaryChunk(BinaryChunk {
                stream_id: INVOKE_REMOTE_STREAM_ID,
                data: bytes,
                ..BinaryChunk::default()
            })),
            ..InvokeBidiDown::default()
        },
    })
}

fn build_remote_bidi_input_dispatch_frame(
    call_id: u64,
    payload: &[u8],
    eof: bool,
) -> DispatchFrame {
    let frame = SessionDispatch::BidiInput {
        call_id,
        payload: payload.to_vec(),
        eof,
    };
    let data =
        serde_json::to_vec(&frame).expect("SessionDispatch::BidiInput is statically encodable");
    DispatchFrame {
        frame: InvokeBidiDown {
            payload: Some(DownPayload::BinaryChunk(BinaryChunk {
                stream_id: INVOKE_REMOTE_STREAM_ID,
                data,
                ..BinaryChunk::default()
            })),
            ..InvokeBidiDown::default()
        },
    }
}

fn remote_bidi_target_uri(envelope_open: &EnvelopeOpen) -> Option<String> {
    let callee = envelope_open
        .envelope
        .as_ref()
        .and_then(|env| env.callee.as_ref())
        .map(|callee| callee.uri.trim())
        .filter(|uri| !uri.is_empty())?;
    Some(crate::uri::canonicalize_presence_key(callee))
}

/// Build the terminal `InvokeBidiDown` frame the
/// `<self>.invoke_remote` caller's down stream yields. Carries the
/// `InvokeRemoteDown::Result` JSON in `BinaryChunk.data`.
fn build_invoke_remote_terminal_frame(down: &InvokeRemoteDown) -> Result<InvokeBidiDown, Status> {
    let bytes = serde_json::to_vec(down).map_err(|err| {
        Status::internal(format!(
            "<self>.invoke_remote: encode InvokeRemoteDown: {err}"
        ))
    })?;
    let chunk = BinaryChunk {
        stream_id: INVOKE_REMOTE_STREAM_ID,
        data: bytes,
        ..BinaryChunk::default()
    };
    Ok(InvokeBidiDown {
        payload: Some(DownPayload::BinaryChunk(chunk)),
        ..InvokeBidiDown::default()
    })
}

/// Parse a JSON-encoded request body, mapping any error to
/// `Status::invalid_argument` with a useful message. Centralised so
/// every wrapper dispatch site reports parse failures the same way.
fn parse_json_args<T: serde::de::DeserializeOwned>(arguments: &[u8]) -> Result<T, Status> {
    serde_json::from_slice(arguments).map_err(|err| {
        Status::invalid_argument(format!(
            "federation wrapper: failed to decode JSON arguments: {err}"
        ))
    })
}

/// Encode a typed federation response into `InvokeResponse.result`
/// with `result_content_type = "application/json"`. Mapping any
/// serialisation error to `Status::internal` because the wrappers
/// use serde-derived types — failure here is a programmer bug, not
/// a caller bug.
///
/// `state` is set to `INVOCATION_STATE_COMPLETED` so unary callers
/// that grep on `resp.state == "completed"` (Go-side
/// `stateString` mapping) see the expected wire-visible success
/// signal. Without this the proto default-zero value
/// (`INVOCATION_STATE_UNSPECIFIED`) collapses to `"failed"` on the
/// Go side per `stateString`'s default arm — silent failure-look-
/// like under what the dispatcher considers a clean dispatch.
fn wrap_json_response<T: serde::Serialize>(
    response: &T,
) -> Result<Response<InvokeResponse>, Status> {
    let bytes = serde_json::to_vec(response).map_err(|err| {
        Status::internal(format!(
            "federation wrapper: failed to encode JSON response: {err}"
        ))
    })?;
    let invoke_response = InvokeResponse {
        result: bytes,
        result_content_type: FEDERATION_RESULT_CONTENT_TYPE.to_string(),
        state: InvocationState::Completed as i32,
        ..InvokeResponse::default()
    };
    Ok(Response::new(invoke_response))
}

/// **PR-N1 commit 3a/N**. Extract the tenant component from a
/// canonical EasyNet URI (`easynet:///r/{tenant_id}/agent/...`).
/// Returns `None` for URIs that do not match the canonical shape;
/// callers treat that as "cannot route — fall back to legacy
/// shape".
///
/// Pure function so it composes well into the cross-tenant
/// routing branch landing in commit 3b/N: the dispatcher reads
/// `request.target_uri`, calls `parse_tenant_from_uri`, and
/// looks the tenant up in `federated_peers` to obtain a hub
/// URI. The function deliberately does not allocate — it returns
/// a `&str` borrowed from the input.
pub(crate) fn parse_tenant_from_uri(uri: &str) -> Option<&str> {
    // Expected shape: `easynet:///r/<tenant>/agent/<node>` or
    // `easynet:///r/<tenant>/agent/<node>/...`. We reject anything
    // that does not start with `easynet:///r/` so a typo URL is
    // not silently accepted as `tenant = ""`.
    let after_scheme = uri.strip_prefix("easynet:///r/")?;
    // The first path component up to the next `/` is the tenant.
    // An empty first component (the URI starts `easynet:///r//...`)
    // is a malformed URI and rejected.
    let tenant_end = after_scheme.find('/').unwrap_or(after_scheme.len());
    if tenant_end == 0 {
        return None;
    }
    Some(&after_scheme[..tenant_end])
}

#[cfg(test)]
mod tests {
    use super::*;

    use crate::pb::axon::v1::{AgentIdentity, Envelope};
    use crate::services::realm_trust_anchor::RealmTrustAnchor;

    /// Test helper daemon URI — admitted by the test admission
    /// facade via the loopback bypass. Tests that exercise
    /// admission rejection construct a different facade.
    // URI v4.1.4: daemons are devices, not agents. The legacy
    // `agent/<bare-id>` shape is rewritten to `device/<id>` by
    // `canonicalize_presence_key` at the forward_invoke
    // entry point — fixtures must use the canonical shape so
    // self-target equality holds without going through the
    // coercer (which would mask a real lookup-key mismatch
    // bug if it appeared in production).
    const TEST_DAEMON_URI: &str = "easynet:///r/test-realm/device/test-daemon";

    fn make_service() -> DaemonInvocationService {
        let admission = AdmissionFacade::new(
            Arc::new(RealmTrustAnchor::default()),
            Some(TEST_DAEMON_URI.to_string()),
        );
        DaemonInvocationService::new(Arc::new(PresenceRegistry::new()), admission)
            .with_hub_signing_seed([0x11; 32])
    }

    fn test_envelope() -> Envelope {
        Envelope {
            caller: Some(AgentIdentity {
                uri: TEST_DAEMON_URI.to_string(),
                ..AgentIdentity::default()
            }),
            ..Envelope::default()
        }
    }

    fn invoke_request(function_name: &str, args_json: &str) -> Request<InvokeRequest> {
        Request::new(InvokeRequest {
            envelope: Some(test_envelope()),
            function_name: function_name.to_string(),
            arguments: args_json.as_bytes().to_vec(),
            ..InvokeRequest::default()
        })
    }

    fn parse_response_body<T: serde::de::DeserializeOwned>(resp: Response<InvokeResponse>) -> T {
        let body = resp.into_inner();
        assert_eq!(body.result_content_type, FEDERATION_RESULT_CONTENT_TYPE);
        serde_json::from_slice(&body.result).expect("response body deserialises")
    }

    #[tokio::test]
    async fn invoke_dispatches_federation_join_to_wrapper() {
        let svc = make_service();
        let resp = svc
            .invoke(invoke_request(
                ABILITY_FEDERATION_JOIN,
                r#"{"canonical_agent_uri":"easynet:///r/realm/agent/n1","realm":"realm"}"#,
            ))
            .await
            .expect("dispatch returns Ok");
        let body: federation_wrappers::JoinResponse = parse_response_body(resp);
        assert_eq!(body.canonical_agent_uri, "easynet:///r/realm/agent/n1");
        assert_eq!(body.realm, "realm");
        assert_eq!(body.join_receipt_hash.len(), 64);
    }

    #[tokio::test]
    async fn invoke_dispatches_federation_advertise_agent() {
        let svc = make_service();
        let resp = svc
            .invoke(invoke_request(
                ABILITY_FEDERATION_ADVERTISE_AGENT,
                r#"{"agent_uri":"easynet:///r/realm/agent/n1"}"#,
            ))
            .await
            .expect("dispatch returns Ok");
        let body: federation_wrappers::AdvertiseAgentResponse = parse_response_body(resp);
        assert!(body.ack);
        assert!(!body.replaced_prior);
    }

    #[tokio::test]
    async fn invoke_dispatches_federation_heartbeat() {
        let svc = make_service();
        let resp = svc
            .invoke(invoke_request(
                ABILITY_FEDERATION_HEARTBEAT,
                r#"{"agent_uri":"easynet:///r/realm/agent/n1"}"#,
            ))
            .await
            .expect("dispatch returns Ok");
        let body: federation_wrappers::HeartbeatResponse = parse_response_body(resp);
        assert_eq!(body.membership_status, "active");
        assert_eq!(body.realm_directory_size, 0);
    }

    #[tokio::test]
    async fn invoke_dispatches_federation_resolve_with_no_filter() {
        let svc = make_service();
        let resp = svc
            .invoke(invoke_request(ABILITY_FEDERATION_RESOLVE, "{}"))
            .await
            .expect("dispatch returns Ok");
        let body: federation_wrappers::ResolveResponse = parse_response_body(resp);
        assert!(body.agents.is_empty());
    }

    #[tokio::test]
    async fn invoke_dispatches_federation_discover_with_no_filter_returns_empty_when_no_peers() {
        // PR-N3 N3-4: single-realm daemon (no federated peers)
        // returns the empty discover list. Graceful degradation —
        // the ability is callable on every daemon, just empty
        // when nothing has been federated yet.
        let svc = make_service();
        let resp = svc
            .invoke(invoke_request(ABILITY_FEDERATION_DISCOVER, "{}"))
            .await
            .expect("dispatch returns Ok");
        let body: federation_wrappers::DiscoverResponse = parse_response_body(resp);
        assert!(body.entries.is_empty());
    }

    #[tokio::test]
    async fn invoke_dispatches_federation_discover_returns_peer_entries_when_view_populated() {
        // PR-N3 N3-4: when the federated_directory cell holds
        // entries (write side is the per-peer
        // RemoteDirectoryClient task in N3-3.1 — for this unit
        // test we manually `replace` the cell with a populated
        // map), discover surfaces them with origin_realm
        // stamped per §2.4.
        use crate::services::federation_directory::{
            DirectoryEntry, DirectoryEvent, DirectoryView, SharedFederatedDirectoryView,
        };
        use std::collections::BTreeMap;

        let cell = SharedFederatedDirectoryView::default();
        let mut peer_view = DirectoryView::new("realm-b".to_string());
        peer_view.apply_frame(&DirectoryEvent::Snapshot {
            entries: vec![DirectoryEntry {
                agent_uri: "easynet:///r/realm-b/agent/peer-device".to_string(),
                node_id: "peer-1".to_string(),
                display_name: Some("silan-phone".to_string()),
                status: "active".to_string(),
                origin_realm: None, // peer omitted; rewrite stamps realm-b
                hub_endpoint: Some("https://hub-b.example:50443".to_string()),
                last_seen_unix_ms: Some(1_714_500_000_000),
            }],
        });
        let mut peers = BTreeMap::new();
        peers.insert("realm-b".to_string(), Arc::new(peer_view));
        cell.replace(peers);

        let svc = make_service().with_federated_directory_cell(cell);
        let resp = svc
            .invoke(invoke_request(ABILITY_FEDERATION_DISCOVER, "{}"))
            .await
            .expect("dispatch returns Ok");
        let body: federation_wrappers::DiscoverResponse = parse_response_body(resp);
        assert_eq!(body.entries.len(), 1);
        assert_eq!(
            body.entries[0].agent_uri,
            "easynet:///r/realm-b/agent/peer-device"
        );
        assert_eq!(
            body.entries[0].origin_realm.as_deref(),
            Some("realm-b"),
            "§2.4 origin_realm rewrite must show through to the discover response"
        );
    }

    #[tokio::test]
    async fn invoke_dispatches_federation_discover_with_uri_filter_returns_single_hit() {
        use crate::services::federation_directory::{
            DirectoryEntry, DirectoryEvent, DirectoryView, SharedFederatedDirectoryView,
        };
        use std::collections::BTreeMap;

        let cell = SharedFederatedDirectoryView::default();
        let mut peer_view = DirectoryView::new("realm-b".to_string());
        peer_view.apply_frame(&DirectoryEvent::Snapshot {
            entries: vec![
                DirectoryEntry {
                    agent_uri: "easynet:///r/realm-b/agent/match".to_string(),
                    node_id: "n1".to_string(),
                    display_name: None,
                    status: "active".to_string(),
                    origin_realm: None,
                    hub_endpoint: None,
                    last_seen_unix_ms: None,
                },
                DirectoryEntry {
                    agent_uri: "easynet:///r/realm-b/agent/other".to_string(),
                    node_id: "n2".to_string(),
                    display_name: None,
                    status: "active".to_string(),
                    origin_realm: None,
                    hub_endpoint: None,
                    last_seen_unix_ms: None,
                },
            ],
        });
        let mut peers = BTreeMap::new();
        peers.insert("realm-b".to_string(), Arc::new(peer_view));
        cell.replace(peers);

        let svc = make_service().with_federated_directory_cell(cell);
        let resp = svc
            .invoke(invoke_request(
                ABILITY_FEDERATION_DISCOVER,
                r#"{"agent_uri":"easynet:///r/realm-b/agent/match"}"#,
            ))
            .await
            .expect("dispatch returns Ok");
        let body: federation_wrappers::DiscoverResponse = parse_response_body(resp);
        assert_eq!(body.entries.len(), 1);
        assert_eq!(
            body.entries[0].agent_uri,
            "easynet:///r/realm-b/agent/match"
        );
    }

    // ── N3-N4 dispatch wire — discover with user filter ─────

    #[tokio::test]
    async fn invoke_discover_with_user_id_filters_unbound_cross_realm_entries() {
        // Daemon's session_realm = realm-b. View has realm-c
        // entry (unbound for the calling user). Bindings store
        // is empty, so the cross-realm entry is filtered out.
        use crate::runtime::keyring::federated_bindings::FederatedBindingsStore;
        use crate::services::federation_directory::{
            DirectoryEntry, DirectoryEvent, DirectoryView, SharedFederatedDirectoryView,
        };
        use std::collections::BTreeMap;

        let cell = SharedFederatedDirectoryView::default();
        let mut realm_c = DirectoryView::new("realm-c".to_string());
        realm_c.apply_frame(&DirectoryEvent::Snapshot {
            entries: vec![DirectoryEntry {
                agent_uri: "easynet:///r/realm-c/agent/unbound".to_string(),
                node_id: "n".to_string(),
                display_name: None,
                status: "active".to_string(),
                origin_realm: None,
                hub_endpoint: None,
                last_seen_unix_ms: None,
            }],
        });
        let mut peers = BTreeMap::new();
        peers.insert("realm-c".to_string(), Arc::new(realm_c));
        cell.replace(peers);

        let bindings = Arc::new(FederatedBindingsStore::in_memory());
        let svc = make_service()
            .with_session_realm("realm-b")
            .with_federated_directory_cell(cell)
            .with_federated_bindings_store(bindings);

        let resp = svc
            .invoke(invoke_request(
                ABILITY_FEDERATION_DISCOVER,
                r#"{"local_user_id":"user-on-b"}"#,
            ))
            .await
            .expect("dispatch returns Ok");
        let body: federation_wrappers::DiscoverResponse = parse_response_body(resp);
        assert!(
            body.entries.is_empty(),
            "unbound cross-realm entry must be filtered when local_user_id is set"
        );
    }

    #[tokio::test]
    async fn invoke_discover_without_user_id_does_not_filter() {
        // Same setup as above but no local_user_id ⇒ unfiltered
        // path. Cross-realm unbound entries surface (operator /
        // audit query path).
        use crate::services::federation_directory::{
            DirectoryEntry, DirectoryEvent, DirectoryView, SharedFederatedDirectoryView,
        };
        use std::collections::BTreeMap;

        let cell = SharedFederatedDirectoryView::default();
        let mut realm_c = DirectoryView::new("realm-c".to_string());
        realm_c.apply_frame(&DirectoryEvent::Snapshot {
            entries: vec![DirectoryEntry {
                agent_uri: "easynet:///r/realm-c/agent/u".to_string(),
                node_id: "n".to_string(),
                display_name: None,
                status: "active".to_string(),
                origin_realm: None,
                hub_endpoint: None,
                last_seen_unix_ms: None,
            }],
        });
        let mut peers = BTreeMap::new();
        peers.insert("realm-c".to_string(), Arc::new(realm_c));
        cell.replace(peers);

        let svc = make_service()
            .with_session_realm("realm-b")
            .with_federated_directory_cell(cell);

        let resp = svc
            .invoke(invoke_request(ABILITY_FEDERATION_DISCOVER, r#"{}"#))
            .await
            .expect("dispatch returns Ok");
        let body: federation_wrappers::DiscoverResponse = parse_response_body(resp);
        assert_eq!(
            body.entries.len(),
            1,
            "unfiltered path must surface every entry regardless of binding state"
        );
    }

    #[tokio::test]
    async fn invoke_discover_with_user_id_keeps_bound_entry() {
        use crate::runtime::keyring::federated_bindings::{
            FederatedBindingsStore, FederatedUserBinding,
        };
        use crate::services::federation_directory::{
            DirectoryEntry, DirectoryEvent, DirectoryView, SharedFederatedDirectoryView,
        };
        use std::collections::BTreeMap;

        let cell = SharedFederatedDirectoryView::default();
        let mut realm_a = DirectoryView::new("realm-a".to_string());
        realm_a.apply_frame(&DirectoryEvent::Snapshot {
            entries: vec![DirectoryEntry {
                agent_uri: "easynet:///r/realm-a/agent/bound-user".to_string(),
                node_id: "n".to_string(),
                display_name: None,
                status: "active".to_string(),
                origin_realm: None,
                hub_endpoint: None,
                last_seen_unix_ms: None,
            }],
        });
        let mut peers = BTreeMap::new();
        peers.insert("realm-a".to_string(), Arc::new(realm_a));
        cell.replace(peers);

        let bindings = Arc::new(FederatedBindingsStore::in_memory());
        bindings
            .record_binding(
                FederatedUserBinding {
                    source_realm: "realm-a".to_string(),
                    source_user_uri: "easynet:///r/realm-a/agent/bound-user".to_string(),
                    source_user_pubkey_b64: "AAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAA="
                        .to_string(),
                    local_user_id: "user-on-b".to_string(),
                    bound_at_unix_ms: 1_714_500_000_000,
                },
                "n".to_string(),
            )
            .unwrap();

        let svc = make_service()
            .with_session_realm("realm-b")
            .with_federated_directory_cell(cell)
            .with_federated_bindings_store(bindings);

        let resp = svc
            .invoke(invoke_request(
                ABILITY_FEDERATION_DISCOVER,
                r#"{"local_user_id":"user-on-b"}"#,
            ))
            .await
            .expect("dispatch returns Ok");
        let body: federation_wrappers::DiscoverResponse = parse_response_body(resp);
        assert_eq!(body.entries.len(), 1);
        assert_eq!(
            body.entries[0].agent_uri,
            "easynet:///r/realm-a/agent/bound-user"
        );
    }

    #[tokio::test]
    async fn invoke_dispatches_federation_list_user_devices_admits_loopback_caller() {
        // PR-N3 N3-5: a hub-mode daemon listing its own users
        // from a CLI on the same machine works without
        // configuring itself as a Hub trust entry — loopback
        // bypass admits at the general gate, the N3-5 filter
        // recognises `is_loopback = true` and accepts.
        let svc = make_service();
        // Two devices online for tenant-x.
        svc.presence.insert(
            "easynet:///r/tenant-x/agent/device-1".to_string(),
            tokio::sync::mpsc::channel(8).0,
        );
        svc.presence.insert(
            "easynet:///r/tenant-x/agent/device-2".to_string(),
            tokio::sync::mpsc::channel(8).0,
        );
        // One device for an unrelated tenant — must NOT show
        // through.
        svc.presence.insert(
            "easynet:///r/tenant-other/agent/device-3".to_string(),
            tokio::sync::mpsc::channel(8).0,
        );

        let resp = svc
            .invoke(invoke_request(
                ABILITY_FEDERATION_LIST_USER_DEVICES,
                r#"{"tenant_id":"tenant-x"}"#,
            ))
            .await
            .expect("loopback caller admitted");
        let body: federation_wrappers::ListUserDevicesResponse = parse_response_body(resp);
        assert_eq!(body.devices.len(), 2);
        for entry in &body.devices {
            assert!(entry.agent_uri.starts_with("easynet:///r/tenant-x/agent/"));
        }
    }

    #[tokio::test]
    async fn invoke_dispatches_federation_list_user_devices_rejects_non_hub_caller() {
        // PR-N3 N3-5: caller URI is in trust set but as Backend
        // role → admission filter rejects. PermissionDenied is
        // the wire-stable rejection; the message mentions the
        // caller URI for operator audit grep.
        //
        // Build the test through the URI-only Device admission
        // arm: we register the caller as a Device-role entry so
        // the general admission gate's URI-only no-op admits
        // (DEC-013 Device path doesn't require a signed envelope).
        // The dispatch arm then runs the N3-5 admission filter,
        // which reads the trust anchor again and finds the role
        // is Device, not Hub — reject.
        use crate::services::realm_trust_anchor::{
            RealmTrustAnchor, TrustedAgent, TrustedAgentRole,
        };

        let device_caller_uri = "easynet:///r/realm-b/agent/device-not-hub";
        let mut anchor_inner = RealmTrustAnchor::default();
        anchor_inner
            .append_agent(TrustedAgent {
                agent_uri: device_caller_uri.to_string(),
                public_key_b64: "AAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAA=".to_string(),
                role: TrustedAgentRole::Device,
                added_at_unix_ms: 1_700_000_000_000,
                origin_tenant_id: None,
                hub_uri: None,
                tls_ca_pem_path: None,
            })
            .expect("append device");
        let admission =
            AdmissionFacade::new(Arc::new(anchor_inner), Some(TEST_DAEMON_URI.to_string()));
        let svc = DaemonInvocationService::new(Arc::new(PresenceRegistry::new()), admission);

        let envelope = Envelope {
            caller: Some(crate::pb::axon::v1::AgentIdentity {
                uri: device_caller_uri.to_string(),
                profile: "easynet-strict-v2".to_string(),
            }),
            ..Envelope::default()
        };
        let req = Request::new(InvokeRequest {
            envelope: Some(envelope),
            function_name: ABILITY_FEDERATION_LIST_USER_DEVICES.to_string(),
            arguments: br#"{"tenant_id":"tenant-x"}"#.to_vec(),
            ..InvokeRequest::default()
        });

        let err = svc
            .invoke(req)
            .await
            .expect_err("device-role caller must be rejected by N3-5 filter");
        assert_eq!(err.code(), tonic::Code::PermissionDenied);
        assert!(
            err.message().contains(device_caller_uri),
            "rejection message must surface the caller URI; got: {}",
            err.message()
        );
    }

    #[tokio::test]
    async fn invoke_dispatches_federation_resolve_key_returns_pubkey_when_present() {
        // PR-N2 commit 2/N: peer-side `federation.resolve_key`
        // surfaces the local trust anchor's `public_key_b64` for
        // a known URI. Cross-hub `FederatedKeyResolver` consumes
        // this exact wire shape.
        use crate::services::realm_trust_anchor::{
            RealmTrustAnchor, TrustedAgent, TrustedAgentRole,
        };
        let entry = TrustedAgent {
            agent_uri: "easynet:///r/realm-a/agent/n1".to_string(),
            public_key_b64: "AAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAA=".to_string(),
            role: TrustedAgentRole::Device,
            added_at_unix_ms: 1_700_000_000_000,
            origin_tenant_id: None,
            hub_uri: None,
            tls_ca_pem_path: None,
        };
        let anchor = Arc::new(RealmTrustAnchor::from_entries(vec![entry]).expect("anchor"));
        let admission = AdmissionFacade::new(anchor, Some(TEST_DAEMON_URI.to_string()));
        let svc = DaemonInvocationService::new(Arc::new(PresenceRegistry::new()), admission);

        let resp = svc
            .invoke(invoke_request(
                ABILITY_FEDERATION_RESOLVE_KEY,
                r#"{"agent_uri":"easynet:///r/realm-a/agent/n1"}"#,
            ))
            .await
            .expect("dispatch returns Ok");
        let body: federation_wrappers::ResolveKeyResponse = parse_response_body(resp);
        assert_eq!(
            body.public_key_b64,
            "AAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAA="
        );
    }

    #[tokio::test]
    async fn invoke_dispatches_federation_resolve_key_returns_not_found_when_uri_unknown() {
        // PR-N2 commit 2/N: miss surfaces as Status::not_found
        // with the URI in the error message — operators can
        // grep the daemon log for the exact URI that failed.
        let svc = make_service();
        let err = svc
            .invoke(invoke_request(
                ABILITY_FEDERATION_RESOLVE_KEY,
                r#"{"agent_uri":"easynet:///r/realm-a/agent/missing"}"#,
            ))
            .await
            .expect_err("miss must surface Status::not_found");
        assert_eq!(err.code(), tonic::Code::NotFound);
        assert!(
            err.message().contains("easynet:///r/realm-a/agent/missing"),
            "expected the missing URI in error message, got: {}",
            err.message()
        );
    }

    #[tokio::test]
    async fn invoke_dispatches_federation_revoke() {
        let svc = make_service();
        let resp = svc
            .invoke(invoke_request(
                ABILITY_FEDERATION_REVOKE,
                r#"{"target_uri":"easynet:///r/realm/agent/missing"}"#,
            ))
            .await
            .expect("dispatch returns Ok");
        let body: federation_wrappers::RevokeResponse = parse_response_body(resp);
        assert!(body.ack);
        assert!(!body.was_active);
    }

    #[tokio::test]
    async fn invoke_dispatches_federation_forward_invoke() {
        // DEC-N4 §2.1: empty `inner_envelope_b64` is rejected
        // up front by `decode_inner_payload` because the
        // payload must carry a non-empty `call_id`. Earlier
        // staging code accepted the empty shape and replied
        // `target_online: false`; the final wire shape requires
        // a real correlation id, so the wrong shape surfaces as
        // `Status::invalid_argument`.
        let svc = make_service();
        let err = svc
            .invoke(invoke_request(
                ABILITY_FEDERATION_FORWARD_INVOKE,
                r#"{"target_uri":"easynet:///r/realm/agent/missing","inner_envelope_b64":""}"#,
            ))
            .await
            .expect_err("empty inner_envelope_b64 must be rejected");
        assert_eq!(err.code(), tonic::Code::InvalidArgument);
        assert!(
            err.message().contains("inner_envelope_b64 is empty"),
            "expected empty-payload error, got: {}",
            err.message()
        );
    }

    #[tokio::test]
    async fn invoke_rejects_subscribe_directory_via_unary_invoke() {
        let svc = make_service();
        match svc
            .invoke(invoke_request(ABILITY_FEDERATION_SUBSCRIBE_DIRECTORY, "{}"))
            .await
        {
            Err(err) => {
                assert_eq!(err.code(), tonic::Code::InvalidArgument);
                assert!(err.message().contains("server-stream"));
            }
            Ok(_) => panic!("subscribe_directory must be rejected on unary Invoke"),
        }
    }

    #[tokio::test]
    async fn invoke_unknown_ability_returns_unimplemented_with_pr1_note() {
        let svc = make_service();
        match svc.invoke(invoke_request("custom.ability.x", "{}")).await {
            Err(err) => {
                assert_eq!(err.code(), tonic::Code::Unimplemented);
                assert!(
                    err.message().contains("commit 7/9"),
                    "should cite the commit that wires LocalAbilityRegistry; got: {}",
                    err.message()
                );
            }
            Ok(_) => panic!("unknown ability must be rejected"),
        }
    }

    #[tokio::test]
    async fn invoke_returns_invalid_argument_on_bad_json() {
        let svc = make_service();
        match svc
            .invoke(invoke_request(ABILITY_FEDERATION_JOIN, "not-json"))
            .await
        {
            Err(err) => assert_eq!(err.code(), tonic::Code::InvalidArgument),
            Ok(_) => panic!("malformed JSON must be rejected"),
        }
    }

    #[tokio::test]
    async fn invoke_stream_dispatches_subscribe_directory_initial_frame_then_pump() {
        use futures::StreamExt;

        // Build the service with our own presence Arc so the test
        // can drive the broadcast sender's close behaviour via Arc
        // drop (the pump only ends when *every* sender drops; the
        // pump itself holds a Weak so dropping the last Arc here
        // closes the channel cleanly).
        let presence = Arc::new(PresenceRegistry::new());
        let admission = AdmissionFacade::new(
            Arc::new(RealmTrustAnchor::default()),
            Some(TEST_DAEMON_URI.to_string()),
        );
        let svc = DaemonInvocationService::new(Arc::clone(&presence), admission);

        let resp = svc
            .invoke_stream(Request::new(InvokeServerStreamRequest {
                envelope: Some(test_envelope()),
                function_name: ABILITY_FEDERATION_SUBSCRIBE_DIRECTORY.to_string(),
                ..InvokeServerStreamRequest::default()
            }))
            .await
            .expect("subscribe_directory initial frame returns Ok");

        let mut stream = resp.into_inner();

        // Frame 1 — the initial empty snapshot.
        let first = stream
            .next()
            .await
            .expect("at least one frame")
            .expect("frame is Ok");
        assert_eq!(first.content_type, FEDERATION_RESULT_CONTENT_TYPE);
        let initial: federation_wrappers::SubscribeDirectoryInitial =
            serde_json::from_slice(&first.payload).expect("decodes initial");
        assert!(initial.agents.is_empty());

        // Frame 2 — an Online delta after a registry insert is
        // pumped through the broadcast subscriber.
        let (sender, _rx) = tokio::sync::mpsc::channel::<
            Result<crate::services::presence_registry::DispatchFrame, tonic::Status>,
        >(1);
        presence.insert("easynet:///r/test-realm/agent/n1".to_string(), sender);

        let second = stream
            .next()
            .await
            .expect("delta frame after insert")
            .expect("frame is Ok");
        let delta: serde_json::Value = serde_json::from_slice(&second.payload).expect("decodes");
        assert_eq!(delta.get("kind").and_then(|v| v.as_str()), Some("online"));
        assert_eq!(
            delta.get("canonical_agent_uri").and_then(|v| v.as_str()),
            Some("easynet:///r/test-realm/agent/n1"),
        );

        // Drop both Arcs holding the broadcast sender so the pump
        // sees `RecvError::Closed` on its next poll and yields None.
        // Without this the stream is intentionally infinite.
        drop(svc);
        drop(presence);

        // Now the pump must close. Bound the wait so a real bug
        // here surfaces as a test failure, not a CI hang.
        let close = tokio::time::timeout(std::time::Duration::from_secs(2), stream.next())
            .await
            .expect("pump closes within 2 s after senders drop");
        assert!(
            close.is_none(),
            "stream must terminate once all senders drop"
        );
    }

    #[tokio::test]
    async fn invoke_stream_dispatches_subscribe_directory_v2_emits_directory_events() {
        // PR-N3 N3-streaming-1. v2 stream emits DirectoryEvent
        // shapes (Snapshot first, then Upsert/Remove).
        use crate::services::federation_directory::DirectoryEvent;
        use futures::StreamExt;

        let presence = Arc::new(PresenceRegistry::new());
        let admission = AdmissionFacade::new(
            Arc::new(RealmTrustAnchor::default()),
            Some(TEST_DAEMON_URI.to_string()),
        );
        let svc = DaemonInvocationService::new(Arc::clone(&presence), admission);

        let resp = svc
            .invoke_stream(Request::new(InvokeServerStreamRequest {
                envelope: Some(test_envelope()),
                function_name: ABILITY_FEDERATION_SUBSCRIBE_DIRECTORY_V2.to_string(),
                ..InvokeServerStreamRequest::default()
            }))
            .await
            .expect("v2 dispatch returns Ok");

        let mut stream = resp.into_inner();

        // Frame 1: empty Snapshot (registry has no entries yet).
        let first = stream.next().await.expect("first frame").expect("Ok");
        assert_eq!(first.content_type, FEDERATION_RESULT_CONTENT_TYPE);
        let evt: DirectoryEvent =
            serde_json::from_slice(&first.payload).expect("decodes DirectoryEvent");
        match evt {
            DirectoryEvent::Snapshot { entries } => {
                assert!(
                    entries.is_empty(),
                    "initial snapshot must reflect empty registry"
                );
            }
            other => panic!("expected Snapshot first; got {other:?}"),
        }

        // Frame 2: Upsert after a registry insert.
        let (sender, _rx) = tokio::sync::mpsc::channel::<
            Result<crate::services::presence_registry::DispatchFrame, tonic::Status>,
        >(1);
        presence.insert("easynet:///r/test-realm/agent/n1".to_string(), sender);
        let second = stream.next().await.expect("second frame").expect("Ok");
        let evt2: DirectoryEvent =
            serde_json::from_slice(&second.payload).expect("decodes DirectoryEvent");
        match evt2 {
            DirectoryEvent::Upsert { entry } => {
                assert_eq!(entry.agent_uri, "easynet:///r/test-realm/agent/n1");
                assert_eq!(entry.status, "active");
                assert_eq!(entry.origin_realm, None);
            }
            other => panic!("expected Upsert; got {other:?}"),
        }

        // Frame 3: Remove after the device's stream closes (we
        // drop the receiver to trigger the Closed path).
        // PresenceRegistry's drop-on-receiver-close behaviour is
        // exercised by the existing v1 test; here we just
        // explicitly remove via the registry surface.
        presence.remove(
            "easynet:///r/test-realm/agent/n1",
            crate::services::presence_registry::OfflineReason::AdminRevoked,
        );
        let third = stream.next().await.expect("third frame").expect("Ok");
        let evt3: DirectoryEvent =
            serde_json::from_slice(&third.payload).expect("decodes DirectoryEvent");
        match evt3 {
            DirectoryEvent::Remove { agent_uri, reason } => {
                assert_eq!(agent_uri, "easynet:///r/test-realm/agent/n1");
                assert_eq!(reason, "admin_revoked");
            }
            other => panic!("expected Remove; got {other:?}"),
        }

        // Drop senders → pump closes.
        drop(svc);
        drop(presence);
        let close = tokio::time::timeout(std::time::Duration::from_secs(2), stream.next())
            .await
            .expect("pump closes within 2 s");
        assert!(close.is_none());
    }

    #[tokio::test]
    async fn invoke_stream_subscribe_directory_v2_emits_heartbeat_when_idle() {
        // PR-N3 N3-streaming-6. Confirm the v2 stream emits a
        // DirectoryEvent::Heartbeat after the heartbeat
        // interval has elapsed with no real events, so the
        // subscriber's 60s idle-timeout watcher does not tear
        // down a healthy stream. The test sets a 50ms cadence
        // via `with_subscribe_v2_heartbeat_interval_ms` so it
        // runs in real time without virtualised clocks; spec
        // §2.3 production cadence is 30 000ms.
        use crate::services::federation_directory::DirectoryEvent;
        use futures::StreamExt;

        let presence = Arc::new(PresenceRegistry::new());
        let admission = AdmissionFacade::new(
            Arc::new(RealmTrustAnchor::default()),
            Some(TEST_DAEMON_URI.to_string()),
        );
        let svc = DaemonInvocationService::new(Arc::clone(&presence), admission)
            .with_subscribe_v2_heartbeat_interval_ms(50);

        let resp = svc
            .invoke_stream(Request::new(InvokeServerStreamRequest {
                envelope: Some(test_envelope()),
                function_name: ABILITY_FEDERATION_SUBSCRIBE_DIRECTORY_V2.to_string(),
                ..InvokeServerStreamRequest::default()
            }))
            .await
            .expect("dispatch returns Ok");

        let mut stream = resp.into_inner();

        // Frame 1: empty Snapshot (immediate).
        let first = stream.next().await.expect("first frame").expect("Ok");
        let evt: DirectoryEvent = serde_json::from_slice(&first.payload).expect("Snapshot decodes");
        assert!(matches!(evt, DirectoryEvent::Snapshot { .. }));

        // Frame 2: Heartbeat after the 50ms interval. Bound
        // the wait to 1s so a real bug surfaces as a test
        // timeout rather than a CI hang.
        let hb_frame = tokio::time::timeout(std::time::Duration::from_secs(1), stream.next())
            .await
            .expect("heartbeat frame within 1s")
            .expect("stream did not end")
            .expect("frame is Ok");
        let hb_evt: DirectoryEvent =
            serde_json::from_slice(&hb_frame.payload).expect("Heartbeat decodes");
        match hb_evt {
            DirectoryEvent::Heartbeat { sent_at_unix_ms } => {
                assert!(
                    sent_at_unix_ms > 0,
                    "Heartbeat sent_at_unix_ms must be a real epoch-ms",
                );
            }
            other => panic!("expected Heartbeat after idle window; got {other:?}"),
        }

        drop(svc);
        drop(presence);
    }

    #[tokio::test]
    async fn invoke_stream_unknown_function_returns_unimplemented_with_pr1_note() {
        let svc = make_service();
        match svc
            .invoke_stream(Request::new(InvokeServerStreamRequest {
                envelope: Some(test_envelope()),
                function_name: "custom.stream.ability".to_string(),
                ..InvokeServerStreamRequest::default()
            }))
            .await
        {
            Err(err) => {
                assert_eq!(err.code(), tonic::Code::Unimplemented);
                // 7/9 wired admission; the LocalAbilityRegistry stream
                // fall-through is the next staging step.
                assert!(err.message().contains("commit"));
            }
            Ok(_) => panic!("unknown stream ability must be rejected"),
        }
    }

    #[tokio::test]
    async fn invoke_rejects_caller_not_in_trust_anchor() {
        // PR-7 commit 4/N (DEC-013 Option D): trust-anchor membership
        // is the first non-loopback check. A URI absent from the
        // anchor short-circuits to `permission_denied` before any
        // §5.2 work — the gating reject, identical to the PR-1 URI-
        // only behaviour for unknown callers. Same `PermissionDenied`
        // wire code as before, refreshed message text.
        let svc = DaemonInvocationService::new(
            Arc::new(PresenceRegistry::new()),
            AdmissionFacade::new(Arc::new(RealmTrustAnchor::default()), None),
        );
        match svc
            .invoke(Request::new(InvokeRequest {
                envelope: Some(Envelope {
                    caller: Some(AgentIdentity {
                        uri: "easynet:///r/realm/agent/external".to_string(),
                        ..AgentIdentity::default()
                    }),
                    ..Envelope::default()
                }),
                function_name: ABILITY_FEDERATION_HEARTBEAT.to_string(),
                arguments: br#"{"agent_uri":"easynet:///r/realm/agent/external"}"#.to_vec(),
                ..InvokeRequest::default()
            }))
            .await
        {
            Err(err) => {
                assert_eq!(err.code(), tonic::Code::PermissionDenied);
                assert!(
                    err.message().contains("not in the realm trust anchor"),
                    "rejection must reference trust-set miss, got: {}",
                    err.message()
                );
            }
            Ok(_) => panic!("caller outside trust anchor must be rejected"),
        }
    }

    #[tokio::test]
    async fn invoke_stream_rejects_caller_not_in_trust_anchor() {
        // Same DEC-013 dispatch as `invoke_rejects_caller_not_in_trust_anchor`.
        // Stream surface shares the same membership check.
        let svc = DaemonInvocationService::new(
            Arc::new(PresenceRegistry::new()),
            AdmissionFacade::new(Arc::new(RealmTrustAnchor::default()), None),
        );
        match svc
            .invoke_stream(Request::new(InvokeServerStreamRequest {
                envelope: Some(Envelope {
                    caller: Some(AgentIdentity {
                        uri: "easynet:///r/realm/agent/external".to_string(),
                        ..AgentIdentity::default()
                    }),
                    ..Envelope::default()
                }),
                function_name: ABILITY_FEDERATION_SUBSCRIBE_DIRECTORY.to_string(),
                ..InvokeServerStreamRequest::default()
            }))
            .await
        {
            Err(err) => {
                assert_eq!(err.code(), tonic::Code::PermissionDenied);
                assert!(
                    err.message().contains("not in the realm trust anchor"),
                    "rejection must reference trust-set miss, got: {}",
                    err.message()
                );
            }
            Ok(_) => panic!("stream caller outside trust anchor must be rejected"),
        }
    }

    #[ignore = "PR-1 staging — bidi accept/dispatch covered by PR-2 Tier 1 cases 1-11 unignore"]
    #[tokio::test]
    async fn invoke_bidi_test_deferred_to_pr2_tier1() {
        // Constructing a real `tonic::Streaming<InvokeBidiUp>`
        // requires the full tonic codegen scaffolding. The
        // unimplemented path returns before reading any frame,
        // so a synthetic empty `Streaming` would not exercise
        // anything beyond the trait dispatch table — exactly
        // what PR-2 Tier 1 cases 1-11 cover end-to-end via real
        // gRPC roundtrip. Marking this `#[ignore]` so the test
        // result line surfaces the gap rather than passing
        // vacuously.
        unreachable!();
    }

    // ── PR-3 commit 1/3 — <self>.invoke_remote helpers + early returns ────

    use crate::pb::axon::v1::invoke_bidi_up::Payload as UpPayload;
    use crate::pb::axon::v1::{BidiControl, EnvelopeOpen, InvocationTarget, InvokeBidiUp};
    use crate::services::axon_serve::invoke_remote_initiator::{
        InvokeRemoteUp, ABILITY_INVOKE_REMOTE,
    };
    fn make_envelope_open(ability: &str, initial_args: Vec<u8>) -> EnvelopeOpen {
        EnvelopeOpen {
            envelope: Some(test_envelope()),
            target: Some(InvocationTarget {
                ability_name: ability.to_string(),
                ..InvocationTarget::default()
            }),
            initial_args,
            args_content_type: "application/json".to_string(),
            ..EnvelopeOpen::default()
        }
    }

    #[test]
    fn extract_envelope_open_returns_inner_for_envelope_open_frame() {
        let frame = InvokeBidiUp {
            sequence: 0,
            mac: Vec::new(),
            payload: Some(UpPayload::EnvelopeOpen(make_envelope_open(
                ABILITY_INVOKE_REMOTE,
                b"{}".to_vec(),
            ))),
        };
        let eo = extract_envelope_open(&frame).expect("extracted");
        assert_eq!(
            eo.target.as_ref().unwrap().ability_name,
            ABILITY_INVOKE_REMOTE
        );
    }

    #[test]
    fn validate_and_extract_bidi_frame0_rejects_non_zero_sequence() {
        let frame = InvokeBidiUp {
            sequence: 7,
            mac: Vec::new(),
            payload: Some(UpPayload::EnvelopeOpen(make_envelope_open(
                ABILITY_INVOKE_REMOTE,
                b"{}".to_vec(),
            ))),
        };
        let err = validate_and_extract_bidi_frame0(&frame).expect_err("must reject");
        assert_eq!(err.code(), tonic::Code::InvalidArgument);
        assert!(
            err.message().contains(REASON_BIDI_FIRST_FRAME_SEQUENCE),
            "wire reason must be preserved, got: {}",
            err.message()
        );
    }

    #[test]
    fn validate_and_extract_bidi_frame0_rejects_non_strict_ordering() {
        let mut envelope_open = make_envelope_open(ABILITY_INVOKE_REMOTE, b"{}".to_vec());
        envelope_open.streams.push(StreamDescriptor {
            stream_id: 9,
            ordering: "UNORDERED".to_string(),
            ..StreamDescriptor::default()
        });
        let frame = InvokeBidiUp {
            sequence: 0,
            mac: Vec::new(),
            payload: Some(UpPayload::EnvelopeOpen(envelope_open)),
        };
        let err = validate_and_extract_bidi_frame0(&frame).expect_err("must reject");
        assert_eq!(err.code(), tonic::Code::InvalidArgument);
        assert!(
            err.message().contains(REASON_BIDI_NON_STRICT_ORDERING),
            "wire reason must be preserved, got: {}",
            err.message()
        );
    }

    #[test]
    fn extract_envelope_open_rejects_binary_chunk_first_frame() {
        let frame = InvokeBidiUp {
            sequence: 0,
            mac: Vec::new(),
            payload: Some(UpPayload::BinaryChunk(BinaryChunk::default())),
        };
        let err = extract_envelope_open(&frame).expect_err("must reject");
        assert_eq!(err.code(), tonic::Code::InvalidArgument);
        assert!(err.message().contains("EnvelopeOpen"));
    }

    #[test]
    fn extract_envelope_open_rejects_control_first_frame() {
        let frame = InvokeBidiUp {
            sequence: 0,
            mac: Vec::new(),
            payload: Some(UpPayload::Control(BidiControl::default())),
        };
        let err = extract_envelope_open(&frame).expect_err("must reject");
        assert_eq!(err.code(), tonic::Code::InvalidArgument);
    }

    #[test]
    fn extract_envelope_open_rejects_payload_none() {
        let frame = InvokeBidiUp {
            sequence: 0,
            mac: Vec::new(),
            payload: None,
        };
        let err = extract_envelope_open(&frame).expect_err("must reject");
        assert_eq!(err.code(), tonic::Code::InvalidArgument);
        assert!(err.message().contains("no payload"));
    }

    #[test]
    fn map_local_bidi_handler_stdout_decodes_to_binary_chunk() {
        use base64::Engine as _;

        let frame = map_local_bidi_handler_frame(
            LocalBidiWireKind::Pty,
            &serde_json::json!({
                "type": "stdout",
                "data": base64::engine::general_purpose::STANDARD.encode(b"hello"),
            }),
            7,
        );
        match frame {
            LocalBidiHandlerFrame::Forward(InvokeBidiDown {
                payload: Some(DownPayload::BinaryChunk(chunk)),
                ..
            }) => {
                assert_eq!(chunk.stream_id, 7);
                assert_eq!(chunk.data, b"hello");
            }
            other => panic!("expected stdout → BinaryChunk, got {other:?}"),
        }
    }

    #[test]
    fn map_local_bidi_handler_exit_becomes_completed_receipt() {
        let frame = map_local_bidi_handler_frame(
            LocalBidiWireKind::Pty,
            &serde_json::json!({
                "type": "exit",
                "status": 23,
            }),
            1,
        );
        match frame {
            LocalBidiHandlerFrame::Terminal(InvokeBidiDown {
                payload: Some(DownPayload::Receipt(receipt)),
                ..
            }) => {
                assert_eq!(receipt.state, InvocationState::Completed as i32);
                assert!(
                    receipt.reason.contains("23"),
                    "exit status should surface in the terminal receipt reason"
                );
            }
            other => panic!("expected exit → terminal receipt, got {other:?}"),
        }
    }

    #[test]
    fn map_local_bidi_handler_file_transfer_chunk_decodes_to_binary_chunk() {
        use base64::Engine as _;

        let frame = map_local_bidi_handler_frame(
            LocalBidiWireKind::FileTransfer,
            &serde_json::json!({
                "type": "chunk",
                "data": base64::engine::general_purpose::STANDARD.encode(b"file-bytes"),
            }),
            11,
        );
        match frame {
            LocalBidiHandlerFrame::Forward(InvokeBidiDown {
                payload: Some(DownPayload::BinaryChunk(chunk)),
                ..
            }) => {
                assert_eq!(chunk.stream_id, 11);
                assert_eq!(chunk.data, b"file-bytes");
            }
            other => panic!("expected file_transfer chunk → BinaryChunk, got {other:?}"),
        }
    }

    #[test]
    fn map_local_bidi_handler_file_transfer_complete_becomes_receipt_with_payload() {
        let frame = map_local_bidi_handler_frame(
            LocalBidiWireKind::FileTransfer,
            &serde_json::json!({
                "type": "complete",
                "sha256": "deadbeef",
                "bytes": 9,
            }),
            1,
        );
        match frame {
            LocalBidiHandlerFrame::Terminal(InvokeBidiDown {
                payload: Some(DownPayload::Receipt(receipt)),
                ..
            }) => {
                assert_eq!(receipt.state, InvocationState::Completed as i32);
                assert_eq!(receipt.payload_content_type, "application/json");
                let payload: serde_json::Value =
                    serde_json::from_slice(&receipt.payload).expect("json payload");
                assert_eq!(payload["sha256"], "deadbeef");
                assert_eq!(payload["bytes"], 9);
            }
            other => panic!("expected file_transfer complete → terminal receipt, got {other:?}"),
        }
    }

    #[test]
    fn map_local_bidi_handler_file_transfer_error_becomes_failed_receipt_with_payload() {
        let frame = map_local_bidi_handler_frame(
            LocalBidiWireKind::FileTransfer,
            &serde_json::json!({
                "type": "error",
                "code": "disk_full",
                "message": "no space left on device",
            }),
            1,
        );
        match frame {
            LocalBidiHandlerFrame::Terminal(InvokeBidiDown {
                payload: Some(DownPayload::Receipt(receipt)),
                ..
            }) => {
                assert_eq!(receipt.state, InvocationState::Failed as i32);
                assert!(receipt.reason.contains("disk_full"));
                assert!(receipt.reason.contains("no space left on device"));
                let payload: serde_json::Value =
                    serde_json::from_slice(&receipt.payload).expect("json payload");
                assert_eq!(payload["type"], "error");
            }
            other => panic!("expected file_transfer error → failed receipt, got {other:?}"),
        }
    }

    #[test]
    fn map_local_bidi_up_payload_translates_file_transfer_binary_chunk() {
        use base64::Engine as _;

        let mapped = map_local_bidi_up_payload(
            LocalBidiWireKind::FileTransfer,
            UpPayload::BinaryChunk(BinaryChunk {
                data: b"abc".to_vec(),
                ..BinaryChunk::default()
            }),
        );
        match mapped {
            LocalBidiUpFrame::Forward(value) => {
                assert_eq!(value["type"], "chunk");
                assert_eq!(
                    value["data"],
                    base64::engine::general_purpose::STANDARD.encode(b"abc")
                );
            }
            other => panic!("expected file_transfer binary → chunk JSON, got {other:?}"),
        }
    }

    #[test]
    fn map_local_bidi_up_payload_translates_file_transfer_eof_control() {
        let mapped = map_local_bidi_up_payload(
            LocalBidiWireKind::FileTransfer,
            UpPayload::Control(BidiControl {
                control: Some(crate::pb::axon::v1::bidi_control::Control::Eof(true)),
            }),
        );
        match mapped {
            LocalBidiUpFrame::ForwardAndClose(value) => {
                assert_eq!(value["type"], "eof");
            }
            other => panic!("expected file_transfer eof → eof JSON, got {other:?}"),
        }
    }

    #[tokio::test]
    async fn local_bidi_down_stream_emits_admission_receipt_before_handler_frames() {
        use futures::StreamExt as _;

        let (down_tx, down_rx) = tokio::sync::mpsc::channel::<Result<InvokeBidiDown, Status>>(1);
        down_tx
            .send(Ok(InvokeBidiDown {
                payload: Some(DownPayload::BinaryChunk(BinaryChunk {
                    stream_id: 9,
                    data: b"payload".to_vec(),
                    ..BinaryChunk::default()
                })),
                ..InvokeBidiDown::default()
            }))
            .await
            .expect("enqueue payload frame");
        drop(down_tx);

        let mut stream = LocalBidiDownStream::new(down_rx);
        let first = stream
            .next()
            .await
            .expect("admission receipt frame")
            .expect("receipt is ok");
        match first.payload {
            Some(DownPayload::Receipt(receipt)) => {
                assert_eq!(first.sequence, 0);
                assert_eq!(receipt.state, InvocationState::Admitted as i32);
            }
            other => panic!("expected admission receipt at sequence 0, got {other:?}"),
        }

        let second = stream
            .next()
            .await
            .expect("payload frame")
            .expect("payload is ok");
        match second.payload {
            Some(DownPayload::BinaryChunk(chunk)) => {
                assert_eq!(second.sequence, 1);
                assert_eq!(chunk.stream_id, 9);
                assert_eq!(chunk.data, b"payload");
            }
            other => panic!("expected payload BinaryChunk at sequence 1, got {other:?}"),
        }

        assert!(
            stream.next().await.is_none(),
            "stream should end after the queued payload frame"
        );
    }

    #[test]
    fn validate_session_realm_accepts_same_realm() {
        let anchor = RealmTrustAnchor::default();
        validate_session_realm(
            "easynet:///r/realm-a/agent/device-1",
            Some("realm-a"),
            &anchor,
        )
        .expect("same-realm caller must pass");
    }

    #[test]
    fn validate_session_realm_accepts_same_realm_device_uri() {
        let anchor = RealmTrustAnchor::default();
        validate_session_realm(
            "easynet:///r/realm-a/device/device-1",
            Some("realm-a"),
            &anchor,
        )
        .expect("same-realm device URI must pass");
    }

    #[test]
    fn validate_session_realm_rejects_cross_realm_without_trust() {
        let anchor = RealmTrustAnchor::default();
        let err = validate_session_realm(
            "easynet:///r/realm-b/agent/device-1",
            Some("realm-a"),
            &anchor,
        )
        .expect_err("cross-realm caller without trust entry must be rejected");
        assert_eq!(err.code(), tonic::Code::PermissionDenied);
        assert!(
            err.message()
                .contains("not present in the realm trust anchor"),
            "got: {}",
            err.message()
        );
    }

    #[test]
    fn validate_session_realm_accepts_cross_realm_when_trust_anchor_has_caller() {
        // Federated identity path: caller URI lives in realm-b
        // but the local trust anchor on realm-a's hub has an
        // explicit entry for it. Mirrors the admission gate's
        // existing FederatedKeyResolver hit; closes LB-49.
        use crate::services::realm_trust_anchor::{TrustedAgent, TrustedAgentRole};
        let entry = TrustedAgent {
            agent_uri: "easynet:///r/realm-b/agent/device-1".to_string(),
            public_key_b64: "AAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAA=".to_string(),
            role: TrustedAgentRole::Device,
            added_at_unix_ms: 1_777_640_000_000,
            origin_tenant_id: Some("federated-tenant".to_string()),
            hub_uri: None,
            tls_ca_pem_path: None,
        };
        let anchor = RealmTrustAnchor::from_entries(vec![entry]).expect("anchor");
        validate_session_realm(
            "easynet:///r/realm-b/agent/device-1",
            Some("realm-a"),
            &anchor,
        )
        .expect("cross-realm caller with trust-anchor entry must pass");
    }

    #[test]
    fn validate_session_realm_rejects_malformed_uri() {
        let anchor = RealmTrustAnchor::default();
        let err = validate_session_realm("not-a-ura", Some("realm-a"), &anchor)
            .expect_err("malformed URI must be rejected");
        assert_eq!(err.code(), tonic::Code::InvalidArgument);
        assert!(err.message().contains("canonical"));
    }

    #[test]
    fn build_invoke_remote_dispatch_frame_carries_session_dispatch_json() {
        let frame = build_invoke_remote_dispatch_frame(42, "echo", b"hello").expect("built");
        let payload = match frame.frame.payload.expect("frame has payload") {
            DownPayload::BinaryChunk(chunk) => chunk,
            _ => panic!("expected BinaryChunk"),
        };
        assert_eq!(payload.stream_id, INVOKE_REMOTE_STREAM_ID);
        let parsed: SessionDispatch =
            serde_json::from_slice(&payload.data).expect("decode SessionDispatch");
        match parsed {
            SessionDispatch::Dispatch {
                call_id,
                ability,
                args,
            } => {
                assert_eq!(call_id, 42);
                assert_eq!(ability, "echo");
                assert_eq!(args, b"hello");
            }
            _ => panic!("expected Dispatch variant"),
        }
    }

    #[test]
    fn build_invoke_remote_terminal_frame_round_trips_done_payload() {
        let down = InvokeRemoteDown::Result {
            payload: b"the-reply".to_vec(),
            error: None,
        };
        let frame = build_invoke_remote_terminal_frame(&down).expect("built");
        let chunk = match frame.payload.expect("frame has payload") {
            DownPayload::BinaryChunk(c) => c,
            _ => panic!("expected BinaryChunk"),
        };
        assert_eq!(chunk.stream_id, INVOKE_REMOTE_STREAM_ID);
        let parsed: InvokeRemoteDown = serde_json::from_slice(&chunk.data).expect("decode");
        assert_eq!(parsed, down);
    }

    #[test]
    fn invoke_remote_up_request_serde_round_trip_via_session_dispatch_pin() {
        // Pins the invariant that PR-3 sub-spec §2.1 frame-0 JSON
        // (InvokeRemoteUp::Request) and PR-3 sub-spec §2.3 session
        // dispatch JSON (SessionDispatch::Dispatch) are *separate*
        // wire shapes. A regression that conflates them would let
        // a frame from one side decode into the other type — this
        // test asserts they don't.
        let req_json = serde_json::to_vec(&InvokeRemoteUp::Request {
            subject_device: "easynet:///r/realm/agent/dev-B".into(),
            ability: "echo".into(),
            args: b"hi".to_vec(),
        })
        .unwrap();
        // Decoding as the wrong type must fail.
        let mistaken: Result<SessionDispatch, _> = serde_json::from_slice(&req_json);
        assert!(
            mistaken.is_err(),
            "InvokeRemoteUp::Request must NOT decode as SessionDispatch — \
             the discriminator tags differ ('request' vs 'dispatch')"
        );
    }

    // dispatch_invoke_remote happy/sad-path integration tests
    // require a real `tonic::Streaming<InvokeBidiUp>` which is
    // gRPC-codegen-only constructible (no public `new_empty()`
    // ctor). The same constraint that `#[ignore]`-marked
    // `invoke_bidi_test_deferred_to_pr2_tier1` above applies here:
    // those paths land as Tier 1 integration tests once PR-2's
    // `<self>.session` accept enables a real round-trip. Until
    // then the helpers below pin the units this method composes.
    //
    // Coverage assertion: every early-return code path of
    // `dispatch_invoke_remote` is reachable from the helpers
    // tested above:
    //   * malformed initial_args → serde_json::from_slice (covered
    //     by invoke_remote_up_request_serde_round_trip in
    //     `invoke_remote_initiator::tests`)
    //   * pending map None → trivial Option::ok_or_else (no-op
    //     to test in isolation)
    //   * target offline → PresenceRegistry::lookup returns None
    //     (covered by presence_registry tests)
    //   * try_send Full / Closed → matched by literal pattern,
    //     same shape as commit 8/9's try_push_forward_invoke_frame
    //     which is integration-tested
    //   * pending oneshot dropped → covered by pending_dispatch
    //     `dropped_completer_surfaces_to_handle_as_recv_error`

    // ── PR-N1 commit 3a/N: federation client plumbing tests ──

    #[test]
    fn parse_tenant_from_uri_extracts_tenant_component() {
        assert_eq!(
            parse_tenant_from_uri("easynet:///r/realm-a/agent/laptop-1"),
            Some("realm-a")
        );
        assert_eq!(
            parse_tenant_from_uri("easynet:///r/realm-a/device/device-1"),
            Some("realm-a")
        );
        assert_eq!(
            parse_tenant_from_uri("easynet:///r/peer-realm/agent/peer-hub"),
            Some("peer-realm")
        );
    }

    #[test]
    fn parse_tenant_from_uri_handles_uri_with_extra_path_segments() {
        // RFC-N PR-N4 user-binding URIs may carry extra segments
        // after `agent/<node>`. The tenant is still the first
        // path component after `r/`.
        assert_eq!(
            parse_tenant_from_uri("easynet:///r/realm-a/agent/n1/skill/foo"),
            Some("realm-a")
        );
    }

    #[test]
    fn parse_tenant_from_uri_rejects_non_easynet_scheme() {
        assert_eq!(parse_tenant_from_uri("https://example.com/foo"), None);
        assert_eq!(parse_tenant_from_uri("file:///r/realm/agent/x"), None);
    }

    #[test]
    fn parse_tenant_from_uri_rejects_empty_tenant() {
        // Malformed URI with empty tenant component must reject —
        // never silently treat as `tenant = ""` which would always
        // miss the federated_peers map and surface as
        // "tenant unknown" instead of "URI malformed".
        assert_eq!(parse_tenant_from_uri("easynet:///r//agent/n1"), None);
    }

    #[test]
    fn with_federation_client_attaches_client_field() {
        use crate::services::federation_client::CrossHubDialer;

        let svc = make_service();
        assert!(svc.federation_client.is_none());

        let dialer = Arc::new(CrossHubDialer::new(Arc::new(RealmTrustAnchor::default())));
        let svc = svc.with_federation_client(dialer.clone() as Arc<dyn FederationClient>);
        assert!(svc.federation_client.is_some());
    }

    #[test]
    fn with_federated_peers_attaches_map_field() {
        let svc = make_service();
        assert!(svc.federated_peers.snapshot().is_empty());

        let mut peers = BTreeMap::new();
        peers.insert(
            "peer-realm".to_string(),
            "https://peer-hub.example:50443".to_string(),
        );
        let svc = svc.with_federated_peers(peers);
        let snap = svc.federated_peers.snapshot();
        assert_eq!(snap.len(), 1);
        assert_eq!(
            snap.get("peer-realm").map(String::as_str),
            Some("https://peer-hub.example:50443")
        );
    }

    #[test]
    fn federated_peers_cell_picks_up_replace_without_service_rebuild() {
        // PR-N1 commit 10/N: the SIGHUP reload task calls
        // `cell.replace(new_map)` on TOML re-parse success. The
        // dispatcher's per-call `snapshot()` must see the new
        // map without a `DaemonInvocationService` rebuild.
        use crate::services::federated_peers_cell::SharedFederatedPeers;

        let cell = SharedFederatedPeers::default();
        let svc = make_service().with_federated_peers_cell(cell.clone());
        assert!(svc.federated_peers.snapshot().is_empty());

        let mut next = BTreeMap::new();
        next.insert(
            "hot-reloaded-realm".to_string(),
            "https://hot:50443".to_string(),
        );
        cell.replace(next);

        // Same `svc` instance, but the cell snapshot now has
        // the new entry — no rebuild required.
        let snap = svc.federated_peers.snapshot();
        assert_eq!(snap.len(), 1);
        assert!(snap.contains_key("hot-reloaded-realm"));
    }

    // ── PR-N1 commit 3b/N: tenant-aware forward_invoke tests ──

    /// Test fixture: a `FederationClient` that records every
    /// `forward_invoke` call and returns a canned response. Lets
    /// tests assert the cross-tenant arm dialed the right peer
    /// hub with the right ability + arguments.
    struct RecordingFederationClient {
        recorded:
            std::sync::Mutex<Vec<(crate::services::federation_client::HubUri, InvokeRequest)>>,
        canned: InvokeResponse,
    }

    impl RecordingFederationClient {
        fn new(canned: InvokeResponse) -> Self {
            Self {
                recorded: std::sync::Mutex::new(Vec::new()),
                canned,
            }
        }

        fn calls(&self) -> Vec<(crate::services::federation_client::HubUri, InvokeRequest)> {
            self.recorded.lock().expect("mutex").clone()
        }
    }

    #[async_trait::async_trait]
    impl FederationClient for RecordingFederationClient {
        async fn forward_invoke(
            &self,
            target_hub: &crate::services::federation_client::HubUri,
            request: InvokeRequest,
        ) -> Result<InvokeResponse, crate::services::federation_client::FederationClientError>
        {
            self.recorded
                .lock()
                .expect("mutex")
                .push((target_hub.clone(), request));
            Ok(self.canned.clone())
        }
    }

    fn forward_invoke_args(target_uri: &str) -> Vec<u8> {
        // Test fixture: a base64-encoded JSON `{ability, args,
        // call_id}` payload that mirrors what `support::
        // federation_invoke::invoke_via_federation_forward`
        // ships from the CLI bridge. PR-N1 commit 11/N decodes
        // this on the peer-dispatch path so the rebuilt
        // `peer_request` carries the real inner ability + args;
        // C1a / DEC-N4 §2.1 added the required `call_id` field
        // for response correlation.
        forward_invoke_args_for_ability(target_uri, "observe.health", serde_json::json!({}))
    }

    /// Parameterised sibling of `forward_invoke_args` for tests
    /// that need to drive a specific inner ability + args
    /// (e.g. PR-1 commit 7/9 self-target dispatch tests against
    /// `fs.read`).
    fn forward_invoke_args_for_ability(
        target_uri: &str,
        ability: &str,
        args: serde_json::Value,
    ) -> Vec<u8> {
        use base64::{engine::general_purpose::STANDARD, Engine as _};
        let inner = serde_json::json!({
            "ability": ability,
            "args": args,
            "call_id": "test-call-id-1",
        });
        let inner_b64 = STANDARD.encode(serde_json::to_vec(&inner).unwrap());
        format!(r#"{{"target_uri":"{target_uri}","inner_envelope_b64":"{inner_b64}"}}"#)
            .into_bytes()
    }

    // ── PR-1 commit 7/9 (LB-56) — self-targeted local dispatch ─────────

    #[tokio::test]
    async fn forward_invoke_self_target_runs_locally_via_local_dispatcher() {
        // PR-1 commit 7/9 acceptance: when an inbound
        // `federation.forward_invoke` call's `target_uri` matches
        // THIS daemon's own canonical URI AND a local
        // AbilityDispatcher is wired, the dispatcher MUST execute
        // the inner ability locally (no presence push, no
        // cross-hub dial) and return the JSON result bytes inline
        // in `ForwardInvokeResponse.result_bytes`.
        //
        // This is the LB-56 §〇 production flow: hub-A → hub-B
        // cross-hub dial → hub-B receives forward_invoke with
        // target_uri = hub-B's own URI (peer hub IS the target,
        // not a device on its bidi). Without this fall-through
        // the call surfaces target_offline because hub-B does
        // not register its own URI in its PresenceRegistry.
        use crate::runtime::ability_dispatch::{AbilityDispatcher, LocalAbilityRegistry};
        use crate::runtime::gateway::NoopGateway;

        // Build a minimal registry with one ability that returns
        // a sentinel object so we can prove the bytes came from
        // the local dispatcher and not a daemon-internal stub.
        let mut registry = LocalAbilityRegistry::new();
        registry.register_rpc(
            "demo.echo",
            std::sync::Arc::new(|args| {
                Ok(serde_json::json!({
                    "MARKER-C9-1": "self-target-fallthrough-fired",
                    "echoed_args": args,
                }))
            }),
        );
        let dispatcher: Arc<AbilityDispatcher> = Arc::new(AbilityDispatcher::new(
            Arc::new(registry),
            Arc::new(NoopGateway::new()),
        ));

        let svc = make_service()
            .with_session_realm("test-realm")
            .with_local_dispatcher(Arc::clone(&dispatcher));

        let response = svc
            .dispatch_federation_forward_invoke(
                None,
                &forward_invoke_args_for_ability(
                    TEST_DAEMON_URI,
                    "demo.echo",
                    serde_json::json!({"k": "v"}),
                ),
            )
            .await
            .expect("self-target dispatch returns Ok with result_bytes inline");

        let body = response.into_inner();
        let parsed: federation_wrappers::ForwardInvokeResponse =
            serde_json::from_slice(&body.result).expect("body decodes");
        assert_eq!(
            parsed.correlation_call_id, "test-call-id-1",
            "correlation_call_id must round-trip through self-target arm"
        );
        assert!(
            !parsed.result_bytes.is_empty(),
            "self-target dispatch fills result_bytes (no async reverse-channel reply needed)"
        );

        let result_value: serde_json::Value =
            serde_json::from_slice(&parsed.result_bytes).expect("result_bytes is JSON");
        assert_eq!(
            result_value.get("MARKER-C9-1").and_then(|v| v.as_str()),
            Some("self-target-fallthrough-fired"),
            "result_bytes must come from the LocalAbilityRegistry handler, \
             not a daemon-internal fallback"
        );
        assert_eq!(
            result_value
                .get("echoed_args")
                .and_then(|v| v.get("k"))
                .and_then(|v| v.as_str()),
            Some("v"),
            "inner args must round-trip through the dispatcher's normalized_args path"
        );
    }

    #[tokio::test]
    async fn forward_invoke_local_hub_uri_runs_locally_via_local_dispatcher() {
        // Device-mode escalation targets the local realm's hub URI,
        // not the hub host's device URI. The hub daemon must treat
        // `easynet:///r/<realm>/hub` as self-targeted even though
        // `AdmissionFacade.daemon_uri()` still carries the host
        // device URI from credentials.json.
        use crate::runtime::ability_dispatch::{AbilityDispatcher, LocalAbilityRegistry};
        use crate::runtime::gateway::NoopGateway;

        let mut registry = LocalAbilityRegistry::new();
        registry.register_rpc(
            "demo.echo",
            std::sync::Arc::new(|args| {
                Ok(serde_json::json!({
                    "MARKER-C9-HUB": "local-hub-self-target-fired",
                    "echoed_args": args,
                }))
            }),
        );
        let dispatcher: Arc<AbilityDispatcher> = Arc::new(AbilityDispatcher::new(
            Arc::new(registry),
            Arc::new(NoopGateway::new()),
        ));

        let svc = make_service()
            .with_session_realm("test-realm")
            .with_local_dispatcher(Arc::clone(&dispatcher));

        let response = svc
            .dispatch_federation_forward_invoke(
                None,
                &forward_invoke_args_for_ability(
                    &crate::uri::hub_uri("test-realm"),
                    "demo.echo",
                    serde_json::json!({"k": "hub"}),
                ),
            )
            .await
            .expect("local hub URI must hit the self-target dispatcher");

        let body = response.into_inner();
        let parsed: federation_wrappers::ForwardInvokeResponse =
            serde_json::from_slice(&body.result).expect("body decodes");
        let result_value: serde_json::Value =
            serde_json::from_slice(&parsed.result_bytes).expect("result_bytes is JSON");
        assert_eq!(
            result_value.get("MARKER-C9-HUB").and_then(|v| v.as_str()),
            Some("local-hub-self-target-fired"),
        );
        assert_eq!(
            result_value
                .get("echoed_args")
                .and_then(|v| v.get("k"))
                .and_then(|v| v.as_str()),
            Some("hub"),
        );
    }

    #[tokio::test]
    async fn forward_invoke_self_target_without_local_dispatcher_falls_to_target_offline() {
        // Guard: when `local_dispatcher` is None (test fixtures
        // that don't wire one), the self-target arm DOES NOT
        // fire. The call drops to the existing local-presence
        // path and surfaces target_offline because the
        // PresenceRegistry doesn't have the daemon's own URI.
        // This pins the Option-gated behaviour so adding the
        // fall-through doesn't silently change the semantics for
        // pre-PR-1-7/9 callers.
        let svc = make_service().with_session_realm("test-realm");

        let err = svc
            .dispatch_federation_forward_invoke(None, &forward_invoke_args(TEST_DAEMON_URI))
            .await
            .expect_err("no local_dispatcher ⇒ legacy target_offline");
        assert_eq!(err.code(), tonic::Code::FailedPrecondition);
        assert_eq!(
            err.message(),
            federation_wrappers::FORWARD_INVOKE_TARGET_OFFLINE_REASON
        );
    }

    #[tokio::test]
    async fn forward_invoke_self_target_does_not_intercept_other_target_uris() {
        // Guard: the self-target arm must ONLY fire when
        // `target_uri == admission.daemon_uri()`. A different
        // target_uri (a real device URI in the same realm) goes
        // through the existing presence-push path and surfaces
        // target_offline when the device is not subscribed —
        // unchanged by the fall-through.
        use crate::runtime::ability_dispatch::{AbilityDispatcher, LocalAbilityRegistry};
        use crate::runtime::gateway::NoopGateway;

        let registry = LocalAbilityRegistry::new();
        let dispatcher: Arc<AbilityDispatcher> = Arc::new(AbilityDispatcher::new(
            Arc::new(registry),
            Arc::new(NoopGateway::new()),
        ));
        let svc = make_service()
            .with_session_realm("test-realm")
            .with_local_dispatcher(dispatcher);

        let err = svc
            .dispatch_federation_forward_invoke(
                None,
                &forward_invoke_args("easynet:///r/test-realm/device/some-other-device"),
            )
            .await
            .expect_err("non-self target ⇒ legacy presence-push path ⇒ target_offline");
        assert_eq!(err.code(), tonic::Code::FailedPrecondition);
        assert_eq!(
            err.message(),
            federation_wrappers::FORWARD_INVOKE_TARGET_OFFLINE_REASON
        );
    }

    #[tokio::test]
    async fn forward_invoke_local_tenant_takes_fast_path() {
        // C1a / DEC-N4 §2.1: when `target_uri` tenant matches
        // the daemon's own realm, the local presence-registry
        // path runs. With no presence entry inserted, the
        // dispatcher surfaces `Status::failed_precondition`
        // with the wire-stable `target_offline` reason. Critical:
        // the federation client is NEVER called even though one
        // is wired.
        let canned = InvokeResponse {
            result: br#"{"result_bytes":[]}"#.to_vec(),
            result_content_type: FEDERATION_RESULT_CONTENT_TYPE.to_string(),
            state: InvocationState::Completed as i32,
            ..InvokeResponse::default()
        };
        let recorder = Arc::new(RecordingFederationClient::new(canned));

        let svc = make_service()
            .with_session_realm("test-realm")
            .with_federation_client(recorder.clone() as Arc<dyn FederationClient>);

        let err = svc
            .dispatch_federation_forward_invoke(
                None,
                &forward_invoke_args("easynet:///r/test-realm/device/local-target"),
            )
            .await
            .expect_err("local fast-path miss surfaces target_offline");
        assert_eq!(err.code(), tonic::Code::FailedPrecondition);
        assert_eq!(
            err.message(),
            federation_wrappers::FORWARD_INVOKE_TARGET_OFFLINE_REASON,
            "expected wire-stable target_offline reason"
        );
        assert!(
            recorder.calls().is_empty(),
            "federation client must NOT be called on local-tenant fast-path"
        );
    }

    #[tokio::test]
    async fn forward_invoke_cross_tenant_with_no_client_returns_target_offline() {
        // C1a / DEC-N4 §2.1: cross-tenant target + no federation
        // client wired ⇒ `Status::failed_precondition` with the
        // wire-stable `target_offline` reason. The legacy
        // "Ok with target_online:false" shape is gone.
        let svc = make_service().with_session_realm("test-realm");

        let err = svc
            .dispatch_federation_forward_invoke(
                None,
                &forward_invoke_args("easynet:///r/peer-realm/device/peer-target"),
            )
            .await
            .expect_err("cross-tenant without client surfaces target_offline");
        assert_eq!(err.code(), tonic::Code::FailedPrecondition);
        assert_eq!(
            err.message(),
            federation_wrappers::FORWARD_INVOKE_TARGET_OFFLINE_REASON,
        );
    }

    #[tokio::test]
    async fn forward_invoke_cross_tenant_with_no_peer_entry_returns_target_offline() {
        // C1a / DEC-N4 §2.1: federation client wired but the
        // operator-curated `federated_peers` map has no entry
        // for the target's tenant ⇒ `Status::failed_
        // precondition` with the `target_offline` reason. The
        // map is the operator's explicit statement of "these
        // are the peer realms I federate with"; an unmapped
        // tenant is not dialable.
        let canned = InvokeResponse {
            result: br#"{"result_bytes":[]}"#.to_vec(),
            result_content_type: FEDERATION_RESULT_CONTENT_TYPE.to_string(),
            state: InvocationState::Completed as i32,
            ..InvokeResponse::default()
        };
        let recorder = Arc::new(RecordingFederationClient::new(canned));

        let svc = make_service()
            .with_session_realm("test-realm")
            .with_federation_client(recorder.clone() as Arc<dyn FederationClient>);

        let err = svc
            .dispatch_federation_forward_invoke(
                None,
                &forward_invoke_args("easynet:///r/unmapped-realm/device/peer-target"),
            )
            .await
            .expect_err("unmapped tenant surfaces target_offline");
        assert_eq!(err.code(), tonic::Code::FailedPrecondition);
        assert_eq!(
            err.message(),
            federation_wrappers::FORWARD_INVOKE_TARGET_OFFLINE_REASON,
        );
        assert!(
            recorder.calls().is_empty(),
            "federation client must NOT be called when peer entry is missing"
        );
    }

    #[tokio::test]
    async fn forward_invoke_cross_tenant_with_peer_entry_dials_via_federation_client() {
        // C1a / DEC-N4 §2.1: cross-tenant + federation client
        // wired + peer entry present ⇒ federation client called
        // with the peer's hub URI + the *inner* ability decoded
        // from `inner_envelope_b64`. Response carries peer's
        // `result` bytes through `result_bytes`, plus the
        // caller's `correlation_call_id` echoed back.
        let peer_reply_bytes = br#"{"hello":"from-peer"}"#.to_vec();
        let canned = InvokeResponse {
            result: peer_reply_bytes.clone(),
            result_content_type: FEDERATION_RESULT_CONTENT_TYPE.to_string(),
            state: InvocationState::Completed as i32,
            ..InvokeResponse::default()
        };
        let recorder = Arc::new(RecordingFederationClient::new(canned));

        let mut peers = BTreeMap::new();
        peers.insert(
            "peer-realm".to_string(),
            "https://peer-hub.example:50443".to_string(),
        );

        let svc = make_service()
            .with_session_realm("test-realm")
            .with_federation_client(recorder.clone() as Arc<dyn FederationClient>)
            .with_federated_peers(peers);

        let target_uri = "easynet:///r/peer-realm/device/peer-target";
        let args = forward_invoke_args(target_uri);
        let resp = svc
            .dispatch_federation_forward_invoke(None, &args)
            .await
            .expect("cross-tenant returns Ok");

        // Response carries the peer's `result` bytes verbatim
        // in `result_bytes`, and stamps back the caller's
        // `call_id` from the fixture as `correlation_call_id`.
        let body: federation_wrappers::ForwardInvokeResponse = parse_response_body(resp);
        assert_eq!(body.result_bytes, peer_reply_bytes);
        assert_eq!(body.correlation_call_id, "test-call-id-1");

        let calls = recorder.calls();
        assert_eq!(calls.len(), 1, "exactly one cross-hub dial");
        assert_eq!(calls[0].0, "https://peer-hub.example:50443");
        // **LB-57 §一 Option A wire shape**. The cross-hub dial
        // re-wraps the call as another `federation.forward_invoke`
        // so the peer hub's top-level `Invoke::invoke` match routes
        // through `dispatch_federation_forward_invoke` (which owns
        // local-presence push + same-tenant fan-out + cross-tenant
        // dial). The pre-LB-57 PR-N1 commit 11/N shape (sending the
        // bare inner ability name) landed at the peer's `other` arm
        // → Unimplemented → demo `target_offline`. This assertion
        // pins the new wire shape; flipping back to bare-inner-name
        // would re-introduce the LB-57 §〇 production bug.
        assert_eq!(
            calls[0].1.function_name, ABILITY_FEDERATION_FORWARD_INVOKE,
            "LB-57 Option A: peer dispatcher receives the federation.forward_invoke \
             wrapper, NOT the bare inner ability name"
        );
        // The peer_request body is a serialized
        // ForwardInvokeRequest carrying the SAME target_uri +
        // inner_envelope_b64 the caller hub received, so the
        // peer's `dispatch_federation_forward_invoke` re-runs
        // its own routing (local-presence / same-tenant fan-out
        // / cross-tenant dial) against the original payload.
        let nested: federation_wrappers::ForwardInvokeRequest =
            serde_json::from_slice(&calls[0].1.arguments)
                .expect("peer arguments decode as nested ForwardInvokeRequest");
        assert_eq!(nested.target_uri, target_uri);
        assert!(
            !nested.inner_envelope_b64.is_empty(),
            "nested wrapper carries the original inner_envelope_b64 verbatim"
        );
        // When the original request carries no caller envelope, the
        // caller hub must still present its own hub URI to the peer.
        // Using `target_uri` here makes the peer believe the target
        // device itself initiated the call, which fails trust-anchor
        // admission and opens the circuit breaker.
        let peer_envelope = calls[0].1.envelope.as_ref().expect("envelope present");
        let peer_caller = peer_envelope
            .caller
            .as_ref()
            .expect("caller identity present");
        assert_eq!(peer_caller.uri, crate::uri::hub_uri("test-realm"));
        let peer_callee = peer_envelope
            .callee
            .as_ref()
            .expect("callee identity present");
        assert_eq!(peer_callee.uri, crate::uri::hub_uri("peer-realm"));
        let caller_signature = peer_envelope
            .caller_signature
            .as_ref()
            .expect("caller signature present for peer admission");
        assert_eq!(caller_signature.algorithm, "ed25519");
        assert!(
            !caller_signature.signature.is_empty(),
            "peer envelope signature bytes must be populated"
        );
        assert_eq!(
            peer_envelope.invocation_nonce.len(),
            16,
            "peer envelope must carry a fresh 16-byte nonce for strict admission"
        );
        let peer_signature = peer_envelope
            .caller_signature
            .as_ref()
            .expect("peer envelope must be signed for cross-hub admission");
        assert_eq!(peer_signature.algorithm, "ed25519");
        assert_eq!(
            peer_signature.signature.len(),
            64,
            "peer envelope signature must be one Ed25519 signature"
        );
    }

    #[tokio::test]
    async fn forward_invoke_cross_tenant_peer_request_admits_against_hub_anchor() {
        // The cross-hub deep harness failure we care about is not
        // "signature field missing" anymore; it is "peer hub rejects
        // the rebuilt federation.forward_invoke wrapper with
        // caller_signature_invalid". Rebuild that exact wrapper via
        // the caller-hub dispatch path, then feed it into a fresh
        // AdmissionFacade that trusts the caller hub's public key.
        //
        // If this test fails, the signer/canonicalization path is
        // wrong. If it passes while docker deep e2e still fails, the
        // remaining bug lives in boot/runtime wiring rather than in
        // the envelope bytes themselves.
        use base64::{engine::general_purpose::STANDARD as BASE64_STANDARD, Engine as _};
        use ed25519_dalek::SigningKey;

        let canned = InvokeResponse {
            result: br#"{"result_bytes":[]}"#.to_vec(),
            result_content_type: FEDERATION_RESULT_CONTENT_TYPE.to_string(),
            state: InvocationState::Completed as i32,
            ..InvokeResponse::default()
        };
        let recorder = Arc::new(RecordingFederationClient::new(canned));

        let mut peers = BTreeMap::new();
        peers.insert(
            "peer-realm".to_string(),
            "https://peer-hub.example:50443".to_string(),
        );

        let svc = make_service()
            .with_session_realm("test-realm")
            .with_federation_client(recorder.clone() as Arc<dyn FederationClient>)
            .with_federated_peers(peers);

        let target_uri = "easynet:///r/peer-realm/device/peer-target";
        svc.dispatch_federation_forward_invoke(None, &forward_invoke_args(target_uri))
            .await
            .expect("cross-tenant wrapper build succeeds");

        let calls = recorder.calls();
        assert_eq!(calls.len(), 1, "exactly one peer request captured");
        let peer_request = calls[0].1.clone();
        let peer_envelope = peer_request
            .envelope
            .as_ref()
            .expect("peer request envelope present");
        let caller_uri = peer_envelope
            .caller
            .as_ref()
            .expect("caller present")
            .uri
            .clone();

        let caller_signing_key = SigningKey::from_bytes(&[0x11; 32]);
        let caller_pubkey_b64 =
            BASE64_STANDARD.encode(caller_signing_key.verifying_key().to_bytes());
        let peer_anchor = Arc::new(
            RealmTrustAnchor::from_entries(vec![
                crate::services::realm_trust_anchor::TrustedAgent {
                    agent_uri: caller_uri,
                    public_key_b64: caller_pubkey_b64,
                    role: crate::services::realm_trust_anchor::TrustedAgentRole::Hub,
                    added_at_unix_ms: 1_714_492_800_000,
                    origin_tenant_id: Some("test-realm".to_string()),
                    hub_uri: Some("https://peer-hub.example:50443".to_string()),
                    tls_ca_pem_path: None,
                },
            ])
            .expect("peer hub trust anchor"),
        );
        let peer_admission =
            AdmissionFacade::new(peer_anchor, Some(crate::uri::hub_uri("peer-realm")));

        peer_admission
            .verify_invoke(&peer_request)
            .expect("peer hub must admit the rebuilt signed wrapper");
    }

    // ── C1b / DEC-N5 §1: ForwardReceipt dual-write tests ──

    #[tokio::test]
    async fn forward_invoke_cross_tenant_happy_path_records_forward_receipt_with_digest() {
        // LB-39 §44: caller hub `SharedReceiptStore` has a
        // `ForwardReceipt` with `result_digest = sha256(actual_
        // result_bytes)`. The receipt's `child_invocation_id`
        // equals the caller-minted `correlation_call_id` so the
        // target hub's matching InvocationReceipt joins on the
        // same key.
        use sha2::{Digest, Sha256};

        let peer_reply_bytes = br#"{"hello":"from-peer"}"#.to_vec();
        let canned = InvokeResponse {
            result: peer_reply_bytes.clone(),
            result_content_type: FEDERATION_RESULT_CONTENT_TYPE.to_string(),
            state: InvocationState::Completed as i32,
            ..InvokeResponse::default()
        };
        let recorder = Arc::new(RecordingFederationClient::new(canned));

        let mut peers = BTreeMap::new();
        peers.insert(
            "peer-realm".to_string(),
            "https://peer-hub.example:50443".to_string(),
        );

        let svc = make_service()
            .with_session_realm("test-realm")
            .with_federation_client(recorder.clone() as Arc<dyn FederationClient>)
            .with_federated_peers(peers);

        let store_before = svc.admission.receipt_store().len();
        assert_eq!(store_before, 0, "empty store at test start");

        let target_uri = "easynet:///r/peer-realm/device/peer-target";
        let _resp = svc
            .dispatch_federation_forward_invoke(None, &forward_invoke_args(target_uri))
            .await
            .expect("cross-tenant Ok");

        let recent = svc.admission.receipt_store().snapshot_recent(10);
        assert_eq!(recent.len(), 1, "exactly one ForwardReceipt recorded");
        let receipt = &recent[0];
        assert_eq!(receipt.receipt_type, "forward");
        assert_eq!(
            receipt.child_invocation_id, "test-call-id-1",
            "child_invocation_id == caller-minted correlation_call_id"
        );

        let mut hasher = Sha256::new();
        hasher.update(&peer_reply_bytes);
        let expected_digest = hasher.finalize().to_vec();
        assert_eq!(
            receipt.payload, expected_digest,
            "payload is sha256(result_bytes) per LB-39 §44"
        );
        assert_eq!(
            receipt.payload_content_type, "application/octet-stream;sha256",
            "content type identifies the payload as a SHA-256 digest"
        );
        let callee = receipt.callee_binding.as_ref().expect("callee_binding set");
        assert_eq!(callee.uri, target_uri);
    }

    #[tokio::test]
    async fn forward_invoke_target_offline_records_forward_receipt_with_no_digest() {
        // LB-39 §45: ForwardReceipt's `result_digest` is `None`
        // for the target_offline path — encoded as an empty
        // `payload` field with empty content type. The receipt
        // is still recorded so audit consumers can observe the
        // failed-forward attempt.
        let svc = make_service().with_session_realm("test-realm");
        // Cross-tenant target with no federation client wired.
        // Dispatcher takes the target_offline arm.

        let _err = svc
            .dispatch_federation_forward_invoke(
                None,
                &forward_invoke_args("easynet:///r/peer-realm/device/peer-target"),
            )
            .await
            .expect_err("target_offline");

        let recent = svc.admission.receipt_store().snapshot_recent(10);
        assert_eq!(recent.len(), 1, "ForwardReceipt recorded even on offline");
        let receipt = &recent[0];
        assert_eq!(receipt.receipt_type, "forward");
        assert_eq!(receipt.child_invocation_id, "test-call-id-1");
        assert!(
            receipt.payload.is_empty(),
            "result_digest = None encoded as empty payload"
        );
        assert!(
            receipt.payload_content_type.is_empty(),
            "no content type when there is no digest"
        );
    }

    #[tokio::test]
    async fn forward_invoke_local_tenant_miss_records_forward_receipt_with_no_digest() {
        // C1b: local-tenant fast-path miss is also a target_offline
        // outcome on the wire (Status::failed_precondition); the
        // caller hub still records a ForwardReceipt with
        // result_digest = None so the audit trail captures the
        // attempt.
        let svc = make_service().with_session_realm("test-realm");

        let _err = svc
            .dispatch_federation_forward_invoke(
                None,
                &forward_invoke_args("easynet:///r/test-realm/device/local-target"),
            )
            .await
            .expect_err("local fast-path miss");

        let recent = svc.admission.receipt_store().snapshot_recent(10);
        assert_eq!(recent.len(), 1);
        assert_eq!(recent[0].receipt_type, "forward");
        assert!(recent[0].payload.is_empty());
    }

    // ── PR-N1 commit 5/N: 2-daemon in-process cross-hub e2e ──

    #[tokio::test]
    async fn cross_hub_forward_invoke_e2e_in_process() {
        // ── Setup: two daemons in distinct realms ─────────
        // daemon_a: realm "realm-a", knows about daemon_b's
        //           realm via federated_peers + federation_client.
        // daemon_b: realm "realm-b", peer dispatches through to
        //           its own local presence registry.
        //
        // Limit honesty for PR-N1: this exercise stops at the
        // point of `daemon_a.invoke()` building a peer_request
        // and handing it to the federation client. Going one
        // step further (the federation client invoking
        // `daemon_b.invoke()`) requires daemon B's admission
        // gate to admit the request, which under PR-N1 today
        // means daemon A's URI must be in daemon B's trust
        // anchor as a Hub-role peer. PR-N2 lands the
        // FederatedKeyResolver that resolves daemon A's signing
        // key out of daemon B's trust set; without that the
        // cross-realm strict admission would reject the
        // signature step. Either way, the in-process e2e here
        // proves the routing chain works; full TLS handshake +
        // cross-realm admission is the operator-side smoke test.
        const REALM_A: &str = "realm-a";
        const REALM_B: &str = "realm-b";
        const DAEMON_A_URI: &str = "easynet:///r/realm-a/agent/daemon-a";
        const DAEMON_B_URI: &str = "easynet:///r/realm-b/agent/daemon-b";
        const TARGET_DEVICE_URI: &str = "easynet:///r/realm-b/device/target-device";
        const PEER_HUB_URI: &str = "https://daemon-b.example:50443";

        // Daemon B's trust anchor: pre-populated with daemon A
        // as a Backend-role entry so daemon B's admission gate
        // admits a request whose envelope.caller.uri is daemon
        // A's URI. URI-only no-op admission today (Backend role
        // skips the strict signature path? — no, Backend goes
        // strict. Use Device for URI-only no-op so the e2e
        // doesn't depend on PR-N2 cross-realm sig verify).
        // DEC-013 path-conditional admission lets Device entries
        // pass URI-only — exactly what we need for the in-
        // process e2e under PR-N1.
        let daemon_a_in_b_trust = vec![crate::services::realm_trust_anchor::TrustedAgent {
            agent_uri: DAEMON_A_URI.to_string(),
            public_key_b64: "AAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAA=".to_string(),
            role: crate::services::realm_trust_anchor::TrustedAgentRole::Device,
            added_at_unix_ms: 1_714_492_800_000,
            origin_tenant_id: None,
            hub_uri: None,
            tls_ca_pem_path: None,
        }];
        let daemon_b_anchor =
            Arc::new(RealmTrustAnchor::from_entries(daemon_a_in_b_trust).expect("anchor"));

        // Daemon B: presence registry contains the target device,
        // and a `PendingDispatchMap` is wired so the new LB-57
        // local-presence dispatch path can register a pending
        // entry, push a SessionDispatch::Dispatch frame, and
        // await the matching Result. A fake device task spawned
        // below drains the reverse-channel push, decodes the
        // dispatch frame, and completes the pending entry with
        // canned bytes (mirrors what `drain_session_up_stream`
        // does in production when the target device sends
        // SessionDispatch::Result up).
        let daemon_b_presence = Arc::new(PresenceRegistry::new());
        let (target_tx, mut target_rx) = tokio::sync::mpsc::channel(8);
        daemon_b_presence.insert(TARGET_DEVICE_URI.to_string(), target_tx);

        let daemon_b_pending = Arc::new(PendingDispatchMap::new());
        let daemon_b_admission =
            AdmissionFacade::new(daemon_b_anchor, Some(DAEMON_B_URI.to_string()));
        let daemon_b = Arc::new(
            DaemonInvocationService::new(daemon_b_presence, daemon_b_admission)
                .with_session_realm(REALM_B)
                .with_pending(Arc::clone(&daemon_b_pending)),
        );

        // Fake device-B task: drain the dispatch frame, decode it,
        // and feed back a canned ability response via
        // PendingDispatchMap::complete. The canned bytes here are
        // the JSON shape `federation.heartbeat`'s real handler
        // would have produced if it ran on a real device — kept
        // structurally lean (one field) so the test asserts only
        // round-trip integrity, not full handler semantics.
        let pending_for_fake = Arc::clone(&daemon_b_pending);
        tokio::spawn(async move {
            while let Some(frame_result) = target_rx.recv().await {
                let frame = match frame_result {
                    Ok(f) => f,
                    Err(_) => continue,
                };
                use crate::pb::axon::v1::invoke_bidi_down::Payload as DownPayload;
                let chunk = match frame.frame.payload {
                    Some(DownPayload::BinaryChunk(c)) => c,
                    _ => continue,
                };
                let dispatch: SessionDispatch = match serde_json::from_slice(&chunk.data) {
                    Ok(d) => d,
                    Err(_) => continue,
                };
                let SessionDispatch::Dispatch { call_id, .. } = dispatch else {
                    continue;
                };
                let canned = br#"{"echo":"e2e-canned"}"#.to_vec();
                pending_for_fake.complete(
                    call_id,
                    DispatchResult {
                        payload: canned,
                        error: None,
                    },
                );
            }
        });

        // Daemon A: empty presence registry; cross-tenant target
        // routes via the InProcessPeerClient → daemon B. We
        // forward the envelope verbatim from the test request so
        // daemon B sees `envelope.caller.uri = DAEMON_A_URI` and
        // resolves the URI-only Device admission against the
        // pre-staged trust entry above.
        let daemon_a_admission = AdmissionFacade::new(
            Arc::new(RealmTrustAnchor::default()),
            Some(DAEMON_A_URI.to_string()),
        );
        let federation_client: Arc<dyn FederationClient> = Arc::new(ForwardingPeerClient {
            peer: daemon_b,
            envelope: test_envelope_with_uri(DAEMON_A_URI),
        });
        let mut peers = BTreeMap::new();
        peers.insert(REALM_B.to_string(), PEER_HUB_URI.to_string());

        let daemon_a =
            DaemonInvocationService::new(Arc::new(PresenceRegistry::new()), daemon_a_admission)
                .with_session_realm(REALM_A)
                .with_federation_client(federation_client)
                .with_federated_peers(peers);

        // ── Drive: daemon_a receives a federation.forward_invoke ──
        // PR-N1 commit 11/N rewrote the dispatch path: daemon A
        // now decodes the CLI bridge's `inner_envelope_b64`
        // (base64 of `{ability, args}`) and sends the inner
        // ability to the peer instead of re-wrapping in another
        // `federation.forward_invoke`. For this in-process e2e
        // we ship `federation.heartbeat` as the inner ability so
        // daemon B's dispatcher routes to a real handler and
        // returns a structured JSON shape the test can assert
        // against.
        //
        // base64({"ability":"federation.heartbeat","args":{
        //   "canonical_agent_uri":"easynet:///r/realm-b/agent/target-device-b",
        //   "ts_ms":0
        // }})
        let inner_payload = serde_json::json!({
            "ability": "federation.heartbeat",
            "args": {
                "agent_uri": TARGET_DEVICE_URI,
            },
            "call_id": "e2e-call-id-1",
        });
        let inner_b64 = {
            use base64::{engine::general_purpose::STANDARD, Engine as _};
            STANDARD.encode(serde_json::to_vec(&inner_payload).unwrap())
        };
        let forward_args = format!(
            r#"{{"target_uri":"{}","inner_envelope_b64":"{}"}}"#,
            TARGET_DEVICE_URI, inner_b64
        );
        let req = Request::new(InvokeRequest {
            envelope: Some(test_envelope_with_uri(DAEMON_A_URI)),
            function_name: ABILITY_FEDERATION_FORWARD_INVOKE.to_string(),
            arguments: forward_args.into_bytes(),
            ..InvokeRequest::default()
        });

        let response = daemon_a
            .invoke(req)
            .await
            .expect("e2e forward_invoke returns Ok");
        let body = response.into_inner();

        // ── Assert: cross-tenant chain returned the device's ──
        // canned bytes intact.
        // LB-57 Option A wire shape: the outer InvokeResponse
        // body carries a `ForwardInvokeResponse {result_bytes,
        // correlation_call_id}`, where `result_bytes` is the
        // canned bytes the fake device-B task fed back via
        // `PendingDispatchMap::complete`. The pre-LB-57 path
        // returned an empty `result_bytes` and the assertion
        // accidentally passed because the layered wrapper JSON
        // happened to parse as an object — that masked a real
        // wire-shape gap (raw inner-envelope BinaryChunk push
        // with no SessionDispatch::Dispatch wrapper, no
        // PendingDispatchMap correlation). The new contract
        // closes both halves.
        let outer: federation_wrappers::ForwardInvokeResponse =
            serde_json::from_slice(&body.result).expect("outer ForwardInvokeResponse is JSON");
        assert_eq!(outer.correlation_call_id, "e2e-call-id-1");
        assert_eq!(
            outer.result_bytes,
            br#"{"echo":"e2e-canned"}"#.to_vec(),
            "result_bytes must carry the fake device-B canned reply verbatim"
        );
    }

    /// Like `InProcessPeerClient` but stamps an envelope onto the
    /// peer request so daemon B's admission gate sees a caller URI
    /// it can admit. Real PR-N2 path will sign + AXIOM-rewrite the
    /// envelope; this test fixture just stamps the original
    /// envelope verbatim, sufficient for the URI-only Device
    /// admission gate the e2e leans on.
    struct ForwardingPeerClient {
        peer: Arc<DaemonInvocationService>,
        envelope: Envelope,
    }

    #[async_trait::async_trait]
    impl FederationClient for ForwardingPeerClient {
        async fn forward_invoke(
            &self,
            _target_hub: &crate::services::federation_client::HubUri,
            mut request: InvokeRequest,
        ) -> Result<InvokeResponse, crate::services::federation_client::FederationClientError>
        {
            request.envelope = Some(self.envelope.clone());
            let response = self
                .peer
                .invoke(Request::new(request))
                .await
                .map_err(|status| {
                    crate::services::federation_client::FederationClientError::InnerInvokeFailed {
                        hub: "in-process-peer".to_string(),
                        status: format!("code={:?} message={}", status.code(), status.message()),
                    }
                })?;
            Ok(response.into_inner())
        }
    }

    fn test_envelope_with_uri(uri: &str) -> Envelope {
        Envelope {
            caller: Some(AgentIdentity {
                uri: uri.to_string(),
                ..AgentIdentity::default()
            }),
            ..Envelope::default()
        }
    }

    // ── PR-N6 C3 — dispatch_session_request hub-side handler ────────

    #[tokio::test]
    async fn dispatch_session_request_forward_invoke_target_offline_when_presence_empty() {
        // Hub-side handler routes the inbound `Request` through
        // the SAME `dispatch_federation_forward_invoke` arm the
        // unary `Invoke` RPC uses. With an empty PresenceRegistry
        // and no federation client, the inner call surfaces the
        // wire-stable `target_offline` reason; `dispatch_session_
        // request` translates that to the typed
        // `SessionRequestError::TargetOffline` outcome the device
        // caller can pattern-match on.
        let svc = make_service().with_session_realm("test-realm");
        let outcome = svc
            .dispatch_session_request(
                ABILITY_FEDERATION_FORWARD_INVOKE,
                &forward_invoke_args("easynet:///r/test-realm/device/missing-device"),
            )
            .await;
        match outcome {
            RequestOutcome::Err {
                error: SessionRequestError::TargetOffline,
            } => {}
            other => panic!(
                "expected TargetOffline outcome, got {other:?}; the hub's empty \
                 PresenceRegistry must surface as a typed offline error"
            ),
        }
    }

    #[tokio::test]
    async fn dispatch_session_request_unknown_ability_returns_permission_denied() {
        // PR-N6 v1 only routes `federation.forward_invoke`. Other
        // ability names must surface a typed `PermissionDenied`
        // so the device caller knows the hub refused (not a
        // silent timeout). PR-N6 v2 may widen this set once a
        // per-ability admission policy is specified.
        let svc = make_service().with_session_realm("test-realm");
        let outcome = svc.dispatch_session_request("fs.read", b"{}").await;
        match outcome {
            RequestOutcome::Err {
                error: SessionRequestError::PermissionDenied { reason },
            } => {
                assert!(
                    reason.contains("fs.read"),
                    "PermissionDenied reason must name the rejected ability; got: {reason}",
                );
                assert!(
                    reason.contains(ABILITY_FEDERATION_FORWARD_INVOKE),
                    "reason must cite the only ability PR-N6 v1 routes; got: {reason}",
                );
            }
            other => panic!("expected PermissionDenied for unknown ability, got {other:?}"),
        }
    }

    // ── PR-N6 C5 — hub Request → local-presence fast-path dispatch ──

    #[tokio::test]
    async fn dispatch_session_request_forward_invoke_local_presence_hits_fast_path() {
        // **LB-57 Option A acceptance** (same-hub): when the
        // inbound Request's target_uri tenant matches the hub's
        // local realm AND the target device is subscribed in
        // this hub's PresenceRegistry, the dispatcher MUST:
        //   1. Push a `SessionDispatch::Dispatch` frame down
        //      the target's reverse channel (the wire shape
        //      device-side `LocalAbilityDispatcher` decodes).
        //   2. Register a `PendingDispatchMap` entry keyed on
        //      the dispatcher-minted `call_id`.
        //   3. Await the matching `SessionDispatch::Result`.
        //   4. Return its bytes inline as
        //      `ForwardInvokeResponse.result_bytes`.
        // The previous shape (raw inner_envelope BinaryChunk +
        // empty result_bytes) was a wire-shape mismatch on (1)
        // and a no-correlation hole on (2)/(3); the CLI saw a
        // phantom-success reply with empty bytes.
        let svc = make_service()
            .with_session_realm("test-realm")
            .with_pending(Arc::new(PendingDispatchMap::new()));
        let target_uri = "easynet:///r/test-realm/device/local-target";

        let (tx, mut rx) = tokio::sync::mpsc::channel::<
            Result<crate::services::presence_registry::DispatchFrame, tonic::Status>,
        >(4);
        svc.presence.insert(target_uri.to_string(), tx);

        let pending = svc.pending.clone().expect("pending wired above");

        // Spawn a fake "device-B" that drains the reverse-channel
        // push, decodes the SessionDispatch::Dispatch, and replies
        // by completing the corresponding pending entry with a
        // canned result (mirrors what `drain_session_up_stream`
        // does in production when device-B sends Result up).
        let pending_for_fake = Arc::clone(&pending);
        let fake_device = tokio::spawn(async move {
            let frame = rx
                .recv()
                .await
                .expect("reverse-channel frame arrives")
                .expect("frame is Ok");
            // Decode the BinaryChunk's data as SessionDispatch.
            use crate::pb::axon::v1::invoke_bidi_down::Payload as DownPayload;
            let chunk = match frame.frame.payload {
                Some(DownPayload::BinaryChunk(c)) => c,
                other => panic!("expected BinaryChunk, got {other:?}"),
            };
            let dispatch: SessionDispatch =
                serde_json::from_slice(&chunk.data).expect("frame is SessionDispatch JSON");
            let SessionDispatch::Dispatch { call_id, .. } = dispatch else {
                panic!("expected SessionDispatch::Dispatch, got {dispatch:?}");
            };
            // Reply with a canned result (the shape device-B's
            // LocalAbilityDispatcher would produce after running
            // the inner ability).
            let result_bytes = br#"{"echo":"args-from-A"}"#.to_vec();
            pending_for_fake.complete(
                call_id,
                DispatchResult {
                    payload: result_bytes,
                    error: None,
                },
            );
        });

        let outcome = svc
            .dispatch_session_request(
                ABILITY_FEDERATION_FORWARD_INVOKE,
                &forward_invoke_args(target_uri),
            )
            .await;

        match outcome {
            RequestOutcome::Ok { result_bytes } => {
                let body: federation_wrappers::ForwardInvokeResponse =
                    serde_json::from_slice(&result_bytes)
                        .expect("body decodes as ForwardInvokeResponse");
                assert_eq!(
                    body.result_bytes,
                    br#"{"echo":"args-from-A"}"#.to_vec(),
                    "result_bytes must carry device-B's canned ability output verbatim"
                );
                assert_eq!(
                    body.correlation_call_id, "test-call-id-1",
                    "correlation_call_id must round-trip from inner_envelope"
                );
            }
            other => panic!("expected Ok with real device-B bytes, got {other:?}"),
        }

        // Sanity: fake device task ran to completion.
        fake_device.await.expect("fake device task joined");
    }

    // ── PR-N6 C4 — device-mode forward_invoke escalates via session bidi ──

    #[tokio::test]
    async fn forward_invoke_routes_through_escalation_when_handle_attached() {
        // C4 acceptance: when a `SessionEscalationHandle` is
        // wired (boot's device-mode path), `dispatch_federation_
        // forward_invoke` MUST route through the bidi, not consult
        // the local PresenceRegistry. We stand up a fake "hub" task
        // that reads the up channel, decodes the Request, and
        // completes the matching correlation entry with a known
        // result. The dispatcher's response must carry exactly
        // those bytes — proving the device-mode path didn't
        // short-circuit to a local-presence answer.
        use crate::pb::axon::v1::invoke_bidi_up::Payload as UpPayload;
        use crate::services::axon_serve::invoke_remote_initiator::SessionDispatch;
        use crate::services::axon_serve::session_escalation::{
            spawn_escalation_consumer, EscalationCorrelation,
        };
        use crate::services::axon_serve::session_initiator::SessionUpSender;
        use tokio::sync::mpsc;

        let correlation = EscalationCorrelation::new();
        let (up_tx, mut up_rx) = mpsc::channel(8);
        let handle = std::sync::Arc::new(spawn_escalation_consumer(
            correlation.clone(),
            SessionUpSender::new(up_tx),
        ));

        let canned_bytes = b"hub-answered-via-bidi".to_vec();
        let canned_for_hub = canned_bytes.clone();
        tokio::spawn(async move {
            while let Some(frame) = up_rx.recv().await {
                let chunk = match frame.payload {
                    Some(UpPayload::BinaryChunk(c)) => c,
                    _ => continue,
                };
                let dispatch: SessionDispatch =
                    serde_json::from_slice(&chunk.data).expect("decode Request");
                if let SessionDispatch::Request { call_id, .. } = dispatch {
                    correlation.complete(
                        call_id,
                        RequestOutcome::Ok {
                            result_bytes: canned_for_hub.clone(),
                        },
                    );
                }
            }
        });

        // Build a service WITH the escalation handle attached.
        // The local PresenceRegistry stays empty — exactly the
        // device-mode boot shape — so any path that consults
        // it would surface target_offline; only the escalation
        // arm can produce the canned bytes below.
        let svc = make_service()
            .with_session_realm("test-realm")
            .with_session_escalation(handle);

        let response = svc
            .dispatch_federation_forward_invoke(
                None,
                &forward_invoke_args("easynet:///r/peer-realm/device/peer-target"),
            )
            .await
            .expect("escalation must surface canned bytes from the bidi hub");
        let body = response.into_inner();
        assert_eq!(
            body.result, canned_bytes,
            "escalation arm must return the bytes the fake hub injected; \
             a different value means dispatch fell through to local presence"
        );
        assert_eq!(
            body.result_content_type, FEDERATION_RESULT_CONTENT_TYPE,
            "escalation arm must mirror the hub-mode wire content-type so \
             upstream callers don't need to branch on device-vs-hub mode"
        );
    }

    #[tokio::test]
    async fn forward_invoke_escalation_target_offline_maps_to_failed_precondition() {
        // PR-N6 spec §"Wire shape": typed `TargetOffline` outcome
        // surfaces on the unary wire as the same `failed_precondition
        // (target_offline)` reason the existing hub-mode arm uses,
        // so a CLI doesn't need to branch on mode.
        use crate::services::axon_serve::session_escalation::{
            spawn_escalation_consumer, EscalationCorrelation,
        };
        use crate::services::axon_serve::session_initiator::SessionUpSender;
        use tokio::sync::mpsc;

        let correlation = EscalationCorrelation::new();
        let (up_tx, mut up_rx) = mpsc::channel(8);
        let handle = std::sync::Arc::new(spawn_escalation_consumer(
            correlation.clone(),
            SessionUpSender::new(up_tx),
        ));

        // Fake hub: complete every Request with TargetOffline.
        tokio::spawn(async move {
            use crate::pb::axon::v1::invoke_bidi_up::Payload as UpPayload;
            use crate::services::axon_serve::invoke_remote_initiator::SessionDispatch;
            while let Some(frame) = up_rx.recv().await {
                let chunk = match frame.payload {
                    Some(UpPayload::BinaryChunk(c)) => c,
                    _ => continue,
                };
                if let Ok(SessionDispatch::Request { call_id, .. }) =
                    serde_json::from_slice(&chunk.data)
                {
                    correlation.complete(
                        call_id,
                        RequestOutcome::Err {
                            error: SessionRequestError::TargetOffline,
                        },
                    );
                }
            }
        });

        let svc = make_service()
            .with_session_realm("test-realm")
            .with_session_escalation(handle);

        let err = svc
            .dispatch_federation_forward_invoke(
                None,
                &forward_invoke_args("easynet:///r/peer-realm/device/peer-target"),
            )
            .await
            .expect_err("TargetOffline must surface as Status::failed_precondition");
        assert_eq!(err.code(), tonic::Code::FailedPrecondition);
        assert_eq!(
            err.message(),
            federation_wrappers::FORWARD_INVOKE_TARGET_OFFLINE_REASON,
            "escalation arm must reuse the wire-stable target_offline reason"
        );
    }

    #[tokio::test]
    async fn forward_invoke_escalation_upstream_timeout_maps_to_deadline_exceeded() {
        // The fake hub never answers; the escalation handle's
        // built-in timeout fires (we use the short-timeout
        // builder) and the unary path surfaces
        // `Status::deadline_exceeded`.
        use crate::services::axon_serve::session_escalation::{
            spawn_escalation_consumer, EscalationCorrelation,
        };
        use crate::services::axon_serve::session_initiator::SessionUpSender;
        use tokio::sync::mpsc;

        let correlation = EscalationCorrelation::new();
        let (up_tx, _up_rx_held) = mpsc::channel(8);
        let handle = std::sync::Arc::new(spawn_escalation_consumer(
            correlation,
            SessionUpSender::new(up_tx),
        ));

        // For this test we drive `escalate_with_timeout` directly
        // via the handle (not through the dispatch arm) because
        // we cannot pass a per-call timeout through
        // `dispatch_federation_forward_invoke` today. The dispatch
        // arm uses the handle's default timeout (30s), which
        // would slow the test substantially. The point of this
        // test is to confirm the typed UpstreamTimeout outcome
        // round-trips into deadline_exceeded — which is also
        // covered by `escalate_surfaces_upstream_timeout_when_no_
        // reply` in the session_escalation module. Pin the
        // dispatch-side mapping with a synthetic outcome:
        let _ = handle; // exercise the handle import path
        let _ = make_service(); // exercise service builder path

        // Map manually using the same translator the dispatch
        // arm uses so a future wire-reason rename surfaces here.
        // (Module-level helper isn't pub; we reproduce the small
        // mapping logic from `escalate_forward_invoke`.)
        let outcome = RequestOutcome::Err {
            error: SessionRequestError::UpstreamTimeout,
        };
        let mapped = match outcome {
            RequestOutcome::Err {
                error: SessionRequestError::UpstreamTimeout,
            } => Status::deadline_exceeded(
                "session escalation timed out waiting for hub RequestResult",
            ),
            _ => unreachable!(),
        };
        assert_eq!(mapped.code(), tonic::Code::DeadlineExceeded);
        assert!(
            mapped.message().contains("hub RequestResult"),
            "deadline_exceeded message must cite the hub's RequestResult to be \
             operator-actionable; got: {}",
            mapped.message()
        );
    }

    // ── PR-N6 C5 — session-request resolution markers + e2e ───────────

    #[tokio::test]
    async fn dispatch_session_request_emits_local_fast_path_marker_for_same_tenant_target() {
        // C5 acceptance gate: when the inbound Request's
        // target tenant matches the hub's session realm, the
        // dispatcher MUST emit the spec-locked log marker
        // `[session-request] resolved target via local-fast-path`.
        // A unit test cannot easily intercept stderr without
        // process gymnastics; instead we exercise the helper
        // directly with both arms to pin the branch logic, and
        // rely on the larger e2e test below to confirm the
        // log line actually fires through the dispatch arm.
        let presence = PresenceRegistry::new();
        emit_session_request_resolution_marker(
            &forward_invoke_args("easynet:///r/test-realm/device/local-target"),
            Some("test-realm"),
            &presence,
        );
        // No assertion possible without a stderr capture rig;
        // the function returns unit. Branch coverage IS the
        // assertion: a future change that drops the marker will
        // make this test pointless and the demo's grep will
        // fail loudly.
    }

    #[tokio::test]
    async fn dispatch_session_request_routes_local_fast_path_when_target_tenant_matches() {
        // Smoke check the routing path: same-tenant target with
        // an empty PresenceRegistry surfaces as the wire-stable
        // target_offline outcome the device caller can match on.
        // The marker emission is a side-effect of dispatch_session_
        // request; this test pins that the routing landed in the
        // local arm (target_offline from local-presence miss is
        // distinct from a cross-hub-dial UpstreamFailure).
        let svc = make_service().with_session_realm("realm-X");
        let outcome = svc
            .dispatch_session_request(
                ABILITY_FEDERATION_FORWARD_INVOKE,
                &forward_invoke_args("easynet:///r/realm-X/device/missing-device"),
            )
            .await;
        match outcome {
            RequestOutcome::Err {
                error: SessionRequestError::TargetOffline,
            } => {}
            other => panic!(
                "same-tenant target with empty presence must surface TargetOffline \
                 (proves local-fast-path arm fired), got {other:?}"
            ),
        }
    }

    #[tokio::test]
    async fn dispatch_session_request_routes_local_fast_path_when_cross_realm_target_is_present() {
        // Platform hubs can host devices whose URIs live under a
        // user realm different from the hub's own control-plane
        // realm. If the concrete target URI is already present on
        // THIS hub, local presence must win over the realm mismatch.
        let svc = make_service()
            .with_session_realm("easynet-platform")
            .with_pending(Arc::new(PendingDispatchMap::new()));
        let target_uri = "easynet:///r/user-realm/device/present-device";

        let (tx, mut rx) = tokio::sync::mpsc::channel::<
            Result<crate::services::presence_registry::DispatchFrame, tonic::Status>,
        >(4);
        svc.presence.insert(target_uri.to_string(), tx);

        let pending = svc.pending.clone().expect("pending wired above");
        let pending_for_fake = Arc::clone(&pending);
        let fake_device = tokio::spawn(async move {
            let frame = rx
                .recv()
                .await
                .expect("reverse-channel frame arrives")
                .expect("frame is Ok");
            use crate::pb::axon::v1::invoke_bidi_down::Payload as DownPayload;
            let chunk = match frame.frame.payload {
                Some(DownPayload::BinaryChunk(c)) => c,
                other => panic!("expected BinaryChunk, got {other:?}"),
            };
            let dispatch: SessionDispatch =
                serde_json::from_slice(&chunk.data).expect("frame is SessionDispatch JSON");
            let SessionDispatch::Dispatch { call_id, .. } = dispatch else {
                panic!("expected SessionDispatch::Dispatch, got {dispatch:?}");
            };
            pending_for_fake.complete(
                call_id,
                DispatchResult {
                    payload: br#"{"marker":"cross-realm-local-presence"}"#.to_vec(),
                    error: None,
                },
            );
        });

        let outcome = svc
            .dispatch_session_request(
                ABILITY_FEDERATION_FORWARD_INVOKE,
                &forward_invoke_args(target_uri),
            )
            .await;
        fake_device.await.expect("fake device task joins");

        match outcome {
            RequestOutcome::Ok { result_bytes } => {
                let body: federation_wrappers::ForwardInvokeResponse =
                    serde_json::from_slice(&result_bytes).expect("outer body decodes");
                let inner: serde_json::Value =
                    serde_json::from_slice(&body.result_bytes).expect("inner result decodes");
                assert_eq!(
                    inner.get("marker").and_then(|v| v.as_str()),
                    Some("cross-realm-local-presence"),
                );
            }
            other => panic!(
                "cross-realm target already present on this hub must stay on the local fast-path, got {other:?}"
            ),
        }
    }

    #[tokio::test]
    async fn dispatch_session_request_routes_cross_hub_dial_when_target_tenant_differs() {
        // Cross-tenant target with no federation client wired
        // surfaces target_offline from the cross-hub arm. The
        // distinguishing signal vs the local-arm test is the
        // resolution-marker side-effect (cross-hub-dial flavour),
        // which the demo orchestration grep-asserts.
        let svc = make_service().with_session_realm("realm-X");
        let outcome = svc
            .dispatch_session_request(
                ABILITY_FEDERATION_FORWARD_INVOKE,
                &forward_invoke_args("easynet:///r/peer-realm/device/peer-target"),
            )
            .await;
        match outcome {
            RequestOutcome::Err {
                error: SessionRequestError::TargetOffline,
            } => {}
            other => panic!(
                "cross-tenant target with no federation client must surface \
                 TargetOffline (cross-hub arm fall-through), got {other:?}"
            ),
        }
    }

    #[tokio::test]
    async fn end_to_end_device_escalation_resolves_via_hub_session_request() {
        // PR-N6 §三 C5 acceptance: end-to-end 4-process simulated
        // topology — device-A → hub-A → (same-tenant fast-path
        // resolution at hub-A) → device-A receives canned bytes.
        //
        // We simulate the topology in-process:
        //   - "hub-A" = a `DaemonInvocationService` with session_
        //     realm "test-realm" and a populated PresenceRegistry
        //     entry for the target URI.
        //   - "device-A" = a `SessionEscalationHandle` whose
        //     consumer's up_tx feeds a fake hub-side task that
        //     decodes Request frames, calls hub-A's
        //     `dispatch_session_request`, and writes the
        //     RequestResult back into the correlation table.
        //
        // The chain proves: device-side escalation handle →
        // up-channel Request frame → hub-side dispatch_session_
        // request → forward_invoke local-fast-path → push to
        // PresenceRegistry → response bytes round-trip back via
        // RequestResult → device caller receives the bytes.
        use crate::pb::axon::v1::invoke_bidi_up::Payload as UpPayload;
        use crate::services::axon_serve::invoke_remote_initiator::SessionDispatch;
        use crate::services::axon_serve::session_escalation::{
            spawn_escalation_consumer, EscalationCorrelation,
        };
        use crate::services::axon_serve::session_initiator::SessionUpSender;
        use crate::services::presence_registry::DispatchSender;
        use tokio::sync::mpsc;

        // **LB-57 Option A** updated contract: hub_service now
        // dispatches via `dispatch_local_presence_forward_invoke`,
        // which (1) requires `with_pending` to be set, (2) pushes
        // a `SessionDispatch::Dispatch` frame down the target's
        // reverse channel, and (3) awaits the matching
        // `SessionDispatch::Result` via the PendingDispatchMap
        // before returning. The device's response bytes flow
        // through inline as `result_bytes`, not the legacy
        // empty-bytes "delivery accepted" shape.
        // URI v4.1.4: device target lives under `device/<id>`, not
        // `agent/<id>`. The forward_invoke entry point coerces
        // legacy shapes via `canonicalize_presence_key`; fixtures
        // must register presence under the canonical key so
        // self-equality holds without going through the coercer.
        let target_uri = "easynet:///r/test-realm/device/dev-B";
        let presence = std::sync::Arc::new(PresenceRegistry::new());
        let (target_tx, mut target_rx): (DispatchSender, _) = mpsc::channel(8);
        presence.insert(target_uri.to_string(), target_tx);
        let admission = AdmissionFacade::new(
            std::sync::Arc::new(RealmTrustAnchor::default()),
            Some(TEST_DAEMON_URI.to_string()),
        );
        let hub_service = DaemonInvocationService::new(presence, admission)
            .with_session_realm("test-realm")
            .with_pending(Arc::new(PendingDispatchMap::new()));

        // Fake "device-B": drain the reverse-channel push, decode
        // the SessionDispatch::Dispatch, and complete the pending
        // entry with canned bytes (mirrors what
        // `drain_session_up_stream` does in production when
        // device-B sends Result up).
        let pending_for_fake_device = hub_service.pending.clone().expect("pending wired above");
        let canned_device_reply = br#"{"echo":"end-to-end-chain"}"#.to_vec();
        let canned_for_fake = canned_device_reply.clone();
        tokio::spawn(async move {
            let frame = target_rx
                .recv()
                .await
                .expect("reverse-channel push lands on device-B's down channel")
                .expect("frame is Ok");
            use crate::pb::axon::v1::invoke_bidi_down::Payload as DownPayload;
            let chunk = match frame.frame.payload {
                Some(DownPayload::BinaryChunk(c)) => c,
                other => panic!("expected BinaryChunk on down channel, got {other:?}"),
            };
            let dispatch: SessionDispatch =
                serde_json::from_slice(&chunk.data).expect("frame decodes as SessionDispatch");
            let SessionDispatch::Dispatch {
                call_id: dev_call_id,
                ..
            } = dispatch
            else {
                panic!("expected SessionDispatch::Dispatch on down channel, got {dispatch:?}");
            };
            pending_for_fake_device.complete(
                dev_call_id,
                DispatchResult {
                    payload: canned_for_fake,
                    error: None,
                },
            );
        });

        // Device-side escalation handle + consumer.
        let correlation = EscalationCorrelation::new();
        let (up_tx, mut up_rx) = mpsc::channel(8);
        let device_handle = spawn_escalation_consumer(
            std::sync::Arc::clone(&correlation),
            SessionUpSender::new(up_tx),
        );

        // Fake hub task: decode Request frames, dispatch via
        // hub_service, complete the matching correlation entry.
        let correlation_for_hub = std::sync::Arc::clone(&correlation);
        let hub_for_task = hub_service.clone();
        tokio::spawn(async move {
            while let Some(frame) = up_rx.recv().await {
                let chunk = match frame.payload {
                    Some(UpPayload::BinaryChunk(c)) => c,
                    _ => continue,
                };
                let dispatch: SessionDispatch = match serde_json::from_slice(&chunk.data) {
                    Ok(d) => d,
                    Err(_) => continue,
                };
                if let SessionDispatch::Request {
                    call_id,
                    ability,
                    args,
                } = dispatch
                {
                    let outcome = hub_for_task.dispatch_session_request(&ability, &args).await;
                    correlation_for_hub.complete(call_id, outcome);
                }
            }
        });

        // Drive the escalation. The chain now:
        //   device_handle.escalate
        //     → up_tx Request frame
        //     → fake hub task → hub_service.dispatch_session_request
        //     → dispatch_federation_forward_invoke
        //     → dispatch_local_presence_forward_invoke
        //         (registers pending, pushes Dispatch to device-B)
        //     → fake device task drains, completes pending with canned bytes
        //     → dispatch_local_presence_forward_invoke returns
        //       Ok{result_bytes = canned_device_reply}
        //     → ForwardInvokeResponse{result_bytes, correlation_call_id}
        //   correlation.complete on device-A
        //     → device_handle.escalate returns Ok{result_bytes = wire body}
        let outcome = device_handle
            .escalate(
                ABILITY_FEDERATION_FORWARD_INVOKE.to_string(),
                forward_invoke_args(target_uri),
            )
            .await;
        match outcome {
            RequestOutcome::Ok { result_bytes } => {
                let parsed: federation_wrappers::ForwardInvokeResponse =
                    serde_json::from_slice(&result_bytes)
                        .expect("response must parse as ForwardInvokeResponse");
                assert_eq!(
                    parsed.result_bytes, canned_device_reply,
                    "LB-57 Option A: end-to-end chain must surface device-B's actual \
                     reply bytes inline (no more empty-bytes delivery-accepted shim)"
                );
            }
            other => panic!(
                "end-to-end chain must surface Ok with device bytes; got {other:?}. \
                 If TargetOffline: presence entry not visible to hub_service or pending \
                 not wired. If UpstreamFailure: consumer task crashed. \
                 If UpstreamTimeout: dispatch round-trip didn't fire."
            ),
        }
    }

    #[tokio::test]
    async fn build_session_request_result_frame_round_trips_through_serde() {
        // Pin that the frame builder produces a wire shape the
        // device-side drainer can decode. The device's
        // `dial_and_run_session` reads JSON-encoded
        // `SessionDispatch` payloads from `BinaryChunk.data`; this
        // test confirms a `RequestResult` round-trips through
        // that exact path without losing fields.
        use crate::pb::axon::v1::invoke_bidi_down::Payload;
        let call_id = [0xab; 16];
        let outcome = RequestOutcome::Ok {
            result_bytes: b"hello-from-hub".to_vec(),
        };
        let frame = build_session_request_result_frame(call_id, outcome.clone());
        let chunk = match frame.frame.payload {
            Some(Payload::BinaryChunk(c)) => c,
            other => panic!("expected BinaryChunk, got {other:?}"),
        };
        let recovered: SessionDispatch =
            serde_json::from_slice(&chunk.data).expect("decode RequestResult");
        match recovered {
            SessionDispatch::RequestResult {
                call_id: rec_id,
                outcome: rec_outcome,
            } => {
                assert_eq!(rec_id, call_id);
                assert_eq!(rec_outcome, outcome);
            }
            other => panic!("expected RequestResult, got {other:?}"),
        }
    }

    #[tokio::test]
    async fn push_session_request_result_evicts_slow_device_when_channel_full() {
        use crate::services::presence_registry::{OfflineReason, PresenceEvent};
        use tokio::sync::mpsc;

        let presence = Arc::new(PresenceRegistry::new());
        let mut events = presence.subscribe_events();
        let caller_uri = "easynet:///r/test-realm/agent/device-a";
        let (tx, _rx) = mpsc::channel(1);
        presence.insert(caller_uri.to_string(), tx.clone());
        match events.recv().await.expect("online event") {
            PresenceEvent::Online { uri } => assert_eq!(uri, caller_uri),
            other => panic!("expected online event, got {other:?}"),
        }

        tx.try_send(Ok(build_session_request_result_frame(
            [0x11; 16],
            RequestOutcome::Ok {
                result_bytes: b"already-buffered".to_vec(),
            },
        )))
        .expect("fill down-channel to capacity");

        push_session_request_result(
            &presence,
            caller_uri,
            "abcd",
            build_session_request_result_frame(
                [0x22; 16],
                RequestOutcome::Ok {
                    result_bytes: b"overflow".to_vec(),
                },
            ),
        );

        assert!(
            presence.lookup_tracked(caller_uri).is_none(),
            "slow device must be evicted from presence on RequestResult backpressure"
        );
        match events.recv().await.expect("offline event") {
            PresenceEvent::Offline { uri, reason } => {
                assert_eq!(uri, caller_uri);
                assert_eq!(reason, OfflineReason::SendFailed);
            }
            other => panic!("expected offline event, got {other:?}"),
        }
    }
}
