// EasyNet CLI — `session.open` device-side LocalAxonSessionDispatcher
// =================================================================
//
// File: src/daemon/invocation/local_session_dispatcher.rs
//
// Device-side `session.open` dispatcher. Canonical product calls arrive as
// typed `DispatchCall` frames containing the original signed
// `InvokeRequest`; only daemon control and bidirectional streaming retain
// the separate JSON session codec.
//
// Author: Silan.Hu <silan.hu@u.nus.edu>
// Copyright (c) 2026 EasyNet. All rights reserved.

use std::collections::HashMap;
use std::sync::{Arc, Mutex};

use axon_sdk::invocation::{BidiInputFrame, BidiInputSender};
use base64::{engine::general_purpose::STANDARD as B64, Engine as _};
use serde_json::{json, Value};
#[cfg(test)]
use tokio::sync::mpsc;
use tokio_util::sync::CancellationToken;

use super::descriptor_binding::{signed_call_mode_from_target, RuntimeBoundAbility};
use super::invocation_wire::{callee_ura_from_envelope, FEDERATION_RESULT_CONTENT_TYPE};
#[cfg(test)]
use crate::daemon::axon_bridge::proof_owner::descriptor_bound_canonical_bytes;
use crate::daemon::invocation::admission::admission_facade::AdmissionFacade;
use crate::daemon::invocation::bidi::session_initiator::{
    SessionDispatchError, SessionFrameDispatcher, SessionUpSender,
};
use crate::daemon::invocation::bidi::session_wire::{call_id_hex, SessionDispatch};
use crate::daemon::invocation::bidi::state::session_failure::SessionFailure;
use crate::daemon::invocation::dispatch::cancellation::RegisteredInvocationLifecycle;
use axon_sdk::pb::axon::v1::invoke_bidi_down::Payload as DownPayload;
use axon_sdk::pb::axon::v1::invoke_bidi_up::Payload as UpPayload;
use axon_sdk::pb::axon::v1::InvokeBidiDown;
#[cfg(test)]
use axon_sdk::pb::axon::v1::{BinaryChunk, InvokeBidiUp};

/// Executes inbound canonical dispatch and streaming-control frames against
/// the daemon's shared Axon `LocalRuntime`.
#[derive(Clone)]
pub struct LocalAxonSessionDispatcher {
    /// PR-N6 C4 device-side correlation table. Populated in
    /// device-mode boot when the daemon also constructs a
    /// `SessionEscalationHandle`; left `None` in hub or `both`
    /// modes (those daemons do not submit reverse requests and so
    /// never receive `RequestResult` frames). When set, inbound
    /// `SessionDispatch::RequestResult` frames are routed here
    /// by `call_id`, completing the awaiting dispatcher future.
    escalation_correlation:
        Option<Arc<crate::daemon::invocation::bidi::session_escalation::EscalationCorrelation>>,
    /// Active same-hub remote bidi sessions keyed by dispatcher
    /// call_id. The hub opens the local bidi on the device, then
    /// subsequent `SessionDispatch::BidiInput` frames route through
    /// this table with ability-specific payload mapping.
    remote_bidi_sessions: Arc<Mutex<HashMap<u64, ActiveRemoteBidi>>>,
    /// Active server-stream sessions keyed by dispatcher call_id.
    /// The hub reuses `BidiInput{eof=true}` as the cancel signal
    /// when a remote stream/SSE consumer disconnects.
    remote_stream_sessions: Arc<Mutex<HashMap<u64, CancellationToken>>>,
    /// Carrier-dispatched invocations use the same explicit cancel-request
    /// registry as local gRPC calls; terminal state still comes only from Axon
    /// finalization.
    lifecycle_cancellations:
        crate::daemon::invocation::dispatch::cancellation::InvocationCancellationRegistry,
    /// Axon runtime that owns ability execution, state transitions, and
    /// receipts for both canonical session dispatch and local bidi opens.
    local_runtime: Option<Arc<axon_sdk::invocation::LocalRuntime>>,
    /// Product-policy coordinator installed in the same runtime admission graph
    /// as `local_runtime`. Production wiring sets the runtime, coordinator, and
    /// policy facade atomically so a destination carrier cannot enter Axon
    /// without the daemon policy stage paired with that runtime.
    runtime_admission: Option<
        Arc<
            crate::daemon::invocation::admission::admission_facade::DaemonRuntimeAdmissionCoordinator,
        >,
    >,
    /// Daemon-owned wire profile registry for local bidi abilities. Plugin
    /// declarations are projected into this table at boot so the dispatcher
    /// does not query package state through process-global helpers.
    ability_wire: Arc<crate::daemon::ability::wire::AbilityWireRegistry>,
    /// On-miss caller key sync for external signed device and same-realm
    /// user callers (see `device_trust_sync`). `None` outside device-mode
    /// boot.
    device_trust_sync:
        Option<Arc<crate::daemon::invocation::admission::device_trust_sync::DeviceTrustSync>>,
    /// RFC-014 runtime policy gate for hub-pushed session dispatch frames.
    /// Transport/session admission proves the carrier can reach this device;
    /// this gate proves the inner ability/subject/action may execute.
    admission: Option<AdmissionFacade>,
    #[cfg(test)]
    canonical_only_test_runtime: bool,
}

type LocalBidiWireKind = crate::daemon::ability::wire::AbilityBidiWireKind;

fn canonical_runtime_assembly_unavailable(context: &str, missing: &str) -> SessionDispatchError {
    SessionDispatchError::Other(format!(
        "{context} requires canonical destination runtime assembly: missing {missing}"
    ))
}

fn receipt_to_session_wire(
    receipt: &axon_sdk::invocation::SignedInvocationReceipt,
) -> Result<axon_sdk::pb::axon::v1::InvocationReceipt, SessionDispatchError> {
    axon_sdk::invocation::wire::receipt_to_wire(receipt).map_err(|error| {
        SessionDispatchError::Other(format!(
            "canonical receipt projection failed before session relay: {error}"
        ))
    })
}

fn unary_checkpoints_to_session_wire(
    outcome: &crate::daemon::axon_bridge::descriptor_bound_dispatch::RpcDispatchOutcome,
) -> Result<
    (
        Option<axon_sdk::pb::axon::v1::InvocationReceipt>,
        Option<axon_sdk::pb::axon::v1::InvocationReceipt>,
    ),
    SessionDispatchError,
> {
    match (
        outcome.admission_receipt.as_ref(),
        outcome.terminal_receipt.as_ref(),
    ) {
        (Some(admission), Some(terminal)) => {
            if admission.state() != axon_sdk::invocation::InvocationState::Admitted {
                return Err(SessionDispatchError::Other(
                    "canonical unary admission checkpoint has a non-admitted state".to_string(),
                ));
            }
            if terminal.state() != outcome.state
                || !matches!(
                    terminal.state(),
                    axon_sdk::invocation::InvocationState::Completed
                        | axon_sdk::invocation::InvocationState::Failed
                        | axon_sdk::invocation::InvocationState::TimedOut
                        | axon_sdk::invocation::InvocationState::Cancelled
                )
            {
                return Err(SessionDispatchError::Other(format!(
                    "canonical unary terminal checkpoint state mismatch: outcome={}, receipt={}",
                    outcome.state.as_str(),
                    terminal.state().as_str(),
                )));
            }
            if admission.invocation_id() != terminal.invocation_id() {
                return Err(SessionDispatchError::Other(
                    "canonical unary checkpoints bind different invocations".to_string(),
                ));
            }
            Ok((
                Some(receipt_to_session_wire(admission)?),
                Some(receipt_to_session_wire(terminal)?),
            ))
        }
        (None, None) if outcome.error.is_some() => Ok((None, None)),
        (None, None) => Err(SessionDispatchError::Other(
            "successful unary carrier result omitted canonical checkpoints".to_string(),
        )),
        _ => Err(SessionDispatchError::Other(
            "unary carrier result requires admission and terminal checkpoints together".to_string(),
        )),
    }
}

fn local_bidi_wire_kind_for(
    registry: &crate::daemon::ability::wire::AbilityWireRegistry,
    ability: &str,
) -> Option<LocalBidiWireKind> {
    registry
        .bidi_wire_kind_for(ability)
        .or_else(|| crate::daemon::ability::wire::core_bidi_wire_kind_for(ability))
}

fn local_is_bidi_wire_ability(
    registry: &crate::daemon::ability::wire::AbilityWireRegistry,
    ability: &str,
) -> bool {
    local_bidi_wire_kind_for(registry, ability).is_some()
}

#[derive(Clone)]
struct ActiveRemoteBidi {
    ability: String,
    sender: BidiInputSender,
}

#[derive(Debug, Clone, PartialEq, Eq)]
struct BidiOutputProjection {
    call_id: u64,
    payload: Vec<u8>,
    failure: Option<SessionFailure>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
struct HandlerErrorFrame {
    code: String,
    message: String,
}

impl HandlerErrorFrame {
    fn parse(value: &Value, frame_label: &'static str) -> Result<Self, SessionDispatchError> {
        let code = required_handler_error_text(value, "code", frame_label)?;
        let message = required_handler_error_text(value, "message", frame_label)?;
        Ok(Self { code, message })
    }

    fn reason(&self) -> String {
        format!("{}: {}", self.code, self.message)
    }

