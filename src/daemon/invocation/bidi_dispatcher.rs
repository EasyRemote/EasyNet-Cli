// EasyNet Daemon — InvokeBidi Dispatcher
// ========================================
//
// File: src/daemon/invocation/bidi_dispatcher.rs
// Description: Owns every `InvokeBidi` routing decision the daemon
//              makes after frame-0 transport policy (commit-plan-2
//              Axis E / E2, final dispatcher):
//
//                * `runtime.invoke_remote` — hub-side RFC-005 per-call
//                  dispatch over a device's session reverse channel
//                * `session.open` — device session accept loop +
//                  the hub-side session-frame request dispatcher
//                * plugin/builtin bidi wire abilities — local PTY/
//                  file-transfer adapters and the remote bidi bridge
//
//              Also owns the bidi wire furniture: frame-0 validation,
//              terminal/admission receipt builders, the local bidi
//              frame mappers, and the session/local down-stream types.
//
//              Composes `UnaryDispatcher` for the self-targeted
//              invoke_remote fast path and the forward-route
//              resolution it shares with the unary plane.
//
// Author: Silan Hu <silan.hu@u.nus.edu>
// Copyright (c) 2026 EasyNet. All rights reserved.

use std::collections::HashMap;
use std::pin::Pin;
use std::sync::Arc;
use std::task::{Context, Poll};
use std::time::Duration;

use std::future::Future;

use futures::Stream;
use tokio::sync::mpsc;
use tokio_stream::wrappers::ReceiverStream;
use tokio_stream::StreamExt;
use tonic::{Response, Status, Streaming};

use easynet_axon::pb::axon::v1::{
    invoke_bidi_down::Payload as DownPayload, invoke_bidi_up::Payload as UpPayload, BidiControl,
    BinaryChunk, EnvelopeOpen, Error, ErrorStage, InvocationReceipt, InvokeBidiDown, InvokeBidiUp,
    SecurityClass, StreamDescriptor,
};

use crate::daemon::invocation::admission_facade::AdmissionFacade;
use crate::daemon::invocation::deps::{DirectoryPlane, IdentityPlane, RuntimePlane, SessionPlane};
use crate::daemon::invocation::descriptor_binding::RuntimeBoundAbility;
use crate::daemon::invocation::federation_wrappers;
use crate::daemon::invocation::federation_wrappers::{
    ABILITY_FEDERATION_ADVERTISE_ABILITIES, ABILITY_FEDERATION_ADVERTISE_AGENT,
    ABILITY_FEDERATION_FORWARD_INVOKE,
};
use crate::daemon::invocation::hosted_agent_delegation::HostedAgentDelegationIssuer;
use crate::daemon::invocation::invocation_wire::{
    status_from_axon_invoke_error, target_ura_from_envelope, BoxedDownStream,
};
use crate::daemon::invocation::invoke_remote_initiator::{
    build_carrier_v1_dispatch_frame, build_invoke_remote_dispatch_frame,
    build_invoke_remote_terminal_frame, call_id_hex, decode_inner_payload,
    invoke_remote_inband_error_response, InnerPayload, InvokeRemoteDispatchFrameRequest,
    InvokeRemoteDown, InvokeRemoteUp, RequestOutcome, SessionContentEnvelope, SessionDispatch,
    SessionRequestError, ABILITY_INVOKE_REMOTE, INVOKE_REMOTE_STREAM_ID,
};
use crate::daemon::invocation::ledger_projection::ledger_record_from_remote_receipt;
use crate::daemon::invocation::register_device_pubkey::parse_realm_from_ura;
use crate::daemon::invocation::route_resolver::{ForwardInvokeRouteSelection, SelectedInvokeRoute};
use crate::daemon::invocation::session_initiator::ABILITY_SESSION_OPEN;
use crate::daemon::invocation::target_gate::{
    envelope_with_selected_callee, route_negative_message, route_negative_status,
    route_owner_mismatch_message, route_profile_blocked_message, route_profile_blocked_status,
    route_selected_remote_host_status, selected_host_unavailable_message, TargetGate,
};
use crate::daemon::invocation::unary_dispatcher::UnaryDispatcher;
use easynet_axon::invocation::{AbilityFrame, BidiInputFrame};

use crate::daemon::invocation::state::pending_dispatch::{
    DispatchResult, DispatchStreamEvent, PendingDispatchMap, PendingStreamDispatchMap,
};
use crate::daemon::invocation::state::presence::{
    DispatchFrame, DispatchSender, OfflineReason, PresenceRegistry, SessionContract,
    SessionTrustContext, DISPATCH_CHANNEL_CAPACITY,
};
use crate::daemon::invocation::state::session_failure::SessionFailure;
use crate::daemon::trust::anchor::RealmTrustAnchor;

/// Named runtime-admin abilities the `InvokeBidi` dispatcher routes by
/// exact name (as opposed to the generic `is_bidi_wire_ability` remote
/// bridge fall-through). This is the single source of truth consumed by
/// `daemon::ability::conformance::RuntimeAdminConformance`; the `match`
/// arms in `dispatch` reference the same constants, so a baseline row can
/// never claim an `AxonRuntimeAdmin` ability the dispatcher does not
/// actually install (SPEC §7.1 notes 6/7, §7.3 item 7, §9.1 item 13).
pub(crate) const RUNTIME_ADMIN_BIDI_ROUTES: &[&str] =
    &[ABILITY_INVOKE_REMOTE, ABILITY_SESSION_OPEN];

fn local_bidi_wire_kind_for(
    registry: &crate::daemon::ability::wire::AbilityWireRegistry,
    ability: &str,
) -> Option<LocalBidiWireKind> {
    local_bidi_wire_kind_for_registry_key(registry, ability).or_else(|| {
        descriptor_ref_local_registry_key(ability)
            .and_then(|key| local_bidi_wire_kind_for_registry_key(registry, &key))
    })
}

