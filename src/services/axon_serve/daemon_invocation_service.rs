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
use std::path::PathBuf;
use std::pin::Pin;
use std::sync::Arc;

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
    BinaryChunk, Envelope, EnvelopeOpen, InvocationReceipt, InvocationState, InvokeBidiDown,
    InvokeBidiUp, InvokeRequest, InvokeResponse, InvokeServerStreamRequest, InvokeStreamChunk,
};
use crate::services::axon_serve::admission_facade::AdmissionFacade;
use crate::services::axon_serve::federation_wrappers::{
    self, ABILITY_FEDERATION_ADVERTISE_AGENT, ABILITY_FEDERATION_DISCOVER,
    ABILITY_FEDERATION_FORWARD_INVOKE, ABILITY_FEDERATION_HEARTBEAT, ABILITY_FEDERATION_JOIN,
    ABILITY_FEDERATION_LIST_USER_DEVICES, ABILITY_FEDERATION_RESOLVE,
    ABILITY_FEDERATION_RESOLVE_KEY, ABILITY_FEDERATION_REVOKE,
    ABILITY_FEDERATION_SUBSCRIBE_DIRECTORY, ABILITY_FEDERATION_SUBSCRIBE_DIRECTORY_V2,
};
use crate::services::federated_peers_cell::SharedFederatedPeers;
use crate::services::federation_client::FederationClient;
use crate::services::axon_serve::invoke_remote_initiator::{
    InvokeRemoteDown, InvokeRemoteUp, SessionDispatch, ABILITY_INVOKE_REMOTE,
    INVOKE_REMOTE_STREAM_ID,
};
use crate::services::axon_serve::register_device_pubkey::{
    handle as handle_register_device_pubkey, parse_realm_from_uri,
    ABILITY_SELF_REGISTER_DEVICE_PUBKEY,
};
use crate::services::axon_serve::session_initiator::ABILITY_SELF_SESSION;
use crate::services::pending_dispatch::{DispatchResult, PendingDispatchMap};
use crate::services::presence_registry::{
    DispatchFrame, DispatchSender, OfflineReason, PresenceRegistry, DISPATCH_CHANNEL_CAPACITY,
};
use crate::services::trust_anchor_cell::SharedTrustAnchor;

