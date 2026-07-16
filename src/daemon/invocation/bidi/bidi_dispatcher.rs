// EasyNet Daemon — InvokeBidi Dispatcher
// ========================================
//
// File: src/daemon/invocation/bidi_dispatcher.rs
// Description: Owns every `InvokeBidi` routing decision the daemon
//              makes after frame-0 transport policy (commit-plan-2
//              Axis E / E2, final dispatcher):
//
//                * typed `DispatchCall` / `ReverseDispatchCall` relay over a
//                  device's session reverse channel
//                * `session.open` — device session accept loop +
//                  the hub-side session-frame request dispatcher
//                * plugin/builtin bidi wire abilities — local PTY/
//                  file-transfer adapters and the remote bidi bridge
//
//              Also owns the bidi wire furniture: frame-0 validation,
//              terminal/admission receipt builders, the local bidi
//              frame mappers, and the session/local down-stream types.
//
//              Composes `UnaryDispatcher` so bidi and unary entry points use
//              the same admission and route-selection policy.
//
// Author: Silan Hu <silan.hu@u.nus.edu>
// Copyright (c) 2026 EasyNet. All rights reserved.

use std::collections::VecDeque;
use std::pin::Pin;
use std::sync::Arc;
use std::task::{Context, Poll};
use std::time::Duration;

use std::future::Future;

use futures::Stream;
use tokio::sync::mpsc;
use tokio_stream::StreamExt;
use tonic::{Response, Status, Streaming};

use easynet_axon::pb::axon::v1::{
    bidi_control, invoke_bidi_down::Payload as DownPayload, invoke_bidi_up::Payload as UpPayload,
    BidiControl, BidiSessionEstablished, BinaryChunk, EnvelopeOpen, InvokeBidiDown, InvokeBidiUp,
    StreamDescriptor,
};

use crate::daemon::invocation::admission::admission_facade::AdmissionFacade;
use crate::daemon::invocation::admission::hosted_agent_delegation::HostedAgentDelegationIssuer;
use crate::daemon::invocation::admission::register_device_pubkey::parse_realm_from_ura;
use crate::daemon::invocation::admission::target_gate::{
    route_negative_status, route_profile_blocked_status, signed_envelope_for_selected_route,
    TargetGate,
};
use crate::daemon::invocation::bidi::session_initiator::ABILITY_SESSION_OPEN;
use crate::daemon::invocation::bidi::session_wire::{
    build_carrier_v1_dispatch_frame, call_id_hex, require_canonical_dispatch_session,
    RequestOutcome, SessionContentEnvelope, SessionDispatch, SessionRequestError,
};
use crate::daemon::invocation::bidi::state::presence::CANONICAL_SESSION_CARRIER_VERSION;
use crate::daemon::invocation::dispatch::deps::{
    DirectoryPlane, IdentityPlane, RuntimePlane, SessionPlane,
};
use crate::daemon::invocation::dispatch::descriptor_binding::RuntimeBoundAbility;
use crate::daemon::invocation::dispatch::federation_wrappers;
use crate::daemon::invocation::dispatch::federation_wrappers::{
    ABILITY_FEDERATION_ADVERTISE_ABILITIES, ABILITY_FEDERATION_ADVERTISE_AGENT,
};
use crate::daemon::invocation::dispatch::forwarded_finalization::{
    ForwardedFinalizationVerifier, ForwardedInvocationBinding,
};
use crate::daemon::invocation::dispatch::invocation_wire::{
    status_from_axon_invoke_error, target_ura_from_envelope, BoxedDownStream,
    SIGNED_DESCRIPTOR_REF_METADATA_KEY,
};
use crate::daemon::invocation::dispatch::unary_dispatcher::UnaryDispatcher;
use crate::daemon::invocation::routing::route_resolver::{
    CanonicalRouteDispatch, CanonicalRouteSelection, SelectedInvokeRoute,
};
use easynet_axon::invocation::{AbilityFrame, BidiInputFrame, CallMode};

use crate::daemon::invocation::bidi::state::pending_dispatch::{
    DispatchResult, DispatchStreamEvent, PendingDispatchMap, PendingStreamDispatchMap,
};
use crate::daemon::invocation::bidi::state::presence::{
    DispatchFrame, DispatchSender, OfflineReason, PresenceRegistry, SessionContract,
    SessionTrustContext, DISPATCH_CHANNEL_CAPACITY,
};
use crate::daemon::invocation::bidi::state::session_failure::SessionFailure;
use crate::daemon::trust::anchor::RealmTrustAnchor;