    fn failure(&self) -> SessionFailure {
        SessionFailure::from_explicit(&self.code, self.reason(), false)
    }
}

fn required_handler_error_text(
    value: &Value,
    field: &'static str,
    frame_label: &'static str,
) -> Result<String, SessionDispatchError> {
    let Some(raw) = value.get(field).and_then(Value::as_str) else {
        return Err(SessionDispatchError::Other(format!(
            "{frame_label} requires non-empty `{field}`"
        )));
    };
    let trimmed = raw.trim();
    if trimmed.is_empty() {
        return Err(SessionDispatchError::Other(format!(
            "{frame_label} requires non-empty `{field}`"
        )));
    }
    Ok(trimmed.to_string())
}

fn carrier_v1_control_failure(
    call_id: u64,
    code: &'static str,
    message: impl Into<String>,
) -> axon_sdk::pb::axon::v1::DispatchResult {
    axon_sdk::pb::axon::v1::DispatchResult {
        call_id,
        payload: Vec::new(),
        terminal: false,
        failure: Some(axon_sdk::pb::axon::v1::Error {
            code: code.to_string(),
            message: message.into(),
            retryable: false,
            ..Default::default()
        }),
        ..Default::default()
    }
}

impl LocalAxonSessionDispatcher {
    /// Canonical dispatch: the frame already is the invocation, so neither
    /// caller identity nor request fields are reconstructed at this hop.
    async fn handle_carrier_v1_dispatch(
        &self,
        call: axon_sdk::pb::axon::v1::DispatchCall,
        outbound: &SessionUpSender,
    ) -> Result<(), SessionDispatchError> {
        use axon_sdk::pb::axon::v1::DispatchResult as PbDispatchResult;

        let call_id = call.call_id;
        if !outbound.carrier_v1() {
            return Err(SessionDispatchError::Other(
                "DispatchCall requires negotiated session carrier v1".to_string(),
            ));
        }
        let Some(request) = call.request else {
            return Err(SessionDispatchError::Other(
                "carrier-v1 DispatchCall without request".to_string(),
            ));
        };
        if call.open_bidi {
            return self
                .handle_carrier_v1_bidi_open(call_id, request, outbound)
                .await;
        }
        let function_name = match crate::daemon::invocation::dispatch::invocation_wire::function_name_from_invocation_target(
                "carrier-v1 DispatchCall",
                request.target.as_ref(),
            ) {
            Ok(function_name) => function_name.to_string(),
            Err(status) => {
                return Self::send_carrier_v1_control_failure(
                    outbound,
                    call_id,
                    "CARRIER_TARGET_INVALID",
                    status.message(),
                )
                .await;
            }
        };
        crate::op_event!(
            component = local_session_dispatcher,
            kind = received_carrier_v1_dispatch,
            call_id = call_id,
            ability = function_name,
        );
        let Some(envelope) = request.envelope else {
            return Self::send_carrier_v1_control_failure(
                outbound,
                call_id,
                "ENVELOPE_INCOMPLETE",
                "carrier-v1 DispatchCall request missing envelope",
            )
            .await;
        };
        let runtime = match self.require_local_runtime("carrier-v1 dispatch") {
            Ok(runtime) => runtime,
            Err(error) => {
                return Self::send_carrier_v1_control_failure(
                    outbound,
                    call_id,
                    "RUNTIME_UNAVAILABLE",
                    error.to_string(),
                )
                .await;
            }
        };
        let target_ura = match callee_ura_from_envelope(Some(&envelope), "carrier-v1 DispatchCall")
        {
            Ok(target_ura) => target_ura,
            Err(status) => {
                return Self::send_carrier_v1_control_failure(
                    outbound,
                    call_id,
                    "ENVELOPE_INCOMPLETE",
                    status.message(),
                )
                .await;
            }
        };
        if let Err(error) = self.sync_external_signed_caller_key(&envelope).await {
            return Self::send_carrier_v1_control_failure(
                outbound,
                call_id,
                "CALLER_KEY_SYNC_FAILED",
                error.to_string(),
            )
            .await;
        }
        let bound_ability = match RuntimeBoundAbility::from_wire_target(
            "carrier-v1 DispatchCall",
            &runtime,
            &target_ura,
            &function_name,
        )
        .await
        {
            Ok(bound_ability) => bound_ability,
            Err(status) => {
                return Self::send_carrier_v1_control_failure(
                    outbound,
                    call_id,
                    "ABILITY_RESOLUTION_FAILED",
                    status.message(),
                )
                .await;
            }
        };
        let call_mode = match signed_call_mode_from_target(
            "carrier-v1 DispatchCall",
            &target_ura,
            request.target.as_ref(),
        ) {
            Ok(call_mode) => call_mode,
            Err(status) => {
                return Self::send_carrier_v1_control_failure(
                    outbound,
                    call_id,
                    "DESCRIPTOR_BINDING_FAILED",
                    status.message(),
                )
                .await;
            }
        };
        let descriptor_ref = match bound_ability.signed_descriptor_ref_from_target(
            "carrier-v1 DispatchCall",
            &target_ura,
            call_mode,
            request.target.as_ref(),
        ) {
            Ok(descriptor_ref) => descriptor_ref,
            Err(status) => {
                return Self::send_carrier_v1_control_failure(
                    outbound,
                    call_id,
                    "DESCRIPTOR_BINDING_FAILED",
                    status.message(),
                )
                .await;
            }
        };
        let wire = match crate::daemon::axon_bridge::descriptor_bound_dispatch::external_signed_from_wire_parts(
                envelope,
                descriptor_ref.into_descriptor_ref(),
                request.arguments,
                request.metadata,
            ) {
            Ok(wire) => wire,
            Err(error) => {
                return Self::send_carrier_v1_control_failure(
                    outbound,
                    call_id,
                    "DISPATCH_WIRE_INVALID",
                    format!("build carrier-v1 signed dispatch: {error}"),
                )
                .await;
            }
        };
        let runtime_admission = match self.stage_runtime_admission(&wire, &function_name, call_mode)
        {
            Ok(runtime_admission) => runtime_admission,
            Err(error) => {
                return Self::send_carrier_v1_control_failure(
                    outbound,
                    call_id,
                    "RUNTIME_ADMISSION_FAILED",
                    error.to_string(),
                )
                .await;
            }
        };

        // ── step-3c: server-stream over carrier ──────────────────────
        // A stream-mode ability (modes.stream && !modes.rpc) emits many
        // non-terminal frames; draining it through the unary path below
        // would collapse the stream to a single terminal DispatchResult.
        // Open the stream and hand the handle to a forwarder that chains
        // typed `DispatchResult` chunks.
        // Carrier-v1 preserves caller identity through the descriptor-bound
        // signature.
        if matches!(call_mode, axon_sdk::invocation::CallMode::Stream) {
            return self
                .handle_carrier_v1_stream_open(call_id, wire, runtime_admission, outbound)
                .await;
        }

        let outcome = crate::daemon::axon_bridge::descriptor_bound_dispatch::dispatch_rpc_admitted(
            &runtime,
            wire,
            &self.lifecycle_cancellations,
        )
        .await;
        if outcome.invocation_id.is_some() {
            Self::commit_runtime_admission(runtime_admission)?;
        }

        let failure = outcome
            .error
            .as_ref()
            .map(|e| axon_sdk::pb::axon::v1::Error {
                code: if e.reason.is_empty() {
                    "INVOCATION_FAILED".to_string()
                } else {
                    e.reason.clone()
                },
                message: e.to_string(),
                retryable: false,
                ..Default::default()
            });
        let (admission_receipt, terminal_receipt) = unary_checkpoints_to_session_wire(&outcome)?;
        let reply = PbDispatchResult {
            call_id,
            payload: outcome.payload_bytes,
            result_content_type: FEDERATION_RESULT_CONTENT_TYPE.to_string(),
            terminal: true,
            admission_receipt,
            terminal_receipt,
            failure,
        };
        outbound
            .send_payload(UpPayload::DispatchResult(reply))
            .await
            .map_err(|_| SessionDispatchError::Other("session up channel closed".to_string()))?;
        Ok(())
    }

    /// step-3c — open a server-stream ability over the carrier-v1
    /// transport and forward its frames as a chain of `DispatchResult`
    /// chunks. Product policy is staged with `wire` and evaluated by the
    /// runtime's receipt-provider boundary; an open failure is reported as a
    /// non-terminal carrier control failure.
    async fn handle_carrier_v1_stream_open(
        &self,
        call_id: u64,
        wire: crate::daemon::axon_bridge::descriptor_bound_dispatch::WireDispatch,
        runtime_admission: Option<
            crate::daemon::invocation::admission::admission_facade::DaemonRuntimeAdmissionLease,
        >,
        outbound: &SessionUpSender,
    ) -> Result<(), SessionDispatchError> {
        let runtime = self.require_local_runtime("carrier-v1 stream")?;
        let lifecycle_envelope = wire.envelope.clone();
        let handle =
            match crate::daemon::axon_bridge::descriptor_bound_dispatch::open_stream_admitted(
                &runtime, wire,
            )
            .await
            {
                Ok(handle) => handle,
                Err(err) => {
                    let reply =
                        carrier_v1_control_failure(call_id, "STREAM_OPEN_FAILED", err.to_string());
                    outbound
                        .send_payload(UpPayload::DispatchResult(reply))
                        .await
                        .map_err(|_| {
                            SessionDispatchError::Other("session up channel closed".to_string())
                        })?;
                    return Ok(());
                }
            };
        let lifecycle = match RegisteredInvocationLifecycle::register(
            self.lifecycle_cancellations.clone(),
            &lifecycle_envelope,
            handle.handle().clone(),
        ) {
            Ok(lifecycle) => lifecycle,
            Err(err) => {
                let _ = handle
                    .cancel("lifecycle cancellation registration failed")
                    .await;
                let _ = handle.finalized().await;
                let reply = carrier_v1_control_failure(
                    call_id,
                    "CANONICAL_CANCELLATION_REGISTRATION_FAILED",
                    err.to_string(),
                );
                outbound
                    .send_payload(UpPayload::DispatchResult(reply))
                    .await
                    .map_err(|_| {
                        SessionDispatchError::Other("session up channel closed".to_string())
                    })?;
                return Ok(());
            }
        };
        if let Err(error) = Self::commit_runtime_admission(runtime_admission) {
            let _ = lifecycle
                .cancel_and_finalize("runtime admission commit failed")
                .await;
            return Err(error);
        }

        Self::spawn_carrier_v1_stream_forwarder(
            call_id,
            handle,
            outbound.clone(),
            Arc::clone(&self.remote_stream_sessions),
            lifecycle,
        );
        Ok(())
    }

    /// Forward a canonical stream through protobuf `DispatchResult` frames
    /// while preserving runtime cancellation and lifecycle registration.
    /// The terminal frame carries the callee-signed execution receipt
    /// (`DispatchResult.terminal_receipt` is REQUIRED on terminal frames), pulled
    /// from the streaming handle the same way the unary arm projects
    /// `terminal_receipt`.
    fn spawn_carrier_v1_stream_forwarder(
        call_id: u64,
        mut handle: axon_sdk::invocation::StreamingInvocationHandle,
        outbound: SessionUpSender,
        sessions: Arc<Mutex<HashMap<u64, CancellationToken>>>,
        lifecycle: RegisteredInvocationLifecycle,
    ) {
        use axon_sdk::pb::axon::v1::DispatchResult as PbDispatchResult;

        let cancel = CancellationToken::new();
        {
            let mut guard = match sessions.lock() {
                Ok(g) => g,
                Err(poisoned) => poisoned.into_inner(),
            };
            guard.insert(call_id, cancel.clone());
        }
        tokio::spawn(async move {
            let mut sent_terminal = false;
            let mut cancelled = false;
            let admission = match handle.admission_receipt().await {
                Ok(receipt) => receipt,
                Err(error) => {
                    let _ = lifecycle.finalized().await;
                    let _ = outbound
                        .send_payload(UpPayload::DispatchResult(carrier_v1_control_failure(
                            call_id,
                            "CANONICAL_ADMISSION_REQUIRED",
                            error.to_string(),
                        )))
                        .await;
                    return;
                }
            };
            let admission_wire = match receipt_to_session_wire(&admission) {
                Ok(receipt) => receipt,
                Err(error) => {
                    let _ = lifecycle
                        .cancel_and_finalize("canonical admission projection failed")
                        .await;
                    let _ = outbound
                        .send_payload(UpPayload::DispatchResult(carrier_v1_control_failure(
                            call_id,
                            "CANONICAL_ADMISSION_PROJECTION_FAILED",
                            error.to_string(),
                        )))
                        .await;
                    return;
                }
            };
            if outbound
                .send_payload(UpPayload::DispatchResult(PbDispatchResult {
                    call_id,
                    terminal: false,
                    admission_receipt: Some(admission_wire),
                    ..PbDispatchResult::default()
                }))
                .await
                .is_err()
            {
                let _ = lifecycle
                    .cancel_and_finalize("session stream closed before admission")
                    .await;
                return;
            }
            loop {
                let frame_result = tokio::select! {
                    _ = cancel.cancelled() => {
                        cancelled = true;
                        break;
                    }
                    next = handle.next_frame() => {
                        let Some(frame_result) = next else {
                            break;
                        };
                        frame_result
                    }
                };
                let reply = match frame_result {
                    Ok(frame) => {
                        let terminal = frame.terminal;
                        crate::op_event!(
                            component = local_session_dispatcher,
                            kind = forwarding_stream_frame_up_carrier_v1,
                            call_id = call_id,
                            payload_bytes = frame.payload.len(),
                            terminal = terminal,
                        );
                        sent_terminal = sent_terminal || terminal;
                        let finalized = if terminal {
                            match lifecycle.finalized().await {
                                Ok(finalized) => Some(finalized),
                                Err(error) => {
                                    let _ = outbound
                                        .send_payload(UpPayload::DispatchResult(
                                            carrier_v1_control_failure(
                                                call_id,
                                                "CANONICAL_FINALIZATION_REQUIRED",
                                                error.to_string(),
                                            ),
                                        ))
                                        .await;
                                    break;
                                }
                            }
                        } else {
                            None
                        };
                        let terminal_receipt = match finalized.as_ref() {
                            Some(value) => match receipt_to_session_wire(&value.terminal_receipt) {
                                Ok(receipt) => Some(receipt),
                                Err(error) => {
                                    let _ = outbound
                                        .send_payload(UpPayload::DispatchResult(
                                            carrier_v1_control_failure(
                                                call_id,
                                                "CANONICAL_TERMINAL_PROJECTION_FAILED",
                                                error.to_string(),
                                            ),
                                        ))
                                        .await;
                                    break;
                                }
                            },
                            None => None,
                        };
                        PbDispatchResult {
                            call_id,
                            payload: finalized
                                .as_ref()
                                .map(|value| value.output().to_vec())
                                .unwrap_or(frame.payload),
                            result_content_type: finalized
                                .as_ref()
                                .map(|value| value.output_content_type().to_string())
                                .unwrap_or_else(|| frame.content_type.clone()),
                            terminal,
                            terminal_receipt,
                            failure: finalized
                                .as_ref()
                                .and_then(|value| value.failure.as_ref())
                                .map(axon_sdk::invocation::wire::error_to_wire),
                            ..PbDispatchResult::default()
                        }
                    }
                    Err(err) => {
                        sent_terminal = true;
                        let finalized = match lifecycle.finalized().await {
                            Ok(finalized) => finalized,
                            Err(error) => {
                                let _ = outbound
                                    .send_payload(UpPayload::DispatchResult(
                                        carrier_v1_control_failure(
                                            call_id,
                                            "CANONICAL_FINALIZATION_REQUIRED",
                                            format!(
                                                "frame_error={err}; finalization_error={error}"
                                            ),
                                        ),
                                    ))
                                    .await;
                                return;
                            }
                        };
                        let terminal_receipt =
                            match receipt_to_session_wire(&finalized.terminal_receipt) {
                                Ok(receipt) => receipt,
                                Err(error) => {
                                    let _ = outbound
                                        .send_payload(UpPayload::DispatchResult(
                                            carrier_v1_control_failure(
                                                call_id,
                                                "CANONICAL_TERMINAL_PROJECTION_FAILED",
                                                error.to_string(),
                                            ),
                                        ))
                                        .await;
                                    return;
                                }
                            };
                        PbDispatchResult {
                            call_id,
                            payload: Vec::new(),
                            terminal: true,
                            terminal_receipt: Some(terminal_receipt),
                            failure: Some(axon_sdk::invocation::wire::error_to_wire(
                                finalized.failure.as_ref().unwrap_or(&err),
                            )),
                            ..PbDispatchResult::default()
                        }
                    }
                };
                let terminal = reply.terminal;
                let send = outbound
                    .send_payload(UpPayload::DispatchResult(reply))
                    .await;
                if send.is_err() || terminal {
                    break;
                }
            }
            if !sent_terminal && !cancelled {
                let message = "stream ended without terminal frame";
                let reply =
                    carrier_v1_control_failure(call_id, "STREAM_ENDED_WITHOUT_TERMINAL", message);
                let _ = outbound
                    .send_payload(UpPayload::DispatchResult(reply))
                    .await;
            }
            // Cancellation must reach the RUNTIME task, not just this
            // forwarder — dropping the handle alone leaves the ability's
            // emit loop alive holding its stream source. cancel() is
            // idempotent and a no-op on already-terminal invocations.
            if !sent_terminal {
                if let Err(err) = lifecycle.cancel_and_finalize("session stream closed").await {
                    let err_msg = err.to_string();
                    crate::op_event!(
                        component = local_session_dispatcher,
                        kind = stream_runtime_cancel_failed,
                        call_id = call_id,
                        error = err_msg,
                    );
                }
            }
            let mut guard = match sessions.lock() {
                Ok(g) => g,
                Err(poisoned) => poisoned.into_inner(),
            };
            guard.remove(&call_id);
        });
    }

