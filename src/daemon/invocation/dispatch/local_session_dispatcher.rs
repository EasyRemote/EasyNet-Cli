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

use base64::{engine::general_purpose::STANDARD as B64, Engine as _};
use easynet_axon::invocation::{BidiInputFrame, BidiInputSender};
use serde_json::{json, Value};
#[cfg(test)]
use tokio::sync::mpsc;
use tokio_util::sync::CancellationToken;

use super::descriptor_binding::RuntimeBoundAbility;
use super::invocation_wire::target_ura_from_envelope;
use crate::daemon::invocation::admission::admission_facade::AdmissionFacade;
use crate::daemon::invocation::bidi::session_initiator::{
    SessionDispatchError, SessionFrameDispatcher, SessionUpSender, SESSION_STREAM_ID,
};
use crate::daemon::invocation::bidi::session_wire::{
    call_id_hex, SessionContentEnvelope, SessionDispatch,
};
use crate::daemon::invocation::bidi::state::session_failure::SessionFailure;
use easynet_axon::pb::axon::v1::invoke_bidi_down::Payload as DownPayload;
use easynet_axon::pb::axon::v1::invoke_bidi_up::Payload as UpPayload;
#[cfg(test)]
use easynet_axon::pb::axon::v1::InvokeBidiUp;
use easynet_axon::pb::axon::v1::{BinaryChunk, InvokeBidiDown};

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
    local_runtime: Option<Arc<easynet_axon::invocation::LocalRuntime>>,
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
}

type LocalBidiWireKind = crate::daemon::ability::wire::AbilityBidiWireKind;

fn receipt_to_session_wire(
    receipt: &easynet_axon::invocation::SignedInvocationReceipt,
) -> Result<easynet_axon::pb::axon::v1::InvocationReceipt, SessionDispatchError> {
    easynet_axon::invocation::wire::receipt_to_wire(receipt).map_err(|error| {
        SessionDispatchError::Other(format!(
            "canonical receipt projection failed before session relay: {error}"
        ))
    })
}

fn unary_checkpoints_to_session_wire(
    outcome: &crate::daemon::axon_bridge::dispatch_shim::RpcDispatchOutcome,
) -> Result<
    (
        Option<easynet_axon::pb::axon::v1::InvocationReceipt>,
        Option<easynet_axon::pb::axon::v1::InvocationReceipt>,
    ),
    SessionDispatchError,
