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
use serde_json::Value;
use tokio::sync::mpsc;
use tokio_util::sync::CancellationToken;

use super::descriptor_binding::RuntimeBoundAbility;
use super::invocation_wire::{callee_ura_from_envelope, FEDERATION_RESULT_CONTENT_TYPE};
#[cfg(test)]
use crate::daemon::axon_bridge::proof_owner::descriptor_bound_canonical_bytes;
use crate::daemon::invocation::admission::admission_facade::AdmissionFacade;
use crate::daemon::invocation::bidi::session_initiator::{
    SessionDispatchError, SessionFrameDispatcher, SessionUpSender,
};
use crate::daemon::invocation::bidi::session_wire::{
    call_id_hex, canonical_dispatch_call_mode, SessionDispatch,
};
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
    /// Carrier-scoped dispatch lifecycle. Wire `call_id` values restart when
    /// `session.open` reconnects, so BIDI and stream ownership must live under
    /// the process-local carrier scope rather than a process-global call-id
    /// table.
    carrier_sessions: Arc<Mutex<CarrierDispatchRegistry>>,
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

#[derive(Clone)]
struct ActiveRemoteBidi {
    ingress: mpsc::Sender<PendingRemoteBidiInput>,
    half_closed: bool,
}

struct PendingRemoteBidiInput {
    content_type: String,
    payload: Vec<u8>,
    eof: bool,
}

enum RemoteBidiSession {
    /// Open was accepted by the session carrier, but canonical runtime
    /// admission has not been published yet. Input frames may arrive on the
    /// same session while trust sync or runtime open is in progress; they are
    /// bounded here and are not delivered to the handler until admission is
    /// sent upstream.
    Opening {
        ingress: mpsc::Sender<PendingRemoteBidiInput>,
        half_closed: bool,
    },
    /// Admission has been published and the Axon handler input channel is live.
    Active(ActiveRemoteBidi),
}

#[derive(Clone, Copy, Debug, Eq, Hash, PartialEq)]
struct CarrierCallKey {
    scope_id: u64,
    call_id: u64,
}

#[derive(Default)]
struct CarrierDispatchSession {
    bidi: HashMap<u64, RemoteBidiSession>,
    streams: HashMap<u64, CancellationToken>,
}

#[derive(Default)]
struct CarrierDispatchRegistry {
    sessions: HashMap<u64, CarrierDispatchSession>,
}

/// Owns one registered stream call for the lifetime of its forwarding task.
/// Every task exit path (admission failure, projection failure, carrier close,
/// normal terminal, or panic unwind) converges through `Drop`, so registry
/// cleanup cannot be skipped by an early return.
struct CarrierStreamRegistration {
    registry: Arc<Mutex<CarrierDispatchRegistry>>,
    key: CarrierCallKey,
}

impl CarrierStreamRegistration {
    fn new(registry: Arc<Mutex<CarrierDispatchRegistry>>, key: CarrierCallKey) -> Self {
        Self { registry, key }
    }
}

impl Drop for CarrierStreamRegistration {
    fn drop(&mut self) {
        let mut guard = match self.registry.lock() {
            Ok(guard) => guard,
            Err(poisoned) => poisoned.into_inner(),
        };
        if let Some(session) = guard.sessions.get_mut(&self.key.scope_id) {
            session.streams.remove(&self.key.call_id);
        }
    }
}

const REMOTE_BIDI_INPUT_CAPACITY: usize = 32;