fn local_bidi_wire_kind_for_registry_key(
    registry: &crate::daemon::ability::wire::AbilityWireRegistry,
    ability: &str,
) -> Option<LocalBidiWireKind> {
    registry
        .bidi_wire_kind_for(ability)
        .or_else(|| crate::daemon::ability::wire::core_bidi_wire_kind_for(ability))
}

fn descriptor_ref_local_registry_key(ability: &str) -> Option<String> {
    let descriptor_ref =
        easynet_axon::invocation::canonical_ability_descriptor_ref(ability).ok()?;
    let ability_ura = crate::daemon::axon_bridge::descriptor_ref::ability_ura_from_descriptor_ref(
        &descriptor_ref,
    )
    .ok()?;
    crate::ura::AbilitySelector::parse(&ability_ura)
        .ok()
        .map(|selector| selector.local_registry_ability().to_string())
}

fn local_is_bidi_wire_ability(
    registry: &crate::daemon::ability::wire::AbilityWireRegistry,
    ability: &str,
) -> bool {
    local_bidi_wire_kind_for(registry, ability).is_some()
}

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
            ABILITY_INVOKE_REMOTE => self.dispatch_invoke_remote(envelope_open, up).await,
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
                let contract = session_contract_from_ext(envelope_open.session_ext.as_ref());
                self.dispatch_self_session_accept(caller_ura, envelope_open, contract, up)
                    .await
            }
            other if local_is_bidi_wire_ability(&self.runtime.ability_wire, other) => {
                if let Some(target_ura) = remote_bidi_target_ura(envelope_open) {
                    if !self.gate.matches_self_target_ura(&target_ura).await {
                        // RFC-005 resolve-first gate. Mirror the
                        // `runtime.invoke_remote` resolver call site:
                        // prove the wire ability exists on the target
                        // and that the selected route is
                        // authoritative-local-or-better BEFORE bridging
                        // the bidi stream.
                        let route_result = self
                            .gate
                            .route_resolver()
                            .await
                            .resolve_route(&target_ura, other);
                        return match route_result {
                            Ok(route) if route.is_authoritative_local_or_better() => {
                                self.dispatch_remote_bidi(&route, envelope_open, up).await
                            }
                            Ok(route) => Err(route_profile_blocked_status(&route)),
                            Err(failure) => Err(route_negative_status(failure)),
                        };
                    }
                }
                self.dispatch_local_bidi_selected_route(envelope_open, up)
                    .await
            }
            other => {
                let ability_debug = format!("{other:?}");
                let ability_hex = other
                    .as_bytes()
                    .iter()
                    .map(|byte| format!("{byte:02x}"))
                    .collect::<Vec<_>>()
                    .join("");
                let registry_known = self
                    .runtime
                    .ability_wire
                    .bidi_wire_kind_for(other)
                    .is_some();
                let core_known =
                    crate::daemon::ability::wire::core_bidi_wire_kind_for(other).is_some();
                crate::op_event!(
                    component = daemon_invocation,
                    kind = invoke_bidi_unwired_ability,
                    ability = other,
                    ability_debug = ability_debug.as_str(),
                    ability_hex = ability_hex.as_str(),
                    registry_known = registry_known.to_string().as_str(),
                    core_known = core_known.to_string().as_str(),
                );
                Err(Status::unimplemented(format!(
                    "easynet-daemon: InvokeBidi ability `{other}` is not yet wired; \
                     only built-in PTY/file-transfer or plugin-declared bidi abilities currently have \
                     daemon gRPC wire adapters"
                )))
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
        receipt: None,
    }
}