/// Named runtime-admin abilities the `InvokeBidi` dispatcher routes by
/// exact name (as opposed to the generic `is_bidi_wire_ability` remote
/// bridge fall-through). This is the single source of truth consumed by
/// `daemon::ability::conformance::RuntimeAdminConformance`; the `match`
/// arms in `dispatch` reference the same constants, so a baseline row can
/// never claim an `AxonRuntimeAdmin` ability the dispatcher does not
/// actually install (SPEC §7.1 notes 6/7, §7.3 item 7, §9.1 item 13).
pub(crate) const RUNTIME_ADMIN_BIDI_ROUTES: &[&str] = &[ABILITY_SESSION_OPEN];

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

    /// Frame-0 routing: the match that used to live in the tonic
    /// `invoke_bidi` method. The trait shell validates + admits frame 0
    /// and delegates here.
    pub(crate) async fn dispatch(
        &self,
        ability_name: &str,
        envelope_open: &EnvelopeOpen,
        up: Streaming<InvokeBidiUp>,
    ) -> Result<Response<BoxedDownStream<InvokeBidiDown>>, Status> {
        match ability_name {
            ABILITY_SESSION_OPEN => {
                let caller_ura = envelope_open
                    .envelope
                    .as_ref()
                    .and_then(|e| e.caller.as_ref())
                    .map(|c| c.ura.clone())
                    .ok_or_else(|| {
                        Status::invalid_argument(
                            "session.open: envelope.caller.ura is required \
                             (already checked by transport policy gate; this is a defensive check)",
                        )
                    })?;
                let contract = session_contract_from_ext(envelope_open.session_ext.as_ref())?;
                self.dispatch_self_session_accept(caller_ura, envelope_open, contract, up)
                    .await
            }
            other => {
                let selection = self.resolve_bidi_route(envelope_open).await?;
                let call_mode = selection.call_mode();
                let selected_route = match selection.into_dispatch() {
                    CanonicalRouteDispatch::Local(route) => route,
                    CanonicalRouteDispatch::Peer(route) => {
                        return Err(Status::unimplemented(format!(
                            "InvokeBidi selected canonical peer route to hub `{}` for `{}`, but \
                             the generic cross-realm bidi carrier is unsupported",
                            route.hub_ura, route.query_name,
                        )));
                    }
                };
                let wire_kind = self
                    .runtime
                    .ability_wire
                    .bidi_wire_kind_for(&selected_route.dispatch_name)
                    .ok_or_else(|| {
                        crate::op_event!(
                            component = daemon_invocation,
                            kind = invoke_bidi_unwired_ability,
                            ability = other,
                            dispatch_ability = selected_route.dispatch_name.as_str(),
                            route_ura = selected_route.route_ura.as_str(),
                        );
                        Status::unimplemented(format!(
                            "InvokeBidi selected route `{}` for ability `{other}`, but dispatch \
                             ability `{}` has no daemon bidi wire adapter",
                            selected_route.route_ura, selected_route.dispatch_name,
                        ))
                    })?;
                let execution_host_is_self = self
                    .gate
                    .matches_self_target_ura(&selected_route.execution_host_ura)
                    .await;
                if selected_route
                    .dispatch_target(execution_host_is_self)
                    .is_local_runtime()
                {
                    self.dispatch_local_bidi_selected_route(
                        envelope_open,
                        up,
                        selected_route,
                        call_mode,
                        wire_kind,
                    )
                    .await
                } else {
                    self.dispatch_remote_bidi(&selected_route, envelope_open, up, call_mode)
                        .await
                }
            }
        }
    }
}

pub(crate) const REASON_BIDI_FIRST_FRAME_SEQUENCE: &str = "AXON_BIDI_FIRST_FRAME_SEQUENCE";
pub(crate) const REASON_BIDI_NON_STRICT_ORDERING: &str = "AXON_BIDI_NON_STRICT_ORDERING";
const REASON_BIDI_FRAME_SEQUENCE: &str = "AXON_BIDI_FRAME_SEQUENCE";
/// Application-level heartbeat cadence for `session.open` down
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
    fallback_code: &str,
    retryable: bool,
) -> DispatchResult {
    let reason = reason.into();
    DispatchResult {
        payload: Vec::new(),
        failure: Some(SessionFailure::from_reason(
            &reason,
            fallback_code,
            retryable,
        )),
        error: Some(reason),
        request_id: None,
        admission_receipt: None,
        terminal_receipt: None,
    }
}

