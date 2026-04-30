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
// 莫浩 hit when bringing both `futures::StreamExt` and
// `tokio_stream::StreamExt` into scope.
use futures::StreamExt as _;
use tokio::sync::mpsc;
use tokio_stream::wrappers::ReceiverStream;
use tonic::{Request, Response, Status, Streaming};

use crate::pb::axon::v1::invocation_server::Invocation;
use crate::pb::axon::v1::{
    invoke_bidi_down::Payload as DownPayload, invoke_bidi_up::Payload as UpPayload, BinaryChunk,
    EnvelopeOpen, InvocationState, InvokeBidiDown, InvokeBidiUp, InvokeRequest, InvokeResponse,
    InvokeServerStreamRequest, InvokeStreamChunk,
};
use crate::services::axon_serve::admission_facade::AdmissionFacade;
use crate::services::axon_serve::federation_wrappers::{
    self, ABILITY_FEDERATION_ADVERTISE_AGENT, ABILITY_FEDERATION_FORWARD_INVOKE,
    ABILITY_FEDERATION_HEARTBEAT, ABILITY_FEDERATION_JOIN, ABILITY_FEDERATION_RESOLVE,
    ABILITY_FEDERATION_REVOKE, ABILITY_FEDERATION_SUBSCRIBE_DIRECTORY,
};
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
    /// **PR-N1 commit 3a/N**. Operator-curated `tenant → hub_uri`
    /// map per `DaemonConfig::federated_peers`. Empty map ⇒ no
    /// cross-tenant routing configured; the dispatcher returns
    /// the legacy shape. PR-N3 will replace this hand-curated
    /// map with auto-discovered cross-realm directory entries.
    federated_peers: BTreeMap<String, String>,
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
            .field("federated_peers_count", &self.federated_peers.len())
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
            federated_peers: BTreeMap::new(),
        }
    }

    /// Attach a `PendingDispatchMap` for `<self>.invoke_remote`
    /// dispatch correlation. Builder-style so existing
    /// `DaemonInvocationService::new(presence, admission)` callers
    /// stay source-compatible.
    ///
    /// PR-3 ownership: 海峰 (this commit). PR-2's `<self>.session`
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
    /// `tenant → hub_uri` map. Empty map (the default from
    /// `DaemonInvocationService::new`) means no cross-tenant
    /// routing is configured; the dispatcher's cross-tenant arm
    /// then refuses to dial regardless of `federation_client`
    /// presence — peer-not-trusted by absence of operator intent.
    #[must_use]
    pub fn with_federated_peers(mut self, peers: BTreeMap<String, String>) -> Self {
        self.federated_peers = peers;
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
            ABILITY_FEDERATION_REVOKE => self.dispatch_federation_revoke(&inner.arguments),
            ABILITY_FEDERATION_FORWARD_INVOKE => {
                self.dispatch_federation_forward_invoke(&inner.arguments)
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
    /// - `<self>.session` → PR-2 (莫浩); arm added when PR-2 lands
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

    fn dispatch_federation_revoke(
        &self,
        arguments: &[u8],
    ) -> Result<Response<InvokeResponse>, Status> {
        let request: federation_wrappers::RevokeRequest = parse_json_args(arguments)?;
        let response = federation_wrappers::handle_revoke(&request, &self.presence);
        wrap_json_response(&response)
    }

    fn dispatch_federation_forward_invoke(
        &self,
        arguments: &[u8],
    ) -> Result<Response<InvokeResponse>, Status> {
        let request: federation_wrappers::ForwardInvokeRequest = parse_json_args(arguments)?;
        let target_online = self.try_push_forward_invoke_frame(&request)?;
        let response = federation_wrappers::ForwardInvokeResponse { target_online };
        wrap_json_response(&response)
    }

    /// Real reverse-channel push for `federation.forward_invoke`.
    ///
    /// Looks up `request.target_uri` in the presence registry and
    /// pushes a `BinaryChunk` containing the inner-envelope bytes
    /// down the target's `<self>.session` `DispatchSender`.
    /// Returns `Ok(true)` when the frame was queued for delivery,
    /// `Ok(false)` when the target was offline, and
    /// `failed_precondition` when the dispatch sender's channel is
    /// full (treated as offline-by-backpressure per spec §3
    /// Invariant 4 — slow consumer is removed and the call surfaces
    /// the eviction).
    ///
    /// PR-1 staging keeps the JSON response shape
    /// `{ target_online: bool }` rather than the spec-§4-final
    /// `{ result_bytes, correlation_call_id }` shape — DEC-003
    /// Reading A pinned the staging shape; the final shape lands
    /// alongside PR-3's `<self>.invoke_remote` per-call dispatch
    /// because the correlated reply path needs the
    /// `pending_dispatch` correlation table that PR-3 introduces.
    fn try_push_forward_invoke_frame(
        &self,
        request: &federation_wrappers::ForwardInvokeRequest,
    ) -> Result<bool, Status> {
        let Some(sender) = self.presence.lookup(&request.target_uri) else {
            return Ok(false);
        };

        let inner_bytes = decode_inner_envelope(&request.inner_envelope_b64)?;
        let frame = build_forward_invoke_dispatch_frame(inner_bytes);

        match sender.try_send(Ok(frame)) {
            Ok(()) => Ok(true),
            Err(tokio::sync::mpsc::error::TrySendError::Full(_)) => {
                // Bounded backpressure (Invariant 4 in
                // `services::presence_registry`). A full channel is
                // a slow consumer; the canonical recovery is to
                // remove the device with `OfflineReason::SendFailed`
                // — that emits the matching presence event and
                // future calls observe a clean `target_online=false`.
                self.presence.remove(
                    &request.target_uri,
                    crate::services::presence_registry::OfflineReason::SendFailed,
                );
                Err(Status::failed_precondition(format!(
                    "federation.forward_invoke: target `{}` channel full; \
                     removed from registry with OfflineReason::SendFailed",
                    request.target_uri,
                )))
            }
            Err(tokio::sync::mpsc::error::TrySendError::Closed(_)) => {
                // Receiver dropped without explicit removal — the
                // channel is dead. Symmetric removal so the next
                // lookup returns None.
                self.presence.remove(
                    &request.target_uri,
                    crate::services::presence_registry::OfflineReason::StreamClosed,
                );
                Ok(false)
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

/// Wrap the inner envelope bytes into a `DispatchFrame` heading
/// down a target's `<self>.session` reverse channel.
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
        let svc = make_service();
        let resp = svc
            .invoke(invoke_request(
                ABILITY_FEDERATION_FORWARD_INVOKE,
                r#"{"target_uri":"easynet:///r/realm/agent/missing","inner_envelope_b64":""}"#,
            ))
            .await
            .expect("dispatch returns Ok");
        let body: federation_wrappers::ForwardInvokeResponse = parse_response_body(resp);
        assert!(!body.target_online);
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
        assert!(svc.federated_peers.is_empty());

        let mut peers = BTreeMap::new();
        peers.insert(
            "peer-realm".to_string(),
            "https://peer-hub.example:50443".to_string(),
        );
        let svc = svc.with_federated_peers(peers);
        assert_eq!(svc.federated_peers.len(), 1);
        assert_eq!(
            svc.federated_peers.get("peer-realm").map(String::as_str),
            Some("https://peer-hub.example:50443")
        );
    }
}
