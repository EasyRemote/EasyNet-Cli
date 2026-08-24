// EasyNet Daemon — InvokeBidi Dispatcher
// ========================================
//
// File: src/daemon/invocation/bidi/bidi_dispatcher.rs
// Description: Owns generic `InvokeBidi` routing and the product providers
//              installed behind exact descriptor-bound routes:
//
//                * typed `DispatchCall` / `ReverseDispatchCall` relay over a
//                  device's session reverse channel
//                * `session.open` - Authority-owned presence and carrier provider
//                * plugin/builtin bidi wire abilities — local PTY/
//                  file-transfer adapters and the remote bidi bridge
//
//              Also owns the bidi wire furniture: frame-0 validation,
//              terminal/admission receipt builders, the local bidi
//              frame mappers, and the session/local down-stream types.
//
//              Composes `UnaryDispatcher` for generic bidi route selection and
//              reverse session requests. Exact-route admission belongs to
//              LocalRuntime.
//
// Author: Silan Hu <silan.hu@u.nus.edu>
// Copyright (c) 2026 EasyNet. All rights reserved.

use futures::Stream;
use prost::Message as _;
use std::collections::{HashMap, VecDeque};
use std::pin::Pin;
use std::sync::{Arc, Mutex};
use std::task::{Context, Poll};
use tokio::sync::mpsc;
use tokio_stream::StreamExt;
use tokio_util::sync::CancellationToken;
use tonic::{Response, Status, Streaming};

use axon_sdk::pb::axon::v1::{
    bidi_control, invoke_bidi_down::Payload as DownPayload, invoke_bidi_up::Payload as UpPayload,
    BidiControl, BidiSessionEstablished, BinaryChunk, EnvelopeOpen, InvokeBidiDown, InvokeBidiUp,
    InvokeRequest, StreamDescriptor,
};

use crate::core::ura::realm_from_ura;
use crate::daemon::invocation::admission::admission_facade::AdmissionFacade;
use crate::daemon::invocation::admission::hosted_agent_delegation::{
    HostedAgentDelegationIngress, HostedAgentDelegationIssuer,
};
use crate::daemon::invocation::admission::target_gate::{
    route_negative_status, route_profile_blocked_status, signed_envelope_for_selected_route,
    TargetGate,
};
use crate::daemon::invocation::bidi::session_wire::{
    build_canonical_dispatch_frame, call_id_hex, canonical_dispatch_call_mode,
    require_canonical_dispatch_session, RequestOutcome, SessionContentEnvelope, SessionDispatch,
    SessionRequestError,
};
use crate::daemon::invocation::bidi::state::presence::CANONICAL_SESSION_CARRIER_VERSION;
use crate::daemon::invocation::dispatch::cancellation::RegisteredInvocationLifecycle;
use crate::daemon::invocation::dispatch::daemon_invocation_service::{
    DaemonBidiRoute, DAEMON_INVOCATION_BIDI_ROUTES,
};
use crate::daemon::invocation::dispatch::daemon_route_runtime::{
    runtime_status_to_axon_error, SESSION_OPEN_EXT_METADATA_KEY,
};
use crate::daemon::invocation::dispatch::deps::{
    DirectoryPlane, IdentityPlane, RuntimePlane, SessionPlane,
};
use crate::daemon::invocation::dispatch::descriptor_binding::RuntimeBoundAbility;
use crate::daemon::invocation::dispatch::federation_wrappers;
use crate::daemon::invocation::dispatch::federation_wrappers::{
    ABILITY_FEDERATION_ADVERTISE_ABILITIES, ABILITY_FEDERATION_ADVERTISE_AGENT,
    ABILITY_NAMESPACE_RESOLVE,
};
use crate::daemon::invocation::dispatch::forwarded_finalization::{
    ensure_forwarded_receipt_signer_key, ForwardedFinalizationVerifier, ForwardedInvocationBinding,
};
use crate::daemon::invocation::dispatch::governance_read_route::require_selected_governance_read_route;
use crate::daemon::invocation::dispatch::invocation_wire::{
    callee_ura_from_envelope, status_from_axon_invoke_error, BoxedDownStream,
};
use crate::daemon::invocation::dispatch::transport_stream::TransportDropNotifyStream;
use crate::daemon::invocation::dispatch::unary_dispatcher::UnaryDispatcher;
use crate::daemon::invocation::routing::route_resolver::{
    CanonicalRouteDispatch, CanonicalRouteSelection, SelectedInvokeRoute,
};
use crate::daemon::invocation::streams::stream_dispatcher::StreamDispatcher;
use axon_sdk::invocation::{AbilityContext, AbilityFrame, AxonError, BidiInputFrame, CallMode};

use crate::daemon::invocation::bidi::state::pending_dispatch::{
    DispatchResult, DispatchStreamEvent, PendingDispatchMap, PendingStreamDispatchMap,
};
use crate::daemon::invocation::bidi::state::presence::{
    DispatchFrame, DispatchSender, OfflineReason, PresenceRegistration, PresenceRegistry,
    SessionContract, SessionTrustContext, DISPATCH_CHANNEL_CAPACITY,
};
use crate::daemon::invocation::bidi::state::session_failure::SessionFailure;
use crate::daemon::trust::anchor::RealmTrustAnchor;

type BoxedUpStream<T> = Pin<Box<dyn Stream<Item = Result<T, Status>> + Send + 'static>>;

/// Named runtime-admin abilities the `InvokeBidi` dispatcher routes by
/// exact name (as opposed to the generic `is_bidi_wire_ability` remote
/// bridge fall-through). This is the single source of truth consumed by
/// `daemon::ability::conformance::RuntimeAdminConformance`; the `match`
/// arms in `dispatch` reference the same constants, so a baseline row can
/// never claim an `AxonRuntimeAdmin` ability the dispatcher does not
/// actually install (SPEC §7.1 notes 6/7, §7.3 item 7, §9.1 item 13).
pub(crate) const RUNTIME_ADMIN_BIDI_ROUTES: &[DaemonBidiRoute] = DAEMON_INVOCATION_BIDI_ROUTES;
pub(crate) const SESSION_RUNTIME_FRAME_CONTENT_TYPE: &str =
    "application/vnd.axon.session-frame+protobuf";
pub(crate) const SESSION_RUNTIME_TRANSPORT_ERROR_CONTENT_TYPE: &str =
    "application/vnd.axon.session-transport-error+text";

/// `InvokeBidi` routing surface. Cheap per-call construction: every
/// plane, the gate, and the composed unary dispatcher are `Arc`-shaped.
#[derive(Clone)]
pub(crate) struct BidiDispatcher {
    admission: AdmissionFacade,
    directory: DirectoryPlane,
    sessions: SessionPlane,
    identity: IdentityPlane,
    runtime: RuntimePlane,
    gate: TargetGate,
    unary: UnaryDispatcher,
}

pub(crate) struct BidiDispatcherDeps {
    pub(crate) admission: AdmissionFacade,
    pub(crate) directory: DirectoryPlane,
    pub(crate) sessions: SessionPlane,
    pub(crate) identity: IdentityPlane,
    pub(crate) runtime: RuntimePlane,
    pub(crate) gate: TargetGate,
    pub(crate) unary: UnaryDispatcher,
}

/// Product provider installed behind every exact daemon bidi registration.
///
/// It routes the typed exact inventory to cohesive product providers;
/// transport adapters can exchange frames with them only through the
/// LocalRuntime bidi handle.
#[derive(Clone)]
pub(crate) struct DaemonBidiRouteProvider {
    session_open: SessionOpenProvider,
}

/// Product owner for the Hub's long-lived Device presence carrier.
///
/// This provider owns only session policy and its direct collaborators. It is
/// transport-agnostic: its sole I/O surface is `AbilityContext`.
#[derive(Clone)]
struct SessionOpenProvider {
    presence: Arc<PresenceRegistry>,
    pending: Option<Arc<PendingDispatchMap>>,
    pending_stream: Option<Arc<PendingStreamDispatchMap>>,
    policy: SessionOpenPolicy,
    session_requests: BidiDispatcher,
}

/// Reload-aware runtime admission for admitting one Device presence into a Hub.
///
/// The narrow snapshot source keeps transport admission machinery out of the
/// provider while preserving trust-anchor reload visibility for every open.
#[derive(Clone)]
struct SessionOpenPolicy {
    session_realm: Option<String>,
    trust_anchor_snapshot: Arc<dyn Fn() -> Arc<RealmTrustAnchor> + Send + Sync + 'static>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum SessionControlRequestKind {
    AdvertiseAgent,
    AdvertiseAbilities,
    NamespaceResolve,
    ResolveKey,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum SessionControlScheduling {
    InlineDrain,
    SpawnTask,
}

#[derive(Debug, Clone)]
struct SessionControlRequest {
    kind: SessionControlRequestKind,
    caller_device_ura: String,
    args: Vec<u8>,
    metadata: HashMap<String, String>,
}

#[derive(Debug, Clone)]
struct SessionControlLifecycle {
    state: SessionControlLifecycleState,
}

#[derive(Debug, Clone)]
enum SessionControlLifecycleState {
    Validated(SessionControlRequest),
    Scheduled {
        request: SessionControlRequest,
        scheduling: SessionControlScheduling,
    },
    Replied,
}

impl DaemonBidiRouteProvider {
    pub(crate) async fn invoke(
        &self,
        route: DaemonBidiRoute,
        context: Arc<AbilityContext>,
    ) -> Result<Vec<u8>, AxonError> {
        match route {
            DaemonBidiRoute::SessionOpen => self.session_open.invoke(context).await,
        }
    }
}

impl SessionOpenProvider {
    fn new(dispatcher: &BidiDispatcher) -> Self {
        Self {
            presence: Arc::clone(&dispatcher.directory.presence),
            pending: dispatcher.sessions.pending.clone(),
            pending_stream: dispatcher.sessions.pending_stream.clone(),
            policy: SessionOpenPolicy::new(
                dispatcher.admission.clone(),
                dispatcher.identity.session_realm.clone(),
            ),
            session_requests: dispatcher.clone(),
        }
    }
}

impl SessionOpenPolicy {
    fn new(admission: AdmissionFacade, session_realm: Option<String>) -> Self {
        Self {
            session_realm,
            trust_anchor_snapshot: Arc::new(move || admission.trust_anchor_snapshot()),
        }
    }

    fn validate_caller(&self, caller_ura: &str) -> Result<(), AxonError> {
        let trust_anchor = (self.trust_anchor_snapshot)();
        validate_session_realm(
            caller_ura,
            self.session_realm.as_deref(),
            trust_anchor.as_ref(),
        )
        .map_err(runtime_status_to_axon_error)
    }
}

impl SessionControlRequestKind {
    fn from_public_ability(ability: &str) -> Option<Self> {
        match ability {
            ABILITY_FEDERATION_ADVERTISE_AGENT => Some(Self::AdvertiseAgent),
            ABILITY_FEDERATION_ADVERTISE_ABILITIES => Some(Self::AdvertiseAbilities),
            ABILITY_NAMESPACE_RESOLVE => Some(Self::NamespaceResolve),
            federation_wrappers::ABILITY_FEDERATION_RESOLVE_KEY => Some(Self::ResolveKey),
            _ => None,
        }
    }

    fn public_ability(self) -> &'static str {
        match self {
            Self::AdvertiseAgent => ABILITY_FEDERATION_ADVERTISE_AGENT,
            Self::AdvertiseAbilities => ABILITY_FEDERATION_ADVERTISE_ABILITIES,
            Self::NamespaceResolve => ABILITY_NAMESPACE_RESOLVE,
            Self::ResolveKey => federation_wrappers::ABILITY_FEDERATION_RESOLVE_KEY,
        }
    }

    fn scheduling(self) -> SessionControlScheduling {
        match self {
            Self::NamespaceResolve | Self::ResolveKey => SessionControlScheduling::InlineDrain,
            Self::AdvertiseAgent | Self::AdvertiseAbilities => SessionControlScheduling::SpawnTask,
        }
    }
}

fn session_control_kind_for_hub(
    session_realm: Option<&str>,
    ability_ura: &str,
) -> Result<SessionControlRequestKind, String> {
    let realm = session_realm
        .map(str::trim)
        .filter(|realm| !realm.is_empty())
        .ok_or_else(|| {
            "session_request requires canonical hub session realm context before validating \
             request ability_ura"
                .to_string()
        })?;
    let hub_ura = crate::core::ura::hub_ura(realm);
    let ability = crate::core::ura::public_ability_name_from_ability_ura(&hub_ura, ability_ura)
        .ok_or_else(|| {
            format!(
                "session_request: ability_ura `{ability_ura}` does not belong to hub `{hub_ura}`"
            )
        })?;
    SessionControlRequestKind::from_public_ability(&ability).ok_or_else(|| {
        format!(
            "session_request: ability `{ability}` is not a session-control request; \
             product invocations must use canonical ReverseDispatchCall"
        )
    })
}

impl SessionControlRequest {
    fn from_validated_parts(
        kind: SessionControlRequestKind,
        caller_device_ura: &str,
        args: &[u8],
        args_content_envelope: &SessionContentEnvelope,
        metadata: HashMap<String, String>,
    ) -> Result<Self, SessionRequestError> {
        Self::validate_content(kind, args_content_envelope)?;
        let caller_device_ura = caller_device_ura.trim();
        if caller_device_ura.is_empty() {
            return Err(SessionRequestError::PermissionDenied {
                reason: format!(
                    "{}: admitted session caller is required",
                    kind.public_ability()
                ),
            });
        }
        crate::core::ura::parse_ura(caller_device_ura).map_err(|error| {
            SessionRequestError::PermissionDenied {
                reason: format!(
                    "{}: caller_device_ura is not a valid URA: {error}",
                    kind.public_ability()
                ),
            }
        })?;
        Ok(Self {
            kind,
            caller_device_ura: caller_device_ura.to_string(),
            args: args.to_vec(),
            metadata,
        })
    }

    fn validate_content(
        kind: SessionControlRequestKind,
        args_content_envelope: &SessionContentEnvelope,
    ) -> Result<(), SessionRequestError> {
        let ability = kind.public_ability();
        if args_content_envelope.is_encrypted() {
            Err(SessionRequestError::PermissionDenied {
                reason: format!(
                    "session.open: Request ability `{ability}` received encrypted args \
                     but no hub-side request decryptor is wired"
                ),
            })
        } else if !args_content_envelope.content_type.is_empty()
            && args_content_envelope.content_type != "application/json"
        {
            Err(SessionRequestError::PermissionDenied {
                reason: format!(
                    "session.open: Request ability `{ability}` received unsupported \
                     args content_type {:?}",
                    args_content_envelope.content_type
                ),
            })
        } else if !args_content_envelope.encoding.is_empty()
            && args_content_envelope.encoding != "identity"
        {
            Err(SessionRequestError::PermissionDenied {
                reason: format!(
                    "session.open: Request ability `{ability}` received unsupported \
                     args encoding {:?}",
                    args_content_envelope.encoding
                ),
            })
        } else {
            Ok(())
        }
    }
}

impl SessionControlLifecycle {
    fn validated(request: SessionControlRequest) -> Self {
        Self {
            state: SessionControlLifecycleState::Validated(request),
        }
    }

    fn schedule(self) -> Self {
        match self.state {
            SessionControlLifecycleState::Validated(request) => Self {
                state: SessionControlLifecycleState::Scheduled {
                    scheduling: request.kind.scheduling(),
                    request,
                },
            },
            other => Self { state: other },
        }
    }

    fn scheduling(&self) -> Option<SessionControlScheduling> {
        match &self.state {
            SessionControlLifecycleState::Scheduled { scheduling, .. } => Some(*scheduling),
            _ => None,
        }
    }

    async fn dispatch(self, dispatcher: &BidiDispatcher) -> RequestOutcome {
        let SessionControlLifecycleState::Scheduled { request, .. } = self.state else {
            return RequestOutcome::Err {
                error: SessionRequestError::PermissionDenied {
                    reason: "session control request was not scheduled".to_string(),
                },
            };
        };
        let outcome = dispatcher.dispatch_session_control_request(&request).await;
        let _replied = SessionControlLifecycleState::Replied;
        outcome
    }
}

impl BidiDispatcher {
    pub(crate) fn new(deps: BidiDispatcherDeps) -> Self {
        let BidiDispatcherDeps {
            admission,
            directory,
            sessions,
            identity,
            runtime,
            gate,
            unary,
        } = deps;

        Self {
            admission,
            directory,
            sessions,
            identity,
            runtime,
            gate,
            unary,
        }
    }

    pub(crate) fn daemon_route_provider(&self) -> DaemonBidiRouteProvider {
        DaemonBidiRouteProvider {
            session_open: SessionOpenProvider::new(self),
        }
    }

    /// Exact bidi transport entry. Product lifecycle is selected by the
    /// LocalRuntime registration, never by this transport dispatcher.
    pub(crate) async fn dispatch_daemon_route_runtime(
        &self,
        route: DaemonBidiRoute,
        envelope_open: &EnvelopeOpen,
        up: Streaming<InvokeBidiUp>,
    ) -> Result<Response<BoxedDownStream<InvokeBidiDown>>, Status> {
        let runtime = self
            .runtime
            .require_local_runtime(format!("{} exact bidi route", route.name()))?;
        crate::daemon::invocation::dispatch::daemon_route_runtime::DaemonRouteRuntimeAdapter::new(
            runtime,
            self.runtime.cancellations.clone(),
            self.admission.clone(),
            self.runtime.runtime_admission()?,
        )
        .open_bidi(route, envelope_open, up)
        .await
    }

    /// Generic frame-0 routing. Exact routes are rejected here because their
    /// sole ingress is the descriptor-bound daemon runtime adapter.
    pub(crate) async fn dispatch(
        &self,
        ability_name: &str,
        envelope_open: &EnvelopeOpen,
        up: Streaming<InvokeBidiUp>,
    ) -> Result<Response<BoxedDownStream<InvokeBidiDown>>, Status> {
        if let Some(route) = DaemonBidiRoute::from_function(ability_name) {
            return Err(Status::failed_precondition(format!(
                "exact bidi route `{}` must enter through DaemonRouteRuntimeAdapter",
                route.name()
            )));
        }
        let selection = match self.resolve_bidi_route(envelope_open).await {
            Ok(selection) => selection,
            Err(status) => return Err(status),
        };
        let call_mode = selection.call_mode();
        let selected_route = match selection.into_dispatch() {
            CanonicalRouteDispatch::Local(route) => route,
            CanonicalRouteDispatch::Peer(route) | CanonicalRouteDispatch::UpstreamHub(route) => {
                return Err(Status::unimplemented(format!(
                    "InvokeBidi selected canonical peer route to hub `{}` for `{}`, but \
                     the generic cross-realm bidi carrier is unsupported; Device mode does not \
                     own a peer dialer",
                    route.hub_ura, route.query_name,
                )));
            }
            CanonicalRouteDispatch::HubSession(route) => {
                return self
                    .dispatch_hub_session_bidi(&route, envelope_open, Box::pin(up))
                    .await;
            }
        };
        let execution_host_is_self = self
            .gate
            .matches_self_target_ura(&selected_route.execution_host_ura)
            .await;
        if selected_route
            .dispatch_target(execution_host_is_self)
            .is_local_runtime()
        {
            let wire_kind = self
                .runtime
                .ability_wire
                .bidi_wire_kind_for(&selected_route.dispatch_name)
                .ok_or_else(|| {
                    crate::op_event!(
                        component = daemon_invocation,
                        kind = invoke_bidi_unwired_ability,
                        ability = ability_name,
                        dispatch_ability = selected_route.dispatch_name.as_str(),
                        route_ura = selected_route.route_ura.as_str(),
                    );
                    Status::unimplemented(format!(
                        "InvokeBidi selected local route `{}` for ability `{ability_name}`, but \
                         dispatch ability `{}` has no daemon bidi wire adapter",
                        selected_route.route_ura, selected_route.dispatch_name,
                    ))
                })?;
            self.dispatch_local_bidi_selected_route(
                envelope_open,
                Box::pin(up),
                selected_route,
                call_mode,
                wire_kind,
            )
            .await
        } else {
            self.dispatch_remote_bidi(&selected_route, envelope_open, Box::pin(up), call_mode)
                .await
        }
    }
}

pub(crate) const REASON_BIDI_FIRST_FRAME_SEQUENCE: &str = "AXON_BIDI_FIRST_FRAME_SEQUENCE";
pub(crate) const REASON_BIDI_NON_STRICT_ORDERING: &str = "AXON_BIDI_NON_STRICT_ORDERING";
const REASON_BIDI_FRAME_SEQUENCE: &str = "AXON_BIDI_FRAME_SEQUENCE";