/// Content type the federation wrappers emit on `InvokeResponse.result`.
/// Centralised here so call sites cannot drift away from the value
/// PR-4's baselines expect.
const FEDERATION_RESULT_CONTENT_TYPE: &str = "application/json";

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
    federated_directory:
        crate::services::federation_directory::SharedFederatedDirectoryView,
    /// **N3-N4 bridge**. Daemon-wide federated user binding
    /// store. When wired, the `federation.discover` dispatch
    /// arm constructs a `FederatedUserResolver` per call and
    /// filters cross-realm entries through it whenever the
    /// request supplies a `local_user_id`. `None` ⇒ no filter
    /// (operator query path). Production daemons attach this
    /// at boot via `with_federated_bindings_store`.
    federated_bindings: Option<
        std::sync::Arc<crate::runtime::keyring::federated_bindings::FederatedBindingsStore>,
    >,
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
                "federation_client",
                &self.federation_client.as_ref().map(|_| "<dyn FederationClient>"),
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
            admission,
            pending: None,
            register_pubkey: None,
            session_realm: None,
            federation_client: None,
            federated_peers: SharedFederatedPeers::default(),
            federated_directory:
                crate::services::federation_directory::SharedFederatedDirectoryView::default(),
            federated_bindings: None,
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
        self.pending = Some(pending);
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
            ABILITY_FEDERATION_HEARTBEAT => self.dispatch_federation_heartbeat(&inner.arguments),
            ABILITY_FEDERATION_RESOLVE => self.dispatch_federation_resolve(&inner.arguments),
            ABILITY_FEDERATION_RESOLVE_KEY => {
                self.dispatch_federation_resolve_key(&inner.arguments)
            }
            ABILITY_FEDERATION_DISCOVER => self.dispatch_federation_discover(&inner.arguments),
            ABILITY_FEDERATION_LIST_USER_DEVICES => self
                .dispatch_federation_list_user_devices(
                    inner.envelope.as_ref(),
                    &inner.arguments,
                ),
            ABILITY_FEDERATION_REVOKE => self.dispatch_federation_revoke(&inner.arguments),
            ABILITY_FEDERATION_FORWARD_INVOKE => {
                self.dispatch_federation_forward_invoke(
                    inner.envelope.as_ref(),
                    &inner.arguments,
                )
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

        let envelope_open = extract_envelope_open(&frame0)?;
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
            other => Err(Status::unimplemented(format!(
                "easynet-daemon: InvokeBidi ability `{other}` is not yet wired; \
                 LocalAbilityRegistry bidi fallback is the next staging step \
                 (see team-work/checklists/PR-2-checklist.md)"
            ))),
        }
    }
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
        let response = federation_wrappers::handle_advertise_agent(&request);
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
        let response = federation_wrappers::handle_resolve(&request, &self.presence);
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
                let resolver =
                    crate::runtime::keyring::resolver::FederatedUserResolver::new(
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
        let response = federation_wrappers::handle_revoke(&request, &self.presence);
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
        let request: federation_wrappers::ForwardInvokeRequest = parse_json_args(arguments)?;

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
        if is_local_tenant {
            // DEC-N5 §1 dual-write — caller hub records the
            // ForwardReceipt eagerly, before the frame push, so a
            // mid-push panic still leaves an audit breadcrumb. On
            // the local fast-path the actual reply flows back
            // through the reverse-channel correlation path; we
            // therefore stamp `result_digest = None` (empty
            // payload) at the point of forward — PR-N5's audit
            // chain extension will append a second receipt with
            // the digest when the reverse-channel reply lands.
            match self.try_push_forward_invoke_frame(&request) {
                Ok(()) => {
                    self.admission.receipt_store().record(build_forward_receipt(
                        &correlation_call_id,
                        &request.target_uri,
                        caller_envelope,
                        None,
                    ));
                    let response = federation_wrappers::ForwardInvokeResponse {
                        result_bytes: Vec::new(),
                        correlation_call_id,
                    };
                    return wrap_json_response(&response);
                }
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
                            let peer_envelope =
                                build_peer_envelope(caller_envelope, &request.target_uri);
                            let peer_request = InvokeRequest {
                                envelope: Some(peer_envelope),
                                function_name: inner_payload.ability.clone(),
                                arguments: inner_payload.args_bytes.clone(),
                                ..InvokeRequest::default()
                            };
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
                                        self.admission.receipt_store().record(
                                            build_forward_receipt(
                                                &correlation_call_id,
                                                &request.target_uri,
                                                caller_envelope,
                                                Some(&peer_response.result),
                                            ),
                                        );
                                        let response = federation_wrappers::
                                            ForwardInvokeResponse {
                                                result_bytes: peer_response.result.clone(),
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

        // Cross-tenant dial. Build a real `InvokeRequest` from
        // the unwrapped inner payload (caller's `(ability,
        // args)` pair) and attach the original caller envelope
        // so the peer's admission gate sees the user's
        // identity. The peer's admission gate verifies the
        // caller URI against its local `realm-trust.toml`; for
        // PR-N1's same-account same-tenant scope the backend
        // tenant fix ensures both daemons store the same
        // `(tenant_id, agent_uri, public_key)` triple, so the
        // forwarded envelope is accepted. PR-N2's
        // FederatedKeyResolver lifts that limitation for
        // genuine cross-realm callers.
        let peer_envelope = build_peer_envelope(caller_envelope, &request.target_uri);
        let peer_request = InvokeRequest {
            envelope: Some(peer_envelope),
            function_name: inner_payload.ability,
            arguments: inner_payload.args_bytes,
            ..InvokeRequest::default()
        };

        match client.forward_invoke(target_hub_uri, peer_request).await {
            Ok(peer_response) => {
                // C1a: thread the peer's response bytes through
                // the DEC-N4 §2.1 wire shape. The peer-side
                // dispatcher returns its ability handler's full
                // `InvokeResponse`; we forward the `result`
                // bytes verbatim and stamp the caller's
                // `correlation_call_id` so the CLI initiator
                // can correlate.
                //
                // DEC-N5 §1 dual-write: caller hub records a
                // ForwardReceipt with the SHA-256 of the actual
                // result bytes, linking to the target hub's
                // InvocationReceipt by `child_invocation_id =
                // correlation_call_id`.
                self.admission.receipt_store().record(build_forward_receipt(
                    &correlation_call_id,
                    &request.target_uri,
                    caller_envelope,
                    Some(&peer_response.result),
                ));
                let response = federation_wrappers::ForwardInvokeResponse {
                    result_bytes: peer_response.result.clone(),
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
    fn try_push_forward_invoke_frame(
        &self,
        request: &federation_wrappers::ForwardInvokeRequest,
    ) -> Result<(), Status> {
        let Some(sender) = self.presence.lookup(&request.target_uri) else {
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
                self.presence.remove(
                    &request.target_uri,
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
                self.presence.remove(
                    &request.target_uri,
                    crate::services::presence_registry::OfflineReason::StreamClosed,
                );
                Err(Status::failed_precondition(
                    federation_wrappers::FORWARD_INVOKE_TARGET_OFFLINE_REASON,
                ))
            }
        }
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

        let initial_evt = federation_wrappers::build_subscribe_directory_v2_snapshot(
            &self.presence,
        );
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

        let initial_stream = futures::stream::once(async move { Ok(initial_chunk) });
        let event_stream = futures::stream::unfold(
            (events, presence_weak),
            |(mut events, presence_weak)| async move {
                use tokio::sync::broadcast::error::RecvError;

                loop {
                    match events.recv().await {
                        Ok(event) => {
                            let evt = presence_event_to_directory_event(&event);
                            // DirectoryEvent is statically Serialize
                            // (tagged enum of plain types); same
                            // expect-rationale as v1.
                            let payload = serde_json::to_vec(&evt).expect(
                                "DirectoryEvent is statically Serialize; a serialise failure here \
                                 means the type grew a fallible field — update this site to \
                                 surface Status::internal instead of panicking",
                            );
                            let chunk = InvokeStreamChunk {
                                content_type: FEDERATION_RESULT_CONTENT_TYPE.to_string(),
                                payload,
                                ..InvokeStreamChunk::default()
                            };
                            return Some((Ok(chunk), (events, presence_weak)));
                        }
                        Err(RecvError::Lagged(_)) => {
                            // Slow consumer; emit a fresh
                            // Snapshot so the receiver's view
                            // converges with the registry.
                            let presence = presence_weak.upgrade()?;
                            let snap_evt =
                                federation_wrappers::build_subscribe_directory_v2_snapshot(
                                    &presence,
                                );
                            drop(presence);
                            let payload = serde_json::to_vec(&snap_evt).expect(
                                "DirectoryEvent::Snapshot is statically Serialize; same rationale \
                                 as the Ok arm above",
                            );
                            // Per spec §2.3 a second Snapshot
                            // mid-stream is a protocol violation
                            // for the *subscriber*; our case is
                            // the recovery resync — the receiver
                            // either treats this Snapshot as
                            // authoritative replacement (matches
                            // the SubscriberFsm Pumping ⇢
                            // Snapshot ⇢ violation rule) or
                            // tears down + reconnects. Lagged is
                            // a transient slow-consumer event;
                            // emitting Snapshot here is the v1
                            // contract carried forward.
                            let _ = DirectoryEvent::Snapshot { entries: vec![] };
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

        let pending = self.pending.as_ref().ok_or_else(|| {
            Status::failed_precondition(
                "<self>.invoke_remote: daemon was constructed without a \
                 PendingDispatchMap; call DaemonInvocationService::with_pending(...) \
                 at boot to enable cross-device invocation",
            )
        })?;

        let target_sender = self.presence.lookup(&subject_device).ok_or_else(|| {
            Status::not_found(format!(
                "<self>.invoke_remote: target `{subject_device}` is not in PresenceRegistry; \
                 either offline or never connected to this hub"
            ))
        })?;

        // Register pending entry BEFORE pushing the dispatch frame —
        // otherwise the target could reply faster than we can register
        // and the reply would land as a no-op `complete`.
        let handle = pending.register_pending();
        let call_id = handle.call_id();

        let dispatch_frame = build_invoke_remote_dispatch_frame(call_id, &ability, &args)?;
        match target_sender.try_send(Ok(dispatch_frame)) {
            Ok(()) => {}
            Err(tokio::sync::mpsc::error::TrySendError::Full(_)) => {
                // Bounded backpressure → presence transition (same
                // policy as forward_invoke commit 8/9).
                self.presence
                    .remove(&subject_device, OfflineReason::SendFailed);
                return Err(Status::failed_precondition(format!(
                    "<self>.invoke_remote: target `{subject_device}` channel full; \
                     removed from registry with OfflineReason::SendFailed"
                )));
            }
            Err(tokio::sync::mpsc::error::TrySendError::Closed(_)) => {
                self.presence
                    .remove(&subject_device, OfflineReason::StreamClosed);
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
        validate_session_realm(&caller_uri, self.session_realm.as_deref())?;

        let (down_tx, down_rx): (DispatchSender, _) =
            mpsc::channel::<Result<DispatchFrame, Status>>(DISPATCH_CHANNEL_CAPACITY);

        // Step 1: register before spawning so a SessionDispatch::Dispatch
        // arriving from `<self>.invoke_remote` immediately can find this
        // sender. The PresenceRegistry handles displacement (Offline +
        // Online emission ordering) under the hood.
        let _displaced = self.presence.insert(caller_uri.clone(), down_tx);

        // Step 2: spawn the up-stream consumer. Reads device replies
        // (SessionDispatch::Result frames) and routes them to the
        // PendingDispatchMap so the originating <self>.invoke_remote
        // caller wakes up.
        let presence_for_drain = Arc::clone(&self.presence);
        let pending_for_drain = self.pending.clone();
        let caller_uri_for_drain = caller_uri.clone();
        tokio::spawn(async move {
            drain_session_up_stream(
                up,
                caller_uri_for_drain,
                presence_for_drain,
                pending_for_drain,
            )
            .await
        });

        // Step 3: hand the down stream to tonic. Frames arrive in
        // `down_tx` from <self>.invoke_remote dispatchers and from
        // federation.forward_invoke pushers as `DispatchFrame`
        // (presence_registry's newtype around `InvokeBidiDown`).
        // The tonic trait wants raw `InvokeBidiDown`, so map each
        // frame to unwrap the newtype.
        let stream = ReceiverStream::new(down_rx).map(|item| match item {
            Ok(frame) => Ok(frame.frame),
            Err(status) => Err(status),
        });
        Ok(Response::new(
            Box::pin(stream) as BoxedDownStream<InvokeBidiDown>
        ))
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
    presence: Arc<PresenceRegistry>,
    pending: Option<Arc<PendingDispatchMap>>,
) {
    use crate::pb::axon::v1::invoke_bidi_up::Payload as UpPayload;

    let mut close_reason = OfflineReason::StreamClosed;

    while let Some(frame_result) = up.next().await {
        let frame = match frame_result {
            Ok(f) => f,
            Err(status) => {
                eprintln!(
                    "[session-accept] up-stream error for {caller_uri}: {status}; \
                     removing from registry"
                );
                close_reason = OfflineReason::StreamReset;
                break;
            }
        };

        let chunk = match frame.payload {
            Some(UpPayload::BinaryChunk(c)) => c,
            Some(other) => {
                eprintln!(
                    "[session-accept] {caller_uri} sent non-BinaryChunk up frame: {other:?}; \
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
                terminal: _terminal,
                error,
            } => {
                let Some(pending) = pending.as_ref() else {
                    eprintln!(
                        "[session-accept] {caller_uri} sent Result for call_id={call_id} but \
                         daemon was constructed without PendingDispatchMap; ignoring"
                    );
                    continue;
                };
                let dispatch_result = DispatchResult { payload, error };
                let completed = pending.complete(call_id, dispatch_result);
                if !completed {
                    eprintln!(
                        "[session-accept] {caller_uri} sent Result for call_id={call_id} but \
                         no pending entry matched (caller may have cancelled); silent no-op"
                    );
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
        }
    }

    presence.remove(&caller_uri, close_reason);
    eprintln!(
        "[session-accept] {caller_uri} session ended ({:?}); removed from registry",
        close_reason
    );
}

fn validate_session_realm(caller_uri: &str, session_realm: Option<&str>) -> Result<(), Status> {
    let Some(daemon_realm) = session_realm else {
        return Ok(());
    };

    let caller_realm = parse_realm_from_uri(caller_uri).ok_or_else(|| {
        Status::invalid_argument(format!(
            "<self>.session: caller URI `{caller_uri}` does not match the canonical \
             `easynet:///r/{{realm}}/agent/{{node}}` shape"
        ))
    })?;

    if caller_realm != daemon_realm {
        return Err(Status::permission_denied(format!(
            "<self>.session: caller realm `{caller_realm}` does not match daemon realm \
             `{daemon_realm}`; cross-realm session is blocked until RFC-N PR-N2 ships"
        )));
    }

    Ok(())
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
pub(crate) fn build_peer_envelope(caller_envelope: Option<&Envelope>, target_uri: &str) -> Envelope {
    if let Some(env) = caller_envelope {
        return env.clone();
    }
    Envelope {
        caller: Some(AgentIdentity {
            uri: target_uri.to_string(),
            ..AgentIdentity::default()
        }),
        ..Envelope::default()
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
    const TEST_DAEMON_URI: &str = "easynet:///r/test-realm/agent/test-daemon";

    fn make_service() -> DaemonInvocationService {
        let admission = AdmissionFacade::new(
            Arc::new(RealmTrustAnchor::default()),
            Some(TEST_DAEMON_URI.to_string()),
        );
        DaemonInvocationService::new(Arc::new(PresenceRegistry::new()), admission)
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
                    source_user_pubkey_b64:
                        "AAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAA=".to_string(),
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
        let admission = AdmissionFacade::new(
            Arc::new(anchor_inner),
            Some(TEST_DAEMON_URI.to_string()),
        );
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
                assert!(entries.is_empty(), "initial snapshot must reflect empty registry");
            }
            other => panic!("expected Snapshot first; got {other:?}"),
        }

        // Frame 2: Upsert after a registry insert.
        let (sender, _rx) = tokio::sync::mpsc::channel::<
            Result<crate::services::presence_registry::DispatchFrame, tonic::Status>,
        >(1);
        presence.insert(
            "easynet:///r/test-realm/agent/n1".to_string(),
            sender,
        );
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
    fn validate_session_realm_accepts_same_realm() {
        validate_session_realm("easynet:///r/realm-a/agent/device-1", Some("realm-a"))
            .expect("same-realm caller must pass");
    }

    #[test]
    fn validate_session_realm_rejects_cross_realm() {
        let err = validate_session_realm("easynet:///r/realm-b/agent/device-1", Some("realm-a"))
            .expect_err("cross-realm caller must be rejected");
        assert_eq!(err.code(), tonic::Code::PermissionDenied);
        assert!(err.message().contains("cross-realm session"));
    }

    #[test]
    fn validate_session_realm_rejects_malformed_uri() {
        let err = validate_session_realm("not-a-ura", Some("realm-a"))
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
        recorded: std::sync::Mutex<Vec<(crate::services::federation_client::HubUri, InvokeRequest)>>,
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
        ) -> Result<InvokeResponse, crate::services::federation_client::FederationClientError> {
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
        use base64::{engine::general_purpose::STANDARD, Engine as _};
        let inner = serde_json::json!({
            "ability": "observe.health",
            "args": {},
            "call_id": "test-call-id-1",
        });
        let inner_b64 = STANDARD.encode(serde_json::to_vec(&inner).unwrap());
        format!(
            r#"{{"target_uri":"{target_uri}","inner_envelope_b64":"{inner_b64}"}}"#
        )
        .into_bytes()
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
            .dispatch_federation_forward_invoke(None, &forward_invoke_args(
                "easynet:///r/test-realm/agent/local-target",
            ))
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
            .dispatch_federation_forward_invoke(None, &forward_invoke_args(
                "easynet:///r/peer-realm/agent/peer-target",
            ))
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
            .dispatch_federation_forward_invoke(None, &forward_invoke_args(
                "easynet:///r/unmapped-realm/agent/peer-target",
            ))
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

        let target_uri = "easynet:///r/peer-realm/agent/peer-target";
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
        // PR-N1 commit 11/N: peer dispatcher receives the
        // inner ability (decoded from the CLI bridge's
        // `inner_envelope_b64`), not the `federation.forward_
        // invoke` wrapper. The fixture's b64 payload is
        // `{"ability":"observe.health","args":{}}` so the
        // peer sees `function_name = "observe.health"`.
        assert_eq!(
            calls[0].1.function_name, "observe.health",
            "peer dispatcher receives the inner ability decoded from inner_envelope_b64"
        );
        // Inner args re-serialised as JSON; equivalent shapes
        // (e.g. {} vs whitespace) compare via parsed JSON.
        let parsed_args: serde_json::Value =
            serde_json::from_slice(&calls[0].1.arguments).expect("inner args parse");
        assert_eq!(parsed_args, serde_json::json!({}));
        // PR-N1 commit 11/N: when the original CLI request
        // carries no envelope (this test passes None), the
        // dispatcher synthesises a minimal envelope with
        // `caller.uri = target_uri` so the peer's URI-only
        // Device admission arm under DEC-013 admits.
        let peer_envelope = calls[0].1.envelope.as_ref().expect("envelope present");
        let peer_caller = peer_envelope
            .caller
            .as_ref()
            .expect("caller identity present");
        assert_eq!(peer_caller.uri, target_uri);
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

        let target_uri = "easynet:///r/peer-realm/agent/peer-target";
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
            .dispatch_federation_forward_invoke(None, &forward_invoke_args(
                "easynet:///r/peer-realm/agent/peer-target",
            ))
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
            .dispatch_federation_forward_invoke(None, &forward_invoke_args(
                "easynet:///r/test-realm/agent/local-target",
            ))
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
        const TARGET_DEVICE_URI: &str = "easynet:///r/realm-b/agent/target-device";
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
        let daemon_b_anchor = Arc::new(
            RealmTrustAnchor::from_entries(daemon_a_in_b_trust)
                .expect("anchor"),
        );

        // Daemon B: presence registry contains the target device;
        // try_send-into-an-mpsc surfaces `target_online: true` if
        // the channel still has receiver capacity. We hold the
        // receiver to keep it open for the test's lifetime.
        let daemon_b_presence = Arc::new(PresenceRegistry::new());
        let (target_tx, _target_rx) = tokio::sync::mpsc::channel(8);
        daemon_b_presence
            .insert(TARGET_DEVICE_URI.to_string(), target_tx);

        let daemon_b_admission =
            AdmissionFacade::new(daemon_b_anchor, Some(DAEMON_B_URI.to_string()));
        let daemon_b = Arc::new(
            DaemonInvocationService::new(daemon_b_presence, daemon_b_admission)
                .with_session_realm(REALM_B),
        );

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
        let federation_client: Arc<dyn FederationClient> = Arc::new(
            ForwardingPeerClient {
                peer: daemon_b,
                envelope: test_envelope_with_uri(DAEMON_A_URI),
            },
        );
        let mut peers = BTreeMap::new();
        peers.insert(REALM_B.to_string(), PEER_HUB_URI.to_string());

        let daemon_a = DaemonInvocationService::new(
            Arc::new(PresenceRegistry::new()),
            daemon_a_admission,
        )
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

        // ── Assert: daemon B handled the inner ability and ──
        // C1a / DEC-N4 §2.1 wire shape: the outer InvokeResponse
        // body carries a `ForwardInvokeResponse {result_bytes,
        // correlation_call_id}` JSON. `result_bytes` is the
        // peer's ability-handler reply bytes (themselves JSON
        // for `federation.heartbeat`), and
        // `correlation_call_id` echoes the caller's id from the
        // inner payload. We assert both layers.
        let outer: federation_wrappers::ForwardInvokeResponse =
            serde_json::from_slice(&body.result)
                .expect("outer ForwardInvokeResponse is JSON");
        assert_eq!(outer.correlation_call_id, "e2e-call-id-1");
        let inner: serde_json::Value = serde_json::from_slice(&outer.result_bytes)
            .expect("inner peer ability response is JSON");
        assert!(
            inner.is_object(),
            "expected JSON object from federation.heartbeat handler, got: {inner}"
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
        ) -> Result<InvokeResponse, crate::services::federation_client::FederationClientError> {
            request.envelope = Some(self.envelope.clone());
            let response = self
                .peer
                .invoke(Request::new(request))
                .await
                .map_err(|status| {
                    crate::services::federation_client::FederationClientError::InnerInvokeFailed {
                        hub: "in-process-peer".to_string(),
                        status: format!(
                            "code={:?} message={}",
                            status.code(),
                            status.message()
                        ),
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
}
