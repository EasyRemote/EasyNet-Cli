// EasyNet Daemon — InvokeStream Dispatcher
// =========================================
//
// File: src/daemon/invocation/stream_dispatcher.rs
// Description: Owns every `InvokeStream` routing decision the daemon
//              makes after transport policy (commit-plan-2 Axis E / E2):
//
//                * `federation.subscribe_directory_v2`  — DirectoryEvent
//                  pump with §2.3 heartbeat cadence
//                * everything else — RFC-005 resolve-first local dispatch
//                  through Axon `LocalRuntime`
//
//              The dispatcher is a pure consumer of the dependency
//              planes plus the `TargetGate`; it never sees the tonic
//              service. `DaemonInvocationService::invoke_stream` stays
//              a thin admission + route shell.
//
// Author: Silan Hu <silan.hu@u.nus.edu>
// Copyright (c) 2026 EasyNet. All rights reserved.

use std::pin::Pin;
use std::sync::{Arc, Weak};
use std::task::{Context, Poll};
use std::time::Duration;

use axon_sdk::invocation::{CallMode, KeyResolver, StreamingInvocationHandle};
use futures::Stream;
use tokio::sync::{mpsc, watch};
use tokio_stream::wrappers::ReceiverStream;
use tonic::{Response, Status};

use axon_sdk::pb::axon::v1::{Error, InvokeServerStreamRequest, InvokeStreamChunk, ResponseHeader};

use crate::daemon::ability::dispatch::StreamSource;
use crate::daemon::invocation::admission::admission_facade::AdmissionFacade;
use crate::daemon::invocation::admission::hosted_agent_delegation::HostedAgentDelegationIssuer;
use crate::daemon::invocation::admission::target_gate::{
    route_negative_status, route_profile_blocked_status, signed_envelope_for_selected_route,
    TargetGate,
};
use crate::daemon::invocation::bidi::session_wire::{
    build_carrier_v1_dispatch_frame, require_canonical_dispatch_session,
};
use crate::daemon::invocation::bidi::state::pending_dispatch::{
    DispatchResult, DispatchStreamEvent, PendingStreamHandle,
};
use crate::daemon::invocation::bidi::state::presence::PresenceRegistry;
use crate::daemon::invocation::dispatch::cancellation::RegisteredInvocationLifecycle;
use crate::daemon::invocation::dispatch::daemon_invocation_service::DaemonStreamRoute;
use crate::daemon::invocation::dispatch::deps::{DirectoryPlane, RuntimePlane, SessionPlane};
use crate::daemon::invocation::dispatch::descriptor_binding::RuntimeBoundAbility;
use crate::daemon::invocation::dispatch::federation_wrappers;
use crate::daemon::invocation::dispatch::forwarded_finalization::{
    ensure_forwarded_receipt_signer_key, ForwardedFinalizationVerifier, ForwardedInvocationBinding,
};
use crate::daemon::invocation::dispatch::invocation_wire::{
    callee_ura_from_envelope, function_name_from_invocation_target, status_from_axon_invoke_error,
    BoxedDownStream, FEDERATION_RESULT_CONTENT_TYPE,
};
use crate::daemon::invocation::dispatch::remote_failure::status_from_remote_failure;
use crate::daemon::invocation::dispatch::unary_dispatcher::require_complete_signed_remote_request;
use crate::daemon::invocation::routing::route_resolver::{
    CanonicalRouteDispatch, CanonicalRouteSelection, SelectedInvokeRoute,
};

/// `InvokeStream` routing surface. Cheap per-call construction: both
/// planes and the gate are `Arc`-shaped.
pub(crate) struct StreamDispatcher {
    admission: AdmissionFacade,
    directory: DirectoryPlane,
    sessions: SessionPlane,
    runtime: RuntimePlane,
    gate: TargetGate,
    daemon_route_lifecycle: Weak<()>,
}

impl StreamDispatcher {
    pub(crate) fn new(
        admission: AdmissionFacade,
        directory: DirectoryPlane,
        sessions: SessionPlane,
        runtime: RuntimePlane,
        gate: TargetGate,
        daemon_route_lifecycle: Weak<()>,
    ) -> Self {
        Self {
            admission,
            directory,
            sessions,
            runtime,
            gate,
            daemon_route_lifecycle,
        }
    }

    pub(crate) fn daemon_route_provider(&self) -> DaemonStreamRouteProvider {
        DaemonStreamRouteProvider::new(
            Arc::downgrade(&self.directory.presence),
            self.directory.subscribe_v2_heartbeat_interval_ms,
            self.daemon_route_lifecycle.clone(),
        )
    }

    pub(crate) async fn dispatch_daemon_route_runtime(
        &self,
        route: DaemonStreamRoute,
        request: &InvokeServerStreamRequest,
    ) -> Result<Response<BoxedDownStream<InvokeStreamChunk>>, Status> {
        let Some(runtime) = self.runtime.local_runtime() else {
            return Err(Status::failed_precondition(format!(
                "InvokeStream: daemon stream route `{}` cannot run because Axon LocalRuntime \
                 is not wired at boot",
                route.name()
            )));
        };
        let local_system_ingress = self
            .admission
            .accepts_local_system_envelope(request.envelope.as_ref());
        let (handle, lifecycle) =
            crate::daemon::invocation::dispatch::daemon_route_runtime::DaemonRouteRuntimeAdapter::new(
                runtime,
                self.runtime.cancellations.clone(),
                self.admission.clone(),
                self.runtime.product_policy()?,
            )
            .open_stream(route, request, local_system_ingress)
            .await?;
        let initial_sequence = daemon_route_initial_sequence(route, request)?;
        project_local_runtime_stream(handle, route.name(), initial_sequence, lifecycle).await
    }