    fn is_json_frame_bidi(&self, ability: &str) -> bool {
        Self::is_json_frame_bidi_with(&self.ability_wire, ability)
    }

    fn is_json_frame_bidi_with(
        registry: &crate::daemon::ability::wire::AbilityWireRegistry,
        ability: &str,
    ) -> bool {
        matches!(
            local_bidi_wire_kind_for(registry, ability),
            Some(LocalBidiWireKind::JsonFrames)
        )
    }

    /// Construct the device-side session dispatcher.
    #[must_use]
    pub fn new() -> Self {
        Self {
            escalation_correlation: None,
            remote_bidi_sessions: Arc::new(Mutex::new(HashMap::new())),
            remote_stream_sessions: Arc::new(Mutex::new(HashMap::new())),
            lifecycle_cancellations: Default::default(),
            local_runtime: None,
            runtime_admission: None,
            ability_wire: Arc::new(crate::daemon::ability::wire::AbilityWireRegistry::core()),
            device_trust_sync: None,
            admission: None,
            #[cfg(test)]
            canonical_only_test_runtime: false,
        }
    }

    /// Builder seam: attach the daemon-owned wire registry computed from the
    /// same plugin runtime state used for ability registration.
    #[must_use]
    pub fn with_ability_wire_registry(
        mut self,
        registry: Arc<crate::daemon::ability::wire::AbilityWireRegistry>,
    ) -> Self {
        self.ability_wire = registry;
        self
    }

    #[must_use]
    pub fn with_device_trust_sync(
        mut self,
        sync: Arc<crate::daemon::invocation::admission::device_trust_sync::DeviceTrustSync>,
    ) -> Self {
        self.device_trust_sync = Some(sync);
        self
    }

    async fn sync_external_signed_caller_key(
        &self,
        envelope: &axon_sdk::pb::axon::v1::Envelope,
    ) -> Result<(), SessionDispatchError> {
        let Some(caller_ura) = envelope
            .caller
            .as_ref()
            .map(|caller| caller.ura.as_str())
            .map(str::trim)
            .filter(|caller| !caller.is_empty())
        else {
            return Ok(());
        };
        let Ok(parsed) = crate::core::ura::parse_ura(caller_ura) else {
            return Ok(());
        };
        if !matches!(
            parsed.kind,
            crate::core::ura::URAKind::Device | crate::core::ura::URAKind::User
        ) {
            return Ok(());
        }
        if self.admission.is_none() {
            return Ok(());
        }
        let Some(sync) = self.device_trust_sync.as_ref() else {
            return Err(canonical_runtime_assembly_unavailable(
                &format!("carrier-v1 external signed caller `{caller_ura}` trust sync"),
                "DeviceTrustSync",
            ));
        };
        let presented_pubkey_b64 = envelope
            .caller_signature
            .as_ref()
            .map(|signature| signature.key_id_hint.trim())
            .filter(|key| !key.is_empty());
        let status = sync
            .ensure_caller_key_status(caller_ura, presented_pubkey_b64)
            .await;
        if status.trusted() {
            return Ok(());
        }
        let diagnostic = status
            .diagnostic()
            .unwrap_or_else(|| "trust sync did not produce a trusted key".to_string());
        Err(SessionDispatchError::Other(format!(
            "carrier-v1 external signed caller `{caller_ura}` is not trusted after resolve_key sync: {diagnostic}"
        )))
    }

    /// Builder seam: attach a device-mode escalation correlation
    /// table so inbound `RequestResult` frames complete the
    /// matching pending dispatcher future. Boot calls this in
    /// device-mode only.
    #[must_use]
    pub fn with_escalation_correlation(
        mut self,
        correlation: Arc<
            crate::daemon::invocation::bidi::session_escalation::EscalationCorrelation,
        >,
    ) -> Self {
        self.escalation_correlation = Some(correlation);
        self
    }

    /// Atomically attach the shared Axon runtime and the daemon policy graph
    /// installed in that exact runtime. Destination carrier dispatch requires
    /// this complete assembly; a runtime-only production path is invalid.
    #[must_use]
    pub(crate) fn with_runtime_admission(
        mut self,
        assembly: crate::daemon::axon_bridge::runtime_factory::DaemonRuntimeAssembly,
        admission: AdmissionFacade,
    ) -> Self {
        self.local_runtime = Some(assembly.runtime());
        self.runtime_admission = Some(assembly.admission_graph().runtime_admission());
        self.admission = Some(admission);
        self
    }

    /// Explicit canonical-only runtime seam for unit tests that exercise Axon
    /// carrier mechanics independently of daemon runtime admission.
    #[cfg(test)]
    #[must_use]
    pub fn with_local_runtime(mut self, runtime: Arc<axon_sdk::invocation::LocalRuntime>) -> Self {
        self.local_runtime = Some(runtime);
        self.canonical_only_test_runtime = true;
        self
    }

    fn require_local_runtime(
        &self,
        context: &str,
    ) -> Result<Arc<axon_sdk::invocation::LocalRuntime>, SessionDispatchError> {
        self.local_runtime
            .clone()
            .ok_or_else(|| canonical_runtime_assembly_unavailable(context, "LocalRuntime"))
    }

    fn stage_runtime_admission(
        &self,
        wire: &crate::daemon::axon_bridge::descriptor_bound_dispatch::WireDispatch,
        ability: &str,
        call_mode: axon_sdk::invocation::CallMode,
    ) -> Result<
        Option<crate::daemon::invocation::admission::admission_facade::DaemonRuntimeAdmissionLease>,
        SessionDispatchError,
    > {
        match (&self.admission, &self.runtime_admission) {
            (Some(admission), Some(runtime_admission)) => runtime_admission
                .stage(admission, wire, ability, call_mode)
                .map(Some)
                .map_err(|status| {
                    SessionDispatchError::Other(format!(
                        "carrier-v1 destination runtime admission staging failed: {status}"
                    ))
                }),
            _ => {
                #[cfg(test)]
                if self.canonical_only_test_runtime
                    && self.admission.is_none()
                    && self.runtime_admission.is_none()
                {
                    return Ok(None);
                }
                Err(SessionDispatchError::Other(
                    "carrier-v1 destination dispatch requires canonical runtime admission graph"
                        .to_string(),
                ))
            }
        }
    }

    fn commit_runtime_admission(
        admission: Option<
            crate::daemon::invocation::admission::admission_facade::DaemonRuntimeAdmissionLease,
        >,
    ) -> Result<(), SessionDispatchError> {
        let Some(admission) = admission else {
            return Ok(());
        };
        admission.commit().map(|_| ()).map_err(|status| {
            SessionDispatchError::Other(format!(
                "carrier-v1 destination runtime admission commit failed: {status}"
            ))
        })
    }

    fn map_remote_file_transfer_output(
        call_id: u64,
        value: &Value,
    ) -> Result<Option<BidiOutputProjection>, SessionDispatchError> {
        match value.get("type").and_then(Value::as_str) {
            Some("chunk") => {
                let data_b64 = value.get("data").and_then(Value::as_str).ok_or_else(|| {
                    SessionDispatchError::Other(
                        "file_transfer chunk frame missing `data`".to_string(),
                    )
                })?;
                let raw = B64.decode(data_b64).map_err(|err| {
                    SessionDispatchError::Other(format!(
                        "file_transfer chunk base64 decode failed: {err}"
                    ))
                })?;
                Ok(Some(BidiOutputProjection {
                    call_id,
                    payload: raw,
                    failure: None,
                }))
            }
            Some("complete") => {
                let payload = serde_json::to_vec(value).map_err(|err| {
                    SessionDispatchError::Other(format!(
                        "encode file_transfer completion payload: {err}"
                    ))
                })?;
                Ok(Some(BidiOutputProjection {
                    call_id,
                    payload,
                    failure: None,
                }))
            }
            Some("error") => {
                let error = HandlerErrorFrame::parse(value, "file_transfer error frame")?;
                let payload = serde_json::to_vec(value).map_err(|err| {
                    SessionDispatchError::Other(format!(
                        "encode file_transfer error payload: {err}"
                    ))
                })?;
                Ok(Some(BidiOutputProjection {
                    call_id,
                    payload,
                    failure: Some(error.failure()),
                }))
            }
            Some("warn") => Ok(None),
            Some(other) => Err(SessionDispatchError::Other(format!(
                "unknown file_transfer handler frame type {other:?}"
            ))),
            None => Ok(None),
        }
    }

    fn map_remote_pty_output(
        call_id: u64,
        value: &Value,
    ) -> Result<Option<BidiOutputProjection>, SessionDispatchError> {
        match value.get("type").and_then(Value::as_str) {
            Some("stdout") => {
                let data_b64 = value.get("data").and_then(Value::as_str).ok_or_else(|| {
                    SessionDispatchError::Other("pty stdout frame missing `data`".to_string())
                })?;
                let raw = B64.decode(data_b64).map_err(|err| {
                    SessionDispatchError::Other(format!("pty stdout base64 decode failed: {err}"))
                })?;
                Ok(Some(BidiOutputProjection {
                    call_id,
                    payload: raw,
                    failure: None,
                }))
            }
            Some("exit") => Ok(Some(BidiOutputProjection {
                call_id,
                payload: Vec::new(),
                failure: None,
            })),
            Some("warn") => Ok(None),
            Some(other) => Err(SessionDispatchError::Other(format!(
                "unknown pty handler frame type {other:?}"
            ))),
            None => Ok(None),
        }
    }

    #[cfg(all(test, feature = "remote-desktop"))]
    fn map_remote_bidi_output(
        &self,
        call_id: u64,
        ability: &str,
        value: &Value,
    ) -> Result<Option<BidiOutputProjection>, SessionDispatchError> {
        Self::map_remote_bidi_output_with(&self.ability_wire, call_id, ability, value)
    }

    fn map_remote_bidi_output_with(
        registry: &crate::daemon::ability::wire::AbilityWireRegistry,
        call_id: u64,
        ability: &str,
        value: &Value,
    ) -> Result<Option<BidiOutputProjection>, SessionDispatchError> {
        if ability == crate::daemon::ability::builtins::device_control::terminal::attach::ABILITY_TERMINAL_ATTACH {
            return Self::map_remote_pty_output(call_id, value);
        }
        if Self::is_json_frame_bidi_with(registry, ability) {
            let frame_type = value.get("type").and_then(Value::as_str);
            let payload = serde_json::to_vec(value).map_err(|err| {
                SessionDispatchError::Other(format!("plugin JSON-frame bidi encode failed: {err}"))
            })?;
            let failure = if frame_type == Some("error") {
                Some(HandlerErrorFrame::parse(value, "JSON-frame bidi error frame")?.failure())
            } else {
                None
            };
            return Ok(Some(BidiOutputProjection {
                call_id,
                payload,
                failure,
            }));
        }
        Self::map_remote_file_transfer_output(call_id, value)
    }