> {
    match (
        outcome.admission_receipt.as_ref(),
        outcome.terminal_receipt.as_ref(),
    ) {
        (Some(admission), Some(terminal)) => {
            if admission.state() != easynet_axon::invocation::InvocationState::Admitted {
                return Err(SessionDispatchError::Other(
                    "canonical unary admission checkpoint has a non-admitted state".to_string(),
                ));
            }
            if terminal.state() != outcome.state
                || !matches!(
                    terminal.state(),
                    easynet_axon::invocation::InvocationState::Completed
                        | easynet_axon::invocation::InvocationState::Failed
                        | easynet_axon::invocation::InvocationState::TimedOut
                        | easynet_axon::invocation::InvocationState::Cancelled
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

struct RemoteBidiOpenRequest {
    call_id: u64,
    callee_ura: Option<String>,
    subject_ura: Option<String>,
    ability: String,
    args: Vec<u8>,
    args_content_envelope: SessionContentEnvelope,
    metadata: HashMap<String, String>,
}

#[derive(Clone, Copy)]
enum BidiReceiptCheckpoint<'a> {
    Admission(&'a easynet_axon::invocation::SignedInvocationReceipt),
    Terminal(&'a easynet_axon::invocation::SignedInvocationReceipt),
}

#[derive(Debug, Clone, PartialEq, Eq)]
enum SessionSelfTargetSubject {
    Explicit(String),
    CalleeSelfTarget(String),
}

impl SessionSelfTargetSubject {
    fn from_optional(subject_ura: Option<&str>, callee_ura: &str) -> Result<Self, String> {
        match subject_ura.map(str::trim).filter(|value| !value.is_empty()) {
            Some(subject) => {
                Self::validate(subject, "subject_ura")?;
                Ok(Self::Explicit(subject.to_string()))
            }
            None => {
                Self::validate(callee_ura, "callee_ura")?;
                Ok(Self::CalleeSelfTarget(callee_ura.to_string()))
            }
        }
    }

    fn as_str(&self) -> &str {
        match self {
            Self::Explicit(subject) | Self::CalleeSelfTarget(subject) => subject,
        }
    }

    fn validate(value: &str, field: &str) -> Result<(), String> {
        let value = value.trim();
        if value.is_empty() {
            return Err(format!("session.open: {field} must not be empty"));
        }
        crate::core::ura::parse_ura(value)
            .map_err(|err| format!("session.open: {field} `{value}` is not a valid URA: {err}"))
            .map(|_| ())
    }
}

fn carrier_v1_stream_control_failure(
    call_id: u64,
    code: &'static str,
    message: impl Into<String>,
) -> easynet_axon::pb::axon::v1::DispatchResult {
    easynet_axon::pb::axon::v1::DispatchResult {
        call_id,
        payload: Vec::new(),
        terminal: false,
        failure: Some(easynet_axon::pb::axon::v1::Error {
            code: code.to_string(),
            message: message.into(),
            retryable: false,
            ..Default::default()
        }),
        ..Default::default()
    }
}

impl LocalAxonSessionDispatcher {
    fn non_empty_ura(raw: Option<&str>) -> Option<&str> {
        raw.map(str::trim).filter(|value| !value.is_empty())
    }

    fn self_target_subject(
        subject_ura: Option<&str>,
        callee_ura: &str,
    ) -> Result<SessionSelfTargetSubject, String> {
        SessionSelfTargetSubject::from_optional(subject_ura, callee_ura)
    }

    /// Canonical dispatch: the frame already is the invocation, so neither
    /// caller identity nor request fields are reconstructed at this hop.
    async fn handle_carrier_v1_dispatch(
        &self,
        call: easynet_axon::pb::axon::v1::DispatchCall,
        outbound: &SessionUpSender,
    ) -> Result<(), SessionDispatchError> {
        use easynet_axon::pb::axon::v1::DispatchResult as PbDispatchResult;

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
        crate::op_event!(
            component = local_session_dispatcher,
            kind = received_carrier_v1_dispatch,
            call_id = call_id,
            ability = request.function_name,
        );
        let Some(envelope) = request.envelope else {
            return Err(SessionDispatchError::Other(
                "carrier-v1 DispatchCall request missing envelope".to_string(),
            ));
        };
        let Some(runtime) = self.local_runtime.clone() else {
            return Err(SessionDispatchError::Other(
                "carrier-v1 dispatch: Axon LocalRuntime is not wired".to_string(),
            ));
        };
        let function_name = request.function_name.clone();
        let target_ura = target_ura_from_envelope(Some(&envelope), "carrier-v1 DispatchCall")
            .map_err(|status| SessionDispatchError::Other(status.message().to_string()))?;
        self.sync_external_signed_caller_key(&envelope).await?;
        let bound_ability = RuntimeBoundAbility::from_wire_target(
            "carrier-v1 DispatchCall",
            &runtime,
            &target_ura,
            &function_name,
        )
        .await
        .map_err(|status| SessionDispatchError::Other(status.message().to_string()))?;
        let carrier_v1_stream = bound_ability
            .supports_mode(easynet_axon::invocation::CallMode::Stream)
            && !bound_ability.supports_mode(easynet_axon::invocation::CallMode::Rpc);
        let call_mode = if carrier_v1_stream {
            easynet_axon::invocation::CallMode::Stream
        } else {
            easynet_axon::invocation::CallMode::Rpc
        };
        let descriptor_ref = match bound_ability
            .signed_descriptor_ref_from_metadata(
                "carrier-v1 DispatchCall",
                &target_ura,
                call_mode,
                &request.metadata,
            )
            .map_err(|status| SessionDispatchError::Other(status.message().to_string()))?
        {
            Some(descriptor_ref) => descriptor_ref,
            None => bound_ability
                .descriptor_ref_for_mode("carrier-v1 DispatchCall", &target_ura, call_mode, None)
                .map_err(|status| SessionDispatchError::Other(status.message().to_string()))?,
        };
        let wire = crate::daemon::axon_bridge::dispatch_shim::external_signed_from_wire_parts(
            envelope,
            descriptor_ref.into_descriptor_ref(),
            request.arguments,
            request.metadata,
        )
        .map_err(|err| {
            SessionDispatchError::Other(format!("build carrier-v1 signed dispatch: {err}"))
        })?;

        // ── step-3c: server-stream over carrier ──────────────────────
        // A stream-mode ability (modes.stream && !modes.rpc) emits many
        // non-terminal frames; draining it through the unary path below
        // would collapse the stream to a single terminal DispatchResult.
        // Open the stream and hand the handle to a forwarder that chains
        // typed `DispatchResult` chunks.
        // Carrier-v1 preserves caller identity through the descriptor-bound
        // signature.
        if carrier_v1_stream {
            return self
                .handle_carrier_v1_stream_open(call_id, wire, outbound)
                .await;
        }

        let outcome = crate::daemon::axon_bridge::dispatch_shim::dispatch_rpc_admitted(
            &runtime,
            wire,
            &self.lifecycle_cancellations,
        )
        .await;

        let failure = outcome
            .error
            .as_ref()
            .map(|e| easynet_axon::pb::axon::v1::Error {
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
    /// chunks. The envelope in `wire` has already passed daemon admission; a
    /// failure to open is reported as a non-terminal carrier control failure.
    async fn handle_carrier_v1_stream_open(
        &self,
        call_id: u64,
        wire: crate::daemon::axon_bridge::dispatch_shim::WireDispatch,
        outbound: &SessionUpSender,
    ) -> Result<(), SessionDispatchError> {
        let Some(runtime) = self.local_runtime.clone() else {
            return Err(SessionDispatchError::Other(
                "carrier-v1 stream: Axon LocalRuntime is not wired".to_string(),
            ));
        };
        let lifecycle_envelope = wire.envelope.clone();
        let handle =
            match crate::daemon::axon_bridge::dispatch_shim::open_stream_admitted(&runtime, wire)
                .await
            {
                Ok(handle) => handle,
                Err(err) => {
                    let reply = carrier_v1_stream_control_failure(
                        call_id,
                        "STREAM_OPEN_FAILED",
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
        let lifecycle_handle = handle.handle().clone();
        let lifecycle_key = match self
            .lifecycle_cancellations
            .register(&lifecycle_envelope, lifecycle_handle.clone())
        {
            Ok(key) => key,
            Err(err) => {
                let _ = handle
                    .cancel("lifecycle cancellation registration failed")
                    .await;
                let reply = carrier_v1_stream_control_failure(
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

        Self::spawn_carrier_v1_stream_forwarder(
            call_id,
            handle,
            outbound.clone(),
            Arc::clone(&self.remote_stream_sessions),
            self.lifecycle_cancellations.clone(),
            lifecycle_key,
            lifecycle_handle,
        );
        Ok(())
    }

    /// Carrier-v1 twin of [`Self::spawn_stream_forwarder`]. Same drain
    /// loop, cancellation registration, and runtime-cancel propagation;
    /// the only difference is the wire shape — frames go out as
    /// `DispatchResult` chunks (carrier-v1) rather than
    /// `SessionDispatch::Result` (carrier-v0). The terminal frame
    /// carries the callee-signed execution receipt
    /// (`DispatchResult.terminal_receipt` is REQUIRED on terminal frames), pulled
    /// from the streaming handle the same way the unary arm projects
    /// `terminal_receipt`.
    fn spawn_carrier_v1_stream_forwarder(
        call_id: u64,
        mut handle: easynet_axon::invocation::StreamingInvocationHandle,
        outbound: SessionUpSender,
        sessions: Arc<Mutex<HashMap<u64, CancellationToken>>>,
        lifecycle_cancellations: crate::daemon::invocation::dispatch::cancellation::InvocationCancellationRegistry,
        lifecycle_key: String,
        lifecycle_handle: easynet_axon::invocation::InvocationHandle,
    ) {
        use easynet_axon::pb::axon::v1::DispatchResult as PbDispatchResult;

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
                    if handle.finalized().await.is_ok() {
                        lifecycle_cancellations
                            .mark_terminal(&lifecycle_key, lifecycle_handle.clone());
                    }
                    let _ = outbound
                        .send_payload(UpPayload::DispatchResult(
                            carrier_v1_stream_control_failure(
                                call_id,
                                "CANONICAL_ADMISSION_REQUIRED",
                                error.to_string(),
                            ),
                        ))
                        .await;
                    return;
                }
            };
            let admission_wire = match receipt_to_session_wire(&admission) {
                Ok(receipt) => receipt,
                Err(error) => {
                    let _ = handle.cancel("canonical admission projection failed").await;
                    if handle.finalized().await.is_ok() {
                        lifecycle_cancellations
                            .mark_terminal(&lifecycle_key, lifecycle_handle.clone());
                    }
                    let _ = outbound
                        .send_payload(UpPayload::DispatchResult(
                            carrier_v1_stream_control_failure(
                                call_id,
                                "CANONICAL_ADMISSION_PROJECTION_FAILED",
                                error.to_string(),
                            ),
                        ))
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
                let _ = handle
                    .cancel("session stream closed before admission")
                    .await;
                if handle.finalized().await.is_ok() {
                    lifecycle_cancellations.mark_terminal(&lifecycle_key, lifecycle_handle);
                }
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
                            match handle.finalized().await {
                                Ok(finalized) => Some(finalized),
                                Err(error) => {
                                    let _ = outbound
                                        .send_payload(UpPayload::DispatchResult(
                                            carrier_v1_stream_control_failure(
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
                                Ok(receipt) => {
                                    lifecycle_cancellations
                                        .mark_terminal(&lifecycle_key, lifecycle_handle.clone());
                                    Some(receipt)
                                }
                                Err(error) => {
                                    let _ = outbound
                                        .send_payload(UpPayload::DispatchResult(
                                            carrier_v1_stream_control_failure(
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
                            terminal,
                            terminal_receipt,
                            failure: finalized
                                .as_ref()
                                .and_then(|value| value.failure.as_ref())
                                .map(easynet_axon::invocation::wire::error_to_wire),
                            ..PbDispatchResult::default()
                        }
                    }
                    Err(err) => {
                        sent_terminal = true;
                        let finalized = match handle.finalized().await {
                            Ok(finalized) => finalized,
                            Err(error) => {
                                let _ = outbound
                                    .send_payload(UpPayload::DispatchResult(
                                        carrier_v1_stream_control_failure(
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
                                Ok(receipt) => {
                                    lifecycle_cancellations
                                        .mark_terminal(&lifecycle_key, lifecycle_handle.clone());
                                    receipt
                                }
                                Err(error) => {
                                    let _ = outbound
                                        .send_payload(UpPayload::DispatchResult(
                                            carrier_v1_stream_control_failure(
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
                            failure: Some(easynet_axon::invocation::wire::error_to_wire(
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
                let reply = carrier_v1_stream_control_failure(
                    call_id,
                    "STREAM_ENDED_WITHOUT_TERMINAL",
                    message,
                );
                let _ = outbound
                    .send_payload(UpPayload::DispatchResult(reply))
                    .await;
            }
            // Cancellation must reach the RUNTIME task, not just this
            // forwarder — dropping the handle alone leaves the ability's
            // emit loop alive holding its stream source. cancel() is
            // idempotent and a no-op on already-terminal invocations.
            if let Err(err) = handle.cancel("session stream closed").await {
                let err_msg = err.to_string();
                crate::op_event!(
                    component = local_session_dispatcher,
                    kind = stream_runtime_cancel_failed,
                    call_id = call_id,
                    error = err_msg,
                );
            }
            if cancelled && handle.finalized().await.is_ok() {
                lifecycle_cancellations.mark_terminal(&lifecycle_key, lifecycle_handle);
            }
            let mut guard = match sessions.lock() {
                Ok(g) => g,
                Err(poisoned) => poisoned.into_inner(),
            };
            guard.remove(&call_id);
        });
    }

    fn session_failure(reason: &str) -> SessionFailure {
        SessionFailure::from_reason(reason, "INVOCATION_FAILED", false)
    }

    fn session_failure_from_handler_code(
        explicit_code: Option<&str>,
        reason: &str,
    ) -> SessionFailure {
        explicit_code
            .map(str::trim)
            .filter(|code| !code.is_empty())
            .map(|code| SessionFailure::from_explicit(code, reason, false))
            .unwrap_or_else(|| Self::session_failure(reason))
    }

    fn session_error_result(call_id: u64, message: impl Into<String>) -> SessionDispatch {
        let message = message.into();
        SessionDispatch::Result {
            call_id,
            payload: Vec::new(),
            terminal: true,
            failure: Some(Self::session_failure(&message)),
            error: Some(message),
            request_id: None,
        }
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
            ability_wire: Arc::new(crate::daemon::ability::wire::AbilityWireRegistry::core()),
            device_trust_sync: None,
            admission: None,
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

    #[must_use]
    pub fn with_admission_policy(mut self, admission: AdmissionFacade) -> Self {
        self.admission = Some(admission);
        self
    }

    async fn sync_external_signed_caller_key(
        &self,
        envelope: &easynet_axon::pb::axon::v1::Envelope,
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
            return Err(SessionDispatchError::Other(format!(
                "carrier-v1 external signed caller `{caller_ura}` cannot warm trust anchor: DeviceTrustSync is not wired"
            )));
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

    /// Attach the shared Axon `LocalRuntime` used by canonical session
    /// dispatch and local streaming handlers.
    #[must_use]
    pub fn with_local_runtime(
        mut self,
        runtime: Arc<easynet_axon::invocation::LocalRuntime>,
    ) -> Self {
        self.local_runtime = Some(runtime);
        self
    }

    async fn send_dispatch_up(
        outbound: &SessionUpSender,
        dispatch: &SessionDispatch,
    ) -> Result<(), SessionDispatchError> {
        let payload = dispatch.encode_frame().map_err(|err| {
            SessionDispatchError::Other(format!("encode SessionDispatch frame: {err}"))
        })?;
        outbound
            .send_binary_chunk(BinaryChunk {
                stream_id: SESSION_STREAM_ID,
                data: payload,
                ..BinaryChunk::default()
            })
            .await
            .map_err(|_| SessionDispatchError::Other("outbound channel closed".to_string()))
    }

    fn file_transfer_terminal_error(call_id: u64, message: impl Into<String>) -> SessionDispatch {
        Self::session_error_result(call_id, message)
    }

    fn validate_session_args_content(
        ability: &str,
        content: &SessionContentEnvelope,
    ) -> Result<(), String> {
        if content.is_encrypted() {
            return Err(format!(
                "session.open: ability `{ability}` received encrypted args \
                 (encryption={}, key_id={:?}) but no session decryptor is wired",
                content.encryption, content.key_id
            ));
        }
        if !content.content_type.is_empty() && content.content_type != "application/json" {
            return Err(format!(
                "session.open: ability `{ability}` received unsupported args content_type {:?}",
                content.content_type
            ));
        }
        if !content.encoding.is_empty() && content.encoding != "identity" {
            return Err(format!(
                "session.open: ability `{ability}` received unsupported args encoding {:?}",
                content.encoding
            ));
        }
        Ok(())
    }

    fn map_remote_file_transfer_output(
        call_id: u64,
        value: &Value,
    ) -> Result<Option<SessionDispatch>, SessionDispatchError> {
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
                Ok(Some(SessionDispatch::Result {
                    call_id,
                    payload: raw,
                    terminal: false,
                    failure: None,
                    error: None,
                    request_id: None,
                }))
            }
            Some("complete") => {
                let payload = serde_json::to_vec(value).map_err(|err| {
                    SessionDispatchError::Other(format!(
                        "encode file_transfer completion payload: {err}"
                    ))
                })?;
                Ok(Some(SessionDispatch::Result {
                    call_id,
                    payload,
                    terminal: false,
                    failure: None,
                    error: None,
                    request_id: None,
                }))
            }
            Some("error") => {
                let code = value.get("code").and_then(Value::as_str);
                let reason = match (code, value.get("message").and_then(Value::as_str)) {
                    (Some(code), Some(message))
                        if !code.trim().is_empty() && !message.trim().is_empty() =>
                    {
                        format!("{code}: {message}")
                    }
                    (_, Some(message)) if !message.trim().is_empty() => message.to_string(),
                    (Some(code), _) if !code.trim().is_empty() => code.to_string(),
                    _ => "file_transfer handler returned error".to_string(),
                };
                let payload = serde_json::to_vec(value).map_err(|err| {
                    SessionDispatchError::Other(format!(
                        "encode file_transfer error payload: {err}"
                    ))
                })?;
                Ok(Some(SessionDispatch::Result {
                    call_id,
                    payload,
                    terminal: false,
                    failure: Some(Self::session_failure_from_handler_code(code, &reason)),
                    error: Some(reason),
                    request_id: None,
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
    ) -> Result<Option<SessionDispatch>, SessionDispatchError> {
        match value.get("type").and_then(Value::as_str) {
            Some("stdout") => {
                let data_b64 = value.get("data").and_then(Value::as_str).ok_or_else(|| {
                    SessionDispatchError::Other("pty stdout frame missing `data`".to_string())
                })?;
                let raw = B64.decode(data_b64).map_err(|err| {
                    SessionDispatchError::Other(format!("pty stdout base64 decode failed: {err}"))
                })?;
                Ok(Some(SessionDispatch::Result {
                    call_id,
                    payload: raw,
                    terminal: false,
                    failure: None,
                    error: None,
                    request_id: None,
                }))
            }
            Some("exit") => Ok(Some(SessionDispatch::Result {
                call_id,
                payload: Vec::new(),
                terminal: false,
                failure: None,
                error: None,
                request_id: None,
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
    ) -> Result<Option<SessionDispatch>, SessionDispatchError> {
        Self::map_remote_bidi_output_with(&self.ability_wire, call_id, ability, value)
    }

    fn map_remote_bidi_output_with(
        registry: &crate::daemon::ability::wire::AbilityWireRegistry,
        call_id: u64,
        ability: &str,
        value: &Value,
    ) -> Result<Option<SessionDispatch>, SessionDispatchError> {
        if ability == crate::daemon::ability::builtins::device_control::terminal::attach::ABILITY_PTY_SESSION_ATTACH {
            return Self::map_remote_pty_output(call_id, value);
        }
        if Self::is_json_frame_bidi_with(registry, ability) {
            let frame_type = value.get("type").and_then(Value::as_str);
            let payload = serde_json::to_vec(value).map_err(|err| {
                SessionDispatchError::Other(format!("plugin JSON-frame bidi encode failed: {err}"))
            })?;
            let (failure, error) = if frame_type == Some("error") {
                let reason = json_frame_error_reason(value);
                let code = value.get("code").and_then(Value::as_str);
                (
                    Some(Self::session_failure_from_handler_code(code, &reason)),
                    Some(reason),
                )
            } else {
                (None, None)
            };
            return Ok(Some(SessionDispatch::Result {
                call_id,
                payload,
                terminal: false,
                failure,
                error,
                request_id: None,
            }));
        }
        Self::map_remote_file_transfer_output(call_id, value)
    }

    /// Carrier-v1 bidi open (DEC-F004 / step 3b): `DispatchCall` with
    /// `open_bidi = true` is the retiring `SessionDispatch::BidiOpen`
    /// collapsed into the canonical frame — the request IS the
    /// invocation, so the open admits through the same wire-parts
    /// path as the unary arm instead of re-deriving identity from
    /// loose URA fields. Open errors and stream output both reply
    /// per the session's negotiated contract.
    async fn handle_carrier_v1_bidi_open(
        &self,
        call_id: u64,
        request: easynet_axon::pb::axon::v1::InvokeRequest,
        outbound: &SessionUpSender,
    ) -> Result<(), SessionDispatchError> {
        let ability = request.function_name.clone();
        crate::op_event!(
            component = local_session_dispatcher,
            kind = received_carrier_v1_bidi_open,
            call_id = call_id,
            ability = ability,
        );
        if !local_is_bidi_wire_ability(&self.ability_wire, &ability) {
            return Self::send_bidi_result(
                outbound,
                &Self::session_error_result(
                    call_id,
                    format!("remote bidi ability `{ability}` is not wired on session.open"),
                ),
                None,
            )
            .await;
        }
        let Some(envelope) = request.envelope else {
            return Self::send_bidi_result(
                outbound,
                &Self::session_error_result(call_id, "carrier-v1 bidi open missing envelope"),
                None,
            )
            .await;
        };
        let Some(runtime) = self.local_runtime.as_ref() else {
            return Self::send_bidi_result(
                outbound,
                &Self::session_error_result(
                    call_id,
                    "session.open: LocalRuntime is not wired for remote bidi",
                ),
                None,
            )
            .await;
        };
        let target_ura = match target_ura_from_envelope(Some(&envelope), "carrier-v1 BidiOpen") {
            Ok(target_ura) => target_ura,
            Err(status) => {
                return Self::send_bidi_result(
                    outbound,
                    &Self::session_error_result(call_id, status.message()),
                    None,
                )
                .await;
            }
        };
        if let Err(err) = self.sync_external_signed_caller_key(&envelope).await {
            return Self::send_bidi_result(
                outbound,
                &Self::session_error_result(call_id, err.to_string()),
                None,
            )
            .await;
        }
        let bound_ability = match RuntimeBoundAbility::from_wire_target(
            "carrier-v1 BidiOpen",
            runtime,
            &target_ura,
            &ability,
        )
        .await
        {
            Ok(bound_ability) => bound_ability,
            Err(status) => {
                return Self::send_bidi_result(
                    outbound,
                    &Self::session_error_result(call_id, status.message()),
                    None,
                )
                .await;
            }
        };
        let descriptor_ref = match bound_ability.signed_descriptor_ref_from_metadata(
            "carrier-v1 BidiOpen",
            &target_ura,
            easynet_axon::invocation::CallMode::Bidi,
            &request.metadata,
        ) {
            Ok(Some(ref_)) => ref_,
            Ok(None) => match bound_ability.descriptor_ref_for_mode(
                "carrier-v1 BidiOpen",
                &target_ura,
                easynet_axon::invocation::CallMode::Bidi,
                None,
            ) {
                Ok(ref_) => ref_,
                Err(status) => {
                    return Self::send_bidi_result(
                        outbound,
                        &Self::session_error_result(call_id, status.message()),
                        None,
                    )
                    .await;
                }
            },
            Err(status) => {
                return Self::send_bidi_result(
                    outbound,
                    &Self::session_error_result(call_id, status.message()),
                    None,
                )
                .await;
            }
        };
        let wire = match crate::daemon::axon_bridge::dispatch_shim::external_signed_from_wire_parts(
            envelope,
            descriptor_ref.into_descriptor_ref(),
            request.arguments,
            request.metadata,
        ) {
            Ok(wire) => wire,
            Err(err) => {
                return Self::send_bidi_result(
                    outbound,
                    &Self::session_error_result(
                        call_id,
                        format!("build carrier-v1 admitted bidi open: {err}"),
                    ),
                    None,
                )
                .await;
            }
        };
        let lifecycle_envelope = wire.envelope.clone();
        let handle = match crate::daemon::axon_bridge::dispatch_shim::open_bidi_admitted(
            runtime, wire,
        )
        .await
        {
            Ok(handle) => handle,
            Err(err) => {
                return Self::send_bidi_result(
                    outbound,
                    &Self::session_error_result(
                        call_id,
                        format!("session.open: remote bidi open failed: {err}"),
                    ),
                    None,
                )
                .await;
            }
        };
        self.register_remote_bidi(
            call_id,
            &ability,
            handle,
            outbound,
            Some(lifecycle_envelope),
        )
        .await
    }

    /// Device → hub bidi stream frame, sent per the session's
    /// negotiated contract: a carrier-v1 session gets the proto
    /// `DispatchResult`, a v0 session the retiring JSON `Result`.
    /// Receipt checkpoints are projected into their corresponding wire
    /// fields; admission is never represented as terminal proof.
    async fn send_bidi_result(
        outbound: &SessionUpSender,
        dispatch: &SessionDispatch,
        checkpoint: Option<BidiReceiptCheckpoint<'_>>,
    ) -> Result<(), SessionDispatchError> {
        if outbound.carrier_v1() {
            if let SessionDispatch::Result {
                call_id,
                payload,
                terminal,
                error,
                failure,
                request_id: _,
            } = dispatch
            {
                use easynet_axon::pb::axon::v1::DispatchResult as PbDispatchResult;
                let failure = failure
                    .as_ref()
                    .map(|f| easynet_axon::pb::axon::v1::Error {
                        code: f.code.clone(),
                        message: f.message.clone(),
                        retryable: f.retryable,
                        ..Default::default()
                    })
                    .or_else(|| {
                        error
                            .as_ref()
                            .map(|message| easynet_axon::pb::axon::v1::Error {
                                code: "INVOCATION_FAILED".to_string(),
                                message: message.clone(),
                                retryable: false,
                                ..Default::default()
                            })
                    });
                let (admission_receipt, terminal_receipt) = match checkpoint {
                    Some(BidiReceiptCheckpoint::Admission(receipt)) => {
                        (Some(receipt_to_session_wire(receipt)?), None)
                    }
                    Some(BidiReceiptCheckpoint::Terminal(receipt)) => {
                        (None, Some(receipt_to_session_wire(receipt)?))
                    }
                    None => (None, None),
                };
                return outbound
                    .send_payload(UpPayload::DispatchResult(PbDispatchResult {
                        call_id: *call_id,
                        payload: payload.clone(),
                        terminal: *terminal,
                        admission_receipt,
                        terminal_receipt,
                        failure,
                    }))
                    .await
                    .map_err(|_| {
                        SessionDispatchError::Other("session up channel closed".to_string())
                    });
            }
        }
        Self::send_dispatch_up(outbound, dispatch).await
    }

    async fn open_remote_bidi(
        &self,
        request: RemoteBidiOpenRequest,
        outbound: &SessionUpSender,
    ) -> Result<(), SessionDispatchError> {
        let RemoteBidiOpenRequest {
            call_id,
            callee_ura,
            subject_ura,
            ability,
            args,
            args_content_envelope,
            metadata: _metadata,
        } = request;
        let ability = ability.as_str();
        if !local_is_bidi_wire_ability(&self.ability_wire, ability) {
            return Self::send_dispatch_up(
                outbound,
                &Self::file_transfer_terminal_error(
                    call_id,
                    format!("remote bidi ability `{ability}` is not wired on session.open"),
                ),
            )
            .await;
        }

        if let Err(reason) = Self::validate_session_args_content(ability, &args_content_envelope) {
            return Self::send_dispatch_up(
                outbound,
                &Self::file_transfer_terminal_error(call_id, reason),
            )
            .await;
        }

        let Some(runtime) = self.local_runtime.as_ref() else {
            return Self::send_dispatch_up(
                outbound,
                &Self::file_transfer_terminal_error(
                    call_id,
                    "session.open: LocalRuntime is not wired for remote bidi",
                ),
            )
            .await;
        };

        let Some(callee) = Self::non_empty_ura(callee_ura.as_deref()) else {
            return Self::send_dispatch_up(
                outbound,
                &Self::file_transfer_terminal_error(
                    call_id,
                    format!("session.open: missing callee URA for bidi ability `{ability}`"),
                ),
            )
            .await;
        };
        let subject = match Self::self_target_subject(subject_ura.as_deref(), callee) {
            Ok(subject) => subject,
            Err(err) => {
                return Self::send_dispatch_up(
                    outbound,
                    &Self::file_transfer_terminal_error(call_id, err),
                )
                .await;
            }
        };
        let handle = crate::daemon::axon_bridge::dispatch_shim::open_bidi_local_with_subject(
            runtime,
            callee,
            subject.as_str(),
            ability,
            args,
        )
        .await;
        let handle = match handle {
            Ok(handle) => handle,
            Err(err) => {
                return Self::send_dispatch_up(
                    outbound,
                    &Self::file_transfer_terminal_error(
                        call_id,
                        format!("session.open: remote bidi open failed: {err}"),
                    ),
                )
                .await;
            }
        };
        self.register_remote_bidi(call_id, ability, handle, outbound, None)
            .await
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
        handle: easynet_axon::invocation::BidiInvocationHandle,
        outbound: &SessionUpSender,
        lifecycle_envelope: Option<easynet_axon::invocation::DescriptorBoundEnvelope>,
    ) -> Result<(), SessionDispatchError> {
        let (handler_in_tx, mut handler_out_rx) = handle.split();
        let lifecycle = if let Some(envelope) = lifecycle_envelope {
            let lifecycle_handle = handler_out_rx.handle().clone();
            match self
                .lifecycle_cancellations
                .register(&envelope, lifecycle_handle.clone())
            {
                Ok(key) => Some((key, lifecycle_handle)),
                Err(error) => {
                    let _ = handler_out_rx
                        .cancel("lifecycle cancellation registration failed")
                        .await;
                    let dispatch = Self::session_error_result(
                        call_id,
                        format!("CANONICAL_CANCELLATION_REGISTRATION_FAILED: {error}"),
                    );
                    return Self::send_bidi_result(outbound, &dispatch, None).await;
                }
            }
        } else {
            None
        };

        let admission = match handler_out_rx.admission_receipt().await {
            Ok(receipt) => receipt,
            Err(error) => {
                if let Some((key, handle)) = lifecycle.as_ref() {
                    if handler_out_rx.finalized().await.is_ok() {
                        self.lifecycle_cancellations
                            .mark_terminal(key, handle.clone());
                    }
                }
                let dispatch = Self::session_error_result(
                    call_id,
                    format!("CANONICAL_ADMISSION_REQUIRED: {error}"),
                );
                return Self::send_bidi_result(outbound, &dispatch, None).await;
            }
        };
        let admission_dispatch = SessionDispatch::Result {
            call_id,
            payload: Vec::new(),
            terminal: false,
            failure: None,
            error: None,
            request_id: None,
        };
        if let Err(error) = Self::send_bidi_result(
            outbound,
            &admission_dispatch,
            Some(BidiReceiptCheckpoint::Admission(&admission)),
        )
        .await
        {
            if let Some((key, handle)) = lifecycle.as_ref() {
                let _ = handler_out_rx
                    .cancel("session bidi closed before admission")
                    .await;
                if handler_out_rx.finalized().await.is_ok() {
                    self.lifecycle_cancellations
                        .mark_terminal(key, handle.clone());
                }
            }
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
        let lifecycle_cancellations = self.lifecycle_cancellations.clone();
        tokio::spawn(async move {
            while let Some(frame_result) = handler_out_rx.next_frame().await {
                let frame = match frame_result {
                    Ok(frame) => frame,
                    Err(err) => {
                        let finalized = match handler_out_rx.finalized().await {
                            Ok(finalized) => finalized,
                            Err(error) => {
                                let dispatch = LocalAxonSessionDispatcher::session_error_result(
                                    call_id,
                                    format!(
                                        "CANONICAL_FINALIZATION_REQUIRED: frame_error={err}; finalization_error={error}"
                                    ),
                                );
                                let _ = LocalAxonSessionDispatcher::send_bidi_result(
                                    &outbound, &dispatch, None,
                                )
                                .await;
                                break;
                            }
                        };
                        if let Some((key, handle)) = lifecycle.as_ref() {
                            lifecycle_cancellations.mark_terminal(key, handle.clone());
                        }
                        let failure = finalized.failure.as_ref().unwrap_or(&err);
                        let dispatch = SessionDispatch::Result {
                            call_id,
                            payload: Vec::new(),
                            terminal: true,
                            failure: Some(SessionFailure::from_explicit(
                                failure.code.as_str(),
                                if failure.message.is_empty() {
                                    &failure.reason
                                } else {
                                    &failure.message
                                },
                                failure.retriable(),
                            )),
                            error: None,
                            request_id: None,
                        };
                        let _ = LocalAxonSessionDispatcher::send_bidi_result(
                            &outbound,
                            &dispatch,
                            Some(BidiReceiptCheckpoint::Terminal(&finalized.terminal_receipt)),
                        )
                        .await;
                        break;
                    }
                };
                if frame.terminal {
                    let finalized = match handler_out_rx.finalized().await {
                        Ok(finalized) => finalized,
                        Err(error) => {
                            let dispatch = LocalAxonSessionDispatcher::session_error_result(
                                call_id,
                                format!("CANONICAL_FINALIZATION_REQUIRED: {error}"),
                            );
                            let _ = LocalAxonSessionDispatcher::send_bidi_result(
                                &outbound, &dispatch, None,
                            )
                            .await;
                            break;
                        }
                    };
                    if let Some((key, handle)) = lifecycle.as_ref() {
                        lifecycle_cancellations.mark_terminal(key, handle.clone());
                    }
                    let dispatch = SessionDispatch::Result {
                        call_id,
                        payload: finalized.output().to_vec(),
                        terminal: true,
                        failure: finalized.failure.as_ref().map(|failure| {
                            SessionFailure::from_explicit(
                                failure.code.as_str(),
                                if failure.message.is_empty() {
                                    &failure.reason
                                } else {
                                    &failure.message
                                },
                                failure.retriable(),
                            )
                        }),
                        error: None,
                        request_id: None,
                    };
                    let _ = LocalAxonSessionDispatcher::send_bidi_result(
                        &outbound,
                        &dispatch,
                        Some(BidiReceiptCheckpoint::Terminal(&finalized.terminal_receipt)),
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
                    Some(SessionDispatch::Result {
                        call_id,
                        payload: frame.payload,
                        terminal: frame.terminal,
                        failure: None,
                        error: None,
                        request_id: None,
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
                                    Some(LocalAxonSessionDispatcher::file_transfer_terminal_error(
                                        call_id,
                                        format!(
                                            "session.open: remote bidi output map failed: {err}"
                                        ),
                                    ))
                                }
                            }
                        }
                        Err(err) => Some(LocalAxonSessionDispatcher::file_transfer_terminal_error(
                            call_id,
                            format!("session.open: remote bidi output was not JSON: {err}"),
                        )),
                    }
                };
                let Some(mapped) = mapped else {
                    continue;
                };
                if LocalAxonSessionDispatcher::send_bidi_result(&outbound, &mapped, None)
                    .await
                    .is_err()
                {
                    break;
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
            return Self::send_dispatch_up(
                outbound,
                &Self::file_transfer_terminal_error(
                    call_id,
                    format!("remote bidi call_id={call_id} is not open on this device"),
                ),
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
            == crate::daemon::ability::builtins::device_control::terminal::attach::ABILITY_PTY_SESSION_ATTACH
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
            return Self::send_dispatch_up(
                outbound,
                &Self::file_transfer_terminal_error(
                    call_id,
                    format!("remote bidi call_id={call_id} input channel closed"),
                ),
            )
            .await;
        }
        Ok(())
    }
}

fn json_frame_error_reason(value: &Value) -> String {
    match (
        value.get("code").and_then(Value::as_str),
        value.get("message").and_then(Value::as_str),
    ) {
        (Some(code), Some(message)) if !code.trim().is_empty() && !message.trim().is_empty() => {
            format!("{code}: {message}")
        }
        (_, Some(message)) if !message.trim().is_empty() => message.to_string(),
        (Some(code), _) if !code.trim().is_empty() => code.to_string(),
        _ => "JSON-frame bidi handler returned error".to_string(),
    }
}

impl Default for LocalAxonSessionDispatcher {
    fn default() -> Self {
        Self::new()
    }
}

#[async_trait::async_trait]
impl SessionFrameDispatcher for LocalAxonSessionDispatcher {
    /// Receive a hub-pushed Dispatch frame, run the named ability
    /// against the local dispatcher, and reply with a terminal
    /// `SessionDispatch::Result`. Down-stream `Result` frames are
    /// ignored: they flow up from device → hub, never down.
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
            let outcome = match result.failure.as_ref() {
                None => {
                    crate::daemon::invocation::bidi::session_escalation::EscalationReply::Canonical(
                        Box::new(easynet_axon::pb::axon::v1::InvokeResponse {
                            result: result.payload.clone(),
                            admission_receipt: result.admission_receipt.clone(),
                            terminal_receipt: result.terminal_receipt.clone(),
                            ..easynet_axon::pb::axon::v1::InvokeResponse::default()
                        }),
                    )
                }
                Some(failure) => {
                    let error = match failure.code.as_str() {
                        "TARGET_OFFLINE" => crate::daemon::invocation::bidi::session_wire::SessionRequestError::TargetOffline,
                        "PERMISSION_DENIED" => crate::daemon::invocation::bidi::session_wire::SessionRequestError::PermissionDenied {
                            reason: failure.message.clone(),
                        },
                        "UPSTREAM_TIMEOUT" => crate::daemon::invocation::bidi::session_wire::SessionRequestError::UpstreamTimeout,
                        _ => crate::daemon::invocation::bidi::session_wire::SessionRequestError::UpstreamFailure {
                            reason: failure.message.clone(),
                        },
                    };
                    crate::daemon::invocation::bidi::session_escalation::EscalationReply::Error(
                        error,
                    )
                }
            };
            if let Some(correlation) = self.escalation_correlation.as_ref() {
                correlation.complete(call_id, outcome);
            }
            return Ok(());
        }

        // Carrier-v1 dual-read (DEC-F004 / T2.1 step 3): the hub sends
        // DispatchCall for v1-negotiated sessions — the complete
        // canonical InvokeRequest, dispatched without re-projection.
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
            SessionDispatch::BidiOpen {
                call_id,
                callee_ura,
                subject_ura,
                ability,
                args,
                args_content_envelope,
                metadata,
            } => {
                let args_bytes = args.len();
                let ability_log = ability.clone();
                crate::op_event!(
                    component = local_session_dispatcher,
                    kind = received_bidi_open_frame,
                    call_id = call_id,
                    ability = ability_log,
                    args_bytes = args_bytes,
                );
                return self
                    .open_remote_bidi(
                        RemoteBidiOpenRequest {
                            call_id,
                            callee_ura,
                            subject_ura,
                            ability,
                            args,
                            args_content_envelope,
                            metadata,
                        },
                        outbound,
                    )
                    .await;
            }
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
            SessionDispatch::Result { .. } | SessionDispatch::Request { .. } => Ok(()),
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
    fn session_bidi_gate_recognizes_core_browser_attach_wire() {
        let registry = crate::daemon::ability::wire::AbilityWireRegistry::core();
        let ability =
            crate::daemon::ability::builtins::device_control::browser::ABILITY_ATTACH_SESSION;

        assert!(local_is_bidi_wire_ability(&registry, ability));
        assert!(LocalAxonSessionDispatcher::is_json_frame_bidi_with(
            &registry, ability
        ));
    }

    #[test]
    fn carrier_v1_stream_control_failure_is_not_lifecycle_terminal() {
        let result = carrier_v1_stream_control_failure(
            9,
            "STREAM_OPEN_FAILED",
            "target rejected stream open",
        );
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

    impl easynet_axon::invocation::KeyResolver for FixedCarrierKey {
        fn resolve(
            &self,
            agent_ura: &str,
        ) -> Result<ed25519_dalek::VerifyingKey, easynet_axon::invocation::AxonError> {
            if agent_ura == TEST_CALLER_URA {
                return Ok(self.0);
            }
            if agent_ura == crate::daemon::identity::local_invocation::LOCAL_SYSTEM_AGENT_URA {
                return crate::daemon::identity::local_invocation::system_verifying_key().map_err(
                    |error| easynet_axon::invocation::AxonError::internal(error.to_string()),
                );
            }
            Err(easynet_axon::invocation::AxonError::invalid_argument(
                format!("unknown_agent_key:{agent_ura}"),
            ))
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
        crate::daemon::axon_bridge::descriptor_ref::descriptor_binding_for_wire(
            descriptor_version,
            TEST_DESCRIPTOR_HASH,
            "invoke",
        )
        .expect("test descriptor binding")
    }

    fn descriptor_ref_for_version(
        callee_ura: &str,
        ability: &str,
        descriptor_version: &str,
    ) -> String {
        crate::daemon::axon_bridge::descriptor_ref::ability_descriptor_ref_for_wire(
            callee_ura,
            ability,
            &descriptor_binding_for_version(descriptor_version),
        )
        .expect("test ability must resolve to a descriptor ref")
    }

    fn catalog_call_mode(
        mode: easynet_axon::invocation::CallMode,
    ) -> crate::daemon::ability::CallMode {
        match mode {
            easynet_axon::invocation::CallMode::Rpc => crate::daemon::ability::CallMode::Rpc,
            easynet_axon::invocation::CallMode::Stream => crate::daemon::ability::CallMode::Stream,
            easynet_axon::invocation::CallMode::Bidi => crate::daemon::ability::CallMode::Bidi,
        }
    }

    fn descriptor_ref_for_call_mode(
        callee_ura: &str,
        ability: &str,
        descriptor_version: &str,
        mode: easynet_axon::invocation::CallMode,
    ) -> String {
        if let Ok(descriptor_ref) =
            easynet_axon::invocation::canonical_ability_descriptor_ref(ability)
        {
            return descriptor_ref;
        }
        crate::daemon::axon_bridge::descriptor_ref::catalog_descriptor_ref_for_wire(
            callee_ura,
            ability,
            catalog_call_mode(mode),
        )
        .unwrap_or_else(|_| descriptor_ref_for_version(callee_ura, ability, descriptor_version))
    }

    /// Proof-bound RPC options mirroring what the control plane stamps in
    /// production. Use for every raw-runtime test registration so the
    /// dispatch path sees a bound descriptor proof.
    fn proof_bound_rpc_options() -> easynet_axon::invocation::AbilityOptions {
        proof_bound_rpc_options_with_version(TEST_DESCRIPTOR_VERSION)
    }

    fn proof_bound_rpc_options_with_version(
        descriptor_version: &str,
    ) -> easynet_axon::invocation::AbilityOptions {
        use easynet_axon::invocation::{AbilityCallModes, AbilityOptions};
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
    fn proof_bound_stream_options() -> easynet_axon::invocation::AbilityOptions {
        use easynet_axon::invocation::{AbilityOptions, CallMode};
        AbilityOptions::streaming().with_mode_descriptor_proof(
            CallMode::Stream,
            TEST_DESCRIPTOR_VERSION,
            "invoke",
            TEST_DESCRIPTOR_HASH,
            TEST_SCHEMA_HASH,
            TEST_IMPL_HASH,
        )
    }

    async fn register_test_rpc(
        runtime: &easynet_axon::invocation::LocalRuntime,
        ability: &str,
        handler: easynet_axon::invocation::AbilityFn,
    ) {
        register_test_ability_with_options(runtime, ability, handler, proof_bound_rpc_options())
            .await;
    }

    async fn register_test_ability_with_options(
        runtime: &easynet_axon::invocation::LocalRuntime,
        ability: &str,
        handler: easynet_axon::invocation::AbilityFn,
        options: easynet_axon::invocation::AbilityOptions,
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

    fn executable_runtime() -> Arc<easynet_axon::invocation::LocalRuntime> {
        let runtime = crate::daemon::axon_bridge::runtime_factory::build_local_runtime(None, None);
        runtime.set_admission_key_resolver(Arc::new(FixedCarrierKey(
            carrier_v1_signing_key().verifying_key(),
        )));
        runtime
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
            easynet_axon::invocation::CallMode::Rpc,
        )
    }

    fn carrier_v1_call_signed_as_with_mode(
        call_id: u64,
        request_ability: &str,
        signed_ability: &str,
        args: Vec<u8>,
        mode: easynet_axon::invocation::CallMode,
    ) -> InvokeBidiDown {
        use easynet_axon::pb::axon::v1::{DispatchCall, InvokeRequest};
        use ed25519_dalek::Signer as _;

        let signing_key = carrier_v1_signing_key();
        let mut envelope = crate::daemon::invocation::ProtoEnvelope::targeted(
            TEST_CALLER_URA,
            "easynet:///r/t/device/d1",
            "easynet:///r/t/device/d1",
        )
        .expect("valid carrier-v1 envelope")
        .into_inner();
        let signed_descriptor_ref = descriptor_ref_for_call_mode(
            TEST_DEVICE_URA,
            signed_ability,
            TEST_DESCRIPTOR_VERSION,
            mode,
        );
        let descriptor_bound =
            crate::daemon::axon_bridge::wire_descriptor::descriptor_bound_from_wire_parts(
                envelope.clone(),
                signed_descriptor_ref.clone(),
                &args,
                crate::daemon::axon_bridge::wire_descriptor::WireCallerIdentity::FromEnvelope,
            )
            .expect("descriptor-bound carrier-v1 envelope");
        let signature = signing_key.sign(&descriptor_bound.envelope.canonical_bytes());
        envelope.caller_signature = Some(easynet_axon::pb::axon::v1::CallerSignature {
            algorithm: "ed25519".to_string(),
            signature: signature.to_bytes().to_vec(),
            key_id_hint: String::new(),
        });

        let mut request = InvokeRequest {
            envelope: Some(envelope),
            function_name: request_ability.to_string(),
            arguments: args,
            ..Default::default()
        };
        request.metadata.insert(
            crate::daemon::invocation::dispatch::invocation_wire::SIGNED_DESCRIPTOR_REF_METADATA_KEY
                .to_string(),
            signed_descriptor_ref,
        );

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
            easynet_axon::invocation::CallMode::Bidi,
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

        let rt = crate::daemon::axon_bridge::runtime_factory::build_ephemeral_test_runtime();
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
            easynet_axon::invocation::InvocationState::Admitted.to_wire_i32()
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
            easynet_axon::invocation::InvocationState::Completed.to_wire_i32(),
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
            carrier_v1_bidi_open(9, "test.echo", b"{}".to_vec()),
            &session_tx,
        )
        .await
        .expect("open error replies as a frame, not an Err");

        let reply = rx.recv().await.expect("reply produced");
        match reply.payload {
            Some(UpPayload::DispatchResult(r)) => {
                assert_eq!(r.call_id, 9);
                assert!(r.terminal);
                let failure = r.failure.expect("typed failure");
                assert!(
                    failure.message.contains("not wired"),
                    "unexpected failure: {}",
                    failure.message
                );
            }
            other => panic!("expected proto DispatchResult, got: {other:?}"),
        }
    }

    /// Quadrant [new hub, old device session] for step-3b: a v1 bidi
    /// open arriving on a v0-negotiated session (hub jumped the gun)
    /// still executes, and the reply stays JSON for the dual-reading
    /// hub.
    #[tokio::test]
    async fn carrier_v1_dispatch_executes_and_replies_proto_on_v1_session() {
        let rt = executable_runtime();
        register_test_rpc(
            &rt,
            "test.echo",
            easynet_axon::invocation::make_ability(|ctx| async move { Ok(ctx.payload.clone()) }),
        )
        .await;
        let disp = LocalAxonSessionDispatcher::new().with_local_runtime(Arc::clone(&rt));
        let (tx, mut rx) = mpsc::channel::<InvokeBidiUp>(4);
        let session_tx = SessionUpSender::new(tx);
        session_tx.set_negotiated_contract(1);

        disp.handle_down(
            carrier_v1_call(7, "test.echo", br#"{"hello":"v1"}"#.to_vec()),
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
            easynet_axon::invocation::InvocationState::Admitted.to_wire_i32()
        );
        assert_eq!(
            terminal.state,
            easynet_axon::invocation::InvocationState::Completed.to_wire_i32()
        );
        assert_eq!(admission.invocation_id, terminal.invocation_id);
    }

    #[tokio::test]
    async fn carrier_v1_dispatch_preserves_non_default_descriptor_version() {
        let rt = executable_runtime();
        register_test_ability_with_options(
            &rt,
            "test.echo",
            easynet_axon::invocation::make_ability(|ctx| async move { Ok(ctx.payload.clone()) }),
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

    /// Quadrant [hub jumped the gun, v0 session]: a DispatchCall on a
    /// session still negotiated v0 executes anyway and replies on the
    /// JSON carrier, which the hub dual-reads — no call is lost to
    /// negotiation skew.
    #[tokio::test]
    async fn carrier_v1_stream_terminal_frame_carries_receipt() {
        use easynet_axon::invocation::make_ability;

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
            carrier_v1_call(18, "screen.subscribe", b"{}".to_vec()),
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
        assert_eq!(
            admission
                .admission_receipt
                .expect("admission frame carries receipt")
                .state,
            easynet_axon::invocation::InvocationState::Admitted.to_wire_i32()
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
            easynet_axon::invocation::InvocationState::Completed.to_wire_i32()
        );
        assert!(
            disp.lifecycle_cancellations
                .contains_invocation_id(&receipt.invocation_id),
            "carrier-v1 stream lifecycle must remain registered for invocation.cancel"
        );
    }

    #[tokio::test]
    async fn down_stream_result_frame_is_ignored() {
        // SessionDispatch::Result on the down stream is a wire
        // mistake (Results flow up, not down). The dispatcher
        // logs nothing and returns Ok without sending a reply
        // frame.
        let disp = LocalAxonSessionDispatcher::new();
        let (tx, mut rx) = mpsc::channel::<InvokeBidiUp>(4);
        let session_tx = SessionUpSender::new(tx);

        let bogus = SessionDispatch::Result {
            call_id: 42,
            payload: Vec::new(),
            terminal: true,
            error: None,
            failure: None,
            request_id: None,
        };
        let bogus_bytes = serde_json::to_vec(&bogus).expect("encode bogus");
        let frame = InvokeBidiDown {
            sequence: 0,
            payload: Some(DownPayload::BinaryChunk(BinaryChunk {
                data: bogus_bytes,
                ..BinaryChunk::default()
            })),
            ..InvokeBidiDown::default()
        };

        disp.handle_down(frame, &session_tx)
            .await
            .expect("ignored cleanly");
        match rx.try_recv() {
            Err(tokio::sync::mpsc::error::TryRecvError::Empty) => {}
            Ok(unexpected) => {
                panic!("ignored Result frame must not produce a reply; got: {unexpected:?}")
            }
            Err(other) => panic!("unexpected channel state: {other:?}"),
        }
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
        local_runtime: Option<Arc<easynet_axon::invocation::LocalRuntime>>,
    ) -> Arc<crate::daemon::ability::dispatch::AxonAbilityCatalog> {
        use crate::daemon::execution::loop_instance::LoopService;
        use crate::daemon::execution::mission::discuss::DiscussService;
        use crate::daemon::execution::permission::PermissionService;
        use crate::daemon::execution::schedule::ScheduleService;
        use crate::daemon::execution::session::SessionService;
        let agents = Default::default();
        let mut config = crate::daemon::ability::catalog::RegistryBuildConfig::new(
            crate::daemon::ability::catalog::RegistryBuildServices::new(
                Arc::new(SessionService::new()),
                Arc::new(PermissionService::new()),
                Arc::new(DiscussService::new()),
                Arc::new(ScheduleService::new()),
                Arc::new(LoopService::new()),
            ),
            &agents,
        );
        config.local_runtime = local_runtime;
        config.authority_context = Some(
            crate::daemon::ability::dispatch::AbilityAuthorityContext::for_device_authority_root(
                TEST_DEVICE_URA,
            )
            .expect("test device URA is a valid device authority root"),
        );
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

    #[tokio::test(flavor = "multi_thread", worker_threads = 2)]
    async fn remote_file_transfer_upload_round_trips_over_session_bidi_frames() {
        let tmp = tempfile::tempdir().expect("tempdir");
        let target = tmp.path().join("upload-from-hub.bin");
        let bytes = b"remote-file-transfer-over-session";

        let rt = executable_runtime();
        let _registry = build_real_daemon_registry_with_runtime(Some(Arc::clone(&rt)));
        let disp = LocalAxonSessionDispatcher::new().with_local_runtime(rt);
        let (tx, mut rx) = mpsc::channel::<InvokeBidiUp>(8);
        let session_tx = SessionUpSender::new(tx);

        disp.handle_down(
            session_frame(SessionDispatch::BidiOpen {
                call_id: 77,
                callee_ura: Some("easynet:///r/t/device/d1".to_string()),
                subject_ura: Some("easynet:///r/t/device/d1".to_string()),
                ability: crate::daemon::ability::builtins::device_control::file_transfer::ABILITY_FILE_TRANSFER
                    .to_string(),
                args: serde_json::to_vec(&json!({
                    "mode": "upload",
                    "resource_ref": crate::daemon::resources::files::resource_ref_for_local_path(
                        &target,
                        crate::daemon::resources::files::FilesystemResourceCapability::Write,
                    )
                    .expect("local fs ResourceRef"),
                }))
                .expect("encode args"),
                args_content_envelope: SessionContentEnvelope::plaintext_json(),
                metadata: HashMap::new(),
            }),
            &session_tx,
        )
        .await
        .expect("bidi open succeeds");

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

        let mut saw_complete_data = false;
        let mut saw_runtime_terminal = false;
        for _ in 0..4 {
            let reply = tokio::time::timeout(Duration::from_secs(3), rx.recv())
                .await
                .expect("reply within 3s")
                .expect("reply produced");
            let chunk = match reply.payload {
                Some(UpPayload::BinaryChunk(c)) => c,
                other => panic!("expected BinaryChunk reply, got: {other:?}"),
            };
            let parsed: SessionDispatch =
                serde_json::from_slice(&chunk.data).expect("Result decodes");
            match parsed {
                SessionDispatch::Result {
                    call_id,
                    terminal,
                    error,
                    payload,
                    request_id: _,
                    ..
                } => {
                    assert_eq!(call_id, 77);
                    assert!(error.is_none(), "upload must succeed, got {error:?}");
                    if !payload.is_empty() {
                        let value: serde_json::Value =
                            serde_json::from_slice(&payload).expect("payload decodes as JSON");
                        if value.get("type").and_then(Value::as_str) == Some("complete") {
                            assert!(!terminal, "handler completion is execution data");
                            saw_complete_data = true;
                        }
                    }
                    if terminal {
                        saw_runtime_terminal = true;
                        break;
                    }
                }
                other => panic!("expected SessionDispatch::Result, got: {other:?}"),
            }
        }
        assert!(saw_complete_data, "upload must preserve completion data");
        assert!(
            saw_runtime_terminal,
            "Axon runtime must emit the terminal frame"
        );

        let on_disk = std::fs::read(&target).expect("file written on device side");
        assert_eq!(on_disk, bytes);
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

        match mapped {
            SessionDispatch::Result {
                call_id,
                payload,
                terminal,
                error,
                failure,
                request_id: _,
            } => {
                assert_eq!(call_id, 91);
                assert!(!terminal);
                assert_eq!(error, None);
                assert!(failure.is_none());
                let payload: Value = serde_json::from_slice(&payload).expect("json payload");
                assert_eq!(payload["type"], "frame");
                assert_eq!(payload["seq"], 3);
                assert_eq!(payload["image_bytes_b64"], "abc");
            }
            other => panic!("expected SessionDispatch::Result, got: {other:?}"),
        }
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

        match mapped {
            SessionDispatch::Result {
                terminal,
                error,
                failure,
                ..
            } => {
                assert!(!terminal);
                assert!(error.is_none());
                assert!(failure.is_none());
            }
            other => panic!("expected SessionDispatch::Result, got: {other:?}"),
        }
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

        match mapped {
            SessionDispatch::Result {
                terminal,
                error,
                failure,
                payload,
                ..
            } => {
                assert!(!terminal);
                assert_eq!(
                    error.as_deref(),
                    Some("permission_denied: screen capture permission denied")
                );
                let failure = failure.expect("typed failure");
                assert_eq!(failure.code, "PERMISSION_DENIED");
                assert_eq!(
                    failure.message,
                    "permission_denied: screen capture permission denied"
                );
                let payload: Value = serde_json::from_slice(&payload).expect("json payload");
                assert_eq!(payload["type"], "error");
            }
            other => panic!("expected SessionDispatch::Result, got: {other:?}"),
        }
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

    #[tokio::test(flavor = "multi_thread", worker_threads = 2)]
    async fn remote_file_transfer_download_round_trips_over_session_bidi_frames() {
        let tmp = tempfile::tempdir().expect("tempdir");
        let target = tmp.path().join("download-to-hub.bin");
        let bytes = b"remote-download-bytes-from-device";
        std::fs::write(&target, bytes).expect("seed file");

        let rt = executable_runtime();
        let _registry = build_real_daemon_registry_with_runtime(Some(Arc::clone(&rt)));
        let disp = LocalAxonSessionDispatcher::new().with_local_runtime(rt);
        let (tx, mut rx) = mpsc::channel::<InvokeBidiUp>(16);
        let session_tx = SessionUpSender::new(tx);

        disp.handle_down(
            session_frame(SessionDispatch::BidiOpen {
                call_id: 88,
                callee_ura: Some("easynet:///r/t/device/d1".to_string()),
                subject_ura: Some("easynet:///r/t/device/d1".to_string()),
                ability: crate::daemon::ability::builtins::device_control::file_transfer::ABILITY_FILE_TRANSFER
                    .to_string(),
                args: serde_json::to_vec(&json!({
                    "mode": "download",
                    "resource_ref": crate::daemon::resources::files::resource_ref_for_local_path(
                        &target,
                        crate::daemon::resources::files::FilesystemResourceCapability::Read,
                    )
                    .expect("local fs ResourceRef"),
                }))
                .expect("encode args"),
                args_content_envelope: SessionContentEnvelope::plaintext_json(),
                metadata: HashMap::new(),
            }),
            &session_tx,
        )
        .await
        .expect("bidi open succeeds");

        disp.handle_down(
            session_frame(SessionDispatch::BidiInput {
                call_id: 88,
                payload: Vec::new(),
                eof: true,
            }),
            &session_tx,
        )
        .await
        .expect("download eof hint forwards");

        let mut streamed = Vec::new();
        let mut saw_complete_data = false;
        let mut saw_runtime_terminal = false;
        for _ in 0..6 {
            let reply = tokio::time::timeout(Duration::from_secs(3), rx.recv())
                .await
                .expect("reply within 3s")
                .expect("reply produced");
            let chunk = match reply.payload {
                Some(UpPayload::BinaryChunk(c)) => c,
                other => panic!("expected BinaryChunk reply, got: {other:?}"),
            };
            let parsed: SessionDispatch =
                serde_json::from_slice(&chunk.data).expect("Result decodes");
            match parsed {
                SessionDispatch::Result {
                    call_id,
                    terminal,
                    error,
                    payload,
                    request_id: _,
                    ..
                } => {
                    assert_eq!(call_id, 88);
                    assert!(error.is_none(), "download must succeed, got {error:?}");
                    if terminal {
                        saw_runtime_terminal = true;
                        break;
                    }
                    match serde_json::from_slice::<serde_json::Value>(&payload) {
                        Ok(value)
                            if value.get("type").and_then(Value::as_str) == Some("complete") =>
                        {
                            saw_complete_data = true;
                        }
                        _ => streamed.extend_from_slice(&payload),
                    }
                }
                other => panic!("expected SessionDispatch::Result, got: {other:?}"),
            }
        }

        assert_eq!(streamed, bytes);
        assert!(saw_complete_data, "download must preserve completion data");
        assert!(
            saw_runtime_terminal,
            "Axon runtime must emit the terminal frame"
        );
    }

    #[tokio::test(flavor = "multi_thread", worker_threads = 2)]
    async fn remote_file_transfer_download_missing_file_returns_typed_terminal_failure() {
        let tmp = tempfile::tempdir().expect("tempdir");
        let target = tmp.path().join("missing-download.bin");

        let rt = executable_runtime();
        let _registry = build_real_daemon_registry_with_runtime(Some(Arc::clone(&rt)));
        let disp = LocalAxonSessionDispatcher::new().with_local_runtime(rt);
        let (tx, mut rx) = mpsc::channel::<InvokeBidiUp>(8);
        let session_tx = SessionUpSender::new(tx);

        disp.handle_down(
            session_frame(SessionDispatch::BidiOpen {
                call_id: 89,
                callee_ura: Some("easynet:///r/t/device/d1".to_string()),
                subject_ura: Some("easynet:///r/t/device/d1".to_string()),
                ability: crate::daemon::ability::builtins::device_control::file_transfer::ABILITY_FILE_TRANSFER
                    .to_string(),
                args: serde_json::to_vec(&json!({
                    "mode": "download",
                    "resource_ref": crate::daemon::resources::files::resource_ref_for_local_path(
                        &target,
                        crate::daemon::resources::files::FilesystemResourceCapability::Read,
                    )
                    .expect("local fs ResourceRef"),
                }))
                .expect("encode args"),
                args_content_envelope: SessionContentEnvelope::plaintext_json(),
                metadata: HashMap::new(),
            }),
            &session_tx,
        )
        .await
        .expect("bidi open succeeds");

        let mut saw_handler_failure = false;
        let mut saw_runtime_terminal = false;
        for _ in 0..4 {
            let reply = tokio::time::timeout(Duration::from_secs(3), rx.recv())
                .await
                .expect("reply within 3s")
                .expect("reply produced");
            let chunk = match reply.payload {
                Some(UpPayload::BinaryChunk(c)) => c,
                other => panic!("expected BinaryChunk reply, got: {other:?}"),
            };
            let parsed: SessionDispatch =
                serde_json::from_slice(&chunk.data).expect("Result decodes");
            match parsed {
                SessionDispatch::Result {
                    call_id,
                    terminal,
                    error,
                    failure,
                    payload,
                    request_id: _,
                } => {
                    assert_eq!(call_id, 89);
                    if let Some(error) = error {
                        assert!(!terminal, "handler failure is execution data");
                        assert!(
                            error.contains("not_found"),
                            "download failure must preserve handler code, got: {error}"
                        );
                        let failure = failure.expect("typed handler failure");
                        assert_eq!(failure.code, "NOT_FOUND");
                        assert_eq!(failure.message, error);
                        let payload: Value =
                            serde_json::from_slice(&payload).expect("json payload");
                        assert_eq!(payload["type"], "error");
                        assert_eq!(payload["code"], "not_found");
                        saw_handler_failure = true;
                    }
                    if terminal {
                        saw_runtime_terminal = true;
                        break;
                    }
                }
                other => panic!("expected SessionDispatch::Result, got: {other:?}"),
            }
        }
        assert!(
            saw_handler_failure,
            "download must preserve typed failure data"
        );
        assert!(
            saw_runtime_terminal,
            "Axon runtime must emit the terminal frame"
        );
    }
}