/// Pull the `EnvelopeOpen` payload out of frame 0 of an
/// `InvokeBidi` up stream. Returns `Status::invalid_argument` for
/// any non-EnvelopeOpen first frame, since the axon protocol
/// mandates frame 0 is the EnvelopeOpen.
pub(crate) fn extract_envelope_open(frame: &InvokeBidiUp) -> Result<&EnvelopeOpen, Status> {
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

pub(crate) fn validate_and_extract_bidi_frame0(
    frame: &InvokeBidiUp,
) -> Result<&EnvelopeOpen, Status> {
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

pub(crate) fn validate_bidi_stream_ordering(streams: &[StreamDescriptor]) -> Result<(), Status> {
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

pub(crate) fn failed_dispatch_result(
    reason: impl Into<String>,
    default_code: &str,
    retryable: bool,
) -> DispatchResult {
    let reason = reason.into();
    DispatchResult {
        payload: Vec::new(),
        result_content_type: String::new(),
        failure: Some(SessionFailure::from_reason(
            &reason,
            default_code,
            retryable,
        )),
        error: Some(reason),
        request_id: None,
        admission_receipt: None,
        terminal_receipt: None,
    }
}

fn hub_session_bidi_status(error: SessionRequestError) -> Status {
    match error {
        SessionRequestError::TargetOffline => {
            Status::unavailable("remote InvokeBidi target is offline")
        }
        SessionRequestError::PermissionDenied { reason } => Status::permission_denied(reason),
        SessionRequestError::UpstreamFailure { reason } => Status::unavailable(format!(
            "remote InvokeBidi HubSession dispatch failed: {reason}"
        )),
        SessionRequestError::UpstreamTimeout => {
            Status::deadline_exceeded("remote InvokeBidi HubSession dispatch timed out")
        }
    }
}

impl BidiDispatcher {
    async fn dispatch_hub_session_bidi(
        &self,
        selected_route: &SelectedInvokeRoute,
        envelope_open: &EnvelopeOpen,
        mut up: BoxedUpStream<InvokeBidiUp>,
    ) -> Result<Response<BoxedDownStream<InvokeBidiDown>>, Status> {
        let Some(escalation) = self.sessions.escalation.as_ref() else {
            return Err(Status::failed_precondition(
                "InvokeBidi selected HubSession route but session escalation is not configured",
            ));
        };
        let request = bidi_open_to_invoke_request(envelope_open)?;
        let forwarded_binding =
            ForwardedInvocationBinding::for_selected_route(&request, selected_route)?;
        let receipt_resolver = self.admission.receipt_key_resolver();
        ensure_forwarded_receipt_signer_key(
            receipt_resolver.as_ref(),
            self.sessions.device_trust_sync.as_ref(),
            &selected_route.execution_host_ura,
            "InvokeBidi HubSession",
        )
        .await?;
        let stdout_stream_id = local_bidi_stdout_stream_id(envelope_open);
        let mut handle = escalation
            .escalate_bidi(request)
            .await
            .map_err(hub_session_bidi_status)?;
        let input_sender = handle.input_sender();
        let call_id = handle.call_id();
        let call_id_hex = call_id_hex(&call_id);

        crate::op_event!(
            component = daemon_invocation,
            kind = canonical_invoke_bidi_hub_session_selected_route,
            call_id = call_id_hex,
            callee_ura = selected_route.callee_ura.as_str(),
            execution_host_ura = selected_route.execution_host_ura.as_str(),
            route_ura = selected_route.route_ura.as_str(),
        );

        let (down_tx, down_rx) = tokio::sync::mpsc::channel::<Result<InvokeBidiDown, Status>>(16);
        let down_tx_for_results = down_tx.clone();
        let call_id_hex_for_results = call_id_hex.clone();
        tokio::spawn(async move {
            let mut finalization =
                ForwardedFinalizationVerifier::new(forwarded_binding, receipt_resolver);
            while let Some(event) = handle.recv().await {
                match event {
                    DispatchStreamEvent::Admission(receipt) => {
                        let receipt = *receipt;
                        crate::op_event!(
                            component = daemon_invocation,
                            kind = hub_session_bidi_admission_received,
                            call_id = call_id_hex_for_results,
                            receipt_state = receipt.state,
                        );
                        if let Err(status) = finalization.admit(receipt.clone()) {
                            crate::op_event!(
                                component = daemon_invocation,
                                kind = hub_session_bidi_admission_rejected,
                                call_id = call_id_hex_for_results,
                                error = status.message(),
                            );
                            let _ = down_tx_for_results.send(Err(status)).await;
                            break;
                        }
                        let frame = InvokeBidiDown {
                            payload: Some(DownPayload::Receipt(receipt)),
                            ..InvokeBidiDown::default()
                        };
                        if down_tx_for_results.send(Ok(frame)).await.is_err() {
                            break;
                        }
                    }
                    DispatchStreamEvent::Chunk(bytes) => {
                        crate::op_event!(
                            component = daemon_invocation,
                            kind = hub_session_bidi_chunk_received,
                            call_id = call_id_hex_for_results,
                            payload_bytes = bytes.len(),
                        );
                        if let Err(status) = finalization.observe_data() {
                            crate::op_event!(
                                component = daemon_invocation,
                                kind = hub_session_bidi_chunk_rejected,
                                call_id = call_id_hex_for_results,
                                error = status.message(),
                            );
                            let _ = down_tx_for_results.send(Err(status)).await;
                            break;
                        }
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
                    DispatchStreamEvent::Terminal(result) => {
                        crate::op_event!(
                            component = daemon_invocation,
                            kind = hub_session_bidi_terminal_received,
                            call_id = call_id_hex_for_results,
                            has_terminal_receipt = result.terminal_receipt.is_some(),
                            has_failure = result.failure.is_some(),
                        );
                        let DispatchResult {
                            payload,
                            result_content_type,
                            error,
                            failure,
                            admission_receipt,
                            terminal_receipt,
                            ..
                        } = *result;
                        let frame = match terminal_receipt {
                            Some(terminal_receipt) => finalization
                                .finalize_with_carrier_result(
                                    admission_receipt,
                                    terminal_receipt,
                                    payload,
                                    result_content_type,
                                )
                                .and_then(|finalized| {
                                    let carrier_terminal = InvokeBidiDown {
                                        payload: Some(DownPayload::ReverseDispatchResult(
                                            axon_sdk::pb::axon::v1::ReverseDispatchResult {
                                                payload: finalized.output.clone(),
                                                result_content_type: finalized
                                                    .output_content_type
                                                    .clone(),
                                                terminal: true,
                                                failure: finalized.failure.clone(),
                                                admission_receipt: Some(
                                                    finalized.admission_receipt.clone(),
                                                ),
                                                terminal_receipt: Some(
                                                    finalized.terminal_receipt.clone(),
                                                ),
                                                ..Default::default()
                                            },
                                        )),
                                        ..InvokeBidiDown::default()
                                    };
                                    down_tx_for_results.try_send(Ok(carrier_terminal)).map_err(
                                        |_| {
                                            Status::unavailable(
                                                "remote bidi terminal carrier result dropped",
                                            )
                                        },
                                    )?;
                                    Ok(InvokeBidiDown {
                                        payload: Some(DownPayload::Receipt(
                                            finalized.terminal_receipt,
                                        )),
                                        ..InvokeBidiDown::default()
                                    })
                                }),
                            None => {
                                let detail = failure
                                    .as_ref()
                                    .map(|failure| failure.message.as_str())
                                    .or(error.as_deref())
                                    .unwrap_or(
                                        "remote HubSession bidi omitted its canonical terminal receipt",
                                    );
                                Err(Status::failed_precondition(format!(
                                    "remote HubSession bidi failed before canonical terminal: {detail}"
                                )))
                            }
                        };
                        if let Err(status) = frame.as_ref() {
                            crate::op_event!(
                                component = daemon_invocation,
                                kind = hub_session_bidi_terminal_rejected,
                                call_id = call_id_hex_for_results,
                                error = status.message(),
                            );
                        }
                        let _ = down_tx_for_results.send(frame).await;
                        break;
                    }
                }
            }
        });

        tokio::spawn(async move {
            let mut expected_up_sequence = 1_u64;
            let mut eof_sent = false;
            while let Some(maybe_frame) = up.next().await {
                let frame = match maybe_frame {
                    Ok(frame) => frame,
                    Err(_) => break,
                };
                if frame.sequence != expected_up_sequence {
                    break;
                }
                expected_up_sequence = expected_up_sequence.saturating_add(1);
                let Some(payload) = frame.payload else {
                    continue;
                };
                match payload {
                    UpPayload::BinaryChunk(chunk) => {
                        if input_sender.send_binary(chunk).await.is_err() {
                            break;
                        }
                    }
                    UpPayload::Control(control) => {
                        if matches!(
                            control.control,
                            Some(axon_sdk::pb::axon::v1::bidi_control::Control::Eof(true))
                        ) {
                            eof_sent = true;
                        }
                        if input_sender.send_control(control).await.is_err() {
                            break;
                        }
                        if eof_sent {
                            break;
                        }
                    }
                    UpPayload::EnvelopeOpen(_)
                    | UpPayload::DispatchResult(_)
                    | UpPayload::ReverseDispatchCall(_)
                    | UpPayload::ReverseBidiInput(_) => {}
                }
            }
            if !eof_sent {
                let _ = input_sender
                    .send_control(BidiControl {
                        control: Some(axon_sdk::pb::axon::v1::bidi_control::Control::Eof(true)),
                    })
                    .await;
            }
        });

        let stream = LocalBidiDownStream::new(down_rx);
        Ok(Response::new(
            Box::pin(stream) as BoxedDownStream<InvokeBidiDown>
        ))
    }

    pub(crate) async fn dispatch_remote_bidi(
        &self,
        selected_route: &SelectedInvokeRoute,
        envelope_open: &EnvelopeOpen,
        mut up: BoxedUpStream<InvokeBidiUp>,
        call_mode: CallMode,
    ) -> Result<Response<BoxedDownStream<InvokeBidiDown>>, Status> {
        let pending = self.sessions.pending_stream.as_ref().ok_or_else(|| {
            Status::failed_precondition(format!(
                "InvokeBidi {}: daemon was constructed without a \
                 PendingStreamDispatchMap; boot must call with_pending_stream(...) \
                 to enable remote bidi bridging",
                selected_route.dispatch_name
            ))
        })?;
        let session = require_canonical_dispatch_session(
            &self.directory.presence,
            &selected_route.execution_host_ura,
            &selected_route.route_ura,
            "InvokeBidi",
        )?;
        let session_id = session.session_id;
        let sender = session.sender;
        let carrier_version = session.contract_version;

        let mut handle = pending.register_lossless_pending_for(&selected_route.execution_host_ura);
        let call_id = handle.call_id();
        let stdout_stream_id = local_bidi_stdout_stream_id(envelope_open);

        let forwarded_request =
            remote_bidi_forwarded_request(selected_route, envelope_open, call_mode)?;
        let forwarded_binding =
            ForwardedInvocationBinding::for_selected_route(&forwarded_request, selected_route)?;
        let receipt_resolver = self.admission.receipt_key_resolver();
        ensure_forwarded_receipt_signer_key(
            receipt_resolver.as_ref(),
            self.sessions.device_trust_sync.as_ref(),
            &selected_route.execution_host_ura,
            "InvokeBidi",
        )
        .await?;
        let open_frame = build_canonical_dispatch_frame(call_id, forwarded_request, CallMode::Bidi);
        match sender.try_send(Ok(open_frame)) {
            Ok(()) => {}
            Err(tokio::sync::mpsc::error::TrySendError::Full(_)) => {
                // Full = device is slow, not dead: keep its session,
                // fail only this call as retryable backpressure.
                return Err(Status::resource_exhausted(
                    crate::daemon::invocation::bidi::state::presence::DISPATCH_TARGET_BUSY_REASON,
                ));
            }
            Err(tokio::sync::mpsc::error::TrySendError::Closed(_)) => {
                self.directory.presence.remove_if_session(
                    &selected_route.execution_host_ura,
                    session_id,
                    crate::daemon::invocation::bidi::state::presence::OfflineReason::StreamClosed,
                );
                return Err(Status::failed_precondition(
                    crate::daemon::invocation::bidi::state::presence::DISPATCH_TARGET_OFFLINE_REASON,
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
            carrier_version = carrier_version,
            call_id = call_id,
        );

        let (down_tx, down_rx) = tokio::sync::mpsc::channel::<Result<InvokeBidiDown, Status>>(16);

        let down_tx_for_results = down_tx.clone();
        tokio::spawn(async move {
            let mut finalization =
                ForwardedFinalizationVerifier::new(forwarded_binding, receipt_resolver);
            while let Some(event) = handle.recv().await {
                match event {
                    DispatchStreamEvent::Admission(receipt) => {
                        let receipt = *receipt;
                        crate::op_event!(
                            component = daemon_invocation,
                            kind = remote_bidi_bridge_admission_received,
                            call_id = call_id,
                            receipt_state = receipt.state,
                        );
                        if let Err(status) = finalization.admit(receipt.clone()) {
                            crate::op_event!(
                                component = daemon_invocation,
                                kind = remote_bidi_bridge_admission_rejected,
                                call_id = call_id,
                                error = status.message(),
                            );
                            let _ = down_tx_for_results.send(Err(status)).await;
                            break;
                        }
                        let frame = InvokeBidiDown {
                            payload: Some(DownPayload::Receipt(receipt)),
                            ..InvokeBidiDown::default()
                        };
                        if down_tx_for_results.send(Ok(frame)).await.is_err() {
                            break;
                        }
                    }
                    DispatchStreamEvent::Chunk(bytes) => {
                        crate::op_event!(
                            component = daemon_invocation,
                            kind = remote_bidi_bridge_chunk_received,
                            call_id = call_id,
                            payload_bytes = bytes.len(),
                        );
                        if let Err(status) = finalization.observe_data() {
                            crate::op_event!(
                                component = daemon_invocation,
                                kind = remote_bidi_bridge_chunk_rejected,
                                call_id = call_id,
                                error = status.message(),
                            );
                            let _ = down_tx_for_results.send(Err(status)).await;
                            break;
                        }
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
                    DispatchStreamEvent::Terminal(result) => {
                        crate::op_event!(
                            component = daemon_invocation,
                            kind = remote_bidi_bridge_terminal_received,
                            call_id = call_id,
                            has_terminal_receipt = result.terminal_receipt.is_some(),
                            has_failure = result.failure.is_some(),
                        );
                        let DispatchResult {
                            payload,
                            result_content_type,
                            error,
                            failure,
                            request_id: _,
                            admission_receipt,
                            terminal_receipt,
                            ..
                        } = *result;
                        let frame = match terminal_receipt {
                            Some(terminal_receipt) => finalization
                                .finalize_with_carrier_result(
                                    admission_receipt,
                                    terminal_receipt,
                                    payload,
                                    result_content_type,
                                )
                                .map(|finalized| InvokeBidiDown {
                                    payload: Some(DownPayload::Receipt(finalized.terminal_receipt)),
                                    ..InvokeBidiDown::default()
                                }),
                            None => {
                                let detail = failure
                                    .as_ref()
                                    .map(|failure| failure.message.as_str())
                                    .or(error.as_deref())
                                    .unwrap_or(
                                        "remote terminal result omitted its canonical receipt",
                                    );
                                Err(Status::failed_precondition(format!(
                                    "remote bidi transport failed before canonical terminal: {detail}"
                                )))
                            }
                        };
                        if let Err(status) = frame.as_ref() {
                            crate::op_event!(
                                component = daemon_invocation,
                                kind = remote_bidi_bridge_terminal_rejected,
                                call_id = call_id,
                                error = status.message(),
                            );
                        }
                        let _ = down_tx_for_results.send(frame).await;
                        break;
                    }
                }
            }
        });

        let execution_host_ura_owned = selected_route.execution_host_ura.clone();
        let core_wire_kind =
            crate::daemon::ability::wire::core_bidi_wire_kind_for(&selected_route.dispatch_name);
        let dispatch_name_for_up = selected_route.dispatch_name.clone();
        let presence_for_up = Arc::clone(&self.directory.presence);
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
                let Some(bridge_frame_result) =
                    build_remote_bidi_input_frame_from_canonical_payload(
                        call_id,
                        &dispatch_name_for_up,
                        core_wire_kind,
                        payload,
                    )
                else {
                    continue;
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
                if remote_bidi_input_dispatch_frame_is_eof(&bridge_frame) {
                    eof_sent = true;
                }
                match sender.try_send(Ok(bridge_frame)) {
                    Ok(()) => {}
                    Err(tokio::sync::mpsc::error::TrySendError::Full(_)) => {
                        // Full = device is slow, not dead: keep its
                        // session, fail only this call as retryable.
                        let reason =
                            crate::daemon::invocation::bidi::state::presence::DISPATCH_TARGET_BUSY_REASON.to_string();
                        let _ = pending_for_up
                            .finish(
                                call_id,
                                failed_dispatch_result(&reason, "TARGET_BUSY", true),
                            )
                            .await;
                        return;
                    }
                    Err(tokio::sync::mpsc::error::TrySendError::Closed(_)) => {
                        presence_for_up.remove_if_session(
                            &execution_host_ura_owned,
                            session_id,
                            crate::daemon::invocation::bidi::state::presence::OfflineReason::StreamClosed,
                        );
                        let reason =
                            crate::daemon::invocation::bidi::state::presence::DISPATCH_TARGET_OFFLINE_REASON.to_string();
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
    /// device-side terminal attach handler expects JSON
    /// `{"type":"stdin","data":"<base64>"}` — its on-the-wire
    /// shape lives with the terminal system ability. We
    /// translate at this seam: BinaryChunk → JSON stdin frame on
    /// the up direction, JSON stdout frame → BinaryChunk on the
    /// down direction. PtyResize control frames map to a JSON
    /// `{"type":"resize","cols":N,"rows":N}` shape the handler
    /// already consumes.
    async fn resolve_bidi_route(
        &self,
        envelope_open: &EnvelopeOpen,
    ) -> Result<CanonicalRouteSelection, Status> {
        let target_ura = callee_ura_from_envelope(envelope_open.envelope.as_ref(), "InvokeBidi")?;
        let ability =
            crate::daemon::invocation::dispatch::invocation_wire::function_name_from_invocation_target(
                "InvokeBidi frame 0",
                envelope_open.target.as_ref(),
            )?;

        let selection = self
            .gate
            .resolve_canonical_route(&target_ura, ability, CallMode::Bidi)
            .await
            .map_err(route_negative_status)?;
        if let CanonicalRouteDispatch::Local(selected_route)
        | CanonicalRouteDispatch::HubSession(selected_route) = selection.dispatch()
        {
            if !selected_route.is_authoritative_local_or_better() {
                return Err(route_profile_blocked_status(selected_route));
            }
        }
        Ok(selection)
    }

    pub(crate) async fn dispatch_local_bidi_selected_route(
        &self,
        envelope_open: &EnvelopeOpen,
        mut up: BoxedUpStream<InvokeBidiUp>,
        selected_route: SelectedInvokeRoute,
        call_mode: CallMode,
        wire_kind: LocalBidiWireKind,
    ) -> Result<Response<BoxedDownStream<InvokeBidiDown>>, Status> {
        let dispatch_ability = selected_route.ability_ura.clone();
        let wire_envelope = envelope_open
            .envelope
            .clone()
            .ok_or_else(|| Status::invalid_argument("InvokeBidi request missing envelope"))?;
        require_selected_governance_read_route("InvokeBidi", &selected_route, &wire_envelope)?;
        crate::op_event!(
            component = daemon_invocation,
            kind = invoke_bidi_local_runtime_dispatch,
            ability = selected_route.dispatch_name.as_str(),
            dispatch_ability = dispatch_ability.as_str(),
            callee_ura = selected_route.callee_ura.as_str(),
            execution_host_ura = selected_route.execution_host_ura.as_str(),
            route_ura = selected_route.route_ura.as_str(),
        );

        let runtime = self.runtime.require_local_runtime(format!(
            "InvokeBidi ability `{}`",
            selected_route.dispatch_name
        ))?;
        let bound_ability = RuntimeBoundAbility::from_selected_route(
            "InvokeBidi",
            &runtime,
            self.directory.local_ability_catalog.as_deref(),
            &selected_route,
            call_mode,
        )
        .await?;
        let dispatch_descriptor_ref = bound_ability
            .descriptor_ref_for_mode(
                "InvokeBidi",
                &selected_route.callee_ura,
                call_mode,
                Some(&selected_route.route_ura),
            )?
            .into_descriptor_ref();
        let target_ability =
            crate::daemon::invocation::dispatch::invocation_wire::ability_binding_from_invocation_target(
                "InvokeBidi",
                envelope_open.target.as_ref(),
            )?;
        bound_ability.require_wire_target_matches(
            "InvokeBidi",
            &selected_route.callee_ura,
            target_ability,
            &selected_route.route_ura,
        )?;
        let local_system_ingress = self
            .admission
            .accepts_local_system_envelope(envelope_open.envelope.as_ref());
        let wire = if local_system_ingress {
            let metadata = HostedAgentDelegationIssuer::materialize_request_metadata(
                &envelope_open.metadata,
                &wire_envelope,
                HostedAgentDelegationIngress::TrustedLocalSystem,
                &selected_route.execution_host_ura,
                &dispatch_ability,
            )?;
            crate::daemon::axon_bridge::descriptor_bound_dispatch::local_system_from_wire_parts(
                wire_envelope,
                dispatch_descriptor_ref,
                envelope_open.initial_args.clone(),
                metadata,
            )
        } else {
            let metadata = HostedAgentDelegationIssuer::materialize_request_metadata(
                &envelope_open.metadata,
                &wire_envelope,
                HostedAgentDelegationIngress::ExternalSigned,
                &selected_route.execution_host_ura,
                &dispatch_ability,
            )?;
            let signed_descriptor_ref = bound_ability
                .signed_descriptor_ref_from_target(
                    "InvokeBidi",
                    &selected_route.callee_ura,
                    call_mode,
                    envelope_open.target.as_ref(),
                )?
                .into_descriptor_ref();
            crate::daemon::axon_bridge::descriptor_bound_dispatch::external_signed_from_wire_parts(
                wire_envelope,
                signed_descriptor_ref,
                envelope_open.initial_args.clone(),
                metadata,
            )
        }
        .map_err(|err| status_from_axon_invoke_error("InvokeBidi", &dispatch_ability, *err))?;
        let lifecycle_envelope = wire.envelope.clone();
        let runtime_admission = self.runtime.stage_runtime_admission(
            &self.admission,
            &wire,
            &dispatch_ability,
            CallMode::Bidi,
        )?;
        let handle =
            crate::daemon::axon_bridge::descriptor_bound_dispatch::open_bidi_external_signed(
                &runtime, wire,
            )
            .await
            .map_err(|err| status_from_axon_invoke_error("InvokeBidi", &dispatch_ability, err))?;
        let lifecycle = match RegisteredInvocationLifecycle::register(
            self.runtime.cancellations.clone(),
            &lifecycle_envelope,
            handle.handle().clone(),
        ) {
            Ok(lifecycle) => lifecycle,
            Err(error) => {
                let _ = handle
                    .handle()
                    .cancel("bidi lifecycle registration failed")
                    .await;
                let _ = handle.handle().finalized().await;
                return Err(Status::failed_precondition(format!(
                    "InvokeBidi `{dispatch_ability}` lifecycle registration failed: {error}"
                )));
            }
        };
        if let Err(error) = runtime_admission.commit() {
            let _ = lifecycle
                .cancel_and_finalize("bidi runtime admission commit failed")
                .await;
            return Err(error);
        }
        let admission_receipt = match handle.admission_receipt().await {
            Ok(receipt) => receipt,
            Err(err) => {
                let _ = lifecycle.finalized().await;
                return Err(Status::failed_precondition(format!(
                    "CANONICAL_ADMISSION_REQUIRED: InvokeBidi `{dispatch_ability}`: {err}"
                )));
            }
        };
        let admission_wire = match axon_sdk::invocation::wire::receipt_to_wire(&admission_receipt) {
            Ok(receipt) => receipt,
            Err(error) => {
                let _ = lifecycle
                    .cancel_and_finalize("canonical admission projection failed")
                    .await;
                return Err(Status::failed_precondition(format!(
                    "CANONICAL_ADMISSION_PROJECTION_FAILED: {error}"
                )));
            }
        };
        let admission_frame = InvokeBidiDown {
            payload: Some(DownPayload::Receipt(admission_wire)),
            ..InvokeBidiDown::default()
        };
        let (handler_in_tx, mut handler_out_rx) = handle.split();
        let stdout_stream_id = local_bidi_stdout_stream_id(envelope_open);

        // Down-stream: handler-emitted JSON → InvokeBidiDown frames.
        // Capacity 16 bounds per-session dispatch backpressure.
        let (down_tx, down_rx) = tokio::sync::mpsc::channel::<Result<InvokeBidiDown, Status>>(16);
        let (transport_closed_tx, mut transport_closed_rx) =
            tokio::sync::mpsc::channel::<String>(2);

        let down_tx_for_handler = down_tx.clone();
        tokio::spawn(async move {
            let mut terminal_authority_observed = false;
            loop {
                let frame_result = tokio::select! {
                    biased;
                    close_reason = transport_closed_rx.recv() => {
                        let reason = close_reason
                            .unwrap_or_else(|| "InvokeBidi transport response dropped".to_string());
                        let projected = cancel_registered_bidi(&lifecycle, reason.clone())
                            .await
                            .map_err(|status| Status::internal(format!("{reason}; {status}")));
                        terminal_authority_observed = true;
                        let _ = down_tx_for_handler.send(projected).await;
                        break;
                    }
                    frame_result = handler_out_rx.next_frame() => frame_result,
                };
                let Some(frame_result) = frame_result else {
                    break;
                };
                let frame = match frame_result {
                    Ok(frame) => frame,
                    Err(err) => {
                        let projected = project_registered_finalized_bidi_receipt(&lifecycle)
                            .await
                            .map_err(|status| {
                                Status::internal(format!(
                                    "InvokeBidi local-runtime frame failed: {err}; {status}"
                                ))
                            });
                        terminal_authority_observed = true;
                        let _ = down_tx_for_handler.send(projected).await;
                        break;
                    }
                };
                let terminal = frame.terminal;
                let mapped = map_local_bidi_ability_frame(wire_kind, frame, stdout_stream_id);
                match mapped {
                    LocalBidiHandlerFrame::Forward(frame) => {
                        if down_tx_for_handler.send(Ok(*frame)).await.is_err() {
                            let _ = cancel_registered_bidi(
                                &lifecycle,
                                "InvokeBidi transport response dropped",
                            )
                            .await;
                            terminal_authority_observed = true;
                            break;
                        }
                        if terminal {
                            break;
                        }
                    }
                    LocalBidiHandlerFrame::Terminal => {
                        let projected = project_registered_finalized_bidi_receipt(&lifecycle).await;
                        terminal_authority_observed = true;
                        let _ = down_tx_for_handler.send(projected).await;
                        break;
                    }
                    LocalBidiHandlerFrame::Ignore => {}
                    LocalBidiHandlerFrame::ProtocolFailure(reason) => {
                        let projected = cancel_registered_bidi(&lifecycle, reason.clone())
                            .await
                            .map_err(|status| Status::internal(format!("{reason}; {status}")));
                        terminal_authority_observed = true;
                        let _ = down_tx_for_handler.send(projected).await;
                        break;
                    }
                }
                if terminal {
                    break;
                }
            }
            if !terminal_authority_observed {
                let projected = project_registered_finalized_bidi_receipt(&lifecycle).await;
                let _ = down_tx_for_handler.send(projected).await;
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

        let stream = TransportDropNotifyStream::new(
            LocalBidiDownStream::with_admission(down_rx, admission_frame),
            transport_closed_tx,
            "InvokeBidi transport response dropped",
        );
        Ok(Response::new(
            Box::pin(stream) as BoxedDownStream<InvokeBidiDown>
        ))
    }
}

impl SessionOpenProvider {
    /// Authority-owned `session.open` lifecycle.
    ///
    /// The signed tuple and carrier contract are read only from the admitted
    /// runtime context. Presence ownership is held by `SessionPresenceLease`,
    /// so every provider exit path, including runtime cancellation, removes
    /// only the session generation installed by this invocation.
    async fn invoke(&self, context: Arc<AbilityContext>) -> Result<Vec<u8>, AxonError> {
        let signed = context.signed_envelope().ok_or_else(|| {
            AxonError::permission_denied(
                "session.open provider requires an admitted signed envelope",
            )
        })?;
        let caller_ura = signed.envelope.caller.ura.trim().to_string();
        if caller_ura.is_empty() {
            return Err(AxonError::invalid_argument(
                "session.open provider requires a non-empty caller URA",
            ));
        }
        let trust_context =
            session_trust_context(&caller_ura, signed.signature.key_id_hint.as_str());
        let contract = session_contract_from_runtime_metadata(&context.request_metadata)?;
        self.policy.validate_caller(&caller_ura)?;

        let (down_tx, down_rx): (DispatchSender, _) =
            mpsc::channel::<Result<DispatchFrame, Status>>(DISPATCH_CHANNEL_CAPACITY);

        // Register before the established frame is emitted so a canonical
        // DispatchCall arriving immediately can find this provider-owned
        // session. PresenceRegistry serializes displacement and preserves
        // Offline-before-Online event ordering.
        let negotiated_version = contract.version.min(CANONICAL_SESSION_CARRIER_VERSION);
        let claimant_nonce = contract.claimant_boot_nonce.clone();
        let presence = Arc::clone(&self.presence);
        let PresenceRegistration {
            session_id,
            displaced,
            displaced_claimant_nonce,
        } = presence
            .insert_negotiated_with_trust(caller_ura.clone(), down_tx, contract, trust_context)
            .map_err(AxonError::invalid_argument)?;
        let displaced_prior = displaced.is_some();
        drop(displaced);
        crate::op_event!(
            component = daemon_invocation,
            kind = self_session_admitted,
            caller = caller_ura,
            displaced_prior = displaced_prior,
            contract_version = negotiated_version,
        );
        // T1.2: a displacement whose claimant fingerprint differs from
        // the newcomer's is two processes fighting over one URA — a
        // claimant conflict, not a same-device restart. Surfaced as a
        // first-class op_event so the ping-pong incident class
        // (2026-06-11, 5,428 reconnects) is attributable from logs.
        if let Some(prior_nonce) = displaced_claimant_nonce
            .as_ref()
            .filter(|prior| !prior.is_empty() && !claimant_nonce.is_empty())
        {
            if *prior_nonce != claimant_nonce {
                crate::op_event!(
                    component = daemon_invocation,
                    kind = claimant_conflict,
                    caller = caller_ura,
                );
            }
        }

        let mut lease =
            SessionPresenceLease::new(Arc::clone(&presence), caller_ura.clone(), session_id);
        let mut down = SessionProviderDownStream::new(
            down_rx,
            build_session_established_control(negotiated_version, session_id, displaced_prior),
        );

        // SessionEstablished remains the first product frame. Axon's
        // admission receipt is projected by the transport immediately after
        // this frame, preserving the established device wire contract while
        // keeping receipt ownership canonical.
        let established = down.next().await.ok_or_else(|| {
            AxonError::internal("session.open provider closed before SessionEstablished")
        })?;
        emit_session_provider_frame(context.as_ref(), established).await?;

        // The two carrier directions are independent pumps. Running the
        // up-stream drain as the never-completing branch future of this select
        // lets a continuously-ready input channel monopolize the task and
        // starve heartbeat acknowledgements queued on `down`. A dedicated
        // task gives the down pump an independent scheduling budget and ties
        // its lifetime back to this provider through `SessionInputPump`.
        let mut input = SessionInputPump::spawn(
            Arc::clone(&context),
            caller_ura.clone(),
            Arc::clone(&presence),
            self.pending.clone(),
            self.pending_stream.clone(),
            self.session_requests.clone(),
        );

        loop {
            tokio::select! {
                input_result = &mut input.handle => {
                    let close_reason = match input_result {
                        Ok(reason) => reason,
                        Err(error) => {
                            crate::op_event!(
                                component = session_accept,
                                kind = session_input_pump_failed,
                                caller = caller_ura,
                                error = error.to_string(),
                            );
                            OfflineReason::StreamReset
                        }
                    };
                    lease.close(close_reason);
                    while let Some(frame) = down.next().await {
                        emit_session_provider_frame(context.as_ref(), frame).await?;
                    }
                    return Ok(Vec::new());
                }
                frame = down.next() => {
                    let Some(frame) = frame else {
                        lease.close(OfflineReason::StreamClosed);
                        return Ok(Vec::new());
                    };
                    emit_session_provider_frame(context.as_ref(), frame).await?;
                }
            }
        }
    }
}

/// Independently scheduled device → Hub half of one accepted session.
///
/// The provider owns this guard, so every provider exit aborts the input pump
/// before the `AbilityContext` can outlive its canonical invocation lifecycle.
struct SessionInputPump {
    handle: tokio::task::JoinHandle<OfflineReason>,
}

impl SessionInputPump {
    fn spawn(
        context: Arc<AbilityContext>,
        caller_ura: String,
        presence: Arc<PresenceRegistry>,
        pending: Option<Arc<PendingDispatchMap>>,
        pending_stream: Option<Arc<PendingStreamDispatchMap>>,
        dispatcher: BidiDispatcher,
    ) -> Self {
        Self {
            handle: tokio::spawn(drain_session_runtime_up_stream(
                context,
                caller_ura,
                presence,
                pending,
                pending_stream,
                dispatcher,
            )),
        }
    }
}

impl Drop for SessionInputPump {
    fn drop(&mut self) {
        self.handle.abort();
    }
}

async fn emit_session_provider_frame(
    context: &AbilityContext,
    frame: Result<InvokeBidiDown, Status>,
) -> Result<(), AxonError> {
    let frame = frame.map_err(runtime_status_to_axon_error)?;
    context
        .emit_progress(frame.encode_to_vec(), SESSION_RUNTIME_FRAME_CONTENT_TYPE)
        .await
}

/// Generation-bound presence ownership for one provider invocation.
///
/// `Drop` is the cancellation path: if Axon aborts the ability while neither
/// stream branch is running, the exact generation is still removed without
/// disturbing a newer displaced replacement.
struct SessionPresenceLease {
    presence: Arc<PresenceRegistry>,
    caller_ura: String,
    session_id: crate::daemon::invocation::bidi::state::presence::PresenceSessionId,
    state: SessionPresenceLeaseState,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
enum SessionPresenceLeaseState {
    Active,
    Closed,
}

impl SessionPresenceLease {
    fn new(
        presence: Arc<PresenceRegistry>,
        caller_ura: String,
        session_id: crate::daemon::invocation::bidi::state::presence::PresenceSessionId,
    ) -> Self {
        Self {
            presence,
            caller_ura,
            session_id,
            state: SessionPresenceLeaseState::Active,
        }
    }

    fn close(&mut self, reason: OfflineReason) {
        if self.state == SessionPresenceLeaseState::Closed {
            return;
        }
        self.state = SessionPresenceLeaseState::Closed;
        let outcome = if self
            .presence
            .remove_if_session(&self.caller_ura, self.session_id, reason)
            .is_some()
        {
            "removed_from_registry"
        } else {
            "superseded_by_newer_session"
        };
        crate::op_event!(
            component = session_accept,
            kind = session_ended,
            caller = self.caller_ura,
            close_reason = reason,
            outcome = outcome,
        );
    }
}

impl Drop for SessionPresenceLease {
    fn drop(&mut self) {
        self.close(OfflineReason::StreamClosed);
    }
}

/// Build the acknowledgement for one device-originated session heartbeat.
///
/// The acknowledgement is intentionally emitted only after the Hub has
/// drained an up-stream control frame. It therefore proves end-to-end
/// progress in both directions; an independently scheduled Hub keepalive
/// would hide a half-open device-to-Hub request stream.
fn build_session_heartbeat_ack_frame() -> DispatchFrame {
    DispatchFrame::control(InvokeBidiDown {
        payload: Some(DownPayload::Control(BidiControl::default())),
        ..InvokeBidiDown::default()
    })
}

/// Admit frame-0 carrier negotiation only when the peer can preserve a
/// complete canonical Invocation and its streaming lifecycle. Contract v0-v2
/// and absent negotiation are retired; accepting them would create a live
/// session that cannot preserve the current carrier contract.
pub(crate) fn session_contract_from_ext(
    ext: Option<&axon_sdk::pb::axon::v1::SessionOpenExt>,
) -> Result<SessionContract, Status> {
    let ext = ext.ok_or_else(|| {
        Status::failed_precondition(
            "CANONICAL_CARRIER_REQUIRED: session.open requires SessionOpenExt carrier negotiation",
        )
    })?;
    if ext.contract_version < CANONICAL_SESSION_CARRIER_VERSION {
        return Err(Status::failed_precondition(format!(
            "CANONICAL_CARRIER_REQUIRED: session.open negotiated carrier v{}; v{} or newer is required",
            ext.contract_version, CANONICAL_SESSION_CARRIER_VERSION,
        )));
    }
    if ext.claimant_boot_nonce.len() != 16 {
        return Err(Status::failed_precondition(format!(
            "CANONICAL_CARRIER_REQUIRED: session.open claimant_boot_nonce must be exactly 16 bytes; got {}",
            ext.claimant_boot_nonce.len()
        )));
    }
    Ok(SessionContract {
        version: ext.contract_version,
        claimant_boot_nonce: ext.claimant_boot_nonce.clone(),
    })
}

fn session_contract_from_runtime_metadata(
    metadata: &std::collections::HashMap<String, String>,
) -> Result<SessionContract, AxonError> {
    let encoded = metadata.get(SESSION_OPEN_EXT_METADATA_KEY).ok_or_else(|| {
        runtime_status_to_axon_error(Status::failed_precondition(
            "CANONICAL_CARRIER_REQUIRED: session.open runtime context is missing carrier negotiation",
        ))
    })?;
    let bytes = hex::decode(encoded).map_err(|error| {
        AxonError::invalid_argument(format!(
            "session.open runtime carrier metadata is malformed: {error}"
        ))
    })?;
    let extension =
        axon_sdk::pb::axon::v1::SessionOpenExt::decode(bytes.as_slice()).map_err(|error| {
            AxonError::invalid_argument(format!(
                "session.open runtime carrier negotiation cannot be decoded: {error}"
            ))
        })?;
    session_contract_from_ext(Some(&extension)).map_err(runtime_status_to_axon_error)
}

/// Frame-0 down is a typed session-control acknowledgement. Session
/// negotiation is transport lifecycle, not Invocation admission, so none of
/// these facts may be encoded in an `SignedInvocationReceipt`.
fn build_session_established_control(
    negotiated_version: u32,
    hub_session_id: u64,
    displaced_prior: bool,
) -> InvokeBidiDown {
    InvokeBidiDown {
        payload: Some(DownPayload::Control(BidiControl {
            control: Some(bidi_control::Control::SessionEstablished(
                BidiSessionEstablished {
                    contract_version: negotiated_version,
                    dispatch_encoding: "proto".to_string(),
                    session_id: hub_session_id,
                    displaced_prior,
                },
            )),
        })),
        ..InvokeBidiDown::default()
    }
}

fn project_bidi_receipt(
    finalized: axon_sdk::invocation::FinalizedInvocation,
) -> Result<InvokeBidiDown, Status> {
    Ok(InvokeBidiDown {
        payload: Some(DownPayload::Receipt(
            axon_sdk::invocation::wire::receipt_to_wire(&finalized.terminal_receipt).map_err(
                |error| {
                    Status::failed_precondition(format!(
                        "CANONICAL_TERMINAL_PROJECTION_FAILED: {error}"
                    ))
                },
            )?,
        )),
        ..InvokeBidiDown::default()
    })
}

pub(crate) async fn project_registered_finalized_bidi_receipt(
    lifecycle: &RegisteredInvocationLifecycle,
) -> Result<InvokeBidiDown, Status> {
    let finalized = lifecycle.finalized().await.map_err(|err| {
        Status::failed_precondition(format!("CANONICAL_FINALIZATION_REQUIRED: {err}"))
    })?;
    project_bidi_receipt(finalized)
}

pub(crate) async fn cancel_registered_bidi(
    lifecycle: &RegisteredInvocationLifecycle,
    reason: impl Into<String>,
) -> Result<InvokeBidiDown, Status> {
    let finalized = lifecycle.cancel_and_finalize(reason).await.map_err(|err| {
        Status::failed_precondition(format!("CANONICAL_CANCELLATION_FAILED: {err}"))
    })?;
    project_bidi_receipt(finalized)
}

const LOCAL_BIDI_DEFAULT_STREAM_ID: u32 = 1;

pub(crate) type LocalBidiWireKind = crate::daemon::ability::wire::AbilityBidiWireKind;

fn local_bidi_stdout_stream_id(envelope_open: &EnvelopeOpen) -> u32 {
    envelope_open
        .streams
        .iter()
        .map(|stream| stream.stream_id)
        .find(|stream_id| *stream_id != 0)
        .unwrap_or(LOCAL_BIDI_DEFAULT_STREAM_ID)
}

#[derive(Debug)]
pub(crate) enum LocalBidiHandlerFrame {
    Forward(Box<InvokeBidiDown>),
    Terminal,
    Ignore,
    ProtocolFailure(String),
}

impl LocalBidiHandlerFrame {
    fn forward(frame: InvokeBidiDown) -> Self {
        Self::Forward(Box::new(frame))
    }
}

#[derive(Debug)]
pub(crate) enum LocalBidiUpFrame {
    Forward(serde_json::Value),
    ForwardAndClose(serde_json::Value),
    Close,
    Ignore,
}

pub(crate) fn map_local_bidi_up_payload(
    wire_kind: LocalBidiWireKind,
    payload: UpPayload,
) -> LocalBidiUpFrame {
    use axon_sdk::pb::axon::v1::bidi_control::Control as ControlVariant;
    use axon_sdk::pb::axon::v1::{BidiControl, PtyResize};
    use base64::{engine::general_purpose::STANDARD as B64, Engine as _};
    use serde_json::json;

    match (wire_kind, payload) {
        (LocalBidiWireKind::Pty, UpPayload::BinaryChunk(chunk))
            if chunk.stream_id == crate::daemon::ability::wire::PTY_CONTROL_STREAM_ID =>
        {
            match serde_json::from_slice::<serde_json::Value>(&chunk.data) {
                Ok(jsonv) => LocalBidiUpFrame::Forward(jsonv),
                Err(_) => LocalBidiUpFrame::Ignore,
            }
        }
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
        // Canonical carrier frames (DEC-F004): not local-bidi wire traffic.
        (_, UpPayload::DispatchResult(_))
        | (_, UpPayload::ReverseDispatchCall(_))
        | (_, UpPayload::ReverseBidiInput(_)) => LocalBidiUpFrame::Ignore,
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

pub(crate) fn map_local_bidi_ability_frame(
    wire_kind: LocalBidiWireKind,
    frame: AbilityFrame,
    stdout_stream_id: u32,
) -> LocalBidiHandlerFrame {
    if frame.terminal {
        return LocalBidiHandlerFrame::Terminal;
    }
    if frame.payload.is_empty() {
        return LocalBidiHandlerFrame::Ignore;
    }
    if matches!(wire_kind, LocalBidiWireKind::JsonFrames)
        && !frame.terminal
        && !frame.content_type.is_empty()
        && frame.content_type != "application/json"
    {
        return LocalBidiHandlerFrame::forward(InvokeBidiDown {
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

fn forward_json_bidi_frame(
    value: &serde_json::Value,
    stdout_stream_id: u32,
) -> LocalBidiHandlerFrame {
    match serde_json::to_vec(value) {
        Ok(payload) => LocalBidiHandlerFrame::forward(InvokeBidiDown {
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
    }
}

pub(crate) fn map_local_bidi_handler_frame(
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
                LocalBidiHandlerFrame::forward(InvokeBidiDown {
                    payload: Some(DownPayload::BinaryChunk(BinaryChunk {
                        stream_id: stdout_stream_id,
                        data: raw,
                        ..BinaryChunk::default()
                    })),
                    ..InvokeBidiDown::default()
                })
            }
            Some("exit") => forward_json_bidi_frame(value, stdout_stream_id),
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
                LocalBidiHandlerFrame::forward(InvokeBidiDown {
                    payload: Some(DownPayload::BinaryChunk(BinaryChunk {
                        stream_id: stdout_stream_id,
                        data: raw,
                        ..BinaryChunk::default()
                    })),
                    ..InvokeBidiDown::default()
                })
            }
            Some("complete" | "error") => forward_json_bidi_frame(value, stdout_stream_id),
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
        LocalBidiWireKind::JsonFrames => forward_json_bidi_frame(value, stdout_stream_id),
    }
}

/// Down-stream wrapper that emits a typed session-established control as
/// frame 0, then prioritizes control/result frames over normal payload
/// backlog.
///
/// Session heartbeat acknowledgements enter through the same
/// `DispatchSender` after the Hub has drained the corresponding up-stream
/// heartbeat. This wrapper never invents an independent keepalive: doing so
/// would make a broken device-to-Hub half-channel look healthy.
///
/// Crucially this wrapper owns no extra `DispatchSender`. That keeps
/// `PresenceRegistry` displacement semantics intact: when a same-URA
/// second session is admitted, dropping the displaced sender still
/// closes the old response stream immediately.
struct SessionProviderDownStream {
    down_rx: tokio::sync::mpsc::Receiver<Result<DispatchFrame, Status>>,
    pending_normal_frames: VecDeque<Result<DispatchFrame, Status>>,
    next_sequence: u64,
    /// Set to `Some(control)` at construction; first `poll_next`
    /// yields it and clears the slot. Subsequent polls follow the
    /// priority receive path.
    pending_initial_control: Option<InvokeBidiDown>,
}

pub(crate) struct LocalBidiDownStream {
    down_rx: tokio::sync::mpsc::Receiver<Result<InvokeBidiDown, Status>>,
    next_sequence: u64,
    pending_admission_receipt: Option<InvokeBidiDown>,
}

/// Stamp the transport-owned bidi down-stream sequence number and advance the
/// counter. Product providers emit unstamped frames; `LocalBidiDownStream` is
/// the single sequence owner for every tonic projection. The `saturating_add`
/// is intentional: at 2^64 frames per session the counter freezes at
/// u64::MAX rather than wrapping.
fn stamp_bidi_down_sequence(next: &mut u64, mut frame: InvokeBidiDown) -> InvokeBidiDown {
    frame.sequence = *next;
    *next = next.saturating_add(1);
    frame
}

impl LocalBidiDownStream {
    pub(crate) fn new(
        down_rx: tokio::sync::mpsc::Receiver<Result<InvokeBidiDown, Status>>,
    ) -> Self {
        Self {
            down_rx,
            next_sequence: 0,
            pending_admission_receipt: None,
        }
    }

    pub(crate) fn with_admission(
        down_rx: tokio::sync::mpsc::Receiver<Result<InvokeBidiDown, Status>>,
        admission_receipt: InvokeBidiDown,
    ) -> Self {
        Self {
            down_rx,
            next_sequence: 0,
            pending_admission_receipt: Some(admission_receipt),
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

impl SessionProviderDownStream {
    fn new(
        down_rx: tokio::sync::mpsc::Receiver<Result<DispatchFrame, Status>>,
        initial_control: InvokeBidiDown,
    ) -> Self {
        Self {
            down_rx,
            pending_normal_frames: VecDeque::new(),
            next_sequence: 0,
            pending_initial_control: Some(initial_control),
        }
    }

    fn stamp_sequence(&mut self, frame: InvokeBidiDown) -> InvokeBidiDown {
        stamp_bidi_down_sequence(&mut self.next_sequence, frame)
    }

    fn poll_dispatch_frame(
        &mut self,
        cx: &mut Context<'_>,
    ) -> Poll<Option<Result<DispatchFrame, Status>>> {
        for _ in 0..DISPATCH_CHANNEL_CAPACITY {
            match Pin::new(&mut self.down_rx).poll_recv(cx) {
                Poll::Ready(Some(Ok(frame))) if frame.is_control() => {
                    return Poll::Ready(Some(Ok(frame)));
                }
                Poll::Ready(Some(Ok(frame))) => {
                    self.pending_normal_frames.push_back(Ok(frame));
                }
                Poll::Ready(Some(Err(status))) => {
                    return Poll::Ready(Some(Err(status)));
                }
                Poll::Ready(None) => {
                    return Poll::Ready(self.pending_normal_frames.pop_front());
                }
                Poll::Pending => break,
            }
        }

        if let Some(frame) = self.pending_normal_frames.pop_front() {
            return Poll::Ready(Some(frame));
        }

        Poll::Pending
    }
}

impl Stream for SessionProviderDownStream {
    type Item = Result<InvokeBidiDown, Status>;

    fn poll_next(mut self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<Option<Self::Item>> {
        if let Some(control) = self.pending_initial_control.take() {
            return Poll::Ready(Some(Ok(self.stamp_sequence(control))));
        }

        match self.poll_dispatch_frame(cx) {
            Poll::Ready(Some(Ok(frame))) => Poll::Ready(Some(Ok(self.stamp_sequence(frame.frame)))),
            Poll::Ready(Some(Err(status))) => Poll::Ready(Some(Err(status))),
            Poll::Ready(None) => Poll::Ready(None),
            Poll::Pending => Poll::Pending,
        }
    }
}

impl BidiDispatcher {
    /// Admit and route an opaque canonical invocation received through a
    /// device-owned reverse session. The session proves transport liveness;
    /// caller authority remains exclusively in the request envelope and is
    /// revalidated here before any route or execution is attempted.
    async fn dispatch_canonical_session_invoke(
        &self,
        request: axon_sdk::pb::axon::v1::InvokeRequest,
    ) -> Result<axon_sdk::pb::axon::v1::InvokeResponse, SessionRequestError> {
        let (result, _) = self.unary.dispatch_local_rpc_selected_route(&request).await;
        match result {
            Ok(response) => Ok(response.into_inner()),
            Err(status) => Err(session_request_error_from_status(status)),
        }
    }

    async fn dispatch_canonical_session_stream(
        &self,
        request: axon_sdk::pb::axon::v1::InvokeRequest,
    ) -> Result<Response<BoxedDownStream<axon_sdk::pb::axon::v1::InvokeStreamChunk>>, Status> {
        let stream_request = axon_sdk::pb::axon::v1::InvokeServerStreamRequest {
            envelope: request.envelope,
            target: request.target,
            arguments: request.arguments,
            content_type: request.content_type,
            timeout_seconds: request.timeout_seconds,
            metadata: request.metadata,
            payload_ref: request.payload_ref,
            content_envelope: request.content_envelope,
        };
        self.stream_dispatcher()
            .dispatch_selected_route(&stream_request)
            .await
    }

    async fn dispatch_canonical_session_bidi(
        &self,
        request: InvokeRequest,
        up: BoxedUpStream<InvokeBidiUp>,
    ) -> Result<Response<BoxedDownStream<InvokeBidiDown>>, Status> {
        let envelope_open = invoke_request_to_bidi_open(request)?;
        let ability =
            crate::daemon::invocation::dispatch::invocation_wire::function_name_from_invocation_target(
                "canonical carrier reverse bidi call",
                envelope_open.target.as_ref(),
            )?;
        let selection = self.resolve_bidi_route(&envelope_open).await?;
        let call_mode = selection.call_mode();
        let selected_route = match selection.into_dispatch() {
            CanonicalRouteDispatch::Local(route) => route,
            CanonicalRouteDispatch::Peer(route) | CanonicalRouteDispatch::UpstreamHub(route) => {
                return Err(Status::unimplemented(format!(
                    "reverse InvokeBidi selected canonical peer route to hub `{}` for `{}`, but \
                     the generic cross-realm bidi carrier is unsupported; Device mode does not \
                     own a peer dialer",
                    route.hub_ura, route.query_name,
                )));
            }
            CanonicalRouteDispatch::HubSession(route) => {
                return self
                    .dispatch_hub_session_bidi(&route, &envelope_open, up)
                    .await;
            }
        };
        let execution_host_is_self = self
            .gate
            .matches_self_target_ura(&selected_route.execution_host_ura)
            .await;
        if selected_route
            .dispatch_target(execution_host_is_self)
            .is_local_runtime()
        {
            let wire_kind = self
                .runtime
                .ability_wire
                .bidi_wire_kind_for(&selected_route.dispatch_name)
                .ok_or_else(|| {
                    Status::unimplemented(format!(
                        "reverse InvokeBidi selected local route `{}` for ability `{ability}`, but \
                         dispatch ability `{}` has no daemon bidi wire adapter",
                        selected_route.route_ura, selected_route.dispatch_name,
                    ))
                })?;
            self.dispatch_local_bidi_selected_route(
                &envelope_open,
                up,
                selected_route,
                call_mode,
                wire_kind,
            )
            .await
        } else {
            self.dispatch_remote_bidi(&selected_route, &envelope_open, up, call_mode)
                .await
        }
    }

    fn stream_dispatcher(&self) -> StreamDispatcher {
        StreamDispatcher::new(
            self.admission.clone(),
            self.directory.clone(),
            self.sessions.clone(),
            self.runtime.clone(),
            self.gate.clone(),
            std::sync::Weak::new(),
        )
    }

    fn session_control_lifecycle_from_wire(
        &self,
        caller_device_ura: &str,
        ability_ura: &str,
        args: &[u8],
        args_content_envelope: &SessionContentEnvelope,
        metadata: HashMap<String, String>,
    ) -> Result<SessionControlLifecycle, SessionRequestError> {
        let kind =
            match session_control_kind_for_hub(self.identity.session_realm.as_deref(), ability_ura)
            {
                Ok(kind) => kind,
                Err(reason) => {
                    return Err(SessionRequestError::PermissionDenied { reason });
                }
            };
        let request = SessionControlRequest::from_validated_parts(
            kind,
            caller_device_ura,
            args,
            args_content_envelope,
            metadata,
        )?;
        Ok(SessionControlLifecycle::validated(request).schedule())
    }

    async fn dispatch_session_control_request(
        &self,
        request: &SessionControlRequest,
    ) -> RequestOutcome {
        let result = match request.kind {
            SessionControlRequestKind::AdvertiseAgent => {
                self.unary.dispatch_federation_advertise_agent_from_session(
                    &request.args,
                    &request.caller_device_ura,
                )
            }
            SessionControlRequestKind::AdvertiseAbilities => self
                .unary
                .dispatch_federation_advertise_abilities_from_session(
                    &request.args,
                    &request.caller_device_ura,
                    &request.metadata,
                ),
            SessionControlRequestKind::NamespaceResolve => {
                self.unary.dispatch_namespace_resolve(&request.args).await
            }
            SessionControlRequestKind::ResolveKey => {
                self.unary.dispatch_federation_resolve_key(&request.args)
            }
        };
        match result {
            Ok(result_bytes) => RequestOutcome::Ok { result_bytes },
            Err(status) => map_status_to_session_request_error(status),
        }
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
    RequestOutcome::Err {
        error: session_request_error_from_status(status),
    }
}

fn session_request_error_from_status(status: Status) -> SessionRequestError {
    let code = status.code();
    let message = status.message().to_string();
    if code == tonic::Code::FailedPrecondition
        && message.trim()
            == crate::daemon::invocation::bidi::state::presence::DISPATCH_TARGET_OFFLINE_REASON
    {
        return SessionRequestError::TargetOffline;
    }
    if code == tonic::Code::PermissionDenied {
        return SessionRequestError::PermissionDenied { reason: message };
    }
    SessionRequestError::UpstreamFailure {
        reason: format!("code={code:?} message={message}"),
    }
}

/// Build a `DispatchFrame` carrying a JSON-serialised
/// `SessionDispatch::RequestResult` ready to push back down a
/// device's `session.open` reverse channel. Encoding failure is
/// vanishingly unlikely (owned `[u8; 16]`, owned `Vec<u8>`,
/// typed enum) but mapped to a synthetic `UpstreamFailure` outcome
/// so a malformed inner result never silently wedges the device.
pub(crate) fn build_session_request_result_frame(
    call_id: [u8; 16],
    outcome: RequestOutcome,
) -> crate::daemon::invocation::bidi::state::presence::DispatchFrame {
    use axon_sdk::pb::axon::v1::invoke_bidi_down::Payload;
    use axon_sdk::pb::axon::v1::{BinaryChunk, InvokeBidiDown};

    let frame = SessionDispatch::RequestResult { call_id, outcome };
    let data = match frame.encode_frame() {
        Ok(bytes) => bytes,
        Err(err) => {
            // Replace the payload with a typed error so the device
            // sees a structured outcome instead of a malformed
            // frame. The id_hex stays in the eprintln below for
            // operator audit.
            let synthetic_error_result = SessionDispatch::RequestResult {
                call_id,
                outcome: RequestOutcome::Err {
                    error: SessionRequestError::UpstreamFailure {
                        reason: format!("encode RequestResult: {err}"),
                    },
                },
            };
            serde_json::to_vec(&synthetic_error_result)
                .expect("typed error variant must always encode")
        }
    };
    crate::daemon::invocation::bidi::state::presence::DispatchFrame::control(InvokeBidiDown {
        payload: Some(Payload::BinaryChunk(BinaryChunk {
            data,
            ..BinaryChunk::default()
        })),
        ..InvokeBidiDown::default()
    })
}

/// Insert one frame into the exact currently-admitted session generation.
///
/// This is the single bounded-backpressure authority for Hub → device session
/// traffic. A full result queue is a carrier failure, not a license to drop a
/// terminal frame while presence remains Online.
fn push_session_down_frame(
    presence: &Arc<PresenceRegistry>,
    caller_ura: &str,
    frame: crate::daemon::invocation::bidi::state::presence::DispatchFrame,
) -> SessionDownPush {
    let Some((session_id, sender)) = presence.lookup_tracked(caller_ura) else {
        return SessionDownPush::NoPresence;
    };
    match sender.try_send(Ok(frame)) {
        Ok(()) => SessionDownPush::Queued { session_id },
        Err(tokio::sync::mpsc::error::TrySendError::Full(_)) => {
            let _ = presence.remove_if_session(caller_ura, session_id, OfflineReason::SendFailed);
            SessionDownPush::RetiredBackpressured { session_id }
        }
        Err(tokio::sync::mpsc::error::TrySendError::Closed(_)) => {
            let _ = presence.remove_if_session(caller_ura, session_id, OfflineReason::StreamClosed);
            SessionDownPush::Closed { session_id }
        }
    }
}

/// Push a `RequestResult` frame back down the device's bidi via
/// the same PresenceRegistry-keyed `DispatchSender` the device's
/// session-accept handler registered. The device drains the down
/// stream in `session_initiator::dial_and_run_session` and routes
/// `RequestResult` frames to the `oneshot::Receiver` matching
/// `call_id` (per PR-N6 spec §"Concurrent multiplexing").
pub(crate) fn push_session_request_result(
    presence: &Arc<PresenceRegistry>,
    caller_ura: &str,
    id_hex: &str,
    frame: crate::daemon::invocation::bidi::state::presence::DispatchFrame,
) -> SessionDownPush {
    let push = push_session_down_frame(presence, caller_ura, frame);
    match push {
        SessionDownPush::Queued { .. } => {}
        SessionDownPush::NoPresence => {
            crate::op_event!(
                component = session_accept,
                kind = request_result_drop_no_presence,
                caller = caller_ura,
                call_id = id_hex,
                reason = "device_disconnected_mid_dispatch",
            );
        }
        SessionDownPush::RetiredBackpressured { .. } => {
            crate::op_event!(
                component = session_accept,
                kind = request_result_push_failed,
                caller = caller_ura,
                call_id = id_hex,
                reason = "down_channel_backpressured_session_retired",
                offline_reason = "SendFailed",
            );
        }
        SessionDownPush::Closed { .. } => {
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
    push
}

fn acknowledge_session_heartbeat(
    presence: &Arc<PresenceRegistry>,
    caller_ura: &str,
) -> SessionDownPush {
    let push = push_session_down_frame(presence, caller_ura, build_session_heartbeat_ack_frame());
    if !matches!(push, SessionDownPush::Queued { .. }) {
        crate::op_event!(
            component = session_accept,
            kind = heartbeat_ack_push_failed,
            caller = caller_ura,
            outcome = push.as_wire_str(),
        );
    }
    push
}

fn reject_reverse_dispatch_call(
    presence: &Arc<PresenceRegistry>,
    caller_ura: &str,
    call_id: [u8; 16],
    reason: impl Into<String>,
) {
    use axon_sdk::pb::axon::v1::ReverseDispatchResult;

    let id_hex = call_id_hex(&call_id);
    let frame = DispatchFrame::control(InvokeBidiDown {
        payload: Some(DownPayload::ReverseDispatchResult(ReverseDispatchResult {
            call_id: call_id.to_vec(),
            terminal: true,
            failure: Some(axon_sdk::pb::axon::v1::Error {
                code: "INVALID_ARGUMENT".to_string(),
                message: reason.into(),
                retryable: false,
                ..Default::default()
            }),
            ..Default::default()
        })),
        ..Default::default()
    });
    let _ = push_session_request_result(presence, caller_ura, &id_hex, frame);
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum SessionDownPush {
    Queued {
        session_id: crate::daemon::invocation::bidi::state::presence::PresenceSessionId,
    },
    RetiredBackpressured {
        session_id: crate::daemon::invocation::bidi::state::presence::PresenceSessionId,
    },
    Closed {
        session_id: crate::daemon::invocation::bidi::state::presence::PresenceSessionId,
    },
    NoPresence,
}

impl SessionDownPush {
    fn as_wire_str(self) -> &'static str {
        match self {
            Self::Queued { .. } => "queued",
            Self::RetiredBackpressured { .. } => "retired_backpressured",
            Self::Closed { .. } => "closed",
            Self::NoPresence => "no_presence",
        }
    }

    fn session_id(
        self,
    ) -> Option<crate::daemon::invocation::bidi::state::presence::PresenceSessionId> {
        match self {
            Self::Queued { session_id }
            | Self::RetiredBackpressured { session_id }
            | Self::Closed { session_id } => Some(session_id),
            Self::NoPresence => None,
        }
    }
}

fn self_revoke_target_for_reverse_dispatch(
    caller_ura: &str,
    ability: &str,
    request: &axon_sdk::pb::axon::v1::InvokeRequest,
) -> Option<String> {
    if ability
        != crate::daemon::invocation::dispatch::federation_wrappers::ABILITY_FEDERATION_REVOKE
    {
        return None;
    }
    let revoke: crate::daemon::invocation::dispatch::federation_wrappers::RevokeRequest =
        serde_json::from_slice(&request.arguments).ok()?;
    let target_ura = revoke.agent_ura.trim();
    if target_ura == caller_ura {
        Some(target_ura.to_string())
    } else {
        None
    }
}

fn remove_deferred_self_revoke_presence(
    presence: &Arc<PresenceRegistry>,
    caller_ura: &str,
    id_hex: &str,
    push: SessionDownPush,
) {
    let Some(session_id) = push.session_id() else {
        return;
    };
    let removed = presence
        .remove_if_session(caller_ura, session_id, OfflineReason::AdminRevoked)
        .is_some();
    crate::op_event!(
        component = session_accept,
        kind = self_revoke_presence_removed_after_result,
        caller = caller_ura,
        call_id = id_hex,
        removed = removed,
    );
}

/// Map a canonical typed failure into the session-plane projection used by
/// pending callers. `error` is the human-readable projection; `failure`
/// preserves the typed class.
pub(crate) fn pending_result_from_canonical_carrier(
    result: &axon_sdk::pb::axon::v1::DispatchResult,
) -> DispatchResult {
    DispatchResult {
        payload: result.payload.clone(),
        result_content_type: result.result_content_type.clone(),
        error: result
            .failure
            .as_ref()
            .map(|f| f.message.clone())
            .filter(|m| !m.is_empty()),
        failure: result.failure.as_ref().map(session_failure_from_axon_error),
        request_id: None,
        admission_receipt: result.admission_receipt.clone(),
        terminal_receipt: result.terminal_receipt.clone(),
    }
}

#[derive(Debug)]
enum CarrierDispatchEvent {
    Admission(Box<axon_sdk::pb::axon::v1::InvocationReceipt>),
    Chunk(Vec<u8>),
    Terminal(Box<DispatchResult>),
}

fn classify_canonical_carrier_result(
    result: axon_sdk::pb::axon::v1::DispatchResult,
) -> Result<(u64, CarrierDispatchEvent), (u64, DispatchResult)> {
    let call_id = result.call_id;
    let protocol_failure =
        |reason: &str, code: &str| (call_id, failed_dispatch_result(reason, code, false));

    if !result.terminal
        && result.failure.is_some()
        && result.admission_receipt.is_none()
        && result.terminal_receipt.is_none()
        && result.payload.is_empty()
    {
        return Err((call_id, pending_result_from_canonical_carrier(&result)));
    }

    // Pending unary and stream dispatches intentionally occupy disjoint
    // session-wide namespaces. The carrier lifecycle therefore has one
    // unambiguous checkpoint geometry without a mode tag on every result.
    if call_id & 1 == 0 {
        if !result.terminal {
            return Err(protocol_failure(
                "unary DispatchResult must be terminal",
                "CARRIER_UNARY_PHASE_INVALID",
            ));
        }
        let has_admission = result.admission_receipt.is_some();
        let has_terminal = result.terminal_receipt.is_some();
        if has_admission != has_terminal {
            return Err(protocol_failure(
                "unary DispatchResult must carry admission and terminal checkpoints together",
                "CANONICAL_CHECKPOINT_PAIR_REQUIRED",
            ));
        }
        if result.failure.is_none() && !has_admission {
            return Err(protocol_failure(
                "successful unary DispatchResult omitted canonical checkpoints",
                "CANONICAL_FINALIZATION_REQUIRED",
            ));
        }
        return Ok((
            call_id,
            CarrierDispatchEvent::Terminal(Box::new(pending_result_from_canonical_carrier(
                &result,
            ))),
        ));
    }

    if result.terminal {
        if result.admission_receipt.is_some() {
            return Err(protocol_failure(
                "stream terminal DispatchResult repeated the admission checkpoint",
                "CARRIER_STREAM_PHASE_INVALID",
            ));
        }
        if result.terminal_receipt.is_none() {
            return Err(protocol_failure(
                "stream terminal DispatchResult omitted its canonical terminal checkpoint",
                "CANONICAL_TERMINAL_RECEIPT_REQUIRED",
            ));
        }
        return Ok((
            call_id,
            CarrierDispatchEvent::Terminal(Box::new(pending_result_from_canonical_carrier(
                &result,
            ))),
        ));
    }

    if result.terminal_receipt.is_some() {
        return Err(protocol_failure(
            "non-terminal DispatchResult carried a terminal checkpoint",
            "CARRIER_STREAM_PHASE_INVALID",
        ));
    }
    if let Some(admission) = result.admission_receipt.clone() {
        if result.failure.is_some() || !result.payload.is_empty() {
            return Err(protocol_failure(
                "stream admission DispatchResult must contain only the admission checkpoint",
                "CARRIER_STREAM_PHASE_INVALID",
            ));
        }
        if admission.state != axon_sdk::invocation::InvocationState::Admitted.to_wire_i32() {
            return Err(protocol_failure(
                "stream admission DispatchResult carried a non-admission checkpoint",
                "CANONICAL_ADMISSION_INVALID",
            ));
        }
        return Ok((
            call_id,
            CarrierDispatchEvent::Admission(Box::new(admission)),
        ));
    }
    if result.failure.is_some() {
        return Err((call_id, pending_result_from_canonical_carrier(&result)));
    }
    Ok((call_id, CarrierDispatchEvent::Chunk(result.payload)))
}

pub(crate) fn session_failure_from_axon_error(
    err: &axon_sdk::pb::axon::v1::Error,
) -> SessionFailure {
    SessionFailure::from_reason(
        &err.message,
        if err.code.is_empty() {
            "INVOCATION_FAILED"
        } else {
            err.code.as_str()
        },
        err.retryable,
    )
}

/// Hub → device reply for a canonical carrier reverse request. Failures ride
/// the single-track typed Error (DEC-F004 point 3).
pub(crate) fn build_reverse_dispatch_result_frame(
    call_id: [u8; 16],
    outcome: Result<axon_sdk::pb::axon::v1::InvokeResponse, SessionRequestError>,
) -> DispatchFrame {
    use axon_sdk::pb::axon::v1::ReverseDispatchResult;
    let (payload, result_content_type, failure, admission_receipt, terminal_receipt) = match outcome
    {
        Ok(response) => {
            if response.admission_receipt.is_none() || response.terminal_receipt.is_none() {
                (
                    Vec::new(),
                    String::new(),
                    Some(axon_sdk::pb::axon::v1::Error {
                        code: "CANONICAL_FINALIZATION_REQUIRED".to_string(),
                        message: "hub canonical reverse dispatch completed without both signed finalization checkpoints".to_string(),
                        retryable: false,
                        ..axon_sdk::pb::axon::v1::Error::default()
                    }),
                    None,
                    None,
                )
            } else {
                (
                    response.result,
                    response.result_content_type,
                    None,
                    response.admission_receipt,
                    response.terminal_receipt,
                )
            }
        }
        Err(error) => {
            let (code, retryable) = match &error {
                SessionRequestError::TargetOffline => ("TARGET_OFFLINE", true),
                SessionRequestError::PermissionDenied { .. } => ("PERMISSION_DENIED", false),
                SessionRequestError::UpstreamFailure { .. } => ("UPSTREAM_FAILURE", true),
                SessionRequestError::UpstreamTimeout => ("UPSTREAM_TIMEOUT", true),
            };
            let message = match &error {
                SessionRequestError::TargetOffline => "target offline".to_string(),
                SessionRequestError::PermissionDenied { reason }
                | SessionRequestError::UpstreamFailure { reason } => reason.clone(),
                SessionRequestError::UpstreamTimeout => "upstream timeout".to_string(),
            };
            (
                Vec::new(),
                String::new(),
                Some(axon_sdk::pb::axon::v1::Error {
                    code: code.to_string(),
                    message,
                    retryable,
                    ..axon_sdk::pb::axon::v1::Error::default()
                }),
                None,
                None,
            )
        }
    };
    DispatchFrame::control(InvokeBidiDown {
        payload: Some(DownPayload::ReverseDispatchResult(ReverseDispatchResult {
            call_id: call_id.to_vec(),
            payload,
            result_content_type,
            terminal: true,
            failure,
            admission_receipt,
            terminal_receipt,
        })),
        ..InvokeBidiDown::default()
    })
}

async fn forward_reverse_dispatch_stream_results(
    mut stream: BoxedDownStream<axon_sdk::pb::axon::v1::InvokeStreamChunk>,
    presence: &Arc<PresenceRegistry>,
    caller_ura: &str,
    id_hex: &str,
    call_id: [u8; 16],
    cancel: CancellationToken,
) {
    loop {
        let next = tokio::select! {
            () = cancel.cancelled() => {
                crate::op_event!(
                    component = session_accept,
                    kind = canonical_carrier_reverse_stream_cancelled,
                    caller = caller_ura,
                    call_id = id_hex,
                );
                return;
            }
            next = stream.next() => next,
        };
        let Some(next) = next else {
            break;
        };
        let frame = match next {
            Ok(chunk) => build_reverse_dispatch_stream_chunk_frame(call_id, chunk),
            Err(status) => build_reverse_dispatch_stream_failure_frame(
                call_id,
                session_request_error_from_status(status),
            ),
        };
        let terminal = reverse_dispatch_frame_is_terminal(&frame);
        let push = push_session_request_result(presence, caller_ura, id_hex, frame);
        crate::op_event!(
            component = session_accept,
            kind = canonical_carrier_reverse_bidi_result_pushed,
            caller = caller_ura,
            call_id = id_hex,
            terminal = terminal,
            push = format!("{push:?}"),
        );
        if terminal {
            return;
        }
    }
    let frame = build_reverse_dispatch_stream_failure_frame(
        call_id,
        SessionRequestError::UpstreamFailure {
            reason: "hub reverse stream ended without terminal checkpoint".to_string(),
        },
    );
    push_session_request_result(presence, caller_ura, id_hex, frame);
}

async fn forward_reverse_dispatch_bidi_results(
    mut stream: BoxedDownStream<InvokeBidiDown>,
    presence: &Arc<PresenceRegistry>,
    caller_ura: &str,
    id_hex: &str,
    call_id: [u8; 16],
) {
    while let Some(next) = stream.next().await {
        let frame = match next {
            Ok(frame) => match build_reverse_dispatch_bidi_result_frame(call_id, frame) {
                Some(frame) => frame,
                None => continue,
            },
            Err(status) => build_reverse_dispatch_stream_failure_frame(
                call_id,
                session_request_error_from_status(status),
            ),
        };
        let terminal = reverse_dispatch_frame_is_terminal(&frame);
        let push = push_session_request_result(presence, caller_ura, id_hex, frame);
        crate::op_event!(
            component = session_accept,
            kind = canonical_carrier_reverse_bidi_result_pushed,
            caller = caller_ura,
            call_id = id_hex,
            terminal = terminal,
            push = format!("{push:?}"),
        );
        if terminal {
            return;
        }
    }
    let frame = build_reverse_dispatch_stream_failure_frame(
        call_id,
        SessionRequestError::UpstreamFailure {
            reason: "hub reverse bidi ended without terminal checkpoint".to_string(),
        },
    );
    let push = push_session_request_result(presence, caller_ura, id_hex, frame);
    crate::op_event!(
        component = session_accept,
        kind = canonical_carrier_reverse_bidi_result_pushed,
        caller = caller_ura,
        call_id = id_hex,
        terminal = true,
        push = format!("{push:?}"),
    );
}

fn build_reverse_dispatch_bidi_result_frame(
    call_id: [u8; 16],
    frame: InvokeBidiDown,
) -> Option<DispatchFrame> {
    use axon_sdk::pb::axon::v1::ReverseDispatchResult;
    let payload = frame.payload?;
    let result = match payload {
        DownPayload::Receipt(receipt) => {
            if receipt.state == axon_sdk::invocation::InvocationState::Admitted.to_wire_i32() {
                ReverseDispatchResult {
                    call_id: call_id.to_vec(),
                    admission_receipt: Some(receipt),
                    ..ReverseDispatchResult::default()
                }
            } else {
                let terminal_state = receipt.state;
                let terminal_payload = receipt.payload.clone();
                let terminal_content_type = receipt.payload_content_type.clone();
                let terminal_failure = receipt.failure.clone();
                ReverseDispatchResult {
                    call_id: call_id.to_vec(),
                    terminal: true,
                    payload: if terminal_state
                        == axon_sdk::invocation::InvocationState::Completed.to_wire_i32()
                    {
                        terminal_payload
                    } else {
                        Vec::new()
                    },
                    result_content_type: if terminal_state
                        == axon_sdk::invocation::InvocationState::Completed.to_wire_i32()
                    {
                        terminal_content_type
                    } else {
                        String::new()
                    },
                    failure: terminal_failure,
                    terminal_receipt: Some(receipt),
                    ..ReverseDispatchResult::default()
                }
            }
        }
        DownPayload::BinaryChunk(chunk) => ReverseDispatchResult {
            call_id: call_id.to_vec(),
            payload: chunk.data,
            ..ReverseDispatchResult::default()
        },
        DownPayload::ReverseDispatchResult(mut result) => {
            result.call_id = call_id.to_vec();
            result
        }
        DownPayload::Control(_) | DownPayload::DispatchCall(_) => return None,
    };
    Some(DispatchFrame::control(InvokeBidiDown {
        payload: Some(DownPayload::ReverseDispatchResult(result)),
        ..InvokeBidiDown::default()
    }))
}

fn build_reverse_dispatch_stream_chunk_frame(
    call_id: [u8; 16],
    chunk: axon_sdk::pb::axon::v1::InvokeStreamChunk,
) -> DispatchFrame {
    use axon_sdk::pb::axon::v1::ReverseDispatchResult;
    DispatchFrame::control(InvokeBidiDown {
        payload: Some(DownPayload::ReverseDispatchResult(ReverseDispatchResult {
            call_id: call_id.to_vec(),
            payload: chunk.payload,
            result_content_type: chunk.content_type,
            terminal: chunk.terminal,
            failure: chunk.error,
            admission_receipt: chunk.admission_receipt,
            terminal_receipt: chunk.terminal_receipt,
        })),
        ..InvokeBidiDown::default()
    })
}

fn build_reverse_dispatch_stream_failure_frame(
    call_id: [u8; 16],
    error: SessionRequestError,
) -> DispatchFrame {
    use axon_sdk::pb::axon::v1::ReverseDispatchResult;
    let (code, retryable) = match &error {
        SessionRequestError::TargetOffline => ("TARGET_OFFLINE", true),
        SessionRequestError::PermissionDenied { .. } => ("PERMISSION_DENIED", false),
        SessionRequestError::UpstreamFailure { .. } => ("UPSTREAM_FAILURE", true),
        SessionRequestError::UpstreamTimeout => ("UPSTREAM_TIMEOUT", true),
    };
    let message = match error {
        SessionRequestError::TargetOffline => "target offline".to_string(),
        SessionRequestError::PermissionDenied { reason }
        | SessionRequestError::UpstreamFailure { reason } => reason,
        SessionRequestError::UpstreamTimeout => "upstream timeout".to_string(),
    };
    DispatchFrame::control(InvokeBidiDown {
        payload: Some(DownPayload::ReverseDispatchResult(ReverseDispatchResult {
            call_id: call_id.to_vec(),
            terminal: true,
            failure: Some(axon_sdk::pb::axon::v1::Error {
                code: code.to_string(),
                message,
                retryable,
                ..axon_sdk::pb::axon::v1::Error::default()
            }),
            ..ReverseDispatchResult::default()
        })),
        ..InvokeBidiDown::default()
    })
}

fn reverse_dispatch_frame_is_terminal(frame: &DispatchFrame) -> bool {
    matches!(
        frame.frame.payload.as_ref(),
        Some(DownPayload::ReverseDispatchResult(result)) if result.terminal
    )
}

/// Terminal-result settlement shared by the JSON `Result` arm and the
/// canonical carrier `DispatchResult` arm: streaming map first, then unary map,
/// every miss surfaced (DEC-F004 — one settle path, not two).
///
/// Deliberately non-blocking: this runs on the session drain — the
/// only reader of the device's whole `session.open` — so it must
/// never wait on one call's consumer. A stalled streaming consumer
/// costs that call alone (`ConsumerStalled`), not the session.
async fn settle_terminal_result(
    pending: &Option<Arc<PendingDispatchMap>>,
    pending_stream: &Option<Arc<PendingStreamDispatchMap>>,
    caller_ura: &str,
    call_id: u64,
    dispatch_result: DispatchResult,
) {
    let mut completed = false;
    if let Some(pending_stream) = pending_stream.as_ref() {
        match pending_stream
            .deliver_terminal(call_id, dispatch_result.clone())
            .await
        {
            crate::daemon::invocation::bidi::state::pending_dispatch::StreamDeliver::Delivered => {
                completed = true
            }
            crate::daemon::invocation::bidi::state::pending_dispatch::StreamDeliver::ConsumerStalled => {
                crate::op_event!(
                    component = session_accept,
                    kind = terminal_result_consumer_stalled,
                    caller = caller_ura,
                    call_id = call_id,
                    note = "terminal dropped; consumer stopped draining its chunks",
                );
                return;
            }
            crate::daemon::invocation::bidi::state::pending_dispatch::StreamDeliver::NoMatch => {}
        }
    }
    if !completed {
        let Some(pending) = pending.as_ref() else {
            crate::op_event!(
                component = session_accept,
                kind = terminal_result_dropped_no_pending_map,
                caller = caller_ura,
                call_id = call_id,
            );
            return;
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
}

/// Surface a non-`Delivered` chunk delivery from the session drain.
/// `ConsumerStalled` means the entry was evicted: that one call is
/// cut so the drain (and every other invocation on the device's
/// session) keeps flowing.
fn report_chunk_delivery(
    outcome: crate::daemon::invocation::bidi::state::pending_dispatch::StreamDeliver,
    caller_ura: &str,
    call_id: u64,
) {
    match outcome {
        crate::daemon::invocation::bidi::state::pending_dispatch::StreamDeliver::Delivered => {}
        crate::daemon::invocation::bidi::state::pending_dispatch::StreamDeliver::NoMatch => {
            crate::op_event!(
                component = session_accept,
                kind = streaming_result_chunk_no_match,
                caller = caller_ura,
                call_id = call_id,
            );
        }
        crate::daemon::invocation::bidi::state::pending_dispatch::StreamDeliver::ConsumerStalled => {
            crate::op_event!(
                component = session_accept,
                kind = streaming_result_consumer_stalled,
                caller = caller_ura,
                call_id = call_id,
                note = "pending entry evicted; stalled call cancelled to protect the session drain",
            );
        }
    }
}

struct ReverseBidiIngress {
    sender: mpsc::Sender<Result<InvokeBidiUp, Status>>,
    next_sequence: u64,
}

/// Owns the lifecycle projection for server streams opened in the reverse
/// direction of `session.open`. A token exists from open acceptance until the
/// forwarding task observes terminal or cancellation; session retirement
/// cancels every remaining token atomically.
#[derive(Default)]
struct ReverseStreamCancellations {
    inner: Mutex<HashMap<[u8; 16], CancellationToken>>,
}

impl ReverseStreamCancellations {
    fn open(&self, call_id: [u8; 16]) -> CancellationToken {
        let token = CancellationToken::new();
        let mut guard = match self.inner.lock() {
            Ok(guard) => guard,
            Err(poisoned) => poisoned.into_inner(),
        };
        if let Some(previous) = guard.insert(call_id, token.clone()) {
            previous.cancel();
        }
        token
    }

    fn cancel(&self, call_id: &[u8; 16]) -> bool {
        let token = {
            let guard = match self.inner.lock() {
                Ok(guard) => guard,
                Err(poisoned) => poisoned.into_inner(),
            };
            guard.get(call_id).cloned()
        };
        token.is_some_and(|token| {
            token.cancel();
            true
        })
    }

    fn retire(&self, call_id: &[u8; 16]) {
        let mut guard = match self.inner.lock() {
            Ok(guard) => guard,
            Err(poisoned) => poisoned.into_inner(),
        };
        guard.remove(call_id);
    }

    fn retire_all(&self) {
        let tokens = {
            let mut guard = match self.inner.lock() {
                Ok(guard) => guard,
                Err(poisoned) => poisoned.into_inner(),
            };
            guard.drain().map(|(_, token)| token).collect::<Vec<_>>()
        };
        for token in tokens {
            token.cancel();
        }
    }
}

fn reverse_bidi_input_to_up_frame(
    input: axon_sdk::pb::axon::v1::ReverseBidiInput,
    sequence: u64,
) -> Option<InvokeBidiUp> {
    use axon_sdk::pb::axon::v1::reverse_bidi_input::Input;
    let payload = match input.input? {
        Input::BinaryChunk(chunk) => UpPayload::BinaryChunk(chunk),
        Input::Control(control) => UpPayload::Control(control),
    };
    Some(InvokeBidiUp {
        sequence,
        payload: Some(payload),
        ..InvokeBidiUp::default()
    })
}

/// Drain a device's runtime-owned `session.open` inbox. Each frame may carry
/// typed `DispatchResult` / `ReverseDispatchCall` frames or the
/// remaining JSON streaming/control frames. Matching pending entries are
/// settled by call_id.
///
/// The returned reason is consumed by `SessionPresenceLease`; this function
/// never mutates lifecycle ownership itself.
async fn drain_session_runtime_up_stream(
    context: Arc<AbilityContext>,
    caller_ura: String,
    presence: Arc<PresenceRegistry>,
    pending: Option<Arc<PendingDispatchMap>>,
    pending_stream: Option<Arc<PendingStreamDispatchMap>>,
    dispatcher: BidiDispatcher,
) -> OfflineReason {
    use axon_sdk::pb::axon::v1::invoke_bidi_up::Payload as UpPayload;

    let mut close_reason = OfflineReason::StreamClosed;
    let mut expected_up_sequence = 1_u64;
    let reverse_bidi_inputs: Arc<Mutex<HashMap<[u8; 16], ReverseBidiIngress>>> =
        Arc::new(Mutex::new(HashMap::new()));
    let reverse_stream_cancellations = Arc::new(ReverseStreamCancellations::default());

    while let Some(message) = context.recv_message(None).await {
        if message.content_type == SESSION_RUNTIME_TRANSPORT_ERROR_CONTENT_TYPE {
            let chain = String::from_utf8_lossy(&message.payload);
            crate::op_event!(
                component = session_accept,
                kind = up_stream_error,
                caller = caller_ura,
                chain = chain.as_ref(),
                code = "transport",
            );
            close_reason = OfflineReason::StreamReset;
            break;
        }
        if message.content_type != SESSION_RUNTIME_FRAME_CONTENT_TYPE {
            crate::op_event!(
                component = session_accept,
                kind = malformed_runtime_session_frame,
                caller = caller_ura,
                content_type = message.content_type,
            );
            close_reason = OfflineReason::StreamReset;
            break;
        }
        let frame = match InvokeBidiUp::decode(message.payload.as_slice()) {
            Ok(frame) => frame,
            Err(error) => {
                crate::op_event!(
                    component = session_accept,
                    kind = malformed_runtime_session_frame,
                    caller = caller_ura,
                    error = error.to_string(),
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
        let payload_kind = match frame.payload.as_ref() {
            Some(UpPayload::EnvelopeOpen(_)) => "EnvelopeOpen",
            Some(UpPayload::BinaryChunk(_)) => "BinaryChunk",
            Some(UpPayload::Control(_)) => "Control",
            Some(UpPayload::DispatchResult(_)) => "DispatchResult",
            Some(UpPayload::ReverseDispatchCall(_)) => "ReverseDispatchCall",
            Some(UpPayload::ReverseBidiInput(_)) => "ReverseBidiInput",
            None => "None",
        };
        crate::op_event!(
            component = session_accept,
            kind = session_up_payload_received,
            caller = caller_ura,
            sequence = frame.sequence,
            payload = payload_kind,
        );

        let chunk = match frame.payload {
            Some(UpPayload::BinaryChunk(c)) => c,
            Some(UpPayload::Control(control)) => {
                if matches!(
                    control.control,
                    Some(axon_sdk::pb::axon::v1::bidi_control::Control::Eof(true))
                ) {
                    break;
                }
                let refreshed = refresh_session_owner_projection_lease(&dispatcher, &caller_ura);
                if refreshed {
                    crate::op_event!(
                        component = session_accept,
                        kind = up_heartbeat_projection_lease_refreshed,
                        caller = caller_ura,
                    );
                }
                match acknowledge_session_heartbeat(&presence, &caller_ura) {
                    SessionDownPush::Queued { .. } => {}
                    SessionDownPush::RetiredBackpressured { .. } => {
                        close_reason = OfflineReason::SendFailed;
                        break;
                    }
                    SessionDownPush::Closed { .. } | SessionDownPush::NoPresence => {
                        close_reason = OfflineReason::StreamClosed;
                        break;
                    }
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
            // Canonical carrier has one checkpoint geometry per call mode. The
            // even/odd pending-call namespaces make mode classification exact.
            Some(UpPayload::DispatchResult(result)) => {
                match classify_canonical_carrier_result(result) {
                    Ok((call_id, CarrierDispatchEvent::Admission(receipt))) => {
                        let receipt = *receipt;
                        crate::op_event!(
                            component = session_accept,
                            kind = canonical_carrier_admission_receipt_received,
                            caller = caller_ura,
                            call_id = call_id,
                            receipt_state = receipt.state,
                        );
                        if let Some(pending_stream) = pending_stream.as_ref() {
                            report_chunk_delivery(
                                pending_stream.deliver_admission(call_id, receipt).await,
                                &caller_ura,
                                call_id,
                            );
                        }
                    }
                    Ok((call_id, CarrierDispatchEvent::Chunk(payload))) => {
                        if let Some(pending_stream) = pending_stream.as_ref() {
                            report_chunk_delivery(
                                pending_stream.deliver_chunk(call_id, payload).await,
                                &caller_ura,
                                call_id,
                            );
                        }
                    }
                    Ok((call_id, CarrierDispatchEvent::Terminal(mapped))) => {
                        settle_terminal_result(
                            &pending,
                            &pending_stream,
                            &caller_ura,
                            call_id,
                            *mapped,
                        )
                        .await;
                    }
                    Err((call_id, mapped)) => {
                        settle_terminal_result(
                            &pending,
                            &pending_stream,
                            &caller_ura,
                            call_id,
                            mapped,
                        )
                        .await;
                    }
                }
                continue;
            }
            Some(UpPayload::ReverseBidiInput(input)) => {
                let Ok(call_id) = <[u8; 16]>::try_from(input.call_id.as_slice()) else {
                    crate::op_event!(
                        component = session_accept,
                        kind = canonical_carrier_reverse_bidi_input_bad_id,
                        caller = caller_ura,
                        id_len = input.call_id.len(),
                    );
                    continue;
                };
                let (sender, sequence) = {
                    let mut guard = match reverse_bidi_inputs.lock() {
                        Ok(guard) => guard,
                        Err(poisoned) => poisoned.into_inner(),
                    };
                    let Some(ingress) = guard.get_mut(&call_id) else {
                        crate::op_event!(
                            component = session_accept,
                            kind = canonical_carrier_reverse_bidi_input_no_open,
                            caller = caller_ura,
                            call_id = call_id_hex(&call_id),
                        );
                        continue;
                    };
                    let sequence = ingress.next_sequence;
                    ingress.next_sequence = ingress.next_sequence.saturating_add(1);
                    (ingress.sender.clone(), sequence)
                };
                let Some(frame) = reverse_bidi_input_to_up_frame(input, sequence) else {
                    continue;
                };
                crate::op_event!(
                    component = session_accept,
                    kind = canonical_carrier_reverse_bidi_input,
                    caller = caller_ura,
                    call_id = call_id_hex(&call_id),
                    sequence = sequence,
                );
                if sender.send(Ok(frame)).await.is_err() {
                    let mut guard = match reverse_bidi_inputs.lock() {
                        Ok(guard) => guard,
                        Err(poisoned) => poisoned.into_inner(),
                    };
                    guard.remove(&call_id);
                }
                continue;
            }
            Some(UpPayload::ReverseDispatchCall(call)) => {
                let Ok(call_id) = <[u8; 16]>::try_from(call.call_id.as_slice()) else {
                    crate::op_event!(
                        component = session_accept,
                        kind = canonical_carrier_reverse_call_bad_id,
                        caller = caller_ura,
                        id_len = call.call_id.len(),
                    );
                    continue;
                };
                let Some(request) = call.request else {
                    crate::op_event!(
                        component = session_accept,
                        kind = canonical_carrier_reverse_call_missing_request,
                        caller = caller_ura,
                        call_id = call_id_hex(&call_id),
                    );
                    reject_reverse_dispatch_call(
                        &presence,
                        &caller_ura,
                        call_id,
                        "canonical ReverseDispatchCall requires a complete InvokeRequest",
                    );
                    continue;
                };
                let call_mode = match canonical_dispatch_call_mode(call.call_mode) {
                    Ok(call_mode) => call_mode,
                    Err(error) => {
                        crate::op_event!(
                            component = session_accept,
                            kind = canonical_reverse_call_invalid_mode,
                            caller = caller_ura,
                            call_id = call_id_hex(&call_id),
                            error = error,
                        );
                        reject_reverse_dispatch_call(
                            &presence,
                            &caller_ura,
                            call_id,
                            format!("canonical ReverseDispatchCall call_mode invalid: {error}"),
                        );
                        continue;
                    }
                };
                let ability =
                    match
                    crate::daemon::invocation::dispatch::invocation_wire::function_name_from_invocation_target(
                        "canonical carrier reverse call",
                        request.target.as_ref(),
                    )
                    {
                        Ok(ability) => ability.to_string(),
                        Err(status) => {
                            crate::op_event!(
                                component = session_accept,
                                kind = canonical_carrier_reverse_call_missing_typed_target,
                                caller = caller_ura,
                                call_id = call_id_hex(&call_id),
                            );
                            reject_reverse_dispatch_call(
                                &presence,
                                &caller_ura,
                                call_id,
                                status.message(),
                            );
                            continue;
                        }
                    };
                let id_hex = call_id_hex(&call_id);
                crate::op_event!(
                    component = daemon_invocation,
                    kind = session_accept_request_frame,
                    call_id = id_hex,
                    ability = ability.as_str(),
                );
                // Same off-drain dispatch discipline as the JSON
                // Request arm: a slow inner call must not stall
                // subsequent up-frames.
                let dispatcher_for_request = dispatcher.clone();
                let presence_for_reply = Arc::clone(&presence);
                let caller_ura_for_reply = caller_ura.clone();
                let reverse_stream_cancellations_for_request =
                    Arc::clone(&reverse_stream_cancellations);
                if matches!(call_mode, CallMode::Bidi) {
                    crate::op_event!(
                        component = session_accept,
                        kind = canonical_carrier_reverse_bidi_opened,
                        caller = caller_ura,
                        call_id = id_hex,
                        ability = ability,
                    );
                    let (reverse_up_tx, reverse_up_rx) =
                        mpsc::channel::<Result<InvokeBidiUp, Status>>(16);
                    {
                        let mut guard = match reverse_bidi_inputs.lock() {
                            Ok(guard) => guard,
                            Err(poisoned) => poisoned.into_inner(),
                        };
                        guard.insert(
                            call_id,
                            ReverseBidiIngress {
                                sender: reverse_up_tx,
                                next_sequence: 1,
                            },
                        );
                    }
                    let reverse_bidi_inputs_for_task = Arc::clone(&reverse_bidi_inputs);
                    tokio::spawn(async move {
                        let up_stream =
                            Box::pin(tokio_stream::wrappers::ReceiverStream::new(reverse_up_rx))
                                as BoxedUpStream<InvokeBidiUp>;
                        match dispatcher_for_request
                            .dispatch_canonical_session_bidi(request, up_stream)
                            .await
                        {
                            Ok(response) => {
                                forward_reverse_dispatch_bidi_results(
                                    response.into_inner(),
                                    &presence_for_reply,
                                    &caller_ura_for_reply,
                                    &id_hex,
                                    call_id,
                                )
                                .await;
                            }
                            Err(status) => {
                                let frame = build_reverse_dispatch_stream_failure_frame(
                                    call_id,
                                    session_request_error_from_status(status),
                                );
                                push_session_request_result(
                                    &presence_for_reply,
                                    &caller_ura_for_reply,
                                    &id_hex,
                                    frame,
                                );
                            }
                        }
                        let mut guard = match reverse_bidi_inputs_for_task.lock() {
                            Ok(guard) => guard,
                            Err(poisoned) => poisoned.into_inner(),
                        };
                        guard.remove(&call_id);
                    });
                    continue;
                }
                tokio::spawn(async move {
                    if matches!(call_mode, CallMode::Stream) {
                        let cancel = reverse_stream_cancellations_for_request.open(call_id);
                        let reverse_stream_cancellations_for_task =
                            Arc::clone(&reverse_stream_cancellations_for_request);
                        match dispatcher_for_request
                            .dispatch_canonical_session_stream(request)
                            .await
                        {
                            Ok(response) => {
                                forward_reverse_dispatch_stream_results(
                                    response.into_inner(),
                                    &presence_for_reply,
                                    &caller_ura_for_reply,
                                    &id_hex,
                                    call_id,
                                    cancel,
                                )
                                .await;
                            }
                            Err(status) => {
                                let frame = build_reverse_dispatch_stream_failure_frame(
                                    call_id,
                                    session_request_error_from_status(status),
                                );
                                push_session_request_result(
                                    &presence_for_reply,
                                    &caller_ura_for_reply,
                                    &id_hex,
                                    frame,
                                );
                            }
                        }
                        reverse_stream_cancellations_for_task.retire(&call_id);
                    } else {
                        let deferred_self_revoke = self_revoke_target_for_reverse_dispatch(
                            &caller_ura_for_reply,
                            ability.as_str(),
                            &request,
                        );
                        let outcome = dispatcher_for_request
                            .dispatch_canonical_session_invoke(request)
                            .await;
                        let remove_after_result = deferred_self_revoke.is_some() && outcome.is_ok();
                        let frame = build_reverse_dispatch_result_frame(call_id, outcome);
                        let push = push_session_request_result(
                            &presence_for_reply,
                            &caller_ura_for_reply,
                            &id_hex,
                            frame,
                        );
                        if remove_after_result {
                            remove_deferred_self_revoke_presence(
                                &presence_for_reply,
                                &caller_ura_for_reply,
                                &id_hex,
                                push,
                            );
                        }
                    }
                });
                continue;
            }
            None => continue,
        };

        // Binary chunks carry daemon-owned JSON control/input frames only.
        // Canonical Invocation results are protobuf DispatchResult frames and
        // are handled above.
        let dispatch = match SessionDispatch::decode_frame(&chunk.data) {
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
            SessionDispatch::StreamCancel { call_id, .. } => {
                crate::op_event!(
                    component = session_accept,
                    kind = unexpected_upstream_frame,
                    caller = caller_ura,
                    frame_kind = "StreamCancel",
                    call_id = call_id,
                );
            }
            SessionDispatch::ReverseStreamCancel { call_id, reason } => {
                if reverse_stream_cancellations.cancel(&call_id) {
                    crate::op_event!(
                        component = session_accept,
                        kind = canonical_carrier_reverse_stream_cancel_requested,
                        caller = caller_ura,
                        call_id = call_id_hex(&call_id),
                        reason = reason,
                    );
                } else {
                    crate::op_event!(
                        component = session_accept,
                        kind = canonical_carrier_reverse_stream_cancel_no_open,
                        caller = caller_ura,
                        call_id = call_id_hex(&call_id),
                    );
                }
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
                metadata,
            } => {
                // Daemon-owned bootstrap/publication control request.
                // Product invocations arrive through the typed
                // `ReverseDispatchCall` arm above.
                let id_hex = call_id_hex(&call_id);
                crate::op_event!(
                    component = daemon_invocation,
                    kind = session_accept_request_frame,
                    call_id = id_hex,
                    ability_ura = ability_ura,
                );

                let presence_for_reply = Arc::clone(&presence);
                let caller_ura_for_reply = caller_ura.clone();
                let lifecycle = match dispatcher.session_control_lifecycle_from_wire(
                    &caller_ura_for_reply,
                    &ability_ura,
                    &args,
                    &args_content_envelope,
                    metadata,
                ) {
                    Ok(lifecycle) => lifecycle,
                    Err(error) => {
                        let frame = build_session_request_result_frame(
                            call_id,
                            RequestOutcome::Err { error },
                        );
                        push_session_request_result(
                            &presence_for_reply,
                            &caller_ura_for_reply,
                            &id_hex,
                            frame,
                        );
                        continue;
                    }
                };
                match lifecycle.scheduling() {
                    Some(SessionControlScheduling::InlineDrain) => {
                        crate::op_event!(
                            component = session_accept,
                            kind = session_control_inline_dispatch,
                            caller = caller_ura,
                            call_id = id_hex,
                            ability_ura = ability_ura,
                        );
                        let outcome = lifecycle.dispatch(&dispatcher).await;
                        let frame = build_session_request_result_frame(call_id, outcome);
                        push_session_request_result(
                            &presence_for_reply,
                            &caller_ura_for_reply,
                            &id_hex,
                            frame,
                        );
                    }
                    Some(SessionControlScheduling::SpawnTask) => {
                        // Dispatch off the drain task so a slow inner
                        // control does not stall subsequent up-frames the
                        // device sends. Each request gets its own
                        // short-lived task.
                        let dispatcher_for_request = dispatcher.clone();
                        tokio::spawn(async move {
                            let outcome = lifecycle.dispatch(&dispatcher_for_request).await;
                            let frame = build_session_request_result_frame(call_id, outcome);
                            push_session_request_result(
                                &presence_for_reply,
                                &caller_ura_for_reply,
                                &id_hex,
                                frame,
                            );
                        });
                    }
                    None => {
                        let frame = build_session_request_result_frame(
                            call_id,
                            RequestOutcome::Err {
                                error: SessionRequestError::PermissionDenied {
                                    reason: "session control request was not scheduled".to_string(),
                                },
                            },
                        );
                        push_session_request_result(
                            &presence_for_reply,
                            &caller_ura_for_reply,
                            &id_hex,
                            frame,
                        );
                    }
                }
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

    reverse_stream_cancellations.retire_all();

    close_reason
}

fn refresh_session_owner_projection_lease(dispatcher: &BidiDispatcher, caller_ura: &str) -> bool {
    refresh_session_owner_projection_lease_at(
        dispatcher,
        caller_ura,
        crate::daemon::federation::directory::now_unix_ms(),
    )
}

pub(crate) fn refresh_session_owner_projection_lease_at(
    dispatcher: &BidiDispatcher,
    caller_ura: &str,
    now_unix_ms: i64,
) -> bool {
    let owner_ura = caller_ura.trim();
    if owner_ura.is_empty() {
        return false;
    }
    let new_expiry =
        crate::daemon::federation::read_model::owner_projection::lease_expiry_from_now(now_unix_ms);
    dispatcher
        .directory
        .ability_catalog
        .refresh_lease(owner_ura, new_expiry)
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
/// `canonical_invoke` admission already uses (PR-N2 commits
/// `d1adbea` + `68f6556`); we extend it to cover
/// `session.open` admission too. Unblocks the cross-hub
/// same-realm directive that LB-49 surfaced.
pub(crate) fn validate_session_realm(
    caller_ura: &str,
    session_realm: Option<&str>,
    trust_anchor: &RealmTrustAnchor,
) -> Result<(), Status> {
    let Some(daemon_realm) = session_realm else {
        return Ok(());
    };

    let caller_realm = realm_from_ura(caller_ura).ok_or_else(|| {
        Status::invalid_argument(format!(
            "session.open: caller URA `{caller_ura}` does not match the canonical \
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
        "session.open: caller `{caller_ura}` from realm `{caller_realm}` is \
         not in this hub's realm `{daemon_realm}` and not present in the \
         realm trust anchor as a federated identity; cross-realm session \
         requires either same-realm or an explicit `[[trusted_agent]]` entry"
    )))
}

fn session_trust_context(caller_ura: &str, key_id_hint: &str) -> SessionTrustContext {
    let is_user = matches!(
        crate::core::ura::parse_ura(caller_ura).map(|parsed| parsed.kind),
        Ok(crate::core::ura::URAKind::User)
    );
    if !is_user {
        return SessionTrustContext::default();
    }
    let presented = key_id_hint.trim().to_string();
    SessionTrustContext::user_pubkey(presented)
}

/// Build the only supported remote bidi-open carrier: a complete canonical
/// `DispatchCall`. Session contract admission and dispatch lookup already
/// reject v0 peers, so this constructor has no compatibility branch.
#[cfg(test)]
pub(crate) fn build_remote_bidi_open_frame(
    call_id: u64,
    selected_route: &SelectedInvokeRoute,
    envelope_open: &EnvelopeOpen,
    call_mode: CallMode,
) -> Result<DispatchFrame, Status> {
    let request = remote_bidi_forwarded_request(selected_route, envelope_open, call_mode)?;
    Ok(build_canonical_dispatch_frame(
        call_id,
        request,
        CallMode::Bidi,
    ))
}

fn remote_bidi_forwarded_request(
    selected_route: &SelectedInvokeRoute,
    envelope_open: &EnvelopeOpen,
    call_mode: CallMode,
) -> Result<axon_sdk::pb::axon::v1::InvokeRequest, Status> {
    if !matches!(call_mode, CallMode::Bidi) {
        return Err(Status::failed_precondition(format!(
            "InvokeBidi route `{}` resolved with non-bidi call mode {call_mode:?}",
            selected_route.route_ura,
        )));
    }
    let envelope = envelope_open
        .envelope
        .clone()
        .ok_or_else(|| Status::invalid_argument("InvokeBidi request missing envelope"))?;
    require_selected_governance_read_route("InvokeBidi", selected_route, &envelope)?;
    Ok(axon_sdk::pb::axon::v1::InvokeRequest {
        envelope: Some(signed_envelope_for_selected_route(
            envelope,
            selected_route,
            envelope_open.target.as_ref(),
            &envelope_open.initial_args,
        )?),
        target: envelope_open.target.clone(),
        arguments: envelope_open.initial_args.clone(),
        metadata: envelope_open.metadata.clone(),
        ..Default::default()
    })
}

fn bidi_open_to_invoke_request(envelope_open: &EnvelopeOpen) -> Result<InvokeRequest, Status> {
    let envelope = envelope_open
        .envelope
        .clone()
        .ok_or_else(|| Status::invalid_argument("InvokeBidi request missing envelope"))?;
    Ok(InvokeRequest {
        envelope: Some(envelope),
        target: envelope_open.target.clone(),
        arguments: envelope_open.initial_args.clone(),
        content_type: envelope_open.args_content_type.clone(),
        metadata: envelope_open.metadata.clone(),
        content_envelope: envelope_open.content_envelope.clone(),
        ..InvokeRequest::default()
    })
}

fn invoke_request_to_bidi_open(request: InvokeRequest) -> Result<EnvelopeOpen, Status> {
    let envelope = request
        .envelope
        .ok_or_else(|| Status::invalid_argument("reverse InvokeBidi request missing envelope"))?;
    let target = request
        .target
        .ok_or_else(|| Status::invalid_argument("reverse InvokeBidi request missing target"))?;
    Ok(EnvelopeOpen {
        envelope: Some(envelope),
        target: Some(target),
        initial_args: request.arguments,
        args_content_type: request.content_type,
        streams: vec![StreamDescriptor {
            stream_id: 1,
            content_type: "application/json".to_string(),
            ordering: "strict".to_string(),
            ..StreamDescriptor::default()
        }],
        metadata: request.metadata,
        content_envelope: request.content_envelope,
        ..EnvelopeOpen::default()
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
    let data = frame
        .encode_frame()
        .expect("SessionDispatch::BidiInput is statically encodable");
    DispatchFrame::normal(InvokeBidiDown {
        payload: Some(DownPayload::BinaryChunk(BinaryChunk {
            stream_id: crate::daemon::invocation::bidi::session_initiator::SESSION_STREAM_ID,
            data,
            ..BinaryChunk::default()
        })),
        ..InvokeBidiDown::default()
    })
}

fn remote_bidi_input_dispatch_frame_is_eof(frame: &DispatchFrame) -> bool {
    let Some(DownPayload::BinaryChunk(chunk)) = frame.frame.payload.as_ref() else {
        return false;
    };
    let Ok(SessionDispatch::BidiInput { eof, .. }) =
        SessionDispatch::decode_frame(chunk.data.as_slice())
    else {
        return false;
    };
    eof
}

fn build_remote_bidi_input_frame_from_canonical_payload(
    call_id: u64,
    ability: &str,
    core_wire_kind: Option<LocalBidiWireKind>,
    payload: UpPayload,
) -> Option<Result<DispatchFrame, Status>> {
    if let Some(wire_kind) = core_wire_kind {
        return build_remote_bidi_input_frame_from_mapped(
            call_id,
            map_local_bidi_up_payload(wire_kind, payload),
        );
    }

    use axon_sdk::pb::axon::v1::bidi_control::Control as ControlVariant;

    match payload {
        UpPayload::BinaryChunk(chunk) => Some(Ok(build_remote_bidi_input_dispatch_frame(
            call_id,
            &chunk.data,
            false,
        ))),
        UpPayload::Control(control) => {
            match control.control {
                Some(ControlVariant::Eof(true)) => Some(Ok(
                    build_remote_bidi_input_dispatch_frame(call_id, &[], true),
                )),
                Some(_) | None => {
                    crate::op_event!(
                        component = daemon_invocation,
                        kind = remote_bidi_non_core_control_ignored,
                        ability = ability,
                        call_id = call_id,
                    );
                    None
                }
            }
        }
        UpPayload::EnvelopeOpen(_)
        | UpPayload::DispatchResult(_)
        | UpPayload::ReverseDispatchCall(_)
        | UpPayload::ReverseBidiInput(_) => None,
    }
}

fn build_remote_bidi_input_frame_from_mapped(
    call_id: u64,
    mapped: LocalBidiUpFrame,
) -> Option<Result<DispatchFrame, Status>> {
    match mapped {
        LocalBidiUpFrame::Forward(value) => {
            let bytes = match serde_json::to_vec(&value) {
                Ok(bytes) => bytes,
                Err(err) => {
                    return Some(Err(Status::internal(format!(
                        "InvokeBidi remote bidi: encode mapped input frame: {err}"
                    ))));
                }
            };
            Some(Ok(build_remote_bidi_input_dispatch_frame(
                call_id, &bytes, false,
            )))
        }
        LocalBidiUpFrame::ForwardAndClose(value) => {
            let bytes = match serde_json::to_vec(&value) {
                Ok(bytes) => bytes,
                Err(err) => {
                    return Some(Err(Status::internal(format!(
                        "InvokeBidi remote bidi: encode final mapped input frame: {err}"
                    ))));
                }
            };
            Some(Ok(build_remote_bidi_input_dispatch_frame(
                call_id, &bytes, true,
            )))
        }
        LocalBidiUpFrame::Close => Some(Ok(build_remote_bidi_input_dispatch_frame(
            call_id,
            &[],
            true,
        ))),
        LocalBidiUpFrame::Ignore => None,
    }
}

#[cfg(test)]
pub(crate) fn remote_bidi_target_ura(envelope_open: &EnvelopeOpen) -> Option<String> {
    envelope_open
        .envelope
        .as_ref()
        .and_then(|env| env.callee.as_ref())
        .map(|callee| callee.ura.trim())
        .filter(|ura| !ura.is_empty())
        .map(ToOwned::to_owned)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::daemon::invocation::bidi::state::presence::PresenceEvent;
    use axon_sdk::pb::axon::v1::SessionOpenExt;

    #[test]
    fn reverse_stream_cancellation_registry_owns_open_cancel_and_retirement() {
        let registry = ReverseStreamCancellations::default();
        let first_id = [0x11; 16];
        let second_id = [0x22; 16];
        let first = registry.open(first_id);
        let second = registry.open(second_id);

        assert!(registry.cancel(&first_id));
        assert!(first.is_cancelled());
        assert!(!second.is_cancelled());

        registry.retire(&first_id);
        assert!(!registry.cancel(&first_id));

        registry.retire_all();
        assert!(second.is_cancelled());
        assert!(!registry.cancel(&second_id));
    }

    #[test]
    fn absent_ext_fails_closed_without_canonical_carrier() {
        let err = session_contract_from_ext(None)
            .expect_err("missing carrier negotiation must reject session.open");
        assert_eq!(err.code(), tonic::Code::FailedPrecondition);
        assert!(err.message().contains("CANONICAL_CARRIER_REQUIRED"));
    }

    #[test]
    fn v0_ext_fails_closed_without_json_fallback() {
        let ext = SessionOpenExt {
            contract_version: 0,
            claimant_boot_nonce: vec![3; 16],
        };
        let err = session_contract_from_ext(Some(&ext))
            .expect_err("v0 carrier must not register a dispatch session");
        assert_eq!(err.code(), tonic::Code::FailedPrecondition);
        assert!(err.message().contains("carrier v0"));
    }

    #[test]
    fn legacy_implicit_call_mode_contract_is_rejected() {
        let ext = SessionOpenExt {
            contract_version: 1,
            claimant_boot_nonce: vec![3; 16],
        };
        let err = session_contract_from_ext(Some(&ext))
            .expect_err("contract v1 cannot carry explicit transport call_mode");
        assert_eq!(err.code(), tonic::Code::FailedPrecondition);
        assert!(err.message().contains("v3 or newer is required"));
    }

    #[test]
    fn v2_carrier_without_canonical_stream_cancellation_is_rejected() {
        let ext = SessionOpenExt {
            contract_version: 2,
            claimant_boot_nonce: vec![3; 16],
        };
        let err = session_contract_from_ext(Some(&ext))
            .expect_err("v2 carrier cannot own provider-backed stream lifecycle");
        assert_eq!(err.code(), tonic::Code::FailedPrecondition);
        assert!(err.message().contains("carrier v2"));
        assert!(err.message().contains("v3 or newer is required"));
    }

    #[test]
    fn invalid_reverse_dispatch_is_settled_with_typed_terminal_failure() {
        let presence = Arc::new(PresenceRegistry::new());
        let caller = "easynet:///r/test/device/caller";
        let (sender, mut receiver) = tokio::sync::mpsc::channel(1);
        presence
            .insert_negotiated(
                caller.to_string(),
                sender,
                SessionContract::new(CANONICAL_SESSION_CARRIER_VERSION, vec![7; 16]),
            )
            .expect("canonical presence");
        let call_id = [9; 16];

        reject_reverse_dispatch_call(
            &presence,
            caller,
            call_id,
            "canonical ReverseDispatchCall requires a complete InvokeRequest",
        );

        let frame = receiver
            .try_recv()
            .expect("typed rejection must settle the reverse caller")
            .expect("dispatch frame");
        let Some(DownPayload::ReverseDispatchResult(result)) = frame.frame.payload else {
            panic!("expected ReverseDispatchResult");
        };
        assert_eq!(result.call_id, call_id);
        assert!(result.terminal);
        let failure = result.failure.expect("typed failure");
        assert_eq!(failure.code, "INVALID_ARGUMENT");
        assert!(!failure.retryable);
    }

    #[test]
    fn reverse_bidi_result_frame_preserves_carrier_terminal_payload_type() {
        let call_id = [4; 16];
        let terminal = InvokeBidiDown {
            payload: Some(DownPayload::ReverseDispatchResult(
                axon_sdk::pb::axon::v1::ReverseDispatchResult {
                    call_id: vec![0; 16],
                    payload: br#"{"type":"complete"}"#.to_vec(),
                    result_content_type: "application/json".to_string(),
                    terminal: true,
                    admission_receipt: Some(axon_sdk::pb::axon::v1::InvocationReceipt {
                        state: axon_sdk::invocation::InvocationState::Admitted.to_wire_i32(),
                        ..Default::default()
                    }),
                    terminal_receipt: Some(axon_sdk::pb::axon::v1::InvocationReceipt {
                        state: axon_sdk::invocation::InvocationState::Completed.to_wire_i32(),
                        ..Default::default()
                    }),
                    ..Default::default()
                },
            )),
            ..Default::default()
        };

        let frame = build_reverse_dispatch_bidi_result_frame(call_id, terminal)
            .expect("carrier terminal must be forwarded");
        let Some(DownPayload::ReverseDispatchResult(result)) = frame.frame.payload else {
            panic!("expected ReverseDispatchResult");
        };

        assert_eq!(result.call_id, call_id);
        assert_eq!(result.payload, br#"{"type":"complete"}"#);
        assert_eq!(result.result_content_type, "application/json");
        assert!(result.terminal);
        assert!(result.admission_receipt.is_some());
        assert!(result.terminal_receipt.is_some());
    }

    #[test]
    fn reverse_bidi_receipt_frame_projects_signed_terminal_payload_type() {
        let call_id = [5; 16];
        let terminal_payload = br#"{"type":"complete","bytes":3}"#.to_vec();
        let terminal = InvokeBidiDown {
            payload: Some(DownPayload::Receipt(
                axon_sdk::pb::axon::v1::InvocationReceipt {
                    state: axon_sdk::invocation::InvocationState::Completed.to_wire_i32(),
                    payload: terminal_payload.clone(),
                    payload_content_type: "application/json".to_string(),
                    ..Default::default()
                },
            )),
            ..Default::default()
        };

        let frame = build_reverse_dispatch_bidi_result_frame(call_id, terminal)
            .expect("terminal receipt must be projected");
        let Some(DownPayload::ReverseDispatchResult(result)) = frame.frame.payload else {
            panic!("expected ReverseDispatchResult");
        };

        assert_eq!(result.call_id, call_id);
        assert!(result.terminal);
        assert_eq!(result.payload, terminal_payload);
        assert_eq!(result.result_content_type, "application/json");
        assert!(result.terminal_receipt.is_some());
    }

    #[tokio::test]
    async fn terminal_result_backpressure_retires_session_instead_of_dropping_result() {
        let presence = Arc::new(PresenceRegistry::new());
        let mut events = presence.subscribe_events();
        let caller = "easynet:///r/test/device/backpressured-caller";
        let (sender, mut receiver) = tokio::sync::mpsc::channel(1);
        let registration = presence
            .insert_negotiated(
                caller.to_string(),
                sender,
                SessionContract::new(CANONICAL_SESSION_CARRIER_VERSION, vec![7; 16]),
            )
            .expect("canonical presence");
        let online = events.recv().await.expect("online event");
        assert!(matches!(online, PresenceEvent::Online { .. }));

        presence
            .lookup(caller)
            .expect("live sender")
            .try_send(Ok(DispatchFrame::control(InvokeBidiDown::default())))
            .expect("prefill down channel");
        let result = push_session_request_result(
            &presence,
            caller,
            "terminal-call",
            DispatchFrame::control(InvokeBidiDown::default()),
        );

        assert_eq!(
            result,
            SessionDownPush::RetiredBackpressured {
                session_id: registration.session_id,
            }
        );
        assert!(
            presence.lookup(caller).is_none(),
            "backpressured result channel must not remain publicly online"
        );
        let offline = events.recv().await.expect("offline event");
        assert!(matches!(
            offline,
            PresenceEvent::Offline {
                reason: OfflineReason::SendFailed,
                ..
            }
        ));
        receiver
            .recv()
            .await
            .expect("prefilled frame")
            .expect("frame");
        assert!(
            receiver.recv().await.is_none(),
            "retiring the presence sender must close the carrier down channel"
        );
    }

    #[tokio::test]
    async fn heartbeat_ack_is_emitted_only_for_the_live_session_generation() {
        let presence = Arc::new(PresenceRegistry::new());
        let caller = "easynet:///r/test/device/heartbeat-caller";
        let (sender, mut receiver) = tokio::sync::mpsc::channel(1);
        let registration = presence
            .insert_negotiated(
                caller.to_string(),
                sender,
                SessionContract::new(CANONICAL_SESSION_CARRIER_VERSION, vec![8; 16]),
            )
            .expect("canonical presence");

        assert_eq!(
            acknowledge_session_heartbeat(&presence, caller),
            SessionDownPush::Queued {
                session_id: registration.session_id,
            }
        );
        let ack = receiver
            .recv()
            .await
            .expect("heartbeat acknowledgement")
            .expect("dispatch frame");
        assert!(matches!(ack.frame.payload, Some(DownPayload::Control(_))));

        assert!(presence
            .remove_if_session(caller, registration.session_id, OfflineReason::StreamClosed,)
            .is_some());
        assert_eq!(
            acknowledge_session_heartbeat(&presence, caller),
            SessionDownPush::NoPresence,
            "a retired generation cannot receive a synthetic acknowledgement"
        );
    }

    #[test]
    fn remote_bidi_input_reuses_pty_wire_mapper_for_stdin() {
        use base64::Engine as _;

        let mapped = map_local_bidi_up_payload(
            LocalBidiWireKind::Pty,
            UpPayload::BinaryChunk(BinaryChunk {
                data: b"whoami\n".to_vec(),
                ..BinaryChunk::default()
            }),
        );
        let frame = build_remote_bidi_input_frame_from_mapped(42, mapped)
            .expect("mapped pty stdin frame is forwarded")
            .expect("mapped pty stdin frame builds");
        let payload = decode_remote_bidi_input_payload(frame);
        let value: serde_json::Value =
            serde_json::from_slice(&payload).expect("terminal stdin JSON");

        assert_eq!(value["type"], "stdin");
        assert!(
            value.get("mac_base64").is_none(),
            "transport MAC must not enter the terminal.attach business frame"
        );
        let data = value["data"].as_str().expect("stdin data is string");
        let decoded = base64::engine::general_purpose::STANDARD
            .decode(data)
            .expect("stdin data is base64");
        assert_eq!(decoded, b"whoami\n");
    }

    #[test]
    fn remote_bidi_input_reuses_pty_wire_mapper_for_resize_and_eof() {
        let mapped = map_local_bidi_up_payload(
            LocalBidiWireKind::Pty,
            UpPayload::Control(BidiControl {
                control: Some(axon_sdk::pb::axon::v1::bidi_control::Control::PtyResize(
                    axon_sdk::pb::axon::v1::PtyResize {
                        cols: 120,
                        rows: 40,
                    },
                )),
            }),
        );
        let frame = build_remote_bidi_input_frame_from_mapped(43, mapped)
            .expect("mapped pty resize frame is forwarded")
            .expect("mapped pty resize frame builds");
        let payload = decode_remote_bidi_input_payload(frame);
        let value: serde_json::Value =
            serde_json::from_slice(&payload).expect("terminal resize JSON");
        assert_eq!(value["type"], "resize");
        assert_eq!(value["cols"], 120);
        assert_eq!(value["rows"], 40);

        let eof_mapped = map_local_bidi_up_payload(
            LocalBidiWireKind::Pty,
            UpPayload::Control(BidiControl {
                control: Some(axon_sdk::pb::axon::v1::bidi_control::Control::Eof(true)),
            }),
        );
        let eof = build_remote_bidi_input_frame_from_mapped(44, eof_mapped)
            .expect("mapped pty eof frame is forwarded")
            .expect("mapped pty eof frame builds");
        let dispatch = decode_remote_bidi_input(eof);
        match dispatch {
            SessionDispatch::BidiInput { payload, eof, .. } => {
                assert!(eof);
                assert!(payload.is_empty());
            }
            other => panic!("expected remote BidiInput EOF, got {other:?}"),
        }
    }

    #[test]
    fn remote_bidi_input_preserves_reserved_pty_lifecycle_control() {
        let mapped = map_local_bidi_up_payload(
            LocalBidiWireKind::Pty,
            UpPayload::BinaryChunk(BinaryChunk {
                stream_id: crate::daemon::ability::wire::PTY_CONTROL_STREAM_ID,
                data: serde_json::to_vec(&serde_json::json!({"type": "detach"}))
                    .expect("detach JSON"),
                ..BinaryChunk::default()
            }),
        );
        let frame = build_remote_bidi_input_frame_from_mapped(45, mapped)
            .expect("mapped pty control is forwarded")
            .expect("mapped pty control builds");
        let payload = decode_remote_bidi_input_payload(frame);
        let value: serde_json::Value =
            serde_json::from_slice(&payload).expect("terminal control JSON");
        assert_eq!(value, serde_json::json!({"type": "detach"}));
    }

    #[test]
    fn remote_file_transfer_final_frame_preserves_payload_and_eof() {
        let mapped = map_local_bidi_up_payload(
            LocalBidiWireKind::FileTransfer,
            UpPayload::Control(BidiControl {
                control: Some(axon_sdk::pb::axon::v1::bidi_control::Control::Eof(true)),
            }),
        );
        let frame = build_remote_bidi_input_frame_from_mapped(45, mapped)
            .expect("mapped file-transfer EOF is forwarded")
            .expect("mapped file-transfer EOF builds");

        match decode_remote_bidi_input(frame) {
            SessionDispatch::BidiInput { payload, eof, .. } => {
                assert!(eof, "ForwardAndClose must retain carrier EOF");
                let value: serde_json::Value =
                    serde_json::from_slice(&payload).expect("file-transfer EOF JSON");
                assert_eq!(value["type"], "eof");
            }
            other => panic!("expected final remote BidiInput, got {other:?}"),
        }
    }

    #[test]
    fn remote_bidi_input_passes_non_core_plugin_frames_without_hub_wire_registry() {
        let raw = br#"{"kind":"audio","media_kind":"audio","payload":"frame-1"}"#.to_vec();
        let frame = build_remote_bidi_input_frame_from_canonical_payload(
            45,
            "media.synthetic_bidi",
            None,
            UpPayload::BinaryChunk(BinaryChunk {
                data: raw.clone(),
                ..BinaryChunk::default()
            }),
        )
        .expect("non-core plugin frame is forwarded")
        .expect("non-core plugin frame builds");

        assert_eq!(decode_remote_bidi_input_payload(frame), raw);
    }

    #[test]
    fn remote_bidi_input_maps_non_core_plugin_eof_to_canonical_close() {
        let frame = build_remote_bidi_input_frame_from_canonical_payload(
            46,
            "media.synthetic_bidi",
            None,
            UpPayload::Control(BidiControl {
                control: Some(axon_sdk::pb::axon::v1::bidi_control::Control::Eof(true)),
            }),
        )
        .expect("non-core plugin EOF is forwarded")
        .expect("non-core plugin EOF builds");

        assert!(remote_bidi_input_dispatch_frame_is_eof(&frame));
        match decode_remote_bidi_input(frame) {
            SessionDispatch::BidiInput { payload, eof, .. } => {
                assert!(eof);
                assert!(payload.is_empty());
            }
            other => panic!("expected remote BidiInput EOF, got {other:?}"),
        }
    }

    fn decode_remote_bidi_input_payload(frame: DispatchFrame) -> Vec<u8> {
        match decode_remote_bidi_input(frame) {
            SessionDispatch::BidiInput { payload, eof, .. } => {
                assert!(!eof, "test helper expected a data input frame");
                payload
            }
            other => panic!("expected remote BidiInput, got {other:?}"),
        }
    }

    fn decode_remote_bidi_input(frame: DispatchFrame) -> SessionDispatch {
        let Some(DownPayload::BinaryChunk(chunk)) = frame.frame.payload else {
            panic!("expected session BinaryChunk dispatch frame");
        };
        SessionDispatch::decode_frame(&chunk.data).expect("session dispatch frame decodes")
    }

    #[test]
    fn future_ext_negotiates_proto_and_caps_at_hub_version() {
        let ext = SessionOpenExt {
            contract_version: 7, // future device, older hub
            claimant_boot_nonce: vec![3; 16],
        };
        let c =
            session_contract_from_ext(Some(&ext)).expect("canonical carrier version negotiates");
        assert_eq!(
            c.version.min(CANONICAL_SESSION_CARRIER_VERSION),
            CANONICAL_SESSION_CARRIER_VERSION
        );
        assert_eq!(c.claimant_boot_nonce.len(), 16);
    }

    #[test]
    fn canonical_ext_rejects_missing_claimant_fingerprint() {
        let ext = SessionOpenExt {
            contract_version: CANONICAL_SESSION_CARRIER_VERSION,
            claimant_boot_nonce: Vec::new(),
        };
        let err = session_contract_from_ext(Some(&ext))
            .expect_err("canonical carrier must include claimant fingerprint");
        assert_eq!(err.code(), tonic::Code::FailedPrecondition);
        assert!(err.message().contains("claimant_boot_nonce"));
    }

    #[test]
    fn failed_dispatch_result_uses_default_code_when_reason_is_unclassified() {
        let result = failed_dispatch_result("peer stream closed", "TARGET_BUSY", true);
        let failure = result.failure.expect("terminal failure");

        assert_eq!(failure.code, "TARGET_BUSY");
        assert_eq!(failure.message, "peer stream closed");
        assert!(failure.retryable);
    }

    #[test]
    fn failed_dispatch_result_preserves_specific_reason_code() {
        let result = failed_dispatch_result(
            "TARGET_NOT_IN_PRESENCE_REGISTRY: session owner is offline",
            "INVOCATION_FAILED",
            true,
        );
        let failure = result.failure.expect("terminal failure");

        assert_eq!(failure.code, "TARGET_NOT_IN_PRESENCE_REGISTRY");
        assert!(failure.retryable);
    }

    #[test]
    fn deferred_self_revoke_detection_requires_exact_caller_target() {
        let caller = "easynet:///r/realm/device/dev-a";
        let mut request = axon_sdk::pb::axon::v1::InvokeRequest {
            arguments: serde_json::to_vec(
                &serde_json::json!({"agent_ura": "easynet:///r/realm/device/dev-a"}),
            )
            .expect("encode revoke request"),
            ..axon_sdk::pb::axon::v1::InvokeRequest::default()
        };

        assert_eq!(
            self_revoke_target_for_reverse_dispatch(
                caller,
                crate::daemon::invocation::dispatch::federation_wrappers::ABILITY_FEDERATION_REVOKE,
                &request,
            )
            .as_deref(),
            Some(caller)
        );

        request.arguments = serde_json::to_vec(
            &serde_json::json!({"agent_ura": "easynet:///r/realm/device/dev-b"}),
        )
        .expect("encode revoke request");
        assert!(self_revoke_target_for_reverse_dispatch(
            caller,
            crate::daemon::invocation::dispatch::federation_wrappers::ABILITY_FEDERATION_REVOKE,
            &request,
        )
        .is_none());
        assert!(
            self_revoke_target_for_reverse_dispatch(caller, "meta.list_abilities", &request)
                .is_none()
        );
    }

    #[test]
    fn runtime_metadata_decodes_the_canonical_session_contract() {
        let extension = SessionOpenExt {
            contract_version: CANONICAL_SESSION_CARRIER_VERSION,
            claimant_boot_nonce: vec![0x5a; 16],
        };
        let metadata = std::collections::HashMap::from([(
            SESSION_OPEN_EXT_METADATA_KEY.to_string(),
            hex::encode(extension.encode_to_vec()),
        )]);
        let contract = session_contract_from_runtime_metadata(&metadata)
            .expect("provider metadata must decode through the canonical contract validator");
        assert_eq!(contract.version, CANONICAL_SESSION_CARRIER_VERSION);
        assert_eq!(contract.claimant_boot_nonce, vec![0x5a; 16]);
    }

    #[test]
    fn runtime_metadata_rejects_missing_session_contract() {
        let error = session_contract_from_runtime_metadata(&std::collections::HashMap::new())
            .expect_err("provider must fail closed without carrier negotiation");
        assert!(
            error.to_string().contains("CANONICAL_CARRIER_REQUIRED"),
            "{error}"
        );
    }

    #[test]
    fn presence_lease_drop_removes_only_its_registered_generation() {
        let presence = Arc::new(PresenceRegistry::new());
        let caller_ura = "easynet:///r/test/device/provider-lease".to_string();
        let (sender, _receiver) =
            mpsc::channel::<Result<DispatchFrame, Status>>(DISPATCH_CHANNEL_CAPACITY);
        let registration = presence
            .insert_negotiated(
                caller_ura.clone(),
                sender,
                SessionContract {
                    version: CANONICAL_SESSION_CARRIER_VERSION,
                    claimant_boot_nonce: vec![0x11; 16],
                },
            )
            .expect("canonical presence key");
        let lease = SessionPresenceLease::new(
            Arc::clone(&presence),
            caller_ura.clone(),
            registration.session_id,
        );
        let (replacement_sender, _replacement_receiver) =
            mpsc::channel::<Result<DispatchFrame, Status>>(DISPATCH_CHANNEL_CAPACITY);
        let PresenceRegistration {
            session_id: replacement_session_id,
            displaced,
            ..
        } = presence
            .insert_negotiated(
                caller_ura.clone(),
                replacement_sender,
                SessionContract {
                    version: CANONICAL_SESSION_CARRIER_VERSION,
                    claimant_boot_nonce: vec![0x22; 16],
                },
            )
            .expect("canonical presence key");
        drop(displaced);
        assert!(presence.lookup(&caller_ura).is_some());
        drop(lease);
        assert!(
            presence.lookup(&caller_ura).is_some(),
            "dropping a displaced provider lease must not remove its replacement"
        );
        let _ = presence.remove_if_session(
            &caller_ura,
            replacement_session_id,
            OfflineReason::StreamClosed,
        );
    }

    #[test]
    fn canonical_carrier_failure_maps_to_single_track_projection() {
        let pb = axon_sdk::pb::axon::v1::DispatchResult {
            call_id: 7,
            payload: b"partial".to_vec(),
            terminal: true,
            failure: Some(axon_sdk::pb::axon::v1::Error {
                code: "TARGET_OFFLINE".into(),
                message: "device went away".into(),
                retryable: true,
                ..Default::default()
            }),
            ..Default::default()
        };
        let mapped = pending_result_from_canonical_carrier(&pb);
        assert_eq!(mapped.payload, b"partial");
        assert_eq!(mapped.error.as_deref(), Some("device went away"));
        let failure = mapped.failure.expect("typed failure carried");
        assert_eq!(failure.code, "TARGET_OFFLINE");
        assert!(failure.retryable);
        assert!(mapped.request_id.is_none());
    }

    #[test]
    fn canonical_carrier_unary_control_failure_settles_without_terminal_claim() {
        let pb = axon_sdk::pb::axon::v1::DispatchResult {
            call_id: 8,
            terminal: false,
            failure: Some(axon_sdk::pb::axon::v1::Error {
                code: "ABILITY_RESOLUTION_FAILED".into(),
                message: "descriptor missing".into(),
                retryable: false,
                ..Default::default()
            }),
            ..Default::default()
        };
        let (_, failure) = classify_canonical_carrier_result(pb)
            .expect_err("control failure settles the waiter without lifecycle terminality");
        assert_eq!(
            failure.failure.as_ref().map(|value| value.code.as_str()),
            Some("ABILITY_RESOLUTION_FAILED")
        );
        assert!(failure.admission_receipt.is_none());
        assert!(failure.terminal_receipt.is_none());
    }

    #[test]
    fn canonical_carrier_stream_control_failure_settles_without_terminal_claim() {
        let pb = axon_sdk::pb::axon::v1::DispatchResult {
            call_id: 9,
            terminal: false,
            failure: Some(axon_sdk::pb::axon::v1::Error {
                code: "STREAM_OPEN_FAILED".into(),
                message: "target rejected stream open".into(),
                retryable: false,
                ..Default::default()
            }),
            ..Default::default()
        };
        let (_, failure) = classify_canonical_carrier_result(pb)
            .expect_err("stream control failure settles the waiter without terminal receipt");
        assert_eq!(
            failure.failure.as_ref().map(|value| value.code.as_str()),
            Some("STREAM_OPEN_FAILED")
        );
        assert!(failure.admission_receipt.is_none());
        assert!(failure.terminal_receipt.is_none());
    }

    #[test]
    fn canonical_carrier_unary_success_without_checkpoints_is_rejected() {
        let pb = axon_sdk::pb::axon::v1::DispatchResult {
            call_id: 8,
            payload: b"ok".to_vec(),
            terminal: true,
            failure: None,
            ..Default::default()
        };
        let (_, failure) = classify_canonical_carrier_result(pb)
            .expect_err("successful unary result without checkpoints must fail closed");
        assert_eq!(
            failure.failure.as_ref().map(|value| value.code.as_str()),
            Some("CANONICAL_FINALIZATION_REQUIRED")
        );
    }

    #[test]
    fn canonical_carrier_unary_success_requires_both_checkpoints() {
        let pb = axon_sdk::pb::axon::v1::DispatchResult {
            call_id: 8,
            payload: b"ok".to_vec(),
            terminal: true,
            admission_receipt: Some(axon_sdk::pb::axon::v1::InvocationReceipt {
                state: axon_sdk::invocation::InvocationState::Admitted.to_wire_i32(),
                ..Default::default()
            }),
            terminal_receipt: Some(axon_sdk::pb::axon::v1::InvocationReceipt {
                state: axon_sdk::invocation::InvocationState::Completed.to_wire_i32(),
                ..Default::default()
            }),
            ..Default::default()
        };
        let (_, CarrierDispatchEvent::Terminal(mapped)) =
            classify_canonical_carrier_result(pb).expect("paired unary checkpoints are accepted")
        else {
            panic!("unary result must classify as terminal");
        };
        let mapped = *mapped;
        assert!(mapped.error.is_none());
        assert!(mapped.admission_receipt.is_some());
        assert!(mapped.terminal_receipt.is_some());
    }

    #[test]
    fn canonical_carrier_stream_terminal_cannot_repeat_admission_checkpoint() {
        let pb = axon_sdk::pb::axon::v1::DispatchResult {
            call_id: 9,
            terminal: true,
            admission_receipt: Some(axon_sdk::pb::axon::v1::InvocationReceipt {
                state: axon_sdk::invocation::InvocationState::Admitted.to_wire_i32(),
                ..Default::default()
            }),
            terminal_receipt: Some(axon_sdk::pb::axon::v1::InvocationReceipt {
                state: axon_sdk::invocation::InvocationState::Completed.to_wire_i32(),
                ..Default::default()
            }),
            ..Default::default()
        };
        let (_, failure) = classify_canonical_carrier_result(pb)
            .expect_err("stream admission and terminal checkpoints use distinct frames");
        assert_eq!(
            failure.failure.as_ref().map(|value| value.code.as_str()),
            Some("CARRIER_STREAM_PHASE_INVALID")
        );
    }

    #[test]
    fn canonical_carrier_stream_terminal_failure_requires_terminal_checkpoint() {
        let pb = axon_sdk::pb::axon::v1::DispatchResult {
            call_id: 9,
            terminal: true,
            failure: Some(axon_sdk::pb::axon::v1::Error {
                code: "STREAM_OPEN_FAILED".into(),
                message: "target rejected stream open".into(),
                retryable: false,
                ..Default::default()
            }),
            ..Default::default()
        };
        let (_, failure) = classify_canonical_carrier_result(pb)
            .expect_err("failed stream terminal without receipt must fail closed");
        assert_eq!(
            failure.failure.as_ref().map(|value| value.code.as_str()),
            Some("CANONICAL_TERMINAL_RECEIPT_REQUIRED")
        );
        assert!(
            failure.terminal_receipt.is_none(),
            "protocol failure must not synthesize a terminal receipt"
        );
    }

    #[test]
    fn reverse_reply_frame_carries_typed_failure_single_track() {
        let frame =
            build_reverse_dispatch_result_frame([9; 16], Err(SessionRequestError::TargetOffline));
        let Some(DownPayload::ReverseDispatchResult(r)) = frame.frame.payload else {
            panic!("expected ReverseDispatchResult payload");
        };
        assert_eq!(r.call_id, vec![9; 16]);
        assert!(r.terminal);
        let failure = r.failure.expect("typed failure");
        assert_eq!(failure.code, "TARGET_OFFLINE");
        assert!(failure.retryable);
    }

    #[test]
    fn json_session_request_rejects_product_ability_bypass() {
        let device_ability =
            crate::core::ura::owner_ability_ura("easynet:///r/test/device/d1", "shell.run")
                .expect("device ability URA");
        let err = session_control_kind_for_hub(Some("test"), &device_ability)
            .expect_err("device-owned product ability must not route as JSON session control");
        assert!(
            err.contains("does not belong to hub"),
            "unexpected error: {err}"
        );

        let hub_product = crate::core::ura::hub_ability_ura("test", "shell.run");
        let err = session_control_kind_for_hub(Some("test"), &hub_product)
            .expect_err("hub-owned product ability must not route as JSON session control");
        assert!(
            err.contains("not a session-control request"),
            "unexpected error: {err}"
        );
    }

    #[test]
    fn json_session_request_controls_have_explicit_lifecycle_policy() {
        let advertise = session_control_kind_for_hub(
            Some("test"),
            &crate::core::ura::hub_ability_ura("test", ABILITY_FEDERATION_ADVERTISE_AGENT),
        )
        .expect("advertise_agent control");
        let resolve_key = session_control_kind_for_hub(
            Some("test"),
            &crate::core::ura::hub_ability_ura(
                "test",
                federation_wrappers::ABILITY_FEDERATION_RESOLVE_KEY,
            ),
        )
        .expect("resolve_key control");
        let namespace_resolve = session_control_kind_for_hub(
            Some("test"),
            &crate::core::ura::hub_ability_ura("test", ABILITY_NAMESPACE_RESOLVE),
        )
        .expect("namespace.resolve control");

        let request = SessionControlRequest::from_validated_parts(
            advertise,
            "easynet:///r/test/device/d1",
            br#"{"agent":true}"#,
            &SessionContentEnvelope::plaintext_json(),
            HashMap::new(),
        )
        .expect("valid control request");
        let lifecycle = SessionControlLifecycle::validated(request).schedule();
        assert_eq!(
            lifecycle.scheduling(),
            Some(SessionControlScheduling::SpawnTask)
        );

        let request = SessionControlRequest::from_validated_parts(
            resolve_key,
            "easynet:///r/test/device/d1",
            br#"{"key":true}"#,
            &SessionContentEnvelope::plaintext_json(),
            HashMap::new(),
        )
        .expect("valid inline control request");
        let lifecycle = SessionControlLifecycle::validated(request).schedule();
        assert_eq!(
            lifecycle.scheduling(),
            Some(SessionControlScheduling::InlineDrain)
        );

        let request = SessionControlRequest::from_validated_parts(
            namespace_resolve,
            "easynet:///r/test/device/d1",
            br#"{"query_name":"easynet:///r/test/device/d2","qtype":"RESOLVE_TYPE_ROUTE"}"#,
            &SessionContentEnvelope::plaintext_json(),
            HashMap::new(),
        )
        .expect("valid namespace.resolve control request");
        let lifecycle = SessionControlLifecycle::validated(request).schedule();
        assert_eq!(
            lifecycle.scheduling(),
            Some(SessionControlScheduling::InlineDrain)
        );

        let encrypted = SessionContentEnvelope {
            encryption: 1,
            ..SessionContentEnvelope::plaintext_json()
        };
        let err = SessionControlRequest::from_validated_parts(
            resolve_key,
            "easynet:///r/test/device/d1",
            b"{}",
            &encrypted,
            HashMap::new(),
        )
        .expect_err("encrypted JSON control args fail closed");
        assert!(matches!(err, SessionRequestError::PermissionDenied { .. }));
    }

    #[tokio::test]
    async fn session_down_stream_prioritizes_control_frames_over_normal_backlog() {
        let (tx, rx) = mpsc::channel::<Result<DispatchFrame, Status>>(8);
        let mut stream =
            SessionProviderDownStream::new(rx, build_session_established_control(1, 7, false));

        let first = stream.next().await.expect("control frame").expect("ok");
        assert!(
            matches!(first.payload, Some(DownPayload::Control(_))),
            "frame 0 is typed session control"
        );
        assert_eq!(first.sequence, 0);

        tx.send(Ok(DispatchFrame::normal(InvokeBidiDown {
            payload: Some(DownPayload::BinaryChunk(BinaryChunk {
                data: b"normal-a".to_vec(),
                ..BinaryChunk::default()
            })),
            ..InvokeBidiDown::default()
        })))
        .await
        .expect("normal-a queued");
        tx.send(Ok(DispatchFrame::normal(InvokeBidiDown {
            payload: Some(DownPayload::BinaryChunk(BinaryChunk {
                data: b"normal-b".to_vec(),
                ..BinaryChunk::default()
            })),
            ..InvokeBidiDown::default()
        })))
        .await
        .expect("normal-b queued");
        tx.send(Ok(DispatchFrame::control(InvokeBidiDown {
            payload: Some(DownPayload::BinaryChunk(BinaryChunk {
                data: b"control".to_vec(),
                ..BinaryChunk::default()
            })),
            ..InvokeBidiDown::default()
        })))
        .await
        .expect("control queued");

        let prioritized = stream.next().await.expect("prioritized frame").expect("ok");
        assert_eq!(prioritized.sequence, 1);
        let Some(DownPayload::BinaryChunk(chunk)) = prioritized.payload else {
            panic!("expected prioritized BinaryChunk");
        };
        assert_eq!(chunk.data, b"control");

        let normal_a = stream.next().await.expect("normal-a").expect("ok");
        assert_eq!(normal_a.sequence, 2);
        let Some(DownPayload::BinaryChunk(chunk)) = normal_a.payload else {
            panic!("expected normal-a BinaryChunk");
        };
        assert_eq!(chunk.data, b"normal-a");

        let normal_b = stream.next().await.expect("normal-b").expect("ok");
        assert_eq!(normal_b.sequence, 3);
        let Some(DownPayload::BinaryChunk(chunk)) = normal_b.payload else {
            panic!("expected normal-b BinaryChunk");
        };
        assert_eq!(chunk.data, b"normal-b");
    }

    #[test]
    fn session_established_control_carries_typed_contract() {
        let frame = build_session_established_control(1, 42, true);
        let Some(DownPayload::Control(control)) = frame.payload else {
            panic!("frame 0 down must be typed control");
        };
        let Some(bidi_control::Control::SessionEstablished(contract)) = control.control else {
            panic!("expected SessionEstablished control");
        };
        assert_eq!(contract.contract_version, 1);
        assert_eq!(contract.dispatch_encoding, "proto");
        assert_eq!(contract.session_id, 42);
        assert!(contract.displaced_prior);
    }
}