    /// Carrier-v1 bidi open: the request is the complete signed Invocation, so
    /// the open enters the same descriptor-bound runtime path as unary calls.
    async fn handle_carrier_v1_bidi_open(
        &self,
        call_id: u64,
        request: axon_sdk::pb::axon::v1::InvokeRequest,
        outbound: &SessionUpSender,
    ) -> Result<(), SessionDispatchError> {
        let ability =
            crate::daemon::invocation::dispatch::invocation_wire::function_name_from_invocation_target(
                "carrier-v1 bidi open",
                request.target.as_ref(),
            )
            .map_err(|status| SessionDispatchError::Other(status.message().to_string()))?
            .to_string();
        crate::op_event!(
            component = local_session_dispatcher,
            kind = received_carrier_v1_bidi_open,
            call_id = call_id,
            ability = ability,
        );
        if !local_is_bidi_wire_ability(&self.ability_wire, &ability) {
            return Self::send_bidi_control_failure(
                outbound,
                call_id,
                "ABILITY_BIDI_NOT_SUPPORTED",
                format!(
                    "remote bidi ability `{ability}` is not published for session.open carrier-v1"
                ),
            )
            .await;
        }
        let Some(envelope) = request.envelope else {
            return Self::send_bidi_control_failure(
                outbound,
                call_id,
                "ENVELOPE_INCOMPLETE",
                "carrier-v1 bidi open missing envelope",
            )
            .await;
        };
        let runtime = match self.require_local_runtime("session.open remote bidi") {
            Ok(runtime) => runtime,
            Err(error) => {
                return Self::send_bidi_control_failure(
                    outbound,
                    call_id,
                    "RUNTIME_UNAVAILABLE",
                    error.to_string(),
                )
                .await;
            }
        };
        let target_ura = match callee_ura_from_envelope(Some(&envelope), "carrier-v1 BidiOpen") {
            Ok(target_ura) => target_ura,
            Err(status) => {
                return Self::send_bidi_control_failure(
                    outbound,
                    call_id,
                    "ENVELOPE_INCOMPLETE",
                    status.message(),
                )
                .await;
            }
        };
        if let Err(err) = self.sync_external_signed_caller_key(&envelope).await {
            return Self::send_bidi_control_failure(
                outbound,
                call_id,
                "CALLER_KEY_SYNC_FAILED",
                err.to_string(),
            )
            .await;
        }
        let bound_ability = match RuntimeBoundAbility::from_wire_target(
            "carrier-v1 BidiOpen",
            &runtime,
            &target_ura,
            &ability,
        )
        .await
        {
            Ok(bound_ability) => bound_ability,
            Err(status) => {
                return Self::send_bidi_control_failure(
                    outbound,
                    call_id,
                    "ABILITY_RESOLUTION_FAILED",
                    status.message(),
                )
                .await;
            }
        };
        let descriptor_ref = match bound_ability.signed_descriptor_ref_from_target(
            "carrier-v1 BidiOpen",
            &target_ura,
            axon_sdk::invocation::CallMode::Bidi,
            request.target.as_ref(),
        ) {
            Ok(ref_) => ref_,
            Err(status) => {
                return Self::send_bidi_control_failure(
                    outbound,
                    call_id,
                    "DESCRIPTOR_BINDING_FAILED",
                    status.message(),
                )
                .await;
            }
        };
        let wire = match crate::daemon::axon_bridge::descriptor_bound_dispatch::external_signed_from_wire_parts(
            envelope,
            descriptor_ref.into_descriptor_ref(),
            request.arguments,
            request.metadata,
        ) {
            Ok(wire) => wire,
            Err(err) => {
                return Self::send_bidi_control_failure(
                    outbound,
                    call_id,
                    "INVOCATION_WIRE_INVALID",
                    format!("build carrier-v1 admitted bidi open: {err}"),
                )
                .await;
            }
        };
        let runtime_admission = match self.stage_runtime_admission(
            &wire,
            &ability,
            axon_sdk::invocation::CallMode::Bidi,
        ) {
            Ok(admission) => admission,
            Err(err) => {
                return Self::send_bidi_control_failure(
                    outbound,
                    call_id,
                    "PRODUCT_ADMISSION_REJECTED",
                    err.to_string(),
                )
                .await;
            }
        };
        let lifecycle_envelope = wire.envelope.clone();
        let handle =
            match crate::daemon::axon_bridge::descriptor_bound_dispatch::open_bidi_admitted(
                &runtime, wire,
            )
            .await
            {
                Ok(handle) => handle,
                Err(err) => {
                    return Self::send_bidi_control_failure(
                        outbound,
                        call_id,
                        "BIDI_OPEN_REJECTED",
                        format!("session.open: remote bidi open failed: {err}"),
                    )
                    .await;
                }
            };
        if let Err(err) = Self::commit_runtime_admission(runtime_admission) {
            return Self::cancel_opened_bidi(
                outbound,
                call_id,
                handle,
                format!("runtime admission commit failed: {err}"),
            )
            .await;
        }
        self.register_remote_bidi(call_id, &ability, handle, outbound, lifecycle_envelope)
            .await
    }

    async fn send_carrier_v1_dispatch_result(
        outbound: &SessionUpSender,
        result: axon_sdk::pb::axon::v1::DispatchResult,
    ) -> Result<(), SessionDispatchError> {
        if !outbound.carrier_v1() {
            return Err(SessionDispatchError::Other(
                "canonical dispatch result requires negotiated session carrier v1".to_string(),
            ));
        }
        outbound
            .send_payload(UpPayload::DispatchResult(result))
            .await
            .map_err(|_| SessionDispatchError::Other("session up channel closed".to_string()))
    }

    async fn send_carrier_v1_control_failure(
        outbound: &SessionUpSender,
        call_id: u64,
        code: &'static str,
        message: impl Into<String>,
    ) -> Result<(), SessionDispatchError> {
        Self::send_carrier_v1_dispatch_result(
            outbound,
            carrier_v1_control_failure(call_id, code, message),
        )
        .await
    }

    async fn send_bidi_control_failure(
        outbound: &SessionUpSender,
        call_id: u64,
        code: &'static str,
        message: impl Into<String>,
    ) -> Result<(), SessionDispatchError> {
        Self::send_carrier_v1_control_failure(outbound, call_id, code, message).await
    }

    async fn send_bidi_admission(
        outbound: &SessionUpSender,
        call_id: u64,
        receipt: &axon_sdk::invocation::SignedInvocationReceipt,
    ) -> Result<(), SessionDispatchError> {
        Self::send_carrier_v1_dispatch_result(
            outbound,
            axon_sdk::pb::axon::v1::DispatchResult {
                call_id,
                admission_receipt: Some(receipt_to_session_wire(receipt)?),
                ..Default::default()
            },
        )
        .await
    }

    async fn send_bidi_progress(
        outbound: &SessionUpSender,
        projection: BidiOutputProjection,
    ) -> Result<(), SessionDispatchError> {
        let failure = projection
            .failure
            .map(|failure| axon_sdk::pb::axon::v1::Error {
                code: failure.code,
                message: failure.message,
                retryable: failure.retryable,
                ..Default::default()
            });
        Self::send_carrier_v1_dispatch_result(
            outbound,
            axon_sdk::pb::axon::v1::DispatchResult {
                call_id: projection.call_id,
                payload: projection.payload,
                failure,
                ..Default::default()
            },
        )
        .await
    }

    async fn send_bidi_terminal(
        outbound: &SessionUpSender,
        call_id: u64,
        finalized: &axon_sdk::invocation::FinalizedInvocation,
    ) -> Result<(), SessionDispatchError> {
        Self::send_carrier_v1_dispatch_result(
            outbound,
            axon_sdk::pb::axon::v1::DispatchResult {
                call_id,
                payload: finalized.output().to_vec(),
                result_content_type: finalized.output_content_type().to_string(),
                terminal: true,
                terminal_receipt: Some(receipt_to_session_wire(&finalized.terminal_receipt)?),
                failure: finalized
                    .failure
                    .as_ref()
                    .map(axon_sdk::invocation::wire::error_to_wire),
                ..Default::default()
            },
        )
        .await
    }

    async fn cancel_opened_bidi(
        outbound: &SessionUpSender,
        call_id: u64,
        handle: axon_sdk::invocation::BidiInvocationHandle,
        reason: String,
    ) -> Result<(), SessionDispatchError> {
        let (_input, output) = handle.split();
        if let Err(error) = output.cancel(reason.clone()).await {
            return Self::send_bidi_control_failure(
                outbound,
                call_id,
                "CANONICAL_CANCELLATION_FAILED",
                format!("{reason}; cancel failed: {error}"),
            )
            .await;
        }
        match output.finalized().await {
            Ok(finalized) => Self::send_bidi_terminal(outbound, call_id, &finalized).await,
            Err(error) => {
                Self::send_bidi_control_failure(
                    outbound,
                    call_id,
                    "CANONICAL_FINALIZATION_REQUIRED",
                    format!("{reason}; finalization failed: {error}"),
                )
                .await
            }
        }
    }

