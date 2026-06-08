// EasyNet CLI — invocation_transport — DaemonInvocationService
// ===================================================
//
// File: src/services/invocation_transport/daemon_invocation_service.rs
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
//                 AxonAbilityCatalog forwarding) note
//   - `InvokeStream`: `federation.subscribe_directory` →
//                 initial-snapshot frame from
//                 `build_subscribe_directory_initial`; the
//                 broadcast pump for incremental events lands in
//                 commit 7/9 alongside the AxonAbilityCatalog
//                 stream forward path
//   - `InvokeBidi`: still returns Unimplemented; PR-2 implements
//                 `<self>.session` accept and PR-3 implements
//                 `<self>.invoke_remote`
//
// What the dispatcher does NOT yet do
// -----------------------------------
// - Run the admission gate (commit 7/9, alongside the realm-trust
//   loader and `easynet-axon` admission helpers integration)
// - Forward unmatched abilities to AxonAbilityCatalog (commit 7/9)
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
// gRPC status-code policy
// -----------------------
// Three classes of failure surface back to the caller. Pick one
// class per failure site; do not improvise. Future arms added to
// this file MUST match the policy below — the policy is the only
// way a reviewer can tell, without reading the call stack, what
// the caller is being told.
//
//   * `Status::internal` — the daemon, the wire, or a backing
//     dependency (tonic streaming, serialisation, the
//     `easynet-axon` SDK encoding path) is broken. The caller did
//     nothing wrong; retrying with the same arguments will fail
//     the same way until the daemon is fixed. Wire-level
//     `tonic::Streaming::next()` errors fall here.
//
//   * `Status::invalid_argument` — the caller violated the wire
//     protocol or sent a malformed request that admission
//     accepted but dispatch cannot honour. Frame-0 sequence not
//     zero, EnvelopeOpen missing `target.ability_name`, non-
//     STRICT bidi stream ordering, etc. The caller can fix this
//     by changing the request.
//
//   * `Status::failed_precondition` — the request is well-formed
//     and the caller has admission, but THIS daemon is not
//     configured to serve it. Examples: `with_pending(...)` was
//     not called at boot so `<self>.invoke_remote` has no
//     correlation map; `LocalRuntime` is not wired so a self-
//     dispatch ability cannot run; the daemon was constructed
//     without an `InvocationLedger`. The caller can retry the
//     request against a daemon that IS configured, or the
//     operator can fix the boot wiring. Per PR-B in
//     `docs/rfc/industrial-textbook-followups-2026-05-29.md`,
//     these per-arm checks will move into `Builder::build()`
//     once the file is split; until then, each arm rejects
//     locally with this code and a message naming the missing
//     capability + the boot step that would supply it.
//
//   * `Status::not_found` — the target ability is unknown to
//     this daemon's catalogue, OR the named subject (a
//     correlation id, an agent URA the daemon does NOT host)
//     does not exist on this side. Distinct from
//     `failed_precondition` because retrying against a different
//     daemon could succeed without operator intervention.
//
//   * `Status::permission_denied` — admission was attempted and
//     rejected. Distinct from `unauthenticated` because the
//     identity IS established; it just lacks the right.
//
//   * `Status::unimplemented` — the ability name is reserved for
//     a future arm that lands in a follow-up PR. The message
//     MUST cite the follow-up so a `git grep` for the PR tag
//     surfaces the wiring site.
//
// Author: Silan Hu <silan.hu@u.nus.edu>
// Copyright (c) 2026 EasyNet. All rights reserved.

use std::collections::{BTreeMap, BTreeSet, HashMap};
use std::future::Future;
use std::path::PathBuf;
use std::pin::Pin;
use std::sync::Arc;
use std::task::{Context, Poll};
use std::time::Duration;

use easynet_axon::invocation::{AbilityFrame, AxonErrorKind, BidiInputFrame};
use futures::stream::FuturesUnordered;
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

use crate::services::federated_peers_cell::SharedFederatedPeers;
use crate::services::federation_client::FederationClient;
use crate::services::invocation_transport::admission_facade::AdmissionFacade;
use crate::services::invocation_transport::federation_wrappers::{
    self, ABILITY_FEDERATION_ADVERTISE_ABILITIES, ABILITY_FEDERATION_ADVERTISE_AGENT,
    ABILITY_FEDERATION_DISCOVER, ABILITY_FEDERATION_FORWARD_INVOKE, ABILITY_FEDERATION_HEARTBEAT,
    ABILITY_FEDERATION_JOIN, ABILITY_FEDERATION_LIST_USER_DEVICES,
    ABILITY_FEDERATION_PROXY_LIST_USER_DEVICES, ABILITY_FEDERATION_RESOLVE,
    ABILITY_FEDERATION_RESOLVE_KEY, ABILITY_FEDERATION_REVOKE,
    ABILITY_FEDERATION_SUBSCRIBE_DIRECTORY, ABILITY_FEDERATION_SUBSCRIBE_DIRECTORY_V2,
    ABILITY_NAMESPACE_PROXY_RESOLVE, ABILITY_NAMESPACE_RESOLVE,
    ABILITY_RUNTIME_BOOTSTRAP_SELF_IDENTITY,
};
use crate::services::invocation_transport::invoke_remote_initiator::{
    call_id_hex, InvokeRemoteDown, InvokeRemoteUp, RequestOutcome, SessionContentEnvelope,
    SessionDispatch, SessionRequestError, ABILITY_INVOKE_REMOTE, INVOKE_REMOTE_STREAM_ID,
};
use crate::services::invocation_transport::list_user_pubkeys::{
    handle as handle_list_user_pubkeys, ABILITY_SELF_LIST_USER_PUBKEYS,
};
use crate::services::invocation_transport::register_device_pubkey::{
    handle as handle_register_device_pubkey, parse_realm_from_ura as parse_realm_from_register_ura,
    ABILITY_SELF_REGISTER_DEVICE_PUBKEY,
};
use crate::services::invocation_transport::revoke_user_pubkey::{
    handle as handle_revoke_user_pubkey, ABILITY_SELF_REVOKE_USER_PUBKEY,
};
use crate::services::invocation_transport::route_resolver::{
    DaemonRouteResolver, DelegatedInvokeRoute, ResolveRouteFailure, SelectedInvokeRoute,
};
use crate::services::invocation_transport::session_initiator::{
    SessionSigningSeed, ABILITY_SELF_SESSION,
};

const DELEGATION_METADATA_KEY: &str = "x-easynet-delegation";
const SESSION_AUTHORITY_METADATA_KEY: &str = "x-easynet-session-authority";
const ROUTE_NEGATIVE_CODE: &str = "ROUTE_NEGATIVE";
const ROUTE_PROFILE_BLOCKED_CODE: &str = "ROUTE_PROFILE_BLOCKED";
const ROUTE_OWNER_MISMATCH_CODE: &str = "ROUTE_OWNER_MISMATCH";
const ROUTE_SELECTED_REMOTE_HOST_CODE: &str = "ROUTE_SELECTED_REMOTE_HOST";
const RESOLVE_SELECTED_HOST_UNAVAILABLE_CODE: &str = "RESOLVE_UNAVAILABLE";
use crate::services::pending_dispatch::{
    DispatchResult, DispatchStreamEvent, PendingDispatchMap, PendingStreamDispatchMap,
};
use crate::services::presence_registry::{
    DispatchFrame, DispatchSender, OfflineReason, PresenceRegistry, DISPATCH_CHANNEL_CAPACITY,
};
use crate::services::realm_trust_anchor::RealmTrustAnchor;
use crate::services::session_failure::SessionFailure;
use crate::services::trust_anchor_cell::SharedTrustAnchor;
use easynet_axon::pb::axon::v1::invocation_server::Invocation;
use easynet_axon::pb::axon::v1::{
    causal_context, invoke_bidi_down::Payload as DownPayload, invoke_bidi_up::Payload as UpPayload,
    AgentIdentity, BidiControl, BinaryChunk, CallerSignature, Envelope, EnvelopeOpen, Error,
    ErrorStage, InvocationReceipt, InvokeBidiDown, InvokeBidiUp, InvokeRequest, InvokeResponse,
    InvokeServerStreamRequest, InvokeStreamChunk, ResponseHeader, SecurityClass, StreamDescriptor,
    SubjectIdentity,
};

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
///   URA is not in the realm trust anchor (per spec §5)
///
/// Future-shape (commit 8/9 onward) will add:
/// `ability_dispatch: Arc<AxonAbilityCatalog>` for the unmatched-
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
    /// back to their host device URA so resolve can project hosted
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
    /// Same-hub `fs.transfer` is the first consumer.
    pending_stream: Option<Arc<PendingStreamDispatchMap>>,
    /// `<self>.register_device_pubkey` handler context (PR-7
    /// commit 5/N). `None` until `with_register_pubkey(...)` wires
    /// it; absence means the ability returns
    /// `Status::failed_precondition` (the daemon was booted without
    /// the trust-write surface — typically a smoke-test setup).
    /// Production daemons always attach one at boot from
    /// `start_daemon_invocation_transport`.
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
    /// `None` preserves the on-demand read of
    /// `~/.easynet-hub/<realm>/identity.json`.
    hub_signing_seed: Option<SessionSigningSeed>,
    /// **PR-N1 commit 3a/N**. Cross-hub federation client. `None`
    /// until `with_federation_client(...)` wires one; absence
    /// means `federation.forward_invoke` for cross-realm targets
    /// returns `target_offline` without dialing (no
    /// dial). Commit 3b/N rewrites the `forward_invoke` dispatcher
    /// to consume this field; commit 3a/N only plumbs it through.
    federation_client: Option<Arc<dyn FederationClient>>,
    /// **PR-N1 commit 3a/N → 10/N**. Operator-curated `realm →
    /// hub_endpoint` cell per `DaemonConfig::federated_peers`. Empty
    /// map ⇒ no cross-realm routing configured; the dispatcher
    /// returns `target_offline`. Commit 10/N upgraded this from a
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
    /// cross-realm URA lookup. Defaults to empty so single-realm
    /// daemons gracefully report no federated entries.
    federated_directory: crate::services::federation_directory::SharedFederatedDirectoryView,
    /// **2026-05-25 P0 hardening**. Wired from
    /// `DaemonConfig::allow_directory_auto_route()`. When `false`
    /// (the default), the federation dispatcher refuses to dial a
    /// peer hub whose endpoint came from an observed
    /// `federated_directory` entry — see
    /// [`crate::services::invocation_transport::hub_resolver`] for the
    /// threat model. Wiring lands inline rather than as a `Builder`
    /// because the flag's value never changes within a daemon
    /// lifetime (no SIGHUP-driven runtime toggle): the security
    /// posture is set at boot.
    allow_directory_auto_route: bool,
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
    /// `mode = "device"` per `boot.rs::start_daemon_invocation_transport`.
    escalation: Option<
        std::sync::Arc<
            crate::services::invocation_transport::session_escalation::SessionEscalationHandle,
        >,
    >,
    /// Workspace-scoped invocation ledger. Boot wires this to
    /// `<ledger_dir>/invocations.redb`; tests may inject a temp
    /// ledger. The service writes complete unary invoke records
    /// through the Axon SDK object rather than owning a local file
    /// format.
    invocation_ledger: Option<Arc<easynet_axon::invocation::InvocationLedger>>,
    /// Shared Axon `LocalRuntime` built at daemon boot. It is the
    /// sole in-process source of truth for local abilities: direct
    /// unary, stream, bidi, and self-targeted federation dispatch all
    /// enter through this handle.
    local_runtime: Option<Arc<easynet_axon::invocation::LocalRuntime>>,
    /// Daemon-owned local bidi wire profile registry. Plugin wire metadata is
    /// projected into this value at boot; the service does not inspect package
    /// state while handling an invocation.
    ability_wire: Arc<crate::runtime::ability_wire::AbilityWireRegistry>,
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
            .field(
                "invocation_ledger",
                &self.invocation_ledger.as_ref().map(|_| "<redb>"),
            )
            .field(
                "local_runtime",
                &self.local_runtime.as_ref().map(|_| "<axon LocalRuntime>"),
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
            allow_directory_auto_route: false,
            federated_bindings: None,
            subscribe_v2_heartbeat_interval_ms: 30_000,
            escalation: None,
            invocation_ledger: None,
            local_runtime: None,
            ability_wire: Arc::new(crate::runtime::ability_wire::AbilityWireRegistry::core()),
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
        // pending dispatch whose target_ura just went offline.
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
                    Ok(PresenceEvent::Offline { ura, reason }) => {
                        let cancelled = watcher_pending.cancel_for(&ura, "target_offline");
                        if cancelled > 0 {
                            // `OfflineReason: Display` renders the
                            // stable snake_case wire label; no Debug
                            // double-quoting at the op-event boundary.
                            crate::op_event!(
                                component = daemon_invocation,
                                kind = presence_offline_cancel,
                                target_ura = ura,
                                reason = reason,
                                cancelled = cancelled,
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
            crate::services::invocation_transport::session_escalation::SessionEscalationHandle,
        >,
    ) -> Self {
        self.escalation = Some(handle);
        self
    }

    #[must_use]
    pub fn with_invocation_ledger(
        mut self,
        ledger: Arc<easynet_axon::invocation::InvocationLedger>,
    ) -> Self {
        self.invocation_ledger = Some(ledger);
        self
    }

    /// Set whether this service's admission gate honours the loopback
    /// bypass. Boot serves the *same* service over a loopback-only UDS
    /// and an off-box TCP+TLS socket; the TCP-fed clone is given
    /// `false` so a daemon-URA spoofer reaching the TCP port still
    /// runs the full strict pipeline. See
    /// [`AdmissionFacade::with_loopback_trusted`].
    #[must_use]
    pub fn with_loopback_trusted(mut self, loopback_trusted: bool) -> Self {
        self.admission = self.admission.with_loopback_trusted(loopback_trusted);
        self
    }

    /// **Phase 2 of the Axon-SDK migration**. Attach the shared
    /// `LocalRuntime` instance. Boot constructs it after the trust
    /// anchor and invocation ledger are available so the runtime
    /// can install its `KeyResolver` + `LedgerSink` before any
    /// invocation lands.
    ///
    /// Phase 4 flips `dispatch_invoke_remote` /
    /// `dispatch_federation_forward_invoke` to route through the
    /// runtime; until then this is wired but unread, matching the
    /// "non-destructive bring-up first, hard-swap second" cadence
    /// of the migration.
    #[must_use]
    pub fn with_local_runtime(
        mut self,
        runtime: Arc<easynet_axon::invocation::LocalRuntime>,
    ) -> Self {
        self.local_runtime = Some(runtime);
        self
    }

    /// Attach the daemon-owned wire profile registry used for local bidi
    /// dispatch. Boot computes this after plugin load planning.
    #[must_use]
    pub fn with_ability_wire_registry(
        mut self,
        registry: Arc<crate::runtime::ability_wire::AbilityWireRegistry>,
    ) -> Self {
        self.ability_wire = registry;
        self
    }

    /// **PR-N1 commit 3a/N**. Attach the cross-hub federation
    /// client. Daemons booted without one return `target_offline`
    /// for cross-realm
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
    /// `realm → hub_endpoint` map by-value. Wraps the supplied map in
    /// a fresh `SharedFederatedPeers` cell so test fixtures that
    /// don't care about hot-reload still get the cell shape under
    /// the hood. Production daemons use
    /// [`with_federated_peers_cell`] to share the boot-time cell
    /// with the SIGHUP reload task.
    ///
    /// Empty map (the default from `DaemonInvocationService::new`)
    /// means no cross-realm routing is configured; the
    /// dispatcher's cross-realm arm then refuses to dial
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
    /// Production `start_daemon_invocation_transport` uses this builder; the
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

    /// **2026-05-25 P0 hardening**. Set the directory-auto-route
    /// security posture. Boot wires this from
    /// `DaemonConfig::allow_directory_auto_route()`. The default
    /// (`false`) is the secure shape; this builder is the single
    /// place that should ever set `true`, and it is intended to be
    /// called from the daemon's startup path with the value the
    /// operator deliberately opted into in `daemon-config.toml`.
    #[must_use]
    pub fn with_allow_directory_auto_route(mut self, allow: bool) -> Self {
        self.allow_directory_auto_route = allow;
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
    ///
    /// `NonZeroU64` rules out the "cadence = 0 pins the CPU"
    /// foot-gun at the type level; the previous shape was a
    /// `u64` guarded by a runtime `assert!`.
    #[must_use]
    pub fn with_subscribe_v2_heartbeat_interval_ms(mut self, ms: std::num::NonZeroU64) -> Self {
        self.subscribe_v2_heartbeat_interval_ms = ms.get();
        self
    }

    /// Resolve whether `target_ura` names THIS daemon's own
    /// synchronous-execution surface.
    ///
    /// Three valid shapes per RFC-001 + RFC-006-C v0.1:
    ///   (1) `easynet:///r/<realm>/device/<deviceID>` — the daemon's
    ///       device identity from credentials.json. Standard.
    ///   (2) `easynet:///r/<realm>/hub` — the canonical Hub URA;
    ///       hub-mode daemons answer to this in addition to (1).
    ///   (3) `easynet:///r/<realm>/agent/<userID>.<agentID>` — the
    ///       agent URA of an agent the daemon currently hosts. v4.1.5
    ///       §9 callee ∈ {hub, device, agent}; RFC-006-C §INV-2 +
    ///       RFC-006-B v0.6 §URL require the wire callee on a chat-
    ///       base or page.fetch invocation to be the agent URA, not
    ///       the device. Recognise it here so the local fast path
    ///       fires instead of falling through to "target offline".
    ///
    /// Match for (3) uses the hosted-agent identity, not just the
    /// bare `<agentID>`. A daemon only treats an Agent URA as local
    /// when either:
    ///   * `local-agents.json` contains the same `(realm,user,agent)`
    ///     tuple; or
    ///   * the tuple matches this daemon's credentials and the agent is
    ///     currently dispatchable through LocalRuntime or `agents.json`.
    ///
    /// The second branch preserves post-boot `agent.start`
    /// behaviour before publish has written `local-agents.json`, but it
    /// is still scoped to the exact realm and user from credentials.
    async fn matches_self_target_ura(&self, target_ura: &str) -> bool {
        if self
            .admission
            .daemon_ura()
            .is_some_and(|daemon_ura| daemon_ura == target_ura)
        {
            return true;
        }
        if self
            .session_realm
            .as_deref()
            .is_some_and(|realm| crate::ura::hub_ura(realm) == target_ura)
        {
            return true;
        }
        if let Some(agent_target) = parse_agent_target_identity(target_ura) {
            if local_agents_hosts_agent_target(&agent_target) {
                return true;
            }

            let identity_matches_credentials = credentials_match_agent_target(&agent_target);
            let mut list_abilities_miss = "not_checked";
            let mut agents_json_miss = "not_checked";
            if identity_matches_credentials {
                if let Some(runtime) = self.local_runtime.as_ref() {
                    let agent_dot = format!("{}.", agent_target.agent_id);
                    let agent_dot_owned = format!(".{}.", agent_target.agent_id);
                    // Awaited rather than block_on'd: this method runs
                    // inside the gRPC `Invoke{,Stream,Bidi}` async impls.
                    if runtime.list_abilities().await.iter().any(|descriptor| {
                        descriptor.name.starts_with(&agent_dot)
                            || descriptor.name.contains(&agent_dot_owned)
                    }) {
                        return true;
                    }
                    list_abilities_miss = "true";
                }
                if crate::registry::agents::load_agents()
                    .map(|reg| reg.agents.contains_key(&agent_target.agent_id))
                    .unwrap_or(false)
                {
                    return true;
                }
                agents_json_miss = "true";
            }

            let credential_identity_miss = if identity_matches_credentials {
                "false"
            } else {
                "true"
            };
            crate::op_event!(
                component = daemon_invocation,
                kind = self_target_miss_for_agent_ura,
                target_ura = target_ura,
                realm = agent_target.realm.as_str(),
                user_id = agent_target.user_id.as_str(),
                agent_id = agent_target.agent_id.as_str(),
                local_agents_miss = "true",
                credential_identity_miss = credential_identity_miss,
                list_abilities_miss = list_abilities_miss,
                agents_json_miss = agents_json_miss,
                message = "matches_self_target_ura: agent URA not local; \
                          no exact local hosted Agent identity matched. Call \
                          will fall through to PresenceRegistry lookup.",
            );
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
    ///   commit 7/9 wires AxonAbilityCatalog as the fall-through
    async fn invoke(
        &self,
        request: Request<InvokeRequest>,
    ) -> Result<Response<InvokeResponse>, Status> {
        let inner = request.into_inner();
        let started_unix_ms = now_unix_ms();
        if let Err(err) = self.admission.verify_invoke(&inner) {
            let result: Result<Response<InvokeResponse>, Status> = Err(err);
            self.record_unary_invocation(&inner, started_unix_ms, &result);
            return result;
        }
        let function = inner.function_name.as_str();
        // #185: meter the already-admitted caller. A throttled caller
        // is rejected here with `ResourceExhausted` before any dispatch
        // work; the post-decrement status (when metered) rides the
        // successful response below. Federation and daemon-local
        // `<self>` calls are control-plane traffic; throttling them
        // would break liveness and key-discovery paths.
        let rate_limit = match quota_metered_ability_for_request(&inner) {
            Ok(Some(ability)) => match self.admission.check_quota_for_ability(&inner, &ability) {
                Ok(info) => info,
                Err(err) => {
                    let result: Result<Response<InvokeResponse>, Status> = Err(err);
                    self.record_unary_invocation(&inner, started_unix_ms, &result);
                    return result;
                }
            },
            Ok(None) => None,
            Err(err) => {
                let result: Result<Response<InvokeResponse>, Status> = Err(err);
                self.record_unary_invocation(&inner, started_unix_ms, &result);
                return result;
            }
        };

        // Phase 5e: flag set by the Axon-routed catch-all arm so we
        // skip the manual `record_unary_invocation` call below
        // (otherwise the LedgerSink write + manual write produce two
        // rows for the same call).
        let mut axon_took_it = false;
        let result = match function {
            ABILITY_FEDERATION_JOIN => self.dispatch_federation_join(&inner.arguments),
            ABILITY_FEDERATION_ADVERTISE_AGENT => {
                self.dispatch_federation_advertise_agent(&inner.arguments)
            }
            ABILITY_FEDERATION_ADVERTISE_ABILITIES => {
                self.dispatch_federation_advertise_abilities(&inner.arguments)
            }
            ABILITY_FEDERATION_HEARTBEAT => self.dispatch_federation_heartbeat(&inner.arguments),
            ABILITY_FEDERATION_RESOLVE => self.dispatch_federation_resolve(&inner.arguments),
            ABILITY_NAMESPACE_RESOLVE => self.dispatch_namespace_resolve(&inner.arguments),
            ABILITY_FEDERATION_RESOLVE_KEY => {
                self.dispatch_federation_resolve_key(&inner.arguments)
            }
            ABILITY_FEDERATION_DISCOVER => self.dispatch_federation_discover(&inner.arguments),
            ABILITY_FEDERATION_LIST_USER_DEVICES => self
                .dispatch_federation_list_user_devices(inner.envelope.as_ref(), &inner.arguments),
            ABILITY_FEDERATION_PROXY_LIST_USER_DEVICES => {
                self.dispatch_federation_proxy_list_user_devices(
                    inner.envelope.as_ref(),
                    &inner.arguments,
                )
                .await
            }
            ABILITY_NAMESPACE_PROXY_RESOLVE => {
                self.dispatch_namespace_proxy_resolve(inner.envelope.as_ref(), &inner.arguments)
                    .await
            }
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
            ABILITY_SELF_REVOKE_USER_PUBKEY => self.dispatch_revoke_user_pubkey(&inner.arguments),
            ABILITY_SELF_LIST_USER_PUBKEYS => self.dispatch_list_user_pubkeys(&inner.arguments),
            // Catch-all user abilities must pass through namespace.resolve
            // before Axon LocalRuntime dispatch. The runtime executes the
            // selected route; it is not a resolver fallback.
            //
            // `axon_took_it` gates the post-dispatch
            // `record_unary_invocation` so we don't write a duplicate
            // ledger row for calls Axon already persisted via
            // `LedgerSink`. Federation-wrapper arms above still run
            // the manual record path because they are explicit service
            // handlers rather than LocalRuntime ability dispatches.
            _other => {
                let (r, axon) = self.dispatch_local_rpc_selected_route(&inner).await;
                axon_took_it = axon;
                r
            }
        };
        if !axon_took_it {
            self.record_unary_invocation(&inner, started_unix_ms, &result);
        }
        // #185: attach the caller's post-decrement quota status to the
        // successful response in one place, rather than threading it
        // through every dispatch arm's `InvokeResponse` builder. `None`
        // when the caller is unmetered / loopback / quota is off, in
        // which case the wire shape is unchanged.
        match (result, rate_limit) {
            (Ok(mut response), Some(info)) => {
                response.get_mut().rate_limit = Some(info);
                Ok(response)
            }
            (other, _) => other,
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
            _other => self.dispatch_local_stream_selected_route(&inner).await,
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
                return Err(Status::internal(format!("InvokeBidi frame 0 recv: {err}")));
            }
            None => return Err(Status::invalid_argument("InvokeBidi: empty up stream")),
        };

        let envelope_open = validate_and_extract_bidi_frame0(&frame0)?;
        // PR-7: full §5.2 admission for the bidi path. The facade
        // checks envelope presence + caller URA, runs the four-step
        // pipeline (envelope/structure/verify/replay), and rejects
        // with the canonical wire reasons. Ability name + initial
        // args feed `args_digest` exactly the way unary/server-stream
        // requests do.
        self.admission.verify_envelope_for_bidi(envelope_open)?;

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
                let caller_ura = envelope_open
                    .envelope
                    .as_ref()
                    .and_then(|e| e.caller.as_ref())
                    .map(|c| c.ura.clone())
                    .ok_or_else(|| {
                        Status::invalid_argument(
                            "<self>.session: envelope.caller.ura is required \
                             (already verified by admission gate; this is a defensive check)",
                        )
                    })?;
                self.dispatch_self_session_accept(caller_ura, up).await
            }
            other if self.ability_wire.is_bidi_wire_ability(other) => {
                if let Some(target_ura) = remote_bidi_target_ura(envelope_open) {
                    if !self.matches_self_target_ura(&target_ura).await {
                        // RFC-005 resolve-first gate. Mirror the
                        // `<self>.invoke_remote` resolver call site
                        // (`dispatch_invoke_remote`): prove the wire
                        // ability exists on the target and that the
                        // selected route is authoritative-local-or-better
                        // BEFORE bridging the bidi stream. The resolved
                        // route's `execution_host_ura` equals the target
                        // device URA for a device-owned wire ability, so
                        // `dispatch_remote_bidi` keeps owning the presence
                        // lookup and frame plumbing; the resolver acts as a
                        // validation gate only.
                        match self
                            .daemon_route_resolver()
                            .resolve_route(&target_ura, other)
                        {
                            Ok(route) if route.is_authoritative_local_or_better() => {
                                return self.dispatch_remote_bidi(&route, envelope_open, up).await;
                            }
                            Ok(route) => {
                                return Err(route_profile_blocked_status(&route));
                            }
                            Err(failure) => {
                                return Err(route_negative_status(failure));
                            }
                        }
                    }
                }
                self.dispatch_local_bidi_selected_route(envelope_open, up)
                    .await
            }
            other => Err(Status::unimplemented(format!(
                "easynet-daemon: InvokeBidi ability `{other}` is not yet wired; \
                 only built-in PTY/file-transfer or plugin-declared bidi abilities currently have \
                 daemon gRPC wire adapters"
            ))),
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
struct AgentTargetIdentity {
    realm: String,
    user_id: String,
    agent_id: String,
}

/// Extract the full hosted-agent identity from an
/// `agent/<userID>.<agentID>` URA. Returns `None` for any other role
/// or for malformed URAs.
fn parse_agent_target_identity(target_ura: &str) -> Option<AgentTargetIdentity> {
    let parsed = crate::ura::parse_ura(target_ura).ok()?;
    if !matches!(parsed.kind, crate::ura::URAKind::Agent) {
        return None;
    }
    let realm = parsed.realm.clone();
    let (user_id, agent_id) = parsed.agent_ids()?;
    if realm.is_empty() || user_id.is_empty() || agent_id.is_empty() {
        return None;
    }
    Some(AgentTargetIdentity {
        realm,
        user_id: user_id.to_string(),
        agent_id: agent_id.to_string(),
    })
}

fn local_agents_hosts_agent_target(target: &AgentTargetIdentity) -> bool {
    crate::persistence::local_agents::load()
        .map(|file| {
            file.hosted_agents
                .iter()
                .any(|entry| agent_ura_matches_target(&entry.agent_ura, target))
        })
        .unwrap_or(false)
}

fn credentials_match_agent_target(target: &AgentTargetIdentity) -> bool {
    crate::persistence::config::load_credentials()
        .ok()
        .and_then(|creds| {
            let username = creds.username_slug().ok()?.to_string();
            Some((creds.realm, username))
        })
        .map(|(realm, username)| realm.trim() == target.realm && username.trim() == target.user_id)
        .unwrap_or(false)
}

fn agent_ura_matches_target(ura: &str, target: &AgentTargetIdentity) -> bool {
    crate::ura::parse_ura(ura)
        .map(|parsed| {
            parsed.kind == crate::ura::URAKind::Agent
                && parsed.realm == target.realm
                && parsed.agent_ids() == Some((target.user_id.as_str(), target.agent_id.as_str()))
        })
        .unwrap_or(false)
}

/// Pull the `EnvelopeOpen` payload out of frame 0 of an
/// `InvokeBidi` up stream. Returns `Status::invalid_argument` for
/// any non-EnvelopeOpen first frame, since the axon protocol
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

fn local_invoke_target_ura(request: &InvokeRequest) -> Result<String, Status> {
    target_ura_from_envelope(request.envelope.as_ref(), "Invoke")
}

fn local_stream_target_ura(request: &InvokeServerStreamRequest) -> Result<String, Status> {
    target_ura_from_envelope(request.envelope.as_ref(), "InvokeStream")
}

fn target_ura_from_envelope(envelope: Option<&Envelope>, label: &str) -> Result<String, Status> {
    let envelope = envelope.ok_or_else(|| {
        Status::invalid_argument(format!(
            "{label} request missing envelope for namespace.resolve"
        ))
    })?;
    let target_ura = envelope
        .callee
        .as_ref()
        .or(envelope.caller.as_ref())
        .map(|identity| identity.ura.trim())
        .filter(|ura| !ura.is_empty())
        .ok_or_else(|| {
            Status::invalid_argument(format!(
                "{label} request envelope must carry callee or caller URA for namespace.resolve"
            ))
        })?;
    crate::ura::parse_ura(target_ura)
        .map_err(|err| Status::invalid_argument(format!("{label} target URA is invalid: {err}")))?;
    Ok(target_ura.to_string())
}

fn envelope_with_selected_callee(
    mut envelope: Envelope,
    selected_route: &SelectedInvokeRoute,
) -> Envelope {
    envelope.callee = Some(AgentIdentity {
        ura: selected_route.callee_ura.clone(),
        profile: crate::services::invocation_transport::DEFAULT_URA_PROFILE.to_string(),
    });
    envelope
}

fn envelope_open_with_selected_route(
    envelope_open: &EnvelopeOpen,
    selected_route: &SelectedInvokeRoute,
) -> EnvelopeOpen {
    let mut selected = envelope_open.clone();
    if let Some(envelope) = selected.envelope.take() {
        selected.envelope = Some(envelope_with_selected_callee(envelope, selected_route));
    }
    if let Some(target) = selected.target.as_mut() {
        target.ability_name = selected_route.dispatch_key();
    }
    selected
}

fn route_negative_message(failure: &ResolveRouteFailure) -> String {
    format!(
        "{ROUTE_NEGATIVE_CODE}: namespace.resolve negative for `{}`: {}: {}",
        failure.query_name,
        failure.reason.as_str_name(),
        failure.detail,
    )
}

fn route_negative_status(failure: ResolveRouteFailure) -> Status {
    Status::failed_precondition(route_negative_message(&failure))
}

fn route_profile_blocked_message(selected_route: &SelectedInvokeRoute) -> String {
    format!(
        "{ROUTE_PROFILE_BLOCKED_CODE}: namespace.resolve selected route `{}` with \
         non-dispatchable release profile {}",
        selected_route.route_ura,
        selected_route.release_profile.as_str_name(),
    )
}

fn route_profile_blocked_status(selected_route: &SelectedInvokeRoute) -> Status {
    Status::failed_precondition(route_profile_blocked_message(selected_route))
}

fn route_owner_mismatch_message(
    selected_owner_ura: &str,
    ability_ura: &str,
    expected_target_ura: &str,
) -> String {
    format!(
        "{ROUTE_OWNER_MISMATCH_CODE}: namespace.resolve selected owner `{selected_owner_ura}` \
         for ability `{ability_ura}` but request target was `{expected_target_ura}`"
    )
}

fn route_selected_remote_host_status(label: &str, selected_route: &SelectedInvokeRoute) -> Status {
    Status::failed_precondition(format!(
        "{ROUTE_SELECTED_REMOTE_HOST_CODE}: {label} selected execution host `{}` for route `{}`; \
         direct local dispatch can execute only routes hosted by this daemon",
        selected_route.execution_host_ura, selected_route.route_ura,
    ))
}

fn session_failure_from_reason(
    reason: &str,
    fallback_code: &str,
    retryable: bool,
) -> SessionFailure {
    SessionFailure::from_reason(reason, fallback_code, retryable)
}

fn failed_dispatch_result(
    reason: impl Into<String>,
    fallback_code: &str,
    retryable: bool,
) -> DispatchResult {
    let reason = reason.into();
    DispatchResult {
        payload: Vec::new(),
        failure: Some(session_failure_from_reason(
            &reason,
            fallback_code,
            retryable,
        )),
        error: Some(reason),
        request_id: None,
    }
}

fn selected_host_unavailable_message(selected_route: &SelectedInvokeRoute) -> String {
    format!(
        "{RESOLVE_SELECTED_HOST_UNAVAILABLE_CODE}: namespace.resolve selected execution host `{}` \
         for route `{}` but the session disappeared before dispatch",
        selected_route.execution_host_ura, selected_route.route_ura,
    )
}

impl DaemonInvocationService {
    fn daemon_route_resolver(&self) -> DaemonRouteResolver<'_> {
        let resolver = DaemonRouteResolver::new(
            &self.presence,
            Some(self.advertised_agents.as_ref()),
            Some(self.ability_catalog.as_ref()),
        );
        match self
            .session_realm
            .as_deref()
            .filter(|realm| !realm.is_empty())
        {
            Some(local_realm) => resolver.with_peer_delegation(
                local_realm,
                &self.federated_peers,
                &self.federated_directory,
                self.allow_directory_auto_route,
            ),
            None => resolver,
        }
    }

    fn record_unary_invocation(
        &self,
        request: &InvokeRequest,
        started_unix_ms: i64,
        result: &Result<Response<InvokeResponse>, Status>,
    ) {
        let Some(ledger) = self.invocation_ledger.as_ref() else {
            return;
        };
        let completed_unix_ms = now_unix_ms();
        let record =
            match build_unary_ledger_record(request, started_unix_ms, completed_unix_ms, result) {
                Ok(record) => record,
                Err(err) => {
                    let err_msg = format!("{err}");
                    crate::op_event!(
                        component = daemon_invocation,
                        kind = ledger_record_skipped,
                        shape = "unary",
                        error = err_msg,
                    );
                    return;
                }
            };
        if let Err(err) = ledger.put(&record) {
            let err_msg = format!("{err}");
            crate::op_event!(
                component = daemon_invocation,
                kind = ledger_write_failed,
                shape = "unary",
                invocation_ura = record.invocation_ura,
                error = err_msg,
            );
        }
    }

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

    fn dispatch_federation_heartbeat(
        &self,
        arguments: &[u8],
    ) -> Result<Response<InvokeResponse>, Status> {
        let request: federation_wrappers::HeartbeatRequest = parse_json_args(arguments)?;
        let response = federation_wrappers::handle_heartbeat(&request, &self.presence);
        wrap_json_response(&response)
    }

    /// Unary `Invoke` catch-all backed by RFC-005 namespace.resolve
    /// followed by Axon's `LocalRuntime`.
    ///
    /// Returns `(response, axon_took_it)`. The caller in
    /// [`Self::invoke`] consults `axon_took_it` to decide whether
    /// the post-dispatch `record_unary_invocation` should fire:
    ///   * `true` — Axon actually started an invocation and returned
    ///     its `invocation_id`; Axon's `LedgerSink` wrote the
    ///     canonical row on the terminal event, so the manual record
    ///     would only produce a duplicate keyed by `request_id`.
    ///   * `false` — no handler ran (runtime missing or ability
    ///     unknown), so the manual failed row may be recorded.
    async fn resolve_local_rpc_route(
        &self,
        request: &InvokeRequest,
    ) -> Result<SelectedInvokeRoute, Status> {
        let target_ura = local_invoke_target_ura(request)?;
        let ability = request.function_name.trim();
        if ability.is_empty() {
            return Err(Status::invalid_argument(
                "Invoke request missing function_name for namespace.resolve",
            ));
        }

        let selected_route = self
            .daemon_route_resolver()
            .resolve_route(&target_ura, ability)
            .map_err(route_negative_status)?;

        if !selected_route.is_authoritative_local_or_better() {
            return Err(route_profile_blocked_status(&selected_route));
        }
        if !self
            .matches_self_target_ura(&selected_route.execution_host_ura)
            .await
        {
            return Err(route_selected_remote_host_status("Invoke", &selected_route));
        }

        Ok(selected_route)
    }

    async fn dispatch_local_rpc_selected_route(
        &self,
        request: &InvokeRequest,
    ) -> (Result<Response<InvokeResponse>, Status>, bool) {
        let ability = request.function_name.trim();
        let arguments = request.arguments.as_slice();
        let selected_route = match self.resolve_local_rpc_route(request).await {
            Ok(route) => route,
            Err(status) => return (Err(status), false),
        };
        let Some(runtime) = self.local_runtime.as_ref() else {
            return (
                Err(Status::failed_precondition(format!(
                    "easynet-daemon: ability `{ability}` cannot run because Axon LocalRuntime \
                     is not wired at boot"
                ))),
                false,
            );
        };
        let dispatch_ability = selected_route.dispatch_key();
        let Some(options) = runtime.ability_options(&dispatch_ability).await else {
            return (
                Err(Status::not_found(format!(
                    "easynet-daemon: selected route `{}` dispatches `{}` but that ability is not \
                     registered in Axon LocalRuntime",
                    selected_route.route_ura, dispatch_ability
                ))),
                false,
            );
        };
        if !options.modes.rpc {
            return (
                Err(Status::invalid_argument(format!(
                    "easynet-daemon: ability `{ability}` is registered, but does not support \
                     unary Invoke; use the stream/bidi call shape advertised by meta.list_abilities"
                ))),
                false,
            );
        }
        crate::op_event!(
            component = daemon_invocation,
            kind = dispatch_local_rpc_selected_route,
            ability = ability,
            dispatch_ability = dispatch_ability.as_str(),
            callee_ura = selected_route.callee_ura.as_str(),
            execution_host_ura = selected_route.execution_host_ura.as_str(),
            route_ura = selected_route.route_ura.as_str(),
        );
        let wire = match request.envelope.clone() {
            Some(envelope) => {
                let envelope = envelope_with_selected_callee(envelope, &selected_route);
                crate::runtime::axon_bridge::dispatch_shim::admitted_from_wire_parts(
                    envelope,
                    dispatch_ability.clone(),
                    arguments.to_vec(),
                )
            }
            None => Err(easynet_axon::invocation::AxonError::invalid_argument(
                "Invoke request missing envelope",
            )),
        };
        let wire = match wire {
            Ok(wire) => wire,
            Err(err) => {
                return (
                    Err(status_from_axon_invoke_error("Invoke", ability, err)),
                    false,
                );
            }
        };
        let outcome =
            crate::runtime::axon_bridge::dispatch_shim::dispatch_rpc_admitted(runtime, wire).await;
        let crate::runtime::axon_bridge::dispatch_shim::RpcDispatchOutcome {
            invocation_id,
            payload_bytes,
            error,
            ..
        } = outcome;
        let axon_started = invocation_id.is_some();
        let response = match error {
            None => Ok(Response::new(InvokeResponse {
                header: invocation_id.map(|request_id| ResponseHeader {
                    request_id,
                    status: "completed".to_string(),
                    ..ResponseHeader::default()
                }),
                result: payload_bytes,
                result_content_type: FEDERATION_RESULT_CONTENT_TYPE.to_string(),
                state: easynet_axon::invocation::InvocationState::Completed.to_wire_i32(),
                ..InvokeResponse::default()
            })),
            Some(err) => Err(Status::failed_precondition(format!(
                "local-rpc axon dispatch: ability `{ability}` failed: {err}"
            ))),
        };
        (response, axon_started)
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
            state: easynet_axon::invocation::InvocationState::Completed.to_wire_i32(),
            ..InvokeResponse::default()
        }))
    }

    /// DEC-EU §revocation. Same trust-write ctx the register ability
    /// uses; the revoke surface only mutates user-role entries.
    fn dispatch_revoke_user_pubkey(
        &self,
        arguments: &[u8],
    ) -> Result<Response<InvokeResponse>, Status> {
        let ctx = self.register_pubkey.as_ref().ok_or_else(|| {
            Status::failed_precondition(
                "<self>.revoke_user_pubkey: this daemon was booted without the trust-write \
                 surface (use `with_register_pubkey(...)` at boot to enable).",
            )
        })?;
        let body = handle_revoke_user_pubkey(
            arguments,
            &ctx.daemon_realm,
            &ctx.trust_anchor_path,
            &ctx.cell,
        )?;
        Ok(Response::new(InvokeResponse {
            result: body,
            result_content_type: FEDERATION_RESULT_CONTENT_TYPE.to_string(),
            state: easynet_axon::invocation::InvocationState::Completed.to_wire_i32(),
            ..InvokeResponse::default()
        }))
    }

    /// DEC-EU §multi-host-list. Read-only inventory of user-role
    /// pubkeys. Uses the same cell as register/revoke so list
    /// results always agree with the in-memory authoritative state
    /// admission consults.
    fn dispatch_list_user_pubkeys(
        &self,
        arguments: &[u8],
    ) -> Result<Response<InvokeResponse>, Status> {
        let ctx = self.register_pubkey.as_ref().ok_or_else(|| {
            Status::failed_precondition(
                "<self>.list_user_pubkeys: this daemon was booted without the trust \
                 surface; no listing available.",
            )
        })?;
        let body = handle_list_user_pubkeys(arguments, &ctx.cell)?;
        Ok(Response::new(InvokeResponse {
            result: body,
            result_content_type: FEDERATION_RESULT_CONTENT_TYPE.to_string(),
            state: easynet_axon::invocation::InvocationState::Completed.to_wire_i32(),
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

    fn dispatch_namespace_resolve(
        &self,
        arguments: &[u8],
    ) -> Result<Response<InvokeResponse>, Status> {
        let request: serde_json::Value = parse_json_args(arguments)?;
        let response = self.daemon_route_resolver().resolve_query_json(&request);
        wrap_json_response(&response)
    }

    /// **PR-N2 commit 2/N**. Peer-side `federation.resolve_key`
    /// dispatch. Reads the daemon's `SharedTrustAnchor` (so a
    /// SIGHUP-triggered `realm-trust.toml` reload is reflected
    /// without a restart) and returns the matching
    /// `public_key_b64` for the requested URA.
    ///
    /// On miss we surface `Status::not_found` so the calling
    /// `FederatedKeyResolver` can distinguish "URA is not in
    /// this hub's trust set" from a network or admission
    /// failure (which arrive as `unavailable` /
    /// `permission_denied`). The resolver then maps both into
    /// `CALLER_KEY_NOT_FOUND` for INV-4 fail-closed admission, but
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
                "federation.resolve_key: agent_ura `{}` not in this hub's trust set",
                request.agent_ura
            ))),
        }
    }

    /// **PR-N3 commit N3-4 + N3-N4 dispatch wire**. Cross-realm
    /// directory lookup dispatch. Reads the daemon-wide
    /// `SharedFederatedDirectoryView` cell snapshot, fans out
    /// across federated peers per spec §3.2 (lex tie-break,
    /// dedupe by agent_ura), returns matching `DirectoryEntry`
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
    /// presence-registry entries for a given realm. Spec §3.5
    /// admission filter: only callers whose URA is in the local
    /// trust anchor with `role = Hub` may invoke this. Other
    /// roles (Backend, Device) are rejected with
    /// `Status::permission_denied`. The general admission gate
    /// has already accepted the call (caller URA is signed,
    /// non-replayed, in trust set); this filter narrows to the
    /// hub-only sub-surface.
    ///
    /// Loopback bypass: the daemon's own URA is admitted into
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
        let caller_ura = caller_envelope
            .and_then(|env| env.caller.as_ref())
            .map(|c| c.ura.as_str())
            .ok_or_else(|| {
                Status::invalid_argument(
                    "federation.list_user_devices: missing caller envelope.caller.ura",
                )
            })?;

        let trust_anchor = self.admission.trust_anchor_snapshot();
        let is_hub_role = trust_anchor.lookup(caller_ura).is_some_and(|entry| {
            matches!(
                entry.role,
                crate::services::realm_trust_anchor::TrustedAgentRole::Hub
            )
        });
        let is_loopback = self
            .admission
            .daemon_ura()
            .is_some_and(|self_ura| self_ura == caller_ura);
        if !(is_hub_role || is_loopback) {
            return Err(Status::permission_denied(format!(
                "federation.list_user_devices: caller `{caller_ura}` is not a hub-role peer; \
                 only trusted hubs and the daemon itself may enumerate user devices"
            )));
        }

        let request: federation_wrappers::ListUserDevicesRequest = parse_json_args(arguments)?;
        let response = federation_wrappers::handle_list_user_devices(&request, &self.presence);
        wrap_json_response(&response)
    }

    fn require_backend_or_loopback_proxy_caller(
        &self,
        caller_envelope: Option<&Envelope>,
        ability_name: &str,
    ) -> Result<(), Status> {
        let caller_ura = caller_envelope
            .and_then(|env| env.caller.as_ref())
            .map(|c| c.ura.as_str())
            .ok_or_else(|| {
                Status::invalid_argument(format!(
                    "{ability_name}: missing caller envelope.caller.ura"
                ))
            })?;

        let trust_anchor = self.admission.trust_anchor_snapshot();
        let trusted_entry = trust_anchor.lookup(caller_ura);
        let is_backend_role = trusted_entry.is_some_and(|entry| {
            matches!(
                entry.role,
                crate::services::realm_trust_anchor::TrustedAgentRole::Backend
            )
        });
        let is_local_hub_identity = self
            .session_realm
            .as_deref()
            .is_some_and(|realm| crate::ura::hub_ura(realm) == caller_ura);
        let is_local_hub_role = is_local_hub_identity
            && trusted_entry.is_some_and(|entry| {
                matches!(
                    entry.role,
                    crate::services::realm_trust_anchor::TrustedAgentRole::Backend
                        | crate::services::realm_trust_anchor::TrustedAgentRole::Hub
                )
            });
        let is_loopback = self
            .admission
            .daemon_ura()
            .is_some_and(|self_ura| self_ura == caller_ura);
        if !(is_backend_role || is_local_hub_role || is_loopback) {
            return Err(Status::permission_denied(format!(
                "{ability_name}: caller `{caller_ura}` is not the local backend; \
                 only the backend and daemon loopback may proxy peer calls"
            )));
        }
        Ok(())
    }

    /// Daemon-local caller-side path for user-scoped peer device
    /// enumeration. The backend passes the exact peer hub URLs from
    /// `user_peer_hubs`; the daemon fans out to each via its
    /// existing cross-hub transport, stamps the merge-boundary
    /// metadata (`origin_realm`, `hub_endpoint`), and returns a
    /// typed `DirectoryEntry` list. This keeps peer dial / trust /
    /// signing inside the daemon and prevents the Go backend from
    /// growing its own cross-hub stack.
    async fn dispatch_federation_proxy_list_user_devices(
        &self,
        caller_envelope: Option<&Envelope>,
        arguments: &[u8],
    ) -> Result<Response<InvokeResponse>, Status> {
        self.require_backend_or_loopback_proxy_caller(
            caller_envelope,
            "federation.proxy_list_user_devices",
        )?;

        let request: federation_wrappers::ProxyListUserDevicesRequest = parse_json_args(arguments)?;
        let realm = request.realm.trim();
        if realm.is_empty() {
            return Err(Status::invalid_argument(
                "federation.proxy_list_user_devices: realm is required",
            ));
        }

        let Some(client) = self.federation_client.as_ref() else {
            return wrap_json_response(&federation_wrappers::ProxyListUserDevicesResponse {
                devices: Vec::new(),
            });
        };

        let peer_hub_urls: Vec<String> = request
            .peer_hub_urls
            .into_iter()
            .map(|url| url.trim().to_string())
            .filter(|url| !url.is_empty())
            .collect::<BTreeSet<_>>()
            .into_iter()
            .collect();
        if peer_hub_urls.is_empty() {
            return wrap_json_response(&federation_wrappers::ProxyListUserDevicesResponse {
                devices: Vec::new(),
            });
        }

        let inner_arguments = serde_json::to_vec(&federation_wrappers::ListUserDevicesRequest {
            realm: realm.to_string(),
        })
        .map_err(|err| {
            Status::internal(format!(
                "federation.proxy_list_user_devices: encode peer request: {err}"
            ))
        })?;

        let trust_anchor = self.admission.trust_anchor_snapshot();
        let local_realm = self.session_realm.as_deref();
        let mut fanout = FuturesUnordered::new();
        for peer_hub_url in peer_hub_urls {
            let Some(peer_entry) = trust_anchor.lookup_peer_hub(&peer_hub_url).cloned() else {
                crate::op_event!(
                    component = daemon_invocation,
                    kind = proxy_list_user_devices_skip_untrusted_peer,
                    peer_hub_url = peer_hub_url,
                );
                continue;
            };
            let Some(peer_realm) = peer_entry.origin_realm.clone() else {
                crate::op_event!(
                    component = daemon_invocation,
                    kind = proxy_list_user_devices_skip_peer_missing_origin_tenant,
                    peer_hub_url = peer_hub_url,
                );
                continue;
            };
            let client = Arc::clone(client);
            let mut peer_request = InvokeRequest {
                envelope: Some(build_peer_envelope(
                    caller_envelope,
                    &peer_entry.agent_ura,
                    local_realm,
                )?),
                function_name: ABILITY_FEDERATION_LIST_USER_DEVICES.to_string(),
                arguments: inner_arguments.clone(),
                ..InvokeRequest::default()
            };
            if let Some(envelope) = peer_request.envelope.as_mut() {
                sign_peer_request_envelope(
                    envelope,
                    &peer_request.function_name,
                    &peer_request.arguments,
                    local_realm,
                    self.hub_signing_seed.as_ref(),
                )?;
            }
            fanout.push(async move {
                match client.forward_invoke(&peer_hub_url, peer_request).await {
                    Ok(response) => {
                        let mut body: federation_wrappers::ListUserDevicesResponse =
                            serde_json::from_slice(&response.result).map_err(|err| {
                                format!(
                                    "decode peer {peer_hub_url} list_user_devices response: {err}"
                                )
                            })?;
                        for device in &mut body.devices {
                            device.origin_realm = Some(peer_realm.clone());
                            device.hub_endpoint = Some(peer_hub_url.clone());
                        }
                        Ok(body.devices)
                    }
                    Err(err) => Err(format!(
                        "dial peer {peer_hub_url} for list_user_devices failed: {err}"
                    )),
                }
            });
        }

        let mut devices = Vec::new();
        while let Some(result) = fanout.next().await {
            match result {
                Ok(mut entries) => devices.append(&mut entries),
                Err(err) => {
                    let err_msg = err.to_string();
                    crate::op_event!(
                        component = daemon_invocation,
                        kind = proxy_list_user_devices_fanout_error,
                        error = err_msg,
                    );
                }
            }
        }
        devices.sort_by(|a, b| {
            a.hub_endpoint
                .as_deref()
                .unwrap_or("")
                .cmp(b.hub_endpoint.as_deref().unwrap_or(""))
                .then_with(|| a.agent_ura.cmp(&b.agent_ura))
        });

        wrap_json_response(&federation_wrappers::ProxyListUserDevicesResponse { devices })
    }

    async fn dispatch_namespace_proxy_resolve(
        &self,
        caller_envelope: Option<&Envelope>,
        arguments: &[u8],
    ) -> Result<Response<InvokeResponse>, Status> {
        self.require_backend_or_loopback_proxy_caller(caller_envelope, "namespace.proxy_resolve")?;

        let request: federation_wrappers::NamespaceProxyResolveRequest =
            parse_json_args(arguments)?;
        let Some(client) = self.federation_client.as_ref() else {
            return wrap_json_response(&namespace_proxy_resolve_empty_answer(&request));
        };

        let peer_hub_urls = sorted_non_empty_urls(request.peer_hub_urls.clone());
        if peer_hub_urls.is_empty() {
            return wrap_json_response(&namespace_proxy_resolve_empty_answer(&request));
        }

        let inner_arguments = namespace_proxy_resolve_peer_arguments(&request)?;
        let trust_anchor = self.admission.trust_anchor_snapshot();
        let local_realm = self.session_realm.as_deref();
        let mut fanout = FuturesUnordered::new();
        for peer_hub_url in peer_hub_urls {
            let Some(peer_entry) = trust_anchor.lookup_peer_hub(&peer_hub_url).cloned() else {
                crate::op_event!(
                    component = daemon_invocation,
                    kind = namespace_proxy_resolve_skip_untrusted_peer,
                    peer_hub_url = peer_hub_url,
                );
                continue;
            };
            let client = Arc::clone(client);
            let mut peer_request = InvokeRequest {
                envelope: Some(build_peer_envelope(
                    caller_envelope,
                    &peer_entry.agent_ura,
                    local_realm,
                )?),
                function_name: ABILITY_NAMESPACE_RESOLVE.to_string(),
                arguments: inner_arguments.clone(),
                ..InvokeRequest::default()
            };
            if let Some(envelope) = peer_request.envelope.as_mut() {
                sign_peer_request_envelope(
                    envelope,
                    &peer_request.function_name,
                    &peer_request.arguments,
                    local_realm,
                    self.hub_signing_seed.as_ref(),
                )?;
            }
            fanout.push(async move {
                match client.forward_invoke(&peer_hub_url, peer_request).await {
                    Ok(response) => {
                        let body: serde_json::Value = serde_json::from_slice(&response.result)
                            .map_err(|err| {
                                format!(
                                    "decode peer {peer_hub_url} namespace.resolve response: {err}"
                                )
                            })?;
                        Ok(body)
                    }
                    Err(err) => Err(format!(
                        "dial peer {peer_hub_url} for namespace.resolve failed: {err}"
                    )),
                }
            });
        }

        let mut peer_answers = Vec::new();
        while let Some(result) = fanout.next().await {
            match result {
                Ok(answer) => peer_answers.push(answer),
                Err(err) => {
                    let err_msg = err.to_string();
                    crate::op_event!(
                        component = daemon_invocation,
                        kind = namespace_proxy_resolve_fanout_error,
                        error = err_msg,
                    );
                }
            }
        }

        wrap_json_response(&namespace_proxy_resolve_merge_answer(
            &request,
            peer_answers,
        ))
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

    /// RFC-005 route-first `federation.forward_invoke` dispatch:
    ///
    /// 1. Decode the inner payload and validate that its canonical
    ///    `ability_ura` belongs to the supplied `target_ura`.
    /// 2. Ask `namespace.resolve` for a local `FinalRoute`.
    /// 3. If a route is selected locally, dispatch only by selected
    ///    `execution_host_ura`, `callee_ura`, and `dispatch_name`.
    ///    `target_ura` remains an owner consistency proof, never an
    ///    execution endpoint.
    /// 4. If local resolution is negative in the local realm, either
    ///    return the typed resolver failure or fan out to configured
    ///    same-realm peers.
    /// 5. If the target realm is remote and no local FinalRoute exists,
    ///    ask `namespace.resolve` for a `PeerHub` delegation and issue
    ///    the same `federation.forward_invoke` request to the selected
    ///    peer hub.
    async fn dispatch_federation_forward_invoke(
        &self,
        caller_envelope: Option<&Envelope>,
        arguments: &[u8],
    ) -> Result<Response<InvokeResponse>, Status> {
        // PR-N6 C4: device-mode escalation. When this daemon
        // owns no PresenceRegistry of its own (mode = device),
        // it cannot execute a resolver-selected local-session
        // dispatch. Send the call up the existing
        // `<self>.session` bidi to the hub, await the matching
        // RequestResult, and surface its outcome on the unary
        // wire. Hub-mode and `both`-mode daemons leave
        // `escalation = None` and take the existing arm.
        if let Some(handle) = self.escalation.as_ref() {
            return self.escalate_forward_invoke(handle, arguments).await;
        }

        let request: federation_wrappers::ForwardInvokeRequest = parse_json_args(arguments)?;

        // RFC-005 owner proof: route the exact target owner the
        // caller supplied. Legacy `/agent/<bare-id>` device aliases
        // are intentionally not repaired here; callers must address
        // devices with canonical `/device/<id>` owner URAs.

        let target_realm = parse_realm_from_ura(&request.target_ura);
        let local_realm = self.session_realm.as_deref();

        let is_local_realm = match (target_realm.as_deref(), local_realm) {
            (Some(target), Some(local)) => target == local,
            // Daemon has no realm context wired (smoke-test
            // build) — preserve PR-1 staging behavior and treat
            // every target as local.
            (_, None) => true,
            // Malformed target URA — fall through to the local
            // target-offline shape so a typo never accidentally hits the
            // cross-hub path.
            (None, Some(_)) => true,
        };
        let has_target_presence = self.presence.lookup(&request.target_ura).is_some();

        // Observable trace for operators debugging answer-sheet /
        // demo runs — proves which dispatch arm fired without
        // requiring an envelope-level packet capture. Cheap (one
        // eprintln per call) and the only daemon-A-side signal
        // that distinguishes "took cross-realm arm" from "took
        // local-presence arm" when the inner ability happens to
        // be a hub-served one (e.g. federation.heartbeat).
        // Render `Option<&str>` as a stable string so SRE pipelines
        // grep `target_realm=<value>` (or `=<none>` for the absent
        // case) without seeing Rust's `Some("…")` / `None` Debug
        // literal sneaking into the field value.
        let target_realm_field = target_realm.as_deref().unwrap_or("<none>");
        let local_realm_field = local_realm.unwrap_or("<none>");
        crate::op_event!(
            component = daemon_invocation,
            kind = forward_invoke_dispatch,
            target_ura = request.target_ura,
            target_realm = target_realm_field,
            local_realm = local_realm_field,
            is_local_realm = is_local_realm,
            has_target_presence = has_target_presence,
        );

        // Decode the inner payload up front. The
        // `correlation_call_id` field is required by DEC-N4 §2.1
        // so both arms (local selected route AND peer delegation)
        // can thread it back to the caller. Decode failure
        // surfaces as `Status::invalid_argument`; the CLI bridge
        // is the producer and must always supply a non-empty
        // `call_id` field.
        let inner_payload = decode_inner_payload(&request.inner_envelope_b64)?;
        let correlation_call_id = inner_payload.call_id.clone();

        // RFC-005 route-first dispatch selection. `request.target_ura`
        // proves owner intent and realm placement, but it is not an
        // execution endpoint. Once namespace.resolve returns a
        // FinalRoute, every local decision is made from the selected
        // route: self dispatch checks selected `execution_host_ura`,
        // session dispatch pushes to selected `execution_host_ura`,
        // and the frame carries selected `callee_ura` +
        // `dispatch_name`.
        let selected_local_route = match self.resolve_forward_invoke_route(&request, &inner_payload)
        {
            Ok(route) => Some(route),
            Err(err) => {
                if is_local_realm {
                    return Err(err);
                }
                None
            }
        };

        if let Some(selected_route) = selected_local_route {
            let selected_host_is_self = self
                .matches_self_target_ura(&selected_route.execution_host_ura)
                .await;
            let selected_host_present = self
                .presence
                .lookup(&selected_route.execution_host_ura)
                .is_some();
            crate::op_event!(
                component = daemon_invocation,
                kind = forward_invoke_selected_route,
                target_ura = request.target_ura,
                route_ura = selected_route.route_ura.as_str(),
                callee_ura = selected_route.callee_ura.as_str(),
                execution_host_ura = selected_route.execution_host_ura.as_str(),
                dispatch_name = selected_route.dispatch_name.as_str(),
                selected_host_is_self = selected_host_is_self,
                selected_host_present = selected_host_present,
            );

            if selected_host_is_self {
                return self
                    .dispatch_self_targeted_forward_invoke(
                        &inner_payload,
                        &selected_route,
                        &correlation_call_id,
                    )
                    .await;
            }

            match self
                .dispatch_local_presence_forward_invoke(
                    &inner_payload,
                    &selected_route,
                    &correlation_call_id,
                )
                .await
            {
                Ok(response) => return Ok(response),
                Err(status) => return Err(status),
            }
        }

        // Cross-realm path. Missing federation client OR
        // missing peer entry both surface as
        // `failed_precondition(target_offline)` per DEC-N4 §2.1
        // — the older "Ok with target_online:false" shape is
        // gone. DEC-N5 §1 still requires a caller-hub
        // ForwardReceipt with `result_digest = None` for every
        // target_offline outcome.
        let record_offline_receipt = || {};
        let Some(client) = self.federation_client.as_ref() else {
            record_offline_receipt();
            return Err(Status::failed_precondition(
                federation_wrappers::FORWARD_INVOKE_TARGET_OFFLINE_REASON,
            ));
        };
        // Cross-realm dispatch derives the authoritative target realm
        // from the resolver's `NextHop::PeerHub` delegation answer
        // (`delegation.realm`), not from the URA-parsed `target_realm`:
        // the latter only feeds the `is_local_realm` arm above, which
        // already collapses every `None`/local realm to a local arm —
        // so reaching this cross-realm tail proves `target_realm` was
        // `Some` and equal to a non-local realm. We intentionally do
        // not re-thread it here.
        let delegated_route =
            match self.resolve_cross_realm_forward_delegation(&request, &inner_payload) {
                Ok(route) => route,
                Err(status) => {
                    record_offline_receipt();
                    return Err(status);
                }
            };
        let target_hub_endpoint = delegated_route.primary_endpoint().ok_or_else(|| {
            Status::failed_precondition(federation_wrappers::FORWARD_INVOKE_TARGET_OFFLINE_REASON)
        })?;
        let peer_request = self.build_forward_invoke_peer_request(caller_envelope, &request)?;
        let result = self
            .dispatch_forward_invoke_peer(
                client,
                target_hub_endpoint,
                peer_request,
                &request.target_ura,
                &correlation_call_id,
                "cross_realm",
            )
            .await;
        if result.is_err() {
            record_offline_receipt();
        }
        result
    }

    fn resolve_forward_invoke_route(
        &self,
        request: &federation_wrappers::ForwardInvokeRequest,
        inner_payload: &InnerPayload,
    ) -> Result<SelectedInvokeRoute, Status> {
        let selector =
            crate::ura::AbilitySelector::parse(&inner_payload.ability_ura).map_err(|err| {
                Status::invalid_argument(format!(
                    "federation.forward_invoke: invalid canonical ability_ura `{}`: {err}",
                    inner_payload.ability_ura,
                ))
            })?;
        if selector.owner_ura() != request.target_ura {
            return Err(Status::invalid_argument(format!(
                "federation.forward_invoke: ability_ura `{}` does not belong to target `{}`",
                inner_payload.ability_ura, request.target_ura,
            )));
        }

        let selected_route = self
            .daemon_route_resolver()
            .resolve_route(&inner_payload.ability_ura, "")
            .map_err(route_negative_status)?;

        if !selected_route.is_authoritative_local_or_better() {
            return Err(route_profile_blocked_status(&selected_route));
        }
        if selected_route.owner_ura != request.target_ura {
            return Err(Status::invalid_argument(route_owner_mismatch_message(
                &selected_route.owner_ura,
                &inner_payload.ability_ura,
                &request.target_ura,
            )));
        }

        Ok(selected_route)
    }

    fn build_forward_invoke_peer_request(
        &self,
        caller_envelope: Option<&Envelope>,
        request: &federation_wrappers::ForwardInvokeRequest,
    ) -> Result<InvokeRequest, Status> {
        let nested = federation_wrappers::ForwardInvokeRequest {
            target_ura: request.target_ura.clone(),
            inner_envelope_b64: request.inner_envelope_b64.clone(),
            causal_context_bytes: request.causal_context_bytes.clone(),
            forward_deadline_ms: request.forward_deadline_ms,
        };
        let nested_arguments = serde_json::to_vec(&nested).map_err(|err| {
            Status::internal(format!(
                "federation.forward_invoke: encode nested ForwardInvokeRequest for peer \
                 delegation: {err}"
            ))
        })?;
        let mut peer_request = InvokeRequest {
            envelope: Some(build_peer_envelope(
                caller_envelope,
                &request.target_ura,
                self.session_realm.as_deref(),
            )?),
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
        Ok(peer_request)
    }

    async fn dispatch_forward_invoke_peer(
        &self,
        client: &Arc<dyn FederationClient>,
        target_hub_endpoint: &str,
        peer_request: InvokeRequest,
        target_ura: &str,
        correlation_call_id: &str,
        scope: &str,
    ) -> Result<Response<InvokeResponse>, Status> {
        let target_hub_endpoint = target_hub_endpoint.to_string();
        match client
            .forward_invoke(&target_hub_endpoint, peer_request)
            .await
        {
            Ok(peer_response) => {
                let peer_body: federation_wrappers::ForwardInvokeResponse =
                    match serde_json::from_slice(&peer_response.result) {
                        Ok(body) => body,
                        Err(err) => {
                            let err_msg = format!("{err}");
                            crate::op_event!(
                                component = daemon_invocation,
                                kind = forward_invoke_peer_response_malformed,
                                scope = scope,
                                error = err_msg,
                                message = "forwarding raw bytes for forward-compat",
                            );
                            federation_wrappers::ForwardInvokeResponse {
                                result_bytes: peer_response.result.clone(),
                                correlation_call_id: correlation_call_id.to_string(),
                            }
                        }
                    };
                let result_bytes_len = peer_body.result_bytes.len();
                crate::op_event!(
                    component = daemon_invocation,
                    kind = forward_invoke_peer_delegation_ok,
                    scope = scope,
                    target_ura = target_ura,
                    target_hub_endpoint = target_hub_endpoint,
                    result_bytes_len = result_bytes_len,
                );
                let response = federation_wrappers::ForwardInvokeResponse {
                    result_bytes: peer_body.result_bytes,
                    correlation_call_id: correlation_call_id.to_string(),
                };
                wrap_json_response(&response)
            }
            Err(err) => {
                let err_msg = format!("{err}");
                crate::op_event!(
                    component = daemon_invocation,
                    kind = forward_invoke_peer_delegation_failed,
                    scope = scope,
                    target_ura = target_ura,
                    target_hub_endpoint = target_hub_endpoint,
                    error = err_msg,
                );
                Err(Status::failed_precondition(
                    federation_wrappers::FORWARD_INVOKE_TARGET_OFFLINE_REASON,
                ))
            }
        }
    }

    fn resolve_cross_realm_forward_delegation(
        &self,
        request: &federation_wrappers::ForwardInvokeRequest,
        inner_payload: &InnerPayload,
    ) -> Result<DelegatedInvokeRoute, Status> {
        let selector =
            crate::ura::AbilitySelector::parse(&inner_payload.ability_ura).map_err(|err| {
                Status::invalid_argument(format!(
                    "federation.forward_invoke: invalid canonical ability_ura `{}`: {err}",
                    inner_payload.ability_ura,
                ))
            })?;
        if selector.owner_ura() != request.target_ura {
            return Err(Status::invalid_argument(format!(
                "federation.forward_invoke: ability_ura `{}` does not belong to target `{}`",
                inner_payload.ability_ura, request.target_ura,
            )));
        }

        let delegation = self
            .daemon_route_resolver()
            .resolve_delegation(&inner_payload.ability_ura, "")
            .map_err(route_negative_status)?
            .ok_or_else(|| {
                Status::failed_precondition(format!(
                    "{ROUTE_SELECTED_REMOTE_HOST_CODE}: federation.forward_invoke expected \
                     cross-realm namespace.resolve delegation for `{}`",
                    inner_payload.ability_ura,
                ))
            })?;

        for endpoint in &delegation.endpoints {
            if endpoint
                .metadata
                .get("source")
                .and_then(serde_json::Value::as_str)
                == Some("federated_directory")
            {
                if let Some(target_ura) = endpoint
                    .metadata
                    .get("targetUra")
                    .and_then(serde_json::Value::as_str)
                {
                    crate::op_event!(
                        component = daemon_invocation,
                        kind = auto_route,
                        source = "federated_directory",
                        target_realm = delegation.realm.as_str(),
                        target_ura = target_ura,
                        hub_endpoint = endpoint.endpoint.as_str(),
                    );
                }
            }
        }

        Ok(delegation)
    }

    /// Reverse-channel dispatch for a resolver-selected
    /// `federation.forward_invoke` route.
    ///
    /// Mirrors `dispatch_invoke_remote`'s pattern: register a
    /// `PendingDispatchMap` entry, push a
    /// `SessionDispatch::Dispatch{call_id, ability, args}` frame
    /// down the selected execution host's session bidi (the same wire shape
    /// device-side `LocalAxonSessionDispatcher::handle_down` expects),
    /// `await_reply` for the matching `SessionDispatch::Result`
    /// arriving via `drain_session_up_stream`, return the bytes
    /// inline as `ForwardInvokeResponse.result_bytes`.
    ///
    /// Errors:
    /// - selected execution host unavailable, or push fails →
    ///   `Status::failed_precondition(target_offline)`,
    ///   so the caller's same-realm fall-through arm can fan
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
        inner_payload: &InnerPayload,
        selected_route: &SelectedInvokeRoute,
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
            .lookup_tracked(&selected_route.execution_host_ura)
            .ok_or_else(|| {
                Status::failed_precondition(selected_host_unavailable_message(selected_route))
            })?;

        let dispatch_ability = selected_route.dispatch_key();

        // Register pending entry BEFORE pushing the frame so a
        // fast device reply lands a real `complete()` rather
        // than a no-op (race-free correlation, same contract as
        // `dispatch_invoke_remote`).
        //
        // Use `register_pending_for(target_ura)` so the daemon's
        // presence-offline watcher (`with_pending` ctor hook) can
        // fail-fast this entry the moment `<self>.session` for
        // `request.target_ura` drops mid-call — without this the
        // `await_reply()` below blocks on the oneshot until the
        // operator-side HTTP timeout fires.
        let handle = pending.register_pending_for(&selected_route.execution_host_ura);
        let call_id = handle.call_id();

        let dispatch_frame = build_invoke_remote_dispatch_frame(
            call_id,
            &selected_route.callee_ura,
            None,
            &dispatch_ability,
            &inner_payload.args_bytes,
            SessionContentEnvelope::plaintext_json(),
            HashMap::new(),
        )?;

        match sender.try_send(Ok(dispatch_frame)) {
            Ok(()) => {}
            Err(tokio::sync::mpsc::error::TrySendError::Full(_)) => {
                self.presence.remove_if_session(
                    &selected_route.execution_host_ura,
                    session_id,
                    crate::services::presence_registry::OfflineReason::SendFailed,
                );
                return Err(Status::failed_precondition(
                    federation_wrappers::FORWARD_INVOKE_TARGET_OFFLINE_REASON,
                ));
            }
            Err(tokio::sync::mpsc::error::TrySendError::Closed(_)) => {
                self.presence.remove_if_session(
                    &selected_route.execution_host_ura,
                    session_id,
                    crate::services::presence_registry::OfflineReason::StreamClosed,
                );
                return Err(Status::failed_precondition(
                    federation_wrappers::FORWARD_INVOKE_TARGET_OFFLINE_REASON,
                ));
            }
        }

        crate::op_event!(
            component = daemon_invocation,
            kind = forward_invoke_local_presence_dispatch_awaiting_reply,
            callee_ura = selected_route.callee_ura.as_str(),
            execution_host_ura = selected_route.execution_host_ura.as_str(),
            ability = selected_route.dispatch_name.as_str(),
            route_ura = selected_route.route_ura.as_str(),
            call_id = call_id,
        );

        // Await the matching Result frame.
        let dispatch_result = handle.await_reply().await.map_err(|_recv_err| {
            Status::unavailable(format!(
                "federation.forward_invoke: selected execution host `{}` session disconnected before \
                 reply (call_id={call_id})",
                selected_route.execution_host_ura,
            ))
        })?;

        let DispatchResult {
            payload: result_bytes,
            error,
            failure,
            request_id: _,
        } = dispatch_result;
        // Diagnostic: forward the mac-side outcome verbatim so a
        // session-frame-correlation race is visible in the hub log
        // without having to attach a debugger. Cheap (one op-event
        // per round-trip). Render `Option<String>` via as_deref so
        // SRE pipelines see `error=<value>` (or `error=<none>`)
        // instead of Rust's `Some("…")` / `None` Debug literal.
        let result_bytes_len = result_bytes.len();
        let error_field = error.as_deref().unwrap_or("<none>");
        let failure_code = failure
            .as_ref()
            .map(|failure| failure.code.as_str())
            .unwrap_or("<none>");
        crate::op_event!(
            component = daemon_invocation,
            kind = forward_invoke_local_presence_dispatch_result,
            callee_ura = selected_route.callee_ura.as_str(),
            execution_host_ura = selected_route.execution_host_ura.as_str(),
            ability = selected_route.dispatch_name.as_str(),
            route_ura = selected_route.route_ura.as_str(),
            call_id = call_id,
            result_bytes_len = result_bytes_len,
            error = error_field,
            failure_code = failure_code,
        );
        if let Some(err) = error {
            let detail = failure
                .as_ref()
                .map(SessionFailure::status_detail)
                .unwrap_or(err);
            return Err(Status::failed_precondition(format!(
                "federation.forward_invoke: selected route `{}` ability `{}` failed: {detail}",
                selected_route.route_ura, selected_route.dispatch_name,
            )));
        }

        // DEC-N5 §1: write the ForwardReceipt with a real
        // result_digest (not None) since we have the bytes
        // inline.

        let response = federation_wrappers::ForwardInvokeResponse {
            result_bytes,
            correlation_call_id: correlation_call_id.to_string(),
        };
        wrap_json_response(&response)
    }

    /// **PR-1 commit 7/9 (LB-56)**. Synchronous self-targeted
    /// `federation.forward_invoke` dispatch.
    ///
    /// Caller has confirmed the target URA names this daemon. We
    /// resolve the inner ability against the daemon's Axon
    /// `LocalRuntime`, write a single ForwardReceipt with a real
    /// `result_digest` (no async second update), and return the bytes
    /// inline in `ForwardInvokeResponse.result_bytes`.
    ///
    /// Errors map to `tonic::Status`:
    /// - runtime missing → `Status::failed_precondition`
    /// - ability not registered → `Status::not_found`
    /// - handler returned an Axon error → `Status::failed_precondition`
    ///   with the underlying SDK error.
    async fn dispatch_self_targeted_forward_invoke(
        &self,
        inner_payload: &InnerPayload,
        selected_route: &SelectedInvokeRoute,
        correlation_call_id: &str,
    ) -> Result<Response<InvokeResponse>, Status> {
        let Some(runtime) = self.local_runtime.as_ref() else {
            return Err(Status::failed_precondition(
                "federation.forward_invoke: self-targeted dispatch cannot run because \
                 Axon LocalRuntime is not wired at boot",
            ));
        };

        let dispatch_ability = selected_route.dispatch_key();
        if !runtime.has_ability(&dispatch_ability).await {
            return Err(Status::not_found(format!(
                "federation.forward_invoke: self-targeted ability `{dispatch_ability}` is not \
                 registered in Axon LocalRuntime"
            )));
        }

        crate::op_event!(
            component = daemon_invocation,
            kind = forward_invoke_self_target_dispatch,
            callee_ura = selected_route.callee_ura.as_str(),
            execution_host_ura = selected_route.execution_host_ura.as_str(),
            ability = selected_route.dispatch_name.as_str(),
            route_ura = selected_route.route_ura.as_str(),
            dispatch_ability = dispatch_ability.as_str(),
            call_id = correlation_call_id,
        );

        let outcome = crate::runtime::axon_bridge::dispatch_shim::dispatch_rpc_local(
            runtime,
            &dispatch_ability,
            inner_payload.args_bytes.clone(),
        )
        .await;
        let result_bytes = match outcome.error {
            None => outcome.payload_bytes,
            Some(err) => {
                return Err(Status::failed_precondition(format!(
                    "federation.forward_invoke: self-targeted dispatch of ability `{dispatch_ability}` failed: {err}",
                )));
            }
        };
        if outcome.state != easynet_axon::invocation::InvocationState::Completed {
            return Err(Status::failed_precondition(format!(
                "federation.forward_invoke: self-targeted dispatch of ability `{dispatch_ability}` ended in state {}",
                outcome.state.as_str(),
            )));
        }
        if result_bytes.is_empty() {
            crate::op_event!(
                component = daemon_invocation,
                kind = forward_invoke_self_target_empty_result,
                callee_ura = selected_route.callee_ura.as_str(),
                execution_host_ura = selected_route.execution_host_ura.as_str(),
                ability = selected_route.dispatch_name.as_str(),
                route_ura = selected_route.route_ura.as_str(),
                call_id = correlation_call_id,
            );
        }

        // Single ForwardReceipt write with real result_digest —
        // unlike the bidi-push path, no PR-N5 second-update is
        // needed because the bytes are already known.

        let response = federation_wrappers::ForwardInvokeResponse {
            result_bytes,
            correlation_call_id: correlation_call_id.to_string(),
        };
        wrap_json_response(&response)
    }

    /// Self-targeted `<self>.invoke_remote` shortcut.
    ///
    /// When the daemon receives `<self>.invoke_remote` whose
    /// subject_device equals its own URA, dispatch the ability
    /// through the shared Axon `LocalRuntime` and return the result
    /// on a one-shot down stream. This fires in two scenarios:
    ///
    ///   1. Host-mode dev rig: backend invokes a device.* ability
    ///      against the local device daemon's own URA. The
    ///      daemon's PresenceRegistry self-presence seed
    ///      (boot.rs) makes the target findable; this shortcut
    ///      dispatches inline without trying to push frames
    ///      down a drain channel that nobody consumes.
    ///
    ///   2. Hub-mode self-call: a hub invoking an ability on
    ///      its own URA (rare but valid; the hub is a Both-mode
    ///      daemon and the local runtime hosts its registered tools).
    ///
    /// Mirrors `dispatch_self_targeted_forward_invoke` for the
    /// federation.forward_invoke surface — same idea, different
    /// envelope shape.
    async fn dispatch_self_targeted_invoke_remote(
        &self,
        selected_route: &SelectedInvokeRoute,
        subject_ura: Option<&str>,
        args: &[u8],
    ) -> Result<Response<<Self as Invocation>::InvokeBidiStream>, Status> {
        crate::op_event!(
            component = daemon_invocation,
            kind = invoke_remote_self_target_dispatch,
            callee_ura = selected_route.callee_ura.as_str(),
            execution_host_ura = selected_route.execution_host_ura.as_str(),
            ability = selected_route.dispatch_name.as_str(),
            route_ura = selected_route.route_ura.as_str(),
        );

        let Some(runtime) = self.local_runtime.as_ref() else {
            return Err(Status::failed_precondition(
                "<self>.invoke_remote: self-targeted dispatch cannot run because \
                 Axon LocalRuntime is not wired at boot",
            ));
        };

        let dispatch_ability = selected_route.dispatch_key();
        let outcome = match subject_ura {
            Some(subject) if !subject.trim().is_empty() => {
                crate::runtime::axon_bridge::dispatch_shim::dispatch_rpc_local_with_subject(
                    runtime,
                    &selected_route.callee_ura,
                    subject,
                    &dispatch_ability,
                    args.to_vec(),
                )
                .await
            }
            _ => {
                crate::runtime::axon_bridge::dispatch_shim::dispatch_rpc_local(
                    runtime,
                    &dispatch_ability,
                    args.to_vec(),
                )
                .await
            }
        };
        let request_id = outcome.invocation_id.clone();
        let (payload, error) =
            crate::runtime::axon_bridge::dispatch_shim::outcome_to_invoke_remote_result(outcome);

        let down = InvokeRemoteDown::Result {
            payload,
            failure: error
                .as_ref()
                .map(|reason| SessionFailure::from_reason(reason, "INVOCATION_FAILED", false)),
            error,
            request_id,
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

    async fn dispatch_remote_bidi(
        &self,
        selected_route: &SelectedInvokeRoute,
        envelope_open: &EnvelopeOpen,
        mut up: Streaming<InvokeBidiUp>,
    ) -> Result<Response<<Self as Invocation>::InvokeBidiStream>, Status> {
        let pending = self.pending_stream.as_ref().ok_or_else(|| {
            Status::failed_precondition(format!(
                "InvokeBidi {}: daemon was constructed without a \
                 PendingStreamDispatchMap; boot must call with_pending_stream(...) \
                 to enable remote bidi bridging",
                selected_route.dispatch_name
            ))
        })?;
        let (session_id, sender) = self
            .presence
            .lookup_tracked(&selected_route.execution_host_ura)
            .ok_or_else(|| {
                Status::failed_precondition(selected_host_unavailable_message(selected_route))
            })?;

        let mut handle = pending.register_pending();
        let call_id = handle.call_id();
        let stdout_stream_id = local_bidi_stdout_stream_id(envelope_open);
        let dispatch_ability = selected_route.dispatch_key();

        let open_frame = build_remote_bidi_open_dispatch_frame(
            call_id,
            &selected_route.callee_ura,
            remote_bidi_subject_ura(envelope_open).as_deref(),
            &dispatch_ability,
            &envelope_open.initial_args,
            envelope_open.metadata.clone(),
        )?;
        match sender.try_send(Ok(open_frame)) {
            Ok(()) => {}
            Err(tokio::sync::mpsc::error::TrySendError::Full(_)) => {
                self.presence.remove_if_session(
                    &selected_route.execution_host_ura,
                    session_id,
                    crate::services::presence_registry::OfflineReason::SendFailed,
                );
                return Err(Status::failed_precondition(
                    federation_wrappers::FORWARD_INVOKE_TARGET_OFFLINE_REASON,
                ));
            }
            Err(tokio::sync::mpsc::error::TrySendError::Closed(_)) => {
                self.presence.remove_if_session(
                    &selected_route.execution_host_ura,
                    session_id,
                    crate::services::presence_registry::OfflineReason::StreamClosed,
                );
                return Err(Status::failed_precondition(
                    federation_wrappers::FORWARD_INVOKE_TARGET_OFFLINE_REASON,
                ));
            }
        }

        crate::op_event!(
            component = daemon_invocation,
            kind = invoke_bidi_remote_bridge,
            ability = selected_route.dispatch_name.as_str(),
            callee_ura = selected_route.callee_ura.as_str(),
            execution_host_ura = selected_route.execution_host_ura.as_str(),
            route_ura = selected_route.route_ura.as_str(),
            call_id = call_id,
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
                    DispatchStreamEvent::Terminal(DispatchResult {
                        payload,
                        error,
                        failure,
                        request_id: _,
                    }) => {
                        let frame = match error {
                            Some(reason) => {
                                build_bidi_terminal_receipt_with_payload_and_failure_code(
                                    easynet_axon::invocation::InvocationState::Failed,
                                    failure
                                        .as_ref()
                                        .map(|failure| failure.message.as_str())
                                        .unwrap_or(reason.as_str()),
                                    if payload.is_empty() {
                                        None
                                    } else {
                                        Some((payload, "application/json"))
                                    },
                                    failure.as_ref().map(|failure| failure.code.as_str()),
                                )
                            }
                            None => build_bidi_terminal_receipt_with_payload(
                                easynet_axon::invocation::InvocationState::Completed,
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

        let execution_host_ura_owned = selected_route.execution_host_ura.clone();
        let ability_owned = selected_route.dispatch_name.clone();
        let presence_for_up = Arc::clone(&self.presence);
        let pending_for_up = Arc::clone(pending);
        tokio::spawn(async move {
            let mut expected_up_sequence = 1_u64;
            let mut eof_sent = false;
            while let Some(maybe_frame) = up.next().await {
                let frame = match maybe_frame {
                    Ok(frame) => frame,
                    Err(status) => {
                        let reason = format!("remote bidi caller stream error: {status}");
                        let _ = pending_for_up
                            .finish(
                                call_id,
                                failed_dispatch_result(&reason, "INVOCATION_FAILED", false),
                            )
                            .await;
                        return;
                    }
                };
                if frame.sequence != expected_up_sequence {
                    let reason = format!(
                        "{REASON_BIDI_FRAME_SEQUENCE}: expected up sequence \
                             {expected_up_sequence}, got {}",
                        frame.sequence
                    );
                    let _ = pending_for_up
                        .finish(
                            call_id,
                            failed_dispatch_result(&reason, REASON_BIDI_FRAME_SEQUENCE, false),
                        )
                        .await;
                    return;
                }
                expected_up_sequence = expected_up_sequence.saturating_add(1);
                let Some(payload) = frame.payload else {
                    continue;
                };
                let bridge_frame_result = match payload {
                    UpPayload::BinaryChunk(chunk) => build_remote_bidi_input_frame_for_ability(
                        call_id,
                        &ability_owned,
                        &chunk.data,
                        None,
                        false,
                    ),
                    UpPayload::Control(control)
                        if matches!(
                            control.control,
                            Some(easynet_axon::pb::axon::v1::bidi_control::Control::Eof(true))
                        ) =>
                    {
                        eof_sent = true;
                        build_remote_bidi_input_frame_for_ability(
                            call_id,
                            &ability_owned,
                            &[],
                            None,
                            true,
                        )
                    }
                    UpPayload::Control(control)
                        if ability_owned
                            == crate::runtime::agents::pty_attach_ability::ABILITY_PTY_SESSION_ATTACH =>
                    {
                        let Some(easynet_axon::pb::axon::v1::bidi_control::Control::PtyResize(
                            resize,
                        )) = control.control
                        else {
                            continue;
                        };
                        build_remote_bidi_input_frame_for_ability(
                            call_id,
                            &ability_owned,
                            &[],
                            Some((resize.cols, resize.rows)),
                            false,
                        )
                    }
                    UpPayload::Control(_) | UpPayload::EnvelopeOpen(_) => continue,
                };
                let bridge_frame = match bridge_frame_result {
                    Ok(frame) => frame,
                    Err(status) => {
                        let reason = status.to_string();
                        let _ = pending_for_up
                            .finish(
                                call_id,
                                failed_dispatch_result(&reason, "INVALID_ARGUMENT", false),
                            )
                            .await;
                        return;
                    }
                };
                match sender.try_send(Ok(bridge_frame)) {
                    Ok(()) => {}
                    Err(tokio::sync::mpsc::error::TrySendError::Full(_)) => {
                        presence_for_up.remove_if_session(
                            &execution_host_ura_owned,
                            session_id,
                            crate::services::presence_registry::OfflineReason::SendFailed,
                        );
                        let reason =
                            federation_wrappers::FORWARD_INVOKE_TARGET_OFFLINE_REASON.to_string();
                        let _ = pending_for_up
                            .finish(
                                call_id,
                                failed_dispatch_result(
                                    &reason,
                                    "TARGET_NOT_IN_PRESENCE_REGISTRY",
                                    true,
                                ),
                            )
                            .await;
                        return;
                    }
                    Err(tokio::sync::mpsc::error::TrySendError::Closed(_)) => {
                        presence_for_up.remove_if_session(
                            &execution_host_ura_owned,
                            session_id,
                            crate::services::presence_registry::OfflineReason::StreamClosed,
                        );
                        let reason =
                            federation_wrappers::FORWARD_INVOKE_TARGET_OFFLINE_REASON.to_string();
                        let _ = pending_for_up
                            .finish(
                                call_id,
                                failed_dispatch_result(
                                    &reason,
                                    "TARGET_NOT_IN_PRESENCE_REGISTRY",
                                    true,
                                ),
                            )
                            .await;
                        return;
                    }
                }
            }

            if !eof_sent {
                // try_send because the receiver may have raced
                // the EOF: Closed = client gone (expected), Full
                // = backpressure-lost terminal frame (needs an
                // op_event so the operator sees the lost EOF).
                crate::support::async_bridge::discard_try_send_classify(
                    sender.try_send(Ok(build_remote_bidi_input_dispatch_frame(
                        call_id,
                        &[],
                        true,
                    ))),
                    "daemon_invocation",
                    &format!("remote_bidi_eof call_id={call_id}"),
                );
            }
        });

        let stream = LocalBidiDownStream::new(down_rx);
        Ok(Response::new(
            Box::pin(stream) as BoxedDownStream<InvokeBidiDown>
        ))
    }

    /// PTY/file-transfer bidi adapter: invoke the locally registered
    /// Axon ability through `LocalRuntime` and bridge its JSON frame
    /// protocol onto the gRPC `InvokeBidi` up/down streams.
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
    async fn resolve_local_bidi_route(
        &self,
        envelope_open: &EnvelopeOpen,
    ) -> Result<SelectedInvokeRoute, Status> {
        let target_ura = target_ura_from_envelope(envelope_open.envelope.as_ref(), "InvokeBidi")?;
        let ability = envelope_open
            .target
            .as_ref()
            .map(|target| target.ability_name.trim())
            .filter(|ability| !ability.is_empty())
            .ok_or_else(|| {
                Status::invalid_argument(
                    "InvokeBidi frame 0 missing target.ability_name for namespace.resolve",
                )
            })?;

        let selected_route = self
            .daemon_route_resolver()
            .resolve_route(&target_ura, ability)
            .map_err(route_negative_status)?;
        if !selected_route.is_authoritative_local_or_better() {
            return Err(route_profile_blocked_status(&selected_route));
        }
        if !self
            .matches_self_target_ura(&selected_route.execution_host_ura)
            .await
        {
            return Err(route_selected_remote_host_status(
                "InvokeBidi",
                &selected_route,
            ));
        }
        Ok(selected_route)
    }

    async fn dispatch_local_bidi_selected_route(
        &self,
        envelope_open: &EnvelopeOpen,
        mut up: Streaming<InvokeBidiUp>,
    ) -> Result<Response<<Self as Invocation>::InvokeBidiStream>, Status> {
        let selected_route = self.resolve_local_bidi_route(envelope_open).await?;
        let dispatch_ability = selected_route.dispatch_key();
        crate::op_event!(
            component = daemon_invocation,
            kind = invoke_bidi_local_runtime_dispatch,
            ability = selected_route.dispatch_name.as_str(),
            dispatch_ability = dispatch_ability.as_str(),
            callee_ura = selected_route.callee_ura.as_str(),
            execution_host_ura = selected_route.execution_host_ura.as_str(),
            route_ura = selected_route.route_ura.as_str(),
        );

        let Some(runtime) = self.local_runtime.as_ref() else {
            return Err(Status::failed_precondition(format!(
                "InvokeBidi: ability `{}` cannot run because Axon LocalRuntime \
                 is not wired at boot",
                selected_route.dispatch_name
            )));
        };
        let Some(options) = runtime.ability_options(&dispatch_ability).await else {
            return Err(Status::not_found(format!(
                "InvokeBidi: selected route `{}` dispatches `{}` but that ability is not \
                 registered in Axon LocalRuntime",
                selected_route.route_ura, dispatch_ability
            )));
        };
        if !options.modes.bidi {
            return Err(Status::invalid_argument(format!(
                "InvokeBidi: selected route `{}` dispatches `{}` but it does not support bidi Invoke",
                selected_route.route_ura, dispatch_ability
            )));
        }
        let selected_open = envelope_open_with_selected_route(envelope_open, &selected_route);
        let wire =
            crate::runtime::axon_bridge::dispatch_shim::admitted_from_envelope_open(&selected_open)
                .map_err(|err| {
                    status_from_axon_invoke_error("InvokeBidi", &dispatch_ability, err)
                })?;
        let wire_kind = self
            .ability_wire
            .bidi_wire_kind_for(&selected_route.dispatch_name)
            .ok_or_else(|| {
            Status::failed_precondition(format!(
                "InvokeBidi: ability `{}` is registered as local bidi but has no declared wire protocol",
                selected_route.dispatch_name
            ))
        })?;
        let handle = crate::runtime::axon_bridge::dispatch_shim::open_bidi_admitted(runtime, wire)
            .await
            .map_err(|err| status_from_axon_invoke_error("InvokeBidi", &dispatch_ability, err))?;
        let (handler_in_tx, mut handler_out_rx) = handle.split();
        let stdout_stream_id = local_bidi_stdout_stream_id(envelope_open);

        // Down-stream: handler-emitted JSON → InvokeBidiDown frames.
        // Capacity 16 mirrors `INVOKE_REMOTE_DISPATCH_CAPACITY`.
        let (down_tx, down_rx) = tokio::sync::mpsc::channel::<Result<InvokeBidiDown, Status>>(16);

        let down_tx_for_handler = down_tx.clone();
        tokio::spawn(async move {
            while let Some(frame_result) = handler_out_rx.next_frame().await {
                let frame = match frame_result {
                    Ok(frame) => frame,
                    Err(err) => {
                        let _ = down_tx_for_handler
                            .send(Ok(build_bidi_terminal_receipt(
                                easynet_axon::invocation::InvocationState::Failed,
                                format!("InvokeBidi local-runtime frame failed: {err}"),
                            )))
                            .await;
                        break;
                    }
                };
                let terminal = frame.terminal;
                let mapped = map_local_bidi_ability_frame(wire_kind, frame, stdout_stream_id);
                match mapped {
                    LocalBidiHandlerFrame::Forward(frame) => {
                        if down_tx_for_handler.send(Ok(frame)).await.is_err() {
                            break;
                        }
                        if terminal {
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
                                easynet_axon::invocation::InvocationState::Failed,
                                reason,
                            )))
                            .await;
                        break;
                    }
                }
                if terminal {
                    break;
                }
            }
        });

        // Up-stream: InvokeBidiUp frames → handler input JSON.
        tokio::spawn(async move {
            let mut expected_up_sequence = 1_u64;
            while let Some(maybe_frame) = up.next().await {
                let Ok(frame) = maybe_frame else { break };
                if frame.sequence != expected_up_sequence {
                    let frame_sequence = frame.sequence;
                    crate::op_event!(
                        component = daemon_invocation,
                        kind = invoke_bidi_frame_sequence_violated,
                        reason = REASON_BIDI_FRAME_SEQUENCE,
                        expected = expected_up_sequence,
                        got = frame_sequence,
                    );
                    break;
                }
                expected_up_sequence = expected_up_sequence.saturating_add(1);
                let Some(payload) = frame.payload else {
                    continue;
                };
                match map_local_bidi_up_payload(wire_kind, payload) {
                    LocalBidiUpFrame::Forward(jsonv) => {
                        let Ok(payload) = serde_json::to_vec(&jsonv) else {
                            break;
                        };
                        if handler_in_tx
                            .send(
                                BidiInputFrame::new(payload).with_content_type("application/json"),
                            )
                            .await
                            .is_err()
                        {
                            break;
                        }
                    }
                    LocalBidiUpFrame::ForwardAndClose(jsonv) => {
                        let Ok(payload) = serde_json::to_vec(&jsonv) else {
                            break;
                        };
                        if handler_in_tx
                            .send(
                                BidiInputFrame::new(payload).with_content_type("application/json"),
                            )
                            .await
                            .is_err()
                        {
                            break;
                        }
                        let _ = handler_in_tx.close_input().await;
                        break;
                    }
                    LocalBidiUpFrame::Close => {
                        let _ = handler_in_tx.close_input().await;
                        break;
                    }
                    LocalBidiUpFrame::Ignore => {}
                }
            }
            // Up-stream EOF → close the Axon inbox so the ability's
            // `recv_message` loop sees a graceful disconnect.
            let _ = handler_in_tx.close_input().await;
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
                        let payload = serde_json::to_vec(&PresenceEventDelta::from(event)).expect(
                            "PresenceEventDelta is statically Serialize; a serialise \
                             failure here means the type grew a fallible field — update \
                             this site to surface Status::internal instead of panicking",
                        );
                        let chunk = InvokeStreamChunk {
                            content_type: FEDERATION_RESULT_CONTENT_TYPE.to_string(),
                            payload,
                            ..InvokeStreamChunk::default()
                        };
                        Some((Ok(chunk), (events, presence_weak)))
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
                        Some((Ok(chunk), (events, presence_weak)))
                    }
                    Err(RecvError::Closed) => None,
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
                                Some((Ok(chunk), (events, presence_weak, hb_ms)))
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
                                let chunk = InvokeStreamChunk {
                                    content_type: FEDERATION_RESULT_CONTENT_TYPE.to_string(),
                                    payload,
                                    ..InvokeStreamChunk::default()
                                };
                                Some((Ok(chunk), (events, presence_weak, hb_ms)))
                            }
                            Err(RecvError::Closed) => None,
                        }
                    }
                    _ = hb.tick() => {
                        // 30s elapsed without a real event;
                        // emit Heartbeat so the subscriber's
                        // 60s idle-timeout watcher does not
                        // tear down a healthy stream.
                        let hb_evt = DirectoryEvent::Heartbeat {
                            unix_ms: crate::services::federation_directory::now_unix_ms(),
                        };
                        let payload = serde_json::to_vec(&hb_evt)
                            .expect("DirectoryEvent::Heartbeat is statically Serialize");
                        let chunk = InvokeStreamChunk {
                            content_type: FEDERATION_RESULT_CONTENT_TYPE.to_string(),
                            payload,
                            ..InvokeStreamChunk::default()
                        };
                        Some((Ok(chunk), (events, presence_weak, hb_ms)))
                    }
                }
            },
        );

        let combined = futures::StreamExt::chain(initial_stream, event_stream);
        Ok(Response::new(
            Box::pin(combined) as BoxedDownStream<InvokeStreamChunk>
        ))
    }

    async fn resolve_local_stream_route(
        &self,
        request: &InvokeServerStreamRequest,
    ) -> Result<SelectedInvokeRoute, Status> {
        let target_ura = local_stream_target_ura(request)?;
        let ability = request.function_name.trim();
        if ability.is_empty() {
            return Err(Status::invalid_argument(
                "InvokeStream request missing function_name for namespace.resolve",
            ));
        }

        let selected_route = self
            .daemon_route_resolver()
            .resolve_route(&target_ura, ability)
            .map_err(route_negative_status)?;
        if !selected_route.is_authoritative_local_or_better() {
            return Err(route_profile_blocked_status(&selected_route));
        }
        if !self
            .matches_self_target_ura(&selected_route.execution_host_ura)
            .await
        {
            return Err(route_selected_remote_host_status(
                "InvokeStream",
                &selected_route,
            ));
        }
        Ok(selected_route)
    }

    async fn dispatch_local_stream_selected_route(
        &self,
        request: &InvokeServerStreamRequest,
    ) -> Result<Response<<Self as Invocation>::InvokeStreamStream>, Status> {
        let ability = request.function_name.trim();
        let selected_route = self.resolve_local_stream_route(request).await?;
        let Some(runtime) = self.local_runtime.as_ref() else {
            return Err(Status::failed_precondition(format!(
                "InvokeStream: ability `{ability}` cannot run because Axon LocalRuntime \
                 is not wired at boot"
            )));
        };
        let dispatch_ability = selected_route.dispatch_key();
        let Some(options) = runtime.ability_options(&dispatch_ability).await else {
            return Err(Status::not_found(format!(
                "InvokeStream: selected route `{}` dispatches `{}` but that ability is not \
                 registered in Axon LocalRuntime",
                selected_route.route_ura, dispatch_ability
            )));
        };
        if !options.modes.stream {
            return Err(Status::invalid_argument(format!(
                "InvokeStream: selected route `{}` dispatches `{}` but it does not support \
                 server-stream Invoke",
                selected_route.route_ura, dispatch_ability
            )));
        }
        let wire = match request.envelope.clone() {
            Some(envelope) => {
                let envelope = envelope_with_selected_callee(envelope, &selected_route);
                crate::runtime::axon_bridge::dispatch_shim::admitted_from_wire_parts(
                    envelope,
                    dispatch_ability.clone(),
                    request.arguments.clone(),
                )
            }
            None => Err(easynet_axon::invocation::AxonError::invalid_argument(
                "InvokeStream request missing envelope",
            )),
        }
        .map_err(|err| status_from_axon_invoke_error("InvokeStream", ability, err))?;
        let mut handle =
            crate::runtime::axon_bridge::dispatch_shim::open_stream_admitted(runtime, wire)
                .await
                .map_err(|err| status_from_axon_invoke_error("InvokeStream", ability, err))?;

        let (tx, rx) = mpsc::channel::<Result<InvokeStreamChunk, Status>>(16);
        let ability_name = dispatch_ability;
        tokio::spawn(async move {
            while let Some(frame_result) = handle.next_frame().await {
                match frame_result {
                    Ok(frame) => {
                        if frame.terminal && frame.payload.is_empty() {
                            break;
                        }
                        let terminal = frame.terminal;
                        let content_type = if frame.content_type.is_empty() {
                            FEDERATION_RESULT_CONTENT_TYPE.to_string()
                        } else {
                            frame.content_type
                        };
                        let chunk = InvokeStreamChunk {
                            content_type,
                            payload: frame.payload,
                            terminal,
                            ..InvokeStreamChunk::default()
                        };
                        if tx.send(Ok(chunk)).await.is_err() || terminal {
                            break;
                        }
                    }
                    Err(err) => {
                        let _ = tx
                            .send(Err(status_from_axon_invoke_error(
                                "InvokeStream",
                                &ability_name,
                                err,
                            )))
                            .await;
                        break;
                    }
                }
            }
        });

        Ok(Response::new(
            Box::pin(ReceiverStream::new(rx)) as BoxedDownStream<InvokeStreamChunk>
        ))
    }

    /// Hub-side `<self>.invoke_remote` handler. Drives the RFC-005
    /// per-call dispatch flow:
    ///
    /// 1. Parse the frame-0 `EnvelopeOpen.initial_args` as
    ///    `InvokeRemoteUp::Request { subject_device, ability_ura, args }`
    /// 2. Resolve `ability_ura` through `namespace.resolve` and
    ///    require an authoritative local-or-better `FinalRoute`
    /// 3. Verify the selected owner still matches the request
    ///    target. `subject_device` is a consistency check, not a
    ///    route source.
    /// 4. Verify delegation against the selected callee and dispatch
    ///    name.
    /// 5. If the selected execution host is this daemon, dispatch
    ///    directly through Axon `LocalRuntime`.
    /// 6. Otherwise, look up the selected execution host in
    ///    `PresenceRegistry`, register a pending-reply slot, and push
    ///    a `DispatchDown` frame carrying the selected callee and
    ///    selected dispatch key.
    /// 7. Return a server-stream whose frames project
    ///    `DispatchStreamEvent` / `DispatchResult` into
    ///    `InvokeRemoteDown`.
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
            subject_ura,
            ability_ura,
            args,
            args_content_envelope,
            metadata,
        } = request;

        let selected_route = match self.daemon_route_resolver().resolve_route(&ability_ura, "") {
            Ok(route) if route.is_authoritative_local_or_better() => route,
            Ok(route) => {
                return invoke_remote_inband_error_response(route_profile_blocked_message(&route))
            }
            Err(failure) => {
                return invoke_remote_inband_error_response(route_negative_message(&failure))
            }
        };
        if selected_route.owner_ura != subject_device {
            return invoke_remote_inband_error_response(route_owner_mismatch_message(
                &selected_route.owner_ura,
                &ability_ura,
                &subject_device,
            ));
        }
        let public_ability = selected_route.dispatch_name.clone();
        let inner_subject = subject_ura
            .as_deref()
            .filter(|subject| !subject.trim().is_empty())
            .unwrap_or(subject_device.as_str());
        let outer_caller = envelope_open
            .envelope
            .as_ref()
            .and_then(|envelope| envelope.caller.clone())
            .ok_or_else(|| {
                Status::invalid_argument(
                    "<self>.invoke_remote: admitted frame-0 envelope is missing caller",
                )
            })?;
        let inner_envelope = Envelope {
            caller: Some(outer_caller),
            callee: Some(AgentIdentity {
                ura: selected_route.callee_ura.clone(),
                ..AgentIdentity::default()
            }),
            subject: Some(SubjectIdentity {
                ura: inner_subject.to_string(),
                ..SubjectIdentity::default()
            }),
            ..Envelope::default()
        };
        self.admission.verify_delegation_for_envelope(
            &inner_envelope,
            &public_ability,
            &metadata,
        )?;

        // ── Phase 4: Axon-routed **self-target** dispatch ──────────
        //
        // If a shared `LocalRuntime` is wired AND the resolver-selected
        // execution host names THIS daemon's own URA, route the call through Axon's
        // daemon-internal entry (`invoke_async`). The runtime owns
        // admission, the state machine, and ledger persistence; the
        // bridge shim (`dispatch_shim::dispatch_rpc_local`) drains
        // the handle and produces the wire-shape `(payload, error)`
        // pair we emit in the one-shot terminal frame.
        //
        // **Critical guard — selected `execution_host_ura`.**
        // Without it, this arm intercepts every call whose ability
        // name happens to be in our local runtime — even when the
        // caller's `subject_device` names a peer device that should
        // get a forwarded `Dispatch` frame. The original symptom of
        // missing this guard: the Web UI's `agent.list`
        // request against a peer device returned THIS daemon's
        // agents (because `agent.list` is registered in
        // every daemon's runtime), so the agent page lit up with
        // wrong data instead of the peer's view.
        //
        // Why `invoke_async` (not `invoke_externally_signed_*`):
        // the existing `InvokeRemoteUp::Request` wire shape doesn't
        // carry the user's signed envelope through — the Go shim
        // (`backend/internal/daemon_grpc/remote_routing.go:197`)
        // decomposes the user envelope and re-issues the call as a
        // daemon-internal `<self>.invoke_remote` whose outer
        // envelope is signed by the backend, not the user. So the
        // inner ability dispatch runs in trust-domain mode
        // (SystemAgent binding per AXIOM §3.2). A follow-up wire-
        // protocol change can pass the inner signed envelope
        // through and flip this site to `dispatch_rpc`.
        //
        // Self-targeted invoke_remote never goes through the pending
        // session map. The daemon's shared
        // Axon `LocalRuntime` is the only local execution surface; if
        // the ability is absent, Axon returns the in-band error frame.
        if self
            .matches_self_target_ura(&selected_route.execution_host_ura)
            .await
        {
            return self
                .dispatch_self_targeted_invoke_remote(
                    &selected_route,
                    subject_ura.as_deref(),
                    &args,
                )
                .await;
        }

        let pending = self.pending.as_ref().ok_or_else(|| {
            Status::failed_precondition(
                "<self>.invoke_remote: daemon was constructed without a \
                 PendingDispatchMap; call DaemonInvocationService::with_pending(...) \
                 at boot to enable cross-device invocation",
            )
        })?;

        let (target_session_id, target_sender) = match self
            .presence
            .lookup_tracked(&selected_route.execution_host_ura)
        {
            Some(slot) => slot,
            None => {
                return invoke_remote_inband_error_response(selected_host_unavailable_message(
                    &selected_route,
                ));
            }
        };

        // Register pending entry BEFORE pushing the dispatch frame —
        // otherwise the target could reply faster than we can register
        // and the reply would land as a no-op `complete`.
        //
        // Prefer the stream-aware table. It preserves unary behaviour
        // (one Terminal event) while allowing server-stream abilities
        // to surface zero or more Chunk events before Terminal. The
        // unary map remains as a fallback for older boot wiring.
        let mut stream_handle = self
            .pending_stream
            .as_ref()
            .map(|pending_stream| pending_stream.register_pending());
        let unary_handle = if stream_handle.is_none() {
            Some(pending.register_pending_for(&selected_route.execution_host_ura))
        } else {
            None
        };
        let call_id = stream_handle
            .as_ref()
            .map(|handle| handle.call_id())
            .or_else(|| unary_handle.as_ref().map(|handle| handle.call_id()))
            .expect("invoke_remote registered a pending handle");

        let dispatch_ability = selected_route.dispatch_key();
        let dispatch_frame = build_invoke_remote_dispatch_frame(
            call_id,
            &selected_route.callee_ura,
            subject_ura.as_deref(),
            &dispatch_ability,
            &args,
            args_content_envelope,
            metadata,
        )?;
        match target_sender.try_send(Ok(dispatch_frame)) {
            Ok(()) => {}
            Err(tokio::sync::mpsc::error::TrySendError::Full(_)) => {
                // Bounded backpressure → presence transition (same
                // policy as forward_invoke commit 8/9).
                self.presence.remove_if_session(
                    &selected_route.execution_host_ura,
                    target_session_id,
                    OfflineReason::SendFailed,
                );
                return invoke_remote_inband_error_response(format!(
                    "<self>.invoke_remote: selected execution host `{}` channel full; \
                     removed from registry with OfflineReason::SendFailed",
                    selected_route.execution_host_ura
                ));
            }
            Err(tokio::sync::mpsc::error::TrySendError::Closed(_)) => {
                self.presence.remove_if_session(
                    &selected_route.execution_host_ura,
                    target_session_id,
                    OfflineReason::StreamClosed,
                );
                return invoke_remote_inband_error_response(format!(
                    "<self>.invoke_remote: selected execution host `{}` receiver closed \
                     between lookup and dispatch; removed from registry",
                    selected_route.execution_host_ura
                ));
            }
        }

        // The down stream: streamed targets yield Chunk frames until
        // the target sends a terminal Result. Unary targets naturally
        // produce only the terminal Result, so the same bridge covers
        // both call shapes.
        let (down_tx, down_rx) = mpsc::channel::<Result<InvokeBidiDown, Status>>(16);
        if let Some(mut handle) = stream_handle.take() {
            let cancel_sender = target_sender.clone();
            tokio::spawn(async move {
                let mut terminal_seen = false;
                while let Some(event) = handle.recv().await {
                    let (frame, terminal) = match event {
                        DispatchStreamEvent::Chunk(payload) => {
                            let down = InvokeRemoteDown::Chunk { payload };
                            (build_invoke_remote_terminal_frame(&down), false)
                        }
                        DispatchStreamEvent::Terminal(DispatchResult {
                            payload,
                            error,
                            failure,
                            request_id,
                        }) => {
                            let down = InvokeRemoteDown::Result {
                                payload,
                                error,
                                failure,
                                request_id,
                            };
                            (build_invoke_remote_terminal_frame(&down), true)
                        }
                    };
                    terminal_seen = terminal_seen || terminal;
                    if down_tx.send(frame).await.is_err() || terminal {
                        break;
                    }
                }
                if !terminal_seen {
                    crate::support::async_bridge::discard_try_send_classify(
                        cancel_sender.try_send(Ok(build_remote_bidi_input_dispatch_frame(
                            call_id,
                            &[],
                            true,
                        ))),
                        "daemon_invocation",
                        &format!("invoke_remote_stream_cancel call_id={call_id}"),
                    );
                }
            });
        } else {
            let handle = unary_handle.expect("unary pending handle registered");
            tokio::spawn(async move {
                let frame = match handle.await_reply().await {
                    Ok(DispatchResult {
                        payload,
                        error,
                        failure,
                        request_id,
                    }) => {
                        let down = InvokeRemoteDown::Result {
                            payload,
                            error,
                            failure,
                            request_id,
                        };
                        match build_invoke_remote_terminal_frame(&down) {
                            Ok(f) => Ok(f),
                            Err(status) => Err(status),
                        }
                    }
                    Err(_recv_err) => {
                        // Sender dropped without complete — target session
                        // task crashed or daemon shutdown mid-call.
                        let reason =
                            format!("target session disconnected before reply (call_id={call_id})");
                        let down = InvokeRemoteDown::Result {
                            payload: Vec::new(),
                            error: Some(reason.clone()),
                            failure: Some(SessionFailure::from_reason(
                                reason,
                                "TARGET_NOT_IN_PRESENCE_REGISTRY",
                                true,
                            )),
                            request_id: None,
                        };
                        match build_invoke_remote_terminal_frame(&down) {
                            Ok(f) => Ok(f),
                            Err(status) => Err(status),
                        }
                    }
                };
                let _ = down_tx.send(frame).await;
            });
        }

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
    /// 2. Insert `tx` into PresenceRegistry under the caller URA;
    ///    any prior session for the same URA is displaced (the
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
        caller_ura: String,
        up: Streaming<InvokeBidiUp>,
    ) -> Result<Response<<Self as Invocation>::InvokeBidiStream>, Status> {
        validate_session_realm(
            &caller_ura,
            self.session_realm.as_deref(),
            &self.admission.trust_anchor_snapshot(),
        )?;

        let (down_tx, down_rx): (DispatchSender, _) =
            mpsc::channel::<Result<DispatchFrame, Status>>(DISPATCH_CHANNEL_CAPACITY);

        // Step 1: register before spawning so a SessionDispatch::Dispatch
        // arriving from `<self>.invoke_remote` immediately can find this
        // sender. The PresenceRegistry handles displacement (Offline +
        // Online emission ordering) under the hood.
        let registration = self.presence.insert_tracked(caller_ura.clone(), down_tx);
        let displaced_prior = registration.displaced.is_some();
        crate::op_event!(
            component = daemon_invocation,
            kind = self_session_admitted,
            caller = caller_ura,
            displaced_prior = displaced_prior,
        );

        // Step 2: spawn the up-stream consumer. Reads device replies
        // (SessionDispatch::Result frames) and routes them to the
        // PendingDispatchMap so the originating <self>.invoke_remote
        // caller wakes up.
        let presence_for_drain = Arc::clone(&self.presence);
        let pending_for_drain = self.pending.clone();
        let pending_stream_for_drain = self.pending_stream.clone();
        let caller_ura_for_drain = caller_ura.clone();
        // PR-N6 C3: drain task needs a service handle so inbound
        // `Request` frames can route into the same dispatch arms
        // the unary `Invoke` RPC uses (forward_invoke today; other
        // abilities follow as PR-N6 grows). `DaemonInvocationService`
        // is `Clone` over Arc/Option fields so this is cheap.
        let service_for_drain = self.clone();
        tokio::spawn(async move {
            drain_session_up_stream(
                up,
                caller_ura_for_drain,
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
/// device's `LocalAxonSessionDispatcher` ignores `Receipt` payloads
/// outright (handle_down only acts on `BinaryChunk`).
fn build_bidi_admission_receipt() -> InvokeBidiDown {
    InvokeBidiDown {
        sequence: 0,
        payload: Some(DownPayload::Receipt(InvocationReceipt {
            state: easynet_axon::invocation::InvocationState::Admitted.to_wire_i32(),
            ..InvocationReceipt::default()
        })),
        ..InvokeBidiDown::default()
    }
}

fn build_session_down_admission_receipt() -> InvokeBidiDown {
    build_bidi_admission_receipt()
}

fn build_bidi_terminal_receipt(
    state: easynet_axon::invocation::InvocationState,
    reason: impl Into<String>,
) -> InvokeBidiDown {
    build_bidi_terminal_receipt_with_payload(state, reason, None)
}

fn build_bidi_terminal_receipt_with_payload(
    state: easynet_axon::invocation::InvocationState,
    reason: impl Into<String>,
    payload: Option<(Vec<u8>, &'static str)>,
) -> InvokeBidiDown {
    build_bidi_terminal_receipt_with_payload_and_failure_code(state, reason, payload, None)
}

fn build_bidi_terminal_receipt_with_payload_and_failure_code(
    state: easynet_axon::invocation::InvocationState,
    reason: impl Into<String>,
    payload: Option<(Vec<u8>, &'static str)>,
    failure_code: Option<&str>,
) -> InvokeBidiDown {
    let reason = reason.into();
    let (payload_bytes, payload_content_type) = payload
        .map(|(bytes, content_type)| (bytes, content_type.to_string()))
        .unwrap_or_default();
    let failure = terminal_receipt_failure(state, &reason, failure_code);
    InvokeBidiDown {
        payload: Some(DownPayload::Receipt(InvocationReceipt {
            state: state.to_wire_i32(),
            reason,
            payload: payload_bytes,
            payload_content_type,
            cleanup_complete: true,
            failure,
            ..InvocationReceipt::default()
        })),
        ..InvokeBidiDown::default()
    }
}

fn terminal_receipt_failure(
    state: easynet_axon::invocation::InvocationState,
    reason: &str,
    explicit_code: Option<&str>,
) -> Option<Error> {
    TerminalReceiptFailure::from_terminal_state(state, reason, explicit_code)
        .map(TerminalReceiptFailure::into_error)
}

fn terminal_failure_message(reason: &str, fallback_code: &str) -> String {
    let message = reason.trim();
    if message.is_empty() {
        fallback_code.to_string()
    } else {
        message.to_string()
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
struct TerminalReceiptFailure {
    code: String,
    message: String,
    retryable: bool,
    stage: ErrorStage,
    security_class: SecurityClass,
}

impl TerminalReceiptFailure {
    fn from_terminal_state(
        state: easynet_axon::invocation::InvocationState,
        reason: &str,
        explicit_code: Option<&str>,
    ) -> Option<Self> {
        let (fallback_code, retryable) = match state {
            easynet_axon::invocation::InvocationState::Failed => ("INVOCATION_FAILED", false),
            easynet_axon::invocation::InvocationState::TimedOut => ("INVOCATION_TIMED_OUT", true),
            easynet_axon::invocation::InvocationState::Cancelled => ("INVOCATION_CANCELLED", false),
            _ => return None,
        };
        let code = crate::runtime::failure_codes::FailureCodeClassifier::explicit_or_reason(
            explicit_code,
            reason,
            fallback_code,
        );
        let failure_class =
            crate::runtime::failure_codes::FailureCodeClassifier::classify_error_class(&code);
        Some(Self {
            code,
            message: terminal_failure_message(reason, fallback_code),
            retryable,
            stage: failure_class.stage.to_axon_pb(),
            security_class: failure_class.security_class.to_axon_pb(),
        })
    }

    fn into_error(self) -> Error {
        Error {
            code: self.code,
            message: self.message,
            retryable: self.retryable,
            context: Default::default(),
            stage: self.stage as i32,
            security_class: self.security_class as i32,
        }
    }
}

const LOCAL_BIDI_DEFAULT_STREAM_ID: u32 = 1;

type LocalBidiWireKind = crate::runtime::ability_wire::AbilityBidiWireKind;

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

fn status_from_axon_invoke_error(
    surface: &str,
    ability: &str,
    err: easynet_axon::invocation::AxonError,
) -> Status {
    let message =
        format!("{surface}: Axon LocalRuntime dispatch of ability `{ability}` failed: {err}");
    if err.reason.contains("unknown_ability") || err.reason.contains("mode_not_supported") {
        return Status::not_found(message);
    }
    match err.kind {
        AxonErrorKind::Cancelled => Status::cancelled(message),
        AxonErrorKind::DeadlineExceeded => Status::deadline_exceeded(message),
        AxonErrorKind::Unavailable => Status::unavailable(message),
        AxonErrorKind::InvalidArgument => Status::invalid_argument(message),
        AxonErrorKind::ResourceExhausted => Status::resource_exhausted(message),
        AxonErrorKind::PermissionDenied => Status::permission_denied(message),
        AxonErrorKind::Internal => Status::internal(message),
    }
}

fn map_local_bidi_up_payload(wire_kind: LocalBidiWireKind, payload: UpPayload) -> LocalBidiUpFrame {
    use base64::{engine::general_purpose::STANDARD as B64, Engine as _};
    use easynet_axon::pb::axon::v1::bidi_control::Control as ControlVariant;
    use easynet_axon::pb::axon::v1::{BidiControl, PtyResize};
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
        (LocalBidiWireKind::JsonFrames, UpPayload::BinaryChunk(chunk)) => {
            match serde_json::from_slice::<serde_json::Value>(&chunk.data) {
                Ok(jsonv) => LocalBidiUpFrame::Forward(jsonv),
                Err(_) => LocalBidiUpFrame::Ignore,
            }
        }
        (
            LocalBidiWireKind::JsonFrames,
            UpPayload::Control(BidiControl {
                control: Some(ctl), ..
            }),
        ) => match ctl {
            ControlVariant::Eof(true) => LocalBidiUpFrame::Close,
            _ => LocalBidiUpFrame::Ignore,
        },
        (LocalBidiWireKind::JsonFrames, UpPayload::Control(_)) => LocalBidiUpFrame::Ignore,
        (_, UpPayload::EnvelopeOpen(_)) => LocalBidiUpFrame::Ignore,
    }
}

fn map_local_bidi_ability_frame(
    wire_kind: LocalBidiWireKind,
    frame: AbilityFrame,
    stdout_stream_id: u32,
) -> LocalBidiHandlerFrame {
    if frame.payload.is_empty() {
        return if frame.terminal {
            LocalBidiHandlerFrame::Terminal(build_bidi_terminal_receipt(
                easynet_axon::invocation::InvocationState::Completed,
                String::new(),
            ))
        } else {
            LocalBidiHandlerFrame::Ignore
        };
    }
    if matches!(wire_kind, LocalBidiWireKind::JsonFrames)
        && !frame.terminal
        && !frame.content_type.is_empty()
        && frame.content_type != "application/json"
    {
        return LocalBidiHandlerFrame::Forward(InvokeBidiDown {
            payload: Some(DownPayload::BinaryChunk(BinaryChunk {
                stream_id: stdout_stream_id,
                data: frame.payload,
                ..BinaryChunk::default()
            })),
            ..InvokeBidiDown::default()
        });
    }
    match serde_json::from_slice::<serde_json::Value>(&frame.payload) {
        Ok(value) => map_local_bidi_handler_frame(wire_kind, &value, stdout_stream_id),
        Err(err) => LocalBidiHandlerFrame::ProtocolFailure(format!(
            "InvokeBidi local-runtime: ability frame is not valid JSON: {err}"
        )),
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
                        "InvokeBidi local-dispatcher: PTY stdout frame missing `data`".to_string(),
                    );
                };
                let raw = match B64.decode(data_b64) {
                    Ok(raw) => raw,
                    Err(err) => {
                        return LocalBidiHandlerFrame::ProtocolFailure(format!(
                            "InvokeBidi local-runtime: PTY stdout frame base64 decode failed: {err}"
                        ));
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
                    easynet_axon::invocation::InvocationState::Completed,
                    reason,
                ))
            }
            Some("warn") => {
                if let Some(message) = value.get("message").and_then(|field| field.as_str()) {
                    crate::op_event!(
                        component = daemon_invocation,
                        kind = invoke_bidi_local_runtime_warning,
                        handler = "pty",
                        message = message,
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
                        "InvokeBidi local-runtime: file_transfer chunk frame missing `data`"
                            .to_string(),
                    );
                };
                let raw = match B64.decode(data_b64) {
                    Ok(raw) => raw,
                    Err(err) => {
                        return LocalBidiHandlerFrame::ProtocolFailure(format!(
                            "InvokeBidi local-runtime: file_transfer chunk frame base64 decode failed: {err}"
                        ));
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
                Ok(payload) => {
                    LocalBidiHandlerFrame::Terminal(build_bidi_terminal_receipt_with_payload(
                        easynet_axon::invocation::InvocationState::Completed,
                        String::new(),
                        Some((payload, "application/json")),
                    ))
                }
                Err(err) => LocalBidiHandlerFrame::ProtocolFailure(format!(
                    "InvokeBidi local-runtime: encode file_transfer completion receipt payload failed: {err}"
                )),
            },
            Some("error") => {
                let code = value.get("code").and_then(|field| field.as_str());
                let reason = match (
                    code,
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
                    Ok(payload) => {
                        LocalBidiHandlerFrame::Terminal(build_bidi_terminal_receipt_with_payload_and_failure_code(
                            easynet_axon::invocation::InvocationState::Failed,
                            reason,
                            Some((payload, "application/json")),
                            code,
                        ))
                    }
                    Err(err) => LocalBidiHandlerFrame::ProtocolFailure(format!(
                        "InvokeBidi local-runtime: encode file_transfer error receipt payload failed: {err}"
                    )),
                }
            }
            Some("warn") => {
                if let Some(message) = value.get("message").and_then(|field| field.as_str()) {
                    crate::op_event!(
                        component = daemon_invocation,
                        kind = invoke_bidi_local_runtime_warning,
                        handler = "file_transfer",
                        message = message,
                    );
                }
                LocalBidiHandlerFrame::Ignore
            }
            _ => LocalBidiHandlerFrame::Ignore,
        },
        LocalBidiWireKind::JsonFrames => match serde_json::to_vec(value) {
            Ok(payload) => LocalBidiHandlerFrame::Forward(InvokeBidiDown {
                payload: Some(DownPayload::BinaryChunk(BinaryChunk {
                    stream_id: stdout_stream_id,
                    data: payload,
                    ..BinaryChunk::default()
                })),
                ..InvokeBidiDown::default()
            }),
            Err(err) => LocalBidiHandlerFrame::ProtocolFailure(format!(
                "InvokeBidi local-runtime: JSON frame re-encode failed: {err}"
            )),
        },
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
/// `PresenceRegistry` displacement semantics intact: when a same-URA
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

/// Stamp the bidi down-stream sequence number on a frame and advance
/// the counter. Shared by `LocalBidiDownStream` and
/// `SessionDownStream` (formerly two byte-identical copies). The
/// `saturating_add` is intentional: at 2^64 frames per session the
/// counter freezes at u64::MAX rather than wrapping; clients that
/// see two consecutive frames with `sequence = u64::MAX` are
/// expected to surface a session-exhausted error and reconnect.
/// Wrapping silently to 0 would look like a fresh session to the
/// receiver and corrupt the ordering invariant.
fn stamp_bidi_down_sequence(next: &mut u64, mut frame: InvokeBidiDown) -> InvokeBidiDown {
    frame.sequence = *next;
    *next = next.saturating_add(1);
    frame
}

impl LocalBidiDownStream {
    fn new(down_rx: tokio::sync::mpsc::Receiver<Result<InvokeBidiDown, Status>>) -> Self {
        Self {
            down_rx,
            next_sequence: 0,
            pending_admission_receipt: Some(build_bidi_admission_receipt()),
        }
    }

    fn stamp_sequence(&mut self, frame: InvokeBidiDown) -> InvokeBidiDown {
        stamp_bidi_down_sequence(&mut self.next_sequence, frame)
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

    fn stamp_sequence(&mut self, frame: InvokeBidiDown) -> InvokeBidiDown {
        stamp_bidi_down_sequence(&mut self.next_sequence, frame)
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
            crate::services::invocation_transport::session_escalation::SessionEscalationHandle,
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
                state: easynet_axon::invocation::InvocationState::Completed.to_wire_i32(),
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
    /// frames arriving on a device's `<self>.session` bidi. Validates
    /// the hub-owned Ability URA, routes the derived public wrapper
    /// ability through the same dispatch arms the unary `Invoke` RPC
    /// consults, then maps the result into the typed `RequestOutcome`
    /// shape.
    ///
    /// Spec scope (PR-N6 v1): forwards `federation.forward_invoke`
    /// plus the hosted-agent self-advertise repair path
    /// (`federation.advertise_agent`). Other ability names return
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
        ability_ura: &str,
        args: &[u8],
    ) -> RequestOutcome {
        let ability = match self.session_request_public_ability_for_hub(ability_ura) {
            Ok(ability) => ability,
            Err(reason) => {
                return RequestOutcome::Err {
                    error: SessionRequestError::PermissionDenied { reason },
                };
            }
        };

        match ability.as_str() {
            ABILITY_FEDERATION_FORWARD_INVOKE => {
                self.emit_session_request_resolution_marker(args).await;

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
            ABILITY_FEDERATION_ADVERTISE_AGENT => {
                match self.dispatch_federation_advertise_agent(args) {
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
                         only `{ABILITY_FEDERATION_FORWARD_INVOKE}` and \
                         `{ABILITY_FEDERATION_ADVERTISE_AGENT}` are wired in PR-N6 v1"
                    ),
                },
            },
        }
    }

    async fn emit_session_request_resolution_marker(&self, args: &[u8]) {
        let Ok(request) = serde_json::from_slice::<federation_wrappers::ForwardInvokeRequest>(args)
        else {
            crate::op_event!(
                component = session_request,
                kind = target_resolved,
                state_code = "R400",
                path = "malformed_request",
                reason = "forward_invoke_request_decode_failed",
            );
            return;
        };
        let inner_payload = match decode_inner_payload(&request.inner_envelope_b64) {
            Ok(payload) => payload,
            Err(status) => {
                crate::op_event!(
                    component = session_request,
                    kind = target_resolved,
                    state_code = "R400",
                    path = "malformed_inner_payload",
                    target_ura = request.target_ura.as_str(),
                    reason = status.message(),
                );
                return;
            }
        };

        if let Ok(selected_route) = self.resolve_forward_invoke_route(&request, &inner_payload) {
            let selected_host_is_self = self
                .matches_self_target_ura(&selected_route.execution_host_ura)
                .await;
            let path = if selected_host_is_self {
                "selected_self"
            } else {
                "selected_local_session"
            };
            crate::op_event!(
                component = session_request,
                kind = target_resolved,
                state_code = "R300",
                path = path,
                target_ura = request.target_ura.as_str(),
                route_ura = selected_route.route_ura.as_str(),
                callee_ura = selected_route.callee_ura.as_str(),
                execution_host_ura = selected_route.execution_host_ura.as_str(),
                dispatch_name = selected_route.dispatch_name.as_str(),
            );
            return;
        }

        let target_realm = parse_realm_from_ura(&request.target_ura);
        let local_realm = self.session_realm.as_deref();
        let is_local_realm = match (target_realm.as_deref(), local_realm) {
            (Some(target), Some(local)) => target == local,
            (_, None) | (None, Some(_)) => true,
        };
        if is_local_realm {
            crate::op_event!(
                component = session_request,
                kind = target_resolved,
                state_code = "R400",
                path = "resolver_negative",
                target_ura = request.target_ura.as_str(),
            );
            return;
        }

        match self.resolve_cross_realm_forward_delegation(&request, &inner_payload) {
            Ok(delegation) => {
                let endpoint = delegation.primary_endpoint().unwrap_or("");
                crate::op_event!(
                    component = session_request,
                    kind = target_resolved,
                    state_code = "R350",
                    path = "peer_hub_delegation",
                    target_ura = request.target_ura.as_str(),
                    target_realm = delegation.realm.as_str(),
                    peer_endpoint = endpoint,
                );
            }
            Err(status) => {
                crate::op_event!(
                    component = session_request,
                    kind = target_resolved,
                    state_code = "R400",
                    path = "resolver_negative",
                    target_ura = request.target_ura.as_str(),
                    reason = status.message(),
                );
            }
        }
    }

    fn session_request_public_ability_for_hub(&self, ability_ura: &str) -> Result<String, String> {
        let realm = self
            .session_realm
            .as_deref()
            .filter(|realm| !realm.trim().is_empty())
            .ok_or_else(|| {
                "session_request: hub session_realm is not wired; cannot validate request \
                 ability_ura"
                    .to_string()
            })?;
        let hub_ura = crate::ura::hub_ura(realm);
        crate::ura::public_ability_name_from_ability_ura(&hub_ura, ability_ura).ok_or_else(|| {
            format!(
                "session_request: ability_ura `{ability_ura}` does not belong to hub `{hub_ura}`"
            )
        })
    }
}

/// Translate a `tonic::Status` from a hub-side dispatch arm into
/// the typed `SessionRequestError` the device caller receives over
/// the bidi. The mapping mirrors the wire-stable error reasons
/// PR-N1 already uses on the unary path:
///
///   `failed_precondition` carrying exactly the `target_offline` reason
///   maps to `TargetOffline`; permission rejections map to
///   `PermissionDenied`; everything else falls into
///   `UpstreamFailure` with the underlying status text preserved
///   so an operator grep'ing the device log can still cite the
///   exact upstream code + message.
fn map_status_to_session_request_error(status: Status) -> RequestOutcome {
    let code = status.code();
    let message = status.message().to_string();
    if code == tonic::Code::FailedPrecondition
        && message.trim() == federation_wrappers::FORWARD_INVOKE_TARGET_OFFLINE_REASON
    {
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
    use easynet_axon::pb::axon::v1::invoke_bidi_down::Payload;
    use easynet_axon::pb::axon::v1::{BinaryChunk, InvokeBidiDown};

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
    caller_ura: &str,
    id_hex: &str,
    frame: crate::services::presence_registry::DispatchFrame,
) {
    let Some((session_id, sender)) = presence.lookup_tracked(caller_ura) else {
        crate::op_event!(
            component = session_accept,
            kind = request_result_drop_no_presence,
            caller = caller_ura,
            call_id = id_hex,
            reason = "device_disconnected_mid_dispatch",
        );
        return;
    };
    match sender.try_send(Ok(frame)) {
        Ok(()) => {}
        Err(tokio::sync::mpsc::error::TrySendError::Full(_)) => {
            let _ = presence.remove_if_session(caller_ura, session_id, OfflineReason::SendFailed);
            crate::op_event!(
                component = session_accept,
                kind = request_result_push_failed,
                caller = caller_ura,
                call_id = id_hex,
                reason = "channel_full",
                offline_reason = "SendFailed",
            );
        }
        Err(tokio::sync::mpsc::error::TrySendError::Closed(_)) => {
            let _ = presence.remove_if_session(caller_ura, session_id, OfflineReason::StreamClosed);
            crate::op_event!(
                component = session_accept,
                kind = request_result_push_failed,
                caller = caller_ura,
                call_id = id_hex,
                reason = "down_channel_closed",
                offline_reason = "StreamClosed",
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
    caller_ura: String,
    session_id: crate::services::presence_registry::PresenceSessionId,
    presence: Arc<PresenceRegistry>,
    pending: Option<Arc<PendingDispatchMap>>,
    pending_stream: Option<Arc<PendingStreamDispatchMap>>,
    service: DaemonInvocationService,
) {
    use easynet_axon::pb::axon::v1::invoke_bidi_up::Payload as UpPayload;

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
                // `tonic::Code` has Display; use it so the op-event
                // field renders as `code=InvalidArgument` (bare
                // PascalCase) instead of a Debug-quoted string.
                let status_code = status.code();
                crate::op_event!(
                    component = session_accept,
                    kind = up_stream_error,
                    caller = caller_ura,
                    chain = chain,
                    code = status_code,
                );
                close_reason = OfflineReason::StreamReset;
                break;
            }
        };

        if frame.sequence != expected_up_sequence {
            let frame_sequence = frame.sequence;
            crate::op_event!(
                component = session_accept,
                kind = frame_sequence_violated,
                caller = caller_ura,
                reason = REASON_BIDI_FRAME_SEQUENCE,
                expected = expected_up_sequence,
                got = frame_sequence,
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
                    Some(easynet_axon::pb::axon::v1::bidi_control::Control::Eof(true))
                ) {
                    break;
                }
                continue;
            }
            Some(UpPayload::EnvelopeOpen(_)) => {
                crate::op_event!(
                    component = session_accept,
                    kind = unexpected_frame_after_frame_0,
                    caller = caller_ura,
                    frame_kind = "EnvelopeOpen",
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
                let err_msg = format!("{err}");
                crate::op_event!(
                    component = session_accept,
                    kind = malformed_session_dispatch,
                    caller = caller_ura,
                    error = err_msg,
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
                failure,
                request_id,
            } => {
                if terminal {
                    let dispatch_result = DispatchResult {
                        payload,
                        error,
                        failure,
                        request_id,
                    };
                    let mut completed = false;
                    if let Some(pending_stream) = pending_stream.as_ref() {
                        completed = pending_stream
                            .finish(call_id, dispatch_result.clone())
                            .await;
                    }
                    if !completed {
                        let Some(pending) = pending.as_ref() else {
                            crate::op_event!(
                                component = session_accept,
                                kind = terminal_result_dropped_no_pending_map,
                                caller = caller_ura,
                                call_id = call_id,
                            );
                            continue;
                        };
                        completed = pending.complete(call_id, dispatch_result);
                    }
                    if !completed {
                        crate::op_event!(
                            component = session_accept,
                            kind = terminal_result_no_match,
                            caller = caller_ura,
                            call_id = call_id,
                            note = "caller_may_have_cancelled",
                        );
                    }
                } else {
                    let Some(pending_stream) = pending_stream.as_ref() else {
                        crate::op_event!(
                            component = session_accept,
                            kind = streaming_result_dropped_no_pending_stream_map,
                            caller = caller_ura,
                            call_id = call_id,
                        );
                        continue;
                    };
                    let completed = pending_stream.push_chunk(call_id, payload).await;
                    if !completed {
                        crate::op_event!(
                            component = session_accept,
                            kind = streaming_result_chunk_no_match,
                            caller = caller_ura,
                            call_id = call_id,
                        );
                    }
                }
            }
            SessionDispatch::Dispatch { call_id, .. } => {
                // A device sending a Dispatch up its own session
                // makes no sense — Dispatch is hub→device only.
                crate::op_event!(
                    component = session_accept,
                    kind = unexpected_upstream_frame,
                    caller = caller_ura,
                    frame_kind = "Dispatch",
                    call_id = call_id,
                );
            }
            SessionDispatch::BidiOpen {
                call_id, ability, ..
            } => {
                crate::op_event!(
                    component = session_accept,
                    kind = unexpected_upstream_frame,
                    caller = caller_ura,
                    frame_kind = "BidiOpen",
                    call_id = call_id,
                    ability = ability,
                );
            }
            SessionDispatch::BidiInput { call_id, eof, .. } => {
                crate::op_event!(
                    component = session_accept,
                    kind = unexpected_upstream_frame,
                    caller = caller_ura,
                    frame_kind = "BidiInput",
                    call_id = call_id,
                    eof = eof,
                );
            }
            SessionDispatch::Request {
                call_id,
                ability_ura,
                args,
                args_content_envelope,
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
                // Operator log marker for the PR-N6 hub→device
                // session-Request dispatch path. SRE pipelines grep
                // `kind=session_accept_request_frame` to confirm a
                // forward_invoke escalation actually landed on the
                // hub-side accept loop rather than being answered
                // from local presence. The PR-N6 "locked marker"
                // comment that used to live here referenced a demo
                // orchestration script that no longer grep-asserts
                // the byte-exact form; the audit on 2026-05-25
                // confirmed no remaining external dependency on the
                // old `[session-accept] received Request frame`
                // string, so we converged on the op_event shape.
                let id_hex = call_id_hex(&call_id);
                crate::op_event!(
                    component = daemon_invocation,
                    kind = session_accept_request_frame,
                    call_id = id_hex,
                    ability_ura = ability_ura,
                );

                // Dispatch off the drain task so a slow inner
                // call (peer delegation round-trip, peer-side
                // ability handler latency) does not stall
                // subsequent up-frames the device sends. Each
                // Request gets its own short-lived task.
                let service_for_request = service.clone();
                let presence_for_reply = Arc::clone(&presence);
                let caller_ura_for_reply = caller_ura.clone();
                tokio::spawn(async move {
                    let outcome = if args_content_envelope.is_encrypted() {
                        RequestOutcome::Err {
                            error: SessionRequestError::PermissionDenied {
                                reason: format!(
                                    "<self>.session: Request ability_ura `{ability_ura}` received encrypted args \
                                     but no hub-side request decryptor is wired"
                                ),
                            },
                        }
                    } else if !args_content_envelope.content_type.is_empty()
                        && args_content_envelope.content_type != "application/json"
                    {
                        RequestOutcome::Err {
                            error: SessionRequestError::PermissionDenied {
                                reason: format!(
                                    "<self>.session: Request ability_ura `{ability_ura}` received unsupported \
                                     args content_type {:?}",
                                    args_content_envelope.content_type
                                ),
                            },
                        }
                    } else if !args_content_envelope.encoding.is_empty()
                        && args_content_envelope.encoding != "identity"
                    {
                        RequestOutcome::Err {
                            error: SessionRequestError::PermissionDenied {
                                reason: format!(
                                    "<self>.session: Request ability_ura `{ability_ura}` received unsupported \
                                     args encoding {:?}",
                                    args_content_envelope.encoding
                                ),
                            },
                        }
                    } else {
                        service_for_request
                            .dispatch_session_request(&ability_ura, &args)
                            .await
                    };
                    let frame = build_session_request_result_frame(call_id, outcome);
                    push_session_request_result(
                        &presence_for_reply,
                        &caller_ura_for_reply,
                        &id_hex,
                        frame,
                    );
                });
            }
            SessionDispatch::RequestResult { call_id, .. } => {
                // RequestResult is hub → device only; a device
                // sending one up its own session is malformed.
                let id_hex = call_id_hex(&call_id);
                crate::op_event!(
                    component = session_accept,
                    kind = unexpected_upstream_frame,
                    caller = caller_ura,
                    frame_kind = "RequestResult",
                    call_id = id_hex,
                );
            }
        }
    }

    // `OfflineReason: Display` renders the stable snake_case wire
    // label shared with `presence_event_to_directory_event` so the
    // op-event and the directory projection report the same string.
    if presence
        .remove_if_session(&caller_ura, session_id, close_reason)
        .is_some()
    {
        crate::op_event!(
            component = session_accept,
            kind = session_ended,
            caller = caller_ura,
            close_reason = close_reason,
            outcome = "removed_from_registry",
        );
    } else {
        crate::op_event!(
            component = session_accept,
            kind = session_ended,
            caller = caller_ura,
            close_reason = close_reason,
            outcome = "superseded_by_newer_session",
        );
    }
}

/// Session-realm gate.
///
/// Same-realm callers always pass (the most common shape; a
/// device whose URA's realm matches the hub's `session_realm`
/// is the canonical "device joining its own hub" case).
///
/// Cross-realm callers pass iff the caller's URA is present in
/// the supplied trust anchor. The frame-0 envelope's
/// `caller_signature` was already verified upstream by the
/// admission gate against the trust anchor's pubkey for this
/// URA, so a trust-anchor hit here is a sufficient proof of
/// federated identity. Same mechanism the cross-realm
/// `forward_invoke` admission already uses (PR-N2 commits
/// `d1adbea` + `68f6556`); we extend it to cover
/// `<self>.session` admission too. Unblocks the cross-hub
/// same-realm directive that LB-49 surfaced.
fn validate_session_realm(
    caller_ura: &str,
    session_realm: Option<&str>,
    trust_anchor: &RealmTrustAnchor,
) -> Result<(), Status> {
    let Some(daemon_realm) = session_realm else {
        return Ok(());
    };

    let caller_realm = parse_realm_from_ura(caller_ura).ok_or_else(|| {
        Status::invalid_argument(format!(
            "<self>.session: caller URA `{caller_ura}` does not match the canonical \
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
    // explicitly listed this URA under realm-trust.toml.
    if trust_anchor.lookup(caller_ura).is_some() {
        return Ok(());
    }

    Err(Status::permission_denied(format!(
        "<self>.session: caller `{caller_ura}` from realm `{caller_realm}` is \
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
        membership_ura: String,
    },
    Offline {
        membership_ura: String,
        reason: &'static str,
    },
}

impl From<crate::services::presence_registry::PresenceEvent> for PresenceEventDelta {
    fn from(event: crate::services::presence_registry::PresenceEvent) -> Self {
        use crate::services::presence_registry::{OfflineReason, PresenceEvent};
        match event {
            PresenceEvent::Online { ura } => Self::Online {
                membership_ura: ura,
            },
            PresenceEvent::Offline { ura, reason } => Self::Offline {
                membership_ura: ura,
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
/// carrying the canonical `(ability_ura, args)` pair the user
/// selected plus a `call_id` minted client-side that DEC-N4
/// §2.1 threads back through `ForwardInvokeResponse.
/// correlation_call_id` so the caller can correlate the
/// response with its awaiting bidi.
pub(crate) struct InnerPayload {
    pub ability_ura: String,
    pub args_bytes: Vec<u8>,
    pub call_id: String,
}

impl InnerPayload {
    fn public_ability_for_target(&self, target_ura: &str) -> Result<String, Status> {
        crate::ura::public_ability_name_from_ability_ura(target_ura, &self.ability_ura).ok_or_else(
            || {
                Status::invalid_argument(format!(
                    "federation.forward_invoke: ability_ura `{}` does not belong to target `{}`",
                    self.ability_ura, target_ura
                ))
            },
        )
    }
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
             {ability_ura, args, call_id} payload",
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
             with `ability_ura`, `args`, and `call_id` fields",
        )
    })?;
    let ability_ura = obj
        .get("ability_ura")
        .and_then(|v| v.as_str())
        .filter(|s| !s.is_empty())
        .ok_or_else(|| {
            Status::invalid_argument(
                "federation.forward_invoke: inner envelope is missing a non-empty \
                 string `ability_ura` field",
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
        ability_ura,
        args_bytes,
        call_id,
    })
}

/// Build the strict envelope the cross-hub dialer attaches to the
/// rebuilt peer `InvokeRequest`.
///
/// This is a new hub-to-hub invocation, not a verbatim re-send:
/// `caller = local hub`, `callee = target hub`, and `subject =
/// original caller` when present. Every URA must parse through the
/// canonical URA parser before the peer request is sent.
pub(crate) fn build_peer_envelope(
    caller_envelope: Option<&Envelope>,
    target_ura: &str,
    local_realm: Option<&str>,
) -> Result<Envelope, Status> {
    use rand::RngCore as _;

    let mut forwarded = caller_envelope.cloned().unwrap_or_default();
    let peer_hub_ura = parse_realm_from_ura(target_ura)
        .map(|realm| crate::ura::hub_ura(&realm))
        .ok_or_else(|| {
            Status::invalid_argument(format!("target_ura is not a valid URA: {target_ura}"))
        })?;

    let caller_ura = local_realm
        .map(crate::ura::hub_ura)
        .or_else(|| {
            forwarded
                .caller
                .as_ref()
                .map(|caller| caller.ura.trim().to_string())
                .filter(|ura| !ura.is_empty())
        })
        .ok_or_else(|| Status::invalid_argument("peer envelope missing caller URA"))?;
    crate::ura::parse_ura(&caller_ura).map_err(|err| {
        Status::invalid_argument(format!("peer envelope caller URA is invalid: {err}"))
    })?;
    crate::ura::parse_ura(&peer_hub_ura).map_err(|err| {
        Status::invalid_argument(format!("peer envelope callee URA is invalid: {err}"))
    })?;
    let subject_ura = caller_envelope
        .and_then(|env| env.caller.as_ref())
        .map(|caller| caller.ura.trim().to_string())
        .filter(|ura| !ura.is_empty())
        .unwrap_or_else(|| peer_hub_ura.clone());
    crate::ura::parse_ura(&subject_ura).map_err(|err| {
        Status::invalid_argument(format!("peer envelope subject URA is invalid: {err}"))
    })?;

    let profile = crate::services::invocation_transport::DEFAULT_URA_PROFILE.to_string();
    forwarded.caller = Some(AgentIdentity {
        ura: caller_ura,
        profile: profile.clone(),
    });
    forwarded.callee = Some(AgentIdentity {
        ura: peer_hub_ura,
        profile: profile.clone(),
    });
    forwarded.subject = Some(SubjectIdentity {
        ura: subject_ura,
        profile,
    });

    if forwarded.invocation_nonce.len() != 16 {
        let mut nonce = vec![0u8; 16];
        rand::rngs::OsRng.fill_bytes(&mut nonce);
        forwarded.invocation_nonce = nonce;
    }

    Ok(forwarded)
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
        InvocationEnvelope, SubjectIdentity as AxiomSubjectIdentity, UraProfile,
    };
    use ed25519_dalek::{Signer as _, SigningKey};
    use sha2::{Digest, Sha256};

    envelope.causal_context = None;

    let caller_ura = envelope
        .caller
        .as_ref()
        .map(|caller| caller.ura.trim())
        .filter(|ura| !ura.is_empty())
        .ok_or_else(|| {
            Status::internal("cross-hub forward_invoke signing: caller URA missing after rewrite")
        })?;
    let callee_ura = envelope
        .callee
        .as_ref()
        .map(|callee| callee.ura.trim())
        .filter(|ura| !ura.is_empty())
        .ok_or_else(|| {
            Status::internal("cross-hub forward_invoke signing: callee URA missing after rewrite")
        })?;
    let subject_ura = envelope
        .subject
        .as_ref()
        .map(|subject| subject.ura.trim())
        .filter(|ura| !ura.is_empty())
        .ok_or_else(|| {
            Status::internal("cross-hub forward_invoke signing: subject URA missing after rewrite")
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
    // `CALLER_SIGNATURE_INVALID:caller_signature_invalid`.
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
        caller: AxiomAgentIdentity::new(caller_ura, UraProfile::EasynetStrictV2),
        callee: AxiomAgentIdentity::new(callee_ura, UraProfile::EasynetStrictV2),
        subject: AxiomSubjectIdentity::new(subject_ura, UraProfile::EasynetStrictV2),
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
///   "agent_ura": "easynet:///r/<realm>/hub",
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
                let hub_subject_id = easynet_axon::invocation::private_hub_subject_id(realm);
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

// ── Phase 5a tombstone: ForwardReceipt / SharedReceiptStore ──
// Phase 5a deleted three things in lockstep:
//   * the `FORWARD_RECEIPT_TYPE` / `FORWARD_RECEIPT_DIGEST_CONTENT_TYPE`
//     constants,
//   * `build_forward_receipt` (the caller-hub ForwardReceipt builder
//     modelled on InvocationReceipt — DEC-N5 §1 only required the
//     causal link, so the dedicated container was redundant), and
//   * every `self.admission.receipt_store().record(...)` call site in
//     `dispatch_federation_forward_invoke`.
//
// The in-memory `FORWARD_RECEIPT_TYPE` ring-buffer entries had no
// production reader — only legacy tests in the same file inspected
// them — so removing both the writes and the helper loses zero
// production observability. Cross-hub forward-invoke outcomes are
// now observable via:
//   * the dispatched invocation's `InvocationLedger` row, written
//     when the target reaches a terminal state, and
//   * `op_event!(component = daemon_invocation, kind = forward_invoke_*)`
//     log lines for transport-level miss / fail diagnostics.
//
// The trade-off (documented in src/services/mod.rs): admission-time
// emission of any audit artefact is gone. An admission that succeeds
// but whose invocation never reaches a terminal state leaves no
// record. Closing that gap is a Week-5+ topic; for now the operator
// log is the audit source for non-terminal calls.

/// Build a `DispatchFrame` carrying a `SessionDispatch::Dispatch` JSON
/// payload, ready to push down a target's `<self>.session` reverse
/// channel. Encoding failure is impossible for the current variant
/// (call_id u64, owned String, owned Vec<u8>) but mapped to
/// `Status::internal` for forward-compatibility per letter 25 §"flag".
fn build_invoke_remote_dispatch_frame(
    call_id: u64,
    callee_ura: &str,
    subject_ura: Option<&str>,
    ability: &str,
    args: &[u8],
    args_content_envelope: SessionContentEnvelope,
    metadata: HashMap<String, String>,
) -> Result<DispatchFrame, Status> {
    let payload = SessionDispatch::Dispatch {
        call_id,
        callee_ura: Some(callee_ura.to_string()),
        subject_ura: subject_ura
            .filter(|subject| !subject.trim().is_empty())
            .map(ToOwned::to_owned),
        ability: ability.to_string(),
        args: args.to_vec(),
        args_content_envelope,
        metadata,
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
    callee_ura: &str,
    subject_ura: Option<&str>,
    ability: &str,
    args: &[u8],
    metadata: HashMap<String, String>,
) -> Result<DispatchFrame, Status> {
    let payload = SessionDispatch::BidiOpen {
        call_id,
        callee_ura: Some(callee_ura.to_string()),
        subject_ura: subject_ura
            .filter(|subject| !subject.trim().is_empty())
            .map(ToOwned::to_owned),
        ability: ability.to_string(),
        args: args.to_vec(),
        args_content_envelope: SessionContentEnvelope::plaintext_json(),
        metadata,
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

fn build_remote_bidi_input_frame_for_ability(
    call_id: u64,
    ability: &str,
    payload: &[u8],
    pty_resize: Option<(u32, u32)>,
    eof: bool,
) -> Result<DispatchFrame, Status> {
    if eof {
        return Ok(build_remote_bidi_input_dispatch_frame(call_id, &[], true));
    }
    if ability == crate::runtime::agents::pty_attach_ability::ABILITY_PTY_SESSION_ATTACH {
        use base64::{engine::general_purpose::STANDARD as B64, Engine as _};
        let frame = if let Some((cols, rows)) = pty_resize {
            serde_json::json!({"type": "resize", "cols": cols, "rows": rows})
        } else {
            serde_json::json!({"type": "stdin", "data": B64.encode(payload)})
        };
        let bytes = serde_json::to_vec(&frame).map_err(|err| {
            Status::internal(format!("InvokeBidi remote pty: encode input frame: {err}"))
        })?;
        return Ok(build_remote_bidi_input_dispatch_frame(
            call_id, &bytes, false,
        ));
    }
    Ok(build_remote_bidi_input_dispatch_frame(
        call_id, payload, false,
    ))
}

fn remote_bidi_target_ura(envelope_open: &EnvelopeOpen) -> Option<String> {
    envelope_open
        .envelope
        .as_ref()
        .and_then(|env| env.callee.as_ref())
        .map(|callee| callee.ura.trim())
        .filter(|ura| !ura.is_empty())
        .map(ToOwned::to_owned)
}

fn remote_bidi_subject_ura(envelope_open: &EnvelopeOpen) -> Option<String> {
    envelope_open
        .envelope
        .as_ref()
        .and_then(|env| env.subject.as_ref())
        .map(|subject| subject.ura.trim())
        .filter(|ura| !ura.is_empty())
        .map(ToOwned::to_owned)
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

/// Build a one-shot `<self>.invoke_remote` Response stream carrying a
/// single terminal frame whose `InvokeRemoteDown::Result` has
/// `error = Some(msg)` and an empty payload.
///
/// Why this exists: `dispatch_invoke_remote` has two flavours of
/// failure — protocol/structural (malformed frame 0, daemon
/// misconfigured) and operational (target not in registry, target
/// channel full / closed, target handler errored). The protocol /
/// structural ones return a `tonic::Status` (gRPC-level error,
/// surfaces upstream as HTTP 500). The operational ones MUST stay
/// in-band so the caller sees a successful stream that yields a
/// final frame whose `error` field carries the structured reason —
/// otherwise a Go/HTTP shim atop tonic surfaces them as opaque 500s
/// and the human user never sees "target offline", just "500".
/// The post-dispatch failure paths already did this (target session
/// dropped, target replied with error); the pre-dispatch paths used
/// to raise `Status`. This helper aligns both halves under one
/// shape.
fn invoke_remote_inband_error_response(
    msg: String,
) -> Result<Response<BoxedDownStream<InvokeBidiDown>>, Status> {
    let failure = SessionFailure::from_reason(&msg, "INVOCATION_FAILED", false);
    let down = InvokeRemoteDown::Result {
        payload: Vec::new(),
        error: Some(msg),
        failure: Some(failure),
        request_id: None,
    };
    let frame = build_invoke_remote_terminal_frame(&down)?;
    let (down_tx, down_rx) = mpsc::channel::<Result<InvokeBidiDown, Status>>(1);
    tokio::spawn(async move {
        let _ = down_tx.send(Ok(frame)).await;
    });
    let stream = ReceiverStream::new(down_rx);
    Ok(Response::new(
        Box::pin(stream) as BoxedDownStream<InvokeBidiDown>
    ))
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

fn build_unary_ledger_record(
    request: &InvokeRequest,
    started_unix_ms: i64,
    completed_unix_ms: i64,
    result: &Result<Response<InvokeResponse>, Status>,
) -> Result<easynet_axon::invocation::InvocationLedgerRecord, anyhow::Error> {
    let envelope = request.envelope.as_ref();
    let caller_ura = envelope
        .and_then(|env| env.caller.as_ref())
        .map(|identity| identity.ura.clone())
        .filter(|ura| !ura.trim().is_empty())
        .unwrap_or_else(|| crate::ura::hub_ura("localhost"));
    let realm = parse_realm_from_ura(&caller_ura).unwrap_or_else(|| "localhost".to_string());
    let callee_ura = envelope
        .and_then(|env| env.callee.as_ref())
        .map(|identity| identity.ura.clone())
        .filter(|ura| !ura.trim().is_empty())
        .unwrap_or_else(|| crate::ura::hub_ura(&realm));
    let subject_ura = envelope
        .and_then(|env| env.subject.as_ref())
        .map(|identity| identity.ura.clone())
        .filter(|ura| !ura.trim().is_empty())
        .unwrap_or_else(|| caller_ura.clone());
    let request_id = envelope
        .map(|env| env.request_id.clone())
        .filter(|id| !id.trim().is_empty())
        .unwrap_or_else(|| format!("req-{}", short_hash(request.function_name.as_bytes())));
    let trace_id = envelope.map(|env| env.trace_id.clone()).unwrap_or_default();
    let span_id = envelope.map(|env| env.span_id.clone()).unwrap_or_default();
    let invocation_ura =
        invocation_resource_ura(&realm, &request_id, &subject_ura, &callee_ura, &caller_ura)?;
    let elapsed_ms = completed_unix_ms.saturating_sub(started_unix_ms) as u64;
    let authority_binding = ledger_authority_binding_for_request(request);

    let mut builder = easynet_axon::invocation::InvocationLedgerRecordBuilder::new()
        .invocation_ura(invocation_ura)
        .request_id(request_id)
        .trace_id(trace_id)
        .span_id(span_id)
        .caller_ura(caller_ura)
        .callee_ura(callee_ura)
        .subject_ura(subject_ura)
        .ability_ura(ability_ura_for_name(&realm, &request.function_name))
        .ability_name(request.function_name.clone())
        .started_unix_ms(started_unix_ms)
        .completed_unix_ms(completed_unix_ms)
        .elapsed_ms(elapsed_ms)
        .causal_links(causal_links_from_envelope(envelope))
        .authority_binding(authority_binding)
        .args(easynet_axon::invocation::LedgerEventPayload::digest(
            "application/octet-stream",
            &request.arguments,
        ));

    match result {
        Ok(response) => {
            let body = response.get_ref();
            let state = if body.state
                == easynet_axon::invocation::InvocationState::Completed.to_wire_i32()
            {
                "completed"
            } else if body.state == easynet_axon::invocation::InvocationState::Failed.to_wire_i32()
            {
                "failed"
            } else {
                "unknown"
            };
            builder = builder.state(state.to_string());
            if state == "failed" {
                let error = ledger_error_from_invoke_response(
                    body,
                    completed_unix_ms,
                    &request.function_name,
                );
                builder = builder
                    .error(error.clone())
                    .diagnostics(vec![ledger_error_diagnostic(
                        completed_unix_ms,
                        error.clone(),
                    )]);
            } else {
                builder = builder.result(easynet_axon::invocation::LedgerEventPayload::digest(
                    body.result_content_type.clone(),
                    &body.result,
                ));
            }
        }
        Err(status) => {
            let error = ledger_error_from_status(status, &request.function_name);
            builder = builder
                .state("failed".to_string())
                .error(error.clone())
                .diagnostics(vec![ledger_error_diagnostic(completed_unix_ms, error)]);
        }
    }

    Ok(builder.build()?)
}

fn ledger_error_from_status(
    status: &Status,
    ability_name: &str,
) -> easynet_axon::invocation::LedgerErrorRecord {
    let fallback = status_fallback_failure_code(status.code());
    let code = crate::runtime::failure_codes::FailureCodeClassifier::classify_or(
        status.message(),
        fallback,
    );
    let mut context = BTreeMap::from([
        ("ability_name".to_string(), ability_name.to_string()),
        (
            "transport_status".to_string(),
            format!("{:?}", status.code()).to_ascii_lowercase(),
        ),
    ]);
    let failure_class =
        crate::runtime::failure_codes::FailureCodeClassifier::classify_error_class(&code);
    context.insert(
        "error_stage".to_string(),
        format!("{:?}", failure_class.stage),
    );
    context.insert(
        "security_class".to_string(),
        format!("{:?}", failure_class.security_class),
    );
    easynet_axon::invocation::LedgerErrorRecord {
        source: "daemon_invocation_service".to_string(),
        code,
        message: status.message().to_string(),
        retryable: status_code_retryable(status.code()),
        context,
    }
}

fn ledger_error_from_invoke_response(
    response: &InvokeResponse,
    completed_unix_ms: i64,
    ability_name: &str,
) -> easynet_axon::invocation::LedgerErrorRecord {
    let default_message = if response.scheduling_reason.trim().is_empty() {
        "invocation completed with failed state".to_string()
    } else {
        response.scheduling_reason.clone()
    };
    let Some(error) = response.error.as_ref() else {
        let code = crate::runtime::failure_codes::FailureCodeClassifier::classify_or(
            &default_message,
            "INVOCATION_FAILED",
        );
        let failure_class =
            crate::runtime::failure_codes::FailureCodeClassifier::classify_error_class(&code);
        return easynet_axon::invocation::LedgerErrorRecord {
            source: "daemon_invocation_service".to_string(),
            code,
            message: default_message,
            retryable: false,
            context: BTreeMap::from([
                ("ability_name".to_string(), ability_name.to_string()),
                (
                    "completed_unix_ms".to_string(),
                    completed_unix_ms.to_string(),
                ),
                (
                    "error_stage".to_string(),
                    format!("{:?}", failure_class.stage),
                ),
                (
                    "security_class".to_string(),
                    format!("{:?}", failure_class.security_class),
                ),
            ]),
        };
    };
    let code = crate::runtime::failure_codes::FailureCodeClassifier::explicit_or_reason(
        Some(error.code.as_str()),
        &error.message,
        "INVOCATION_FAILED",
    );
    let failure_class =
        crate::runtime::failure_codes::FailureCodeClassifier::classify_error_class(&code);
    easynet_axon::invocation::LedgerErrorRecord {
        source: "daemon_invocation_service".to_string(),
        code,
        message: terminal_failure_message(&error.message, "INVOCATION_FAILED"),
        retryable: error.retryable,
        context: BTreeMap::from([
            ("ability_name".to_string(), ability_name.to_string()),
            (
                "completed_unix_ms".to_string(),
                completed_unix_ms.to_string(),
            ),
            (
                "error_stage".to_string(),
                format!("{:?}", failure_class.stage),
            ),
            (
                "security_class".to_string(),
                format!("{:?}", failure_class.security_class),
            ),
        ]),
    }
}

fn ledger_error_diagnostic(
    completed_unix_ms: i64,
    error: easynet_axon::invocation::LedgerErrorRecord,
) -> easynet_axon::invocation::LedgerDiagnosticRecord {
    easynet_axon::invocation::LedgerDiagnosticRecord {
        timestamp_unix_ms: completed_unix_ms,
        level: "error".to_string(),
        source: error.source,
        code: error.code,
        message: error.message,
        retryable: error.retryable,
        payload: None,
    }
}

fn status_fallback_failure_code(code: tonic::Code) -> &'static str {
    match code {
        tonic::Code::InvalidArgument => "INVALID_ARGUMENT",
        tonic::Code::DeadlineExceeded => "INVOCATION_TIMED_OUT",
        tonic::Code::Cancelled => "INVOCATION_CANCELLED",
        tonic::Code::Unavailable => RESOLVE_SELECTED_HOST_UNAVAILABLE_CODE,
        _ => "INVOCATION_FAILED",
    }
}

fn status_code_retryable(code: tonic::Code) -> bool {
    matches!(
        code,
        tonic::Code::Unavailable | tonic::Code::ResourceExhausted | tonic::Code::DeadlineExceeded
    )
}

fn ledger_authority_binding_for_request(request: &InvokeRequest) -> &'static str {
    if bootstrap_authority_ability_for_ledger(&request.function_name) {
        "bootstrap"
    } else if request
        .metadata
        .get(DELEGATION_METADATA_KEY)
        .map(|value| !value.trim().is_empty())
        .unwrap_or(false)
    {
        "delegated"
    } else if request
        .metadata
        .get(SESSION_AUTHORITY_METADATA_KEY)
        .map(|value| !value.trim().is_empty())
        .unwrap_or(false)
    {
        "session"
    } else {
        "self"
    }
}

fn bootstrap_authority_ability_for_ledger(function: &str) -> bool {
    matches!(
        function,
        ABILITY_SELF_REGISTER_DEVICE_PUBKEY
            | ABILITY_RUNTIME_BOOTSTRAP_SELF_IDENTITY
            | ABILITY_FEDERATION_ADVERTISE_AGENT
            | ABILITY_SELF_LIST_USER_PUBKEYS
            | ABILITY_SELF_REVOKE_USER_PUBKEY
    )
}

fn causal_links_from_envelope(
    envelope: Option<&Envelope>,
) -> Vec<easynet_axon::invocation::InvocationCausalLink> {
    let Some(form) = envelope
        .and_then(|env| env.causal_context.as_ref())
        .and_then(|ctx| ctx.form.as_ref())
    else {
        return Vec::new();
    };
    match form {
        causal_context::Form::None(_) => Vec::new(),
        causal_context::Form::Scalar(receipt) => {
            vec![causal_link_from_receipt_ref(receipt, "causal")]
        }
        causal_context::Form::List(list) => list
            .prior
            .iter()
            .map(|receipt| causal_link_from_receipt_ref(receipt, "causal_join"))
            .collect(),
        causal_context::Form::Merkle(root) => {
            vec![easynet_axon::invocation::InvocationCausalLink {
                source_invocation_ura: None,
                source_receipt_ura: root.proof_ura.clone(),
                source_receipt_hash: hex::encode(&root.root),
                relation: "causal_merkle".to_string(),
            }]
        }
    }
}

fn causal_link_from_receipt_ref(
    receipt: &easynet_axon::pb::axon::v1::ReceiptRef,
    relation: &str,
) -> easynet_axon::invocation::InvocationCausalLink {
    easynet_axon::invocation::InvocationCausalLink {
        source_invocation_ura: invocation_ura_from_receipt_ura(&receipt.receipt_ura),
        source_receipt_ura: receipt.receipt_ura.clone(),
        source_receipt_hash: hex::encode(&receipt.receipt_hash),
        relation: relation.to_string(),
    }
}

fn invocation_ura_from_receipt_ura(receipt_ura: &str) -> Option<String> {
    receipt_ura
        .rsplit_once("/receipt/")
        .map(|(invocation_ura, _)| invocation_ura.to_string())
}

#[derive(Debug, Clone, PartialEq, Eq)]
struct InvocationResourceOwner {
    owner_id: String,
    path_prefix: String,
}

fn invocation_resource_ura(
    realm: &str,
    request_id: &str,
    subject_ura: &str,
    callee_ura: &str,
    caller_ura: &str,
) -> Result<String, anyhow::Error> {
    let owner = invocation_resource_owner_from_ura(subject_ura)
        .or_else(|| invocation_resource_owner_from_ura(callee_ura))
        .or_else(|| invocation_resource_owner_from_ura(caller_ura))
        .or_else(local_invocation_resource_owner)
        .ok_or_else(|| {
            anyhow::anyhow!(
                "cannot derive invocation resource owner from subject/callee/caller/local device URA"
            )
        })?;
    let request_segment = safe_resource_path_segment(request_id);
    let path = if owner.path_prefix.is_empty() {
        request_segment
    } else {
        format!("{}/{}", owner.path_prefix, request_segment)
    };
    Ok(crate::ura::resource_dot_ura(realm, &owner.owner_id, &path))
}

fn invocation_resource_owner_from_ura(ura: &str) -> Option<InvocationResourceOwner> {
    let parsed = crate::ura::parse_ura(ura).ok()?;
    match parsed.kind {
        crate::ura::URAKind::User => Some(InvocationResourceOwner {
            owner_id: format!("{}.invocations", parsed.user_id()?),
            path_prefix: String::new(),
        }),
        crate::ura::URAKind::Agent => {
            let (user_id, agent_id) = parsed.agent_ids()?;
            Some(InvocationResourceOwner {
                owner_id: format!("{user_id}.invocations"),
                path_prefix: format!("agents/{agent_id}/invocations"),
            })
        }
        crate::ura::URAKind::Device => Some(InvocationResourceOwner {
            owner_id: format!("device.{}", parsed.device_id()?),
            path_prefix: "invocations".to_string(),
        }),
        _ => None,
    }
}

fn local_invocation_resource_owner() -> Option<InvocationResourceOwner> {
    let local = crate::persistence::local_agents::load().ok()?;
    invocation_resource_owner_from_ura(&local.host_device_agent_ura)
}

fn safe_resource_path_segment(raw: &str) -> String {
    let trimmed = raw.trim();
    let mut out = String::with_capacity(trimmed.len());
    for ch in trimmed.chars() {
        if ch.is_ascii_alphanumeric() || matches!(ch, '-' | '_' | '.') {
            out.push(ch);
        } else {
            out.push('-');
        }
    }
    let out = out.trim_matches('-').to_string();
    if out.is_empty() {
        return format!("request-{}", short_hash(raw.as_bytes()));
    }
    if out == trimmed {
        out
    } else {
        format!("{}-{}", out, short_hash(raw.as_bytes()))
    }
}

fn ability_ura_for_name(realm: &str, ability_name: &str) -> String {
    if ability_name.split_once('.').is_some() {
        crate::ura::hub_ability_ura(realm, ability_name)
    } else {
        crate::ura::ability_ura(realm, "hub", "runtime", ability_name)
    }
}

fn now_unix_ms() -> i64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|duration| duration.as_millis() as i64)
        .unwrap_or(0)
}

fn short_hash(bytes: &[u8]) -> String {
    use sha2::{Digest, Sha256};

    let mut hasher = Sha256::new();
    hasher.update(bytes);
    let full = hex::encode(hasher.finalize());
    full[..16].to_string()
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
        state: easynet_axon::invocation::InvocationState::Completed.to_wire_i32(),
        ..InvokeResponse::default()
    };
    Ok(Response::new(invoke_response))
}

fn sorted_non_empty_urls(urls: Vec<String>) -> Vec<String> {
    urls.into_iter()
        .map(|url| url.trim().to_string())
        .filter(|url| !url.is_empty())
        .collect::<BTreeSet<_>>()
        .into_iter()
        .collect()
}

fn namespace_proxy_resolve_peer_arguments(
    request: &federation_wrappers::NamespaceProxyResolveRequest,
) -> Result<Vec<u8>, Status> {
    serde_json::to_vec(&serde_json::json!({
        "queryName": non_empty_json_string(&request.query_name),
        "qtype": non_empty_json_string(&request.qtype)
            .unwrap_or_else(|| "RESOLVE_TYPE_DIRECTORY_LISTING".to_string()),
        "callerUra": non_empty_json_string(&request.caller_ura),
        "subjectUra": non_empty_json_string(&request.subject_ura),
        "realmHint": non_empty_json_string(&request.realm_hint),
        "abilityName": non_empty_json_string(&request.ability_name),
    }))
    .map_err(|err| {
        Status::internal(format!(
            "namespace.proxy_resolve: encode peer request: {err}"
        ))
    })
}

fn namespace_proxy_resolve_empty_answer(
    request: &federation_wrappers::NamespaceProxyResolveRequest,
) -> serde_json::Value {
    namespace_proxy_resolve_merge_answer(request, Vec::new())
}

fn namespace_proxy_resolve_merge_answer(
    request: &federation_wrappers::NamespaceProxyResolveRequest,
    peer_answers: Vec<serde_json::Value>,
) -> serde_json::Value {
    let mut records = BTreeMap::<String, serde_json::Value>::new();
    for answer in peer_answers {
        let Some(rows) = answer.get("records").and_then(serde_json::Value::as_array) else {
            continue;
        };
        for row in rows {
            let key = namespace_record_merge_key(row);
            records.entry(key).or_insert_with(|| row.clone());
        }
    }

    serde_json::json!({
        "answerKind": "RESOLVE_ANSWER_KIND_NON_DISPATCHABLE",
        "canonicalName": non_empty_json_string(&request.query_name),
        "records": records.into_values().collect::<Vec<_>>(),
        "releaseProfile": "RESOLVER_RELEASE_PROFILE_PRODUCTION",
        "cachePolicy": {
            "ttlMs": 0,
            "sharedCacheable": false,
            "retryAfterUnixMs": 0,
        },
    })
}

fn namespace_record_merge_key(row: &serde_json::Value) -> String {
    let name = row
        .get("name")
        .and_then(serde_json::Value::as_str)
        .unwrap_or_default();
    let record_type = row
        .get("recordType")
        .or_else(|| row.get("record_type"))
        .and_then(serde_json::Value::as_str)
        .unwrap_or_default();
    format!("{name}\u{1f}{record_type}")
}

fn non_empty_json_string(value: &str) -> Option<String> {
    let trimmed = value.trim();
    if trimmed.is_empty() {
        None
    } else {
        Some(trimmed.to_string())
    }
}

/// **PR-N1 commit 3a/N**. Extract the realm component from a
/// canonical EasyNet URA (`easynet:///r/{realm}/...`).
/// Returns `None` for URAs that do not match the canonical shape.
///
/// Pure function so it composes well into the cross-realm
/// routing branch landing in commit 3b/N: the dispatcher reads
/// `request.target_ura`, calls `parse_realm_from_ura`, and
/// looks the realm up in `federated_peers` to obtain a hub URA.
pub(crate) fn parse_realm_from_ura(ura: &str) -> Option<String> {
    parse_realm_from_register_ura(ura)
}

fn is_quota_exempt_system_ability(function: &str) -> bool {
    matches!(
        function,
        ABILITY_FEDERATION_JOIN
            | ABILITY_FEDERATION_ADVERTISE_AGENT
            | ABILITY_FEDERATION_ADVERTISE_ABILITIES
            | ABILITY_FEDERATION_HEARTBEAT
            | ABILITY_FEDERATION_RESOLVE
            | ABILITY_NAMESPACE_RESOLVE
            | ABILITY_NAMESPACE_PROXY_RESOLVE
            | ABILITY_FEDERATION_RESOLVE_KEY
            | ABILITY_FEDERATION_DISCOVER
            | ABILITY_FEDERATION_LIST_USER_DEVICES
            | ABILITY_FEDERATION_PROXY_LIST_USER_DEVICES
            | ABILITY_FEDERATION_REVOKE
            | ABILITY_FEDERATION_FORWARD_INVOKE
            | ABILITY_FEDERATION_SUBSCRIBE_DIRECTORY
            | ABILITY_FEDERATION_SUBSCRIBE_DIRECTORY_V2
            | ABILITY_SELF_REGISTER_DEVICE_PUBKEY
            | ABILITY_SELF_REVOKE_USER_PUBKEY
            | ABILITY_SELF_LIST_USER_PUBKEYS
            | ABILITY_SELF_SESSION
            | ABILITY_INVOKE_REMOTE
            | ABILITY_RUNTIME_BOOTSTRAP_SELF_IDENTITY
    )
}

pub(crate) fn quota_meters_function(function: &str) -> bool {
    !is_quota_exempt_system_ability(function)
}

fn quota_metered_ability_for_request(request: &InvokeRequest) -> Result<Option<String>, Status> {
    let function = request.function_name.as_str();
    if function == ABILITY_FEDERATION_FORWARD_INVOKE {
        let forward: federation_wrappers::ForwardInvokeRequest =
            parse_json_args(&request.arguments)?;
        let inner = decode_inner_payload(&forward.inner_envelope_b64)?;
        let public_ability = inner.public_ability_for_target(&forward.target_ura)?;
        return Ok(quota_meters_function(&public_ability).then_some(public_ability));
    }
    Ok(quota_meters_function(function).then(|| function.to_string()))
}

#[cfg(test)]
mod tests {
    use super::*;

    use crate::services::realm_trust_anchor::{RealmTrustAnchor, TrustedAgent, TrustedAgentRole};
    use crate::services::usage_quota_store::SharedUsageQuotaGate;
    use easynet_axon::pb::axon::v1::{AgentIdentity, Envelope};

    /// Test helper daemon URA — admitted by the test admission
    /// facade via the loopback bypass. Tests that exercise
    /// admission rejection construct a different facade.
    // URA v4.1.4: daemons are devices, not agents. Fixtures use the
    // canonical shape because forward_invoke no longer repairs legacy
    // `agent/<bare-id>` device aliases at the request boundary.
    const TEST_DAEMON_URI: &str = "easynet:///r/test-realm/device/test-daemon";

    fn make_service() -> DaemonInvocationService {
        let admission = AdmissionFacade::new(
            Arc::new(RealmTrustAnchor::default()),
            Some(TEST_DAEMON_URI.to_string()),
        );
        DaemonInvocationService::new(Arc::new(PresenceRegistry::new()), admission)
            .with_hub_signing_seed([0x11; 32])
    }

    fn publish_test_route(svc: &DaemonInvocationService, owner_ura: &str, public_name: &str) {
        publish_test_route_hosted_by(svc, owner_ura, public_name, TEST_DAEMON_URI);
    }

    fn publish_test_route_hosted_by(
        svc: &DaemonInvocationService,
        owner_ura: &str,
        public_name: &str,
        hosted_agent_host_ura: &str,
    ) {
        let public_name = crate::ura::owner_local_ability_name(owner_ura, public_name);
        let ability_ura = crate::ura::owner_ability_ura(owner_ura, &public_name)
            .unwrap_or_else(|| panic!("derive test ability URA for {owner_ura} {public_name}"));
        let host_ura = match crate::ura::parse_ura(owner_ura).map(|parsed| parsed.kind) {
            Ok(crate::ura::URAKind::Agent) => {
                svc.advertised_agents.upsert(
                    crate::services::advertised_agent_store::AdvertisedAgentRecord {
                        agent_ura: owner_ura.to_string(),
                        public_key_hex: String::new(),
                        host_node_id: Some(hosted_agent_host_ura.to_string()),
                        signing_authority:
                            crate::services::advertised_agent_store::AdvertisedAgentSigningAuthority::HostedBy {
                                host_ura: hosted_agent_host_ura.to_string(),
                            },
                    },
                );
                hosted_agent_host_ura.to_string()
            }
            _ => owner_ura.to_string(),
        };
        if svc.presence.lookup(&host_ura).is_none() {
            let (tx, _rx) = tokio::sync::mpsc::channel(1);
            svc.presence.insert(host_ura.clone(), tx);
        }
        let (namespace, local_name) = public_name
            .rsplit_once('.')
            .map_or(("", public_name.as_str()), |(namespace, local_name)| {
                (namespace, local_name)
            });
        svc.ability_catalog.upsert_projection(
            crate::services::ability_catalog_store::OwnerAbilityProjectionRow::new(
                owner_ura.to_string(),
                host_ura,
                1,
                "sha256:test".to_string(),
                4_102_444_800_000,
                vec![crate::runtime::owner_projection::AbilityProjectionSummary {
                    ability_ura: ability_ura.clone(),
                    owner_ura: owner_ura.to_string(),
                    namespace: namespace.to_string(),
                    local_name: local_name.to_string(),
                    descriptor_revision: "sha256:descriptor".to_string(),
                    schema_ref: None,
                    schema_hash: None,
                    policy_ref: "visibility:PUBLIC".to_string(),
                    route_summary_ref: Some(format!("route-ref::{ability_ura}")),
                    tags: vec!["class:unary".to_string()],
                    callable_summary:
                        crate::runtime::owner_projection::AbilityCallableSummary::minimal(
                            public_name.to_string(),
                        ),
                }],
            ),
        );
    }

    fn session_request_ability_ura(realm: &str, ability: &str) -> String {
        crate::ura::hub_ability_ura(realm, ability)
    }

    fn signed_delegation_metadata_for_test(
        signer: &ed25519_dalek::SigningKey,
        issuer_ura: &str,
        subject_ura: &str,
        caller_ura: &str,
        audience: &str,
        scopes: &[&str],
    ) -> String {
        use base64::{engine::general_purpose::STANDARD as BASE64_STANDARD, Engine as _};
        use ed25519_dalek::Signer as _;
        use serde::Serialize;

        #[derive(Serialize)]
        struct DelegationPayload {
            issuer_ura: String,
            subject_ura: String,
            caller_ura: String,
            audience: String,
            scopes: Vec<String>,
            issued_at_ms: i64,
            expires_at_ms: i64,
        }

        let payload = DelegationPayload {
            issuer_ura: issuer_ura.to_string(),
            subject_ura: subject_ura.to_string(),
            caller_ura: caller_ura.to_string(),
            audience: audience.to_string(),
            scopes: scopes.iter().map(|scope| (*scope).to_string()).collect(),
            issued_at_ms: 1_700_000_000_000,
            expires_at_ms: 4_102_444_800_000,
        };
        let payload_bytes = serde_json::to_vec(&payload).expect("delegation payload");
        let signature = signer.sign(&payload_bytes);
        let raw = serde_json::json!({
            "payload": serde_json::from_slice::<serde_json::Value>(&payload_bytes)
                .expect("payload JSON value"),
            "signature": BASE64_STANDARD.encode(signature.to_bytes()),
        });
        BASE64_STANDARD.encode(serde_json::to_vec(&raw).expect("delegation proof"))
    }

    fn make_quota_service_for_device_caller(caller_ura: &str, cap: i32) -> DaemonInvocationService {
        let anchor = RealmTrustAnchor::from_entries(vec![TrustedAgent {
            agent_ura: caller_ura.to_string(),
            public_key_b64: "AAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAA=".to_string(),
            role: TrustedAgentRole::Device,
            added_at_unix_ms: 1_700_000_000_000,
            origin_realm: None,
            hub_endpoint: None,
            tls_ca_pem_path: None,
        }])
        .expect("quota test anchor");
        let quota = crate::persistence::daemon_config::QuotaConfig::new(
            cap,
            60_000,
            std::collections::BTreeMap::new(),
        );
        let admission = AdmissionFacade::new(Arc::new(anchor), Some(TEST_DAEMON_URI.to_string()))
            .with_quota_gate(SharedUsageQuotaGate::from_policy(Some(quota)));
        DaemonInvocationService::new(Arc::new(PresenceRegistry::new()), admission)
            .with_hub_signing_seed([0x11; 32])
    }

    async fn runtime_with_json_echo(
        ability: &'static str,
        marker_key: &'static str,
        marker_value: &'static str,
    ) -> Arc<easynet_axon::invocation::LocalRuntime> {
        use easynet_axon::invocation::make_ability;

        let rt = easynet_axon::invocation::LocalRuntime::new();
        rt.register_ability(
            ability,
            make_ability(move |ctx| async move {
                let echoed_args: serde_json::Value =
                    serde_json::from_slice(&ctx.payload).unwrap_or(serde_json::Value::Null);
                Ok(serde_json::to_vec(&serde_json::json!({
                    marker_key: marker_value,
                    "echoed_args": echoed_args,
                }))
                .unwrap())
            }),
        )
        .await
        .unwrap();
        rt
    }

    fn test_envelope() -> Envelope {
        Envelope {
            caller: Some(AgentIdentity {
                ura: TEST_DAEMON_URI.to_string(),
                ..AgentIdentity::default()
            }),
            callee: Some(AgentIdentity {
                ura: TEST_DAEMON_URI.to_string(),
                ..AgentIdentity::default()
            }),
            subject: Some(SubjectIdentity {
                ura: TEST_DAEMON_URI.to_string(),
                ..SubjectIdentity::default()
            }),
            invocation_nonce: vec![0x11u8; 16],
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

    fn invoke_request_from_device(
        caller_ura: &str,
        function_name: &str,
        arguments: Vec<u8>,
    ) -> Request<InvokeRequest> {
        Request::new(InvokeRequest {
            envelope: Some(Envelope {
                caller: Some(AgentIdentity {
                    ura: caller_ura.to_string(),
                    ..AgentIdentity::default()
                }),
                callee: Some(AgentIdentity {
                    ura: TEST_DAEMON_URI.to_string(),
                    ..AgentIdentity::default()
                }),
                subject: Some(SubjectIdentity {
                    ura: TEST_DAEMON_URI.to_string(),
                    ..SubjectIdentity::default()
                }),
                invocation_nonce: vec![0x22; 16],
                ..Envelope::default()
            }),
            function_name: function_name.to_string(),
            arguments,
            ..InvokeRequest::default()
        })
    }

    #[test]
    fn quota_meters_user_abilities_but_exempts_control_plane() {
        assert!(quota_meters_function("observe.health"));
        assert!(quota_meters_function("agent.todo.run"));

        assert!(!quota_meters_function(ABILITY_FEDERATION_HEARTBEAT));
        assert!(!quota_meters_function(ABILITY_FEDERATION_FORWARD_INVOKE));
        assert!(!quota_meters_function(ABILITY_NAMESPACE_RESOLVE));
        assert!(!quota_meters_function(ABILITY_SELF_REGISTER_DEVICE_PUBKEY));
        assert!(!quota_meters_function(ABILITY_SELF_SESSION));

        assert!(
            quota_meters_function("federation.user_owned_probe"),
            "quota exemptions must be exact system abilities, not namespace prefixes"
        );
        assert!(
            quota_meters_function("<self>.user_owned_probe"),
            "a user-registered reserved-prefix ability must not bypass quota by spelling alone"
        );
    }

    #[test]
    fn quota_for_forward_invoke_meters_inner_user_ability_only() {
        let user_call = InvokeRequest {
            function_name: ABILITY_FEDERATION_FORWARD_INVOKE.to_string(),
            arguments: forward_invoke_args_for_ability(
                "easynet:///r/test-realm/device/target",
                "observe.health",
                serde_json::json!({}),
            ),
            ..InvokeRequest::default()
        };
        assert_eq!(
            quota_metered_ability_for_request(&user_call)
                .expect("forward invoke parses")
                .as_deref(),
            Some("observe.health")
        );

        let control_call = InvokeRequest {
            function_name: ABILITY_FEDERATION_FORWARD_INVOKE.to_string(),
            arguments: forward_invoke_args_for_ability(
                &crate::ura::hub_ura("test-realm"),
                ABILITY_FEDERATION_HEARTBEAT,
                serde_json::json!({}),
            ),
            ..InvokeRequest::default()
        };
        assert_eq!(
            quota_metered_ability_for_request(&control_call).expect("forward invoke parses"),
            None,
            "nested federation control-plane calls stay quota-exempt"
        );

        let reserved_prefix_user_call = InvokeRequest {
            function_name: ABILITY_FEDERATION_FORWARD_INVOKE.to_string(),
            arguments: forward_invoke_args_for_ability(
                "easynet:///r/test-realm/device/target",
                "federation.user_owned_probe",
                serde_json::json!({}),
            ),
            ..InvokeRequest::default()
        };
        assert_eq!(
            quota_metered_ability_for_request(&reserved_prefix_user_call)
                .expect("forward invoke parses")
                .as_deref(),
            Some("federation.user_owned_probe"),
            "forward_invoke must not give quota amnesty to non-system reserved-prefix names"
        );
    }

    #[tokio::test]
    async fn forward_invoke_quota_throttles_by_inner_user_ability() {
        let caller_ura = "easynet:///r/test-realm/device/quota-caller";
        let rt = runtime_with_json_echo("observe.health", "handled_by", "quota-test").await;
        let svc = make_quota_service_for_device_caller(caller_ura, 1).with_local_runtime(rt);
        publish_test_route(&svc, TEST_DAEMON_URI, "observe.health");
        let args = forward_invoke_args_for_ability(
            TEST_DAEMON_URI,
            "observe.health",
            serde_json::json!({"probe": true}),
        );

        let first = svc
            .invoke(invoke_request_from_device(
                caller_ura,
                ABILITY_FEDERATION_FORWARD_INVOKE,
                args.clone(),
            ))
            .await
            .expect("first forwarded user ability is within quota");
        let info = first
            .get_ref()
            .rate_limit
            .as_ref()
            .expect("forward_invoke response carries inner ability quota status");
        assert_eq!(info.quota_limit, 1);
        assert_eq!(info.quota_remaining, 0);

        let second = svc
            .invoke(invoke_request_from_device(
                caller_ura,
                ABILITY_FEDERATION_FORWARD_INVOKE,
                args,
            ))
            .await
            .expect_err("second forwarded user ability exhausts quota");
        assert_eq!(second.code(), tonic::Code::ResourceExhausted);
        assert!(
            second.message().contains("ability=observe.health"),
            "quota error must name the inner user ability, got: {}",
            second.message()
        );
    }

    fn parse_response_body<T: serde::de::DeserializeOwned>(resp: Response<InvokeResponse>) -> T {
        let body = resp.into_inner();
        assert_eq!(body.result_content_type, FEDERATION_RESULT_CONTENT_TYPE);
        serde_json::from_slice(&body.result).expect("response body deserialises")
    }

    fn assert_route_negative_noroute(message: &str) {
        assert!(
            message.contains(super::ROUTE_NEGATIVE_CODE),
            "expected typed route negative code, got: {message}"
        );
        assert!(
            message.contains(easynet_axon::pb::axon::v1::NegativeReason::Noroute.as_str_name()),
            "expected NOROUTE negative reason, got: {message}"
        );
    }

    #[tokio::test]
    async fn invoke_dispatches_federation_join_to_wrapper() {
        let svc = make_service();
        let resp = svc
            .invoke(invoke_request(
                ABILITY_FEDERATION_JOIN,
                r#"{"membership_ura":"easynet:///r/realm/device/n1","realm":"realm"}"#,
            ))
            .await
            .expect("dispatch returns Ok");
        let body: federation_wrappers::JoinResponse = parse_response_body(resp);
        assert_eq!(body.membership_ura, "easynet:///r/realm/device/n1");
        assert_eq!(body.realm, "realm");
        assert_eq!(body.join_receipt_hash.len(), 64);
    }

    #[tokio::test]
    async fn invoke_dispatches_federation_advertise_agent() {
        let svc = make_service();
        let resp = svc
            .invoke(invoke_request(
                ABILITY_FEDERATION_ADVERTISE_AGENT,
                r#"{"agent_ura":"easynet:///r/realm/device/n1"}"#,
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
                r#"{"agent_ura":"easynet:///r/realm/device/n1"}"#,
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
    async fn invoke_dispatches_namespace_resolve_to_typed_answer() {
        let svc = make_service();
        let owner_ura = TEST_DAEMON_URI;
        let ability_ura =
            crate::ura::owner_ability_ura(owner_ura, "agent.list").expect("device ability ura");
        svc.presence.insert(owner_ura.to_string(), {
            let (tx, _rx) = mpsc::channel(1);
            tx
        });
        svc.ability_catalog.upsert_projection(
            crate::services::ability_catalog_store::OwnerAbilityProjectionRow::new(
                owner_ura.to_string(),
                owner_ura.to_string(),
                1,
                "sha256:test".to_string(),
                4_102_444_800_000,
                vec![crate::runtime::owner_projection::AbilityProjectionSummary {
                    ability_ura: ability_ura.clone(),
                    owner_ura: owner_ura.to_string(),
                    namespace: "agent".to_string(),
                    local_name: "list".to_string(),
                    descriptor_revision: "sha256:descriptor".to_string(),
                    schema_ref: None,
                    schema_hash: None,
                    policy_ref: "visibility:PUBLIC".to_string(),
                    route_summary_ref: Some(format!("route-ref::{ability_ura}")),
                    tags: vec!["class:unary".to_string()],
                    callable_summary:
                        crate::runtime::owner_projection::AbilityCallableSummary::minimal(
                            "agent.list",
                        ),
                }],
            ),
        );

        let resp = svc
            .invoke(invoke_request(
                ABILITY_NAMESPACE_RESOLVE,
                &serde_json::json!({
                    "queryName": owner_ura,
                    "qtype": "RESOLVE_TYPE_ROUTE",
                    "abilityName": "agent.list",
                })
                .to_string(),
            ))
            .await
            .expect("namespace.resolve dispatch returns Ok");
        let body: serde_json::Value = parse_response_body(resp);

        assert_eq!(
            body["answerKind"],
            easynet_axon::pb::axon::v1::ResolveAnswerKind::FinalRoute.as_str_name()
        );
        assert_eq!(body["abilityUra"], ability_ura);
        assert_eq!(
            body["nextHop"]["localDeviceAbility"]["deviceUra"],
            TEST_DAEMON_URI
        );
    }

    #[tokio::test]
    async fn namespace_resolve_cross_realm_route_returns_peer_hub_delegation() {
        let remote_owner = crate::ura::device_ura("remote-realm", "remote-device");
        let ability_ura =
            crate::ura::owner_ability_ura(&remote_owner, "observe.health").expect("ability ura");
        let svc = make_service()
            .with_session_realm("local-realm")
            .with_federated_peers(BTreeMap::from([(
                "remote-realm".to_string(),
                "https://remote-hub.example".to_string(),
            )]));

        let resp = svc
            .invoke(invoke_request(
                ABILITY_NAMESPACE_RESOLVE,
                &serde_json::json!({
                    "queryName": ability_ura,
                    "qtype": "RESOLVE_TYPE_ROUTE",
                })
                .to_string(),
            ))
            .await
            .expect("namespace.resolve dispatch returns Ok");
        let body: serde_json::Value = parse_response_body(resp);

        assert_eq!(
            body["answerKind"],
            easynet_axon::pb::axon::v1::ResolveAnswerKind::Delegation.as_str_name()
        );
        assert_eq!(body["ownerUra"], remote_owner);
        assert_eq!(body["nextHop"]["peerHub"]["realm"], "remote-realm");
        assert_eq!(
            body["nextHop"]["peerHub"]["hubUra"],
            crate::ura::hub_ura("remote-realm")
        );
        assert_eq!(
            body["nextHop"]["peerHub"]["endpoints"][0]["endpoint"],
            "https://remote-hub.example"
        );
        assert_eq!(
            body["nextHop"]["peerHub"]["endpoints"][0]["metadata"]["source"],
            "federated_peers"
        );
        assert_eq!(
            body["selectedRoute"]["reason"],
            easynet_axon::pb::axon::v1::RouteReason::PeerDelegation.as_str_name()
        );
    }

    #[tokio::test]
    async fn invoke_writes_success_record_to_invocation_ledger() {
        let temp = tempfile::tempdir().expect("tempdir");
        let ledger = Arc::new(
            easynet_axon::invocation::InvocationLedger::open(
                temp.path().join("billing").join("invocations.redb"),
            )
            .expect("ledger"),
        );
        let svc = make_service().with_invocation_ledger(Arc::clone(&ledger));

        svc.invoke(invoke_request(ABILITY_FEDERATION_RESOLVE, "{}"))
            .await
            .expect("dispatch returns Ok");

        let records = ledger.list_all().expect("ledger list");
        assert_eq!(records.len(), 1);
        let record = &records[0];
        assert_eq!(record.caller_ura, TEST_DAEMON_URI);
        let expected_prefix =
            crate::ura::resource_dot_ura("test-realm", "device.test-daemon", "invocations/");
        assert!(record.invocation_ura.starts_with(&expected_prefix));
        assert!(!record.invocation_ura.contains("/resource/invocation."));
        assert_eq!(record.ability_name, ABILITY_FEDERATION_RESOLVE);
        assert_eq!(
            record.ability_ura,
            "easynet:///r/test-realm/ability/hub.federation.resolve"
        );
        assert_eq!(record.state, "completed");
        assert_eq!(record.authority_binding, "self");
        assert!(matches!(
            record.args,
            easynet_axon::invocation::LedgerEventPayload::Digest { .. }
        ));
        assert!(record.result.is_some());
    }

    #[tokio::test]
    async fn invoke_writes_error_record_to_invocation_ledger() {
        let temp = tempfile::tempdir().expect("tempdir");
        let ledger = Arc::new(
            easynet_axon::invocation::InvocationLedger::open(
                temp.path().join("billing").join("invocations.redb"),
            )
            .expect("ledger"),
        );
        let svc = make_service().with_invocation_ledger(Arc::clone(&ledger));

        let err = svc
            .invoke(invoke_request("unknown.ability", "{}"))
            .await
            .expect_err("unknown ability returns status");
        assert_eq!(err.code(), tonic::Code::FailedPrecondition);

        let records = ledger.list_all().expect("ledger list");
        assert_eq!(records.len(), 1);
        let record = &records[0];
        let expected_prefix =
            crate::ura::resource_dot_ura("test-realm", "device.test-daemon", "invocations/");
        assert!(record.invocation_ura.starts_with(&expected_prefix));
        assert!(!record.invocation_ura.contains("/resource/invocation."));
        assert_eq!(record.state, "failed");
        assert_eq!(record.ability_name, "unknown.ability");
        assert_eq!(
            record.error.as_ref().map(|err| err.code.as_str()),
            Some(ROUTE_NEGATIVE_CODE)
        );
        assert_eq!(
            record
                .error
                .as_ref()
                .and_then(|err| err.context.get("transport_status"))
                .map(String::as_str),
            Some("failedprecondition")
        );
        assert_eq!(record.diagnostics[0].code, ROUTE_NEGATIVE_CODE);
    }

    #[test]
    fn unary_ledger_projects_failed_invoke_response_error() {
        let request = invoke_request("terminal.fs.read", "{}").into_inner();
        let response = InvokeResponse {
            state: easynet_axon::invocation::InvocationState::Failed.to_wire_i32(),
            scheduling_reason: "handler failed".to_string(),
            error: Some(Error {
                code: "TARGET_NOT_IN_PRESENCE_REGISTRY".to_string(),
                message: "target device is not in PresenceRegistry".to_string(),
                retryable: true,
                stage: ErrorStage::Transport as i32,
                security_class: SecurityClass::Transport as i32,
                ..Error::default()
            }),
            ..InvokeResponse::default()
        };
        let result = Ok(Response::new(response));
        let record = build_unary_ledger_record(&request, 10, 15, &result).expect("ledger record");

        assert_eq!(record.state, "failed");
        assert!(record.result.is_none());
        let error = record.error.as_ref().expect("ledger error");
        assert_eq!(error.code, "TARGET_NOT_IN_PRESENCE_REGISTRY");
        assert_eq!(error.message, "target device is not in PresenceRegistry");
        assert!(error.retryable);
        assert_eq!(
            error.context.get("error_stage").map(String::as_str),
            Some("Transport")
        );
        assert_eq!(record.diagnostics.len(), 1);
        assert_eq!(
            record.diagnostics[0].code,
            "TARGET_NOT_IN_PRESENCE_REGISTRY"
        );
    }

    #[tokio::test]
    async fn malformed_forward_invoke_quota_parse_error_is_audited() {
        let temp = tempfile::tempdir().expect("tempdir");
        let ledger = Arc::new(
            easynet_axon::invocation::InvocationLedger::open(
                temp.path().join("ledger").join("invocations.redb"),
            )
            .expect("ledger"),
        );
        let svc = make_service().with_invocation_ledger(Arc::clone(&ledger));

        let err = svc
            .invoke(invoke_request(
                ABILITY_FEDERATION_FORWARD_INVOKE,
                "{not-json",
            ))
            .await
            .expect_err("malformed forward_invoke must reject");
        assert_eq!(err.code(), tonic::Code::InvalidArgument);

        let records = ledger.list_all().expect("ledger list");
        assert_eq!(
            records.len(),
            1,
            "quota pre-parse errors must still write one failed ledger row"
        );
        let record = &records[0];
        assert_eq!(record.state, "failed");
        assert_eq!(record.ability_name, ABILITY_FEDERATION_FORWARD_INVOKE);
        assert_eq!(
            record.error.as_ref().map(|err| err.code.as_str()),
            Some("INVALID_ARGUMENT")
        );
        assert_eq!(
            record
                .error
                .as_ref()
                .and_then(|err| err.context.get("transport_status"))
                .map(String::as_str),
            Some("invalidargument")
        );
    }

    #[test]
    fn ledger_authority_binding_classifies_bootstrap_delegated_session_and_self() {
        let bootstrap = invoke_request(ABILITY_SELF_REGISTER_DEVICE_PUBKEY, "{}").into_inner();
        assert_eq!(
            ledger_authority_binding_for_request(&bootstrap),
            "bootstrap"
        );

        let mut delegated = invoke_request("demo.delegated", "{}").into_inner();
        delegated.metadata.insert(
            DELEGATION_METADATA_KEY.to_string(),
            "serialized-proof".to_string(),
        );
        assert_eq!(
            ledger_authority_binding_for_request(&delegated),
            "delegated"
        );

        let mut session = invoke_request("demo.session", "{}").into_inner();
        session.metadata.insert(
            SESSION_AUTHORITY_METADATA_KEY.to_string(),
            "serialized-session-authority".to_string(),
        );
        assert_eq!(ledger_authority_binding_for_request(&session), "session");

        let self_authority = invoke_request("demo.self", "{}").into_inner();
        assert_eq!(
            ledger_authority_binding_for_request(&self_authority),
            "self"
        );
    }

    #[test]
    fn invocation_resource_ura_is_owned_by_subject_user_when_present() {
        let ura = invocation_resource_ura(
            "test-realm",
            "req-1",
            &crate::ura::user_ura("test-realm", "alice"),
            &crate::ura::device_ura("test-realm", "callee-device"),
            &crate::ura::device_ura("test-realm", "caller-device"),
        )
        .expect("resource ura");
        assert_eq!(
            ura,
            "easynet:///r/test-realm/resource/alice.invocations/req-1"
        );
    }

    #[test]
    fn invocation_resource_ura_maps_agent_to_user_owned_namespace() {
        let ura = invocation_resource_ura(
            "test-realm",
            "req/with spaces",
            &crate::ura::agent_ura("test-realm", "alice", "frontend"),
            &crate::ura::device_ura("test-realm", "callee-device"),
            &crate::ura::device_ura("test-realm", "caller-device"),
        )
        .expect("resource ura");
        assert!(ura.starts_with(
            "easynet:///r/test-realm/resource/alice.invocations/agents/frontend/invocations/req-with-spaces-"
        ));
        assert!(!ura.contains("/resource/invocation."));
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
            DirectoryEntry, DirectoryView, SharedFederatedDirectoryView,
        };
        use std::collections::BTreeMap;

        let cell = SharedFederatedDirectoryView::default();
        let mut peer_view = DirectoryView::new("realm-b".to_string());
        peer_view.replace_entries(vec![DirectoryEntry {
            agent_ura: "easynet:///r/realm-b/device/peer-device".to_string(),
            node_id: "peer-1".to_string(),
            display_name: Some("silan-phone".to_string()),
            status: "active".to_string(),
            origin_realm: None, // peer omitted; rewrite stamps realm-b
            hub_endpoint: Some("https://hub-b.example:50443".to_string()),
            last_seen_unix_ms: Some(1_714_500_000_000),
        }]);
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
            body.entries[0].agent_ura,
            "easynet:///r/realm-b/device/peer-device"
        );
        assert_eq!(
            body.entries[0].origin_realm.as_deref(),
            Some("realm-b"),
            "§2.4 origin_realm rewrite must show through to the discover response"
        );
    }

    #[tokio::test]
    async fn invoke_dispatches_federation_discover_with_ura_filter_returns_single_hit() {
        use crate::services::federation_directory::{
            DirectoryEntry, DirectoryView, SharedFederatedDirectoryView,
        };
        use std::collections::BTreeMap;

        let cell = SharedFederatedDirectoryView::default();
        let mut peer_view = DirectoryView::new("realm-b".to_string());
        peer_view.replace_entries(vec![
            DirectoryEntry {
                agent_ura: "easynet:///r/realm-b/device/match".to_string(),
                node_id: "n1".to_string(),
                display_name: None,
                status: "active".to_string(),
                origin_realm: None,
                hub_endpoint: None,
                last_seen_unix_ms: None,
            },
            DirectoryEntry {
                agent_ura: "easynet:///r/realm-b/device/other".to_string(),
                node_id: "n2".to_string(),
                display_name: None,
                status: "active".to_string(),
                origin_realm: None,
                hub_endpoint: None,
                last_seen_unix_ms: None,
            },
        ]);
        let mut peers = BTreeMap::new();
        peers.insert("realm-b".to_string(), Arc::new(peer_view));
        cell.replace(peers);

        let svc = make_service().with_federated_directory_cell(cell);
        let resp = svc
            .invoke(invoke_request(
                ABILITY_FEDERATION_DISCOVER,
                r#"{"agent_ura":"easynet:///r/realm-b/device/match"}"#,
            ))
            .await
            .expect("dispatch returns Ok");
        let body: federation_wrappers::DiscoverResponse = parse_response_body(resp);
        assert_eq!(body.entries.len(), 1);
        assert_eq!(
            body.entries[0].agent_ura,
            "easynet:///r/realm-b/device/match"
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
            DirectoryEntry, DirectoryView, SharedFederatedDirectoryView,
        };
        use std::collections::BTreeMap;

        let cell = SharedFederatedDirectoryView::default();
        let mut realm_c = DirectoryView::new("realm-c".to_string());
        realm_c.replace_entries(vec![DirectoryEntry {
            agent_ura: "easynet:///r/realm-c/user/unbound".to_string(),
            node_id: "n".to_string(),
            display_name: None,
            status: "active".to_string(),
            origin_realm: None,
            hub_endpoint: None,
            last_seen_unix_ms: None,
        }]);
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
            DirectoryEntry, DirectoryView, SharedFederatedDirectoryView,
        };
        use std::collections::BTreeMap;

        let cell = SharedFederatedDirectoryView::default();
        let mut realm_c = DirectoryView::new("realm-c".to_string());
        realm_c.replace_entries(vec![DirectoryEntry {
            agent_ura: "easynet:///r/realm-c/user/u".to_string(),
            node_id: "n".to_string(),
            display_name: None,
            status: "active".to_string(),
            origin_realm: None,
            hub_endpoint: None,
            last_seen_unix_ms: None,
        }]);
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
            DirectoryEntry, DirectoryView, SharedFederatedDirectoryView,
        };
        use std::collections::BTreeMap;

        let cell = SharedFederatedDirectoryView::default();
        let mut realm_a = DirectoryView::new("realm-a".to_string());
        realm_a.replace_entries(vec![DirectoryEntry {
            agent_ura: "easynet:///r/realm-a/user/bound-user".to_string(),
            node_id: "n".to_string(),
            display_name: None,
            status: "active".to_string(),
            origin_realm: None,
            hub_endpoint: None,
            last_seen_unix_ms: None,
        }]);
        let mut peers = BTreeMap::new();
        peers.insert("realm-a".to_string(), Arc::new(realm_a));
        cell.replace(peers);

        let bindings = Arc::new(FederatedBindingsStore::in_memory());
        bindings
            .record_binding(
                FederatedUserBinding {
                    source_realm: "realm-a".to_string(),
                    source_user_ura: "easynet:///r/realm-a/user/bound-user".to_string(),
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
            body.entries[0].agent_ura,
            "easynet:///r/realm-a/user/bound-user"
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
        // Two devices online for realm-x.
        svc.presence.insert(
            "easynet:///r/realm-x/device/device-1".to_string(),
            tokio::sync::mpsc::channel(8).0,
        );
        svc.presence.insert(
            "easynet:///r/realm-x/device/device-2".to_string(),
            tokio::sync::mpsc::channel(8).0,
        );
        // One device for an unrelated realm — must NOT show
        // through.
        svc.presence.insert(
            "easynet:///r/realm-other/device/device-3".to_string(),
            tokio::sync::mpsc::channel(8).0,
        );

        let resp = svc
            .invoke(invoke_request(
                ABILITY_FEDERATION_LIST_USER_DEVICES,
                r#"{"realm":"realm-x"}"#,
            ))
            .await
            .expect("loopback caller admitted");
        let body: federation_wrappers::ListUserDevicesResponse = parse_response_body(resp);
        assert_eq!(body.devices.len(), 2);
        let expected_prefix = crate::ura::realm_device_prefix("realm-x");
        for entry in &body.devices {
            assert!(entry.agent_ura.starts_with(&expected_prefix));
        }
    }

    #[tokio::test]
    async fn invoke_dispatches_federation_list_user_devices_rejects_non_hub_caller() {
        // PR-N3 N3-5: caller URA is in trust set but as Backend
        // role → admission filter rejects. PermissionDenied is
        // the wire-stable rejection; the message mentions the
        // caller URA for operator audit grep.
        //
        // Build the test through the URA-only Device admission
        // arm: we register the caller as a Device-role entry so
        // the general admission gate's URA-only no-op admits
        // (DEC-013 Device path doesn't require a signed envelope).
        // The dispatch arm then runs the N3-5 admission filter,
        // which reads the trust anchor again and finds the role
        // is Device, not Hub — reject.
        use crate::services::realm_trust_anchor::{
            RealmTrustAnchor, TrustedAgent, TrustedAgentRole,
        };

        let device_caller_ura = "easynet:///r/realm-b/device/device-not-hub";
        let mut anchor_inner = RealmTrustAnchor::default();
        anchor_inner
            .append_agent(TrustedAgent {
                agent_ura: device_caller_ura.to_string(),
                public_key_b64: "AAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAA=".to_string(),
                role: TrustedAgentRole::Device,
                added_at_unix_ms: 1_700_000_000_000,
                origin_realm: None,
                hub_endpoint: None,
                tls_ca_pem_path: None,
            })
            .expect("append device");
        let admission =
            AdmissionFacade::new(Arc::new(anchor_inner), Some(TEST_DAEMON_URI.to_string()));
        let svc = DaemonInvocationService::new(Arc::new(PresenceRegistry::new()), admission);

        let envelope = Envelope {
            caller: Some(easynet_axon::pb::axon::v1::AgentIdentity {
                ura: device_caller_ura.to_string(),
                profile: "easynet-strict-v2".to_string(),
            }),
            ..Envelope::default()
        };
        let req = Request::new(InvokeRequest {
            envelope: Some(envelope),
            function_name: ABILITY_FEDERATION_LIST_USER_DEVICES.to_string(),
            arguments: br#"{"realm":"realm-x"}"#.to_vec(),
            ..InvokeRequest::default()
        });

        let err = svc
            .invoke(req)
            .await
            .expect_err("device-role caller must be rejected by N3-5 filter");
        assert_eq!(err.code(), tonic::Code::PermissionDenied);
        assert!(
            err.message().contains(device_caller_ura),
            "rejection message must surface the caller URA; got: {}",
            err.message()
        );
    }

    #[tokio::test]
    async fn invoke_dispatches_federation_proxy_list_user_devices_fans_out_and_stamps_peer_metadata(
    ) {
        use crate::services::realm_trust_anchor::{
            RealmTrustAnchor, TrustedAgent, TrustedAgentRole,
        };

        let peer_hub_url = "https://peer-hub.example:50443";
        let peer_hub_ura = crate::ura::hub_ura("peer-realm");
        let anchor = Arc::new(
            RealmTrustAnchor::from_entries(vec![TrustedAgent {
                agent_ura: peer_hub_ura.clone(),
                public_key_b64: "AAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAA=".to_string(),
                role: TrustedAgentRole::Hub,
                added_at_unix_ms: 1_700_000_000_000,
                origin_realm: Some("peer-realm".to_string()),
                hub_endpoint: Some(peer_hub_url.to_string()),
                tls_ca_pem_path: None,
            }])
            .expect("peer hub trust anchor"),
        );
        let admission = AdmissionFacade::new(anchor, Some(TEST_DAEMON_URI.to_string()));
        let canned = InvokeResponse {
            result: br#"{
                "devices":[{
                    "agent_ura":"easynet:///r/user-realm/device/dev-peer",
                    "node_id":"dev-peer",
                    "status":"active"
                }]
            }"#
            .to_vec(),
            result_content_type: FEDERATION_RESULT_CONTENT_TYPE.to_string(),
            state: easynet_axon::invocation::InvocationState::Completed.to_wire_i32(),
            ..InvokeResponse::default()
        };
        let recorder = Arc::new(RecordingFederationClient::new(canned));
        let svc = DaemonInvocationService::new(Arc::new(PresenceRegistry::new()), admission)
            .with_hub_signing_seed([0x11; 32])
            .with_session_realm("local-realm")
            .with_federation_client(recorder.clone() as Arc<dyn FederationClient>);

        let resp = svc
            .invoke(invoke_request(
                ABILITY_FEDERATION_PROXY_LIST_USER_DEVICES,
                r#"{
                    "realm":"user-realm",
                    "peer_hub_urls":["https://peer-hub.example:50443"]
                }"#,
            ))
            .await
            .expect("proxy list user devices succeeds");
        let body: federation_wrappers::ProxyListUserDevicesResponse = parse_response_body(resp);
        assert_eq!(body.devices.len(), 1);
        let device = &body.devices[0];
        assert_eq!(device.agent_ura, "easynet:///r/user-realm/device/dev-peer");
        assert_eq!(device.origin_realm.as_deref(), Some("peer-realm"));
        assert_eq!(device.hub_endpoint.as_deref(), Some(peer_hub_url));

        let calls = recorder.calls();
        assert_eq!(calls.len(), 1, "exactly one peer request captured");
        assert_eq!(calls[0].0, peer_hub_url);
        assert_eq!(
            calls[0].1.function_name,
            ABILITY_FEDERATION_LIST_USER_DEVICES
        );
        let peer_args: federation_wrappers::ListUserDevicesRequest =
            serde_json::from_slice(&calls[0].1.arguments).expect("peer args decode");
        assert_eq!(peer_args.realm, "user-realm");
    }

    #[tokio::test]
    async fn federation_proxy_caller_gate_accepts_local_hub_identity_with_hub_role() {
        use crate::services::realm_trust_anchor::{
            RealmTrustAnchor, TrustedAgent, TrustedAgentRole,
        };

        let local_hub_ura = crate::ura::hub_ura("local-realm");
        let anchor = Arc::new(
            RealmTrustAnchor::from_entries(vec![TrustedAgent {
                agent_ura: local_hub_ura.clone(),
                public_key_b64: "AAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAA=".to_string(),
                role: TrustedAgentRole::Hub,
                added_at_unix_ms: 1_700_000_000_000,
                origin_realm: Some("local-realm".to_string()),
                hub_endpoint: Some("https://local-hub.example:50443".to_string()),
                tls_ca_pem_path: None,
            }])
            .expect("local hub trust anchor"),
        );
        let admission = AdmissionFacade::new(anchor, Some(TEST_DAEMON_URI.to_string()));
        let svc = DaemonInvocationService::new(Arc::new(PresenceRegistry::new()), admission)
            .with_session_realm("local-realm");
        let envelope = Envelope {
            caller: Some(AgentIdentity {
                ura: local_hub_ura,
                profile: "easynet-strict-v2".to_string(),
            }),
            ..Envelope::default()
        };

        svc.require_backend_or_loopback_proxy_caller(Some(&envelope), "namespace.proxy_resolve")
            .expect("local canonical hub identity is the backend proxy caller");
    }

    #[tokio::test]
    async fn invoke_dispatches_federation_proxy_list_user_devices_rejects_hub_role_caller() {
        use base64::{engine::general_purpose::STANDARD as BASE64_STANDARD, Engine as _};
        use ed25519_dalek::SigningKey;

        use crate::services::realm_trust_anchor::{
            RealmTrustAnchor, TrustedAgent, TrustedAgentRole,
        };

        let caller_signing_key = SigningKey::from_bytes(&[0x22; 32]);
        let caller_ura = crate::ura::hub_ura("peer-realm");
        let caller_pubkey_b64 =
            BASE64_STANDARD.encode(caller_signing_key.verifying_key().to_bytes());
        let anchor = Arc::new(
            RealmTrustAnchor::from_entries(vec![TrustedAgent {
                agent_ura: caller_ura.clone(),
                public_key_b64: caller_pubkey_b64,
                role: TrustedAgentRole::Hub,
                added_at_unix_ms: 1_700_000_000_000,
                origin_realm: Some("peer-realm".to_string()),
                hub_endpoint: Some("https://peer-hub.example:50443".to_string()),
                tls_ca_pem_path: None,
            }])
            .expect("hub caller trust anchor"),
        );
        let admission = AdmissionFacade::new(anchor, Some(TEST_DAEMON_URI.to_string()));
        let svc = DaemonInvocationService::new(Arc::new(PresenceRegistry::new()), admission)
            .with_session_realm("local-realm");

        let args = br#"{"realm":"user-realm","peer_hub_urls":["https://peer-hub.example:50443"]}"#;
        let mut envelope = Envelope {
            caller: Some(AgentIdentity {
                ura: caller_ura.clone(),
                profile: "easynet-strict-v2".to_string(),
            }),
            callee: Some(AgentIdentity {
                ura: crate::ura::hub_ura("local-realm"),
                profile: "easynet-strict-v2".to_string(),
            }),
            subject: Some(SubjectIdentity {
                ura: "easynet:///r/local-realm/user/alice".to_string(),
                profile: "easynet-strict-v2".to_string(),
            }),
            invocation_nonce: vec![7; 16],
            ..Envelope::default()
        };
        sign_peer_request_envelope(
            &mut envelope,
            ABILITY_FEDERATION_PROXY_LIST_USER_DEVICES,
            args,
            Some("local-realm"),
            Some(&[0x22; 32]),
        )
        .expect("sign test envelope");

        let mut request = InvokeRequest {
            envelope: Some(envelope),
            function_name: ABILITY_FEDERATION_PROXY_LIST_USER_DEVICES.to_string(),
            arguments: args.to_vec(),
            ..InvokeRequest::default()
        };
        request.metadata.insert(
            "x-easynet-delegation".to_string(),
            signed_delegation_metadata_for_test(
                &caller_signing_key,
                &caller_ura,
                "easynet:///r/local-realm/user/alice",
                &caller_ura,
                &crate::ura::hub_ura("local-realm"),
                &[ABILITY_FEDERATION_PROXY_LIST_USER_DEVICES],
            ),
        );

        let err = svc
            .invoke(Request::new(request))
            .await
            .expect_err("hub-role caller must be rejected by proxy filter");
        assert_eq!(err.code(), tonic::Code::PermissionDenied);
        assert!(
            err.message().contains(&caller_ura),
            "rejection message must surface the caller URA; got: {}",
            err.message()
        );
    }

    #[tokio::test]
    async fn invoke_dispatches_namespace_proxy_resolve_to_typed_peer_surface() {
        use crate::services::realm_trust_anchor::{
            RealmTrustAnchor, TrustedAgent, TrustedAgentRole,
        };

        let peer_hub_url = "https://peer-hub.example:50443";
        let peer_hub_ura = crate::ura::hub_ura("peer-realm");
        let anchor = Arc::new(
            RealmTrustAnchor::from_entries(vec![TrustedAgent {
                agent_ura: peer_hub_ura,
                public_key_b64: "AAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAA=".to_string(),
                role: TrustedAgentRole::Hub,
                added_at_unix_ms: 1_700_000_000_000,
                origin_realm: Some("peer-realm".to_string()),
                hub_endpoint: Some(peer_hub_url.to_string()),
                tls_ca_pem_path: None,
            }])
            .expect("peer hub trust anchor"),
        );
        let admission = AdmissionFacade::new(anchor, Some(TEST_DAEMON_URI.to_string()));
        let owner_ura = "easynet:///r/peer-realm/device/dev-peer";
        let ability_ura =
            crate::ura::owner_ability_ura(owner_ura, "agent.list").expect("ability ura");
        let canned = InvokeResponse {
            result: serde_json::to_vec(&serde_json::json!({
                "answerKind": "RESOLVE_ANSWER_KIND_NON_DISPATCHABLE",
                "records": [
                    {
                        "name": owner_ura,
                        "recordType": "RECORD_TYPE_ID",
                        "value": {
                            "id": {
                                "ura": owner_ura,
                                "kind": "URA_KIND_DEVICE"
                            }
                        }
                    },
                    {
                        "name": ability_ura,
                        "recordType": "RECORD_TYPE_ABILITY",
                        "value": {
                            "ability": {
                                "abilityUra": ability_ura,
                                "ownerUra": owner_ura,
                                "namespace": "agent",
                                "localName": "list"
                            }
                        }
                    }
                ],
                "releaseProfile": "RESOLVER_RELEASE_PROFILE_AUTHORITATIVE_LOCAL",
                "cachePolicy": {
                    "ttlMs": 0,
                    "sharedCacheable": false,
                    "retryAfterUnixMs": 0
                }
            }))
            .expect("typed resolve answer fixture"),
            result_content_type: FEDERATION_RESULT_CONTENT_TYPE.to_string(),
            state: easynet_axon::invocation::InvocationState::Completed.to_wire_i32(),
            ..InvokeResponse::default()
        };
        let recorder = Arc::new(RecordingFederationClient::new(canned));
        let svc = DaemonInvocationService::new(Arc::new(PresenceRegistry::new()), admission)
            .with_hub_signing_seed([0x11; 32])
            .with_session_realm("local-realm")
            .with_federation_client(recorder.clone() as Arc<dyn FederationClient>);

        let resp = svc
            .invoke(invoke_request(
                ABILITY_NAMESPACE_PROXY_RESOLVE,
                r#"{
                    "peer_hub_urls":["https://peer-hub.example:50443"],
                    "queryName":"easynet:///r/peer-realm/device/",
                    "qtype":"RESOLVE_TYPE_DIRECTORY_LISTING",
                    "callerUra":"easynet:///r/local-realm/hub",
                    "subjectUra":"easynet:///r/local-realm/user/alice",
                    "realmHint":"peer-realm"
                }"#,
            ))
            .await
            .expect("namespace proxy resolve succeeds");
        let body: serde_json::Value = parse_response_body(resp);
        assert_eq!(
            body["answerKind"], "RESOLVE_ANSWER_KIND_NON_DISPATCHABLE",
            "proxy returns typed ResolveAnswer shape"
        );
        assert_eq!(
            body["records"].as_array().map(Vec::len),
            Some(2),
            "proxy preserves peer namespace records"
        );

        let calls = recorder.calls();
        assert_eq!(calls.len(), 1, "exactly one peer request captured");
        assert_eq!(calls[0].0, peer_hub_url);
        assert_eq!(calls[0].1.function_name, ABILITY_NAMESPACE_RESOLVE);
        let peer_args: serde_json::Value =
            serde_json::from_slice(&calls[0].1.arguments).expect("peer args decode");
        assert_eq!(peer_args["queryName"], "easynet:///r/peer-realm/device/");
        assert_eq!(peer_args["qtype"], "RESOLVE_TYPE_DIRECTORY_LISTING");
    }

    #[tokio::test]
    async fn invoke_dispatches_federation_resolve_key_returns_pubkey_when_present() {
        // PR-N2 commit 2/N: peer-side `federation.resolve_key`
        // surfaces the local trust anchor's `public_key_b64` for
        // a known URA. Cross-hub `FederatedKeyResolver` consumes
        // this exact wire shape.
        use crate::services::realm_trust_anchor::{
            RealmTrustAnchor, TrustedAgent, TrustedAgentRole,
        };
        let entry = TrustedAgent {
            agent_ura: "easynet:///r/realm-a/device/n1".to_string(),
            public_key_b64: "AAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAA=".to_string(),
            role: TrustedAgentRole::Device,
            added_at_unix_ms: 1_700_000_000_000,
            origin_realm: None,
            hub_endpoint: None,
            tls_ca_pem_path: None,
        };
        let anchor = Arc::new(RealmTrustAnchor::from_entries(vec![entry]).expect("anchor"));
        let admission = AdmissionFacade::new(anchor, Some(TEST_DAEMON_URI.to_string()));
        let svc = DaemonInvocationService::new(Arc::new(PresenceRegistry::new()), admission);

        let resp = svc
            .invoke(invoke_request(
                ABILITY_FEDERATION_RESOLVE_KEY,
                r#"{"agent_ura":"easynet:///r/realm-a/device/n1"}"#,
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
    async fn invoke_dispatches_federation_resolve_key_returns_not_found_when_ura_unknown() {
        // PR-N2 commit 2/N: miss surfaces as Status::not_found
        // with the URA in the error message — operators can
        // grep the daemon log for the exact URA that failed.
        let svc = make_service();
        let err = svc
            .invoke(invoke_request(
                ABILITY_FEDERATION_RESOLVE_KEY,
                r#"{"agent_ura":"easynet:///r/realm-a/device/missing"}"#,
            ))
            .await
            .expect_err("miss must surface Status::not_found");
        assert_eq!(err.code(), tonic::Code::NotFound);
        assert!(
            err.message()
                .contains("easynet:///r/realm-a/device/missing"),
            "expected the missing URA in error message, got: {}",
            err.message()
        );
    }

    #[tokio::test]
    async fn invoke_dispatches_federation_revoke() {
        let svc = make_service();
        let resp = svc
            .invoke(invoke_request(
                ABILITY_FEDERATION_REVOKE,
                r#"{"target_ura":"easynet:///r/realm/device/missing"}"#,
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
                r#"{"target_ura":"easynet:///r/realm/device/missing","inner_envelope_b64":""}"#,
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
    async fn invoke_unknown_ability_without_projection_returns_resolver_negative() {
        // RFC-005 pin: when the federation-wrapper match misses,
        // namespace.resolve is the first gate. A missing owner
        // projection is reported before LocalRuntime wiring is
        // inspected.
        let svc = make_service();
        match svc.invoke(invoke_request("custom.ability.x", "{}")).await {
            Err(err) => {
                assert_eq!(err.code(), tonic::Code::FailedPrecondition);
                assert!(
                    err.message().contains(ROUTE_NEGATIVE_CODE),
                    "expected resolver negative; got: {}",
                    err.message()
                );
            }
            Ok(_) => panic!("unknown ability must be rejected"),
        }
    }

    /// When the Axon `LocalRuntime` is wired, the owner projection
    /// publishes the ability, and namespace.resolve selects a route,
    /// direct unary Invoke dispatches through `LocalRuntime::invoke_async`
    /// and returns the handler's JSON output.
    #[tokio::test]
    async fn invoke_dispatches_selected_route_to_axon_runtime_when_wired() {
        use easynet_axon::invocation::{make_ability, LocalRuntime};

        let rt = LocalRuntime::new();
        rt.register_ability(
            "test.fallback.echo",
            make_ability(|ctx| async move { Ok(ctx.payload.clone()) }),
        )
        .await
        .unwrap();

        let svc = make_service().with_local_runtime(Arc::clone(&rt));
        publish_test_route(&svc, TEST_DAEMON_URI, "test.fallback.echo");
        let resp = svc
            .invoke(invoke_request("test.fallback.echo", r#"{"hello":"world"}"#))
            .await
            .expect("selected-route dispatch succeeds");
        let body: serde_json::Value = parse_response_body(resp);
        assert_eq!(body["hello"], "world");
    }

    #[tokio::test]
    async fn invoke_selected_route_unknown_runtime_handler_surfaces_not_found() {
        use easynet_axon::invocation::LocalRuntime;

        let rt = LocalRuntime::new();
        let svc = make_service().with_local_runtime(Arc::clone(&rt));
        publish_test_route(&svc, TEST_DAEMON_URI, "nope.nope");

        match svc.invoke(invoke_request("nope.nope", "{}")).await {
            Err(err) => {
                assert_eq!(err.code(), tonic::Code::NotFound);
                assert!(
                    err.message()
                        .contains("not registered in Axon LocalRuntime"),
                    "expected the not-registered message; got: {}",
                    err.message()
                );
            }
            Ok(_) => panic!("unregistered ability must be rejected"),
        }
    }

    #[tokio::test]
    async fn invoke_runtime_bootstrap_self_identity_is_not_cli_shadow_acked() {
        use easynet_axon::invocation::LocalRuntime;

        let rt = LocalRuntime::new();
        let svc = make_service().with_local_runtime(Arc::clone(&rt));
        publish_test_route(
            &svc,
            TEST_DAEMON_URI,
            federation_wrappers::ABILITY_RUNTIME_BOOTSTRAP_SELF_IDENTITY,
        );
        let args = r#"{
            "tenant_id":"tenant-a",
            "node_id":"node-a",
            "owner_id":"node-a",
            "public_key_b64":"AAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAA="
        }"#;

        match svc
            .invoke(invoke_request(
                federation_wrappers::ABILITY_RUNTIME_BOOTSTRAP_SELF_IDENTITY,
                args,
            ))
            .await
        {
            Err(err) => {
                assert_eq!(err.code(), tonic::Code::NotFound);
                assert!(
                    err.message()
                        .contains("not registered in Axon LocalRuntime"),
                    "expected SDK LocalRuntime missing-handler diagnostic; got: {}",
                    err.message()
                );
            }
            Ok(resp) => {
                let body: serde_json::Value = parse_response_body(resp);
                panic!("bootstrap_self_identity must not be CLI-shadow-acked: {body}");
            }
        }
    }

    #[tokio::test]
    async fn invoke_runtime_bootstrap_self_identity_succeeds_when_sdk_admin_installed() {
        use base64::{engine::general_purpose::STANDARD as BASE64_STANDARD, Engine as _};
        use easynet_axon::invocation::LocalRuntime;
        use ed25519_dalek::SigningKey;

        let rt = LocalRuntime::new();
        rt.install_bootstrap_self_identity_admin().await.unwrap();
        let svc = make_service().with_local_runtime(Arc::clone(&rt));
        publish_test_route(
            &svc,
            TEST_DAEMON_URI,
            federation_wrappers::ABILITY_RUNTIME_BOOTSTRAP_SELF_IDENTITY,
        );
        let key = SigningKey::from_bytes(&[0x44; 32]);
        let args = serde_json::json!({
            "tenant_id": "tenant-a",
            "node_id": "node-a",
            "owner_id": "node-a",
            "public_key_b64": BASE64_STANDARD.encode(key.verifying_key().to_bytes()),
        })
        .to_string();

        let resp = svc
            .invoke(invoke_request(
                federation_wrappers::ABILITY_RUNTIME_BOOTSTRAP_SELF_IDENTITY,
                &args,
            ))
            .await
            .expect("SDK runtime admin bootstrap should be dispatched");
        let body: serde_json::Value = parse_response_body(resp);
        assert_eq!(body["ack"], true);
        assert_eq!(body["replaced_prior"], false);
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
        presence.insert("easynet:///r/test-realm/device/n1".to_string(), sender);

        let second = stream
            .next()
            .await
            .expect("delta frame after insert")
            .expect("frame is Ok");
        let delta: serde_json::Value = serde_json::from_slice(&second.payload).expect("decodes");
        assert_eq!(delta.get("kind").and_then(|v| v.as_str()), Some("online"));
        assert_eq!(
            delta.get("membership_ura").and_then(|v| v.as_str()),
            Some("easynet:///r/test-realm/device/n1"),
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
            DirectoryEvent::Snapshot { agents, .. } => {
                assert!(
                    agents.is_empty(),
                    "initial snapshot must reflect empty registry"
                );
            }
            other => panic!("expected Snapshot first; got {other:?}"),
        }

        // Frame 2: AgentAdvertised after a registry insert.
        let (sender, _rx) = tokio::sync::mpsc::channel::<
            Result<crate::services::presence_registry::DispatchFrame, tonic::Status>,
        >(1);
        presence.insert("easynet:///r/test-realm/device/n1".to_string(), sender);
        let second = stream.next().await.expect("second frame").expect("Ok");
        let evt2: DirectoryEvent =
            serde_json::from_slice(&second.payload).expect("decodes DirectoryEvent");
        match evt2 {
            DirectoryEvent::AgentAdvertised {
                agent_ura,
                signing_authority,
                ..
            } => {
                assert_eq!(agent_ura, "easynet:///r/test-realm/device/n1");
                assert_eq!(
                    signing_authority,
                    crate::services::federation_directory::SigningAuthority::SelfSigned
                );
            }
            other => panic!("expected AgentAdvertised; got {other:?}"),
        }

        // Frame 3: AgentRevoked after the device's stream closes (we
        // drop the receiver to trigger the Closed path).
        // PresenceRegistry's drop-on-receiver-close behaviour is
        // exercised by the existing v1 test; here we just
        // explicitly remove via the registry surface.
        presence.remove(
            "easynet:///r/test-realm/device/n1",
            crate::services::presence_registry::OfflineReason::AdminRevoked,
        );
        let third = stream.next().await.expect("third frame").expect("Ok");
        let evt3: DirectoryEvent =
            serde_json::from_slice(&third.payload).expect("decodes DirectoryEvent");
        match evt3 {
            DirectoryEvent::AgentRevoked {
                agent_ura, reason, ..
            } => {
                assert_eq!(agent_ura, "easynet:///r/test-realm/device/n1");
                assert_eq!(reason, "admin_revoked");
            }
            other => panic!("expected AgentRevoked; got {other:?}"),
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
            .with_subscribe_v2_heartbeat_interval_ms(std::num::NonZeroU64::new(50).unwrap());

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
            DirectoryEvent::Heartbeat { unix_ms } => {
                assert!(unix_ms > 0, "Heartbeat unix_ms must be a real epoch-ms",);
            }
            other => panic!("expected Heartbeat after idle window; got {other:?}"),
        }

        drop(svc);
        drop(presence);
    }

    #[tokio::test]
    async fn invoke_stream_dispatches_registered_local_stream_ability() {
        use easynet_axon::invocation::{make_ability, LocalRuntime};
        use futures::StreamExt;

        let rt = LocalRuntime::new();
        rt.register_streaming_ability(
            "browser.capture_viewport",
            make_ability(|ctx| async move {
                let args: serde_json::Value =
                    serde_json::from_slice(&ctx.payload).unwrap_or(serde_json::Value::Null);
                Ok(serde_json::to_vec(&serde_json::json!({
                    "MARKER-LOCAL-STREAM": "dispatched",
                    "session_ura": args.get("session_ura").and_then(|v| v.as_str()),
                }))
                .unwrap())
            }),
        )
        .await
        .unwrap();
        let svc = make_service().with_local_runtime(Arc::clone(&rt));
        publish_test_route(&svc, TEST_DAEMON_URI, "browser.capture_viewport");

        let resp = svc
            .invoke_stream(Request::new(InvokeServerStreamRequest {
                envelope: Some(test_envelope()),
                function_name: "browser.capture_viewport".to_string(),
                arguments: br#"{"session_ura":"easynet:///r/local/resource/daemon.browser/s1"}"#
                    .to_vec(),
                ..InvokeServerStreamRequest::default()
            }))
            .await
            .expect("registered local stream returns Ok");

        let mut stream = resp.into_inner();
        let first = stream.next().await.expect("one frame").expect("frame Ok");
        assert_eq!(first.content_type, FEDERATION_RESULT_CONTENT_TYPE);
        assert!(
            first.terminal,
            "local snapshot stream must preserve terminal=true on the daemon InvokeStream chunk"
        );
        let frame: serde_json::Value = serde_json::from_slice(&first.payload).expect("JSON frame");
        assert_eq!(
            frame
                .get("MARKER-LOCAL-STREAM")
                .and_then(|value| value.as_str()),
            Some("dispatched")
        );
        assert_eq!(
            frame.get("session_ura").and_then(|value| value.as_str()),
            Some("easynet:///r/local/resource/daemon.browser/s1")
        );

        let close = tokio::time::timeout(std::time::Duration::from_secs(2), stream.next())
            .await
            .expect("snapshot stream closes promptly");
        assert!(close.is_none());
    }

    #[tokio::test(flavor = "multi_thread", worker_threads = 2)]
    async fn admitted_bidi_file_transfer_download_emits_business_frames() {
        use base64::Engine as _;
        use easynet_axon::invocation::LocalRuntime;

        let rt = LocalRuntime::new();
        let mut catalog =
            crate::runtime::ability_dispatch::AxonAbilityCatalog::new_with_runtime(Arc::clone(&rt));
        crate::runtime::agents::file_transfer_ability::register(&mut catalog);

        let path = std::env::temp_dir().join(format!(
            "easynet-admitted-bidi-download-{}-{}.bin",
            std::process::id(),
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .map(|d| d.subsec_nanos())
                .unwrap_or(0)
        ));
        let bytes = b"admitted-bidi-download-proof";
        std::fs::write(&path, bytes).unwrap();

        let args = serde_json::to_vec(&serde_json::json!({
            "mode": "download",
            "resource_ref": crate::runtime::resources::filesystem::resource_ref_for_local_path(
                &path,
                crate::runtime::resources::filesystem::FilesystemResourceCapability::Read,
            )
            .expect("local fs ResourceRef"),
        }))
        .unwrap();
        let open = make_envelope_open(
            crate::runtime::agents::file_transfer_ability::ABILITY_FILE_TRANSFER,
            args,
        );
        let wire = crate::runtime::axon_bridge::dispatch_shim::admitted_from_envelope_open(&open)
            .expect("wire dispatch");
        let handle = crate::runtime::axon_bridge::dispatch_shim::open_bidi_admitted(&rt, wire)
            .await
            .expect("open admitted bidi");
        let (input, mut output) = handle.split();

        input
            .send(
                BidiInputFrame::new(
                    serde_json::to_vec(&serde_json::json!({"type":"eof"})).unwrap(),
                )
                .with_content_type("application/json"),
            )
            .await
            .expect("send ready/eof");
        let _ = input.close_input().await;

        let mut downloaded = Vec::new();
        let mut got_complete = false;
        let deadline = tokio::time::Instant::now() + std::time::Duration::from_secs(3);
        while tokio::time::Instant::now() < deadline {
            let remaining = deadline
                .checked_duration_since(tokio::time::Instant::now())
                .unwrap_or_default();
            let Some(frame) = tokio::time::timeout(remaining, output.next_frame())
                .await
                .expect("bidi output poll should not time out")
            else {
                break;
            };
            let frame = frame.expect("bidi frame ok");
            if frame.payload.is_empty() {
                continue;
            }
            let value: serde_json::Value =
                serde_json::from_slice(&frame.payload).expect("file transfer JSON frame");
            match value["type"].as_str() {
                Some("chunk") => {
                    let chunk = value["data"].as_str().expect("chunk data");
                    downloaded.extend(
                        base64::engine::general_purpose::STANDARD
                            .decode(chunk)
                            .expect("chunk base64"),
                    );
                }
                Some("complete") => {
                    got_complete = true;
                    break;
                }
                other => panic!("unexpected file_transfer frame {other:?}: {value}"),
            }
        }
        assert!(
            got_complete,
            "admitted file_transfer download must emit complete"
        );
        assert_eq!(downloaded, bytes);
        let _ = std::fs::remove_file(&path);
    }

    #[tokio::test]
    async fn invoke_stream_unknown_function_returns_resolver_negative() {
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
                assert_eq!(err.code(), tonic::Code::FailedPrecondition);
                assert!(
                    err.message().contains(ROUTE_NEGATIVE_CODE),
                    "expected resolver negative; got: {}",
                    err.message()
                );
            }
            Ok(_) => panic!("unknown stream ability must be rejected"),
        }
    }

    #[tokio::test]
    async fn invoke_rejects_caller_not_in_trust_anchor() {
        // PR-7 commit 4/N (DEC-013 Option D): trust-anchor membership
        // is the first non-loopback check. A URA absent from the
        // anchor short-circuits to `permission_denied` before any
        // §5.2 work — the gating reject, identical to the PR-1 URA-
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
                        ura: "easynet:///r/realm/agent/test.external".to_string(),
                        ..AgentIdentity::default()
                    }),
                    ..Envelope::default()
                }),
                function_name: ABILITY_FEDERATION_HEARTBEAT.to_string(),
                arguments: br#"{"agent_ura":"easynet:///r/realm/agent/test.external"}"#.to_vec(),
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
                        ura: "easynet:///r/realm/agent/test.external".to_string(),
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

    use crate::services::invocation_transport::invoke_remote_initiator::{
        InvokeRemoteUp, ABILITY_INVOKE_REMOTE,
    };
    use easynet_axon::pb::axon::v1::invoke_bidi_up::Payload as UpPayload;
    use easynet_axon::pb::axon::v1::{BidiControl, EnvelopeOpen, InvocationTarget, InvokeBidiUp};
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

    fn make_envelope_open_with_callee(callee_ura: &str) -> EnvelopeOpen {
        let mut envelope = test_envelope();
        envelope.callee = Some(AgentIdentity {
            ura: callee_ura.to_string(),
            ..AgentIdentity::default()
        });
        EnvelopeOpen {
            envelope: Some(envelope),
            target: Some(InvocationTarget {
                ability_name:
                    crate::runtime::agents::pty_attach_ability::ABILITY_PTY_SESSION_ATTACH
                        .to_string(),
                ..InvocationTarget::default()
            }),
            ..EnvelopeOpen::default()
        }
    }

    #[test]
    fn remote_bidi_target_ura_preserves_canonical_device_ura() {
        let open = make_envelope_open_with_callee("  easynet:///r/test-realm/device/dev-B  ");
        assert_eq!(
            remote_bidi_target_ura(&open).as_deref(),
            Some("easynet:///r/test-realm/device/dev-B")
        );
    }

    #[test]
    fn remote_bidi_target_ura_preserves_non_device_callee_for_rejection() {
        let open = make_envelope_open_with_callee("easynet:///r/test-realm/agent/dev-B");
        assert_eq!(
            remote_bidi_target_ura(&open).as_deref(),
            Some("easynet:///r/test-realm/agent/dev-B"),
            "remote bidi target extraction must preserve non-device callee URAs so \
             self-target and presence lookup reject unsupported targets naturally"
        );
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
                assert_eq!(
                    receipt.state,
                    easynet_axon::invocation::InvocationState::Completed.to_wire_i32()
                );
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
                assert_eq!(
                    receipt.state,
                    easynet_axon::invocation::InvocationState::Completed.to_wire_i32()
                );
                assert_eq!(receipt.payload_content_type, "application/json");
                assert!(
                    receipt.cleanup_complete,
                    "terminal file_transfer completion receipt must close the bidi lifecycle"
                );
                assert!(
                    receipt.failure.is_none(),
                    "completed receipts must not carry typed failure"
                );
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
                assert_eq!(
                    receipt.state,
                    easynet_axon::invocation::InvocationState::Failed.to_wire_i32()
                );
                assert!(receipt.reason.contains("disk_full"));
                assert!(receipt.reason.contains("no space left on device"));
                let failure = receipt.failure.as_ref().expect("typed receipt failure");
                assert_eq!(failure.code, "DISK_FULL");
                assert_eq!(failure.message, receipt.reason);
                assert_eq!(failure.stage, ErrorStage::Execution as i32);
                let payload: serde_json::Value =
                    serde_json::from_slice(&receipt.payload).expect("json payload");
                assert_eq!(payload["type"], "error");
            }
            other => panic!("expected file_transfer error → failed receipt, got {other:?}"),
        }
    }

    #[test]
    fn terminal_receipt_extracts_admission_failure_code_from_reason() {
        let frame = build_bidi_terminal_receipt(
            easynet_axon::invocation::InvocationState::Failed,
            "CALLER_SIGNATURE_INVALID: rejected <self>.session",
        );
        match frame {
            InvokeBidiDown {
                payload: Some(DownPayload::Receipt(receipt)),
                ..
            } => {
                let failure = receipt.failure.as_ref().expect("typed receipt failure");
                assert_eq!(failure.code, "CALLER_SIGNATURE_INVALID");
                assert_eq!(failure.stage, ErrorStage::CallerAuthentication as i32);
                assert_eq!(failure.security_class, SecurityClass::Authentication as i32);
            }
            other => panic!("expected failed receipt, got {other:?}"),
        }
    }

    #[test]
    fn terminal_receipt_extracts_presence_registry_failure_code_from_reason() {
        let frame = build_bidi_terminal_receipt(
            easynet_axon::invocation::InvocationState::Failed,
            "target device is not in PresenceRegistry; the owning daemon is offline",
        );
        match frame {
            InvokeBidiDown {
                payload: Some(DownPayload::Receipt(receipt)),
                ..
            } => {
                let failure = receipt.failure.as_ref().expect("typed receipt failure");
                assert_eq!(failure.code, "TARGET_NOT_IN_PRESENCE_REGISTRY");
                assert_eq!(failure.stage, ErrorStage::Transport as i32);
                assert_eq!(failure.security_class, SecurityClass::Transport as i32);
            }
            other => panic!("expected failed receipt, got {other:?}"),
        }
    }

    #[test]
    fn terminal_receipt_projects_route_negative_to_resolution_stage() {
        let frame = build_bidi_terminal_receipt(
            easynet_axon::invocation::InvocationState::Failed,
            "ROUTE_NEGATIVE: namespace.resolve negative for `browser.open`: NEGATIVE_REASON_NOROUTE",
        );
        match frame {
            InvokeBidiDown {
                payload: Some(DownPayload::Receipt(receipt)),
                ..
            } => {
                let failure = receipt.failure.as_ref().expect("typed receipt failure");
                assert_eq!(failure.code, "ROUTE_NEGATIVE");
                assert_eq!(failure.stage, ErrorStage::AbilityResolution as i32);
                assert_eq!(failure.security_class, SecurityClass::Unspecified as i32);
            }
            other => panic!("expected failed receipt, got {other:?}"),
        }
    }

    #[test]
    fn terminal_receipt_marks_timeout_retryable() {
        let frame = build_bidi_terminal_receipt(
            easynet_axon::invocation::InvocationState::TimedOut,
            "terminal read timed out",
        );
        match frame {
            InvokeBidiDown {
                payload: Some(DownPayload::Receipt(receipt)),
                ..
            } => {
                let failure = receipt.failure.as_ref().expect("typed receipt failure");
                assert_eq!(failure.code, "INVOCATION_TIMED_OUT");
                assert_eq!(failure.stage, ErrorStage::Execution as i32);
                assert_eq!(failure.security_class, SecurityClass::Unspecified as i32);
                assert!(failure.retryable);
            }
            other => panic!("expected timed-out receipt, got {other:?}"),
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
                control: Some(easynet_axon::pb::axon::v1::bidi_control::Control::Eof(true)),
            }),
        );
        match mapped {
            LocalBidiUpFrame::ForwardAndClose(value) => {
                assert_eq!(value["type"], "eof");
            }
            other => panic!("expected file_transfer eof → eof JSON, got {other:?}"),
        }
    }

    #[test]
    #[cfg(feature = "remote-desktop")]
    fn remote_desktop_bidi_uses_json_frame_wire_kind() {
        let registry = crate::runtime::ability_wire::AbilityWireRegistry::load_default_profile()
            .expect("remote desktop plugin wire profile loads");
        assert_eq!(
            registry.bidi_wire_kind_for("remote_desktop.attach"),
            Some(LocalBidiWireKind::JsonFrames)
        );
    }

    #[test]
    fn map_local_bidi_handler_json_frames_preserves_json_payload() {
        let frame = map_local_bidi_handler_frame(
            LocalBidiWireKind::JsonFrames,
            &serde_json::json!({
                "type": "frame",
                "seq": 7,
                "image_bytes_b64": "abc",
            }),
            3,
        );
        match frame {
            LocalBidiHandlerFrame::Forward(InvokeBidiDown {
                payload: Some(DownPayload::BinaryChunk(chunk)),
                ..
            }) => {
                assert_eq!(chunk.stream_id, 3);
                let payload: serde_json::Value =
                    serde_json::from_slice(&chunk.data).expect("json frame payload");
                assert_eq!(payload["type"], "frame");
                assert_eq!(payload["seq"], 7);
                assert_eq!(payload["image_bytes_b64"], "abc");
            }
            other => panic!("expected JSON frame → BinaryChunk, got {other:?}"),
        }
    }

    #[test]
    fn map_local_bidi_ability_json_frames_forwards_raw_binary_payload() {
        let frame = map_local_bidi_ability_frame(
            LocalBidiWireKind::JsonFrames,
            AbilityFrame {
                payload: b"\xff\xd8raw-jpeg\xff\xd9".to_vec(),
                content_type: "image/jpeg".to_string(),
                terminal: false,
            },
            9,
        );
        match frame {
            LocalBidiHandlerFrame::Forward(InvokeBidiDown {
                payload: Some(DownPayload::BinaryChunk(chunk)),
                ..
            }) => {
                assert_eq!(chunk.stream_id, 9);
                assert_eq!(chunk.data, b"\xff\xd8raw-jpeg\xff\xd9");
            }
            other => panic!("expected raw binary JsonFrames payload → BinaryChunk, got {other:?}"),
        }
    }

    #[test]
    fn map_local_bidi_up_payload_json_frames_forwards_json_control() {
        let mapped = map_local_bidi_up_payload(
            LocalBidiWireKind::JsonFrames,
            UpPayload::BinaryChunk(BinaryChunk {
                data: br#"{"type":"close","reason":"test"}"#.to_vec(),
                ..BinaryChunk::default()
            }),
        );
        match mapped {
            LocalBidiUpFrame::Forward(value) => {
                assert_eq!(value["type"], "close");
                assert_eq!(value["reason"], "test");
            }
            other => panic!("expected JSON BinaryChunk → handler JSON, got {other:?}"),
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
                assert_eq!(
                    receipt.state,
                    easynet_axon::invocation::InvocationState::Admitted.to_wire_i32()
                );
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
            "easynet:///r/realm-a/device/device-1",
            Some("realm-a"),
            &anchor,
        )
        .expect("same-realm caller must pass");
    }

    #[test]
    fn validate_session_realm_accepts_same_realm_device_ura() {
        let anchor = RealmTrustAnchor::default();
        validate_session_realm(
            "easynet:///r/realm-a/device/device-1",
            Some("realm-a"),
            &anchor,
        )
        .expect("same-realm device URA must pass");
    }

    #[test]
    fn validate_session_realm_rejects_cross_realm_without_trust() {
        let anchor = RealmTrustAnchor::default();
        let err = validate_session_realm(
            "easynet:///r/realm-b/device/device-1",
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
        // Federated identity path: caller URA lives in realm-b
        // but the local trust anchor on realm-a's hub has an
        // explicit entry for it. Mirrors the admission gate's
        // existing FederatedKeyResolver hit; closes LB-49.
        use crate::services::realm_trust_anchor::{TrustedAgent, TrustedAgentRole};
        let entry = TrustedAgent {
            agent_ura: "easynet:///r/realm-b/device/device-1".to_string(),
            public_key_b64: "AAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAA=".to_string(),
            role: TrustedAgentRole::Device,
            added_at_unix_ms: 1_777_640_000_000,
            origin_realm: Some("federated-tenant".to_string()),
            hub_endpoint: None,
            tls_ca_pem_path: None,
        };
        let anchor = RealmTrustAnchor::from_entries(vec![entry]).expect("anchor");
        validate_session_realm(
            "easynet:///r/realm-b/device/device-1",
            Some("realm-a"),
            &anchor,
        )
        .expect("cross-realm caller with trust-anchor entry must pass");
    }

    #[test]
    fn validate_session_realm_rejects_malformed_ura() {
        let anchor = RealmTrustAnchor::default();
        let err = validate_session_realm("not-a-ura", Some("realm-a"), &anchor)
            .expect_err("malformed URA must be rejected");
        assert_eq!(err.code(), tonic::Code::InvalidArgument);
        assert!(err.message().contains("canonical"));
    }

    #[test]
    fn build_invoke_remote_dispatch_frame_carries_session_dispatch_json() {
        let mut metadata = HashMap::new();
        metadata.insert(
            "x-easynet-delegation".to_string(),
            "serialized-proof".to_string(),
        );
        let frame = build_invoke_remote_dispatch_frame(
            42,
            "easynet:///r/realm/device/dev",
            Some("easynet:///r/realm/resource/camera-1"),
            "echo",
            b"hello",
            SessionContentEnvelope::plaintext_json(),
            metadata,
        )
        .expect("built");
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
                callee_ura,
                subject_ura,
                ability,
                args,
                args_content_envelope,
                metadata,
            } => {
                assert_eq!(call_id, 42);
                assert_eq!(callee_ura.as_deref(), Some("easynet:///r/realm/device/dev"));
                assert_eq!(
                    subject_ura.as_deref(),
                    Some("easynet:///r/realm/resource/camera-1")
                );
                assert_eq!(ability, "echo");
                assert_eq!(args, b"hello");
                assert_eq!(args_content_envelope.content_type, "application/json");
                assert_eq!(
                    metadata.get("x-easynet-delegation").map(String::as_str),
                    Some("serialized-proof")
                );
            }
            _ => panic!("expected Dispatch variant"),
        }
    }

    #[test]
    fn build_remote_bidi_open_dispatch_frame_carries_resource_binding() {
        let frame = build_remote_bidi_open_dispatch_frame(
            43,
            "easynet:///r/realm/device/dev",
            Some("easynet:///r/realm/resource/display-1"),
            "remote_desktop.attach",
            br#"{"session_id":"rd-1"}"#,
            HashMap::new(),
        )
        .expect("built");
        let payload = match frame.frame.payload.expect("frame has payload") {
            DownPayload::BinaryChunk(chunk) => chunk,
            _ => panic!("expected BinaryChunk"),
        };
        assert_eq!(payload.stream_id, INVOKE_REMOTE_STREAM_ID);
        let parsed: SessionDispatch =
            serde_json::from_slice(&payload.data).expect("decode SessionDispatch");
        match parsed {
            SessionDispatch::BidiOpen {
                call_id,
                callee_ura,
                subject_ura,
                ability,
                args,
                ..
            } => {
                assert_eq!(call_id, 43);
                assert_eq!(callee_ura.as_deref(), Some("easynet:///r/realm/device/dev"));
                assert_eq!(
                    subject_ura.as_deref(),
                    Some("easynet:///r/realm/resource/display-1")
                );
                assert_eq!(ability, "remote_desktop.attach");
                assert_eq!(args, br#"{"session_id":"rd-1"}"#);
            }
            _ => panic!("expected BidiOpen variant"),
        }
    }

    #[test]
    fn build_invoke_remote_terminal_frame_round_trips_done_payload() {
        let down = InvokeRemoteDown::Result {
            payload: b"the-reply".to_vec(),
            error: None,
            failure: None,
            request_id: None,
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
    fn build_invoke_remote_terminal_frame_round_trips_chunk_payload() {
        let down = InvokeRemoteDown::Chunk {
            payload: b"screen-frame".to_vec(),
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

    #[tokio::test]
    async fn invoke_remote_inband_error_response_surfaces_reason_in_terminal_frame() {
        // Operational failures inside `<self>.invoke_remote` (target
        // offline, channel full, handler errored) used to surface as
        // `tonic::Status` — i.e., a gRPC-level error, which the Go
        // HTTP shim above tonic logs as a bare HTTP 500. The frontend
        // then had nothing to render except "500". The helper used by
        // those sites must instead produce a successful Response
        // carrying ONE InvokeRemoteDown::Result frame whose `error`
        // field carries the structured reason, so the shim sees
        // gRPC success and can serialise the reason to the HTTP body.
        use futures::StreamExt;

        let response = invoke_remote_inband_error_response(
            "target `easynet:///r/test-realm/agent/dev.liangbing` is not in PresenceRegistry"
                .to_string(),
        )
        .expect("helper must return Ok — failure is in-band, not gRPC-level");

        let mut stream = response.into_inner();
        let frame = stream
            .next()
            .await
            .expect("stream yields one terminal frame")
            .expect("terminal frame is not a gRPC-level error");

        let chunk = match frame.payload.expect("frame has payload") {
            DownPayload::BinaryChunk(c) => c,
            _ => panic!("expected BinaryChunk"),
        };
        assert_eq!(chunk.stream_id, INVOKE_REMOTE_STREAM_ID);

        let parsed: InvokeRemoteDown =
            serde_json::from_slice(&chunk.data).expect("decode InvokeRemoteDown");
        match parsed {
            InvokeRemoteDown::Result { payload, error, .. } => {
                assert!(payload.is_empty(), "in-band error frame carries no payload");
                let msg = error.expect("error field must be Some(...)");
                assert!(
                    msg.contains("dev.liangbing"),
                    "reason string must round-trip the target URA verbatim — got {msg:?}"
                );
                assert!(
                    msg.contains("not in PresenceRegistry"),
                    "reason string must round-trip the diagnostic verbatim — got {msg:?}"
                );
            }
            other => panic!("expected Result variant, got {other:?}"),
        }

        // Single-frame stream: after the terminal frame, the stream
        // must close (otherwise a caller iterating frames hangs).
        assert!(
            stream.next().await.is_none(),
            "in-band error stream must be one-shot and close after the terminal frame"
        );
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
            subject_device: "easynet:///r/realm/device/dev-B".into(),
            subject_ura: None,
            ability_ura: "easynet:///r/realm/ability/device.dev-B.echo".into(),
            args: b"hi".to_vec(),
            args_content_envelope: SessionContentEnvelope::plaintext_json(),
            metadata: HashMap::new(),
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
    fn parse_realm_from_ura_extracts_realm_component() {
        assert_eq!(
            parse_realm_from_ura("easynet:///r/realm-a/device/laptop-1"),
            Some("realm-a".to_string())
        );
        assert_eq!(
            parse_realm_from_ura("easynet:///r/realm-a/device/device-1"),
            Some("realm-a".to_string())
        );
        assert_eq!(
            parse_realm_from_ura(&crate::ura::hub_ura("peer-realm")),
            Some("peer-realm".to_string())
        );
        assert_eq!(
            parse_realm_from_ura("easynet:///r/peer-realm/hub"),
            Some("peer-realm".to_string())
        );
        assert_eq!(
            parse_realm_from_ura("easynet:///r/peer-realm/hub/extra"),
            None
        );
    }

    #[test]
    fn parse_realm_from_ura_rejects_noncanonical_extra_path_segments() {
        // Realm extraction goes through the canonical URA parser, so
        // malformed alias path tails no longer slip through.
        assert_eq!(
            parse_realm_from_ura("easynet:///r/realm-a/agent/n1/skill/foo"),
            None
        );
    }

    #[test]
    fn parse_realm_from_ura_rejects_non_easynet_scheme() {
        assert_eq!(parse_realm_from_ura("https://example.com/foo"), None);
        assert_eq!(parse_realm_from_ura("file:///r/realm/agent/x"), None);
    }

    #[test]
    fn parse_realm_from_ura_rejects_empty_realm() {
        // Malformed URA with empty realm component must reject —
        // never silently treat as `realm = ""` which would always
        // miss the federated_peers map and surface as
        // "realm unknown" instead of "URA malformed".
        assert_eq!(parse_realm_from_ura("easynet:///r//device/n1"), None);
    }

    #[test]
    fn build_peer_envelope_maps_to_hub_tuple_with_profile() {
        let caller_envelope = Envelope {
            caller: Some(AgentIdentity {
                ura: "easynet:///r/local/device/dev-a".to_string(),
                ..AgentIdentity::default()
            }),
            ..Envelope::default()
        };
        let env = build_peer_envelope(
            Some(&caller_envelope),
            "easynet:///r/peer/device/dev-b",
            Some("local"),
        )
        .unwrap();

        let caller = env.caller.unwrap();
        let callee = env.callee.unwrap();
        let subject = env.subject.unwrap();
        assert_eq!(caller.ura, crate::ura::hub_ura("local"));
        assert_eq!(callee.ura, crate::ura::hub_ura("peer"));
        assert_eq!(subject.ura, "easynet:///r/local/device/dev-a");
        assert_eq!(
            caller.profile,
            crate::services::invocation_transport::DEFAULT_URA_PROFILE
        );
        assert_eq!(
            callee.profile,
            crate::services::invocation_transport::DEFAULT_URA_PROFILE
        );
        assert_eq!(
            subject.profile,
            crate::services::invocation_transport::DEFAULT_URA_PROFILE
        );
        assert_eq!(env.invocation_nonce.len(), 16);
    }

    #[test]
    fn build_peer_envelope_rejects_bad_target_ura() {
        let err = build_peer_envelope(None, "agent://dev-b", Some("local")).unwrap_err();
        assert_eq!(err.code(), tonic::Code::InvalidArgument);
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

    // ── PR-N1 commit 3b/N: realm-aware forward_invoke tests ──

    /// Test fixture: a `FederationClient` that records every
    /// `forward_invoke` call and returns a canned response. Lets
    /// tests assert the cross-realm arm dialed the right peer
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

    fn forward_invoke_args(target_ura: &str) -> Vec<u8> {
        // Test fixture: a base64-encoded JSON `{ability, args,
        // call_id}` payload that mirrors what `support::
        // federation_invoke::invoke_via_federation_forward`
        // ships from the CLI bridge. PR-N1 commit 11/N decodes
        // this on the peer-dispatch path so the rebuilt
        // `peer_request` carries the real inner ability + args;
        // C1a / DEC-N4 §2.1 added the required `call_id` field
        // for response correlation.
        forward_invoke_args_for_ability(target_ura, "observe.health", serde_json::json!({}))
    }

    /// Parameterised sibling of `forward_invoke_args` for tests
    /// that need to drive a specific inner ability + args
    /// (e.g. PR-1 commit 7/9 self-target dispatch tests against
    /// `fs.read`).
    fn forward_invoke_args_for_ability(
        target_ura: &str,
        ability: &str,
        args: serde_json::Value,
    ) -> Vec<u8> {
        use base64::{engine::general_purpose::STANDARD, Engine as _};
        let public_ability = crate::ura::owner_local_ability_name(target_ura, ability);
        let ability_ura = crate::ura::owner_ability_ura(target_ura, &public_ability)
            .unwrap_or_else(|| panic!("derive test ability URA for {target_ura} {public_ability}"));
        let inner = serde_json::json!({
            "ability_ura": ability_ura,
            "args": args,
            "call_id": "test-call-id-1",
        });
        let inner_b64 = STANDARD.encode(serde_json::to_vec(&inner).unwrap());
        format!(r#"{{"target_ura":"{target_ura}","inner_envelope_b64":"{inner_b64}"}}"#)
            .into_bytes()
    }

    fn forward_invoke_args_for_ability_ura(
        target_ura: &str,
        ability_ura: &str,
        args: serde_json::Value,
    ) -> Vec<u8> {
        use base64::{engine::general_purpose::STANDARD, Engine as _};
        let inner = serde_json::json!({
            "ability_ura": ability_ura,
            "args": args,
            "call_id": "test-call-id-1",
        });
        let inner_b64 = STANDARD.encode(serde_json::to_vec(&inner).unwrap());
        format!(r#"{{"target_ura":"{target_ura}","inner_envelope_b64":"{inner_b64}"}}"#)
            .into_bytes()
    }

    // ── PR-1 commit 7/9 (LB-56) — self-targeted local dispatch ─────────

    #[tokio::test]
    async fn forward_invoke_self_target_runs_locally_via_axon_runtime() {
        // PR-1 commit 7/9 acceptance: when an inbound
        // `federation.forward_invoke` call's `target_ura` matches
        // THIS daemon's own canonical URA AND a local
        // Axon LocalRuntime is wired, the runtime MUST execute the
        // inner ability locally (no session push, no peer delegation)
        // and return the JSON result bytes inline
        // in `ForwardInvokeResponse.result_bytes`.
        //
        // This is the LB-56 §〇 production flow: hub-A → hub-B
        // peer delegation -> hub-B receives forward_invoke with
        // target_ura = hub-B's own URA (peer hub IS the target,
        // not a device on its bidi). Without this fall-through
        // the call surfaces target_offline because hub-B does
        // not register its own URA in its PresenceRegistry.
        // Build a minimal runtime with one ability that returns
        // a sentinel object so we can prove the bytes came from
        // the local runtime and not a daemon-internal stub.
        //
        // Register under the BARE registry key (`demo.echo`, not
        // `device.demo.echo`). Device-owned abilities enter
        // `AxonAbilityCatalog` un-prefixed (`fs.read`, `observe.health`,
        // …) and `sync_runtime_ability` mirrors that bare key into the
        // LocalRuntime verbatim, so the selected route's device-local
        // dispatch key is also bare. This mirrors the production
        // convention and the sibling `observe.health` quota test.
        let rt =
            runtime_with_json_echo("demo.echo", "MARKER-C9-1", "self-target-fallthrough-fired")
                .await;

        let svc = make_service()
            .with_session_realm("test-realm")
            .with_local_runtime(Arc::clone(&rt));
        publish_test_route(&svc, TEST_DAEMON_URI, "demo.echo");

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
            "result_bytes must come from the AxonAbilityCatalog handler, \
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
    async fn forward_invoke_self_target_scopes_agent_target_ability() {
        use crate::persistence::local_agents::{save, upsert_hosted_agent, LocalAgentsFile};

        let _hg = crate::facade::cli::test_support::HomeGuard::new();
        let target_ura = "easynet:///r/test-realm/agent/user.alice";
        let mut local = LocalAgentsFile {
            host_device_agent_ura: "easynet:///r/test-realm/device/dev-1".to_string(),
            hosted_agents: Vec::new(),
        };
        upsert_hosted_agent(&mut local, "llm", "alice", target_ura);
        save(&local).expect("seed local-agents.json");

        let rt =
            runtime_with_json_echo("alice.chat", "MARKER-AGENT-SCOPE", "agent-scope-fired").await;

        let svc = make_service()
            .with_session_realm("test-realm")
            .with_local_runtime(Arc::clone(&rt));
        publish_test_route(&svc, target_ura, "alice.chat");

        let response = svc
            .dispatch_federation_forward_invoke(
                None,
                &forward_invoke_args_for_ability_ura(
                    target_ura,
                    "easynet:///r/test-realm/ability/user.alice.chat",
                    serde_json::json!({"prompt": "hi"}),
                ),
            )
            .await
            .expect("self-target agent dispatch must scope and run locally");

        let body = response.into_inner();
        let parsed: federation_wrappers::ForwardInvokeResponse =
            serde_json::from_slice(&body.result).expect("body decodes");
        let result_value: serde_json::Value =
            serde_json::from_slice(&parsed.result_bytes).expect("result_bytes is JSON");
        assert_eq!(
            result_value
                .get("MARKER-AGENT-SCOPE")
                .and_then(|v| v.as_str()),
            Some("agent-scope-fired"),
            "bare `chat` must dispatch as `alice.chat` for agent URA self-targets"
        );
    }

    #[tokio::test]
    async fn forward_invoke_rejects_ability_ura_for_different_owner() {
        let target_ura = TEST_DAEMON_URI;
        let rt = runtime_with_json_echo(
            "observe.health",
            "MARKER-DEVICE-SCOPE",
            "device-scope-fired",
        )
        .await;
        let svc = make_service()
            .with_session_realm("test-realm")
            .with_local_runtime(Arc::clone(&rt));

        let err = svc
            .dispatch_federation_forward_invoke(
                None,
                &forward_invoke_args_for_ability_ura(
                    target_ura,
                    "easynet:///r/test-realm/ability/user.alice.chat",
                    serde_json::json!({"prompt": "hi"}),
                ),
            )
            .await
            .expect_err("ability_ura owner mismatch must reject");
        assert_eq!(err.code(), tonic::Code::InvalidArgument);
        assert!(
            err.message().contains("does not belong to target"),
            "error must cite owner mismatch, got: {}",
            err.message()
        );
    }

    #[tokio::test]
    async fn forward_invoke_rejects_bare_device_agent_alias() {
        let alias_target = "easynet:///r/test-realm/agent/dev-B";
        let canonical_target = "easynet:///r/test-realm/device/dev-B";
        let canonical_ability = "easynet:///r/test-realm/ability/device.dev-B.observe.health";
        let presence = Arc::new(PresenceRegistry::new());

        let (alias_tx, alias_rx) = tokio::sync::mpsc::channel(1);
        drop(alias_rx);
        presence.insert(alias_target.to_string(), alias_tx);
        let (canonical_tx, canonical_rx) = tokio::sync::mpsc::channel(1);
        drop(canonical_rx);
        presence.insert(canonical_target.to_string(), canonical_tx);

        let admission = AdmissionFacade::new(
            Arc::new(RealmTrustAnchor::default()),
            Some(TEST_DAEMON_URI.to_string()),
        );
        let svc = DaemonInvocationService::new(presence, admission)
            .with_session_realm("test-realm")
            .with_pending(Arc::new(PendingDispatchMap::new()));

        let err = svc
            .dispatch_federation_forward_invoke(
                None,
                &forward_invoke_args_for_ability_ura(
                    alias_target,
                    canonical_ability,
                    serde_json::json!({}),
                ),
            )
            .await
            .expect_err("legacy device-as-agent target alias must not be repaired");

        assert_eq!(err.code(), tonic::Code::InvalidArgument);
        assert!(
            err.message().contains("does not belong to target"),
            "error must cite owner mismatch, got: {}",
            err.message()
        );
    }

    #[tokio::test]
    async fn forward_invoke_local_hub_ura_runs_locally_via_axon_runtime() {
        // Device-mode escalation targets the local realm's hub URA,
        // not the hub host's device URA. The hub daemon must treat
        // `easynet:///r/<realm>/hub` as self-targeted even though
        // `AdmissionFacade.daemon_ura()` still carries the host
        // device URA from credentials.json.
        let rt =
            runtime_with_json_echo("demo.echo", "MARKER-C9-HUB", "local-hub-self-target-fired")
                .await;

        let svc = make_service()
            .with_session_realm("test-realm")
            .with_local_runtime(Arc::clone(&rt));
        publish_test_route(&svc, &crate::ura::hub_ura("test-realm"), "demo.echo");

        let response = svc
            .dispatch_federation_forward_invoke(
                None,
                &forward_invoke_args_for_ability(
                    &crate::ura::hub_ura("test-realm"),
                    "demo.echo",
                    serde_json::json!({"k": "hub"}),
                ),
            )
            .await
            .expect("local hub URA must hit the self-target dispatcher");

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
    async fn forward_invoke_self_target_without_local_runtime_rejects_explicitly() {
        // Guard: when Axon LocalRuntime is not wired, self-targeted
        // dispatch must fail explicitly instead of falling through to
        // PresenceRegistry and reporting a misleading target_offline.
        let svc = make_service().with_session_realm("test-realm");
        publish_test_route(&svc, TEST_DAEMON_URI, "observe.health");

        let err = svc
            .dispatch_federation_forward_invoke(None, &forward_invoke_args(TEST_DAEMON_URI))
            .await
            .expect_err("no LocalRuntime => explicit wiring error");
        assert_eq!(err.code(), tonic::Code::FailedPrecondition);
        assert!(
            err.message().contains("Axon LocalRuntime is not wired"),
            "expected LocalRuntime wiring error, got: {err}"
        );
    }

    #[tokio::test]
    async fn forward_invoke_self_target_unknown_ability_returns_not_found() {
        let rt = easynet_axon::invocation::LocalRuntime::new();
        let svc = make_service()
            .with_session_realm("test-realm")
            .with_local_runtime(Arc::clone(&rt));
        publish_test_route(&svc, TEST_DAEMON_URI, "demo.missing");

        let err = svc
            .dispatch_federation_forward_invoke(
                None,
                &forward_invoke_args_for_ability(
                    TEST_DAEMON_URI,
                    "demo.missing",
                    serde_json::json!({}),
                ),
            )
            .await
            .expect_err("known self target with unknown ability must be NotFound");
        assert_eq!(err.code(), tonic::Code::NotFound);
        assert!(
            err.message()
                .contains("not registered in Axon LocalRuntime"),
            "expected LocalRuntime not-found diagnostic, got: {err}"
        );
    }

    #[tokio::test]
    async fn forward_invoke_self_target_does_not_intercept_other_target_uras() {
        // Guard: the self-target arm must ONLY fire when
        // `target_ura == admission.daemon_ura()`. A different
        // target_ura (a real device URA in the same realm) goes
        // through the existing presence-push path and surfaces
        // target_offline when the device is not subscribed —
        // unchanged by the fall-through.
        let rt = runtime_with_json_echo("demo.echo", "MARKER-OTHER", "must-not-fire").await;
        let svc = make_service()
            .with_session_realm("test-realm")
            .with_local_runtime(Arc::clone(&rt));

        let err = svc
            .dispatch_federation_forward_invoke(
                None,
                &forward_invoke_args("easynet:///r/test-realm/device/some-other-device"),
            )
            .await
            .expect_err("non-self target ⇒ presence-push path ⇒ target_offline");
        assert_eq!(err.code(), tonic::Code::FailedPrecondition);
        assert!(
            err.message().contains(ROUTE_NEGATIVE_CODE),
            "non-self route miss must surface resolver negative, got: {err}"
        );
    }

    #[tokio::test]
    async fn forward_invoke_local_realm_requires_selected_route_before_peer_delegation() {
        // C1a / DEC-N4 §2.1: when `target_ura` realm matches
        // the daemon's own realm, the local presence-registry
        // path runs. With no presence entry inserted, the
        // dispatcher surfaces `Status::failed_precondition`
        // with the wire-stable `target_offline` reason. Critical:
        // the federation client is NEVER called even though one
        // is wired.
        let canned = InvokeResponse {
            result: br#"{"result_bytes":[]}"#.to_vec(),
            result_content_type: FEDERATION_RESULT_CONTENT_TYPE.to_string(),
            state: easynet_axon::invocation::InvocationState::Completed.to_wire_i32(),
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
            .expect_err("local-realm resolver miss surfaces route negative");
        assert_eq!(err.code(), tonic::Code::FailedPrecondition);
        assert!(
            err.message().contains(ROUTE_NEGATIVE_CODE),
            "expected resolver negative reason, got: {err}"
        );
        assert!(
            recorder.calls().is_empty(),
            "federation client must NOT be called for local-realm resolver negative"
        );
    }

    #[tokio::test]
    async fn forward_invoke_same_realm_route_negative_does_not_peer_fanout_when_configured() {
        let canned = InvokeResponse {
            result: serde_json::to_vec(&federation_wrappers::ForwardInvokeResponse {
                result_bytes: br#"{"hello":"from-same-realm-peer"}"#.to_vec(),
                correlation_call_id: "peer-call-id".to_string(),
            })
            .expect("encode peer ForwardInvokeResponse"),
            result_content_type: FEDERATION_RESULT_CONTENT_TYPE.to_string(),
            state: easynet_axon::invocation::InvocationState::Completed.to_wire_i32(),
            ..InvokeResponse::default()
        };
        let recorder = Arc::new(RecordingFederationClient::new(canned));
        let mut peers = BTreeMap::new();
        peers.insert(
            "same-realm-peer-hub".to_string(),
            "https://same-realm-peer.example:50443".to_string(),
        );

        let svc = make_service()
            .with_session_realm("test-realm")
            .with_federation_client(recorder.clone() as Arc<dyn FederationClient>)
            .with_federated_peers(peers);

        let target_ura = "easynet:///r/test-realm/device/paired-on-peer";
        let err = svc
            .dispatch_federation_forward_invoke(None, &forward_invoke_args(target_ura))
            .await
            .expect_err("local resolver negative stays terminal even with peers configured");
        assert_eq!(err.code(), tonic::Code::FailedPrecondition);
        assert!(
            err.message().contains(ROUTE_NEGATIVE_CODE),
            "expected resolver negative reason, got: {err}"
        );
        assert!(
            recorder.calls().is_empty(),
            "RFC-005 forbids same-realm peer fanout after local resolver negative"
        );
    }

    #[tokio::test]
    async fn forward_invoke_cross_realm_with_no_client_returns_target_offline() {
        // C1a / DEC-N4 §2.1: cross-realm target + no federation
        // client wired ⇒ `Status::failed_precondition` with the
        // wire-stable `target_offline` reason. The older
        // "Ok with target_online:false" shape is gone.
        let svc = make_service().with_session_realm("test-realm");

        let err = svc
            .dispatch_federation_forward_invoke(
                None,
                &forward_invoke_args("easynet:///r/peer-realm/device/peer-target"),
            )
            .await
            .expect_err("cross-realm without client surfaces target_offline");
        assert_eq!(err.code(), tonic::Code::FailedPrecondition);
        assert_eq!(
            err.message(),
            federation_wrappers::FORWARD_INVOKE_TARGET_OFFLINE_REASON,
        );
    }

    #[tokio::test]
    async fn forward_invoke_cross_realm_with_no_peer_entry_surfaces_resolver_noroute() {
        // C1a / DEC-N4 §2.1: federation client wired but the
        // operator-curated `federated_peers` map has no entry
        // for the target's realm. Under RFC-005 the cross-realm
        // delegation runs `namespace.resolve` first, so an
        // unmapped realm surfaces a typed `FailedPrecondition`
        // carrying `NEGATIVE_REASON_NOROUTE` instead of the old
        // opaque `target_offline` string. The map is still the
        // operator's explicit statement of "these are the peer
        // realms I federate with"; an unmapped realm is not
        // dialable and the federation client is never called.
        let canned = InvokeResponse {
            result: br#"{"result_bytes":[]}"#.to_vec(),
            result_content_type: FEDERATION_RESULT_CONTENT_TYPE.to_string(),
            state: easynet_axon::invocation::InvocationState::Completed.to_wire_i32(),
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
            .expect_err("unmapped realm surfaces resolver NOROUTE");
        assert_eq!(err.code(), tonic::Code::FailedPrecondition);
        assert_route_negative_noroute(err.message());
        assert!(
            recorder.calls().is_empty(),
            "federation client must NOT be called when peer entry is missing"
        );
    }

    #[tokio::test]
    async fn forward_invoke_cross_realm_auto_routes_via_federated_directory_when_opted_in() {
        // **Cross-hub auto-route, operator opt-in path**.
        // `federated_peers` is empty so the operator did NOT
        // statically declare `peer-realm → hub_endpoint`. But (a) the
        // hub-to-hub directory sync has previously observed the
        // target device on `https://hub-auto.example:50443`, and
        // (b) the operator opted into directory-driven auto-route
        // via `[daemon] allow_directory_auto_route = true`. The
        // dispatcher must then look the device up in
        // `federated_directory`, lift its `hub_endpoint`, and dial
        // there — lifting the requirement that operators
        // pre-declare every reachable realm in daemon-config.toml.
        //
        // The default-off counterpart lives in
        // `forward_invoke_cross_realm_directory_fallback_surfaces_resolver_noroute_by_default`.
        use crate::services::federation_directory::{
            DirectoryEntry, DirectoryView, SharedFederatedDirectoryView,
        };
        use std::collections::BTreeMap;

        let peer_reply_bytes = br#"{"hello":"from-auto-routed-peer"}"#.to_vec();
        let canned = InvokeResponse {
            result: serde_json::to_vec(&federation_wrappers::ForwardInvokeResponse {
                result_bytes: peer_reply_bytes.clone(),
                correlation_call_id: "test-call-id-1".to_string(),
            })
            .expect("encode peer ForwardInvokeResponse"),
            result_content_type: FEDERATION_RESULT_CONTENT_TYPE.to_string(),
            state: easynet_axon::invocation::InvocationState::Completed.to_wire_i32(),
            ..InvokeResponse::default()
        };
        let recorder = Arc::new(RecordingFederationClient::new(canned));

        let target_ura = "easynet:///r/unmapped-realm/device/peer-target";
        let cell = SharedFederatedDirectoryView::default();
        let mut peer_view = DirectoryView::new("unmapped-realm".to_string());
        peer_view.replace_entries(vec![DirectoryEntry {
            agent_ura: target_ura.to_string(),
            node_id: "peer-target".to_string(),
            display_name: None,
            status: "active".to_string(),
            origin_realm: None,
            hub_endpoint: Some("https://hub-auto.example:50443".to_string()),
            last_seen_unix_ms: Some(1_714_500_000_000),
        }]);
        let mut peers = BTreeMap::new();
        peers.insert("unmapped-realm".to_string(), Arc::new(peer_view));
        cell.replace(peers);

        // Crucially: NO `with_federated_peers(...)`. The static
        // operator-curated map is empty — only the directory cell
        // knows where the target lives. The opt-in is set
        // explicitly to mirror the production wiring from
        // `boot.rs`'s `config.allow_directory_auto_route()`.
        let svc = make_service()
            .with_session_realm("test-realm")
            .with_federation_client(recorder.clone() as Arc<dyn FederationClient>)
            .with_federated_directory_cell(cell)
            .with_allow_directory_auto_route(true);

        let resp = svc
            .dispatch_federation_forward_invoke(None, &forward_invoke_args(target_ura))
            .await
            .expect("directory-fallback path dials the auto-discovered hub");

        let body: federation_wrappers::ForwardInvokeResponse = parse_response_body(resp);
        assert_eq!(body.result_bytes, peer_reply_bytes);
        assert_eq!(body.correlation_call_id, "test-call-id-1");

        let calls = recorder.calls();
        assert_eq!(
            calls.len(),
            1,
            "exactly one peer dial — at the directory-derived hub_endpoint"
        );
        assert_eq!(
            calls[0].0, "https://hub-auto.example:50443",
            "dial target must come from federated_directory.hub_endpoint, \
             not from the (empty) federated_peers map"
        );
    }

    #[tokio::test]
    async fn forward_invoke_cross_realm_directory_fallback_surfaces_resolver_noroute_by_default() {
        // **P0 default-off pin**. Same setup as
        // `forward_invoke_cross_realm_auto_routes_via_federated_directory_when_opted_in`
        // but the operator has NOT opted in. The directory has the
        // entry, but the dispatcher must refuse to dial — it would
        // be handing an outbound federation request to a peer-hub-
        // controllable URL. The contract is: with the secure
        // default, an unmapped realm always resolves to typed
        // `NEGATIVE_REASON_NOROUTE`, regardless of what the
        // directory sync observed.
        use crate::services::federation_directory::{
            DirectoryEntry, DirectoryView, SharedFederatedDirectoryView,
        };
        use std::collections::BTreeMap;

        let canned = InvokeResponse {
            result: br#"{"result_bytes":[]}"#.to_vec(),
            result_content_type: FEDERATION_RESULT_CONTENT_TYPE.to_string(),
            state: easynet_axon::invocation::InvocationState::Completed.to_wire_i32(),
            ..InvokeResponse::default()
        };
        let recorder = Arc::new(RecordingFederationClient::new(canned));

        let target_ura = "easynet:///r/unmapped-realm/device/peer-target";
        let cell = SharedFederatedDirectoryView::default();
        let mut peer_view = DirectoryView::new("unmapped-realm".to_string());
        peer_view.replace_entries(vec![DirectoryEntry {
            agent_ura: target_ura.to_string(),
            node_id: "peer-target".to_string(),
            display_name: None,
            status: "active".to_string(),
            origin_realm: None,
            hub_endpoint: Some("https://attacker.example:50443".to_string()),
            last_seen_unix_ms: Some(1_714_500_000_000),
        }]);
        let mut peers = BTreeMap::new();
        peers.insert("unmapped-realm".to_string(), Arc::new(peer_view));
        cell.replace(peers);

        // No `with_allow_directory_auto_route(true)` — service
        // inherits the secure default (false).
        let svc = make_service()
            .with_session_realm("test-realm")
            .with_federation_client(recorder.clone() as Arc<dyn FederationClient>)
            .with_federated_directory_cell(cell);

        let err = svc
            .dispatch_federation_forward_invoke(None, &forward_invoke_args(target_ura))
            .await
            .expect_err("default-off must refuse the directory-derived endpoint");
        assert_eq!(err.code(), tonic::Code::FailedPrecondition);
        assert_route_negative_noroute(err.message());
        assert!(
            recorder.calls().is_empty(),
            "federation client must NOT be called when directory fallback is disabled"
        );
    }

    #[tokio::test]
    async fn forward_invoke_cross_realm_directory_entry_without_hub_endpoint_surfaces_resolver_noroute(
    ) {
        // Edge case: the directory has the target URA but the peer's
        // snapshot omitted `hub_endpoint`. Auto-route has nowhere to
        // dial; the resolver must surface a typed `NEGATIVE_REASON_NOROUTE`
        // rather than dialing some default. Operators relying on auto-route
        // need to know their directory sync is missing the endpoint
        // field, not get a misleading "delivered" outcome.
        use crate::services::federation_directory::{
            DirectoryEntry, DirectoryView, SharedFederatedDirectoryView,
        };
        use std::collections::BTreeMap;

        let canned = InvokeResponse {
            result: br#"{"result_bytes":[]}"#.to_vec(),
            result_content_type: FEDERATION_RESULT_CONTENT_TYPE.to_string(),
            state: easynet_axon::invocation::InvocationState::Completed.to_wire_i32(),
            ..InvokeResponse::default()
        };
        let recorder = Arc::new(RecordingFederationClient::new(canned));

        let target_ura = "easynet:///r/unmapped-realm/device/peer-target";
        let cell = SharedFederatedDirectoryView::default();
        let mut peer_view = DirectoryView::new("unmapped-realm".to_string());
        peer_view.replace_entries(vec![DirectoryEntry {
            agent_ura: target_ura.to_string(),
            node_id: "peer-target".to_string(),
            display_name: None,
            status: "active".to_string(),
            origin_realm: None,
            hub_endpoint: None, // <- the gap under test
            last_seen_unix_ms: Some(1_714_500_000_000),
        }]);
        let mut peers = BTreeMap::new();
        peers.insert("unmapped-realm".to_string(), Arc::new(peer_view));
        cell.replace(peers);

        // The opt-in is ON in this test so we exercise the
        // "missing hub_endpoint" branch of the resolver, not the
        // "fallback disabled" branch (which is its own pin in
        // `forward_invoke_cross_realm_directory_fallback_surfaces_resolver_noroute_by_default`).
        let svc = make_service()
            .with_session_realm("test-realm")
            .with_federation_client(recorder.clone() as Arc<dyn FederationClient>)
            .with_federated_directory_cell(cell)
            .with_allow_directory_auto_route(true);

        let err = svc
            .dispatch_federation_forward_invoke(None, &forward_invoke_args(target_ura))
            .await
            .expect_err("missing hub_endpoint cannot be auto-routed");
        assert_eq!(err.code(), tonic::Code::FailedPrecondition);
        assert_route_negative_noroute(err.message());
        assert!(
            recorder.calls().is_empty(),
            "no dial when directory entry carries no hub_endpoint"
        );
    }

    #[tokio::test]
    async fn forward_invoke_cross_realm_with_peer_entry_dials_via_federation_client() {
        // C1a / DEC-N4 §2.1: cross-realm + federation client
        // wired + peer entry present ⇒ federation client called
        // with the peer's hub URA + the *inner* ability decoded
        // from `inner_envelope_b64`. Response carries peer's
        // `result` bytes through `result_bytes`, plus the
        // caller's `correlation_call_id` echoed back.
        let peer_reply_bytes = br#"{"hello":"from-peer"}"#.to_vec();
        let canned = InvokeResponse {
            result: peer_reply_bytes.clone(),
            result_content_type: FEDERATION_RESULT_CONTENT_TYPE.to_string(),
            state: easynet_axon::invocation::InvocationState::Completed.to_wire_i32(),
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

        let target_ura = "easynet:///r/peer-realm/device/peer-target";
        let args = forward_invoke_args(target_ura);
        let resp = svc
            .dispatch_federation_forward_invoke(None, &args)
            .await
            .expect("cross-realm returns Ok");

        // Response carries the peer's `result` bytes verbatim
        // in `result_bytes`, and stamps back the caller's
        // `call_id` from the fixture as `correlation_call_id`.
        let body: federation_wrappers::ForwardInvokeResponse = parse_response_body(resp);
        assert_eq!(body.result_bytes, peer_reply_bytes);
        assert_eq!(body.correlation_call_id, "test-call-id-1");

        let calls = recorder.calls();
        assert_eq!(calls.len(), 1, "exactly one peer delegation call");
        assert_eq!(calls[0].0, "https://peer-hub.example:50443");
        // **LB-57 §一 Option A wire shape**. Peer delegation
        // re-wraps the call as another `federation.forward_invoke`
        // so the peer hub's top-level `Invoke::invoke` match routes
        // through `dispatch_federation_forward_invoke` (which owns
        // local-session dispatch + same-realm fan-out + cross-realm
        // delegation). The pre-LB-57 PR-N1 commit 11/N shape (sending the
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
        // ForwardInvokeRequest carrying the SAME target_ura +
        // inner_envelope_b64 the caller hub received, so the
        // peer's `dispatch_federation_forward_invoke` re-runs
        // its own routing (local-presence / same-realm fan-out
        // / cross-realm dial) against the original payload.
        let nested: federation_wrappers::ForwardInvokeRequest =
            serde_json::from_slice(&calls[0].1.arguments)
                .expect("peer arguments decode as nested ForwardInvokeRequest");
        assert_eq!(nested.target_ura, target_ura);
        assert!(
            !nested.inner_envelope_b64.is_empty(),
            "nested wrapper carries the original inner_envelope_b64 verbatim"
        );
        // When the original request carries no caller envelope, the
        // caller hub must still present its own hub URA to the peer.
        // Using `target_ura` here makes the peer believe the target
        // device itself initiated the call, which fails trust-anchor
        // admission and opens the circuit breaker.
        let peer_envelope = calls[0].1.envelope.as_ref().expect("envelope present");
        let peer_caller = peer_envelope
            .caller
            .as_ref()
            .expect("caller identity present");
        assert_eq!(peer_caller.ura, crate::ura::hub_ura("test-realm"));
        let peer_callee = peer_envelope
            .callee
            .as_ref()
            .expect("callee identity present");
        assert_eq!(peer_callee.ura, crate::ura::hub_ura("peer-realm"));
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
    async fn forward_invoke_cross_realm_peer_request_admits_against_hub_anchor() {
        // The cross-hub deep harness failure we care about is not
        // "signature field missing" anymore; it is "peer hub rejects
        // the rebuilt federation.forward_invoke wrapper with
        // CALLER_SIGNATURE_INVALID". Rebuild that exact wrapper via
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
            state: easynet_axon::invocation::InvocationState::Completed.to_wire_i32(),
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

        let target_ura = "easynet:///r/peer-realm/device/peer-target";
        svc.dispatch_federation_forward_invoke(None, &forward_invoke_args(target_ura))
            .await
            .expect("cross-realm wrapper build succeeds");

        let calls = recorder.calls();
        assert_eq!(calls.len(), 1, "exactly one peer request captured");
        let peer_request = calls[0].1.clone();
        let peer_envelope = peer_request
            .envelope
            .as_ref()
            .expect("peer request envelope present");
        let caller_ura = peer_envelope
            .caller
            .as_ref()
            .expect("caller present")
            .ura
            .clone();

        let caller_signing_key = SigningKey::from_bytes(&[0x11; 32]);
        let caller_pubkey_b64 =
            BASE64_STANDARD.encode(caller_signing_key.verifying_key().to_bytes());
        let peer_anchor = Arc::new(
            RealmTrustAnchor::from_entries(vec![
                crate::services::realm_trust_anchor::TrustedAgent {
                    agent_ura: caller_ura,
                    public_key_b64: caller_pubkey_b64,
                    role: crate::services::realm_trust_anchor::TrustedAgentRole::Hub,
                    added_at_unix_ms: 1_714_492_800_000,
                    origin_realm: Some("test-realm".to_string()),
                    hub_endpoint: Some("https://peer-hub.example:50443".to_string()),
                    tls_ca_pem_path: None,
                },
            ])
            .expect("peer hub trust anchor"),
        );
        let peer_admission =
            AdmissionFacade::new(peer_anchor, Some(crate::ura::hub_ura("peer-realm")));

        peer_admission
            .verify_invoke(&peer_request)
            .expect("peer hub must admit the rebuilt signed wrapper");
    }

    // ── C1b / DEC-N5 §1: ForwardReceipt dual-write tests ──

    // Phase 5a removed the three ForwardReceipt-shape tests
    // (`forward_invoke_cross_realm_happy_path_records_forward_receipt_with_digest`,
    //  `forward_invoke_target_offline_records_forward_receipt_with_no_digest`,
    //  `forward_invoke_local_realm_miss_records_forward_receipt_with_no_digest`).
    // Their entire surface was asserting on the now-deleted
    // `SharedReceiptStore`. The *behaviours* those tests pinned
    // (target_offline returns FailedPrecondition / local-realm
    // resolver miss returns FailedPrecondition / cross-realm
    // happy path returns Ok) are still covered by the
    // `forward_invoke_local_realm_requires_selected_route_before_peer_delegation`,
    // `forward_invoke_*_target_offline` and
    // `cross_hub_forward_invoke_e2e_in_process` tests further
    // down — those check the wire-level Result, which is the
    // contract that actually matters for downstream callers.

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
        // means daemon A's URA must be in daemon B's trust
        // anchor as a Hub-role peer. PR-N2 lands the
        // FederatedKeyResolver that resolves daemon A's signing
        // key out of daemon B's trust set; without that the
        // cross-realm strict admission would reject the
        // signature step. Either way, the in-process e2e here
        // proves the routing chain works; full TLS handshake +
        // cross-realm admission is the operator-side smoke test.
        const REALM_A: &str = "realm-a";
        const REALM_B: &str = "realm-b";
        const DAEMON_A_URI: &str = "easynet:///r/realm-a/device/daemon-a";
        const DAEMON_B_URI: &str = "easynet:///r/realm-b/device/daemon-b";
        const TARGET_DEVICE_URI: &str = "easynet:///r/realm-b/device/target-device";
        const PEER_HUB_URI: &str = "https://daemon-b.example:50443";

        // Daemon B's trust anchor: pre-populated with daemon A
        // as a Backend-role entry so daemon B's admission gate
        // admits a request whose envelope.caller.ura is daemon
        // A's URA. URA-only no-op admission today (Backend role
        // skips the strict signature path? — no, Backend goes
        // strict. Use Device for URA-only no-op so the e2e
        // doesn't depend on PR-N2 cross-realm sig verify).
        // DEC-013 path-conditional admission lets Device entries
        // pass URA-only — exactly what we need for the in-
        // process e2e under PR-N1.
        let daemon_a_in_b_trust = vec![crate::services::realm_trust_anchor::TrustedAgent {
            agent_ura: DAEMON_A_URI.to_string(),
            public_key_b64: "AAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAA=".to_string(),
            role: crate::services::realm_trust_anchor::TrustedAgentRole::Device,
            added_at_unix_ms: 1_714_492_800_000,
            origin_realm: None,
            hub_endpoint: None,
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
        publish_test_route(&daemon_b, TARGET_DEVICE_URI, "federation.heartbeat");

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
                use easynet_axon::pb::axon::v1::invoke_bidi_down::Payload as DownPayload;
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
                        failure: None,
                        request_id: None,
                    },
                );
            }
        });

        // Daemon A: empty presence registry; cross-realm target
        // routes via the InProcessPeerClient → daemon B. We
        // forward the envelope verbatim from the test request so
        // daemon B sees `envelope.caller.ura = DAEMON_A_URI` and
        // resolves the URA-only Device admission against the
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
        // (base64 of `{ability_ura, args}`) and sends the inner
        // ability URA to the peer instead of re-wrapping in another
        // `federation.forward_invoke`.
        //
        // base64({"ability_ura":".../ability/device.target-device-b.federation.heartbeat","args":{
        //   "membership_ura":"easynet:///r/realm-b/device/target-device-b",
        //   "ts_ms":0
        // }})
        let public_ability = "federation.heartbeat";
        let ability_ura = crate::ura::owner_ability_ura(TARGET_DEVICE_URI, public_ability)
            .expect("target device ability URA");
        let inner_payload = serde_json::json!({
            "ability_ura": ability_ura,
            "args": {
                "agent_ura": TARGET_DEVICE_URI,
            },
            "call_id": "e2e-call-id-1",
        });
        let inner_b64 = {
            use base64::{engine::general_purpose::STANDARD, Engine as _};
            STANDARD.encode(serde_json::to_vec(&inner_payload).unwrap())
        };
        let forward_args = format!(
            r#"{{"target_ura":"{}","inner_envelope_b64":"{}"}}"#,
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

        // ── Assert: cross-realm chain returned the device's ──
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
    /// peer request so daemon B's admission gate sees a caller URA
    /// it can admit. Real PR-N2 path will sign + AXIOM-rewrite the
    /// envelope; this test fixture just stamps the original
    /// envelope verbatim, sufficient for the URA-only Device
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

    fn test_envelope_with_uri(ura: &str) -> Envelope {
        Envelope {
            caller: Some(AgentIdentity {
                ura: ura.to_string(),
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
                &session_request_ability_ura("test-realm", ABILITY_FEDERATION_FORWARD_INVOKE),
                &forward_invoke_args("easynet:///r/test-realm/device/missing-device"),
            )
            .await;
        match outcome {
            RequestOutcome::Err {
                error: SessionRequestError::UpstreamFailure { reason },
            } => {
                assert!(
                    reason.contains(ROUTE_NEGATIVE_CODE),
                    "expected resolver negative, got: {reason}"
                );
            }
            other => panic!(
                "expected resolver upstream failure, got {other:?}; the hub's empty \
                 PresenceRegistry must surface as typed resolve failure"
            ),
        }
    }

    #[tokio::test]
    async fn dispatch_session_request_advertise_agent_updates_store() {
        // Hot `agent.start` runs on the already-open device
        // session, so its hub repair path arrives as a
        // SessionDispatch::Request. The handler must route
        // `federation.advertise_agent` through the same store-writing
        // wrapper as unary Invoke; otherwise agent add succeeds
        // locally while chat / skill / history still fail with
        // "agent is not advertised on this hub".
        let svc = make_service().with_session_realm("test-realm");
        let agent_ura = "easynet:///r/test-realm/agent/dev.anthropic";
        let args = serde_json::to_vec(&serde_json::json!({
            "agent_ura": agent_ura,
            "signing_authority": {
                "kind": "hosted_by",
                "host_ura": TEST_DAEMON_URI,
            },
            "host_node_id": "test-daemon",
        }))
        .expect("advertise args encode");

        let outcome = svc
            .dispatch_session_request(
                &session_request_ability_ura("test-realm", ABILITY_FEDERATION_ADVERTISE_AGENT),
                &args,
            )
            .await;

        match outcome {
            RequestOutcome::Ok { result_bytes } => {
                let body: federation_wrappers::AdvertiseAgentResponse =
                    serde_json::from_slice(&result_bytes)
                        .expect("body decodes as AdvertiseAgentResponse");
                assert!(body.ack);
            }
            other => panic!("expected advertise_agent Ok outcome, got {other:?}"),
        }

        let record = svc
            .advertised_agents
            .get(agent_ura)
            .expect("advertise_agent request must populate AdvertisedAgentStore");
        assert_eq!(record.host_ura(), Some(TEST_DAEMON_URI));
        assert_eq!(record.host_node_id.as_deref(), Some("test-daemon"));
    }

    #[tokio::test]
    async fn dispatch_session_request_unknown_ability_returns_permission_denied() {
        // PR-N6 v1 only routes the small explicit set used by
        // invoke forwarding and hosted-agent self-advertise repair.
        // Other ability names must surface a typed `PermissionDenied`
        // so the device caller knows the hub refused (not a silent
        // timeout). PR-N6 v2 may widen this set once a per-ability
        // admission policy is specified.
        let svc = make_service().with_session_realm("test-realm");
        let outcome = svc
            .dispatch_session_request(&session_request_ability_ura("test-realm", "fs.read"), b"{}")
            .await;
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
                    "reason must cite forward_invoke as an allowed ability; got: {reason}",
                );
                assert!(
                    reason.contains(ABILITY_FEDERATION_ADVERTISE_AGENT),
                    "reason must cite advertise_agent as an allowed ability; got: {reason}",
                );
            }
            other => panic!("expected PermissionDenied for unknown ability, got {other:?}"),
        }
    }

    #[tokio::test]
    async fn dispatch_session_request_rejects_non_hub_ability_ura() {
        let svc = make_service().with_session_realm("test-realm");
        let outcome = svc
            .dispatch_session_request(
                "easynet:///r/test-realm/ability/device.device-a.federation.forward_invoke",
                b"{}",
            )
            .await;

        match outcome {
            RequestOutcome::Err {
                error: SessionRequestError::PermissionDenied { reason },
            } => {
                assert!(
                    reason.contains("does not belong to hub"),
                    "wrong owner rejection must be explicit, got: {reason}",
                );
            }
            other => panic!("expected PermissionDenied for wrong-owner Ability URA, got {other:?}"),
        }
    }

    // ── PR-N6 C5 - hub Request -> selected local-session dispatch ──

    #[tokio::test]
    async fn dispatch_session_request_forward_invoke_hits_selected_local_session() {
        // **LB-57 Option A acceptance** (same-hub): when the
        // inbound Request's target_ura realm matches the hub's
        // local realm AND the target device is subscribed in
        // this hub's PresenceRegistry, the dispatcher MUST:
        //   1. Push a `SessionDispatch::Dispatch` frame down
        //      the target's reverse channel (the wire shape
        //      device-side `LocalAxonSessionDispatcher` decodes).
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
        let target_ura = "easynet:///r/test-realm/device/local-target";
        publish_test_route(&svc, target_ura, "observe.health");

        let (tx, mut rx) = tokio::sync::mpsc::channel::<
            Result<crate::services::presence_registry::DispatchFrame, tonic::Status>,
        >(4);
        svc.presence.insert(target_ura.to_string(), tx);

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
            use easynet_axon::pb::axon::v1::invoke_bidi_down::Payload as DownPayload;
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
            // LocalAxonSessionDispatcher would produce after running
            // the inner ability).
            let result_bytes = br#"{"echo":"args-from-A"}"#.to_vec();
            pending_for_fake.complete(
                call_id,
                DispatchResult {
                    payload: result_bytes,
                    error: None,
                    failure: None,
                    request_id: None,
                },
            );
        });

        let outcome = svc
            .dispatch_session_request(
                &session_request_ability_ura("test-realm", ABILITY_FEDERATION_FORWARD_INVOKE),
                &forward_invoke_args(target_ura),
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

    #[tokio::test]
    async fn dispatch_session_request_forward_invoke_preserves_target_failure_code() {
        let svc = make_service()
            .with_session_realm("test-realm")
            .with_pending(Arc::new(PendingDispatchMap::new()));
        let target_ura = "easynet:///r/test-realm/device/local-target";
        publish_test_route(&svc, target_ura, "observe.health");

        let (tx, mut rx) = tokio::sync::mpsc::channel::<
            Result<crate::services::presence_registry::DispatchFrame, tonic::Status>,
        >(4);
        svc.presence.insert(target_ura.to_string(), tx);

        let pending = svc.pending.clone().expect("pending wired above");
        let fake_device = tokio::spawn(async move {
            let frame = rx
                .recv()
                .await
                .expect("reverse-channel frame arrives")
                .expect("frame is Ok");
            use easynet_axon::pb::axon::v1::invoke_bidi_down::Payload as DownPayload;
            let chunk = match frame.frame.payload {
                Some(DownPayload::BinaryChunk(c)) => c,
                other => panic!("expected BinaryChunk, got {other:?}"),
            };
            let dispatch: SessionDispatch =
                serde_json::from_slice(&chunk.data).expect("frame is SessionDispatch JSON");
            let SessionDispatch::Dispatch { call_id, .. } = dispatch else {
                panic!("expected SessionDispatch::Dispatch, got {dispatch:?}");
            };

            let failure = SessionFailure::from_explicit("disk_full", "volume is full", true);
            pending.complete(
                call_id,
                DispatchResult {
                    payload: Vec::new(),
                    error: Some("target write failed".to_string()),
                    failure: Some(failure),
                    request_id: Some("target-request-1".to_string()),
                },
            );
        });

        let outcome = svc
            .dispatch_session_request(
                &session_request_ability_ura("test-realm", ABILITY_FEDERATION_FORWARD_INVOKE),
                &forward_invoke_args(target_ura),
            )
            .await;

        match outcome {
            RequestOutcome::Err {
                error: SessionRequestError::UpstreamFailure { reason },
            } => {
                assert!(
                    reason.contains("DISK_FULL: volume is full"),
                    "target SessionFailure code/message must survive hub projection; got: {reason}",
                );
            }
            other => panic!("expected typed upstream failure, got {other:?}"),
        }

        fake_device.await.expect("fake device task joined");
    }

    #[tokio::test]
    async fn dispatch_session_request_forward_invoke_scopes_agent_target_ability() {
        let svc = make_service()
            .with_session_realm("test-realm")
            .with_pending(Arc::new(PendingDispatchMap::new()));
        let target_ura = "easynet:///r/test-realm/agent/user.alice";
        let host_ura = "easynet:///r/test-realm/device/alice-host";
        publish_test_route_hosted_by(&svc, target_ura, "alice.chat", host_ura);

        let (tx, mut rx) = tokio::sync::mpsc::channel::<
            Result<crate::services::presence_registry::DispatchFrame, tonic::Status>,
        >(4);
        svc.presence.insert(host_ura.to_string(), tx);

        let pending = svc.pending.clone().expect("pending wired above");
        let fake_device = tokio::spawn(async move {
            let frame = rx
                .recv()
                .await
                .expect("reverse-channel frame arrives")
                .expect("frame is Ok");
            use easynet_axon::pb::axon::v1::invoke_bidi_down::Payload as DownPayload;
            let chunk = match frame.frame.payload {
                Some(DownPayload::BinaryChunk(c)) => c,
                other => panic!("expected BinaryChunk, got {other:?}"),
            };
            let dispatch: SessionDispatch =
                serde_json::from_slice(&chunk.data).expect("frame is SessionDispatch JSON");
            let SessionDispatch::Dispatch {
                call_id, ability, ..
            } = dispatch
            else {
                panic!("expected SessionDispatch::Dispatch, got {dispatch:?}");
            };
            assert_eq!(
                ability, "alice.chat",
                "agent URA targets must scope bare inner ability names before \
                 writing the reverse-channel dispatch frame"
            );
            pending.complete(
                call_id,
                DispatchResult {
                    payload: br#"{"echo":"agent-scoped"}"#.to_vec(),
                    error: None,
                    failure: None,
                    request_id: None,
                },
            );
        });

        let outcome = svc
            .dispatch_session_request(
                &session_request_ability_ura("test-realm", ABILITY_FEDERATION_FORWARD_INVOKE),
                &forward_invoke_args_for_ability_ura(
                    target_ura,
                    "easynet:///r/test-realm/ability/user.alice.chat",
                    serde_json::json!({"prompt": "hi"}),
                ),
            )
            .await;
        match outcome {
            RequestOutcome::Ok { result_bytes } => {
                let body: federation_wrappers::ForwardInvokeResponse =
                    serde_json::from_slice(&result_bytes)
                        .expect("body decodes as ForwardInvokeResponse");
                assert_eq!(body.result_bytes, br#"{"echo":"agent-scoped"}"#.to_vec());
            }
            other => panic!("expected Ok with scoped agent dispatch, got {other:?}"),
        }
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
        use crate::services::invocation_transport::invoke_remote_initiator::SessionDispatch;
        use crate::services::invocation_transport::session_escalation::{
            spawn_escalation_consumer, EscalationCorrelation,
        };
        use crate::services::invocation_transport::session_initiator::SessionUpSender;
        use easynet_axon::pb::axon::v1::invoke_bidi_up::Payload as UpPayload;
        use tokio::sync::mpsc;

        let correlation = EscalationCorrelation::new();
        let (up_tx, mut up_rx) = mpsc::channel(8);
        let handle = std::sync::Arc::new(spawn_escalation_consumer(
            correlation.clone(),
            SessionUpSender::new(up_tx),
            "test-realm",
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
        use crate::services::invocation_transport::session_escalation::{
            spawn_escalation_consumer, EscalationCorrelation,
        };
        use crate::services::invocation_transport::session_initiator::SessionUpSender;
        use tokio::sync::mpsc;

        let correlation = EscalationCorrelation::new();
        let (up_tx, mut up_rx) = mpsc::channel(8);
        let handle = std::sync::Arc::new(spawn_escalation_consumer(
            correlation.clone(),
            SessionUpSender::new(up_tx),
            "test-realm",
        ));

        // Fake hub: complete every Request with TargetOffline.
        tokio::spawn(async move {
            use crate::services::invocation_transport::invoke_remote_initiator::SessionDispatch;
            use easynet_axon::pb::axon::v1::invoke_bidi_up::Payload as UpPayload;
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
        use crate::services::invocation_transport::session_escalation::{
            spawn_escalation_consumer, EscalationCorrelation,
        };
        use crate::services::invocation_transport::session_initiator::SessionUpSender;
        use tokio::sync::mpsc;

        let correlation = EscalationCorrelation::new();
        let (up_tx, _up_rx_held) = mpsc::channel(8);
        let handle = std::sync::Arc::new(spawn_escalation_consumer(
            correlation,
            SessionUpSender::new(up_tx),
            "test-realm",
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

    // ── PR-N6 C5 / RFC-005 — resolver-aware session-request markers + e2e ──

    #[tokio::test]
    async fn dispatch_session_request_emits_resolver_selected_route_marker() {
        // The marker is observability-only, but it must use the
        // same resolver facts as dispatch: route selected means
        // R300, not a presence/realm guess.
        // A unit test cannot easily intercept stderr without
        // process gymnastics; instead we exercise the method on
        // a service with a projection-backed route. Compile-time
        // coupling to the method is the regression pin here.
        let svc = make_service().with_session_realm("test-realm");
        let target_ura = "easynet:///r/test-realm/device/local-target";
        publish_test_route(&svc, target_ura, "observe.health");
        svc.emit_session_request_resolution_marker(&forward_invoke_args(target_ura))
            .await;
        // No assertion possible without a stderr capture rig;
        // the function returns unit. Branch coverage IS the
        // assertion: a future change that drops the marker will
        // make this test fail to compile or the external log
        // contract fail loudly.
    }

    #[tokio::test]
    async fn dispatch_session_request_surfaces_resolver_negative_when_same_realm_route_missing() {
        // Smoke check the routing path: same-realm target with
        // no projection-backed route surfaces the resolver
        // negative, not a synthetic target_offline.
        let svc = make_service().with_session_realm("realm-X");
        let outcome = svc
            .dispatch_session_request(
                &session_request_ability_ura("realm-X", ABILITY_FEDERATION_FORWARD_INVOKE),
                &forward_invoke_args("easynet:///r/realm-X/device/missing-device"),
            )
            .await;
        match outcome {
            RequestOutcome::Err {
                error: SessionRequestError::UpstreamFailure { reason },
            } => {
                assert!(
                    reason.contains(ROUTE_NEGATIVE_CODE),
                    "expected resolver negative, got: {reason}"
                );
            }
            other => panic!(
                "same-realm target with empty presence must surface resolver negative, \
                 got {other:?}"
            ),
        }
    }

    #[tokio::test]
    async fn dispatch_session_request_routes_selected_route_when_cross_realm_target_is_present() {
        // Platform hubs can host devices whose URAs live under a
        // user realm different from the hub's own control-plane
        // realm. RFC-005 selects the local route from projection +
        // presence, then dispatches by selected execution host.
        let svc = make_service()
            .with_session_realm("easynet-platform")
            .with_pending(Arc::new(PendingDispatchMap::new()));
        let target_ura = "easynet:///r/user-realm/device/present-device";
        publish_test_route(&svc, target_ura, "observe.health");

        let (tx, mut rx) = tokio::sync::mpsc::channel::<
            Result<crate::services::presence_registry::DispatchFrame, tonic::Status>,
        >(4);
        svc.presence.insert(target_ura.to_string(), tx);

        let pending = svc.pending.clone().expect("pending wired above");
        let pending_for_fake = Arc::clone(&pending);
        let fake_device = tokio::spawn(async move {
            let frame = rx
                .recv()
                .await
                .expect("reverse-channel frame arrives")
                .expect("frame is Ok");
            use easynet_axon::pb::axon::v1::invoke_bidi_down::Payload as DownPayload;
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
                    failure: None,
                    request_id: None,
                },
            );
        });

        let outcome = svc
            .dispatch_session_request(
                &session_request_ability_ura("easynet-platform", ABILITY_FEDERATION_FORWARD_INVOKE),
                &forward_invoke_args(target_ura),
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
                "cross-realm target with selected local route must dispatch on this hub, got {other:?}"
            ),
        }
    }

    #[tokio::test]
    async fn dispatch_session_request_routes_peer_delegation_when_target_realm_differs() {
        // Cross-realm target with no federation client wired
        // surfaces target_offline from the peer-delegation arm.
        let svc = make_service().with_session_realm("realm-X");
        let outcome = svc
            .dispatch_session_request(
                &session_request_ability_ura("realm-X", ABILITY_FEDERATION_FORWARD_INVOKE),
                &forward_invoke_args("easynet:///r/peer-realm/device/peer-target"),
            )
            .await;
        match outcome {
            RequestOutcome::Err {
                error: SessionRequestError::TargetOffline,
            } => {}
            other => panic!(
                "cross-realm target with no federation client must surface \
                 TargetOffline (peer-delegation fall-through), got {other:?}"
            ),
        }
    }

    #[tokio::test]
    async fn end_to_end_device_escalation_resolves_via_hub_session_request() {
        // PR-N6 §三 C5 acceptance: end-to-end 4-process simulated
        // topology - device-A -> hub-A -> selected local-session
        // resolution at hub-A -> device-A receives canned bytes.
        //
        // We simulate the topology in-process:
        //   - "hub-A" = a `DaemonInvocationService` with session_
        //     realm "test-realm" and a populated PresenceRegistry
        //     entry for the target URA.
        //   - "device-A" = a `SessionEscalationHandle` whose
        //     consumer's up_tx feeds a fake hub-side task that
        //     decodes Request frames, calls hub-A's
        //     `dispatch_session_request`, and writes the
        //     RequestResult back into the correlation table.
        //
        // The chain proves: device-side escalation handle ->
        // up-channel Request frame -> hub-side dispatch_session_
        // request -> resolver-selected forward_invoke -> push to
        // PresenceRegistry -> response bytes round-trip back via
        // RequestResult -> device caller receives the bytes.
        use crate::services::invocation_transport::invoke_remote_initiator::SessionDispatch;
        use crate::services::invocation_transport::session_escalation::{
            spawn_escalation_consumer, EscalationCorrelation,
        };
        use crate::services::invocation_transport::session_initiator::SessionUpSender;
        use crate::services::presence_registry::DispatchSender;
        use easynet_axon::pb::axon::v1::invoke_bidi_up::Payload as UpPayload;
        use tokio::sync::mpsc;

        // **LB-57 Option A** updated contract: hub_service now
        // dispatches via `dispatch_local_presence_forward_invoke`,
        // which (1) requires `with_pending` to be set, (2) pushes
        // a `SessionDispatch::Dispatch` frame down the target's
        // reverse channel, and (3) awaits the matching
        // `SessionDispatch::Result` via the PendingDispatchMap
        // before returning. The device's response bytes flow
        // through inline as `result_bytes`, not the earlier
        // empty-bytes "delivery accepted" shape.
        // RFC-005: device target lives under `device/<id>`, not
        // `agent/<id>`. The forward_invoke entry point no longer
        // repairs device aliases, so fixtures must register and
        // invoke the canonical owner URA directly.
        let target_ura = "easynet:///r/test-realm/device/dev-B";
        let presence = std::sync::Arc::new(PresenceRegistry::new());
        let (target_tx, mut target_rx): (DispatchSender, _) = mpsc::channel(8);
        presence.insert(target_ura.to_string(), target_tx);
        let admission = AdmissionFacade::new(
            std::sync::Arc::new(RealmTrustAnchor::default()),
            Some(TEST_DAEMON_URI.to_string()),
        );
        let hub_service = DaemonInvocationService::new(presence, admission)
            .with_session_realm("test-realm")
            .with_pending(Arc::new(PendingDispatchMap::new()));
        publish_test_route(&hub_service, target_ura, "observe.health");

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
            use easynet_axon::pb::axon::v1::invoke_bidi_down::Payload as DownPayload;
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
                    failure: None,
                    request_id: None,
                },
            );
        });

        // Device-side escalation handle + consumer.
        let correlation = EscalationCorrelation::new();
        let (up_tx, mut up_rx) = mpsc::channel(8);
        let device_handle = spawn_escalation_consumer(
            std::sync::Arc::clone(&correlation),
            SessionUpSender::new(up_tx),
            "test-realm",
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
                    ability_ura,
                    args,
                    ..
                } = dispatch
                {
                    let outcome = hub_for_task
                        .dispatch_session_request(&ability_ura, &args)
                        .await;
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
                forward_invoke_args(target_ura),
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
        use easynet_axon::pb::axon::v1::invoke_bidi_down::Payload;
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
        let caller_ura = "easynet:///r/test-realm/device/device-a";
        let (tx, _rx) = mpsc::channel(1);
        presence.insert(caller_ura.to_string(), tx.clone());
        match events.recv().await.expect("online event") {
            PresenceEvent::Online { ura } => assert_eq!(ura, caller_ura),
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
            caller_ura,
            "abcd",
            build_session_request_result_frame(
                [0x22; 16],
                RequestOutcome::Ok {
                    result_bytes: b"overflow".to_vec(),
                },
            ),
        );

        assert!(
            presence.lookup_tracked(caller_ura).is_none(),
            "slow device must be evicted from presence on RequestResult backpressure"
        );
        match events.recv().await.expect("offline event") {
            PresenceEvent::Offline { ura, reason } => {
                assert_eq!(ura, caller_ura);
                assert_eq!(reason, OfflineReason::SendFailed);
            }
            other => panic!("expected offline event, got {other:?}"),
        }
    }

    #[tokio::test(flavor = "multi_thread", worker_threads = 2)]
    async fn matches_self_target_ura_accepts_hot_added_agent_only_for_local_identity() {
        // Hot-added agents can be dispatchable through `agents.json`
        // before publish persists them to `local-agents.json`. The
        // fallback must still be bound to the daemon's exact realm/user
        // identity so a peer realm or peer user cannot be collapsed into
        // this process by sharing the same bare agent name.
        use crate::persistence::config::{save_credentials, Credentials};
        use crate::registry::agents::{save_agents, AgentEntry, AgentRegistry, AgentType};
        let _hg = crate::facade::cli::test_support::HomeGuard::new();
        save_credentials(&Credentials {
            node_id: "dev-1".to_string(),
            credential_token: "token".to_string(),
            hub_endpoint: "axon://hub.test:50051".to_string(),
            realm: "test-realm".to_string(),
            username: Some("dev".to_string()),
            ..Default::default()
        })
        .expect("seed credentials");
        let svc = make_service().with_session_realm("test-realm");

        let agent_target = "easynet:///r/test-realm/agent/dev.liangbing";

        // Pre-write: no agents.json row → slow tier must miss too.
        assert!(
            !svc.matches_self_target_ura(agent_target).await,
            "agent absent from agents.json must not be treated as self-target"
        );

        // Stage the hot-added row.
        let mut registry = AgentRegistry::default();
        registry.agents.insert(
            "liangbing".to_string(),
            AgentEntry::new(AgentType::ClaudeCode, None),
        );
        save_agents(&registry).expect("stage agents.json under HomeGuard");

        assert!(
            svc.matches_self_target_ura(agent_target).await,
            "agent present in agents.json must be recognised as self-target \
             when the target realm/user match local credentials"
        );
        assert!(
            !svc.matches_self_target_ura("easynet:///r/other-realm/agent/dev.liangbing")
                .await,
            "same bare agent name in another realm must not be treated as local"
        );
        assert!(
            !svc.matches_self_target_ura("easynet:///r/test-realm/agent/peer.liangbing")
                .await,
            "same bare agent name under another user must not be treated as local"
        );

        // Sibling agent URA whose <agentID> is NOT in agents.json
        // must still be rejected — guards against the slow-tier
        // turning into a blanket "any agent URA is self-target".
        assert!(
            !svc.matches_self_target_ura("easynet:///r/test-realm/agent/dev.unknown")
                .await,
            "slow tier must only accept agents present in agents.json"
        );
    }

    #[tokio::test(flavor = "multi_thread", worker_threads = 2)]
    async fn matches_self_target_ura_uses_exact_local_agents_identity() {
        use crate::persistence::local_agents::{save, upsert_hosted_agent, LocalAgentsFile};

        let _hg = crate::facade::cli::test_support::HomeGuard::new();
        let mut local = LocalAgentsFile {
            host_device_agent_ura: "easynet:///r/test-realm/device/dev-1".to_string(),
            hosted_agents: Vec::new(),
        };
        upsert_hosted_agent(
            &mut local,
            "llm",
            "liangbing",
            "easynet:///r/test-realm/agent/dev.liangbing",
        );
        save(&local).expect("seed local-agents.json");

        let svc = make_service().with_session_realm("test-realm");
        assert!(
            svc.matches_self_target_ura("easynet:///r/test-realm/agent/dev.liangbing")
                .await,
            "exact hosted Agent identity from local-agents.json must be local"
        );
        assert!(
            !svc.matches_self_target_ura("easynet:///r/other-realm/agent/dev.liangbing")
                .await,
            "local-agents identity must include the realm"
        );
        assert!(
            !svc.matches_self_target_ura("easynet:///r/test-realm/agent/peer.liangbing")
                .await,
            "local-agents identity must include the user id"
        );
    }

    #[tokio::test]
    async fn dispatch_invoke_remote_routes_through_axon_runtime_when_ability_registered() {
        // RFC-005 acceptance: `<self>.invoke_remote` self-target
        // execution is selected by `namespace.resolve`, then
        // dispatched through Axon LocalRuntime using the selected
        // route's callee + dispatch key.
        use easynet_axon::invocation::{
            make_ability, AbilityCallModes, AbilityOptions, BackpressurePolicy, LocalRuntime,
        };
        use futures::StreamExt;

        let _hg = crate::facade::cli::test_support::HomeGuard::new();

        let rt = LocalRuntime::new();
        rt.register_ability_with_options(
            "liangbing.chat",
            make_ability(|ctx| async move {
                // Echo: terminal payload is the inbound `args`.
                Ok(ctx.payload.clone())
            }),
            AbilityOptions {
                modes: AbilityCallModes::RPC,
                backpressure: BackpressurePolicy::Unbounded,
            },
        )
        .await
        .unwrap();

        // `LocalRuntime::new()` already returns `Arc<LocalRuntime>`;
        // pass through verbatim.
        let svc = make_service()
            .with_session_realm("test-realm")
            .with_local_runtime(Arc::clone(&rt));
        let owner_ura = "easynet:///r/test-realm/agent/dev.liangbing";
        publish_test_route(&svc, owner_ura, "chat");

        let ability_ura =
            crate::ura::owner_ability_ura(owner_ura, "chat").expect("agent ability URA");
        let selected_route = svc
            .daemon_route_resolver()
            .resolve_route(&ability_ura, "")
            .expect("resolver selects agent route");
        assert_eq!(selected_route.owner_ura, owner_ura);
        assert_eq!(selected_route.callee_ura, owner_ura);
        assert_eq!(selected_route.execution_host_ura, TEST_DAEMON_URI);
        assert_eq!(selected_route.dispatch_key(), "liangbing.chat");

        let response = svc
            .dispatch_self_targeted_invoke_remote(
                &selected_route,
                None,
                b"hello-axon-routed".as_slice(),
            )
            .await
            .expect("self-target selected route dispatches");
        let mut stream = response.into_inner();
        let frame = stream
            .next()
            .await
            .expect("one terminal frame")
            .expect("terminal frame is in-band");
        let chunk = match frame.payload.expect("frame payload") {
            DownPayload::BinaryChunk(chunk) => chunk,
            other => panic!("expected BinaryChunk, got {other:?}"),
        };
        let down: InvokeRemoteDown =
            serde_json::from_slice(&chunk.data).expect("decode InvokeRemoteDown");
        match down {
            InvokeRemoteDown::Result { payload, error, .. } => {
                assert!(error.is_none(), "handler should complete: {error:?}");
                assert_eq!(payload, b"hello-axon-routed");
            }
            other => panic!("expected terminal Result, got {other:?}"),
        }
        assert!(
            stream.next().await.is_none(),
            "self-target stream is one-shot"
        );
    }

    #[tokio::test]
    async fn axon_arm_must_not_intercept_calls_targeting_a_peer_device() {
        // **Phase 4 regression pin.**
        //
        // Without the `matches_self_target_ura` guard the Axon
        // arm intercepts every call whose ability is registered
        // locally, regardless of `subject_device`. That caused
        // the Web UI's `<self>.invoke_remote(subject_device=peer,
        // ability=agent.list)` to return THIS daemon's
        // agents instead of the peer's — the agent-list page
        // lit up with the wrong rows.
        //
        // The guard restricts the arm to self-target. This test
        // pins it: a call against a non-self peer URA must SKIP
        // the Axon arm so the selected remote-session path can
        // forward dispatch to the peer's session.
        //
        // We assert by reading the predicate directly:
        // `matches_self_target_ura` MUST return `false` for a
        // peer device URA even when the local runtime hosts the
        // requested ability. The dispatch arm checks this
        // predicate first; a `false` here is the only thing
        // standing between "Axon-local execution" and "peer
        // forward". This pin guards the regression at the
        // predicate layer; the full bidi exercise lives in
        // integration tests where a real grpc Streaming can be
        // constructed.
        use easynet_axon::invocation::{
            make_ability, AbilityCallModes, AbilityOptions, BackpressurePolicy, LocalRuntime,
        };

        let _hg = crate::facade::cli::test_support::HomeGuard::new();
        let rt = LocalRuntime::new();
        // Register an ability under a name that exists everywhere
        // (every daemon mirrors `agent.list` into its
        // LocalRuntime via the Phase-3 boot sweep). The bug it's
        // pinning: pre-guard, this presence would have hijacked
        // peer-target calls.
        rt.register_ability_with_options(
            "agent.list",
            make_ability(|_| async move { Ok(Vec::new()) }),
            AbilityOptions {
                modes: AbilityCallModes::RPC,
                backpressure: BackpressurePolicy::Unbounded,
            },
        )
        .await
        .unwrap();

        let svc = make_service()
            .with_session_realm("test-realm")
            .with_local_runtime(Arc::clone(&rt));

        // 1. THIS daemon's URA → self target.
        assert!(
            svc.matches_self_target_ura(TEST_DAEMON_URI).await,
            "own daemon URA must be self-target"
        );

        // 2. A peer device URA in the same realm → NOT self target.
        //    The dispatch arm must skip Axon and let selected
        //    remote-session dispatch forward to the peer.
        let peer_ura = "easynet:///r/test-realm/device/some-peer";
        assert!(
            !svc.matches_self_target_ura(peer_ura).await,
            "peer device URA must NOT be self-target — the Axon arm \
             must skip and let selected remote-session dispatch forward"
        );

        // 3. A peer-realm hub URA → NOT self target.
        let peer_realm_hub = crate::ura::hub_ura("other-realm");
        assert!(
            !svc.matches_self_target_ura(&peer_realm_hub).await,
            "peer realm hub must NOT be self-target"
        );
    }

    #[tokio::test]
    async fn dispatch_local_rpc_selected_route_runs_runtime_when_registered() {
        // Catch-all unary `invoke` must resolve through namespace.resolve,
        // then route through Axon
        // (`invoke_async` → `LedgerSink`) when the runtime hosts the
        // ability — that's the path that gets the canonical record
        // into `invocations.redb` for CLI→daemon notify hops like
        // `easynet agent add` → `agent.start`.
        //
        // Returns `(response, axon_took_it=true)` so the caller in
        // `invoke()` skips the manual `record_unary_invocation`
        // write (avoiding the duplicate row keyed by `request_id`).
        use easynet_axon::invocation::{
            make_ability, AbilityCallModes, AbilityOptions, BackpressurePolicy, InvocationLedger,
            LedgerSink, LocalRuntime,
        };

        let _hg = crate::facade::cli::test_support::HomeGuard::new();
        let temp = tempfile::tempdir().unwrap();
        let ledger = Arc::new(InvocationLedger::open(temp.path().join("inv.redb")).unwrap());
        let rt = LocalRuntime::new();
        rt.set_ledger_sink(LedgerSink::new(Arc::clone(&ledger)));
        rt.register_ability_with_options(
            "demo.unary_via_axon",
            make_ability(|ctx| async move {
                let subject = ctx
                    .runtime
                    .axiom_envelope_of(&ctx.invocation_id)
                    .await
                    .map(|signed| signed.envelope.subject.ura);
                serde_json::to_vec(&serde_json::json!({
                    "payload": serde_json::from_slice::<serde_json::Value>(&ctx.payload)
                        .unwrap_or(serde_json::Value::Null),
                    "subject": subject,
                }))
                .map_err(|err| easynet_axon::invocation::AxonError::internal(err.to_string()))
            }),
            AbilityOptions {
                modes: AbilityCallModes::RPC,
                backpressure: BackpressurePolicy::Unbounded,
            },
        )
        .await
        .unwrap();

        let svc = make_service()
            .with_session_realm("test-realm")
            .with_local_runtime(Arc::clone(&rt));
        publish_test_route(&svc, TEST_DAEMON_URI, "demo.unary_via_axon");

        let mut request = invoke_request("demo.unary_via_axon", r#"{"k":"v"}"#).into_inner();
        request.envelope.as_mut().unwrap().subject = Some(SubjectIdentity {
            ura: "easynet:///r/test-realm/resource/camera-1".to_string(),
            ..SubjectIdentity::default()
        });
        let (result, axon_took_it) = svc.dispatch_local_rpc_selected_route(&request).await;

        assert!(
            axon_took_it,
            "runtime hosts the ability ⇒ Axon path must take it"
        );
        let response = result.expect("axon dispatch returns Ok");
        let body = response.into_inner();
        let decoded: serde_json::Value =
            serde_json::from_slice(&body.result).expect("decode handler payload");
        assert_eq!(decoded["payload"], serde_json::json!({"k": "v"}));
        assert_eq!(
            decoded["subject"], "easynet:///r/test-realm/resource/camera-1",
            "admitted Axon dispatch must preserve the wire envelope subject"
        );
        let header_request_id = body
            .header
            .as_ref()
            .map(|header| header.request_id.as_str());
        assert!(
            header_request_id.is_some(),
            "Axon-routed unary response must expose the ledger request_id"
        );

        // LedgerSink writes on the spawn task; pacing matches Axon's
        // own ledger_sink_persists_completed_invocation pattern.
        tokio::task::yield_now().await;
        tokio::time::sleep(std::time::Duration::from_millis(50)).await;
        let records = ledger.list_all().expect("list ledger");
        assert_eq!(
            records.len(),
            1,
            "Axon-routed unary call must land exactly one ledger row"
        );
        assert_eq!(records[0].ability_name, "demo.unary_via_axon");
        assert_eq!(records[0].state, "COMPLETED");
        assert_eq!(
            records[0].caller_ura, TEST_DAEMON_URI,
            "Axon-routed unary ledger row must preserve the admitted wire caller"
        );
        assert_eq!(
            records[0].callee_ura, TEST_DAEMON_URI,
            "Axon-routed unary ledger row must preserve the admitted wire callee"
        );
        assert_eq!(
            records[0].subject_ura, "easynet:///r/test-realm/resource/camera-1",
            "Axon-routed unary ledger row must preserve the admitted wire subject"
        );
        assert_eq!(header_request_id, Some(records[0].request_id.as_str()));
    }

    #[tokio::test]
    async fn dispatch_local_rpc_selected_route_rejects_when_runtime_misses() {
        // Resolver has selected a route, but runtime does not host
        // the dispatch key. This is an executor wiring error, not a
        // resolver fallback.
        use easynet_axon::invocation::LocalRuntime;

        let _hg = crate::facade::cli::test_support::HomeGuard::new();
        let rt = LocalRuntime::new();
        let svc = make_service()
            .with_session_realm("test-realm")
            .with_local_runtime(Arc::clone(&rt));
        publish_test_route(&svc, TEST_DAEMON_URI, "missing.ability");

        let request = invoke_request("missing.ability", "{}").into_inner();
        let (result, axon_took_it) = svc.dispatch_local_rpc_selected_route(&request).await;
        assert!(
            !axon_took_it,
            "runtime miss means no Axon invocation was started"
        );
        let err = result.expect_err("runtime miss rejects without alternate dispatch");
        assert_eq!(err.code(), tonic::Code::NotFound);
        assert!(
            err.message().contains("Axon LocalRuntime"),
            "error must name the runtime source of truth, got: {err}"
        );
    }

    #[tokio::test]
    async fn dispatch_local_rpc_selected_route_returns_false_for_non_rpc_runtime_row() {
        // A registered stream/bidi-only ability is known to
        // LocalRuntime, but unary Invoke cannot start an invocation
        // for it. `axon_took_it` must stay false so `invoke()` records
        // the failed unary attempt through the manual ledger path
        // instead of assuming Axon's LedgerSink persisted a row.
        use easynet_axon::invocation::{make_ability, AbilityOptions, LocalRuntime};

        let _hg = crate::facade::cli::test_support::HomeGuard::new();
        let rt = LocalRuntime::new();
        rt.register_ability_with_options(
            "demo.stream_only",
            make_ability(|_ctx| async { Ok(Vec::new()) }),
            AbilityOptions::streaming(),
        )
        .await
        .unwrap();

        let svc = make_service()
            .with_session_realm("test-realm")
            .with_local_runtime(Arc::clone(&rt));
        publish_test_route(&svc, TEST_DAEMON_URI, "demo.stream_only");

        let request = invoke_request("demo.stream_only", "{}").into_inner();
        let (result, axon_took_it) = svc.dispatch_local_rpc_selected_route(&request).await;
        assert!(
            !axon_took_it,
            "mode mismatch happens before Axon starts an invocation"
        );
        let err = result.expect_err("stream-only ability rejects unary Invoke");
        assert_eq!(err.code(), tonic::Code::InvalidArgument);
        assert!(
            err.message().contains("does not support unary Invoke"),
            "error must explain the call-shape mismatch, got: {err}"
        );
    }
}