    /// RFC-005 resolve-first dispatch for every other server-stream
    /// ability: prove the route, then send it either to this daemon's
    /// `LocalRuntime` or to the resolver-selected execution host's
    /// presence session.
    pub(crate) async fn dispatch_selected_route(
        &self,
        request: &InvokeServerStreamRequest,
    ) -> Result<Response<BoxedDownStream<InvokeStreamChunk>>, Status> {
        let selection = match self.resolve_stream_route(request).await {
            Ok(selection) => selection,
            Err(status) => {
                if let Some(handle) = self.sessions.escalation.as_ref() {
                    return self
                        .escalate_canonical_stream(handle, request, status)
                        .await;
                }
                return Err(status);
            }
        };
        let call_mode = selection.call_mode();
        let selected_route = match selection.into_dispatch() {
            CanonicalRouteDispatch::Local(route) => route,
            CanonicalRouteDispatch::Peer(route) => {
                return Err(Status::unimplemented(format!(
                    "InvokeStream selected canonical peer route to hub `{}` for `{}`, but the \
                     generic cross-realm server-stream carrier is unsupported",
                    route.hub_ura, route.query_name,
                )));
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
            self.dispatch_local_resolved_route(request, selected_route, call_mode)
                .await
        } else {
            self.dispatch_remote_selected_route(request, selected_route, call_mode)
                .await
        }
    }

    async fn escalate_canonical_stream(
        &self,
        handle: &Arc<crate::daemon::invocation::bidi::session_escalation::SessionEscalationHandle>,
        request: &InvokeServerStreamRequest,
        local_route_failure: Status,
    ) -> Result<Response<BoxedDownStream<InvokeStreamChunk>>, Status> {
        if matches!(
            local_route_failure.code(),
            tonic::Code::InvalidArgument | tonic::Code::PermissionDenied
        ) {
            return Err(local_route_failure);
        }
        let forwarded_request = stream_request_as_invoke_request(request);
        require_complete_signed_remote_request(&forwarded_request)?;
        let forwarded_binding = ForwardedInvocationBinding::from_request(&forwarded_request)?;
        let receipt_resolver = self.admission.receipt_key_resolver();
        match handle.escalate_stream(request.clone()).await {
            Ok(stream) => project_forwarded_remote_stream(
                RemoteStreamEventSource::Session(stream),
                forwarded_binding,
                receipt_resolver,
                request_timeout(request),
            )
            .await,
            Err(crate::daemon::invocation::bidi::session_wire::SessionRequestError::TargetOffline) => {
                Err(Status::failed_precondition("remote InvokeStream target is offline"))
            }
            Err(crate::daemon::invocation::bidi::session_wire::SessionRequestError::PermissionDenied {
                reason,
            }) => Err(Status::permission_denied(reason)),
            Err(crate::daemon::invocation::bidi::session_wire::SessionRequestError::UpstreamFailure {
                reason,
            }) => Err(Status::unavailable(format!(
                "remote InvokeStream session escalation failed: {reason}"
            ))),
            Err(crate::daemon::invocation::bidi::session_wire::SessionRequestError::UpstreamTimeout) => {
                Err(Status::deadline_exceeded(
                    "remote InvokeStream session escalation timed out",
                ))
            }
        }
    }

    async fn dispatch_local_resolved_route(
        &self,
        request: &InvokeServerStreamRequest,
        selected_route: SelectedInvokeRoute,
        call_mode: CallMode,
    ) -> Result<Response<BoxedDownStream<InvokeStreamChunk>>, Status> {
        let ability =
            function_name_from_invocation_target("InvokeStream", request.target.as_ref())?;
        let Some(runtime) = self.runtime.local_runtime() else {
            return Err(Status::failed_precondition(format!(
                "InvokeStream: ability `{ability}` cannot run because Axon LocalRuntime \
                 is not wired at boot"
            )));
        };
        let bound_ability = RuntimeBoundAbility::from_selected_route(
            "InvokeStream",
            &runtime,
            self.directory.local_ability_catalog.as_deref(),
            &selected_route,
            call_mode,
        )
        .await?;
        let selected_descriptor_ref = bound_ability
            .descriptor_ref_for_mode(
                "InvokeStream",
                &selected_route.callee_ura,
                call_mode,
                Some(&selected_route.route_ura),
            )?
            .into_descriptor_ref();
        bound_ability.require_wire_target_matches(
            "InvokeStream",
            &selected_route.callee_ura,
            ability,
            &selected_route.route_ura,
        )?;
        let local_system_ingress = self
            .admission
            .accepts_local_system_envelope(request.envelope.as_ref());
        let wire = match request.envelope.clone() {
            Some(envelope) if local_system_ingress => {
                let metadata = HostedAgentDelegationIssuer::materialize_request_metadata(
                    &request.metadata,
                    &envelope,
                    true,
                    ability,
                )?;
                crate::daemon::axon_bridge::dispatch_shim::local_system_from_wire_parts(
                    envelope,
                    selected_descriptor_ref,
                    request.arguments.clone(),
                    metadata,
                )
            }
            Some(envelope) => {
                let metadata = HostedAgentDelegationIssuer::materialize_request_metadata(
                    &request.metadata,
                    &envelope,
                    false,
                    ability,
                )?;
                let signed_descriptor_ref = bound_ability
                    .signed_descriptor_ref_from_target(
                        "InvokeStream",
                        &selected_route.callee_ura,
                        call_mode,
                        request.target.as_ref(),
                    )?
                    .into_descriptor_ref();
                crate::daemon::axon_bridge::dispatch_shim::external_signed_from_wire_parts(
                    envelope,
                    signed_descriptor_ref,
                    request.arguments.clone(),
                    metadata,
                )
            }
            None => Err(Box::new(axon_sdk::invocation::AxonError::invalid_argument(
                "InvokeStream request missing envelope",
            ))),
        }
        .map_err(|err| status_from_axon_invoke_error("InvokeStream", ability, *err))?;
        let lifecycle_envelope = wire.envelope.clone();
        let product_admission = self.runtime.stage_product_admission(
            &self.admission,
            &wire,
            ability,
            CallMode::Stream,
        )?;
        let handle =
            crate::daemon::axon_bridge::dispatch_shim::open_stream_admitted(&runtime, wire)
                .await
                .map_err(|err| status_from_axon_invoke_error("InvokeStream", ability, err))?;
        let lifecycle = match RegisteredInvocationLifecycle::register(
            self.runtime.cancellations.clone(),
            &lifecycle_envelope,
            handle.handle().clone(),
        ) {
            Ok(lifecycle) => lifecycle,
            Err(error) => {
                let _ = handle.cancel("stream lifecycle registration failed").await;
                let _ = handle.finalized().await;
                return Err(Status::failed_precondition(format!(
                    "InvokeStream `{ability}` lifecycle registration failed: {error}"
                )));
            }
        };
        if let Err(error) = product_admission.commit() {
            let _ = lifecycle
                .cancel_and_finalize("stream product admission commit failed")
                .await;
            return Err(error);
        }
        project_local_runtime_stream(handle, ability, 0, lifecycle).await
    }

    async fn dispatch_remote_selected_route(
        &self,
        request: &InvokeServerStreamRequest,
        selected_route: SelectedInvokeRoute,
        call_mode: CallMode,
    ) -> Result<Response<BoxedDownStream<InvokeStreamChunk>>, Status> {
        let ability =
            function_name_from_invocation_target("InvokeStream", request.target.as_ref())?
                .to_string();
        let Some(envelope) = request.envelope.clone() else {
            return Err(Status::invalid_argument(format!(
                "InvokeStream: remote-hosted ability `{ability}` requires the seven-tuple \
                 envelope on the canonical Invocation face",
            )));
        };
        signed_envelope_for_selected_route(
            envelope,
            &selected_route,
            request.target.as_ref(),
            &request.arguments,
        )?;
        let pending = self.sessions.pending_stream.as_ref().ok_or_else(|| {
            Status::failed_precondition(format!(
                "InvokeStream {}: daemon was constructed without a \
                 PendingStreamDispatchMap; boot must call with_pending_stream(...) \
                 to enable remote stream dispatch",
                selected_route.dispatch_name
            ))
        })?;
        let session = require_canonical_dispatch_session(
            &self.directory.presence,
            &selected_route.execution_host_ura,
            &selected_route.route_ura,
            "InvokeStream",
        )?;
        let session_id = session.session_id;
        let sender = session.sender;
        let carrier_version = session.contract_version;

        let handle = pending.register_pending_for(&selected_route.execution_host_ura);
        let call_id = handle.call_id();
        let forwarded_request = axon_sdk::pb::axon::v1::InvokeRequest {
            envelope: request.envelope.clone(),
            target: request.target.clone(),
            arguments: request.arguments.clone(),
            content_type: request.content_type.clone(),
            content_envelope: request.content_envelope.clone(),
            metadata: request.metadata.clone(),
            payload_ref: request.payload_ref.clone(),
            ..axon_sdk::pb::axon::v1::InvokeRequest::default()
        };
        let forwarded_binding = ForwardedInvocationBinding::from_request(&forwarded_request)?;
        ensure_forwarded_receipt_signer_key(
            self.sessions.device_trust_sync.as_ref(),
            &selected_route.execution_host_ura,
            "InvokeStream",
        )
        .await?;
        let receipt_resolver = self.admission.receipt_key_resolver();
        let dispatch_frame = build_carrier_v1_dispatch_frame(
            call_id,
            forwarded_request,
            matches!(call_mode, CallMode::Bidi),
        );
        match sender.try_send(Ok(dispatch_frame)) {
            Ok(()) => {}
            Err(tokio::sync::mpsc::error::TrySendError::Full(_)) => {
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
            kind = invoke_stream_remote_selected_route_dispatch,
            ability = ability.as_str(),
            dispatch_ability = selected_route.ability_ura.as_str(),
            callee_ura = selected_route.callee_ura.as_str(),
            execution_host_ura = selected_route.execution_host_ura.as_str(),
            route_ura = selected_route.route_ura.as_str(),
            carrier_version = carrier_version,
            call_id = call_id,
        );

        project_forwarded_remote_stream(
            RemoteStreamEventSource::Presence(handle),
            forwarded_binding,
            receipt_resolver,
            request_timeout(request),
        )
        .await
    }
}

enum RemoteStreamEventSource {
    Presence(PendingStreamHandle),
    Session(crate::daemon::invocation::bidi::session_escalation::EscalatedStreamHandle),
}

impl RemoteStreamEventSource {
    async fn recv(&mut self) -> Option<DispatchStreamEvent> {
        match self {
            Self::Presence(handle) => handle.recv().await,
            Self::Session(handle) => handle.recv().await,
        }
    }
}

async fn project_forwarded_remote_stream(
    mut source: RemoteStreamEventSource,
    forwarded_binding: ForwardedInvocationBinding,
    receipt_resolver: Arc<dyn KeyResolver>,
    timeout: Option<Duration>,
) -> Result<Response<BoxedDownStream<InvokeStreamChunk>>, Status> {
    let (tx, rx) = mpsc::channel::<Result<InvokeStreamChunk, Status>>(16);
    tokio::spawn(async move {
        let mut sequence = 0_u64;
        let mut canonical_invocation_id = None::<String>;
        let mut finalization =
            ForwardedFinalizationVerifier::new(forwarded_binding, receipt_resolver);
        let deadline = timeout.map(|duration| tokio::time::Instant::now() + duration);
        loop {
            let event = match deadline {
                Some(deadline) => match tokio::time::timeout_at(deadline, source.recv()).await {
                    Ok(event) => event,
                    Err(_) => {
                        let _ = tx
                            .send(Err(Status::deadline_exceeded(
                                "REMOTE_STREAM_TERMINAL_TIMEOUT: remote stream did not produce a terminal event before the invocation deadline",
                            )))
                            .await;
                        break;
                    }
                },
                None => source.recv().await,
            };
            let Some(event) = event else {
                let _ = tx
                    .send(Err(Status::failed_precondition(
                        "CANONICAL_TERMINAL_REQUIRED: remote stream source closed without a terminal event",
                    )))
                    .await;
                break;
            };
            match event {
                DispatchStreamEvent::Admission(receipt) => {
                    let receipt = *receipt;
                    if let Err(status) = finalization.admit(receipt.clone()) {
                        let _ = tx.send(Err(status)).await;
                        break;
                    }
                    canonical_invocation_id = Some(receipt.invocation_id.clone());
                    let chunk = remote_stream_chunk(RemoteStreamChunkParts {
                        invocation_id: receipt.invocation_id.clone(),
                        state: axon_sdk::invocation::InvocationState::Running,
                        payload: Vec::new(),
                        sequence,
                        terminal: false,
                        admission_receipt: Some(receipt),
                        terminal_receipt: None,
                        error: None,
                    });
                    sequence = sequence.saturating_add(1);
                    if tx.send(Ok(chunk)).await.is_err() {
                        break;
                    }
                }
                DispatchStreamEvent::Chunk(payload) => {
                    if let Err(status) = finalization.observe_data() {
                        let _ = tx.send(Err(status)).await;
                        break;
                    }
                    let Some(invocation_id) = canonical_invocation_id.clone() else {
                        let _ = tx
                            .send(Err(Status::failed_precondition(
                                "CANONICAL_ADMISSION_REQUIRED: remote stream data arrived before admission receipt",
                            )))
                            .await;
                        break;
                    };
                    let chunk = remote_stream_chunk(RemoteStreamChunkParts {
                        invocation_id,
                        state: axon_sdk::invocation::InvocationState::Running,
                        payload,
                        sequence,
                        terminal: false,
                        admission_receipt: None,
                        terminal_receipt: None,
                        error: None,
                    });
                    sequence = sequence.saturating_add(1);
                    if tx.send(Ok(chunk)).await.is_err() {
                        break;
                    }
                }
                DispatchStreamEvent::Terminal(result) => {
                    let DispatchResult {
                        payload: _,
                        admission_receipt,
                        terminal_receipt,
                        error,
                        failure,
                        request_id: _,
                        result_content_type: _,
                    } = *result;
                    if let Some(error) = error.filter(|_| terminal_receipt.is_none()) {
                        let _ = tx
                            .send(Err(status_from_remote_failure(
                                "remote stream transport failed",
                                &error,
                                failure.as_ref(),
                            )))
                            .await;
                        break;
                    }
                    let Some(terminal_receipt) = terminal_receipt else {
                        let _ = tx
                            .send(Err(Status::failed_precondition(
                                "CANONICAL_TERMINAL_RECEIPT_REQUIRED: remote stream ended without a terminal checkpoint",
                            )))
                            .await;
                        break;
                    };
                    let finalized = match finalization.finalize(admission_receipt, terminal_receipt)
                    {
                        Ok(finalized) => finalized,
                        Err(status) => {
                            let _ = tx.send(Err(status)).await;
                            break;
                        }
                    };
                    let chunk = remote_stream_chunk(RemoteStreamChunkParts {
                        invocation_id: finalized.terminal_receipt.invocation_id.clone(),
                        state: finalized.terminal_state,
                        payload: finalized.output,
                        sequence,
                        terminal: true,
                        admission_receipt: None,
                        terminal_receipt: Some(finalized.terminal_receipt),
                        error: finalized.failure,
                    });
                    let _ = tx.send(Ok(chunk)).await;
                    break;
                }
            }
        }
    });

    Ok(Response::new(
        Box::pin(ReceiverStream::new(rx)) as BoxedDownStream<InvokeStreamChunk>
    ))
}

async fn project_local_runtime_stream(
    mut handle: StreamingInvocationHandle,
    ability: &str,
    initial_sequence: u64,
    lifecycle: RegisteredInvocationLifecycle,
) -> Result<Response<BoxedDownStream<InvokeStreamChunk>>, Status> {
    let admission_receipt = match handle.admission_receipt().await {
        Ok(receipt) => receipt,
        Err(err) => {
            let _ = lifecycle.finalized().await;
            return Err(Status::failed_precondition(format!(
                "InvokeStream `{ability}` canonical admission unavailable: {err}"
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

    let (tx, rx) = mpsc::channel::<Result<InvokeStreamChunk, Status>>(16);
    let (consumer_closed_tx, mut consumer_closed_rx) = watch::channel(false);
    let invocation_id = handle.invocation_id().to_string();
    let ability_name = ability.to_string();
    tokio::spawn(async move {
        let mut sequence = initial_sequence;
        let mut admission_receipt_sent = false;
        loop {
            let frame_result = tokio::select! {
                changed = consumer_closed_rx.changed() => {
                    if changed.is_ok() && *consumer_closed_rx.borrow() {
                        cancel_abandoned_local_stream(
                            &lifecycle,
                            ability_name.as_str(),
                            invocation_id.as_str(),
                            "stream consumer disconnected",
                        )
                        .await;
                    }
                    break;
                }
                next = handle.next_frame() => {
                    let Some(frame_result) = next else {
                        break;
                    };
                    frame_result
                }
            };
            match frame_result {
                Ok(frame) => {
                    let terminal = frame.terminal;
                    let finalized = if terminal {
                        match lifecycle.finalized().await {
                            Ok(finalized) => Some(finalized),
                            Err(err) => {
                                let _ = tx
                                    .send(Err(Status::failed_precondition(format!(
                                        "CANONICAL_FINALIZATION_REQUIRED: {err}"
                                    ))))
                                    .await;
                                break;
                            }
                        }
                    } else {
                        None
                    };
                    if finalized.as_ref().is_some_and(|value| {
                        value.terminal_state != axon_sdk::invocation::InvocationState::Completed
                            || value.failure.is_some()
                    }) {
                        let _ = tx
                                .send(Err(Status::failed_precondition(
                                    "CANONICAL_FINALIZATION_STATE_MISMATCH: successful stream frame did not finalize Completed",
                                )))
                                .await;
                        break;
                    }
                    let frame_admission_receipt = if admission_receipt_sent {
                        None
                    } else {
                        admission_receipt_sent = true;
                        Some(admission_wire.clone())
                    };
                    let state = finalized
                        .as_ref()
                        .map(|value| value.terminal_state)
                        .unwrap_or(axon_sdk::invocation::InvocationState::Running);
                    let frame_payload = frame.payload;
                    let frame_content_type = frame.content_type;
                    let payload = if terminal && frame_payload.is_empty() {
                        finalized
                            .as_ref()
                            .map(|value| value.output().to_vec())
                            .unwrap_or_default()
                    } else {
                        frame_payload
                    };
                    let frame_content_type = if frame_content_type.is_empty() {
                        finalized
                            .as_ref()
                            .map(|value| value.output_content_type().to_string())
                            .unwrap_or_default()
                    } else {
                        frame_content_type
                    };
                    let content_type = if frame_content_type.is_empty() {
                        FEDERATION_RESULT_CONTENT_TYPE.to_string()
                    } else {
                        frame_content_type
                    };
                    let terminal_receipt = match finalized.as_ref() {
                        Some(value) => match axon_sdk::invocation::wire::receipt_to_wire(
                            &value.terminal_receipt,
                        ) {
                            Ok(receipt) => Some(receipt),
                            Err(error) => {
                                let _ = tx
                                    .send(Err(Status::failed_precondition(format!(
                                        "CANONICAL_TERMINAL_PROJECTION_FAILED: {error}"
                                    ))))
                                    .await;
                                break;
                            }
                        },
                        None => None,
                    };
                    let chunk = InvokeStreamChunk {
                        header: Some(ResponseHeader {
                            request_id: invocation_id.clone(),
                            status: state.as_str().to_string(),
                            ..ResponseHeader::default()
                        }),
                        invocation_id: invocation_id.clone(),
                        state: state.to_wire_i32(),
                        content_type,
                        payload,
                        sequence,
                        terminal,
                        admission_receipt: frame_admission_receipt,
                        terminal_receipt,
                        ..InvokeStreamChunk::default()
                    };
                    sequence = sequence.saturating_add(1);
                    if tx.send(Ok(chunk)).await.is_err() {
                        if !terminal {
                            cancel_abandoned_local_stream(
                                &lifecycle,
                                ability_name.as_str(),
                                invocation_id.as_str(),
                                "stream consumer disconnected",
                            )
                            .await;
                        }
                        break;
                    }
                    if terminal {
                        break;
                    }
                }
                Err(err) => {
                    let finalized = match lifecycle.finalized().await {
                        Ok(finalized) => finalized,
                        Err(finalization_error) => {
                            let _ = tx
                                    .send(Err(Status::failed_precondition(format!(
                                        "CANONICAL_FINALIZATION_REQUIRED: frame_error={err}; finalization_error={finalization_error}"
                                    ))))
                                    .await;
                            break;
                        }
                    };
                    let terminal_error = finalized.failure.as_ref().unwrap_or(&err);
                    let terminal_receipt = match axon_sdk::invocation::wire::receipt_to_wire(
                        &finalized.terminal_receipt,
                    ) {
                        Ok(receipt) => receipt,
                        Err(error) => {
                            let _ = tx
                                .send(Err(Status::failed_precondition(format!(
                                    "CANONICAL_TERMINAL_PROJECTION_FAILED: {error}"
                                ))))
                                .await;
                            break;
                        }
                    };
                    let chunk = InvokeStreamChunk {
                        header: Some(ResponseHeader {
                            request_id: invocation_id.clone(),
                            status: finalized.terminal_state.as_str().to_string(),
                            ..ResponseHeader::default()
                        }),
                        invocation_id: invocation_id.clone(),
                        state: finalized.terminal_state.to_wire_i32(),
                        sequence,
                        terminal: true,
                        admission_receipt: (!admission_receipt_sent)
                            .then(|| admission_wire.clone()),
                        terminal_receipt: Some(terminal_receipt),
                        error: Some(axon_sdk::invocation::wire::error_to_wire(terminal_error)),
                        ..InvokeStreamChunk::default()
                    };
                    let _ = tx.send(Ok(chunk)).await;
                    break;
                }
            }
        }
    });

    Ok(Response::new(
        Box::pin(DropNotifyingReceiverStream::new(rx, consumer_closed_tx))
            as BoxedDownStream<InvokeStreamChunk>,
    ))
}

struct DropNotifyingReceiverStream<T> {
    inner: ReceiverStream<Result<T, Status>>,
    close_tx: Option<watch::Sender<bool>>,
}

impl<T> DropNotifyingReceiverStream<T> {
    fn new(rx: mpsc::Receiver<Result<T, Status>>, close_tx: watch::Sender<bool>) -> Self {
        Self {
            inner: ReceiverStream::new(rx),
            close_tx: Some(close_tx),
        }
    }
}

impl<T> Unpin for DropNotifyingReceiverStream<T> {}

impl<T> Stream for DropNotifyingReceiverStream<T> {
    type Item = Result<T, Status>;

    fn poll_next(mut self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<Option<Self::Item>> {
        Pin::new(&mut self.inner).poll_next(cx)
    }
}

impl<T> Drop for DropNotifyingReceiverStream<T> {
    fn drop(&mut self) {
        if let Some(close_tx) = self.close_tx.take() {
            let _ = close_tx.send(true);
        }
    }
}

async fn cancel_abandoned_local_stream(
    lifecycle: &RegisteredInvocationLifecycle,
    ability: &str,
    invocation_id: &str,
    reason: &'static str,
) {
    match lifecycle.cancel_and_finalize(reason).await {
        Ok(_) => {}
        Err(err) => {
            let err_msg = err.to_string();
            crate::op_event!(
                component = daemon_invocation,
                kind = invoke_stream_local_cancel_finalization_failed,
                ability = ability,
                invocation_id = invocation_id,
                error = err_msg,
            );
        }
    }
}

impl StreamDispatcher {
    async fn resolve_stream_route(
        &self,
        request: &InvokeServerStreamRequest,
    ) -> Result<CanonicalRouteSelection, Status> {
        let target_ura = local_stream_target_ura(request)?;
        let ability =
            function_name_from_invocation_target("InvokeStream", request.target.as_ref())?;

        let selection = self
            .gate
            .route_resolver()
            .await
            .resolve_canonical_route(&target_ura, ability, CallMode::Stream)
            .map_err(route_negative_status)?;
        if let CanonicalRouteDispatch::Local(selected_route) = selection.dispatch() {
            if !selected_route.is_authoritative_local_or_better() {
                return Err(route_profile_blocked_status(selected_route));
            }
        }
        Ok(selection)
    }
}

#[derive(Clone)]
pub(crate) struct DaemonStreamRouteProvider {
    presence: Weak<PresenceRegistry>,
    subscribe_v2_heartbeat_interval_ms: u64,
    daemon_route_lifecycle: Weak<()>,
}

impl DaemonStreamRouteProvider {
    pub(crate) fn new(
        presence: Weak<PresenceRegistry>,
        subscribe_v2_heartbeat_interval_ms: u64,
        daemon_route_lifecycle: Weak<()>,
    ) -> Self {
        Self {
            presence,
            subscribe_v2_heartbeat_interval_ms,
            daemon_route_lifecycle,
        }
    }

    pub(crate) fn invoke(
        &self,
        route: DaemonStreamRoute,
        arguments: serde_json::Value,
    ) -> anyhow::Result<StreamSource> {
        match route {
            DaemonStreamRoute::FederationSubscribeDirectoryV2 => {
                self.subscribe_directory_v2(arguments)
            }
        }
    }

    fn presence(&self, route: DaemonStreamRoute) -> anyhow::Result<Arc<PresenceRegistry>> {
        self.presence.upgrade().ok_or_else(|| {
            anyhow::anyhow!("{}: presence registry is no longer available", route.name())
        })
    }

    fn subscribe_directory_v2(&self, arguments: serde_json::Value) -> anyhow::Result<StreamSource> {
        use crate::daemon::federation::directory::{
            presence_event_to_directory_event, DirectoryEvent,
        };

        subscribe_directory_resume_sequence_value(&arguments)?;
        let presence = self.presence(DaemonStreamRoute::FederationSubscribeDirectoryV2)?;
        let initial_snapshot =
            federation_wrappers::build_subscribe_directory_v2_snapshot(&presence).map_err(
                |err| anyhow::anyhow!("federation.subscribe_directory_v2 initial snapshot: {err}"),
            )?;
        let initial = serde_json::to_value(initial_snapshot).map_err(|err| {
            anyhow::anyhow!("federation.subscribe_directory_v2 initial snapshot: {err}")
        })?;
        let mut events = presence.subscribe_events();
        let presence_weak = Arc::downgrade(&presence);
        let lifecycle_weak = self.daemon_route_lifecycle.clone();
        drop(presence);
        let heartbeat_interval_ms = self.subscribe_v2_heartbeat_interval_ms;
        let (tx, rx) = tokio::sync::broadcast::channel(256);
        tokio::spawn(async move {
            use tokio::sync::broadcast::error::RecvError;

            let mut hb =
                tokio::time::interval(std::time::Duration::from_millis(heartbeat_interval_ms));
            hb.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Skip);
            hb.tick().await;
            let mut shutdown_tick = tokio::time::interval(std::time::Duration::from_millis(100));
            shutdown_tick.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Skip);
            loop {
                tokio::select! {
                    recv = events.recv() => {
                        match recv {
                            Ok(event) => {
                                let directory_event = match presence_event_to_directory_event(&event) {
                                    Ok(event) => event,
                                    Err(err) => {
                                        crate::op_event!(
                                            component = federation_directory,
                                            kind = invalid_presence_event,
                                            error = err,
                                        );
                                        break;
                                    }
                                };
                                let value = match serde_json::to_value(directory_event) {
                                    Ok(value) => value,
                                    Err(_) => break,
                                };
                                if tx.send(value).is_err() {
                                    break;
                                }
                            }
                            Err(RecvError::Lagged(_)) => {
                                let Some(presence) = presence_weak.upgrade() else {
                                    break;
                                };
                                let snapshot = match federation_wrappers::build_subscribe_directory_v2_snapshot(
                                    &presence,
                                ) {
                                    Ok(snapshot) => snapshot,
                                    Err(err) => {
                                        crate::op_event!(
                                            component = federation_directory,
                                            kind = invalid_presence_snapshot,
                                            error = err,
                                        );
                                        break;
                                    }
                                };
                                drop(presence);
                                let Ok(value) = serde_json::to_value(snapshot) else {
                                    break;
                                };
                                if tx.send(value).is_err() {
                                    break;
                                }
                            }
                            Err(RecvError::Closed) => break,
                        }
                    }
                    _ = hb.tick() => {
                        let heartbeat = DirectoryEvent::Heartbeat {
                            unix_ms: crate::daemon::federation::directory::now_unix_ms(),
                        };
                        let Ok(value) = serde_json::to_value(heartbeat) else {
                            break;
                        };
                        if tx.send(value).is_err() {
                            break;
                        }
                    }
                    _ = shutdown_tick.tick() => {
                        if lifecycle_weak.upgrade().is_none()
                            || presence_weak.upgrade().is_none()
                        {
                            break;
                        }
                    }
                }
            }
        });
        Ok(StreamSource::SnapshotThenLive(vec![initial], rx))
    }
}

fn subscribe_directory_resume_sequence(arguments: &[u8]) -> Result<u64, Status> {
    if arguments.is_empty() {
        return Ok(0);
    }
    let value: serde_json::Value = serde_json::from_slice(arguments).map_err(|err| {
        Status::invalid_argument(format!(
            "federation.subscribe_directory_v2 arguments must be JSON: {err}"
        ))
    })?;
    if value.is_null() {
        return Ok(0);
    }
    let object = value.as_object().ok_or_else(|| {
        Status::invalid_argument("federation.subscribe_directory_v2 arguments must be an object")
    })?;
    match object.get("resume_sequence") {
        Some(value) if value.is_null() => Ok(0),
        Some(value) => value.as_u64().ok_or_else(|| {
            Status::invalid_argument(
                "federation.subscribe_directory_v2 resume_sequence must be a non-negative integer",
            )
        }),
        None => Ok(0),
    }
}

fn subscribe_directory_resume_sequence_value(arguments: &serde_json::Value) -> anyhow::Result<u64> {
    if arguments.is_null() {
        return Ok(0);
    }
    let object = arguments.as_object().ok_or_else(|| {
        anyhow::anyhow!("federation.subscribe_directory_v2 arguments must be an object")
    })?;
    match object.get("resume_sequence") {
        Some(value) if value.is_null() => Ok(0),
        Some(value) => value.as_u64().ok_or_else(|| {
            anyhow::anyhow!(
                "federation.subscribe_directory_v2 resume_sequence must be a non-negative integer"
            )
        }),
        None => Ok(0),
    }
}

fn daemon_route_initial_sequence(
    route: DaemonStreamRoute,
    request: &InvokeServerStreamRequest,
) -> Result<u64, Status> {
    match route {
        DaemonStreamRoute::FederationSubscribeDirectoryV2 => {
            let resume_sequence = subscribe_directory_resume_sequence(&request.arguments)?;
            Ok(if resume_sequence == 0 {
                0
            } else {
                resume_sequence.saturating_add(1)
            })
        }
    }
}

fn local_stream_target_ura(request: &InvokeServerStreamRequest) -> Result<String, Status> {
    callee_ura_from_envelope(request.envelope.as_ref(), "InvokeStream")
}

fn stream_request_as_invoke_request(
    request: &InvokeServerStreamRequest,
) -> axon_sdk::pb::axon::v1::InvokeRequest {
    axon_sdk::pb::axon::v1::InvokeRequest {
        envelope: request.envelope.clone(),
        target: request.target.clone(),
        arguments: request.arguments.clone(),
        content_type: request.content_type.clone(),
        timeout_seconds: request.timeout_seconds,
        metadata: request.metadata.clone(),
        payload_ref: request.payload_ref.clone(),
        content_envelope: request.content_envelope.clone(),
    }
}

fn request_timeout(request: &InvokeServerStreamRequest) -> Option<Duration> {
    u64::try_from(request.timeout_seconds)
        .ok()
        .filter(|seconds| *seconds > 0)
        .map(Duration::from_secs)
}

struct RemoteStreamChunkParts {
    invocation_id: String,
    state: axon_sdk::invocation::InvocationState,
    payload: Vec<u8>,
    sequence: u64,
    terminal: bool,
    admission_receipt: Option<axon_sdk::pb::axon::v1::InvocationReceipt>,
    terminal_receipt: Option<axon_sdk::pb::axon::v1::InvocationReceipt>,
    error: Option<Error>,
}

fn remote_stream_chunk(parts: RemoteStreamChunkParts) -> InvokeStreamChunk {
    InvokeStreamChunk {
        header: Some(ResponseHeader {
            request_id: parts.invocation_id.clone(),
            status: parts.state.as_str().to_string(),
            ..ResponseHeader::default()
        }),
        invocation_id: parts.invocation_id,
        state: parts.state.to_wire_i32(),
        content_type: FEDERATION_RESULT_CONTENT_TYPE.to_string(),
        payload: parts.payload,
        sequence: parts.sequence,
        terminal: parts.terminal,
        admission_receipt: parts.admission_receipt,
        terminal_receipt: parts.terminal_receipt,
        error: parts.error,
        ..InvokeStreamChunk::default()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::daemon::invocation::bidi::state::pending_dispatch::PendingStreamDispatchMap;
    use axon_sdk::pb::axon::v1::{
        causal_context, AgentIdentity, CausalContext, Empty, Envelope, InvokeRequest,
        SubjectIdentity,
    };
    use ed25519_dalek::VerifyingKey;
    use futures::StreamExt as _;

    struct RejectingKeyResolver;

    impl KeyResolver for RejectingKeyResolver {
        fn resolve(
            &self,
            _agent_ura: &str,
        ) -> Result<VerifyingKey, axon_sdk::invocation::AxonError> {
            Err(axon_sdk::invocation::AxonError::permission_denied(
                "test resolver should not be reached before remote stream timeout",
            ))
        }
    }

    fn forwarded_request_for_timeout_test() -> InvokeRequest {
        let descriptor_ref = "easynet:///r/test/ability/device.target.media.synthetic_stream@1.0.0#aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa!read";
        InvokeRequest {
            envelope: Some(Envelope {
                caller: Some(AgentIdentity {
                    ura: "easynet:///r/test/device/caller".to_string(),
                    profile: "axon-strict-v2".to_string(),
                }),
                callee: Some(AgentIdentity {
                    ura: "easynet:///r/test/device/target".to_string(),
                    profile: "axon-strict-v2".to_string(),
                }),
                subject: Some(SubjectIdentity {
                    ura: "easynet:///r/test/resource/media/session".to_string(),
                    profile: "axon-strict-v2".to_string(),
                }),
                invocation_nonce: vec![1; 16],
                causal_context: Some(CausalContext {
                    form: Some(causal_context::Form::None(Empty {})),
                }),
                ..Envelope::default()
            }),
            target: Some(
                crate::daemon::invocation::dispatch::invocation_wire::wire_invocation_target(
                    descriptor_ref,
                    "media.synthetic_stream",
                )
                .expect("descriptor target"),
            ),
            arguments: b"{}".to_vec(),
            ..InvokeRequest::default()
        }
    }

    #[tokio::test]
    async fn forwarded_remote_stream_times_out_without_terminal_event() {
        let pending = PendingStreamDispatchMap::new();
        let handle = pending.register_pending_for("easynet:///r/test/device/target");
        let binding =
            ForwardedInvocationBinding::from_request(&forwarded_request_for_timeout_test())
                .expect("forwarded binding");

        let response = project_forwarded_remote_stream(
            RemoteStreamEventSource::Presence(handle),
            binding,
            Arc::new(RejectingKeyResolver),
            Some(Duration::from_millis(10)),
        )
        .await
        .expect("stream response");

        let mut stream = response.into_inner();
        let error = stream
            .next()
            .await
            .expect("deadline emits one item")
            .expect_err("deadline item is an error");
        assert_eq!(error.code(), tonic::Code::DeadlineExceeded);
        assert!(
            error.message().contains("REMOTE_STREAM_TERMINAL_TIMEOUT"),
            "unexpected timeout error: {}",
            error.message()
        );
    }
}