    /// Bind an admitted local bidi handle to `call_id` and enter the Active
    /// phase. Admission is a barrier: no `BidiInput` can observe the call in
    /// `remote_bidi_sessions` until its canonical admission proof has been
    /// published upstream. This preserves the lifecycle order
    /// `Opening -> Admitted -> Active -> Terminal` even when the peer queues
    /// input immediately after its open frame.
    async fn register_remote_bidi(
        &self,
        call_id: u64,
        ability: &str,
        handle: axon_sdk::invocation::BidiInvocationHandle,
        outbound: &SessionUpSender,
        lifecycle_envelope: axon_sdk::invocation::DescriptorBoundEnvelope,
    ) -> Result<(), SessionDispatchError> {
        let (handler_in_tx, mut handler_out_rx) = handle.split();
        let lifecycle = match RegisteredInvocationLifecycle::register(
            self.lifecycle_cancellations.clone(),
            &lifecycle_envelope,
            handler_out_rx.handle().clone(),
        ) {
            Ok(lifecycle) => lifecycle,
            Err(error) => {
                let reason = format!("lifecycle cancellation registration failed: {error}");
                if let Err(cancel_error) = handler_out_rx.cancel(reason.clone()).await {
                    return Self::send_bidi_control_failure(
                        outbound,
                        call_id,
                        "CANONICAL_CANCELLATION_FAILED",
                        format!("{reason}; cancel failed: {cancel_error}"),
                    )
                    .await;
                }
                return match handler_out_rx.finalized().await {
                    Ok(finalized) => Self::send_bidi_terminal(outbound, call_id, &finalized).await,
                    Err(finalization_error) => {
                        Self::send_bidi_control_failure(
                            outbound,
                            call_id,
                            "CANONICAL_FINALIZATION_REQUIRED",
                            format!("{reason}; finalization failed: {finalization_error}"),
                        )
                        .await
                    }
                };
            }
        };

        let admission = match handler_out_rx.admission_receipt().await {
            Ok(receipt) => receipt,
            Err(error) => {
                if let Ok(finalized) = lifecycle.finalized().await {
                    return Self::send_bidi_terminal(outbound, call_id, &finalized).await;
                }
                return Self::send_bidi_control_failure(
                    outbound,
                    call_id,
                    "CANONICAL_ADMISSION_REQUIRED",
                    format!("CANONICAL_ADMISSION_REQUIRED: {error}"),
                )
                .await;
            }
        };
        if let Err(error) = Self::send_bidi_admission(outbound, call_id, &admission).await {
            let _ = lifecycle
                .cancel_and_finalize("session bidi closed before admission")
                .await;
            return Err(error);
        }

        {
            let mut guard = match self.remote_bidi_sessions.lock() {
                Ok(g) => g,
                Err(poisoned) => poisoned.into_inner(),
            };
            guard.insert(
                call_id,
                ActiveRemoteBidi {
                    ability: ability.to_string(),
                    sender: handler_in_tx,
                },
            );
        }

        let sessions = Arc::clone(&self.remote_bidi_sessions);
        let outbound = outbound.clone();
        let ability_owned = ability.to_string();
        let ability_wire = Arc::clone(&self.ability_wire);
        tokio::spawn(async move {
            let mut canonical_terminal_observed = false;
            while let Some(frame_result) = handler_out_rx.next_frame().await {
                let frame = match frame_result {
                    Ok(frame) => frame,
                    Err(err) => {
                        let finalized = match lifecycle.finalized().await {
                            Ok(finalized) => finalized,
                            Err(error) => {
                                let _ = LocalAxonSessionDispatcher::send_bidi_control_failure(
                                    &outbound,
                                    call_id,
                                    "CANONICAL_FINALIZATION_REQUIRED",
                                    format!("frame_error={err}; finalization_error={error}"),
                                )
                                .await;
                                break;
                            }
                        };
                        canonical_terminal_observed = true;
                        let _ = LocalAxonSessionDispatcher::send_bidi_terminal(
                            &outbound, call_id, &finalized,
                        )
                        .await;
                        break;
                    }
                };
                if frame.terminal {
                    let finalized = match lifecycle.finalized().await {
                        Ok(finalized) => finalized,
                        Err(error) => {
                            let _ = LocalAxonSessionDispatcher::send_bidi_control_failure(
                                &outbound,
                                call_id,
                                "CANONICAL_FINALIZATION_REQUIRED",
                                error.to_string(),
                            )
                            .await;
                            break;
                        }
                    };
                    canonical_terminal_observed = true;
                    let _ = LocalAxonSessionDispatcher::send_bidi_terminal(
                        &outbound, call_id, &finalized,
                    )
                    .await;
                    break;
                }
                let mapped = if frame.payload.is_empty() {
                    None
                } else if LocalAxonSessionDispatcher::is_json_frame_bidi_with(
                    &ability_wire,
                    &ability_owned,
                ) && !frame.content_type.is_empty()
                    && frame.content_type != "application/json"
                {
                    Some(BidiOutputProjection {
                        call_id,
                        payload: frame.payload,
                        failure: None,
                    })
                } else {
                    match serde_json::from_slice::<Value>(&frame.payload) {
                        Ok(value) => {
                            match LocalAxonSessionDispatcher::map_remote_bidi_output_with(
                                &ability_wire,
                                call_id,
                                &ability_owned,
                                &value,
                            ) {
                                Ok(mapped) => mapped,
                                Err(err) => {
                                    let reason = format!(
                                        "session.open: remote bidi output map failed: {err}"
                                    );
                                    match lifecycle.cancel_and_finalize(reason.clone()).await {
                                        Ok(finalized) => {
                                            canonical_terminal_observed = true;
                                            let _ = LocalAxonSessionDispatcher::send_bidi_terminal(
                                                &outbound, call_id, &finalized,
                                            )
                                            .await;
                                        }
                                        Err(finalization_error) => {
                                            let _ = LocalAxonSessionDispatcher::send_bidi_control_failure(
                                                &outbound,
                                                call_id,
                                                "CANONICAL_FINALIZATION_REQUIRED",
                                                format!("{reason}; finalization failed: {finalization_error}"),
                                            )
                                            .await;
                                        }
                                    }
                                    break;
                                }
                            }
                        }
                        Err(err) => {
                            let reason =
                                format!("session.open: remote bidi output was not JSON: {err}");
                            match lifecycle.cancel_and_finalize(reason.clone()).await {
                                Ok(finalized) => {
                                    canonical_terminal_observed = true;
                                    let _ = LocalAxonSessionDispatcher::send_bidi_terminal(
                                        &outbound, call_id, &finalized,
                                    )
                                    .await;
                                }
                                Err(finalization_error) => {
                                    let _ = LocalAxonSessionDispatcher::send_bidi_control_failure(
                                        &outbound,
                                        call_id,
                                        "CANONICAL_FINALIZATION_REQUIRED",
                                        format!(
                                            "{reason}; finalization failed: {finalization_error}"
                                        ),
                                    )
                                    .await;
                                }
                            }
                            break;
                        }
                    }
                };
                let Some(mapped) = mapped else {
                    continue;
                };
                if LocalAxonSessionDispatcher::send_bidi_progress(&outbound, mapped)
                    .await
                    .is_err()
                {
                    break;
                }
            }
            if !canonical_terminal_observed {
                if let Ok(finalized) = lifecycle
                    .cancel_and_finalize("session bidi output forwarder closed")
                    .await
                {
                    let _ = LocalAxonSessionDispatcher::send_bidi_terminal(
                        &outbound, call_id, &finalized,
                    )
                    .await;
                }
            }
            let mut guard = match sessions.lock() {
                Ok(g) => g,
                Err(poisoned) => poisoned.into_inner(),
            };
            guard.remove(&call_id);
        });
        Ok(())
    }