#[derive(Debug, Clone, PartialEq, Eq)]
struct BidiOutputProjection {
    call_id: u64,
    payload: Vec<u8>,
    content_type: String,
    failure: Option<SessionFailure>,
    disposition: BidiOutputDisposition,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum BidiOutputDisposition {
    Data,
    Completion,
    Failure,
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

fn canonical_carrier_control_failure(
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

fn canonical_carrier_invocation_failure(
    call_id: u64,
    error: &axon_sdk::invocation::AxonError,
) -> axon_sdk::pb::axon::v1::DispatchResult {
    axon_sdk::pb::axon::v1::DispatchResult {
        call_id,
        payload: Vec::new(),
        terminal: false,
        failure: Some(axon_sdk::invocation::wire::error_to_wire(error)),
        ..Default::default()
    }
}

impl LocalAxonSessionDispatcher {
    fn begin_carrier_scope(&self, scope_id: u64) {
        let mut guard = match self.carrier_sessions.lock() {
            Ok(g) => g,
            Err(poisoned) => poisoned.into_inner(),
        };
        guard.sessions.entry(scope_id).or_default();
    }

    fn end_carrier_scope(&self, scope_id: u64) {
        let retired = {
            let mut guard = match self.carrier_sessions.lock() {
                Ok(g) => g,
                Err(poisoned) => poisoned.into_inner(),
            };
            guard.sessions.remove(&scope_id)
        };
        let Some(retired) = retired else {
            return;
        };
        let bidi_count = retired.bidi.len();
        let stream_count = retired.streams.len();
        for cancel in retired.streams.into_values() {
            cancel.cancel();
        }
        // Dropping ActiveRemoteBidi senders closes handler input. Their output
        // forwarders own canonical cancel+finalize and converge independently;
        // no stale call id can survive into the next carrier scope.
        drop(retired.bidi);
        crate::op_event!(
            component = local_session_dispatcher,
            kind = carrier_dispatch_scope_retired,
            scope_id = scope_id,
            bidi_sessions = bidi_count,
            stream_sessions = stream_count,
        );
    }

    /// Canonical dispatch: the frame already is the invocation, so neither
    /// caller identity nor request fields are reconstructed at this hop.
    async fn handle_canonical_carrier_dispatch(
        &self,
        call: axon_sdk::pb::axon::v1::DispatchCall,
        outbound: &SessionUpSender,
    ) -> Result<(), SessionDispatchError> {
        use axon_sdk::pb::axon::v1::DispatchResult as PbDispatchResult;

        let call_id = call.call_id;
        if !outbound.canonical_carrier() {
            return Err(SessionDispatchError::Other(
                "DispatchCall requires negotiated session canonical carrier".to_string(),
            ));
        }
        let Some(request) = call.request else {
            return Err(SessionDispatchError::Other(
                "canonical carrier DispatchCall without request".to_string(),
            ));
        };
        let call_mode = match canonical_dispatch_call_mode(call.call_mode) {
            Ok(call_mode) => call_mode,
            Err(message) => {
                return Self::send_canonical_carrier_control_failure(
                    outbound,
                    call_id,
                    "CALL_MODE_INVALID",
                    message,
                )
                .await;
            }
        };
        if matches!(call_mode, axon_sdk::invocation::CallMode::Bidi) {
            return Self::send_canonical_carrier_control_failure(
                outbound,
                call_id,
                "BIDI_DISPATCH_NOT_PREPARED",
                "canonical carrier bidi dispatch must be reserved by the carrier ingress",
            )
            .await;
        }
        let function_name = match crate::daemon::invocation::dispatch::invocation_wire::function_name_from_invocation_target(
                "canonical carrier DispatchCall",
                request.target.as_ref(),
            ) {
            Ok(function_name) => function_name.to_string(),
            Err(status) => {
                return Self::send_canonical_carrier_control_failure(
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
            kind = received_canonical_carrier_dispatch,
            call_id = call_id,
            ability = function_name,
        );
        let Some(envelope) = request.envelope else {
            return Self::send_canonical_carrier_control_failure(
                outbound,
                call_id,
                "ENVELOPE_INCOMPLETE",
                "canonical carrier DispatchCall request missing envelope",
            )
            .await;
        };
        let runtime = match self.require_local_runtime("canonical carrier dispatch") {
            Ok(runtime) => runtime,
            Err(error) => {
                return Self::send_canonical_carrier_control_failure(
                    outbound,
                    call_id,
                    "RUNTIME_UNAVAILABLE",
                    error.to_string(),
                )
                .await;
            }
        };
        let target_ura =
            match callee_ura_from_envelope(Some(&envelope), "canonical carrier DispatchCall") {
                Ok(target_ura) => target_ura,
                Err(status) => {
                    return Self::send_canonical_carrier_control_failure(
                        outbound,
                        call_id,
                        "ENVELOPE_INCOMPLETE",
                        status.message(),
                    )
                    .await;
                }
            };
        if let Err(error) = self.sync_external_signed_caller_key(&envelope).await {
            return Self::send_canonical_carrier_control_failure(
                outbound,
                call_id,
                "CALLER_KEY_SYNC_FAILED",
                error.to_string(),
            )
            .await;
        }
        let bound_ability = match RuntimeBoundAbility::from_wire_target(
            "canonical carrier DispatchCall",
            &runtime,
            &target_ura,
            &function_name,
        )
        .await
        {
            Ok(bound_ability) => bound_ability,
            Err(status) => {
                return Self::send_canonical_carrier_control_failure(
                    outbound,
                    call_id,
                    "ABILITY_RESOLUTION_FAILED",
                    status.message(),
                )
                .await;
            }
        };
        let descriptor_ref = match bound_ability.signed_descriptor_ref_from_target(
            "canonical carrier DispatchCall",
            &target_ura,
            call_mode,
            request.target.as_ref(),
        ) {
            Ok(descriptor_ref) => descriptor_ref,
            Err(status) => {
                return Self::send_canonical_carrier_control_failure(
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
                return Self::send_canonical_carrier_control_failure(
                    outbound,
                    call_id,
                    "DISPATCH_WIRE_INVALID",
                    format!("build canonical carrier signed dispatch: {error}"),
                )
                .await;
            }
        };
        let runtime_admission = match self.stage_runtime_admission(&wire, &function_name, call_mode)
        {
            Ok(runtime_admission) => runtime_admission,
            Err(error) => {
                return Self::send_canonical_carrier_control_failure(
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
        // Canonical carrier preserves caller identity through the descriptor-bound
        // signature.
        if matches!(call_mode, axon_sdk::invocation::CallMode::Stream) {
            return self
                .handle_canonical_carrier_stream_open(call_id, wire, runtime_admission, outbound)
                .await;
        }

        crate::op_event!(
            component = local_session_dispatcher,
            kind = canonical_carrier_rpc_dispatch_started,
            call_id = call_id,
            ability = function_name.as_str(),
            call_mode = "rpc",
        );
        let outcome = crate::daemon::axon_bridge::descriptor_bound_dispatch::dispatch_rpc_admitted(
            &runtime,
            wire,
            &self.lifecycle_cancellations,
        )
        .await;
        crate::op_event!(
            component = local_session_dispatcher,
            kind = canonical_carrier_rpc_dispatch_completed,
            call_id = call_id,
            ability = function_name.as_str(),
            invocation_id = outcome.invocation_id.as_deref().unwrap_or(""),
            state = format!("{:?}", outcome.state),
            payload_bytes = outcome.payload_bytes.len(),
            has_error = outcome.error.is_some(),
            has_admission_receipt = outcome.admission_receipt.is_some(),
            has_terminal_receipt = outcome.terminal_receipt.is_some(),
        );
        if outcome.invocation_id.is_some() {
            Self::commit_runtime_admission(runtime_admission)?;
        }

        let failure = outcome
            .error
            .as_ref()
            .map(axon_sdk::invocation::wire::error_to_wire);
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
        crate::op_event!(
            component = local_session_dispatcher,
            kind = canonical_carrier_rpc_dispatch_result_sent,
            call_id = call_id,
            ability = function_name.as_str(),
        );
        Ok(())
    }

    /// Reserve a BIDI call synchronously at carrier ingress, then perform its
    /// descriptor-bound admission asynchronously.
    ///
    /// `BidiInput` is allowed to follow `DispatchCall` immediately on the
    /// wire. Registering `Opening` before `handle_down` returns is therefore a
    /// state-machine requirement, not an optimization: it gives those frames
    /// one deterministic owner while admission is in flight.
    async fn schedule_canonical_carrier_bidi_open(
        &self,
        call: axon_sdk::pb::axon::v1::DispatchCall,
        outbound: &SessionUpSender,
    ) -> Result<(), SessionDispatchError> {
        let call_id = call.call_id;
        if !outbound.canonical_carrier() {
            return Err(SessionDispatchError::Other(
                "DispatchCall requires negotiated session canonical carrier".to_string(),
            ));
        }
        let Some(request) = call.request else {
            return Self::send_canonical_carrier_control_failure(
                outbound,
                call_id,
                "CARRIER_REQUEST_MISSING",
                "canonical carrier BIDI DispatchCall without request",
            )
            .await;
        };
        let ability = match crate::daemon::invocation::dispatch::invocation_wire::function_name_from_invocation_target(
            "canonical carrier bidi open",
            request.target.as_ref(),
        ) {
            Ok(ability) => ability.to_string(),
            Err(status) => {
                return Self::send_canonical_carrier_control_failure(
                    outbound,
                    call_id,
                    "CARRIER_TARGET_INVALID",
                    status.message(),
                )
                .await;
            }
        };
        let input_rx = match self.insert_remote_bidi_opening(outbound.scope_id(), call_id, &ability)
        {
            Ok(input_rx) => input_rx,
            Err(error) => {
                return Self::send_bidi_control_failure(
                    outbound,
                    call_id,
                    "BIDI_SESSION_CONFLICT",
                    error.to_string(),
                )
                .await;
            }
        };

        let dispatcher = self.clone();
        let outbound = outbound.clone();
        tokio::spawn(async move {
            if let Err(error) = dispatcher
                .handle_canonical_carrier_bidi_open(call_id, ability, request, input_rx, &outbound)
                .await
            {
                dispatcher.remove_remote_bidi_session(outbound.scope_id(), call_id);
                crate::op_event!(
                    component = local_session_dispatcher,
                    kind = canonical_carrier_bidi_open_task_failed,
                    scope_id = outbound.scope_id(),
                    call_id = call_id,
                    error = error.to_string(),
                );
            }
        });
        Ok(())
    }

    /// step-3c — open a server-stream ability over the canonical carrier
    /// transport and forward its frames as a chain of `DispatchResult`
    /// chunks. Product policy is staged with `wire` and evaluated by the
    /// runtime's receipt-provider boundary; an open failure is reported as a
    /// non-terminal carrier control failure.
    async fn handle_canonical_carrier_stream_open(
        &self,
        call_id: u64,
        wire: crate::daemon::axon_bridge::descriptor_bound_dispatch::WireDispatch,
        runtime_admission: Option<
            crate::daemon::invocation::admission::admission_facade::DaemonRuntimeAdmissionLease,
        >,
        outbound: &SessionUpSender,
    ) -> Result<(), SessionDispatchError> {
        let runtime = self.require_local_runtime("canonical carrier stream")?;
        let lifecycle_envelope = wire.envelope.clone();
        let handle =
            match crate::daemon::axon_bridge::descriptor_bound_dispatch::open_stream_admitted(
                &runtime, wire,
            )
            .await
            {
                Ok(handle) => handle,
                Err(err) => {
                    let reply = canonical_carrier_invocation_failure(call_id, &err);
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
                let reply = canonical_carrier_control_failure(
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

        let key = CarrierCallKey {
            scope_id: outbound.scope_id(),
            call_id,
        };
        let cancel = CancellationToken::new();
        if !self.insert_remote_stream(key, cancel.clone()) {
            let _ = lifecycle
                .cancel_and_finalize("carrier ended before stream registration")
                .await;
            return Ok(());
        }
        Self::spawn_canonical_carrier_stream_forwarder(
            key,
            handle,
            outbound.clone(),
            Arc::clone(&self.carrier_sessions),
            cancel,
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
    fn spawn_canonical_carrier_stream_forwarder(
        key: CarrierCallKey,
        mut handle: axon_sdk::invocation::StreamingInvocationHandle,
        outbound: SessionUpSender,
        sessions: Arc<Mutex<CarrierDispatchRegistry>>,
        cancel: CancellationToken,
        lifecycle: RegisteredInvocationLifecycle,
    ) {
        use axon_sdk::pb::axon::v1::DispatchResult as PbDispatchResult;

        let call_id = key.call_id;
        tokio::spawn(async move {
            let _registration = CarrierStreamRegistration::new(sessions, key);
            let mut sent_terminal = false;
            let mut cancelled = false;
            let admission = match handle.admission_receipt().await {
                Ok(receipt) => receipt,
                Err(error) => {
                    let _ = lifecycle.finalized().await;
                    let _ = outbound
                        .send_payload(UpPayload::DispatchResult(
                            canonical_carrier_control_failure(
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
                    let _ = lifecycle
                        .cancel_and_finalize("canonical admission projection failed")
                        .await;
                    let _ = outbound
                        .send_payload(UpPayload::DispatchResult(
                            canonical_carrier_control_failure(
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
                            kind = forwarding_stream_frame_up_canonical_carrier,
                            call_id = call_id,
                            payload_bytes = frame.payload.len(),
                            terminal = terminal,
                        );
                        if terminal {
                            sent_terminal = true;
                            let finalized = match lifecycle.finalized().await {
                                Ok(finalized) => finalized,
                                Err(error) => {
                                    let _ = outbound
                                        .send_payload(UpPayload::DispatchResult(
                                            canonical_carrier_control_failure(
                                                call_id,
                                                "CANONICAL_FINALIZATION_REQUIRED",
                                                error.to_string(),
                                            ),
                                        ))
                                        .await;
                                    break;
                                }
                            };
                            match Self::canonical_terminal_dispatch_result(call_id, &finalized) {
                                Ok(reply) => reply,
                                Err(error) => {
                                    let _ = outbound
                                        .send_payload(UpPayload::DispatchResult(
                                            canonical_carrier_control_failure(
                                                call_id,
                                                "CANONICAL_TERMINAL_PROJECTION_FAILED",
                                                error.to_string(),
                                            ),
                                        ))
                                        .await;
                                    break;
                                }
                            }
                        } else {
                            PbDispatchResult {
                                call_id,
                                payload: frame.payload,
                                result_content_type: frame.content_type,
                                ..PbDispatchResult::default()
                            }
                        }
                    }
                    Err(err) => {
                        sent_terminal = true;
                        let finalized = match lifecycle.finalized().await {
                            Ok(finalized) => finalized,
                            Err(error) => {
                                let _ = outbound
                                    .send_payload(UpPayload::DispatchResult(
                                        canonical_carrier_control_failure(
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
                        match Self::canonical_terminal_dispatch_result(call_id, &finalized) {
                            Ok(reply) => reply,
                            Err(error) => {
                                let _ = outbound
                                    .send_payload(UpPayload::DispatchResult(
                                        canonical_carrier_control_failure(
                                            call_id,
                                            "CANONICAL_TERMINAL_PROJECTION_FAILED",
                                            format!("frame_error={err}; projection_error={error}"),
                                        ),
                                    ))
                                    .await;
                                return;
                            }
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
            // Cancellation must reach the RUNTIME task, not just this
            // forwarder — dropping the handle alone leaves the ability's
            // emit loop alive holding its stream source. cancel() is
            // idempotent and a no-op on already-terminal invocations.
            if !sent_terminal {
                let reason = if cancelled {
                    "canonical carrier stream cancellation requested"
                } else {
                    "canonical carrier stream ended without terminal frame"
                };
                match lifecycle.cancel_and_finalize(reason).await {
                    Ok(finalized) => {
                        match Self::canonical_terminal_dispatch_result(call_id, &finalized) {
                            Ok(reply) => {
                                let _ = outbound
                                    .send_payload(UpPayload::DispatchResult(reply))
                                    .await;
                            }
                            Err(error) => {
                                let _ = outbound
                                    .send_payload(UpPayload::DispatchResult(
                                        canonical_carrier_control_failure(
                                            call_id,
                                            "CANONICAL_TERMINAL_PROJECTION_FAILED",
                                            error.to_string(),
                                        ),
                                    ))
                                    .await;
                            }
                        }
                    }
                    Err(err) => {
                        let err_msg = err.to_string();
                        crate::op_event!(
                            component = local_session_dispatcher,
                            kind = stream_runtime_cancel_failed,
                            call_id = call_id,
                            error = err_msg,
                        );
                    }
                }
            }
        });
    }

    fn insert_remote_stream(&self, key: CarrierCallKey, cancel: CancellationToken) -> bool {
        let mut guard = match self.carrier_sessions.lock() {
            Ok(g) => g,
            Err(poisoned) => poisoned.into_inner(),
        };
        let Some(session) = guard.sessions.get_mut(&key.scope_id) else {
            return false;
        };
        session.streams.insert(key.call_id, cancel);
        true
    }

    /// Construct the device-side session dispatcher over the daemon-owned
    /// canonical Invocation lifecycle registry.
    ///
    /// Requiring this capability at construction prevents carrier BIDI/stream
    /// lifecycles and the `invocation.cancel` ability from observing different
    /// registries. There is no valid production dispatcher with a private
    /// cancellation authority.
    #[must_use]
    pub fn new(
        lifecycle_cancellations: crate::daemon::invocation::dispatch::cancellation::InvocationCancellationRegistry,
    ) -> Self {
        Self {
            escalation_correlation: None,
            carrier_sessions: Arc::new(Mutex::new(CarrierDispatchRegistry::default())),
            lifecycle_cancellations,
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
            crate::core::ura::URAKind::Device
                | crate::core::ura::URAKind::User
                | crate::core::ura::URAKind::Authority
        ) {
            return Ok(());
        }
        if self.admission.is_none() {
            return Ok(());
        }
        let Some(sync) = self.device_trust_sync.as_ref() else {
            return Err(canonical_runtime_assembly_unavailable(
                &format!("canonical carrier external signed caller `{caller_ura}` trust sync"),
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
        Err(external_caller_key_sync_error(caller_ura, &status))
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
                        "canonical carrier destination runtime admission staging failed: {status}"
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
                    "canonical carrier destination dispatch requires canonical runtime admission graph"
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
                "canonical carrier destination runtime admission commit failed: {status}"
            ))
        })
    }

    fn map_remote_file_transfer_output(
        call_id: u64,
        value: &Value,
    ) -> Result<Option<BidiOutputProjection>, SessionDispatchError> {
        match value.get("type").and_then(Value::as_str) {
            Some("complete") => {
                let payload = serde_json::to_vec(value).map_err(|err| {
                    SessionDispatchError::Other(format!(
                        "encode file_transfer completion payload: {err}"
                    ))
                })?;
                Ok(Some(BidiOutputProjection {
                    call_id,
                    payload,
                    content_type: crate::daemon::ability::wire::CONTROL_CONTENT_TYPE.to_string(),
                    failure: None,
                    disposition: BidiOutputDisposition::Completion,
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
                    content_type: crate::daemon::ability::wire::CONTROL_CONTENT_TYPE.to_string(),
                    failure: Some(error.failure()),
                    disposition: BidiOutputDisposition::Failure,
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
            Some("attached" | "detached" | "output_gap" | "exit") => {
                let payload = serde_json::to_vec(value).map_err(|err| {
                    SessionDispatchError::Other(format!("encode pty lifecycle frame failed: {err}"))
                })?;
                Ok(Some(BidiOutputProjection {
                    call_id,
                    payload,
                    content_type: crate::daemon::ability::wire::CONTROL_CONTENT_TYPE.to_string(),
                    failure: None,
                    disposition: BidiOutputDisposition::Data,
                }))
            }
            Some("error") => {
                let error = HandlerErrorFrame::parse(value, "pty error frame")?;
                let payload = serde_json::to_vec(value).map_err(|err| {
                    SessionDispatchError::Other(format!("encode pty error frame failed: {err}"))
                })?;
                Ok(Some(BidiOutputProjection {
                    call_id,
                    payload,
                    content_type: crate::daemon::ability::wire::CONTROL_CONTENT_TYPE.to_string(),
                    failure: Some(error.failure()),
                    disposition: BidiOutputDisposition::Failure,
                }))
            }
            Some("warn") => Ok(None),
            Some(other) => Err(SessionDispatchError::Other(format!(
                "unknown pty handler frame type {other:?}"
            ))),
            None => Ok(None),
        }
    }

    fn map_remote_tunnel_output(
        call_id: u64,
        value: &Value,
    ) -> Result<Option<BidiOutputProjection>, SessionDispatchError> {
        let (failure, disposition) = match value.get("type").and_then(Value::as_str) {
            Some("connected" | "listener_ready" | "accepted" | "half_close") => {
                (None, BidiOutputDisposition::Data)
            }
            Some("complete") => (None, BidiOutputDisposition::Completion),
            Some("error") => (
                Some(HandlerErrorFrame::parse(value, "tunnel error frame")?.failure()),
                BidiOutputDisposition::Failure,
            ),
            Some(other) => {
                return Err(SessionDispatchError::Other(format!(
                    "unknown tunnel handler frame type {other:?}"
                )))
            }
            None => return Ok(None),
        };
        let payload = serde_json::to_vec(value).map_err(|err| {
            SessionDispatchError::Other(format!("encode tunnel control frame failed: {err}"))
        })?;
        Ok(Some(BidiOutputProjection {
            call_id,
            payload,
            content_type: crate::daemon::ability::wire::CONTROL_CONTENT_TYPE.to_string(),
            failure,
            disposition,
        }))
    }

    fn map_remote_json_frame_output(
        call_id: u64,
        value: &Value,
    ) -> Result<Option<BidiOutputProjection>, SessionDispatchError> {
        let frame_type = value.get("type").and_then(Value::as_str);
        let payload = serde_json::to_vec(value).map_err(|err| {
            SessionDispatchError::Other(format!("plugin JSON-frame bidi encode failed: {err}"))
        })?;
        let failure = if frame_type == Some("error") {
            Some(HandlerErrorFrame::parse(value, "JSON-frame bidi error frame")?.failure())
        } else {
            None
        };
        Ok(Some(BidiOutputProjection {
            call_id,
            payload,
            content_type: crate::daemon::ability::wire::CONTROL_CONTENT_TYPE.to_string(),
            failure,
            disposition: if frame_type == Some("error") {
                BidiOutputDisposition::Failure
            } else {
                BidiOutputDisposition::Data
            },
        }))
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
        match local_bidi_wire_kind_for(registry, ability) {
            Some(LocalBidiWireKind::Pty) => Self::map_remote_pty_output(call_id, value),
            Some(LocalBidiWireKind::FileTransfer) => {
                Self::map_remote_file_transfer_output(call_id, value)
            }
            Some(LocalBidiWireKind::Tunnel) => Self::map_remote_tunnel_output(call_id, value),
            Some(LocalBidiWireKind::JsonFrames) => {
                Self::map_remote_json_frame_output(call_id, value)
            }
            None => Err(SessionDispatchError::Other(format!(
                "remote bidi ability {ability:?} has no registered wire profile"
            ))),
        }
    }

    fn map_native_bidi_data(
        call_id: u64,
        payload: Vec<u8>,
        content_type: String,
    ) -> BidiOutputProjection {
        BidiOutputProjection {
            call_id,
            payload,
            content_type,
            failure: None,
            disposition: BidiOutputDisposition::Data,
        }
    }

    /// Canonical carrier bidi open: the request is the complete signed Invocation, so
    /// the open enters the same descriptor-bound runtime path as unary calls.
    async fn handle_canonical_carrier_bidi_open(
        &self,
        call_id: u64,
        ability: String,
        request: axon_sdk::pb::axon::v1::InvokeRequest,
        input_rx: mpsc::Receiver<PendingRemoteBidiInput>,
        outbound: &SessionUpSender,
    ) -> Result<(), SessionDispatchError> {
        crate::op_event!(
            component = local_session_dispatcher,
            kind = received_canonical_carrier_bidi_open,
            call_id = call_id,
            ability = ability,
        );
        if local_bidi_wire_kind_for(&self.ability_wire, &ability).is_none() {
            return Self::send_bidi_control_failure(
                outbound,
                call_id,
                "ABILITY_BIDI_NOT_SUPPORTED",
                format!(
                    "remote bidi ability `{ability}` is not published for session.open canonical carrier"
                ),
            )
            .await;
        }
        let Some(envelope) = request.envelope else {
            return self
                .fail_remote_bidi_opening(
                    outbound,
                    call_id,
                    "ENVELOPE_INCOMPLETE",
                    "canonical carrier bidi open missing envelope",
                )
                .await;
        };
        let runtime = match self.require_local_runtime("session.open remote bidi") {
            Ok(runtime) => runtime,
            Err(error) => {
                return self
                    .fail_remote_bidi_opening(
                        outbound,
                        call_id,
                        "RUNTIME_UNAVAILABLE",
                        error.to_string(),
                    )
                    .await;
            }
        };
        let target_ura =
            match callee_ura_from_envelope(Some(&envelope), "canonical carrier BidiOpen") {
                Ok(target_ura) => target_ura,
                Err(status) => {
                    return self
                        .fail_remote_bidi_opening(
                            outbound,
                            call_id,
                            "ENVELOPE_INCOMPLETE",
                            status.message(),
                        )
                        .await;
                }
            };
        if let Err(err) = self.sync_external_signed_caller_key(&envelope).await {
            return self
                .fail_remote_bidi_opening(
                    outbound,
                    call_id,
                    "CALLER_KEY_SYNC_FAILED",
                    err.to_string(),
                )
                .await;
        }
        crate::op_event!(
            component = local_session_dispatcher,
            kind = canonical_carrier_bidi_caller_key_synced,
            call_id = call_id,
            ability = ability,
            target_ura = target_ura,
        );
        let bound_ability = match RuntimeBoundAbility::from_wire_target(
            "canonical carrier BidiOpen",
            &runtime,
            &target_ura,
            &ability,
        )
        .await
        {
            Ok(bound_ability) => bound_ability,
            Err(status) => {
                return self
                    .fail_remote_bidi_opening(
                        outbound,
                        call_id,
                        "ABILITY_RESOLUTION_FAILED",
                        status.message(),
                    )
                    .await;
            }
        };
        crate::op_event!(
            component = local_session_dispatcher,
            kind = canonical_carrier_bidi_ability_bound,
            call_id = call_id,
            ability = ability,
            target_ura = target_ura,
        );
        let descriptor_ref = match bound_ability.signed_descriptor_ref_from_target(
            "canonical carrier BidiOpen",
            &target_ura,
            axon_sdk::invocation::CallMode::Bidi,
            request.target.as_ref(),
        ) {
            Ok(ref_) => ref_,
            Err(status) => {
                return self
                    .fail_remote_bidi_opening(
                        outbound,
                        call_id,
                        "DESCRIPTOR_BINDING_FAILED",
                        status.message(),
                    )
                    .await;
            }
        };
        let descriptor_ref_label = descriptor_ref.clone().into_descriptor_ref();
        crate::op_event!(
            component = local_session_dispatcher,
            kind = canonical_carrier_bidi_descriptor_bound,
            call_id = call_id,
            ability = ability,
            descriptor_ref = descriptor_ref_label,
        );
        let wire = match crate::daemon::axon_bridge::descriptor_bound_dispatch::external_signed_from_wire_parts(
            envelope,
            descriptor_ref.into_descriptor_ref(),
            request.arguments,
            request.metadata,
        ) {
            Ok(wire) => wire,
            Err(err) => {
                return self
                    .fail_remote_bidi_opening(
                    outbound,
                    call_id,
                    "INVOCATION_WIRE_INVALID",
                    format!("build canonical carrier admitted bidi open: {err}"),
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
                return self
                    .fail_remote_bidi_opening(
                        outbound,
                        call_id,
                        "PRODUCT_ADMISSION_REJECTED",
                        err.to_string(),
                    )
                    .await;
            }
        };
        crate::op_event!(
            component = local_session_dispatcher,
            kind = canonical_carrier_bidi_runtime_admission_staged,
            call_id = call_id,
            ability = ability,
        );
        let lifecycle_envelope = wire.envelope.clone();
        crate::op_event!(
            component = local_session_dispatcher,
            kind = canonical_carrier_bidi_open_admitted_started,
            call_id = call_id,
            ability = ability,
        );
        let handle =
            match crate::daemon::axon_bridge::descriptor_bound_dispatch::open_bidi_admitted(
                &runtime, wire,
            )
            .await
            {
                Ok(handle) => handle,
                Err(err) => {
                    self.remove_remote_bidi_session(outbound.scope_id(), call_id);
                    return Self::send_canonical_carrier_dispatch_result(
                        outbound,
                        canonical_carrier_invocation_failure(call_id, &err),
                    )
                    .await;
                }
            };
        crate::op_event!(
            component = local_session_dispatcher,
            kind = canonical_carrier_bidi_open_admitted_completed,
            call_id = call_id,
            ability = ability,
        );
        if let Err(err) = Self::commit_runtime_admission(runtime_admission) {
            self.remove_remote_bidi_session(outbound.scope_id(), call_id);
            return Self::cancel_opened_bidi(
                outbound,
                call_id,
                handle,
                format!("runtime admission commit failed: {err}"),
            )
            .await;
        }
        crate::op_event!(
            component = local_session_dispatcher,
            kind = canonical_carrier_bidi_runtime_admission_committed,
            call_id = call_id,
            ability = ability,
        );
        self.register_remote_bidi(
            call_id,
            &ability,
            handle,
            input_rx,
            outbound,
            lifecycle_envelope,
        )
        .await
    }

    async fn send_canonical_carrier_dispatch_result(
        outbound: &SessionUpSender,
        result: axon_sdk::pb::axon::v1::DispatchResult,
    ) -> Result<(), SessionDispatchError> {
        if !outbound.canonical_carrier() {
            return Err(SessionDispatchError::Other(
                "canonical dispatch result requires negotiated session canonical carrier"
                    .to_string(),
            ));
        }
        outbound
            .send_payload(UpPayload::DispatchResult(result))
            .await
            .map_err(|_| SessionDispatchError::Other("session up channel closed".to_string()))
    }

    async fn send_canonical_carrier_control_failure(
        outbound: &SessionUpSender,
        call_id: u64,
        code: &'static str,
        message: impl Into<String>,
    ) -> Result<(), SessionDispatchError> {
        Self::send_canonical_carrier_dispatch_result(
            outbound,
            canonical_carrier_control_failure(call_id, code, message),
        )
        .await
    }

    async fn send_bidi_control_failure(
        outbound: &SessionUpSender,
        call_id: u64,
        code: &'static str,
        message: impl Into<String>,
    ) -> Result<(), SessionDispatchError> {
        Self::send_canonical_carrier_control_failure(outbound, call_id, code, message).await
    }

    async fn send_bidi_admission(
        outbound: &SessionUpSender,
        call_id: u64,
        receipt: &axon_sdk::invocation::SignedInvocationReceipt,
    ) -> Result<(), SessionDispatchError> {
        Self::send_canonical_carrier_dispatch_result(
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
        Self::send_canonical_carrier_dispatch_result(
            outbound,
            axon_sdk::pb::axon::v1::DispatchResult {
                call_id: projection.call_id,
                payload: projection.payload,
                result_content_type: projection.content_type,
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
        Self::send_canonical_carrier_dispatch_result(
            outbound,
            Self::canonical_terminal_dispatch_result(call_id, finalized)?,
        )
        .await
    }

    fn canonical_terminal_dispatch_result(
        call_id: u64,
        finalized: &axon_sdk::invocation::FinalizedInvocation,
    ) -> Result<axon_sdk::pb::axon::v1::DispatchResult, SessionDispatchError> {
        Ok(axon_sdk::pb::axon::v1::DispatchResult {
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
        })
    }

    fn insert_remote_bidi_opening(
        &self,
        scope_id: u64,
        call_id: u64,
        ability: &str,
    ) -> Result<mpsc::Receiver<PendingRemoteBidiInput>, SessionDispatchError> {
        let mut guard = match self.carrier_sessions.lock() {
            Ok(g) => g,
            Err(poisoned) => poisoned.into_inner(),
        };
        let Some(session) = guard.sessions.get_mut(&scope_id) else {
            return Err(SessionDispatchError::Other(format!(
                "carrier scope_id={scope_id} ended before remote bidi call_id={call_id} opened"
            )));
        };
        if session.bidi.contains_key(&call_id) {
            return Err(SessionDispatchError::Other(format!(
                "remote bidi scope_id={scope_id} call_id={call_id} already exists"
            )));
        }
        let (ingress, input_rx) = mpsc::channel(REMOTE_BIDI_INPUT_CAPACITY);
        session.bidi.insert(
            call_id,
            RemoteBidiSession::Opening {
                ingress,
                half_closed: false,
            },
        );
        crate::op_event!(
            component = local_session_dispatcher,
            kind = remote_bidi_opening_registered,
            scope_id = scope_id,
            call_id = call_id,
            ability = ability,
        );
        Ok(input_rx)
    }

    fn remove_remote_bidi_session(&self, scope_id: u64, call_id: u64) {
        let mut guard = match self.carrier_sessions.lock() {
            Ok(g) => g,
            Err(poisoned) => poisoned.into_inner(),
        };
        if let Some(session) = guard.sessions.get_mut(&scope_id) {
            session.bidi.remove(&call_id);
        }
    }

    async fn fail_remote_bidi_opening(
        &self,
        outbound: &SessionUpSender,
        call_id: u64,
        code: &'static str,
        message: impl Into<String>,
    ) -> Result<(), SessionDispatchError> {
        self.remove_remote_bidi_session(outbound.scope_id(), call_id);
        Self::send_bidi_control_failure(outbound, call_id, code, message).await
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
    /// phase. Admission is a delivery barrier: `BidiInput` may be observed
    /// while the call is Opening, but it remains in the bounded ingress queue.
    /// A full queue backpressures the session transport; no frame is discarded.
    /// No input is delivered to the handler until the canonical admission proof
    /// has been published upstream. This preserves the lifecycle order
    /// `Opening -> Admitted -> Active -> Terminal` even when the peer queues
    /// input immediately after its open frame.
    async fn register_remote_bidi(
        &self,
        call_id: u64,
        ability: &str,
        handle: axon_sdk::invocation::BidiInvocationHandle,
        mut input_rx: mpsc::Receiver<PendingRemoteBidiInput>,
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
        crate::op_event!(
            component = local_session_dispatcher,
            kind = canonical_carrier_bidi_admission_receipt_observed,
            call_id = call_id,
            ability = ability,
            invocation_id = admission.invocation_id(),
        );
        if let Err(error) = Self::send_bidi_admission(outbound, call_id, &admission).await {
            let _ = lifecycle
                .cancel_and_finalize("session bidi closed before admission")
                .await;
            return Err(error);
        }
        crate::op_event!(
            component = local_session_dispatcher,
            kind = canonical_carrier_bidi_admission_sent,
            call_id = call_id,
            ability = ability,
        );

        let key = CarrierCallKey {
            scope_id: outbound.scope_id(),
            call_id,
        };
        let transition = {
            let mut guard = match self.carrier_sessions.lock() {
                Ok(g) => g,
                Err(poisoned) => poisoned.into_inner(),
            };
            match guard.sessions.get_mut(&key.scope_id) {
                Some(session) => match session.bidi.get(&call_id) {
                    Some(RemoteBidiSession::Opening {
                        ingress,
                        half_closed,
                    }) => {
                        let active = ActiveRemoteBidi {
                            ingress: ingress.clone(),
                            half_closed: *half_closed,
                        };
                        session
                            .bidi
                            .insert(call_id, RemoteBidiSession::Active(active));
                        crate::op_event!(
                            component = local_session_dispatcher,
                            kind = remote_bidi_opening_promoted_active,
                            scope_id = key.scope_id,
                            call_id = call_id,
                            ability = ability,
                        );
                        Ok(())
                    }
                    Some(RemoteBidiSession::Active(_)) => Err(format!(
                        "remote bidi scope_id={} call_id={call_id} was already active",
                        key.scope_id
                    )),
                    None => Err(format!(
                        "remote bidi scope_id={} call_id={call_id} no longer owns its opening",
                        key.scope_id
                    )),
                },
                None => Err(format!(
                    "carrier scope_id={} ended during bidi open",
                    key.scope_id
                )),
            }
        };
        if let Err(message) = transition {
            return Self::send_bidi_control_failure(
                outbound,
                call_id,
                "BIDI_SESSION_CONFLICT",
                message,
            )
            .await;
        }

        let dispatcher_for_input = self.clone();
        let outbound_for_input = outbound.clone();
        tokio::spawn(async move {
            while let Some(pending) = input_rx.recv().await {
                let eof = pending.eof;
                if dispatcher_for_input
                    .send_remote_bidi_input_to_handler(
                        call_id,
                        &handler_in_tx,
                        pending.content_type,
                        pending.payload,
                        eof,
                        &outbound_for_input,
                    )
                    .await
                    .is_err()
                {
                    break;
                }
                if eof {
                    break;
                }
            }
        });

        let sessions = Arc::clone(&self.carrier_sessions);
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
                } else if !frame.content_type.is_empty() && frame.content_type != "application/json"
                {
                    Some(LocalAxonSessionDispatcher::map_native_bidi_data(
                        call_id,
                        frame.payload,
                        frame.content_type,
                    ))
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
                if mapped.disposition == BidiOutputDisposition::Completion {
                    // Axon follows the result-bearing ability frame with the
                    // signed terminal checkpoint. Completion belongs only to
                    // that checkpoint; emitting it here would duplicate
                    // terminal metadata as ordinary stream bytes.
                    continue;
                }
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
            if let Some(session) = guard.sessions.get_mut(&key.scope_id) {
                session.bidi.remove(&key.call_id);
            }
        });
        Ok(())
    }

    async fn forward_remote_bidi_input(
        &self,
        call_id: u64,
        content_type: String,
        payload: Vec<u8>,
        eof: bool,
        outbound: &SessionUpSender,
    ) -> Result<(), SessionDispatchError> {
        enum InputRoute {
            Ingress(mpsc::Sender<PendingRemoteBidiInput>),
            Missing,
        }

        let key = CarrierCallKey {
            scope_id: outbound.scope_id(),
            call_id,
        };
        let route = {
            let mut guard = match self.carrier_sessions.lock() {
                Ok(g) => g,
                Err(poisoned) => poisoned.into_inner(),
            };
            match guard.sessions.get_mut(&key.scope_id) {
                Some(session) => match session.bidi.get_mut(&call_id) {
                    Some(RemoteBidiSession::Active(active)) if !active.half_closed => {
                        let ingress = active.ingress.clone();
                        if eof {
                            active.half_closed = true;
                        }
                        InputRoute::Ingress(ingress)
                    }
                    Some(RemoteBidiSession::Opening {
                        ingress,
                        half_closed,
                    }) if !*half_closed => {
                        let ingress = ingress.clone();
                        if eof {
                            *half_closed = true;
                        }
                        InputRoute::Ingress(ingress)
                    }
                    Some(RemoteBidiSession::Active(_))
                    | Some(RemoteBidiSession::Opening { .. }) => InputRoute::Missing,
                    None => InputRoute::Missing,
                },
                None => InputRoute::Missing,
            }
        };

        let ingress = match route {
            InputRoute::Ingress(ingress) => ingress,
            InputRoute::Missing => {
                if eof {
                    let stream_cancel = {
                        let mut guard = match self.carrier_sessions.lock() {
                            Ok(g) => g,
                            Err(poisoned) => poisoned.into_inner(),
                        };
                        guard
                            .sessions
                            .get_mut(&key.scope_id)
                            .and_then(|session| session.streams.remove(&call_id))
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
            }
        };

        ingress
            .send(PendingRemoteBidiInput {
                content_type,
                payload,
                eof,
            })
            .await
            .map_err(|_| {
                SessionDispatchError::Other(format!(
                    "remote bidi call_id={call_id} ingress closed before delivery"
                ))
            })
    }

    async fn send_remote_bidi_input_to_handler(
        &self,
        call_id: u64,
        sender: &BidiInputSender,
        content_type: String,
        payload: Vec<u8>,
        eof: bool,
        outbound: &SessionUpSender,
    ) -> Result<(), SessionDispatchError> {
        let send_result = if payload.is_empty() {
            Ok(())
        } else {
            sender
                .send(BidiInputFrame::new(payload).with_content_type(content_type))
                .await
                .map(|_| ())
        };
        if eof {
            let _ = sender.close_input().await;
        }
        if send_result.is_err() {
            if eof {
                // Some bidi abilities legitimately finish before consuming a
                // best-effort EOF frame; terminal authority remains the
                // handler's receipt chain, not EOF delivery.
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

fn external_caller_key_sync_error(
    caller_ura: &str,
    status: &crate::daemon::invocation::admission::device_trust_sync::DeviceTrustSyncStatus,
) -> SessionDispatchError {
    let diagnostic = status
        .diagnostic()
        .unwrap_or_else(|| "trust sync did not produce a trusted key".to_string());
    let reason =
        crate::daemon::invocation::admission::decision::SignatureDecisionReason::CallerKeyNotFound
            .as_str();
    SessionDispatchError::Other(format!(
        "{reason}: canonical carrier external signed caller `{caller_ura}` is not trusted after resolve_key sync: {diagnostic}"
    ))
}

#[async_trait::async_trait]
impl SessionFrameDispatcher for LocalAxonSessionDispatcher {
    fn session_started(&self, scope_id: u64) {
        self.begin_carrier_scope(scope_id);
    }

    fn session_ended(&self, scope_id: u64) {
        self.end_carrier_scope(scope_id);
    }

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
            let id_hex = call_id_hex(&call_id);
            if let Some(correlation) = self.escalation_correlation.as_ref() {
                let delivered =
                    correlation.deliver_reverse_dispatch_result(call_id, result.clone());
                crate::op_event!(
                    component = local_session_dispatcher,
                    kind = reverse_dispatch_result_delivered,
                    call_id = id_hex,
                    delivered = delivered,
                    terminal = result.terminal,
                    has_admission = result.admission_receipt.is_some(),
                    has_terminal = result.terminal_receipt.is_some(),
                    payload_bytes = result.payload.len(),
                );
            } else {
                crate::op_event!(
                    component = local_session_dispatcher,
                    kind = reverse_dispatch_result_dropped_no_correlation,
                    call_id = id_hex,
                );
            }
            return Ok(());
        }

        // DispatchCall carries the complete canonical InvokeRequest and is
        // dispatched without a product-side request projection.
        if let Some(DownPayload::DispatchCall(call)) = frame.payload.as_ref() {
            if matches!(
                canonical_dispatch_call_mode(call.call_mode),
                Ok(axon_sdk::invocation::CallMode::Bidi)
            ) {
                return self
                    .schedule_canonical_carrier_bidi_open(call.clone(), outbound)
                    .await;
            }
            let dispatcher = self.clone();
            let outbound = outbound.clone();
            let call = call.clone();
            tokio::spawn(async move {
                if let Err(err) = dispatcher
                    .handle_canonical_carrier_dispatch(call, &outbound)
                    .await
                {
                    crate::op_event!(
                        component = local_session_dispatcher,
                        kind = canonical_carrier_dispatch_task_failed,
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

        if let Some(input) =
            crate::daemon::invocation::bidi::session_wire::decode_carrier_bidi_input(&chunk.data)
                .map_err(|error| {
                    SessionDispatchError::Other(format!(
                        "session down native Bidi input is invalid: {error}"
                    ))
                })?
        {
            return self
                .forward_remote_bidi_input(
                    input.call_id,
                    input.content_type,
                    input.payload,
                    input.eof,
                    outbound,
                )
                .await;
        }

        let dispatch = SessionDispatch::decode_frame(&chunk.data).map_err(|err| {
            SessionDispatchError::Other(format!(
                "session down BinaryChunk is not valid SessionDispatch JSON: {err}"
            ))
        })?;

        match dispatch {
            SessionDispatch::StreamCancel { call_id, reason } => {
                let key = CarrierCallKey {
                    scope_id: outbound.scope_id(),
                    call_id,
                };
                let cancel = {
                    let mut guard = match self.carrier_sessions.lock() {
                        Ok(guard) => guard,
                        Err(poisoned) => poisoned.into_inner(),
                    };
                    guard
                        .sessions
                        .get_mut(&key.scope_id)
                        .and_then(|session| session.streams.remove(&key.call_id))
                };
                let Some(cancel) = cancel else {
                    return Self::send_bidi_control_failure(
                        outbound,
                        call_id,
                        "STREAM_SESSION_NOT_OPEN",
                        format!(
                            "canonical stream call_id={call_id} is not open in carrier scope {}",
                            key.scope_id
                        ),
                    )
                    .await;
                };
                crate::op_event!(
                    component = local_session_dispatcher,
                    kind = canonical_carrier_stream_cancel_requested,
                    call_id = call_id,
                    reason = reason,
                );
                cancel.cancel();
                return Ok(());
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
            SessionDispatch::ReverseStreamCancel { call_id, .. } => {
                crate::op_event!(
                    component = local_session_dispatcher,
                    kind = unexpected_downstream_frame,
                    frame_kind = "ReverseStreamCancel",
                    call_id = call_id_hex(&call_id),
                );
                Ok(())
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
    const TEST_LOCOMOTION_SYSTEM_AGENT_URA: &str = "easynet:///r/t/agent/device.d1.locomotion";

    fn start_test_carrier(
        dispatcher: &LocalAxonSessionDispatcher,
        tx: mpsc::Sender<InvokeBidiUp>,
    ) -> SessionUpSender {
        let outbound = SessionUpSender::new(tx);
        outbound.set_negotiated_contract(
            crate::daemon::invocation::bidi::state::presence::CANONICAL_SESSION_CARRIER_VERSION,
        );
        dispatcher.session_started(outbound.scope_id());
        outbound
    }

    #[test]
    fn carrier_scope_owns_call_ids_and_releases_them_for_reconnect() {
        let dispatcher = LocalAxonSessionDispatcher::new(Default::default());
        let first_scope = 41;
        let next_scope = 42;
        let reused_call_id = 5;

        dispatcher.session_started(first_scope);
        dispatcher
            .insert_remote_bidi_opening(first_scope, reused_call_id, "terminal.attach")
            .expect("first carrier owns its call id");
        assert!(
            dispatcher
                .insert_remote_bidi_opening(first_scope, reused_call_id, "terminal.attach")
                .is_err(),
            "a duplicate call id inside one carrier must be rejected"
        );

        let stream_cancel = CancellationToken::new();
        assert!(dispatcher.insert_remote_stream(
            CarrierCallKey {
                scope_id: first_scope,
                call_id: 7,
            },
            stream_cancel.clone(),
        ));

        dispatcher.session_ended(first_scope);
        assert!(
            stream_cancel.is_cancelled(),
            "retiring a carrier must cancel every stream it owns"
        );
        {
            let registry = dispatcher
                .carrier_sessions
                .lock()
                .unwrap_or_else(|poisoned| poisoned.into_inner());
            assert!(
                !registry.sessions.contains_key(&first_scope),
                "no BIDI or stream state may survive carrier retirement"
            );
        }

        dispatcher.session_started(next_scope);
        dispatcher
            .insert_remote_bidi_opening(next_scope, reused_call_id, "terminal.attach")
            .expect("a reconnect may safely reuse a wire call id in its own scope");
    }

    #[test]
    fn simultaneous_carriers_do_not_share_call_id_ownership() {
        let dispatcher = LocalAxonSessionDispatcher::new(Default::default());
        dispatcher.session_started(51);
        dispatcher.session_started(52);

        dispatcher
            .insert_remote_bidi_opening(51, 3, "terminal.attach")
            .expect("first carrier call");
        dispatcher
            .insert_remote_bidi_opening(52, 3, "terminal.attach")
            .expect("second carrier may use the same wire call id");

        let registry = dispatcher
            .carrier_sessions
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner());
        assert!(registry.sessions[&51].bidi.contains_key(&3));
        assert!(registry.sessions[&52].bidi.contains_key(&3));
    }

    #[tokio::test]
    async fn opening_bidi_ingress_backpressures_without_dropping_ten_mibibytes() {
        const CHUNK_BYTES: usize = 64 * 1024;
        const SOURCE_BYTES: usize = 10 * 1024 * 1024;

        let dispatcher = LocalAxonSessionDispatcher::new(Default::default());
        let (tx, _rx) = mpsc::channel::<InvokeBidiUp>(2);
        let outbound = start_test_carrier(&dispatcher, tx);
        let mut ingress_rx = dispatcher
            .insert_remote_bidi_opening(outbound.scope_id(), 61, "file.transfer")
            .expect("opening owns one bounded ingress");
        let source = (0..SOURCE_BYTES)
            .map(|index| ((index * 31 + 7) % 251) as u8)
            .collect::<Vec<_>>();

        let producer_source = source.clone();
        let producer_dispatcher = dispatcher.clone();
        let producer_outbound = outbound.clone();
        let producer = tokio::spawn(async move {
            for chunk in producer_source.chunks(CHUNK_BYTES) {
                producer_dispatcher
                    .forward_remote_bidi_input(
                        61,
                        "application/octet-stream".to_string(),
                        chunk.to_vec(),
                        false,
                        &producer_outbound,
                    )
                    .await
                    .expect("every data frame enters the lossless ingress");
            }
            producer_dispatcher
                .forward_remote_bidi_input(61, String::new(), Vec::new(), true, &producer_outbound)
                .await
                .expect("EOF follows all data frames");
        });

        while ingress_rx.len() < REMOTE_BIDI_INPUT_CAPACITY {
            tokio::task::yield_now().await;
        }
        tokio::task::yield_now().await;
        assert!(
            !producer.is_finished(),
            "the 33rd frame must backpressure instead of deleting the call or dropping bytes"
        );

        let mut received = Vec::with_capacity(SOURCE_BYTES);
        while let Some(frame) = ingress_rx.recv().await {
            received.extend_from_slice(&frame.payload);
            if frame.eof {
                break;
            }
        }
        producer.await.expect("producer task completes after drain");

        assert_eq!(received.len(), SOURCE_BYTES);
        assert_eq!(received, source);
    }

    #[tokio::test]
    async fn late_frame_cannot_resurrect_a_retired_carrier_scope() {
        let dispatcher = LocalAxonSessionDispatcher::new(Default::default());
        let (tx, mut rx) = mpsc::channel::<InvokeBidiUp>(2);
        let outbound = start_test_carrier(&dispatcher, tx);
        let retired_scope = outbound.scope_id();
        dispatcher.session_ended(retired_scope);

        dispatcher
            .handle_down(
                carrier_bidi_input_frame(9, "application/octet-stream", b"late", false),
                &outbound,
            )
            .await
            .expect("late data is rejected as a typed carrier result");

        let reply = rx.recv().await.expect("typed rejection emitted");
        let Some(UpPayload::DispatchResult(result)) = reply.payload else {
            panic!("late frame rejection must be a canonical DispatchResult");
        };
        assert_eq!(
            result.failure.as_ref().map(|failure| failure.code.as_str()),
            Some("BIDI_SESSION_NOT_OPEN")
        );
        let registry = dispatcher
            .carrier_sessions
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner());
        assert!(
            !registry.sessions.contains_key(&retired_scope),
            "dispatch must not recreate a carrier after its terminal transition"
        );
    }

    #[test]
    fn caller_key_sync_failure_starts_with_canonical_admission_reason() {
        let error = external_caller_key_sync_error(
            "easynet:///r/t/user/alice",
            &crate::daemon::invocation::admission::device_trust_sync::DeviceTrustSyncStatus::ResolveFailed(
                "authority has no matching presented key".to_string(),
            ),
        );
        match error {
            SessionDispatchError::Other(message) => {
                assert!(
                    message.starts_with("CALLER_KEY_NOT_FOUND:"),
                    "machine consumers require the canonical admission reason; got: {message}"
                );
                assert!(message.contains("easynet:///r/t/user/alice"));
            }
        }
    }

    #[test]
    fn canonical_carrier_stream_control_failure_is_not_lifecycle_terminal() {
        let result = canonical_carrier_control_failure(
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

    #[test]
    fn canonical_carrier_unary_control_failure_is_not_lifecycle_terminal() {
        let result = canonical_carrier_control_failure(
            10,
            "ABILITY_RESOLUTION_FAILED",
            "descriptor missing",
        );
        assert_eq!(result.call_id, 10);
        assert!(
            !result.terminal,
            "synthetic unary control failures must not claim canonical terminality"
        );
        assert!(
            result.terminal_receipt.is_none(),
            "control failures must not synthesize terminal receipts"
        );
        assert_eq!(
            result.failure.as_ref().map(|failure| failure.code.as_str()),
            Some("ABILITY_RESOLUTION_FAILED")
        );
        assert_eq!(
            result
                .failure
                .as_ref()
                .map(|failure| failure.message.as_str()),
            Some("descriptor missing")
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

    fn canonical_carrier_signing_key() -> ed25519_dalek::SigningKey {
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
        crate::daemon::axon_bridge::descriptor_ref::system_protocol_descriptor_ref_for_wire(
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
            Arc::new(FixedCarrierKey(
                canonical_carrier_signing_key().verifying_key(),
            )),
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

    fn carrier_bidi_input_frame(
        call_id: u64,
        content_type: &str,
        payload: &[u8],
        eof: bool,
    ) -> InvokeBidiDown {
        InvokeBidiDown {
            sequence: 0,
            payload: Some(DownPayload::BinaryChunk(BinaryChunk {
                data: crate::daemon::invocation::bidi::session_wire::encode_carrier_bidi_input(
                    call_id,
                    content_type,
                    payload,
                    eof,
                )
                .expect("encode carrier bidi input"),
                ..BinaryChunk::default()
            })),
            ..InvokeBidiDown::default()
        }
    }

    fn canonical_carrier_explicit_test_call(
        call_id: u64,
        ability: &str,
        args: Vec<u8>,
    ) -> InvokeBidiDown {
        canonical_carrier_explicit_test_call_with_mode(
            call_id,
            ability,
            args,
            axon_sdk::invocation::CallMode::Rpc,
        )
    }

    fn canonical_carrier_explicit_test_call_with_mode(
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
        canonical_carrier_call_signed_as_with_mode(call_id, ability, &descriptor_ref, args, mode)
    }

    fn canonical_carrier_call_signed_as(
        call_id: u64,
        request_ability: &str,
        signed_ability: &str,
        args: Vec<u8>,
    ) -> InvokeBidiDown {
        canonical_carrier_call_signed_as_with_mode(
            call_id,
            request_ability,
            signed_ability,
            args,
            axon_sdk::invocation::CallMode::Rpc,
        )
    }

    fn canonical_carrier_call_signed_as_with_mode(
        call_id: u64,
        request_ability: &str,
        signed_ability: &str,
        args: Vec<u8>,
        mode: axon_sdk::invocation::CallMode,
    ) -> InvokeBidiDown {
        canonical_carrier_call_signed_as_with_mode_for_target(
            call_id,
            request_ability,
            signed_ability,
            args,
            mode,
            TEST_DEVICE_URA,
            TEST_DEVICE_URA,
        )
    }

    fn canonical_carrier_call_signed_as_with_mode_for_target(
        call_id: u64,
        request_ability: &str,
        signed_ability: &str,
        args: Vec<u8>,
        mode: axon_sdk::invocation::CallMode,
        callee_ura: &str,
        subject_ura: &str,
    ) -> InvokeBidiDown {
        use axon_sdk::pb::axon::v1::{DispatchCall, InvokeRequest};
        use ed25519_dalek::Signer as _;

        let signing_key = canonical_carrier_signing_key();
        let signed_descriptor_ref = descriptor_ref_for_call_mode(callee_ura, signed_ability, mode);
        let mut envelope = crate::daemon::invocation::ProtoEnvelope::from_target(
            TEST_CALLER_URA,
            callee_ura,
            subject_ura,
            crate::daemon::invocation::InvocationDerivationPolicy::FreshRoot,
        )
        .expect("valid canonical carrier envelope")
        .into_inner(&signed_descriptor_ref, &args)
        .expect("complete canonical carrier tuple");
        let descriptor_bound =
            crate::daemon::axon_bridge::wire_descriptor::descriptor_bound_from_wire_parts(
                envelope.clone(),
                signed_descriptor_ref.clone(),
                &args,
            )
            .expect("descriptor-bound canonical carrier envelope");
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
                .expect("canonical carrier typed target"),
            ),
            arguments: args,
            ..Default::default()
        };

        InvokeBidiDown {
            payload: Some(DownPayload::DispatchCall(DispatchCall {
                call_id,
                request: Some(request),
                call_mode: crate::daemon::invocation::bidi::session_wire::canonical_call_mode_wire(
                    mode,
                ),
            })),
            ..InvokeBidiDown::default()
        }
    }

    fn canonical_carrier_bidi_open(
        call_id: u64,
        ability: &str,
        args: Vec<u8>,
        subject_ura: &str,
    ) -> InvokeBidiDown {
        let mut frame = canonical_carrier_call_signed_as_with_mode_for_target(
            call_id,
            ability,
            ability,
            args,
            axon_sdk::invocation::CallMode::Bidi,
            TEST_LOCOMOTION_SYSTEM_AGENT_URA,
            subject_ura,
        );
        if let Some(DownPayload::DispatchCall(call)) = frame.payload.as_mut() {
            call.call_mode =
                crate::daemon::invocation::bidi::session_wire::canonical_call_mode_wire(
                    axon_sdk::invocation::CallMode::Bidi,
                );
        }
        frame
    }

    fn canonical_carrier_explicit_test_bidi_open(
        call_id: u64,
        ability: &str,
        args: Vec<u8>,
    ) -> InvokeBidiDown {
        let mut frame = canonical_carrier_explicit_test_call_with_mode(
            call_id,
            ability,
            args,
            axon_sdk::invocation::CallMode::Bidi,
        );
        if let Some(DownPayload::DispatchCall(call)) = frame.payload.as_mut() {
            call.call_mode =
                crate::daemon::invocation::bidi::session_wire::canonical_call_mode_wire(
                    axon_sdk::invocation::CallMode::Bidi,
                );
        }
        frame
    }

    /// Quadrant [new hub, new device] for step-3b: a canonical carrier bidi
    /// open admits through the canonical wire-parts path, streams
    /// over the same byte channel, and the terminal frame replies as
    /// a proto DispatchResult.
    #[test]
    fn canonical_carrier_bidi_open_round_trips_and_replies_proto_on_canonical_session() {
        let _home = crate::cli::commands::test_support::HomeGuard::new();
        crate::daemon::persistence::config::save_credentials(
            &crate::daemon::persistence::config::Credentials {
                node_id: "d1".to_string(),
                realm: "t".to_string(),
                credential_token: "token".to_string(),
                hub_endpoint: "https://hub.example:50443".to_string(),
                join_receipt_hash: Some("join-hash".to_string()),
                username: Some("alice".to_string()),
                user_id: Some("alice".to_string()),
                ..Default::default()
            },
        )
        .expect("test Device identity");
        let runtime = tokio::runtime::Builder::new_multi_thread()
            .worker_threads(2)
            .thread_stack_size(32 * 1024 * 1024)
            .enable_all()
            .build()
            .expect("test runtime");
        let test = runtime.spawn(async {
            let tmp = tempfile::tempdir().expect("tempdir");
            let target = tmp.path().join("upload-from-hub-canonical.bin");
            let bytes = b"canonical carrier-bidi-over-session";

        let rt = executable_runtime();
        let registry = build_real_daemon_registry_with_runtime(Some(Arc::clone(&rt)));
        let lifecycle_cancellations = registry.invocation_cancellations.clone();
        let disp = LocalAxonSessionDispatcher::new(lifecycle_cancellations.clone())
            .with_local_runtime(rt);
        let (tx, mut rx) = mpsc::channel::<InvokeBidiUp>(8);
        let session_tx = start_test_carrier(&disp, tx);
        session_tx.set_negotiated_contract(
            crate::daemon::invocation::bidi::state::presence::CANONICAL_SESSION_CARRIER_VERSION,
        );

        let resource_ref = crate::daemon::resources::files::FilesystemResourceProvider::for_device(
            TEST_DEVICE_URA,
        )
        .expect("test filesystem Device authority")
        .resource_ref_for_local_path(
            &target,
            crate::daemon::resources::files::FilesystemResourceCapability::Write,
        )
        .expect("local fs ResourceRef");
        let subject_ura = resource_ref["resource_ura"]
            .as_str()
            .expect("ResourceRef subject URA")
            .to_string();
        let args = serde_json::to_vec(&json!({
            "mode": "upload",
            "resource_ref": resource_ref,
        }))
        .expect("encode args");
        disp.handle_down(
            canonical_carrier_bidi_open(
                77,
                crate::daemon::ability::builtins::device_control::file_transfer::ABILITY_FILE_TRANSFER,
                args,
                &subject_ura,
            ),
            &session_tx,
        )
        .await
        .expect("canonical bidi open succeeds");

        disp.handle_down(
            carrier_bidi_input_frame(77, "application/octet-stream", bytes, false),
            &session_tx,
        )
        .await
        .expect("bidi chunk forwards");
        disp.handle_down(
            carrier_bidi_input_frame(77, "", &[], true),
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
            other => {
                panic!("expected admission DispatchResult on a canonical session, got: {other:?}")
            }
        };
        assert_eq!(admission.call_id, 77);
        assert!(
            !admission.terminal,
            "first canonical carrier bidi frame must be admission, got {admission:?}"
        );
        assert_eq!(
            admission
                .admission_receipt
                .expect("admission frame carries receipt")
                .state,
            axon_sdk::invocation::InvocationState::Admitted.to_wire_i32()
        );

        let reply = tokio::time::timeout(Duration::from_secs(3), rx.recv())
            .await
            .expect("upload completion within 3s")
            .expect("upload completion produced");
        let result = match reply.payload {
            Some(UpPayload::DispatchResult(r)) => r,
            other => panic!("expected completion DispatchResult, got: {other:?}"),
        };
        assert_eq!(result.call_id, 77);
        assert!(
            result.terminal,
            "completion is the canonical terminal result: {result:?}"
        );
        let completion: serde_json::Value =
            serde_json::from_slice(&result.payload).expect("upload completion payload");
        assert_eq!(completion["type"], "complete", "{completion}");
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
        assert_eq!(
            receipt.payload, result.payload,
            "terminal carrier payload must be the signed receipt payload"
        );
        assert!(
            lifecycle_cancellations.contains_invocation_id(&receipt.invocation_id),
            "canonical carrier bidi lifecycle must remain registered for invocation.cancel"
        );
            assert_eq!(
                std::fs::read(&target).expect("uploaded file exists"),
                bytes,
                "payload bytes must land on the device-side filesystem"
            );
        });
        runtime
            .block_on(test)
            .expect("canonical bidi carrier test task");
    }

    /// step-3b open errors are frames, not transport errors: an
    /// unwired ability on a canonical session replies a typed proto failure.
    #[tokio::test]
    async fn canonical_carrier_bidi_open_of_unwired_ability_fails_proto_on_canonical_session() {
        let rt = executable_runtime();
        let disp = LocalAxonSessionDispatcher::new(Default::default()).with_local_runtime(rt);
        let (tx, mut rx) = mpsc::channel::<InvokeBidiUp>(4);
        let session_tx = start_test_carrier(&disp, tx);
        session_tx.set_negotiated_contract(
            crate::daemon::invocation::bidi::state::presence::CANONICAL_SESSION_CARRIER_VERSION,
        );

        disp.handle_down(
            canonical_carrier_explicit_test_bidi_open(9, "test.echo", b"{}".to_vec()),
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
    async fn canonical_carrier_stream_dispatch_of_unpublished_ability_fails_proto_without_timeout()
    {
        let rt = executable_runtime();
        let disp = LocalAxonSessionDispatcher::new(Default::default()).with_local_runtime(rt);
        let (tx, mut rx) = mpsc::channel::<InvokeBidiUp>(4);
        let session_tx = start_test_carrier(&disp, tx);
        session_tx.set_negotiated_contract(
            crate::daemon::invocation::bidi::state::presence::CANONICAL_SESSION_CARRIER_VERSION,
        );

        disp.handle_down(
            canonical_carrier_explicit_test_call_with_mode(
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
    async fn canonical_carrier_dispatch_executes_and_replies_proto_on_canonical_session() {
        let rt = executable_runtime();
        register_test_rpc(
            &rt,
            "test.echo",
            axon_sdk::invocation::make_ability(|ctx| async move { Ok(ctx.payload.clone()) }),
        )
        .await;
        let disp =
            LocalAxonSessionDispatcher::new(Default::default()).with_local_runtime(Arc::clone(&rt));
        let (tx, mut rx) = mpsc::channel::<InvokeBidiUp>(4);
        let session_tx = start_test_carrier(&disp, tx);
        session_tx.set_negotiated_contract(
            crate::daemon::invocation::bidi::state::presence::CANONICAL_SESSION_CARRIER_VERSION,
        );

        disp.handle_down(
            canonical_carrier_explicit_test_call(
                7,
                "test.echo",
                br#"{"hello":"canonical"}"#.to_vec(),
            ),
            &session_tx,
        )
        .await
        .expect("canonical carrier dispatch succeeds");

        let reply = tokio::time::timeout(Duration::from_secs(3), rx.recv())
            .await
            .expect("reply within 3s")
            .expect("reply produced");
        let Some(UpPayload::DispatchResult(result)) = reply.payload else {
            panic!(
                "canonical session must reply DispatchResult, got {:?}",
                reply.payload
            );
        };
        assert_eq!(result.call_id, 7);
        assert!(result.terminal);
        assert!(
            result.failure.is_none(),
            "canonical carrier dispatch failed: {:?}",
            result.failure
        );
        assert_eq!(result.payload, br#"{"hello":"canonical"}"#);
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
    async fn canonical_carrier_dispatch_preserves_non_default_descriptor_version() {
        let rt = executable_runtime();
        register_test_ability_with_options(
            &rt,
            "test.echo",
            axon_sdk::invocation::make_ability(|ctx| async move { Ok(ctx.payload.clone()) }),
            proof_bound_rpc_options_with_version(TEST_DESCRIPTOR_VERSION_V2),
        )
        .await;
        let disp =
            LocalAxonSessionDispatcher::new(Default::default()).with_local_runtime(Arc::clone(&rt));
        let (tx, mut rx) = mpsc::channel::<InvokeBidiUp>(4);
        let session_tx = start_test_carrier(&disp, tx);
        session_tx.set_negotiated_contract(
            crate::daemon::invocation::bidi::state::presence::CANONICAL_SESSION_CARRIER_VERSION,
        );
        let signed_ability =
            crate::daemon::axon_bridge::descriptor_ref::ability_descriptor_ref_for_wire(
                TEST_DEVICE_URA,
                "test.echo",
                &descriptor_binding_for_version(TEST_DESCRIPTOR_VERSION_V2),
            )
            .expect("versioned canonical carrier descriptor ref");

        disp.handle_down(
            canonical_carrier_call_signed_as(
                19,
                "test.echo",
                &signed_ability,
                br#"{"hello":"v2"}"#.to_vec(),
            ),
            &session_tx,
        )
        .await
        .expect("canonical carrier dispatch succeeds with non-default descriptor version");

        let reply = tokio::time::timeout(Duration::from_secs(3), rx.recv())
            .await
            .expect("reply within 3s")
            .expect("reply produced");
        let Some(UpPayload::DispatchResult(result)) = reply.payload else {
            panic!(
                "canonical session must reply DispatchResult, got {:?}",
                reply.payload
            );
        };
        assert_eq!(result.call_id, 19);
        assert!(result.terminal);
        assert!(
            result.failure.is_none(),
            "canonical carrier dispatch failed: {:?}",
            result.failure
        );
        assert_eq!(result.payload, br#"{"hello":"v2"}"#);
    }

    #[tokio::test]
    async fn canonical_carrier_stream_terminal_frame_carries_receipt() {
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
        let disp =
            LocalAxonSessionDispatcher::new(Default::default()).with_local_runtime(Arc::clone(&rt));
        let (tx, mut rx) = mpsc::channel::<InvokeBidiUp>(8);
        let session_tx = start_test_carrier(&disp, tx);
        session_tx.set_negotiated_contract(
            crate::daemon::invocation::bidi::state::presence::CANONICAL_SESSION_CARRIER_VERSION,
        );

        disp.handle_down(
            canonical_carrier_explicit_test_call_with_mode(
                18,
                "screen.subscribe",
                b"{}".to_vec(),
                axon_sdk::invocation::CallMode::Stream,
            ),
            &session_tx,
        )
        .await
        .expect("canonical carrier stream dispatch opens and forwards asynchronously");

        let admission = tokio::time::timeout(Duration::from_secs(3), rx.recv())
            .await
            .expect("admission reply within 3s")
            .expect("admission reply produced");
        let admission = match admission.payload {
            Some(UpPayload::DispatchResult(result)) => result,
            other => panic!("expected canonical carrier admission result, got: {other:?}"),
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
            other => panic!("expected canonical carrier progress result, got: {other:?}"),
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
            other => panic!("expected canonical carrier terminal result, got: {other:?}"),
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
            .expect("canonical carrier terminal stream result must carry receipt");
        assert_eq!(
            receipt.state,
            axon_sdk::invocation::InvocationState::Completed.to_wire_i32()
        );
        assert!(
            disp.lifecycle_cancellations
                .contains_invocation_id(&receipt.invocation_id),
            "canonical carrier stream lifecycle must remain registered for invocation.cancel"
        );
    }

    #[tokio::test]
    async fn canonical_carrier_stream_cancel_finalizes_and_returns_terminal_receipt() {
        use axon_sdk::invocation::make_ability;

        let rt = executable_runtime();
        register_test_ability_with_options(
            &rt,
            "camera.subscribe",
            make_ability(|ctx| async move {
                ctx.emit_progress(br#"{"frame":1}"#.to_vec(), "application/json")
                    .await?;
                ctx.wait_for_cancel().await;
                Ok(Vec::new())
            }),
            proof_bound_stream_options(),
        )
        .await;
        let disp =
            LocalAxonSessionDispatcher::new(Default::default()).with_local_runtime(Arc::clone(&rt));
        let (tx, mut rx) = mpsc::channel::<InvokeBidiUp>(8);
        let session_tx = start_test_carrier(&disp, tx);

        disp.handle_down(
            canonical_carrier_explicit_test_call_with_mode(
                31,
                "camera.subscribe",
                b"{}".to_vec(),
                axon_sdk::invocation::CallMode::Stream,
            ),
            &session_tx,
        )
        .await
        .expect("camera stream opens");

        let admission = rx.recv().await.expect("admission frame");
        assert!(matches!(
            admission.payload,
            Some(UpPayload::DispatchResult(ref result)) if !result.terminal
        ));
        let progress = rx.recv().await.expect("progress frame");
        assert!(matches!(
            progress.payload,
            Some(UpPayload::DispatchResult(ref result)) if !result.terminal && !result.payload.is_empty()
        ));

        disp.handle_down(
            session_frame(SessionDispatch::StreamCancel {
                call_id: 31,
                reason: "browser camera window closed".to_string(),
            }),
            &session_tx,
        )
        .await
        .expect("carrier stream cancellation is admitted");

        let terminal = tokio::time::timeout(Duration::from_secs(3), rx.recv())
            .await
            .expect("cancelled stream finalizes promptly")
            .expect("terminal frame exists");
        let Some(UpPayload::DispatchResult(terminal)) = terminal.payload else {
            panic!("cancelled stream must return canonical DispatchResult terminal");
        };
        assert!(terminal.terminal);
        let receipt = terminal
            .terminal_receipt
            .expect("cancelled stream carries a signed terminal receipt");
        assert_eq!(
            receipt.state,
            axon_sdk::invocation::InvocationState::Cancelled.to_wire_i32()
        );
    }

    #[tokio::test]
    async fn canonical_carrier_stream_descriptor_selects_stream_even_when_rpc_is_supported() {
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
        let disp =
            LocalAxonSessionDispatcher::new(Default::default()).with_local_runtime(Arc::clone(&rt));
        let (tx, mut rx) = mpsc::channel::<InvokeBidiUp>(8);
        let session_tx = start_test_carrier(&disp, tx);
        session_tx.set_negotiated_contract(
            crate::daemon::invocation::bidi::state::presence::CANONICAL_SESSION_CARRIER_VERSION,
        );

        disp.handle_down(
            canonical_carrier_explicit_test_call_with_mode(
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
            other => panic!("expected canonical carrier stream admission result, got: {other:?}"),
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
            other => panic!("expected canonical carrier stream progress result, got: {other:?}"),
        };
        assert_eq!(progress.call_id, 21);
        assert!(!progress.terminal);
        let payload: serde_json::Value =
            serde_json::from_slice(&progress.payload).expect("progress payload is JSON");
        assert_eq!(payload["kind"], "progress");
    }

    #[tokio::test]
    async fn canonical_rpc_call_mode_is_independent_from_stream_admission_action() {
        use axon_sdk::invocation::{make_ability, AbilityCallModes, AbilityOptions, CallMode};

        let rt = executable_runtime();
        let options = AbilityOptions::default()
            .with_modes(AbilityCallModes::RPC)
            .with_mode_descriptor_proof(
                CallMode::Rpc,
                TEST_DESCRIPTOR_VERSION,
                crate::daemon::ability::descriptors::AdmissionAction::Stream.as_str(),
                TEST_DESCRIPTOR_HASH,
                TEST_SCHEMA_HASH,
                TEST_IMPL_HASH,
            );
        register_test_ability_with_options(
            &rt,
            "terminal.create",
            make_ability(|_| async { Ok(br#"{"created":true}"#.to_vec()) }),
            options,
        )
        .await;

        let descriptor_ref = explicit_test_descriptor_ref_with_action(
            TEST_DEVICE_URA,
            "terminal.create",
            TEST_DESCRIPTOR_VERSION,
            crate::daemon::ability::descriptors::AdmissionAction::Stream.as_str(),
        );
        let frame = canonical_carrier_call_signed_as_with_mode(
            22,
            "terminal.create",
            &descriptor_ref,
            b"{}".to_vec(),
            CallMode::Rpc,
        );
        let dispatcher =
            LocalAxonSessionDispatcher::new(Default::default()).with_local_runtime(Arc::clone(&rt));
        let (tx, mut rx) = mpsc::channel::<InvokeBidiUp>(4);
        let outbound = start_test_carrier(&dispatcher, tx);
        outbound.set_negotiated_contract(
            crate::daemon::invocation::bidi::state::presence::CANONICAL_SESSION_CARRIER_VERSION,
        );

        dispatcher
            .handle_down(frame, &outbound)
            .await
            .expect("RPC dispatch accepted");

        let result = tokio::time::timeout(Duration::from_secs(3), rx.recv())
            .await
            .expect("RPC result within timeout")
            .expect("RPC result produced");
        let Some(UpPayload::DispatchResult(result)) = result.payload else {
            panic!("expected canonical DispatchResult");
        };
        assert!(result.terminal, "explicit RPC mode must stay unary");
        assert_eq!(result.payload, br#"{"created":true}"#);
        assert!(result.failure.is_none());
        assert!(result.admission_receipt.is_some());
        assert!(result.terminal_receipt.is_some());
    }

    #[tokio::test]
    async fn malformed_dispatch_json_returns_error() {
        let disp = LocalAxonSessionDispatcher::new(Default::default());
        let (tx, _rx) = mpsc::channel::<InvokeBidiUp>(4);
        let session_tx = start_test_carrier(&disp, tx);

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
    // This test uses the canonical canonical carrier DispatchCall/DispatchResult
    // path. The retired JSON Dispatch frame must not reappear merely to keep
    // a device-mode test alive.

    fn build_real_daemon_registry_with_runtime(
        local_runtime: Option<Arc<axon_sdk::invocation::LocalRuntime>>,
    ) -> crate::daemon::ability::catalog::BuiltAbilityRegistry {
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
    }

    #[tokio::test]
    async fn device_mode_dispatcher_executes_fs_read_through_baseline_locomotion_registry() {
        let _home = crate::cli::commands::test_support::HomeGuard::new();
        crate::daemon::persistence::config::save_credentials(
            &crate::daemon::persistence::config::Credentials {
                node_id: "d1".to_string(),
                realm: "t".to_string(),
                credential_token: "token".to_string(),
                hub_endpoint: "https://hub.example:50443".to_string(),
                join_receipt_hash: Some("join-hash".to_string()),
                username: Some("alice".to_string()),
                user_id: Some("alice".to_string()),
                ..Default::default()
            },
        )
        .expect("test Device identity");
        let tmp = tempfile::tempdir().expect("tempdir");
        let target = tmp.path().join("hello.txt");
        std::fs::write(&target, "device-B-bytes-from-real-fs-read").expect("seed temp file");

        let rt = executable_runtime();
        let _registry = build_real_daemon_registry_with_runtime(Some(Arc::clone(&rt)));
        let disp =
            LocalAxonSessionDispatcher::new(Default::default()).with_local_runtime(Arc::clone(&rt));
        let (tx, mut rx) = mpsc::channel::<InvokeBidiUp>(4);
        let session_tx = start_test_carrier(&disp, tx);

        let args = serde_json::json!({
            "resource_ref": crate::daemon::resources::files::FilesystemResourceProvider::for_device(
                crate::core::ura::device_ura("t", "d1"),
            )
            .expect("test filesystem Device authority")
            .resource_ref_for_local_path(
                &target,
                crate::daemon::resources::files::FilesystemResourceCapability::Read,
            )
            .expect("local fs ResourceRef"),
            "encoding": "utf8",
        });
        let locomotion_callee = crate::core::ura::device_agent_ura(
            "t",
            "d1",
            crate::daemon::ability::names::device_control::LOCOMOTION_SYSTEM_AGENT_ID,
        );
        let frame = canonical_carrier_call_signed_as_with_mode_for_target(
            42,
            "fs.read",
            "fs.read",
            serde_json::to_vec(&args).expect("encode args"),
            axon_sdk::invocation::CallMode::Rpc,
            &locomotion_callee,
            TEST_DEVICE_URA,
        );
        session_tx.set_negotiated_contract(
            crate::daemon::invocation::bidi::state::presence::CANONICAL_SESSION_CARRIER_VERSION,
        );

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
    fn pty_output_projection_preserves_lifecycle_and_uses_native_binary_stdout() {
        let attached = LocalAxonSessionDispatcher::map_remote_pty_output(
            17,
            &json!({"type": "attached", "attachment_id": "a", "epoch": 2}),
        )
        .expect("attached projection")
        .expect("attached frame");
        let attached_json: Value =
            serde_json::from_slice(&attached.payload).expect("attached JSON payload");
        assert_eq!(attached_json["type"], "attached");
        assert_eq!(attached_json["epoch"], 2);
        assert_eq!(
            attached.content_type,
            crate::daemon::ability::wire::CONTROL_CONTENT_TYPE
        );

        let stdout = LocalAxonSessionDispatcher::map_native_bidi_data(
            17,
            vec![0, 1, 2],
            "application/octet-stream".to_string(),
        );
        assert_eq!(stdout.payload, vec![0, 1, 2]);
        assert_eq!(stdout.content_type, "application/octet-stream");

        let exit = LocalAxonSessionDispatcher::map_remote_pty_output(
            17,
            &json!({"type": "exit", "status": 0}),
        )
        .expect("exit projection")
        .expect("exit frame");
        let exit_json: Value = serde_json::from_slice(&exit.payload).expect("exit JSON payload");
        assert_eq!(exit_json["type"], "exit");
        assert_eq!(exit_json["status"], 0);
    }

    #[test]
    fn tunnel_wire_profile_projects_control_frames_without_file_transfer_fallback() {
        let registry = crate::daemon::ability::wire::AbilityWireRegistry::core();
        let ability =
            crate::daemon::ability::builtins::device_control::net_tunnel::ABILITY_NET_TUNNEL;

        for value in [
            json!({"type": "connected", "connection_id": "c1"}),
            json!({"type": "listener_ready", "address": "127.0.0.1:1"}),
            json!({"type": "accepted", "connection_id": "c2"}),
            json!({"type": "half_close", "connection_id": "c1", "direction": "read"}),
        ] {
            let mapped = LocalAxonSessionDispatcher::map_remote_bidi_output_with(
                &registry, 18, ability, &value,
            )
            .expect("tunnel control projection")
            .expect("tunnel control frame");
            assert_eq!(mapped.call_id, 18);
            assert_eq!(
                mapped.content_type,
                crate::daemon::ability::wire::CONTROL_CONTENT_TYPE
            );
            assert_eq!(mapped.disposition, BidiOutputDisposition::Data);
            assert!(mapped.failure.is_none());
            assert_eq!(
                serde_json::from_slice::<Value>(&mapped.payload).expect("control JSON"),
                value
            );
        }

        let complete = LocalAxonSessionDispatcher::map_remote_bidi_output_with(
            &registry,
            18,
            ability,
            &json!({"type": "complete", "connection_id": "c1", "bytes": 4}),
        )
        .expect("tunnel completion projection")
        .expect("tunnel completion frame");
        assert_eq!(complete.disposition, BidiOutputDisposition::Completion);

        let failure = LocalAxonSessionDispatcher::map_remote_bidi_output_with(
            &registry,
            18,
            ability,
            &json!({"type": "error", "code": "IDLE_TIMEOUT", "message": "IDLE_TIMEOUT"}),
        )
        .expect("tunnel failure projection")
        .expect("tunnel failure frame");
        assert_eq!(failure.disposition, BidiOutputDisposition::Failure);
        assert_eq!(failure.failure.expect("typed failure").code, "IDLE_TIMEOUT");
    }

    #[test]
    fn unregistered_bidi_wire_profile_is_rejected_explicitly() {
        let registry = crate::daemon::ability::wire::AbilityWireRegistry::core();
        let error = LocalAxonSessionDispatcher::map_remote_bidi_output_with(
            &registry,
            19,
            "unknown.bidi",
            &json!({"type": "complete"}),
        )
        .expect_err("unknown wire profiles must not inherit file-transfer semantics");
        assert!(
            error.to_string().contains("has no registered wire profile"),
            "{error}"
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
        LocalAxonSessionDispatcher::new(Default::default()).with_ability_wire_registry(Arc::new(
            crate::daemon::ability::wire::AbilityWireRegistry::for_test_plugin_bidi([(
                "remote_desktop.attach".to_string(),
                crate::daemon::ability::wire::AbilityBidiWireKind::JsonFrames,
            )]),
        ))
    }
}