impl BidiDispatcher {
    pub(crate) async fn dispatch_remote_bidi(
        &self,
        selected_route: &SelectedInvokeRoute,
        envelope_open: &EnvelopeOpen,
        mut up: Streaming<InvokeBidiUp>,
    ) -> Result<Response<BoxedDownStream<InvokeBidiDown>>, Status> {
        let pending = self.sessions.pending_stream.as_ref().ok_or_else(|| {
            Status::failed_precondition(format!(
                "InvokeBidi {}: daemon was constructed without a \
                 PendingStreamDispatchMap; boot must call with_pending_stream(...) \
                 to enable remote bidi bridging",
                selected_route.dispatch_name
            ))
        })?;
        let (session_id, sender) = self
            .directory
            .presence
            .lookup_tracked(&selected_route.execution_host_ura)
            .ok_or_else(|| {
                Status::failed_precondition(selected_host_unavailable_message(selected_route))
            })?;

        let mut handle = pending.register_pending_for(&selected_route.execution_host_ura);
        let call_id = handle.call_id();
        let stdout_stream_id = local_bidi_stdout_stream_id(envelope_open);

        let target_contract_v1 = self
            .directory
            .presence
            .dispatch_contract_version(&selected_route.execution_host_ura)
            .unwrap_or(0)
            >= 1;
        let open_frame = build_remote_bidi_open_frame_for_contract(
            target_contract_v1,
            call_id,
            selected_route,
            envelope_open,
        )?;
        match sender.try_send(Ok(open_frame)) {
            Ok(()) => {}
            Err(tokio::sync::mpsc::error::TrySendError::Full(_)) => {
                // Full = device is slow, not dead: keep its session,
                // fail only this call as retryable backpressure.
                return Err(Status::resource_exhausted(
                    federation_wrappers::FORWARD_INVOKE_TARGET_BUSY_REASON,
                ));
            }
            Err(tokio::sync::mpsc::error::TrySendError::Closed(_)) => {
                self.directory.presence.remove_if_session(
                    &selected_route.execution_host_ura,
                    session_id,
                    crate::daemon::invocation::state::presence::OfflineReason::StreamClosed,
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
                    DispatchStreamEvent::Terminal(result) => {
                        let DispatchResult {
                            payload,
                            error,
                            failure,
                            request_id: _,
                            receipt: _,
                        } = *result;
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
                            federation_wrappers::FORWARD_INVOKE_TARGET_BUSY_REASON.to_string();
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
                            crate::daemon::invocation::state::presence::OfflineReason::StreamClosed,
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
    /// device-side terminal attach handler expects JSON
    /// `{"type":"stdin","data":"<base64>"}` — its on-the-wire
    /// shape lives with the terminal system ability. We
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
            .gate
            .route_resolver()
            .await
            .resolve_route(&target_ura, ability)
            .map_err(route_negative_status)?;
        if !selected_route.is_authoritative_local_or_better() {
            return Err(route_profile_blocked_status(&selected_route));
        }
        let execution_host_is_self = self
            .gate
            .matches_self_target_ura(&selected_route.execution_host_ura)
            .await;
        if !selected_route
            .dispatch_target(execution_host_is_self)
            .is_local_runtime()
        {
            return Err(route_selected_remote_host_status(
                "InvokeBidi",
                &selected_route,
            ));
        }
        Ok(selected_route)
    }

    pub(crate) async fn dispatch_local_bidi_selected_route(
        &self,
        envelope_open: &EnvelopeOpen,
        mut up: Streaming<InvokeBidiUp>,
    ) -> Result<Response<BoxedDownStream<InvokeBidiDown>>, Status> {
        let selected_route = self.resolve_local_bidi_route(envelope_open).await?;
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
        let bound_ability =
            RuntimeBoundAbility::from_selected_route("InvokeBidi", runtime, &selected_route)
                .await?;
        let dispatch_descriptor_ref = bound_ability
            .descriptor_ref_for_mode(
                "InvokeBidi",
                &selected_route.callee_ura,
                easynet_axon::invocation::CallMode::Bidi,
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
        let loopback_admitted = self
            .admission
            .accepts_loopback_envelope(envelope_open.envelope.as_ref());
        let wire = if loopback_admitted {
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
        let wire_kind = local_bidi_wire_kind_for(
            &self.runtime.ability_wire,
            &selected_route.dispatch_name,
        )
        .ok_or_else(|| {
            Status::failed_precondition(format!(
                "InvokeBidi: ability `{}` is registered as local bidi but has no declared wire protocol",
                selected_route.dispatch_name
            ))
        })?;
        let handle =
            crate::daemon::axon_bridge::dispatch_shim::open_bidi_external_signed(runtime, wire)
                .await
                .map_err(|err| {
                    status_from_axon_invoke_error("InvokeBidi", &dispatch_ability, err)
                })?;
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

    /// Hub-side `runtime.invoke_remote` handler. Drives the RFC-005
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
    pub(crate) async fn dispatch_invoke_remote(
        &self,
        envelope_open: &EnvelopeOpen,
        _up: Streaming<InvokeBidiUp>,
    ) -> Result<Response<BoxedDownStream<InvokeBidiDown>>, Status> {
        let request: InvokeRemoteUp =
            serde_json::from_slice(&envelope_open.initial_args).map_err(|err| {
                Status::invalid_argument(format!(
                    "runtime.invoke_remote: frame-0 initial_args is not valid \
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
            origin_caller,
        } = request;

        let selected_route = match self
            .gate
            .route_resolver()
            .await
            .resolve_route(&ability_ura, "")
        {
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
        let inner_subject = subject_ura.trim();
        if inner_subject.is_empty() {
            return invoke_remote_inband_error_response(
                "runtime.invoke_remote: missing inner subject_ura".to_string(),
            );
        }
        let outer_caller = envelope_open
            .envelope
            .as_ref()
            .and_then(|envelope| envelope.caller.clone())
            .ok_or_else(|| {
                Status::invalid_argument(
                    "runtime.invoke_remote: admitted frame-0 envelope is missing caller",
                )
            })?;
        let inner_envelope = crate::daemon::invocation::ProtoEnvelope::targeted(
            outer_caller.ura,
            selected_route.callee_ura.clone(),
            inner_subject,
        )
        .map_err(|err| {
            Status::invalid_argument(format!(
                "runtime.invoke_remote: invalid inner envelope: {err}"
            ))
        })?
        .into_inner();
        self.admission.verify_delegation_for_envelope(
            &inner_envelope,
            &public_ability,
            &metadata,
        )?;

        // ── Phase 4: Axon-routed **self-target** dispatch ──────────
        //
        // If a shared `LocalRuntime` is wired AND the resolver-selected
        // execution host names THIS daemon's own URA, route the call through
        // the Axon bridge's descriptor-bound request path. Axon owns
        // admission, the state machine, and ledger persistence; the bridge
        // shim drains the handle and produces the wire-shape `(payload,
        // error)` pair we emit in the one-shot terminal frame.
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
        // Why the bridge uses the outer signed envelope:
        // `InvokeRemoteUp::Request` does not carry a separate inner
        // user-signed descriptor-bound envelope. The Go shim
        // (`backend/internal/daemon_grpc/remote_routing.go:197`)
        // decomposes the user route and re-issues the request through
        // `runtime.invoke_remote`; this daemon therefore verifies the
        // caller material present on the outer envelope and dispatches the
        // resolver-selected local ability under the Axon bridge's public
        // descriptor-bound request APIs.
        //
        // Self-targeted invoke_remote never goes through the pending
        // session map. The daemon's shared
        // Axon `LocalRuntime` is the only local execution surface; if
        // the ability is absent, Axon returns the in-band error frame.
        let execution_host_is_self = self
            .gate
            .matches_self_target_ura(&selected_route.execution_host_ura)
            .await;
        if selected_route
            .dispatch_target(execution_host_is_self)
            .is_local_runtime()
        {
            return self
                .unary
                .dispatch_self_targeted_invoke_remote(
                    &selected_route,
                    Some(inner_subject),
                    &args,
                    &metadata,
                    origin_caller.as_ref(),
                )
                .await;
        }

        let pending = self.sessions.pending.as_ref().ok_or_else(|| {
            Status::failed_precondition(
                "runtime.invoke_remote: daemon was constructed without a \
                 PendingDispatchMap; call DaemonInvocationService::with_pending(...) \
                 at boot to enable cross-device invocation",
            )
        })?;

        let (target_session_id, target_sender) = match self
            .directory
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
        let mut stream_handle = self.sessions.pending_stream.as_ref().map(|pending_stream| {
            pending_stream.register_pending_for(&selected_route.execution_host_ura)
        });
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
        // Carrier selection (DEC-F004 rolling upgrade): a v1 device
        // gets the canonical proto frame — the caller's EnvelopeOpen
        // envelope forwarded verbatim inside a complete InvokeRequest.
        // Calls carrying an origin_caller claim stay on the JSON shape
        // until the backend submits real-user envelopes directly
        // (T2.1b dissolves the claim into caller + caller_signature);
        // that fallback dies with the JSON carrier.
        let target_contract_v1 = self
            .directory
            .presence
            .dispatch_contract_version(&selected_route.execution_host_ura)
            .unwrap_or(0)
            >= 1;
        let dispatch_frame = if target_contract_v1 && origin_caller.is_none() {
            build_carrier_v1_dispatch_frame(
                call_id,
                easynet_axon::pb::axon::v1::InvokeRequest {
                    envelope: Some(inner_envelope),
                    function_name: selected_route.dispatch_name.clone(),
                    arguments: args.clone(),
                    content_envelope: envelope_open.content_envelope.clone(),
                    metadata: metadata.clone(),
                    ..easynet_axon::pb::axon::v1::InvokeRequest::default()
                },
                false,
            )
        } else {
            build_invoke_remote_dispatch_frame(InvokeRemoteDispatchFrameRequest {
                call_id,
                callee_ura: &selected_route.callee_ura,
                subject_ura: inner_subject,
                ability: &dispatch_ability,
                args: &args,
                args_content_envelope,
                metadata,
                origin_caller,
            })?
        };
        match target_sender.try_send(Ok(dispatch_frame)) {
            Ok(()) => {}
            Err(tokio::sync::mpsc::error::TrySendError::Full(_)) => {
                // Full = the device is slow (its session drain is
                // behind), not dead. Keep its session and fail only
                // this call as retryable backpressure — evicting
                // here turned one >256-frame burst into a false
                // offline plus a failure avalanche for every
                // pending call (measured 2026-06-12).
                return invoke_remote_inband_error_response(format!(
                    "runtime.invoke_remote: selected execution host `{}` dispatch \
                     channel full ({}); retry",
                    selected_route.execution_host_ura,
                    federation_wrappers::FORWARD_INVOKE_TARGET_BUSY_REASON,
                ));
            }
            Err(tokio::sync::mpsc::error::TrySendError::Closed(_)) => {
                self.directory.presence.remove_if_session(
                    &selected_route.execution_host_ura,
                    target_session_id,
                    OfflineReason::StreamClosed,
                );
                return invoke_remote_inband_error_response(format!(
                    "runtime.invoke_remote: selected execution host `{}` receiver closed \
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
                        DispatchStreamEvent::Terminal(result) => {
                            let DispatchResult {
                                payload,
                                error,
                                failure,
                                request_id,
                                receipt: _,
                            } = *result;
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
            // Carrier-v1 receipt projection (DEC-F004 landing audit 3):
            // the consumer side holds the dispatch context the receipt's
            // axiom echo lacks (ability), so the hub's ledger row is
            // written here, not in the drain.
            let ledger_for_receipt = self.runtime.invocation_ledger.clone();
            let ability_for_receipt = dispatch_ability.clone();
            let dispatch_started_unix_ms = crate::daemon::federation::directory::now_unix_ms();
            tokio::spawn(async move {
                let frame = match handle.await_reply().await {
                    Ok(DispatchResult {
                        payload,
                        error,
                        failure,
                        request_id,
                        receipt,
                    }) => {
                        if let (Some(ledger), Some(receipt)) =
                            (ledger_for_receipt.as_ref(), receipt.as_ref())
                        {
                            match ledger_record_from_remote_receipt(
                                receipt,
                                &ability_for_receipt,
                                dispatch_started_unix_ms,
                            )
                            .and_then(|record| {
                                ledger.put(&record).map(|()| record).map_err(Into::into)
                            }) {
                                Ok(record) => crate::op_event!(
                                    component = daemon_invocation,
                                    kind = carrier_v1_receipt_ledgered,
                                    invocation_ura = record.invocation_ura,
                                    state = record.state,
                                ),
                                Err(err) => {
                                    let err_msg = format!("{err}");
                                    crate::op_event!(
                                        component = daemon_invocation,
                                        kind = ledger_write_failed,
                                        shape = "carrier_v1_receipt",
                                        error = err_msg,
                                    );
                                }
                            }
                        }
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

    /// Hub-side acceptor for `session.open`. The device opens a
    /// long-lived `InvokeBidi` against the daemon at boot and holds
    /// the stream open for the daemon process's lifetime; this is
    /// the canonical reverse channel through which the hub pushes
    /// `runtime.invoke_remote` `SessionDispatch::Dispatch` frames
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
    ///    `runtime.invoke_remote` caller is awaiting; on stream
    ///    close, remove the registry entry with the appropriate
    ///    `OfflineReason`
    /// 4. Return the down-stream wrapping `rx` so tonic pumps every
    ///    `DispatchFrame` (BinaryChunk-wrapped `SessionDispatch::Dispatch`)
    ///    pushed into `tx` back to the device
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

        // Step 1: register before spawning so a SessionDispatch::Dispatch
        // arriving from `runtime.invoke_remote` immediately can find this
        // sender. The PresenceRegistry handles displacement (Offline +
        // Online emission ordering) under the hood; the slot remembers
        // the frame-0 carrier negotiation (DEC-F004).
        let negotiated_version = contract.version.min(HUB_DISPATCH_CONTRACT_VERSION);
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

        // Step 2: spawn the up-stream consumer. Reads device replies
        // (SessionDispatch::Result frames) and routes them to the
        // PendingDispatchMap so the originating runtime.invoke_remote
        // caller wakes up.
        let presence_for_drain = Arc::clone(&self.directory.presence);
        let pending_for_drain = self.sessions.pending.clone();
        let pending_stream_for_drain = self.sessions.pending_stream.clone();
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
        // `down_tx` from runtime.invoke_remote dispatchers and from
        // federation.forward_invoke pushers as `DispatchFrame`
        // (presence_registry's newtype around `InvokeBidiDown`).
        // The tonic trait wants raw `InvokeBidiDown`, so map each
        // frame to unwrap the newtype.
        let stream = SessionDownStream::new(
            down_rx,
            build_session_down_admission_receipt(
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
    DispatchFrame {
        frame: InvokeBidiDown {
            payload: Some(DownPayload::Control(BidiControl::default())),
            ..InvokeBidiDown::default()
        },
    }
}

/// Build the spec §1.1 admission-accept frame: down frame 0 carries
/// an `InvocationReceipt` with `state = Admitted`. The receipt is
/// what tells the device-side caller "your `session.open` open was
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

/// Hub's highest supported dispatch contract (DEC-F004). Bump when a
/// new frame generation lands; negotiation is min(device, hub).
pub(crate) const HUB_DISPATCH_CONTRACT_VERSION: u32 = 1;

/// Map frame-0's optional SessionOpenExt into negotiation facts.
/// Absent ext = legacy JSON device (contract v0).
pub(crate) fn session_contract_from_ext(
    ext: Option<&easynet_axon::pb::axon::v1::SessionOpenExt>,
) -> SessionContract {
    ext.map(|e| SessionContract {
        version: e.contract_version,
        claimant_boot_nonce: e.claimant_boot_nonce.clone(),
    })
    .unwrap_or_else(SessionContract::legacy)
}

/// Frame-0 down: admission receipt carrying the negotiated session
/// contract (mini-RFC §2). The device reads `session_contract` to
/// learn which frame encoding to write and whether it displaced a
/// prior session (T1.1 skew + displacement become protocol facts,
/// not inference).
fn build_session_down_admission_receipt(
    negotiated_version: u32,
    hub_session_id: u64,
    displaced_prior: bool,
) -> InvokeBidiDown {
    let payload = serde_json::to_vec(&serde_json::json!({
        "session_contract": {
            "version": negotiated_version,
            "dispatch_encoding": if negotiated_version >= 1 { "proto" } else { "json" },
            "hub_session_id": hub_session_id.to_string(),
            "displaced_prior": displaced_prior,
        }
    }))
    .expect("session_contract is statically serializable");
    let mut frame = build_bidi_admission_receipt();
    if let Some(DownPayload::Receipt(receipt)) = frame.payload.as_mut() {
        receipt.payload = payload;
    }
    frame
}

pub(crate) fn build_bidi_terminal_receipt(
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

pub(crate) fn terminal_failure_message(reason: &str, fallback_code: &str) -> String {
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
    Terminal(InvokeBidiDown),
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
        LocalBidiWireKind::JsonFrames => {
            let payload = match serde_json::to_vec(value) {
                Ok(payload) => payload,
                Err(err) => {
                    return LocalBidiHandlerFrame::ProtocolFailure(format!(
                        "InvokeBidi local-runtime: JSON frame re-encode failed: {err}"
                    ));
                }
            };
            match value.get("type").and_then(|field| field.as_str()) {
                Some("error") => {
                    let code = value.get("code").and_then(|field| field.as_str());
                    LocalBidiHandlerFrame::Terminal(
                        build_bidi_terminal_receipt_with_payload_and_failure_code(
                            easynet_axon::invocation::InvocationState::Failed,
                            json_frame_error_reason(value),
                            Some((payload, "application/json")),
                            code,
                        ),
                    )
                }
                Some("closed") => {
                    LocalBidiHandlerFrame::Terminal(build_bidi_terminal_receipt_with_payload(
                        easynet_axon::invocation::InvocationState::Completed,
                        String::new(),
                        Some((payload, "application/json")),
                    ))
                }
                _ => LocalBidiHandlerFrame::Forward(InvokeBidiDown {
                    payload: Some(DownPayload::BinaryChunk(BinaryChunk {
                        stream_id: stdout_stream_id,
                        data: payload,
                        ..BinaryChunk::default()
                    })),
                    ..InvokeBidiDown::default()
                }),
            }
        }
    }
}

fn json_frame_error_reason(value: &serde_json::Value) -> String {
    match (
        value.get("code").and_then(|field| field.as_str()),
        value.get("message").and_then(|field| field.as_str()),
    ) {
        (Some(code), Some(message)) if !code.trim().is_empty() && !message.trim().is_empty() => {
            format!("{code}: {message}")
        }
        (_, Some(message)) if !message.trim().is_empty() => message.to_string(),
        (Some(code), _) if !code.trim().is_empty() => code.to_string(),
        _ => "JSON-frame bidi handler returned error".to_string(),
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
    fn new(
        down_rx: tokio::sync::mpsc::Receiver<Result<DispatchFrame, Status>>,
        admission_receipt: InvokeBidiDown,
    ) -> Self {
        Self {
            down_rx,
            next_heartbeat: Box::pin(tokio::time::sleep(SESSION_DOWN_HEARTBEAT_INTERVAL)),
            next_sequence: 0,
            pending_admission_receipt: Some(admission_receipt),
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

impl BidiDispatcher {
    /// PR-N6 C3 hub-side handler for inbound `SessionDispatch::Request`
    /// frames arriving on a device's `session.open` bidi. Validates
    /// the hub-owned Ability URA, routes the derived public wrapper
    /// ability through the same dispatch arms the unary `Invoke` RPC
    /// consults, then maps the result into the typed `RequestOutcome`
    /// shape.
    ///
    /// Spec scope: forwards `federation.forward_invoke` plus the
    /// hosted-agent self-advertise repair pair —
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
                }
            }
        };
        self.dispatch_session_request_named(&ability, args).await
    }

    /// Carrier-v1 entry (DEC-F004): the proto frame already carries the
    /// canonical ability name in `request.function_name`, so the URA →
    /// public-name projection of the JSON path is unnecessary. The
    /// dispatch match below is itself the hub's public-ability
    /// whitelist (unknown names return PermissionDenied).
    pub(crate) async fn dispatch_session_request_named(
        &self,
        ability: &str,
        args: &[u8],
    ) -> RequestOutcome {
        match ability {
            ABILITY_FEDERATION_FORWARD_INVOKE => {
                self.emit_session_request_resolution_marker(args).await;

                match self
                    .unary
                    .dispatch_federation_forward_invoke(None, args)
                    .await
                {
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
                match self.unary.dispatch_federation_advertise_agent(args) {
                    Ok(response) => {
                        let body = response.into_inner();
                        RequestOutcome::Ok {
                            result_bytes: body.result,
                        }
                    }
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
                match self.unary.dispatch_federation_advertise_abilities(args) {
                    Ok(response) => {
                        let body = response.into_inner();
                        RequestOutcome::Ok {
                            result_bytes: body.result,
                        }
                    }
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
                         only `{ABILITY_FEDERATION_FORWARD_INVOKE}`, \
                         `{ABILITY_FEDERATION_ADVERTISE_AGENT}`, \
                         `{ABILITY_FEDERATION_ADVERTISE_ABILITIES}`, and \
                         `{}` are wired",
                        federation_wrappers::ABILITY_FEDERATION_RESOLVE_KEY
                    ),
                },
            },
        }
    }

    pub(crate) async fn emit_session_request_resolution_marker(&self, args: &[u8]) {
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

        match self
            .unary
            .resolve_forward_invoke_selection(&request, &inner_payload)
            .await
        {
            Ok(ForwardInvokeRouteSelection::Local(selected_route)) => {
                let execution_host_is_self = self
                    .gate
                    .matches_self_target_ura(&selected_route.execution_host_ura)
                    .await;
                let path = if selected_route
                    .dispatch_target(execution_host_is_self)
                    .is_local_runtime()
                {
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
            }
            Ok(ForwardInvokeRouteSelection::Peer(delegation)) => {
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
            .identity
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
/// device's `session.open` reverse channel. Encoding failure is
/// vanishingly unlikely (owned `[u8; 16]`, owned `Vec<u8>`,
/// typed enum) but mapped to a synthetic `UpstreamFailure` outcome
/// so a malformed inner result never silently wedges the device.
pub(crate) fn build_session_request_result_frame(
    call_id: [u8; 16],
    outcome: RequestOutcome,
) -> crate::daemon::invocation::state::presence::DispatchFrame {
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
    crate::daemon::invocation::state::presence::DispatchFrame {
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
pub(crate) fn push_session_request_result(
    presence: &Arc<PresenceRegistry>,
    caller_ura: &str,
    id_hex: &str,
    frame: crate::daemon::invocation::state::presence::DispatchFrame,
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
        receipt: result.receipt.clone(),
    }
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
    outcome: RequestOutcome,
) -> DispatchFrame {
    use easynet_axon::pb::axon::v1::ReverseDispatchResult;
    let (payload, failure) = match outcome {
        RequestOutcome::Ok { result_bytes } => (result_bytes, None),
        RequestOutcome::Err { error } => {
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
            )
        }
    };
    DispatchFrame {
        frame: InvokeBidiDown {
            payload: Some(DownPayload::ReverseDispatchResult(ReverseDispatchResult {
                call_id: call_id.to_vec(),
                payload,
                terminal: true,
                receipt: None,
                failure,
            })),
            ..InvokeBidiDown::default()
        },
    }
}

/// Terminal-result settlement shared by the JSON `Result` arm and the
/// carrier-v1 `DispatchResult` arm: streaming map first, unary map as
/// fallback, every miss surfaced (DEC-F004 — one settle path, not two).
///
/// Deliberately non-blocking: this runs on the session drain — the
/// only reader of the device's whole `session.open` — so it must
/// never wait on one call's consumer. A stalled streaming consumer
/// costs that call alone (`ConsumerStalled`), not the session.
fn settle_terminal_result(
    pending: &Option<Arc<PendingDispatchMap>>,
    pending_stream: &Option<Arc<PendingStreamDispatchMap>>,
    caller_ura: &str,
    call_id: u64,
    dispatch_result: DispatchResult,
) {
    let mut completed = false;
    if let Some(pending_stream) = pending_stream.as_ref() {
        match pending_stream.try_finish(call_id, dispatch_result.clone()) {
            crate::daemon::invocation::state::pending_dispatch::StreamDeliver::Delivered => {
                completed = true
            }
            crate::daemon::invocation::state::pending_dispatch::StreamDeliver::ConsumerStalled => {
                crate::op_event!(
                    component = session_accept,
                    kind = terminal_result_consumer_stalled,
                    caller = caller_ura,
                    call_id = call_id,
                    note = "terminal dropped; consumer stopped draining its chunks",
                );
                return;
            }
            crate::daemon::invocation::state::pending_dispatch::StreamDeliver::NoMatch => {}
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
    outcome: crate::daemon::invocation::state::pending_dispatch::StreamDeliver,
    caller_ura: &str,
    call_id: u64,
) {
    match outcome {
        crate::daemon::invocation::state::pending_dispatch::StreamDeliver::Delivered => {}
        crate::daemon::invocation::state::pending_dispatch::StreamDeliver::NoMatch => {
            crate::op_event!(
                component = session_accept,
                kind = streaming_result_chunk_no_match,
                caller = caller_ura,
                call_id = call_id,
            );
        }
        crate::daemon::invocation::state::pending_dispatch::StreamDeliver::ConsumerStalled => {
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
/// expected to be a `BinaryChunk` carrying a JSON-serialised
/// `SessionDispatch::Result`; on parse the matching pending entry
/// in the `PendingDispatchMap` is completed so the
/// `runtime.invoke_remote` caller wakes up.
///
/// On stream close (any reason — graceful CloseSend, transport
/// reset, RST_STREAM, peer crash) the device is removed from the
/// presence registry with an appropriate `OfflineReason` so that
/// future `lookup` calls see it as offline immediately.
async fn drain_session_up_stream(
    mut up: Streaming<InvokeBidiUp>,
    caller_ura: String,
    session_id: crate::daemon::invocation::state::presence::PresenceSessionId,
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
            // Carrier-v1 dual-read (DEC-F004 / T2.1 step 2b): proto
            // frames settle through the same core as the JSON shapes.
            Some(UpPayload::DispatchResult(result)) => {
                if result.terminal && result.receipt.is_none() {
                    // The contract REQUIRES a callee-signed receipt on
                    // terminal frames; surface the violation but still
                    // settle so the caller is not left hanging.
                    crate::op_event!(
                        component = session_accept,
                        kind = carrier_v1_result_missing_receipt,
                        caller = caller_ura,
                        call_id = result.call_id,
                    );
                }
                if let Some(receipt) = result.receipt.as_ref() {
                    // Receipt-chain closure at the hub hop: observable
                    // now; hub-ledger projection lands in step 2c.
                    crate::op_event!(
                        component = session_accept,
                        kind = carrier_v1_receipt_received,
                        caller = caller_ura,
                        call_id = result.call_id,
                        receipt_state = receipt.state,
                    );
                }
                let terminal = result.terminal;
                let call_id = result.call_id;
                let mapped = pending_result_from_carrier_v1(&result);
                if terminal {
                    settle_terminal_result(&pending, &pending_stream, &caller_ura, call_id, mapped);
                } else if let Some(pending_stream) = pending_stream.as_ref() {
                    report_chunk_delivery(
                        pending_stream.try_push_chunk(call_id, mapped.payload),
                        &caller_ura,
                        call_id,
                    );
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
                    let outcome = if request
                        .content_envelope
                        .as_ref()
                        .is_some_and(|c| c.encryption != 0)
                    {
                        RequestOutcome::Err {
                            error: SessionRequestError::PermissionDenied {
                                reason: format!(
                                    "session.open: ReverseDispatchCall `{}` carries encrypted \
                                     args but no hub-side request decryptor is wired",
                                    request.function_name
                                ),
                            },
                        }
                    } else {
                        dispatcher_for_request
                            .dispatch_session_request_named(
                                &request.function_name,
                                &request.arguments,
                            )
                            .await
                    };
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
                        receipt: None,
                    };
                    settle_terminal_result(
                        &pending,
                        &pending_stream,
                        &caller_ura,
                        call_id,
                        dispatch_result,
                    );
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
                        pending_stream.try_push_chunk(call_id, payload),
                        &caller_ura,
                        call_id,
                    );
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
                // device's open `session.open` bidi.
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
                let dispatcher_for_request = dispatcher.clone();
                let presence_for_reply = Arc::clone(&presence);
                let caller_ura_for_reply = caller_ura.clone();
                tokio::spawn(async move {
                    let outcome = if args_content_envelope.is_encrypted() {
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
                        dispatcher_for_request
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
/// `forward_invoke` admission already uses (PR-N2 commits
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
        crate::ura::parse_ura(caller_ura).map(|parsed| parsed.kind),
        Ok(crate::ura::URAKind::User)
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

impl InnerPayload {
    pub(crate) fn public_ability_for_target(&self, target_ura: &str) -> Result<String, Status> {
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
// The trade-off: admission-time emission of any audit artefact is
// gone. An admission that succeeds but whose invocation never reaches
// a terminal state leaves no record. Closing that gap is a Week-5+
// topic; for now the operator log is the audit source for
// non-terminal calls.

/// step-3b hub arm (DEC-F004): pick the bidi-open carrier by the
/// execution host's negotiated contract. A v1 host with the caller's
/// seven-tuple envelope on the open frame receives the canonical
/// `DispatchCall{open_bidi}` — envelope transplanted verbatim with the
/// resolver-selected callee, arguments riding as canonical bytes. A v1
/// host WITHOUT an envelope still gets JSON: a canonical frame minus
/// its envelope would be hollow (same doctrine as the unary slot
/// fallback). v0 hosts keep JSON until the deletion window closes it.
pub(crate) fn build_remote_bidi_open_frame_for_contract(
    target_contract_v1: bool,
    call_id: u64,
    selected_route: &SelectedInvokeRoute,
    envelope_open: &EnvelopeOpen,
) -> Result<DispatchFrame, Status> {
    let dispatch_ability = selected_route.dispatch_key();
    match (target_contract_v1, envelope_open.envelope.clone()) {
        (true, Some(envelope)) => Ok(build_carrier_v1_dispatch_frame(
            call_id,
            easynet_axon::pb::axon::v1::InvokeRequest {
                envelope: Some(envelope_with_selected_callee(envelope, selected_route)),
                function_name: selected_route.dispatch_name.clone(),
                arguments: envelope_open.initial_args.clone(),
                ..Default::default()
            },
            true,
        )),
        _ => build_remote_bidi_open_dispatch_frame(
            call_id,
            &selected_route.callee_ura,
            remote_bidi_subject_ura(envelope_open).as_deref(),
            &dispatch_ability,
            &envelope_open.initial_args,
            envelope_open.metadata.clone(),
        ),
    }
}

pub(crate) fn build_remote_bidi_open_dispatch_frame(
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
    let data = frame
        .encode_frame()
        .expect("SessionDispatch::BidiInput is statically encodable");
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

pub(crate) fn remote_bidi_target_ura(envelope_open: &EnvelopeOpen) -> Option<String> {
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

#[cfg(test)]
mod tests {
    use super::*;
    use easynet_axon::pb::axon::v1::SessionOpenExt;

    #[test]
    fn invoke_bidi_gate_recognizes_core_browser_attach_wire() {
        let registry = crate::daemon::ability::wire::AbilityWireRegistry::core();
        let ability =
            crate::daemon::ability::builtins::device_control::browser::ABILITY_ATTACH_SESSION;

        assert!(local_is_bidi_wire_ability(&registry, ability));
        assert_eq!(
            local_bidi_wire_kind_for(&registry, ability),
            Some(LocalBidiWireKind::JsonFrames)
        );
    }

    #[test]
    fn invoke_bidi_gate_recognizes_descriptor_ref_wire_target() {
        let registry = crate::daemon::ability::wire::AbilityWireRegistry::core();
        let ability =
            crate::daemon::ability::builtins::device_control::browser::ABILITY_ATTACH_SESSION;
        let owner_ura = crate::ura::device_ura("test-realm", "dev-a");
        let ability_ura = crate::ura::owner_ability_ura(&owner_ura, ability).unwrap();
        let descriptor_ref = format!(
            "{ability_ura}@{}",
            crate::daemon::ability::DEFAULT_ABILITY_DESCRIPTOR_VERSION
        );

        assert!(local_is_bidi_wire_ability(&registry, &descriptor_ref));
        assert_eq!(
            local_bidi_wire_kind_for(&registry, &descriptor_ref),
            Some(LocalBidiWireKind::JsonFrames)
        );
    }

    #[test]
    fn absent_ext_negotiates_legacy_json() {
        let c = session_contract_from_ext(None);
        assert_eq!(c, SessionContract::legacy());
        assert_eq!(c.version.min(HUB_DISPATCH_CONTRACT_VERSION), 0);
    }

    #[test]
    fn v1_ext_negotiates_proto_and_caps_at_hub_version() {
        let ext = SessionOpenExt {
            contract_version: 7, // future device, older hub
            claimant_boot_nonce: vec![3; 16],
        };
        let c = session_contract_from_ext(Some(&ext));
        assert_eq!(c.version.min(HUB_DISPATCH_CONTRACT_VERSION), 1);
        assert_eq!(c.claimant_boot_nonce.len(), 16);
    }

    #[test]
    fn carrier_v1_failure_maps_to_single_track_projection() {
        let pb = easynet_axon::pb::axon::v1::DispatchResult {
            call_id: 7,
            payload: b"partial".to_vec(),
            terminal: true,
            receipt: None,
            failure: Some(easynet_axon::pb::axon::v1::Error {
                code: "TARGET_OFFLINE".into(),
                message: "device went away".into(),
                retryable: true,
                ..Default::default()
            }),
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
    fn carrier_v1_clean_result_has_no_error_projection() {
        let pb = easynet_axon::pb::axon::v1::DispatchResult {
            call_id: 8,
            payload: b"ok".to_vec(),
            terminal: true,
            receipt: None,
            failure: None,
        };
        let mapped = pending_result_from_carrier_v1(&pb);
        assert!(mapped.error.is_none());
        assert!(mapped.failure.is_none());
    }

    #[test]
    fn reverse_reply_frame_carries_typed_failure_single_track() {
        let frame = build_reverse_dispatch_result_frame(
            [9; 16],
            RequestOutcome::Err {
                error: SessionRequestError::TargetOffline,
            },
        );
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
    fn admission_receipt_carries_session_contract_payload() {
        let frame = build_session_down_admission_receipt(1, 42, true);
        let Some(DownPayload::Receipt(receipt)) = frame.payload else {
            panic!("frame 0 down must be a receipt");
        };
        let body: serde_json::Value = serde_json::from_slice(&receipt.payload).unwrap();
        let sc = &body["session_contract"];
        assert_eq!(sc["version"], 1);
        assert_eq!(sc["dispatch_encoding"], "proto");
        assert_eq!(sc["hub_session_id"], "42");
        assert_eq!(sc["displaced_prior"], true);
    }
}