    async fn forward_remote_bidi_input(
        &self,
        call_id: u64,
        payload: Vec<u8>,
        eof: bool,
        outbound: &SessionUpSender,
    ) -> Result<(), SessionDispatchError> {
        let active = {
            let mut guard = match self.remote_bidi_sessions.lock() {
                Ok(g) => g,
                Err(poisoned) => poisoned.into_inner(),
            };
            let active = guard.get(&call_id).cloned();
            if eof {
                guard.remove(&call_id);
            }
            active
        };

        let Some(active) = active else {
            if eof {
                let stream_cancel = {
                    let mut guard = match self.remote_stream_sessions.lock() {
                        Ok(g) => g,
                        Err(poisoned) => poisoned.into_inner(),
                    };
                    guard.remove(&call_id)
                };
                if let Some(token) = stream_cancel {
                    token.cancel();
                    return Ok(());
                }
            }
            return Self::send_bidi_control_failure(
                outbound,
                call_id,
                "BIDI_SESSION_NOT_OPEN",
                format!("remote bidi call_id={call_id} is not open on this device"),
            )
            .await;
        };

        let frame = if eof {
            if self.is_json_frame_bidi(&active.ability) {
                json!({"type": "close", "reason": "bidi_eof"})
            } else {
                json!({"type": "eof"})
            }
        } else if active.ability
            == crate::daemon::ability::builtins::device_control::terminal::attach::ABILITY_TERMINAL_ATTACH
            || self.is_json_frame_bidi(&active.ability)
        {
            serde_json::from_slice::<Value>(&payload).map_err(|err| {
                SessionDispatchError::Other(format!("decode remote bidi JSON input: {err}"))
            })?
        } else {
            json!({"type": "chunk", "data": B64.encode(payload)})
        };
        let payload = serde_json::to_vec(&frame).map_err(|err| {
            SessionDispatchError::Other(format!("encode remote bidi input: {err}"))
        })?;
        let send_result = active
            .sender
            .send(BidiInputFrame::new(payload).with_content_type("application/json"))
            .await;
        if eof {
            let _ = active.sender.close_input().await;
        }
        if send_result.is_err() {
            if eof {
                // Download-mode file_transfer does not consume the
                // up-direction at all; the caller's EOF is a best-
                // effort readiness hint, not a mandatory delivery.
                return Ok(());
            }
            return Self::send_bidi_control_failure(
                outbound,
                call_id,
                "BIDI_INPUT_CLOSED",
                format!("remote bidi call_id={call_id} input channel closed"),
            )
            .await;
        }
        Ok(())
    }
}

impl Default for LocalAxonSessionDispatcher {
    fn default() -> Self {
        Self::new()
    }
}

#[async_trait::async_trait]
impl SessionFrameDispatcher for LocalAxonSessionDispatcher {
    /// Receive a hub-pushed canonical dispatch or daemon-control frame.
    /// Calls execute against the local runtime and reply with protobuf
    /// `DispatchResult` frames.
    async fn handle_down(
        &self,
        frame: InvokeBidiDown,
        outbound: &SessionUpSender,
    ) -> Result<(), SessionDispatchError> {
        if let Some(DownPayload::ReverseDispatchResult(result)) = frame.payload.as_ref() {
            let call_id: [u8; 16] = result.call_id.as_slice().try_into().map_err(|_| {
                SessionDispatchError::Other(format!(
                    "ReverseDispatchResult call_id must be 16 bytes, got {}",
                    result.call_id.len()
                ))
            })?;
            if let Some(correlation) = self.escalation_correlation.as_ref() {
                correlation.deliver_reverse_dispatch_result(call_id, result.clone());
            }
            return Ok(());
        }

        // DispatchCall carries the complete canonical InvokeRequest and is
        // dispatched without a product-side request projection.
        if let Some(DownPayload::DispatchCall(call)) = frame.payload.as_ref() {
            if call.open_bidi {
                return self
                    .handle_carrier_v1_dispatch(call.clone(), outbound)
                    .await;
            }
            let dispatcher = self.clone();
            let outbound = outbound.clone();
            let call = call.clone();
            tokio::spawn(async move {
                if let Err(err) = dispatcher.handle_carrier_v1_dispatch(call, &outbound).await {
                    crate::op_event!(
                        component = local_session_dispatcher,
                        kind = carrier_v1_dispatch_task_failed,
                        error = err.to_string(),
                    );
                }
            });
            return Ok(());
        }

        // Only `BinaryChunk` frames carry SessionDispatch; ignore
        // Receipt / Control frames silently (PR-1 semantics).
        let DownPayload::BinaryChunk(chunk) = frame.payload.ok_or_else(|| {
            SessionDispatchError::Other("session down frame had no payload".to_string())
        })?
        else {
            return Ok(());
        };

        // Surface a marker even on the cold path: reading the
        // BinaryChunk size confirms the down-stream is feeding the
        // dispatcher, distinct from a stalled supervisor or a
        // transport hang. Without this we cannot tell from logs
        // alone whether the bidi delivered a frame at all.
        let stream_id = chunk.stream_id;
        let data_bytes = chunk.data.len();
        crate::op_event!(
            component = local_session_dispatcher,
            kind = handle_down_binary_chunk,
            stream_id = stream_id,
            data_bytes = data_bytes,
        );

        let dispatch = SessionDispatch::decode_frame(&chunk.data).map_err(|err| {
            SessionDispatchError::Other(format!(
                "session down BinaryChunk is not valid SessionDispatch JSON: {err}"
            ))
        })?;

        match dispatch {
            SessionDispatch::BidiInput {
                call_id,
                payload,
                eof,
            } => {
                return self
                    .forward_remote_bidi_input(call_id, payload, eof, outbound)
                    .await;
            }
            SessionDispatch::RequestResult { call_id, outcome } => {
                if let Some(correlation) = self.escalation_correlation.as_ref() {
                    let id_hex = call_id_hex(&call_id);
                    let fired = correlation.complete(
                        call_id,
                        crate::daemon::invocation::bidi::session_escalation::EscalationReply::Control(
                            outcome,
                        ),
                    );
                    if !fired {
                        crate::op_event!(
                            component = local_session_dispatcher,
                            kind = request_result_orphan,
                            call_id = id_hex,
                            message = "no pending entry matched; dropping (caller may have timed out, or hub double-replied)",
                        );
                    } else {
                        crate::op_event!(
                            component = local_session_dispatcher,
                            kind = request_result_completed,
                            call_id = id_hex,
                        );
                    }
                } else {
                    crate::op_event!(
                        component = local_session_dispatcher,
                        kind = request_result_dropped_hub_mode,
                        message = "inbound RequestResult on a hub-mode daemon (no escalation_correlation wired); ignoring",
                    );
                }
                return Ok(());
            }
            SessionDispatch::Request { .. } => Ok(()),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    use serde_json::json;
    use std::time::Duration;

    const TEST_DEVICE_URA: &str = "easynet:///r/t/device/d1";

    #[test]
    fn carrier_v1_control_failure_is_not_lifecycle_terminal() {
        let result =
            carrier_v1_control_failure(9, "STREAM_OPEN_FAILED", "target rejected stream open");
        assert_eq!(result.call_id, 9);
        assert!(
            !result.terminal,
            "synthetic stream failures must not claim canonical terminality"
        );
        assert!(
            result.terminal_receipt.is_none(),
            "control failures must not synthesize terminal receipts"
        );
        assert_eq!(
            result.failure.as_ref().map(|failure| failure.code.as_str()),
            Some("STREAM_OPEN_FAILED")
        );
    }

    #[test]
    fn handler_error_frame_requires_code_and_message_before_failure_projection() {
        for (payload, expected) in [
            (
                json!({"type": "error", "message": "permission denied"}),
                "`code`",
            ),
            (
                json!({"type": "error", "code": "permission_denied"}),
                "`message`",
            ),
            (
                json!({"type": "error", "code": " ", "message": "permission denied"}),
                "`code`",
            ),
            (
                json!({"type": "error", "code": "permission_denied", "message": " "}),
                "`message`",
            ),
        ] {
            let error = HandlerErrorFrame::parse(&payload, "JSON-frame bidi error frame")
                .expect_err("incomplete handler error frames must fail closed");
            match error {
                SessionDispatchError::Other(message) => {
                    assert!(
                        message.contains("JSON-frame bidi error frame requires non-empty")
                            && message.contains(expected),
                        "schema failure must name the missing error fact; got: {message}"
                    );
                }
            }
        }
    }

    #[test]
    fn file_transfer_error_frame_rejects_missing_message_before_failure_projection() {
        let error = LocalAxonSessionDispatcher::map_remote_file_transfer_output(
            7,
            &json!({
                "type": "error",
                "code": "disk_full",
            }),
        )
        .expect_err("file_transfer error frames must carry typed failure facts");
        match error {
            SessionDispatchError::Other(message) => {
                assert!(
                    message.contains("file_transfer error frame requires non-empty `message`"),
                    "schema failure must reject missing message before projection; got: {message}"
                );
            }
        }
    }

    // Descriptor proof a test ability must carry so Axon's receipt-proof
    // normalizer admits its dispatch. Production stamps these from the
    // control-plane record (AxonAbilityCatalog::bind_runtime_proof_for_mode);
    // a raw-runtime test registration has no control plane, so it binds the
    // same non-zero stub facts the rest of the suite uses. The version must
    // match what the descriptor-bound dispatch path stamps on the envelope —
    // the default descriptor version for these owner-local test abilities.
    const TEST_DESCRIPTOR_VERSION: &str =
        crate::daemon::ability::DEFAULT_ABILITY_DESCRIPTOR_VERSION;
    const TEST_DESCRIPTOR_VERSION_V2: &str = "2.0.0";
    const TEST_DESCRIPTOR_HASH: [u8; 32] = [0x33; 32];
    const TEST_SCHEMA_HASH: [u8; 32] = [0x11; 32];
    const TEST_IMPL_HASH: [u8; 32] = [0x22; 32];
    const TEST_CALLER_URA: &str = "easynet:///r/t/user/alice";

    struct FixedCarrierKey(ed25519_dalek::VerifyingKey);

    impl axon_sdk::invocation::KeyResolver for FixedCarrierKey {
        fn resolve(
            &self,
            agent_ura: &str,
        ) -> Result<ed25519_dalek::VerifyingKey, axon_sdk::invocation::AxonError> {
            if agent_ura == TEST_CALLER_URA {
                return Ok(self.0);
            }
            if agent_ura == crate::daemon::identity::local_invocation::LOCAL_SYSTEM_AGENT_URA {
                return crate::daemon::identity::local_invocation::system_verifying_key()
                    .map_err(|error| axon_sdk::invocation::AxonError::internal(error.to_string()));
            }
            Err(axon_sdk::invocation::AxonError::invalid_argument(format!(
                "unknown_agent_key:{agent_ura}"
            )))
        }
    }

    fn carrier_v1_signing_key() -> ed25519_dalek::SigningKey {
        ed25519_dalek::SigningKey::from_bytes(&[0x4Du8; 32])
    }

    fn runtime_ability_for(callee_ura: &str, ability: &str) -> String {
        crate::daemon::axon_bridge::descriptor_ref::ability_ura_for_wire(callee_ura, ability)
            .expect("test ability must resolve to canonical Ability URA")
    }

    fn descriptor_binding_for_version(descriptor_version: &str) -> String {
        descriptor_binding_for_version_and_action(descriptor_version, "invoke")
    }

    fn descriptor_binding_for_version_and_action(
        descriptor_version: &str,
        admission_action: &str,
    ) -> String {
        crate::daemon::axon_bridge::descriptor_ref::descriptor_binding_for_wire(
            descriptor_version,
            TEST_DESCRIPTOR_HASH,
            admission_action,
        )
        .expect("test descriptor binding")
    }

    fn catalog_call_mode(mode: axon_sdk::invocation::CallMode) -> crate::daemon::ability::CallMode {
        match mode {
            axon_sdk::invocation::CallMode::Rpc => crate::daemon::ability::CallMode::Rpc,
            axon_sdk::invocation::CallMode::Stream => crate::daemon::ability::CallMode::Stream,
            axon_sdk::invocation::CallMode::Bidi => crate::daemon::ability::CallMode::Bidi,
        }
    }

    fn descriptor_ref_for_call_mode(
        callee_ura: &str,
        ability: &str,
        mode: axon_sdk::invocation::CallMode,
    ) -> String {
        if let Ok(descriptor_ref) = axon_sdk::invocation::canonical_ability_descriptor_ref(ability)
        {
            return descriptor_ref;
        }
        crate::daemon::axon_bridge::descriptor_ref::catalog_descriptor_ref_for_wire(
            callee_ura,
            ability,
            catalog_call_mode(mode),
        )
        .expect("test ability must resolve through canonical catalog descriptor authority")
    }

    fn explicit_test_descriptor_ref_with_action(
        callee_ura: &str,
        ability: &str,
        descriptor_version: &str,
        admission_action: &str,
    ) -> String {
        crate::daemon::axon_bridge::descriptor_ref::ability_descriptor_ref_for_wire(
            callee_ura,
            ability,
            &descriptor_binding_for_version_and_action(descriptor_version, admission_action),
        )
        .expect("explicit proof-bound test descriptor ref")
    }

    /// Proof-bound RPC options mirroring what the control plane stamps in
    /// production. Use for every raw-runtime test registration so the
    /// dispatch path sees a bound descriptor proof.
    fn proof_bound_rpc_options() -> axon_sdk::invocation::AbilityOptions {
        proof_bound_rpc_options_with_version(TEST_DESCRIPTOR_VERSION)
    }

    fn proof_bound_rpc_options_with_version(
        descriptor_version: &str,
    ) -> axon_sdk::invocation::AbilityOptions {
        use axon_sdk::invocation::{AbilityCallModes, AbilityOptions};
        AbilityOptions::default()
            .with_modes(AbilityCallModes::RPC)
            .with_descriptor_proof(
                descriptor_version,
                "invoke",
                TEST_DESCRIPTOR_HASH,
                TEST_SCHEMA_HASH,
                TEST_IMPL_HASH,
            )
    }

    /// Stream-mode twin of [`proof_bound_rpc_options`]: a server-streaming
    /// test ability binds its proof on the Stream call mode.
    fn proof_bound_stream_options() -> axon_sdk::invocation::AbilityOptions {
        use axon_sdk::invocation::{AbilityOptions, CallMode};
        AbilityOptions::streaming().with_mode_descriptor_proof(
            CallMode::Stream,
            TEST_DESCRIPTOR_VERSION,
            crate::daemon::ability::descriptors::AdmissionAction::Stream.as_str(),
            TEST_DESCRIPTOR_HASH,
            TEST_SCHEMA_HASH,
            TEST_IMPL_HASH,
        )
    }

    fn proof_bound_rpc_stream_options() -> axon_sdk::invocation::AbilityOptions {
        use axon_sdk::invocation::{AbilityCallModes, AbilityOptions, CallMode};
        AbilityOptions::default()
            .with_modes(AbilityCallModes {
                rpc: true,
                stream: true,
                bidi: false,
            })
            .with_mode_descriptor_proof(
                CallMode::Rpc,
                TEST_DESCRIPTOR_VERSION,
                "invoke",
                TEST_DESCRIPTOR_HASH,
                TEST_SCHEMA_HASH,
                TEST_IMPL_HASH,
            )
            .with_mode_descriptor_proof(
                CallMode::Stream,
                TEST_DESCRIPTOR_VERSION,
                crate::daemon::ability::descriptors::AdmissionAction::Stream.as_str(),
                TEST_DESCRIPTOR_HASH,
                TEST_SCHEMA_HASH,
                TEST_IMPL_HASH,
            )
    }

    async fn register_test_rpc(
        runtime: &axon_sdk::invocation::LocalRuntime,
        ability: &str,
        handler: axon_sdk::invocation::AbilityFn,
    ) {
        register_test_ability_with_options(runtime, ability, handler, proof_bound_rpc_options())
            .await;
    }

    async fn register_test_ability_with_options(
        runtime: &axon_sdk::invocation::LocalRuntime,
        ability: &str,
        handler: axon_sdk::invocation::AbilityFn,
        options: axon_sdk::invocation::AbilityOptions,
    ) {
        runtime
            .register_ability_with_options(
                runtime_ability_for(TEST_DEVICE_URA, ability),
                handler,
                options,
            )
            .await
            .expect("test ability registers under canonical runtime key");
    }

    fn executable_runtime() -> Arc<axon_sdk::invocation::LocalRuntime> {
        crate::daemon::axon_bridge::runtime_factory::build_local_runtime(
            Arc::new(FixedCarrierKey(carrier_v1_signing_key().verifying_key())),
            None,
        )
    }

    fn session_frame(dispatch: SessionDispatch) -> InvokeBidiDown {
        let payload = serde_json::to_vec(&dispatch).expect("encode session dispatch");
        InvokeBidiDown {
            sequence: 0,
            payload: Some(DownPayload::BinaryChunk(BinaryChunk {
                data: payload,
                ..BinaryChunk::default()
            })),
            ..InvokeBidiDown::default()
        }
    }

    fn carrier_v1_call(call_id: u64, ability: &str, args: Vec<u8>) -> InvokeBidiDown {
        carrier_v1_call_signed_as(call_id, ability, ability, args)
    }

    fn carrier_v1_explicit_test_call(call_id: u64, ability: &str, args: Vec<u8>) -> InvokeBidiDown {
        carrier_v1_explicit_test_call_with_mode(
            call_id,
            ability,
            args,
            axon_sdk::invocation::CallMode::Rpc,
        )
    }

    fn carrier_v1_explicit_test_call_with_mode(
        call_id: u64,
        ability: &str,
        args: Vec<u8>,
        mode: axon_sdk::invocation::CallMode,
    ) -> InvokeBidiDown {
        let admission_action = match mode {
            axon_sdk::invocation::CallMode::Stream => {
                crate::daemon::ability::descriptors::AdmissionAction::Stream.as_str()
            }
            axon_sdk::invocation::CallMode::Rpc | axon_sdk::invocation::CallMode::Bidi => "invoke",
        };
        let descriptor_ref = explicit_test_descriptor_ref_with_action(
            TEST_DEVICE_URA,
            ability,
            TEST_DESCRIPTOR_VERSION,
            admission_action,
        );
        carrier_v1_call_signed_as_with_mode(call_id, ability, &descriptor_ref, args, mode)
    }

    fn carrier_v1_call_signed_as(
        call_id: u64,
        request_ability: &str,
        signed_ability: &str,
        args: Vec<u8>,
    ) -> InvokeBidiDown {
        carrier_v1_call_signed_as_with_mode(
            call_id,
            request_ability,
            signed_ability,
            args,
            axon_sdk::invocation::CallMode::Rpc,
        )
    }

    fn carrier_v1_call_signed_as_with_mode(
        call_id: u64,
        request_ability: &str,
        signed_ability: &str,
        args: Vec<u8>,
        mode: axon_sdk::invocation::CallMode,
    ) -> InvokeBidiDown {
        use axon_sdk::pb::axon::v1::{DispatchCall, InvokeRequest};
        use ed25519_dalek::Signer as _;

        let signing_key = carrier_v1_signing_key();
        let signed_descriptor_ref =
            descriptor_ref_for_call_mode(TEST_DEVICE_URA, signed_ability, mode);
        let mut envelope = crate::daemon::invocation::ProtoEnvelope::from_target(
            TEST_CALLER_URA,
            "easynet:///r/t/device/d1",
            "easynet:///r/t/device/d1",
            crate::daemon::invocation::InvocationDerivationPolicy::FreshRoot,
        )
        .expect("valid carrier-v1 envelope")
        .into_inner(&signed_descriptor_ref, &args)
        .expect("complete carrier-v1 tuple");
        let descriptor_bound =
            crate::daemon::axon_bridge::wire_descriptor::descriptor_bound_from_wire_parts(
                envelope.clone(),
                signed_descriptor_ref.clone(),
                &args,
            )
            .expect("descriptor-bound carrier-v1 envelope");
        let signature = signing_key.sign(&descriptor_bound_canonical_bytes(
            &descriptor_bound.envelope,
        ));
        envelope.caller_signature = Some(axon_sdk::pb::axon::v1::CallerSignature {
            algorithm: "ed25519".to_string(),
            signature: signature.to_bytes().to_vec(),
            key_id_hint: String::new(),
        });

        let request = InvokeRequest {
            envelope: Some(envelope),
            target: Some(
                crate::daemon::invocation::dispatch::invocation_wire::wire_invocation_target(
                    &signed_descriptor_ref,
                    request_ability,
                )
                .expect("carrier-v1 typed target"),
            ),
            arguments: args,
            ..Default::default()
        };

        InvokeBidiDown {
            payload: Some(DownPayload::DispatchCall(DispatchCall {
                call_id,
                request: Some(request),
                open_bidi: false,
            })),
            ..InvokeBidiDown::default()
        }
    }

    fn carrier_v1_bidi_open(call_id: u64, ability: &str, args: Vec<u8>) -> InvokeBidiDown {
        let mut frame = carrier_v1_call_signed_as_with_mode(
            call_id,
            ability,
            ability,
            args,
            axon_sdk::invocation::CallMode::Bidi,
        );
        if let Some(DownPayload::DispatchCall(call)) = frame.payload.as_mut() {
            call.open_bidi = true;
        }
        frame
    }

    fn carrier_v1_explicit_test_bidi_open(
        call_id: u64,
        ability: &str,
        args: Vec<u8>,
    ) -> InvokeBidiDown {
        let mut frame = carrier_v1_explicit_test_call_with_mode(
            call_id,
            ability,
            args,
            axon_sdk::invocation::CallMode::Bidi,
        );
        if let Some(DownPayload::DispatchCall(call)) = frame.payload.as_mut() {
            call.open_bidi = true;
        }
        frame
    }

    /// Quadrant [new hub, new device] for step-3b: a carrier-v1 bidi
    /// open admits through the canonical wire-parts path, streams
    /// over the same byte channel, and the terminal frame replies as
    /// a proto DispatchResult.
    #[tokio::test(flavor = "multi_thread", worker_threads = 2)]
    async fn carrier_v1_bidi_open_round_trips_and_replies_proto_on_v1_session() {
        let tmp = tempfile::tempdir().expect("tempdir");
        let target = tmp.path().join("upload-from-hub-v1.bin");
        let bytes = b"carrier-v1-bidi-over-session";

        let rt = executable_runtime();
        let _registry = build_real_daemon_registry_with_runtime(Some(Arc::clone(&rt)));
        let disp = LocalAxonSessionDispatcher::new().with_local_runtime(rt);
        let (tx, mut rx) = mpsc::channel::<InvokeBidiUp>(8);
        let session_tx = SessionUpSender::new(tx);
        session_tx.set_negotiated_contract(1);

        let args = serde_json::to_vec(&json!({
            "mode": "upload",
            "resource_ref": crate::daemon::resources::files::resource_ref_for_local_path(
                &target,
                crate::daemon::resources::files::FilesystemResourceCapability::Write,
            )
            .expect("local fs ResourceRef"),
        }))
        .expect("encode args");
        disp.handle_down(
            carrier_v1_bidi_open(
                77,
                crate::daemon::ability::builtins::device_control::file_transfer::ABILITY_FILE_TRANSFER,
                args,
            ),
            &session_tx,
        )
        .await
        .expect("v1 bidi open succeeds");

        disp.handle_down(
            session_frame(SessionDispatch::BidiInput {
                call_id: 77,
                payload: bytes.to_vec(),
                eof: false,
            }),
            &session_tx,
        )
        .await
        .expect("bidi chunk forwards");
        disp.handle_down(
            session_frame(SessionDispatch::BidiInput {
                call_id: 77,
                payload: Vec::new(),
                eof: true,
            }),
            &session_tx,
        )
        .await
        .expect("bidi eof forwards");

        let admission = tokio::time::timeout(Duration::from_secs(3), rx.recv())
            .await
            .expect("admission reply within 3s")
            .expect("admission reply produced");
        let admission = match admission.payload {
            Some(UpPayload::DispatchResult(result)) => result,
            other => panic!("expected admission DispatchResult on a v1 session, got: {other:?}"),
        };
        assert_eq!(admission.call_id, 77);
        assert!(
            !admission.terminal,
            "first carrier-v1 bidi frame must be admission, got {admission:?}"
        );
        assert_eq!(
            admission
                .admission_receipt
                .expect("admission frame carries receipt")
                .state,
            axon_sdk::invocation::InvocationState::Admitted.to_wire_i32()
        );

        let progress = tokio::time::timeout(Duration::from_secs(3), rx.recv())
            .await
            .expect("progress reply within 3s")
            .expect("progress reply produced");
        let progress = match progress.payload {
            Some(UpPayload::DispatchResult(result)) => result,
            other => panic!("expected progress DispatchResult on a v1 session, got: {other:?}"),
        };
        assert_eq!(progress.call_id, 77);
        assert!(!progress.terminal);
        assert!(progress.admission_receipt.is_none());
        assert!(progress.terminal_receipt.is_none());

        let reply = tokio::time::timeout(Duration::from_secs(3), rx.recv())
            .await
            .expect("terminal reply within 3s")
            .expect("reply produced");
        let result = match reply.payload {
            Some(UpPayload::DispatchResult(r)) => r,
            other => panic!("expected proto DispatchResult on a v1 session, got: {other:?}"),
        };
        assert_eq!(result.call_id, 77);
        assert!(result.terminal, "upload reply must be terminal");
        assert!(
            result.failure.is_none(),
            "upload must succeed: {:?}",
            result.failure
        );
        let receipt = result
            .terminal_receipt
            .expect("terminal bidi frame carries the execution receipt (chain closure)");
        assert_eq!(
            receipt.state,
            axon_sdk::invocation::InvocationState::Completed.to_wire_i32(),
            "receipt must record the terminal state"
        );
        assert!(
            disp.lifecycle_cancellations
                .contains_invocation_id(&receipt.invocation_id),
            "carrier-v1 bidi lifecycle must remain registered for invocation.cancel"
        );
        assert_eq!(
            std::fs::read(&target).expect("uploaded file exists"),
            bytes,
            "payload bytes must land on the device-side filesystem"
        );
    }

    /// step-3b open errors are frames, not transport errors: an
    /// unwired ability on a v1 session replies a typed proto failure.
    #[tokio::test]
    async fn carrier_v1_bidi_open_of_unwired_ability_fails_proto_on_v1_session() {
        let rt = executable_runtime();
        let disp = LocalAxonSessionDispatcher::new().with_local_runtime(rt);
        let (tx, mut rx) = mpsc::channel::<InvokeBidiUp>(4);
        let session_tx = SessionUpSender::new(tx);
        session_tx.set_negotiated_contract(1);

        disp.handle_down(
            carrier_v1_explicit_test_bidi_open(9, "test.echo", b"{}".to_vec()),
            &session_tx,
        )
        .await
        .expect("open error replies as a frame, not an Err");

        let reply = rx.recv().await.expect("reply produced");
        match reply.payload {
            Some(UpPayload::DispatchResult(r)) => {
                assert_eq!(r.call_id, 9);
                assert!(!r.terminal);
                assert!(r.admission_receipt.is_none());
                assert!(r.terminal_receipt.is_none());
                let failure = r.failure.expect("typed failure");
                assert!(
                    failure.message.contains("not published"),
                    "unexpected failure: {}",
                    failure.message
                );
            }
            other => panic!("expected proto DispatchResult, got: {other:?}"),
        }
    }

    #[tokio::test]
    async fn carrier_v1_stream_dispatch_of_unpublished_ability_fails_proto_without_timeout() {
        let rt = executable_runtime();
        let disp = LocalAxonSessionDispatcher::new().with_local_runtime(rt);
        let (tx, mut rx) = mpsc::channel::<InvokeBidiUp>(4);
        let session_tx = SessionUpSender::new(tx);
        session_tx.set_negotiated_contract(1);

        disp.handle_down(
            carrier_v1_explicit_test_call_with_mode(
                17,
                "screen.removed",
                b"{}".to_vec(),
                axon_sdk::invocation::CallMode::Stream,
            ),
            &session_tx,
        )
        .await
        .expect("stream publication miss replies as a frame, not a task error");

        let reply = tokio::time::timeout(Duration::from_secs(1), rx.recv())
            .await
            .expect("unpublished stream must not wait for invocation timeout")
            .expect("reply produced");
        match reply.payload {
            Some(UpPayload::DispatchResult(result)) => {
                assert_eq!(result.call_id, 17);
                assert!(
                    !result.terminal,
                    "control failure must not synthesize canonical stream terminality"
                );
                assert!(result.admission_receipt.is_none());
                assert!(result.terminal_receipt.is_none());
                let failure = result.failure.expect("typed failure");
                assert_eq!(failure.code, "ABILITY_RESOLUTION_FAILED");
                assert!(
                    failure
                        .message
                        .contains("is not registered in Axon LocalRuntime"),
                    "unexpected failure: {}",
                    failure.message
                );
            }
            other => panic!("expected proto DispatchResult, got: {other:?}"),
        }
    }

    #[tokio::test]
    async fn carrier_v1_dispatch_executes_and_replies_proto_on_v1_session() {
        let rt = executable_runtime();
        register_test_rpc(
            &rt,
            "test.echo",
            axon_sdk::invocation::make_ability(|ctx| async move { Ok(ctx.payload.clone()) }),
        )
        .await;
        let disp = LocalAxonSessionDispatcher::new().with_local_runtime(Arc::clone(&rt));
        let (tx, mut rx) = mpsc::channel::<InvokeBidiUp>(4);
        let session_tx = SessionUpSender::new(tx);
        session_tx.set_negotiated_contract(1);

        disp.handle_down(
            carrier_v1_explicit_test_call(7, "test.echo", br#"{"hello":"v1"}"#.to_vec()),
            &session_tx,
        )
        .await
        .expect("carrier-v1 dispatch succeeds");

        let reply = tokio::time::timeout(Duration::from_secs(3), rx.recv())
            .await
            .expect("reply within 3s")
            .expect("reply produced");
        let Some(UpPayload::DispatchResult(result)) = reply.payload else {
            panic!(
                "v1 session must reply DispatchResult, got {:?}",
                reply.payload
            );
        };
        assert_eq!(result.call_id, 7);
        assert!(result.terminal);
        assert!(
            result.failure.is_none(),
            "carrier-v1 dispatch failed: {:?}",
            result.failure
        );
        assert_eq!(result.payload, br#"{"hello":"v1"}"#);
        let admission = result
            .admission_receipt
            .expect("successful unary carrier reply carries admission checkpoint");
        let terminal = result
            .terminal_receipt
            .expect("successful unary carrier reply carries terminal checkpoint");
        assert_eq!(
            admission.state,
            axon_sdk::invocation::InvocationState::Admitted.to_wire_i32()
        );
        assert_eq!(
            terminal.state,
            axon_sdk::invocation::InvocationState::Completed.to_wire_i32()
        );
        assert_eq!(admission.invocation_id, terminal.invocation_id);
    }

    #[tokio::test]
    async fn carrier_v1_dispatch_preserves_non_default_descriptor_version() {
        let rt = executable_runtime();
        register_test_ability_with_options(
            &rt,
            "test.echo",
            axon_sdk::invocation::make_ability(|ctx| async move { Ok(ctx.payload.clone()) }),
            proof_bound_rpc_options_with_version(TEST_DESCRIPTOR_VERSION_V2),
        )
        .await;
        let disp = LocalAxonSessionDispatcher::new().with_local_runtime(Arc::clone(&rt));
        let (tx, mut rx) = mpsc::channel::<InvokeBidiUp>(4);
        let session_tx = SessionUpSender::new(tx);
        session_tx.set_negotiated_contract(1);
        let signed_ability =
            crate::daemon::axon_bridge::descriptor_ref::ability_descriptor_ref_for_wire(
                TEST_DEVICE_URA,
                "test.echo",
                &descriptor_binding_for_version(TEST_DESCRIPTOR_VERSION_V2),
            )
            .expect("versioned carrier-v1 descriptor ref");

        disp.handle_down(
            carrier_v1_call_signed_as(
                19,
                "test.echo",
                &signed_ability,
                br#"{"hello":"v2"}"#.to_vec(),
            ),
            &session_tx,
        )
        .await
        .expect("carrier-v1 dispatch succeeds with non-default descriptor version");

        let reply = tokio::time::timeout(Duration::from_secs(3), rx.recv())
            .await
            .expect("reply within 3s")
            .expect("reply produced");
        let Some(UpPayload::DispatchResult(result)) = reply.payload else {
            panic!(
                "v1 session must reply DispatchResult, got {:?}",
                reply.payload
            );
        };
        assert_eq!(result.call_id, 19);
        assert!(result.terminal);
        assert!(
            result.failure.is_none(),
            "carrier-v1 dispatch failed: {:?}",
            result.failure
        );
        assert_eq!(result.payload, br#"{"hello":"v2"}"#);
    }

    #[tokio::test]
    async fn carrier_v1_stream_terminal_frame_carries_receipt() {
        use axon_sdk::invocation::make_ability;

        let rt = executable_runtime();
        register_test_ability_with_options(
            &rt,
            "screen.subscribe",
            make_ability(|ctx| async move {
                ctx.emit_progress(
                    serde_json::to_vec(&json!({"seq": 1, "width": 640, "height": 360})).unwrap(),
                    "application/json",
                )
                .await?;
                Ok(Vec::new())
            }),
            proof_bound_stream_options(),
        )
        .await;
        let disp = LocalAxonSessionDispatcher::new().with_local_runtime(Arc::clone(&rt));
        let (tx, mut rx) = mpsc::channel::<InvokeBidiUp>(8);
        let session_tx = SessionUpSender::new(tx);
        session_tx.set_negotiated_contract(1);

        disp.handle_down(
            carrier_v1_explicit_test_call_with_mode(
                18,
                "screen.subscribe",
                b"{}".to_vec(),
                axon_sdk::invocation::CallMode::Stream,
            ),
            &session_tx,
        )
        .await
        .expect("carrier-v1 stream dispatch opens and forwards asynchronously");

        let admission = tokio::time::timeout(Duration::from_secs(3), rx.recv())
            .await
            .expect("admission reply within 3s")
            .expect("admission reply produced");
        let admission = match admission.payload {
            Some(UpPayload::DispatchResult(result)) => result,
            other => panic!("expected carrier-v1 admission result, got: {other:?}"),
        };
        assert_eq!(admission.call_id, 18);
        assert!(!admission.terminal);
        assert!(
            admission.failure.is_none(),
            "stream open returned carrier control failure before admission: {:?}",
            admission.failure
        );
        assert_eq!(
            admission
                .admission_receipt
                .expect("admission frame carries receipt")
                .state,
            axon_sdk::invocation::InvocationState::Admitted.to_wire_i32()
        );

        let progress = tokio::time::timeout(Duration::from_secs(3), rx.recv())
            .await
            .expect("progress reply within 3s")
            .expect("progress reply produced");
        let progress = match progress.payload {
            Some(UpPayload::DispatchResult(result)) => result,
            other => panic!("expected carrier-v1 progress result, got: {other:?}"),
        };
        assert_eq!(progress.call_id, 18);
        assert!(!progress.terminal, "first stream frame is progress");
        assert!(
            progress.admission_receipt.is_none() && progress.terminal_receipt.is_none(),
            "non-terminal progress frames do not close the receipt chain"
        );

        let terminal = tokio::time::timeout(Duration::from_secs(3), rx.recv())
            .await
            .expect("terminal reply within 3s")
            .expect("terminal reply produced");
        let terminal = match terminal.payload {
            Some(UpPayload::DispatchResult(result)) => result,
            other => panic!("expected carrier-v1 terminal result, got: {other:?}"),
        };
        assert_eq!(terminal.call_id, 18);
        assert!(terminal.terminal);
        assert!(
            terminal.failure.is_none(),
            "successful stream terminal must not carry failure: {:?}",
            terminal.failure
        );
        let receipt = terminal
            .terminal_receipt
            .expect("carrier-v1 terminal stream result must carry receipt");
        assert_eq!(
            receipt.state,
            axon_sdk::invocation::InvocationState::Completed.to_wire_i32()
        );
        assert!(
            disp.lifecycle_cancellations
                .contains_invocation_id(&receipt.invocation_id),
            "carrier-v1 stream lifecycle must remain registered for invocation.cancel"
        );
    }

    #[tokio::test]
    async fn carrier_v1_stream_descriptor_selects_stream_even_when_rpc_is_supported() {
        use axon_sdk::invocation::make_ability;

        let rt = executable_runtime();
        register_test_ability_with_options(
            &rt,
            "mixed.subscribe",
            make_ability(|ctx| async move {
                ctx.emit_progress(
                    serde_json::to_vec(&json!({"kind": "progress"})).unwrap(),
                    "application/json",
                )
                .await?;
                Ok(serde_json::to_vec(&json!({"kind": "done"})).unwrap())
            }),
            proof_bound_rpc_stream_options(),
        )
        .await;
        let disp = LocalAxonSessionDispatcher::new().with_local_runtime(Arc::clone(&rt));
        let (tx, mut rx) = mpsc::channel::<InvokeBidiUp>(8);
        let session_tx = SessionUpSender::new(tx);
        session_tx.set_negotiated_contract(1);

        disp.handle_down(
            carrier_v1_explicit_test_call_with_mode(
                21,
                "mixed.subscribe",
                b"{}".to_vec(),
                axon_sdk::invocation::CallMode::Stream,
            ),
            &session_tx,
        )
        .await
        .expect("mixed-mode stream dispatch opens asynchronously");

        let admission = tokio::time::timeout(Duration::from_secs(3), rx.recv())
            .await
            .expect("admission reply within 3s")
            .expect("admission reply produced");
        let admission = match admission.payload {
            Some(UpPayload::DispatchResult(result)) => result,
            other => panic!("expected carrier-v1 stream admission result, got: {other:?}"),
        };
        assert_eq!(admission.call_id, 21);
        assert!(
            !admission.terminal,
            "signed !stream descriptor must not be collapsed into an RPC terminal"
        );
        assert!(admission.admission_receipt.is_some());
        assert!(admission.terminal_receipt.is_none());

        let progress = tokio::time::timeout(Duration::from_secs(3), rx.recv())
            .await
            .expect("progress reply within 3s")
            .expect("progress reply produced");
        let progress = match progress.payload {
            Some(UpPayload::DispatchResult(result)) => result,
            other => panic!("expected carrier-v1 stream progress result, got: {other:?}"),
        };
        assert_eq!(progress.call_id, 21);
        assert!(!progress.terminal);
        let payload: serde_json::Value =
            serde_json::from_slice(&progress.payload).expect("progress payload is JSON");
        assert_eq!(payload["kind"], "progress");
    }

    #[tokio::test]
    async fn malformed_dispatch_json_returns_error() {
        let disp = LocalAxonSessionDispatcher::new();
        let (tx, _rx) = mpsc::channel::<InvokeBidiUp>(4);
        let session_tx = SessionUpSender::new(tx);

        let frame = InvokeBidiDown {
            sequence: 0,
            payload: Some(DownPayload::BinaryChunk(BinaryChunk {
                data: b"{not json}".to_vec(),
                ..BinaryChunk::default()
            })),
            ..InvokeBidiDown::default()
        };

        let err = disp
            .handle_down(frame, &session_tx)
            .await
            .expect_err("malformed JSON must surface as SessionDispatchError");
        match err {
            SessionDispatchError::Other(msg) => {
                assert!(
                    msg.contains("not valid SessionDispatch JSON"),
                    "error must cite JSON decode; got: {msg}"
                );
            }
        }
    }

    // ── Device-mode boot wiring exposes baseline locomotion ───────────────
    //
    // This test uses the canonical carrier-v1 DispatchCall/DispatchResult
    // path. The retired JSON Dispatch frame must not reappear merely to keep
    // a device-mode test alive.

    fn build_real_daemon_registry_with_runtime(
        local_runtime: Option<Arc<axon_sdk::invocation::LocalRuntime>>,
    ) -> Arc<crate::daemon::ability::dispatch::AxonAbilityCatalog> {
        use crate::daemon::execution::loop_instance::LoopService;
        use crate::daemon::execution::mission::discuss::DiscussService;
        use crate::daemon::execution::permission::PermissionService;
        use crate::daemon::execution::schedule::ScheduleService;
        use crate::daemon::execution::session::SessionService;
        let agents = Default::default();
        let authority_context =
            crate::daemon::ability::dispatch::AbilityAuthorityContext::for_device_authority_root(
                TEST_DEVICE_URA,
            )
            .expect("test device URA is a valid device authority root");
        let mut config =
            crate::daemon::ability::catalog::RegistryBuildConfig::new_with_authority_context(
                crate::daemon::ability::catalog::RegistryBuildServices::new(
                    Arc::new(SessionService::new()),
                    Arc::new(PermissionService::new()),
                    Arc::new(DiscussService::new()),
                    Arc::new(ScheduleService::new()),
                    Arc::new(LoopService::new()),
                ),
                &agents,
                authority_context,
            );
        config.local_runtime = local_runtime;
        crate::daemon::ability::catalog::build_registry_with_services_result(config)
            .expect("assemble local session dispatcher test catalog")
            .catalog
    }

    #[tokio::test]
    async fn device_mode_dispatcher_executes_fs_read_through_baseline_locomotion_registry() {
        let tmp = tempfile::tempdir().expect("tempdir");
        let target = tmp.path().join("hello.txt");
        std::fs::write(&target, "device-B-bytes-from-real-fs-read").expect("seed temp file");

        let rt = executable_runtime();
        let _registry = build_real_daemon_registry_with_runtime(Some(Arc::clone(&rt)));
        let disp = LocalAxonSessionDispatcher::new().with_local_runtime(Arc::clone(&rt));
        let (tx, mut rx) = mpsc::channel::<InvokeBidiUp>(4);
        let session_tx = SessionUpSender::new(tx);

        let args = serde_json::json!({
            "resource_ref": crate::daemon::resources::files::resource_ref_for_local_path(
                &target,
                crate::daemon::resources::files::FilesystemResourceCapability::Read,
            )
            .expect("local fs ResourceRef"),
            "encoding": "utf8",
        });
        let frame = carrier_v1_call(
            42,
            "fs.read",
            serde_json::to_vec(&args).expect("encode args"),
        );
        session_tx.set_negotiated_contract(1);

        disp.handle_down(frame, &session_tx)
            .await
            .expect("fs.read dispatches through device-mode registry");

        let reply = rx.recv().await.expect("reply produced");
        let result = match reply.payload {
            Some(UpPayload::DispatchResult(result)) => result,
            other => panic!("expected canonical DispatchResult reply, got: {other:?}"),
        };
        assert_eq!(result.call_id, 42);
        assert!(result.terminal, "fs.read RPC reply is terminal");
        assert!(
            result.failure.is_none(),
            "fs.read must succeed: {:?}",
            result.failure
        );
        let value: serde_json::Value =
            serde_json::from_slice(&result.payload).expect("payload decodes as JSON");
        let bytes = value
            .get("content")
            .and_then(|v| v.as_str())
            .or_else(|| value.get("text").and_then(|v| v.as_str()))
            .expect("fs.read response carries content/text field");
        assert_eq!(
            bytes, "device-B-bytes-from-real-fs-read",
            "payload bytes must come from the device-side filesystem, not a daemon-internal stub"
        );
    }

    #[test]
    #[cfg(feature = "remote-desktop")]
    fn remote_desktop_bidi_output_preserves_json_frame_payload() {
        let dispatcher = remote_desktop_wire_dispatcher();
        let mapped = dispatcher
            .map_remote_bidi_output(
                91,
                "remote_desktop.attach",
                &json!({
                    "type": "frame",
                    "seq": 3,
                    "image_bytes_b64": "abc",
                }),
            )
            .expect("map succeeds")
            .expect("frame forwards");

        assert_eq!(mapped.call_id, 91);
        assert!(mapped.failure.is_none());
        let payload: Value = serde_json::from_slice(&mapped.payload).expect("json payload");
        assert_eq!(payload["type"], "frame");
        assert_eq!(payload["seq"], 3);
        assert_eq!(payload["image_bytes_b64"], "abc");
    }

    #[test]
    #[cfg(feature = "remote-desktop")]
    fn remote_desktop_bidi_closed_frame_remains_data_until_runtime_terminal() {
        let dispatcher = remote_desktop_wire_dispatcher();
        let mapped = dispatcher
            .map_remote_bidi_output(
                92,
                "remote_desktop.attach",
                &json!({
                    "type": "closed",
                    "reason": "client_closed",
                }),
            )
            .expect("map succeeds")
            .expect("closed forwards");

        assert_eq!(mapped.call_id, 92);
        assert!(mapped.failure.is_none());
    }

    #[test]
    #[cfg(feature = "remote-desktop")]
    fn remote_desktop_bidi_error_frame_is_typed_data_until_runtime_terminal() {
        let dispatcher = remote_desktop_wire_dispatcher();
        let mapped = dispatcher
            .map_remote_bidi_output(
                93,
                "remote_desktop.attach",
                &json!({
                    "type": "error",
                    "code": "permission_denied",
                    "message": "screen capture permission denied",
                }),
            )
            .expect("map succeeds")
            .expect("error forwards");

        assert_eq!(mapped.call_id, 93);
        let failure = mapped.failure.expect("typed failure");
        assert_eq!(failure.code, "PERMISSION_DENIED");
        assert_eq!(
            failure.message,
            "permission_denied: screen capture permission denied"
        );
        let payload: Value = serde_json::from_slice(&mapped.payload).expect("json payload");
        assert_eq!(payload["type"], "error");
    }

    #[cfg(feature = "remote-desktop")]
    fn remote_desktop_wire_dispatcher() -> LocalAxonSessionDispatcher {
        LocalAxonSessionDispatcher::new().with_ability_wire_registry(Arc::new(
            crate::daemon::ability::wire::AbilityWireRegistry::for_test_plugin_bidi([(
                "remote_desktop.attach".to_string(),
                crate::daemon::ability::wire::AbilityBidiWireKind::JsonFrames,
            )]),
        ))
    }
}