impl BidiDispatcher {
    pub(crate) async fn dispatch_remote_bidi(
        &self,
        selected_route: &SelectedInvokeRoute,
        envelope_open: &EnvelopeOpen,
        mut up: Streaming<InvokeBidiUp>,
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
        let forwarded_binding = ForwardedInvocationBinding::from_request(&forwarded_request)?;
        let receipt_resolver = self.admission.receipt_key_resolver();
        let open_frame = build_carrier_v1_dispatch_frame(call_id, forwarded_request, true);
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
                        if let Err(status) = finalization.admit(receipt.clone()) {
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
                        if let Err(status) = finalization.observe_data() {
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
                        let DispatchResult {
                            payload: _,
                            error,
                            failure,
                            request_id: _,
                            admission_receipt,
                            terminal_receipt,
                            ..
                        } = *result;
                        let frame = match terminal_receipt {
                            Some(terminal_receipt) => finalization
                                .finalize(admission_receipt, terminal_receipt)
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
                        let _ = down_tx_for_results.send(frame).await;
                        break;
                    }
                }
            }
        });

        let execution_host_ura_owned = selected_route.execution_host_ura.clone();
        let ability_owned = selected_route.dispatch_name.clone();
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
                            == crate::daemon::ability::builtins::device_control::terminal::attach::ABILITY_PTY_SESSION_ATTACH =>
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
                    // Direction discipline: dispatch results flow
                    // device→hub on the device's own session, never on
                    // the caller's up stream — a carrier-v1 frame here
                    // is a peer bug, not a negotiation gap.
                    UpPayload::DispatchResult(_) | UpPayload::ReverseDispatchCall(_) => continue,
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

        let selection = self
            .gate
            .route_resolver()
            .await
            .resolve_canonical_route(&target_ura, ability, CallMode::Bidi)
            .map_err(route_negative_status)?;
        if let CanonicalRouteDispatch::Local(selected_route) = selection.dispatch() {
            if !selected_route.is_authoritative_local_or_better() {
                return Err(route_profile_blocked_status(selected_route));
            }
        }
        Ok(selection)
    }

    pub(crate) async fn dispatch_local_bidi_selected_route(
        &self,
        envelope_open: &EnvelopeOpen,
        mut up: Streaming<InvokeBidiUp>,
        selected_route: SelectedInvokeRoute,
        call_mode: CallMode,
        wire_kind: LocalBidiWireKind,
    ) -> Result<Response<BoxedDownStream<InvokeBidiDown>>, Status> {
        let dispatch_ability = selected_route.ability_ura.clone();
        crate::op_event!(
            component = daemon_invocation,
            kind = invoke_bidi_local_runtime_dispatch,
            ability = selected_route.dispatch_name.as_str(),
            dispatch_ability = dispatch_ability.as_str(),
            callee_ura = selected_route.callee_ura.as_str(),
            execution_host_ura = selected_route.execution_host_ura.as_str(),
            route_ura = selected_route.route_ura.as_str(),
        );

        let Some(runtime) = self.runtime.local_runtime.as_ref() else {
            return Err(Status::failed_precondition(format!(
                "InvokeBidi: ability `{}` cannot run because Axon LocalRuntime \
                 is not wired at boot",
                selected_route.dispatch_name
            )));
        };
        let bound_ability = RuntimeBoundAbility::from_selected_route(
            "InvokeBidi",
            runtime,
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
        let signed_ability = envelope_open
            .target
            .as_ref()
            .map(|target| target.ability_name.as_str())
            .unwrap_or_default();
        bound_ability.require_wire_target_matches(
            "InvokeBidi",
            &selected_route.callee_ura,
            signed_ability,
            &selected_route.route_ura,
        )?;
        let wire_envelope = envelope_open
            .envelope
            .clone()
            .ok_or_else(|| Status::invalid_argument("InvokeBidi request missing envelope"))?;
        let local_self_admitted = self
            .admission
            .accepts_local_self_envelope(envelope_open.envelope.as_ref());
        let wire = if local_self_admitted {
            let metadata = HostedAgentDelegationIssuer::materialize_request_metadata(
                &envelope_open.metadata,
                &wire_envelope,
                true,
                &dispatch_ability,
            )?;
            crate::daemon::axon_bridge::dispatch_shim::local_system_from_wire_parts(
                wire_envelope,
                dispatch_descriptor_ref,
                envelope_open.initial_args.clone(),
                metadata,
            )
        } else {
            let metadata = HostedAgentDelegationIssuer::materialize_request_metadata(
                &envelope_open.metadata,
                &wire_envelope,
                false,
                &dispatch_ability,
            )?;
            crate::daemon::axon_bridge::dispatch_shim::external_signed_from_wire_parts(
                wire_envelope,
                dispatch_descriptor_ref,
                envelope_open.initial_args.clone(),
                metadata,
            )
        }
        .map_err(|err| status_from_axon_invoke_error("InvokeBidi", &dispatch_ability, *err))?;
        let handle =
            crate::daemon::axon_bridge::dispatch_shim::open_bidi_external_signed(runtime, wire)
                .await
                .map_err(|err| {
                    status_from_axon_invoke_error("InvokeBidi", &dispatch_ability, err)
                })?;
        let admission_receipt = handle.admission_receipt().await.map_err(|err| {
            Status::failed_precondition(format!(
                "CANONICAL_ADMISSION_REQUIRED: InvokeBidi `{dispatch_ability}`: {err}"
            ))
        })?;
        let admission_frame = InvokeBidiDown {
            payload: Some(DownPayload::Receipt(
                easynet_axon::invocation::wire::receipt_to_wire(&admission_receipt).map_err(
                    |error| {
                        Status::failed_precondition(format!(
                            "CANONICAL_ADMISSION_PROJECTION_FAILED: {error}"
                        ))
                    },
                )?,
            )),
            ..InvokeBidiDown::default()
        };
        let (handler_in_tx, mut handler_out_rx) = handle.split();
        let stdout_stream_id = local_bidi_stdout_stream_id(envelope_open);

        // Down-stream: handler-emitted JSON → InvokeBidiDown frames.
        // Capacity 16 bounds per-session dispatch backpressure.
        let (down_tx, down_rx) = tokio::sync::mpsc::channel::<Result<InvokeBidiDown, Status>>(16);

        let down_tx_for_handler = down_tx.clone();
        tokio::spawn(async move {
            while let Some(frame_result) = handler_out_rx.next_frame().await {
                let frame = match frame_result {
                    Ok(frame) => frame,
                    Err(err) => {
                        let projected = project_finalized_bidi_receipt(&handler_out_rx)
                            .await
                            .map_err(|status| {
                                Status::internal(format!(
                                    "InvokeBidi local-runtime frame failed: {err}; {status}"
                                ))
                            });
                        let _ = down_tx_for_handler.send(projected).await;
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
                    LocalBidiHandlerFrame::Terminal => {
                        let projected = project_finalized_bidi_receipt(&handler_out_rx).await;
                        let _ = down_tx_for_handler.send(projected).await;
                        break;
                    }
                    LocalBidiHandlerFrame::Ignore => {}
                    LocalBidiHandlerFrame::ProtocolFailure(reason) => {
                        let _ = handler_out_rx.cancel(reason.clone()).await;
                        let projected = project_finalized_bidi_receipt(&handler_out_rx)
                            .await
                            .map_err(|status| Status::internal(format!("{reason}; {status}")));
                        let _ = down_tx_for_handler.send(projected).await;
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

        let stream = LocalBidiDownStream::with_admission(down_rx, admission_frame);
        Ok(Response::new(
            Box::pin(stream) as BoxedDownStream<InvokeBidiDown>
        ))
    }

    pub(crate) async fn dispatch_self_session_accept(
        &self,
        caller_ura: String,
        envelope_open: &EnvelopeOpen,
        contract: SessionContract,
        up: Streaming<InvokeBidiUp>,
    ) -> Result<Response<BoxedDownStream<InvokeBidiDown>>, Status> {
        validate_session_realm(
            &caller_ura,
            self.identity.session_realm.as_deref(),
            &self.admission.trust_anchor_snapshot(),
        )?;

        let (down_tx, down_rx): (DispatchSender, _) =
            mpsc::channel::<Result<DispatchFrame, Status>>(DISPATCH_CHANNEL_CAPACITY);

        // Register before spawning so a canonical `DispatchCall` arriving
        // immediately can find this
        // sender. The PresenceRegistry handles displacement (Offline +
        // Online emission ordering) under the hood; the slot remembers
        // the frame-0 carrier negotiation (DEC-F004).
        let negotiated_version = contract.version.min(CANONICAL_SESSION_CARRIER_VERSION);
        let claimant_nonce = contract.claimant_boot_nonce.clone();
        let trust_context = session_trust_context_from_open(caller_ura.as_str(), envelope_open);
        let registration = self.directory.presence.insert_negotiated_with_trust(
            caller_ura.clone(),
            down_tx,
            contract,
            trust_context,
        );
        let displaced_prior = registration.displaced.is_some();
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
        if let Some(prior_nonce) = registration
            .displaced_claimant_nonce
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

        // Spawn the upstream consumer. It routes typed dispatch results and
        // remaining streaming/control replies to their correlation tables.
        let presence_for_drain = Arc::clone(&self.directory.presence);
        let pending_for_drain = self.sessions.pending.clone();
        let pending_stream_for_drain = self.sessions.pending_stream.clone();
        let caller_ura_for_drain = caller_ura.clone();
        // The drain task needs a service handle so reverse calls use the same
        // dispatch policy as unary `Invoke`. `DaemonInvocationService`
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

        // Hand the down stream to tonic. Frames arrive in `down_tx` as
        // `DispatchFrame`
        // (presence_registry's newtype around `InvokeBidiDown`).
        // The tonic trait wants raw `InvokeBidiDown`, so map each
        // frame to unwrap the newtype.
        let stream = SessionDownStream::new(
            down_rx,
            build_session_established_control(
                negotiated_version,
                registration.session_id,
                displaced_prior,
            ),
        );
        Ok(Response::new(
            Box::pin(stream) as BoxedDownStream<InvokeBidiDown>
        ))
    }
}

/// Build a no-op down-stream control frame suitable for session
/// liveness probing. Current readers treat `Control` frames as
/// non-business metadata and ignore them, so this is wire-compatible
/// with every existing `session.open` consumer.
fn build_session_down_keepalive_frame() -> DispatchFrame {
    DispatchFrame::control(InvokeBidiDown {
        payload: Some(DownPayload::Control(BidiControl::default())),
        ..InvokeBidiDown::default()
    })
}

/// Admit frame-0 carrier negotiation only when the peer can preserve a
/// complete canonical Invocation. Contract v0 and absent negotiation are
/// retired; accepting either would create a live session that can only carry
/// the removed JSON projection.
pub(crate) fn session_contract_from_ext(
    ext: Option<&easynet_axon::pb::axon::v1::SessionOpenExt>,
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
    Ok(SessionContract {
        version: ext.contract_version,
        claimant_boot_nonce: ext.claimant_boot_nonce.clone(),
    })
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

async fn project_finalized_bidi_receipt(
    receiver: &easynet_axon::invocation::BidiOutputReceiver,
) -> Result<InvokeBidiDown, Status> {
    let finalized = receiver.finalized().await.map_err(|err| {
        Status::failed_precondition(format!("CANONICAL_FINALIZATION_REQUIRED: {err}"))
    })?;
    Ok(InvokeBidiDown {
        payload: Some(DownPayload::Receipt(
            easynet_axon::invocation::wire::receipt_to_wire(&finalized.terminal_receipt).map_err(
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
    Forward(InvokeBidiDown),
    Terminal,
    Ignore,
    ProtocolFailure(String),
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
        // Carrier-v1 frames (DEC-F004): not local-bidi wire traffic.
        (_, UpPayload::DispatchResult(_)) | (_, UpPayload::ReverseDispatchCall(_)) => {
            LocalBidiUpFrame::Ignore
        }
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

fn forward_json_bidi_frame(
    value: &serde_json::Value,
    stdout_stream_id: u32,
) -> LocalBidiHandlerFrame {
    match serde_json::to_vec(value) {
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
                LocalBidiHandlerFrame::Forward(InvokeBidiDown {
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
                LocalBidiHandlerFrame::Forward(InvokeBidiDown {
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

/// Down-stream wrapper that:
///   1. Emits a typed session-established control as down frame 0.
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
    pending_normal_frames: VecDeque<Result<DispatchFrame, Status>>,
    next_heartbeat: Pin<Box<tokio::time::Sleep>>,
    next_sequence: u64,
    /// Set to `Some(control)` at construction; first `poll_next`
    /// yields it and clears the slot. Subsequent polls follow the
    /// recv-then-heartbeat path.
    pending_initial_control: Option<InvokeBidiDown>,
}

pub(crate) struct LocalBidiDownStream {
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

impl SessionDownStream {
    fn new(
        down_rx: tokio::sync::mpsc::Receiver<Result<DispatchFrame, Status>>,
        initial_control: InvokeBidiDown,
    ) -> Self {
        Self {
            down_rx,
            pending_normal_frames: VecDeque::new(),
            next_heartbeat: Box::pin(tokio::time::sleep(SESSION_DOWN_HEARTBEAT_INTERVAL)),
            next_sequence: 0,
            pending_initial_control: Some(initial_control),
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

impl Stream for SessionDownStream {
    type Item = Result<InvokeBidiDown, Status>;

    fn poll_next(mut self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<Option<Self::Item>> {
        if let Some(control) = self.pending_initial_control.take() {
            self.reset_heartbeat();
            return Poll::Ready(Some(Ok(self.stamp_sequence(control))));
        }

        match self.poll_dispatch_frame(cx) {
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

impl BidiDispatcher {
    /// Admit and route an opaque canonical invocation received through a
    /// device-owned reverse session. The session proves transport liveness;
    /// caller authority remains exclusively in the request envelope and is
    /// revalidated here before any route or execution is attempted.
    async fn dispatch_canonical_session_invoke(
        &self,
        request: easynet_axon::pb::axon::v1::InvokeRequest,
    ) -> Result<easynet_axon::pb::axon::v1::InvokeResponse, SessionRequestError> {
        if let Err(status) = self.admission.verify_invoke(&request) {
            return Err(session_request_error_from_status(status));
        }
        let (result, _) = self.unary.dispatch_local_rpc_selected_route(&request).await;
        match result {
            Ok(response) => Ok(response.into_inner()),
            Err(status) => Err(session_request_error_from_status(status)),
        }
    }

    /// Handle daemon-owned control requests arriving on a device's
    /// `session.open` stream. Product invocations use the typed canonical
    /// request arm above. The allowed control set is the hosted-agent
    /// self-advertise pair —
    /// `federation.advertise_agent` (identity, PR-N6 v1) and
    /// `federation.advertise_abilities` (hot-add ability projection,
    /// ISS-002 closure). Other ability names return
    /// `PermissionDenied` so the device-side caller surfaces a
    /// structured error instead of a silent timeout; widening
    /// further awaits a per-ability admission policy.
    ///
    /// Trust boundary (PR-N6 spec §"What this spec does NOT cover"):
    /// the bidi was established with a signed Bootstrap frame, so
    /// the hub trusts the originating device on every Request frame
    /// — no per-Request signature verify happens here.
    pub(crate) async fn dispatch_session_request_from_session(
        &self,
        caller_device_ura: &str,
        ability_ura: &str,
        args: &[u8],
    ) -> RequestOutcome {
        self.dispatch_session_request_for_caller(Some(caller_device_ura), ability_ura, args)
            .await
    }

    async fn dispatch_session_request_for_caller(
        &self,
        caller_device_ura: Option<&str>,
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
        self.dispatch_session_request_named_for_caller(caller_device_ura, &ability, args)
            .await
    }

    fn is_inline_session_request(&self, ability_ura: &str) -> bool {
        matches!(
            self.session_request_public_ability_for_hub(ability_ura)
                .as_deref(),
            Ok(federation_wrappers::ABILITY_FEDERATION_RESOLVE_KEY)
        )
    }

    async fn dispatch_checked_session_request(
        &self,
        caller_device_ura: &str,
        ability_ura: &str,
        args: &[u8],
        args_content_envelope: &SessionContentEnvelope,
    ) -> RequestOutcome {
        if args_content_envelope.is_encrypted() {
            RequestOutcome::Err {
                error: SessionRequestError::PermissionDenied {
                    reason: format!(
                        "session.open: Request ability_ura `{ability_ura}` received encrypted args \
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
                        "session.open: Request ability_ura `{ability_ura}` received unsupported \
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
                        "session.open: Request ability_ura `{ability_ura}` received unsupported \
                         args encoding {:?}",
                        args_content_envelope.encoding
                    ),
                },
            }
        } else {
            self.dispatch_session_request_from_session(caller_device_ura, ability_ura, args)
                .await
        }
    }

    async fn dispatch_session_request_named_for_caller(
        &self,
        caller_device_ura: Option<&str>,
        ability: &str,
        args: &[u8],
    ) -> RequestOutcome {
        match ability {
            ABILITY_FEDERATION_ADVERTISE_AGENT => {
                let result = match caller_device_ura {
                    Some(caller_device_ura) => self
                        .unary
                        .dispatch_federation_advertise_agent_from_session(args, caller_device_ura),
                    None => Err(Status::failed_precondition(
                        "federation.advertise_agent: admitted session caller is required",
                    )),
                };
                match result {
                    Ok(result_bytes) => RequestOutcome::Ok { result_bytes },
                    Err(status) => map_status_to_session_request_error(status),
                }
            }
            // Hot-add ability projection: `easynet agent add` while the
            // session is live pushes the new agent's ability payload
            // through this Request frame (agent_lifecycle ISS-002).
            // Without this arm the identity advertise above lands but
            // the hub directory shows the agent with ZERO abilities
            // until a stop/start republish. Routes to the same handler
            // the unary Invoke path uses.
            ABILITY_FEDERATION_ADVERTISE_ABILITIES => {
                let result = match caller_device_ura {
                    Some(caller_device_ura) => self
                        .unary
                        .dispatch_federation_advertise_abilities_from_session(
                            args,
                            caller_device_ura,
                        ),
                    None => Err(Status::failed_precondition(
                        "federation.advertise_abilities: admitted session caller is required",
                    )),
                };
                match result {
                    Ok(result_bytes) => RequestOutcome::Ok { result_bytes },
                    Err(status) => map_status_to_session_request_error(status),
                }
            }
            // Device-pulled trust sync (paired-user keys at session
            // attach, peer-device keys on origin-claim miss): the hub
            // is the realm's key registrar, and the session bidi is
            // the device's authenticated channel to ask it. Routes to
            // the same handler the unary Invoke path uses.
            federation_wrappers::ABILITY_FEDERATION_RESOLVE_KEY => {
                match self.unary.dispatch_federation_resolve_key(args) {
                    Ok(result_bytes) => RequestOutcome::Ok { result_bytes },
                    Err(status) => map_status_to_session_request_error(status),
                }
            }
            other => RequestOutcome::Err {
                error: SessionRequestError::PermissionDenied {
                    reason: format!(
                        "session_request: ability `{other}` is not yet routed; \
                         only `{ABILITY_FEDERATION_ADVERTISE_AGENT}`, \
                         `{ABILITY_FEDERATION_ADVERTISE_ABILITIES}`, and \
                         `{}` are wired",
                        federation_wrappers::ABILITY_FEDERATION_RESOLVE_KEY
                    ),
                },
            },
        }
    }

    fn session_request_public_ability_for_hub(&self, ability_ura: &str) -> Result<String, String> {
        let realm = self
            .identity
            .session_realm
            .as_deref()
            .filter(|realm| !realm.trim().is_empty())
            .ok_or_else(|| {
                "session_request: hub session_realm is not wired; cannot validate request \
                 ability_ura"
                    .to_string()
            })?;
        let hub_ura = crate::core::ura::hub_ura(realm);
        crate::core::ura::public_ability_name_from_ability_ura(&hub_ura, ability_ura).ok_or_else(
            || {
                format!(
                "session_request: ability_ura `{ability_ura}` does not belong to hub `{hub_ura}`"
            )
            },
        )
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
    use easynet_axon::pb::axon::v1::invoke_bidi_down::Payload;
    use easynet_axon::pb::axon::v1::{BinaryChunk, InvokeBidiDown};

    let frame = SessionDispatch::RequestResult { call_id, outcome };
    let data = match frame.encode_frame() {
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
    crate::daemon::invocation::bidi::state::presence::DispatchFrame::control(InvokeBidiDown {
        payload: Some(Payload::BinaryChunk(BinaryChunk {
            data,
            ..BinaryChunk::default()
        })),
        ..InvokeBidiDown::default()
    })
}

/// Push a `RequestResult` frame back down the device's bidi via
/// the same PresenceRegistry-keyed `DispatchSender` the device's
/// session-accept handler registered. The device drains the down
/// stream in `session_initiator::dial_and_run_session` and routes
/// `RequestResult` frames to the `oneshot::Receiver` matching
/// `call_id` (per PR-N6 spec §"Concurrent multiplexing"). Lookup
/// failure means the device disconnected between issuing the
/// Request and the hub finishing dispatch — log + drop, which is
/// the same shape PR-N1's `try_push_canonical_invoke_frame` uses for
/// the symmetric race.
pub(crate) fn push_session_request_result(
    presence: &Arc<PresenceRegistry>,
    caller_ura: &str,
    id_hex: &str,
    frame: crate::daemon::invocation::bidi::state::presence::DispatchFrame,
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
            // Full = device is slow, not dead: drop this one frame
            // (the device-side waiter times out and retries) instead
            // of evicting the whole session.
            crate::op_event!(
                component = session_accept,
                kind = request_result_push_failed,
                caller = caller_ura,
                call_id = id_hex,
                reason = "channel_full_dropped",
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

/// Map a carrier-v1 typed failure into the session-plane projection
/// the pending maps carry. `error` keeps the human-readable string the
/// JSON-era consumers expect; `failure` keeps the typed class.
pub(crate) fn pending_result_from_carrier_v1(
    result: &easynet_axon::pb::axon::v1::DispatchResult,
) -> DispatchResult {
    DispatchResult {
        payload: result.payload.clone(),
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
    Admission(easynet_axon::pb::axon::v1::InvocationReceipt),
    Chunk(Vec<u8>),
    Terminal(DispatchResult),
}

fn classify_carrier_v1_result(
    result: easynet_axon::pb::axon::v1::DispatchResult,
) -> Result<(u64, CarrierDispatchEvent), (u64, DispatchResult)> {
    let call_id = result.call_id;
    let protocol_failure =
        |reason: &str, code: &str| (call_id, failed_dispatch_result(reason, code, false));

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
            CarrierDispatchEvent::Terminal(pending_result_from_carrier_v1(&result)),
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
            CarrierDispatchEvent::Terminal(pending_result_from_carrier_v1(&result)),
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
        if admission.state != easynet_axon::invocation::InvocationState::Admitted.to_wire_i32() {
            return Err(protocol_failure(
                "stream admission DispatchResult carried a non-admission checkpoint",
                "CANONICAL_ADMISSION_INVALID",
            ));
        }
        return Ok((call_id, CarrierDispatchEvent::Admission(admission)));
    }
    if result.failure.is_some() {
        return Err((call_id, pending_result_from_carrier_v1(&result)));
    }
    Ok((call_id, CarrierDispatchEvent::Chunk(result.payload)))
}

pub(crate) fn session_failure_from_axon_error(
    err: &easynet_axon::pb::axon::v1::Error,
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

/// Hub → device reply for a carrier-v1 reverse request. Failures ride
/// the single-track typed Error (DEC-F004 point 3).
pub(crate) fn build_reverse_dispatch_result_frame(
    call_id: [u8; 16],
    outcome: Result<easynet_axon::pb::axon::v1::InvokeResponse, SessionRequestError>,
) -> DispatchFrame {
    use easynet_axon::pb::axon::v1::ReverseDispatchResult;
    let (payload, failure, admission_receipt, terminal_receipt) = match outcome {
        Ok(response) => {
            if response.admission_receipt.is_none() || response.terminal_receipt.is_none() {
                (
                    Vec::new(),
                    Some(easynet_axon::pb::axon::v1::Error {
                        code: "CANONICAL_FINALIZATION_REQUIRED".to_string(),
                        message: "hub canonical reverse dispatch completed without both signed finalization checkpoints".to_string(),
                        retryable: false,
                        ..easynet_axon::pb::axon::v1::Error::default()
                    }),
                    None,
                    None,
                )
            } else {
                (
                    response.result,
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
                Some(easynet_axon::pb::axon::v1::Error {
                    code: code.to_string(),
                    message,
                    retryable,
                    ..easynet_axon::pb::axon::v1::Error::default()
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
            terminal: true,
            failure,
            admission_receipt,
            terminal_receipt,
        })),
        ..InvokeBidiDown::default()
    })
}

/// Terminal-result settlement shared by the JSON `Result` arm and the
/// carrier-v1 `DispatchResult` arm: streaming map first, unary map as
/// fallback, every miss surfaced (DEC-F004 — one settle path, not two).
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

/// Drain a device's `session.open` up-stream. Each up-frame is
/// may carry typed `DispatchResult` / `ReverseDispatchCall` frames or the
/// remaining JSON streaming/control frames. Matching pending entries are
/// settled by call_id.
///
/// On stream close (any reason — graceful CloseSend, transport
/// reset, RST_STREAM, peer crash) the device is removed from the
/// presence registry with an appropriate `OfflineReason` so that
/// future `lookup` calls see it as offline immediately.
async fn drain_session_up_stream(
    mut up: Streaming<InvokeBidiUp>,
    caller_ura: String,
    session_id: crate::daemon::invocation::bidi::state::presence::PresenceSessionId,
    presence: Arc<PresenceRegistry>,
    pending: Option<Arc<PendingDispatchMap>>,
    pending_stream: Option<Arc<PendingStreamDispatchMap>>,
    dispatcher: BidiDispatcher,
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
                let refreshed = refresh_session_owner_projection_lease(&dispatcher, &caller_ura);
                if refreshed {
                    crate::op_event!(
                        component = session_accept,
                        kind = up_heartbeat_projection_lease_refreshed,
                        caller = caller_ura,
                    );
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
            // Carrier-v1 has one checkpoint geometry per call mode. The
            // even/odd pending-call namespaces make mode classification exact.
            Some(UpPayload::DispatchResult(result)) => {
                match classify_carrier_v1_result(result) {
                    Ok((call_id, CarrierDispatchEvent::Admission(receipt))) => {
                        crate::op_event!(
                            component = session_accept,
                            kind = carrier_v1_admission_receipt_received,
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
                    Ok((call_id, CarrierDispatchEvent::Terminal(mapped)))
                    | Err((call_id, mapped)) => {
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
            Some(UpPayload::ReverseDispatchCall(call)) => {
                let Ok(call_id) = <[u8; 16]>::try_from(call.call_id.as_slice()) else {
                    crate::op_event!(
                        component = session_accept,
                        kind = carrier_v1_reverse_call_bad_id,
                        caller = caller_ura,
                        id_len = call.call_id.len(),
                    );
                    continue;
                };
                let Some(request) = call.request else {
                    crate::op_event!(
                        component = session_accept,
                        kind = carrier_v1_reverse_call_missing_request,
                        caller = caller_ura,
                        call_id = call_id_hex(&call_id),
                    );
                    continue;
                };
                let id_hex = call_id_hex(&call_id);
                crate::op_event!(
                    component = daemon_invocation,
                    kind = session_accept_request_frame,
                    call_id = id_hex,
                    ability = request.function_name,
                );
                // Same off-drain dispatch discipline as the JSON
                // Request arm: a slow inner call must not stall
                // subsequent up-frames.
                let dispatcher_for_request = dispatcher.clone();
                let presence_for_reply = Arc::clone(&presence);
                let caller_ura_for_reply = caller_ura.clone();
                tokio::spawn(async move {
                    let outcome = dispatcher_for_request
                        .dispatch_canonical_session_invoke(request)
                        .await;
                    let frame = build_reverse_dispatch_result_frame(call_id, outcome);
                    push_session_request_result(
                        &presence_for_reply,
                        &caller_ura_for_reply,
                        &id_hex,
                        frame,
                    );
                });
                continue;
            }
            None => continue,
        };

        // Parse SessionDispatch::Result. A malformed frame is logged
        // but does not tear down the session — the device may send
        // future frames that are well-formed.
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
                        admission_receipt: None,
                        terminal_receipt: None,
                    };
                    settle_terminal_result(
                        &pending,
                        &pending_stream,
                        &caller_ura,
                        call_id,
                        dispatch_result,
                    )
                    .await;
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
                    report_chunk_delivery(
                        pending_stream.deliver_chunk(call_id, payload).await,
                        &caller_ura,
                        call_id,
                    );
                }
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

                let inline_control_request = dispatcher.is_inline_session_request(&ability_ura);
                let dispatcher_for_request = dispatcher.clone();
                let presence_for_reply = Arc::clone(&presence);
                let caller_ura_for_reply = caller_ura.clone();
                if inline_control_request {
                    crate::op_event!(
                        component = session_accept,
                        kind = session_request_inline_control_dispatch,
                        caller = caller_ura,
                        call_id = id_hex,
                        ability_ura = ability_ura,
                    );
                    let outcome = dispatcher_for_request
                        .dispatch_checked_session_request(
                            &caller_ura_for_reply,
                            &ability_ura,
                            &args,
                            &args_content_envelope,
                        )
                        .await;
                    let frame = build_session_request_result_frame(call_id, outcome);
                    push_session_request_result(
                        &presence_for_reply,
                        &caller_ura_for_reply,
                        &id_hex,
                        frame,
                    );
                } else {
                    // Dispatch off the drain task so a slow inner
                    // call (peer delegation round-trip, peer-side
                    // ability handler latency) does not stall
                    // subsequent up-frames the device sends. Each
                    // Request gets its own short-lived task.
                    tokio::spawn(async move {
                        let outcome = dispatcher_for_request
                            .dispatch_checked_session_request(
                                &caller_ura_for_reply,
                                &ability_ura,
                                &args,
                                &args_content_envelope,
                            )
                            .await;
                        let frame = build_session_request_result_frame(call_id, outcome);
                        push_session_request_result(
                            &presence_for_reply,
                            &caller_ura_for_reply,
                            &id_hex,
                            frame,
                        );
                    });
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

    let caller_realm = parse_realm_from_ura(caller_ura).ok_or_else(|| {
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

fn session_trust_context_from_open(
    caller_ura: &str,
    envelope_open: &EnvelopeOpen,
) -> SessionTrustContext {
    let is_user = matches!(
        crate::core::ura::parse_ura(caller_ura).map(|parsed| parsed.kind),
        Ok(crate::core::ura::URAKind::User)
    );
    if !is_user {
        return SessionTrustContext::default();
    }
    let presented = envelope_open
        .envelope
        .as_ref()
        .and_then(|envelope| envelope.caller_signature.as_ref())
        .map(|signature| signature.key_id_hint.trim().to_string())
        .unwrap_or_default();
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
    Ok(build_carrier_v1_dispatch_frame(call_id, request, true))
}

fn remote_bidi_forwarded_request(
    selected_route: &SelectedInvokeRoute,
    envelope_open: &EnvelopeOpen,
    call_mode: CallMode,
) -> Result<easynet_axon::pb::axon::v1::InvokeRequest, Status> {
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
    Ok(easynet_axon::pb::axon::v1::InvokeRequest {
        envelope: Some(signed_envelope_for_selected_route(
            envelope,
            selected_route,
            envelope_open
                .metadata
                .get(SIGNED_DESCRIPTOR_REF_METADATA_KEY)
                .map(String::as_str),
            &envelope_open.initial_args,
        )?),
        function_name: selected_route.dispatch_name.clone(),
        arguments: envelope_open.initial_args.clone(),
        metadata: envelope_open.metadata.clone(),
        ..Default::default()
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
    if ability == crate::daemon::ability::builtins::device_control::terminal::attach::ABILITY_PTY_SESSION_ATTACH {
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
    use easynet_axon::pb::axon::v1::SessionOpenExt;

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
    fn v1_ext_negotiates_proto_and_caps_at_hub_version() {
        let ext = SessionOpenExt {
            contract_version: 7, // future device, older hub
            claimant_boot_nonce: vec![3; 16],
        };
        let c = session_contract_from_ext(Some(&ext)).expect("v1+ carrier negotiates");
        assert_eq!(
            c.version.min(CANONICAL_SESSION_CARRIER_VERSION),
            CANONICAL_SESSION_CARRIER_VERSION
        );
        assert_eq!(c.claimant_boot_nonce.len(), 16);
    }

    #[test]
    fn carrier_v1_failure_maps_to_single_track_projection() {
        let pb = easynet_axon::pb::axon::v1::DispatchResult {
            call_id: 7,
            payload: b"partial".to_vec(),
            terminal: true,
            failure: Some(easynet_axon::pb::axon::v1::Error {
                code: "TARGET_OFFLINE".into(),
                message: "device went away".into(),
                retryable: true,
                ..Default::default()
            }),
            ..Default::default()
        };
        let mapped = pending_result_from_carrier_v1(&pb);
        assert_eq!(mapped.payload, b"partial");
        assert_eq!(mapped.error.as_deref(), Some("device went away"));
        let failure = mapped.failure.expect("typed failure carried");
        assert_eq!(failure.code, "TARGET_OFFLINE");
        assert!(failure.retryable);
        assert!(mapped.request_id.is_none());
    }

    #[test]
    fn carrier_v1_unary_success_without_checkpoints_is_rejected() {
        let pb = easynet_axon::pb::axon::v1::DispatchResult {
            call_id: 8,
            payload: b"ok".to_vec(),
            terminal: true,
            failure: None,
            ..Default::default()
        };
        let (_, failure) = classify_carrier_v1_result(pb)
            .expect_err("successful unary result without checkpoints must fail closed");
        assert_eq!(
            failure.failure.as_ref().map(|value| value.code.as_str()),
            Some("CANONICAL_FINALIZATION_REQUIRED")
        );
    }

    #[test]
    fn carrier_v1_unary_success_requires_both_checkpoints() {
        let pb = easynet_axon::pb::axon::v1::DispatchResult {
            call_id: 8,
            payload: b"ok".to_vec(),
            terminal: true,
            admission_receipt: Some(easynet_axon::pb::axon::v1::InvocationReceipt {
                state: easynet_axon::invocation::InvocationState::Admitted.to_wire_i32(),
                ..Default::default()
            }),
            terminal_receipt: Some(easynet_axon::pb::axon::v1::InvocationReceipt {
                state: easynet_axon::invocation::InvocationState::Completed.to_wire_i32(),
                ..Default::default()
            }),
            ..Default::default()
        };
        let (_, CarrierDispatchEvent::Terminal(mapped)) =
            classify_carrier_v1_result(pb).expect("paired unary checkpoints are accepted")
        else {
            panic!("unary result must classify as terminal");
        };
        assert!(mapped.error.is_none());
        assert!(mapped.admission_receipt.is_some());
        assert!(mapped.terminal_receipt.is_some());
    }

    #[test]
    fn carrier_v1_stream_terminal_cannot_repeat_admission_checkpoint() {
        let pb = easynet_axon::pb::axon::v1::DispatchResult {
            call_id: 9,
            terminal: true,
            admission_receipt: Some(easynet_axon::pb::axon::v1::InvocationReceipt {
                state: easynet_axon::invocation::InvocationState::Admitted.to_wire_i32(),
                ..Default::default()
            }),
            terminal_receipt: Some(easynet_axon::pb::axon::v1::InvocationReceipt {
                state: easynet_axon::invocation::InvocationState::Completed.to_wire_i32(),
                ..Default::default()
            }),
            ..Default::default()
        };
        let (_, failure) = classify_carrier_v1_result(pb)
            .expect_err("stream admission and terminal checkpoints use distinct frames");
        assert_eq!(
            failure.failure.as_ref().map(|value| value.code.as_str()),
            Some("CARRIER_STREAM_PHASE_INVALID")
        );
    }

    #[test]
    fn carrier_v1_stream_terminal_failure_requires_terminal_checkpoint() {
        let pb = easynet_axon::pb::axon::v1::DispatchResult {
            call_id: 9,
            terminal: true,
            failure: Some(easynet_axon::pb::axon::v1::Error {
                code: "STREAM_OPEN_FAILED".into(),
                message: "target rejected stream open".into(),
                retryable: false,
                ..Default::default()
            }),
            ..Default::default()
        };
        let (_, failure) = classify_carrier_v1_result(pb)
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

    #[tokio::test]
    async fn session_down_stream_prioritizes_control_frames_over_normal_backlog() {
        let (tx, rx) = mpsc::channel::<Result<DispatchFrame, Status>>(8);
        let mut stream = SessionDownStream::new(rx, build_session_established_control(1, 7, false));

        let first = stream.next().await.expect("control frame").expect("ok");
        assert!(
            matches!(first.payload, Some(DownPayload::Control(_))),
            "frame 0 is typed session control"
        );

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
        let Some(DownPayload::BinaryChunk(chunk)) = prioritized.payload else {
            panic!("expected prioritized BinaryChunk");
        };
        assert_eq!(chunk.data, b"control");

        let normal_a = stream.next().await.expect("normal-a").expect("ok");
        let Some(DownPayload::BinaryChunk(chunk)) = normal_a.payload else {
            panic!("expected normal-a BinaryChunk");
        };
        assert_eq!(chunk.data, b"normal-a");

        let normal_b = stream.next().await.expect("normal-b").expect("ok");
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
