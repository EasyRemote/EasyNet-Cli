// EasyNet Daemon — InvokeStream Dispatcher
// =========================================
//
// File: src/daemon/invocation/stream_dispatcher.rs
// Description: Owns every `InvokeStream` routing decision the daemon
//              makes after transport policy (commit-plan-2 Axis E / E2):
//
//                * `federation.subscribe_directory`     — v1 presence pump
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

use std::sync::Arc;

use tokio::sync::mpsc;
use tokio_stream::wrappers::ReceiverStream;
use tonic::{Response, Status};

use easynet_axon::invocation::CallMode;
use easynet_axon::pb::axon::v1::{
    Error, InvokeServerStreamRequest, InvokeStreamChunk, ResponseHeader,
};

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
    DispatchResult, DispatchStreamEvent,
};
use crate::daemon::invocation::dispatch::deps::{DirectoryPlane, RuntimePlane, SessionPlane};
use crate::daemon::invocation::dispatch::descriptor_binding::RuntimeBoundAbility;
use crate::daemon::invocation::dispatch::federation_wrappers;
use crate::daemon::invocation::dispatch::forwarded_finalization::{
    ForwardedFinalizationVerifier, ForwardedInvocationBinding,
};
use crate::daemon::invocation::dispatch::invocation_wire::{
    status_from_axon_invoke_error, target_ura_from_envelope, BoxedDownStream,
    FEDERATION_RESULT_CONTENT_TYPE, SIGNED_DESCRIPTOR_REF_METADATA_KEY,
};
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
}

impl StreamDispatcher {
    pub(crate) fn new(
        admission: AdmissionFacade,
        directory: DirectoryPlane,
        sessions: SessionPlane,
        runtime: RuntimePlane,
        gate: TargetGate,
    ) -> Self {
        Self {
            admission,
            directory,
            sessions,
            runtime,
            gate,
        }
    }

    pub(crate) fn dispatch_subscribe_directory_initial(
        &self,
    ) -> Result<Response<BoxedDownStream<InvokeStreamChunk>>, Status> {
        let initial =
            federation_wrappers::build_subscribe_directory_initial(&self.directory.presence);
        let initial_bytes = serde_json::to_vec(&initial).map_err(|err| {
            Status::internal(format!(
                "federation.subscribe_directory: failed to encode initial snapshot: {err}"
            ))
        })?;
        let initial_chunk = InvokeStreamChunk {
            content_type: FEDERATION_RESULT_CONTENT_TYPE.to_string(),
            payload: initial_bytes,
            sequence: 0,
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
        let events = self.directory.presence.subscribe_events();
        let presence_weak = Arc::downgrade(&self.directory.presence);

        let initial_stream = futures::stream::once(async move { Ok(initial_chunk) });
        let event_stream = futures::stream::unfold(
            (events, presence_weak, 1_u64),
            |(mut events, presence_weak, next_sequence)| async move {
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
                            sequence: next_sequence,
                            ..InvokeStreamChunk::default()
                        };
                        Some((
                            Ok(chunk),
                            (events, presence_weak, next_sequence.saturating_add(1)),
                        ))
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
                            sequence: next_sequence,
                            ..InvokeStreamChunk::default()
                        };
                        Some((
                            Ok(chunk),
                            (events, presence_weak, next_sequence.saturating_add(1)),
                        ))
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
    pub(crate) fn dispatch_subscribe_directory_v2(
        &self,
        arguments: &[u8],
    ) -> Result<Response<BoxedDownStream<InvokeStreamChunk>>, Status> {
        use crate::daemon::federation::directory::{
            presence_event_to_directory_event, DirectoryEvent,
        };

        let resume_sequence = subscribe_directory_resume_sequence(arguments)?;
        let first_sequence = if resume_sequence == 0 {
            0
        } else {
            resume_sequence.saturating_add(1)
        };
        let initial_evt =
            federation_wrappers::build_subscribe_directory_v2_snapshot(&self.directory.presence);
        let initial_bytes = serde_json::to_vec(&initial_evt).map_err(|err| {
            Status::internal(format!(
                "federation.subscribe_directory_v2: encode initial snapshot: {err}"
            ))
        })?;
        let initial_chunk = InvokeStreamChunk {
            content_type: FEDERATION_RESULT_CONTENT_TYPE.to_string(),
            payload: initial_bytes,
            sequence: first_sequence,
            ..InvokeStreamChunk::default()
        };

        let events = self.directory.presence.subscribe_events();
        let presence_weak = Arc::downgrade(&self.directory.presence);

        // Heartbeat tick: spec §2.3 says emit Heartbeat every
        // 30s when no other frame has been emitted in window.
        // The interval is plane-configurable for test ergonomics;
        // production stays at the 30 000ms default.
        // Skip-on-missed-tick keeps cadence aligned when a real
        // event arrives close to the deadline.
        let heartbeat_interval_ms: u64 = self.directory.subscribe_v2_heartbeat_interval_ms;
        let next_sequence = first_sequence.saturating_add(1);
        let initial_stream = futures::stream::once(async move { Ok(initial_chunk) });
        let event_stream = futures::stream::unfold(
            (events, presence_weak, heartbeat_interval_ms, next_sequence),
            |(mut events, presence_weak, hb_ms, next_sequence)| async move {
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
                                    sequence: next_sequence,
                                    ..InvokeStreamChunk::default()
                                };
                                Some((Ok(chunk), (events, presence_weak, hb_ms, next_sequence.saturating_add(1))))
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
                                    sequence: next_sequence,
                                    ..InvokeStreamChunk::default()
                                };
                                Some((Ok(chunk), (events, presence_weak, hb_ms, next_sequence.saturating_add(1))))
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
                            unix_ms: crate::daemon::federation::directory::now_unix_ms(),
                        };
                        let payload = serde_json::to_vec(&hb_evt)
                            .expect("DirectoryEvent::Heartbeat is statically Serialize");
                            let chunk = InvokeStreamChunk {
                                content_type: FEDERATION_RESULT_CONTENT_TYPE.to_string(),
                                payload,
                                sequence: next_sequence,
                                ..InvokeStreamChunk::default()
                            };
                            Some((Ok(chunk), (events, presence_weak, hb_ms, next_sequence.saturating_add(1))))
                    }
                }
            },
        );

        let combined = futures::StreamExt::chain(initial_stream, event_stream);
        Ok(Response::new(
            Box::pin(combined) as BoxedDownStream<InvokeStreamChunk>
        ))
    }

    /// RFC-005 resolve-first dispatch for every other server-stream
    /// ability: prove the route, then send it either to this daemon's
    /// `LocalRuntime` or to the resolver-selected execution host's
    /// presence session.
    pub(crate) async fn dispatch_selected_route(
        &self,
        request: &InvokeServerStreamRequest,
    ) -> Result<Response<BoxedDownStream<InvokeStreamChunk>>, Status> {
        let selection = self.resolve_stream_route(request).await?;
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

    async fn dispatch_local_resolved_route(
        &self,
        request: &InvokeServerStreamRequest,
        selected_route: SelectedInvokeRoute,
        call_mode: CallMode,
    ) -> Result<Response<BoxedDownStream<InvokeStreamChunk>>, Status> {
        let ability = request.function_name.trim();
        let Some(runtime) = self.runtime.local_runtime.as_ref() else {
            return Err(Status::failed_precondition(format!(
                "InvokeStream: ability `{ability}` cannot run because Axon LocalRuntime \
                 is not wired at boot"
            )));
        };
        let bound_ability =
            RuntimeBoundAbility::from_selected_route("InvokeStream", runtime, &selected_route)
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
        let local_self_admitted = self
            .admission
            .accepts_local_self_envelope(request.envelope.as_ref());
        let wire = match request.envelope.clone() {
            Some(envelope) if local_self_admitted => {
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
                crate::daemon::axon_bridge::dispatch_shim::external_signed_from_wire_parts(
                    envelope,
                    selected_descriptor_ref,
                    request.arguments.clone(),
                    metadata,
                )
            }
            None => Err(Box::new(
                easynet_axon::invocation::AxonError::invalid_argument(
                    "InvokeStream request missing envelope",
                ),
            )),
        }
        .map_err(|err| status_from_axon_invoke_error("InvokeStream", ability, *err))?;
        let mut handle =
            crate::daemon::axon_bridge::dispatch_shim::open_stream_admitted(runtime, wire)
                .await
                .map_err(|err| status_from_axon_invoke_error("InvokeStream", ability, err))?;
        let admission_receipt = handle.admission_receipt().await.map_err(|err| {
            Status::failed_precondition(format!(
                "InvokeStream `{ability}` canonical admission unavailable: {err}"
            ))
        })?;
        let admission_wire = easynet_axon::invocation::wire::receipt_to_wire(&admission_receipt)
            .map_err(|error| {
                Status::failed_precondition(format!(
                    "CANONICAL_ADMISSION_PROJECTION_FAILED: {error}"
                ))
            })?;

        let (tx, rx) = mpsc::channel::<Result<InvokeStreamChunk, Status>>(16);
        let invocation_id = handle.invocation_id().to_string();
        let selected_node_id = selected_route.route_ura.clone();
        tokio::spawn(async move {
            let mut sequence = 0_u64;
            let mut admission_receipt_sent = false;
            while let Some(frame_result) = handle.next_frame().await {
                match frame_result {
                    Ok(frame) => {
                        let terminal = frame.terminal;
                        let finalized = if terminal {
                            match handle.finalized().await {
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
                            value.terminal_state
                                != easynet_axon::invocation::InvocationState::Completed
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
                            .unwrap_or(easynet_axon::invocation::InvocationState::Running);
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
                            Some(value) => match easynet_axon::invocation::wire::receipt_to_wire(
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
                            selected_node_id: selected_node_id.clone(),
                            scheduling_reason: "local-runtime".to_string(),
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
                        if tx.send(Ok(chunk)).await.is_err() || terminal {
                            break;
                        }
                    }
                    Err(err) => {
                        let finalized = match handle.finalized().await {
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
                        let terminal_receipt = match easynet_axon::invocation::wire::receipt_to_wire(
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
                            selected_node_id: selected_node_id.clone(),
                            scheduling_reason: "local-runtime".to_string(),
                            state: finalized.terminal_state.to_wire_i32(),
                            sequence,
                            terminal: true,
                            admission_receipt: (!admission_receipt_sent)
                                .then(|| admission_wire.clone()),
                            terminal_receipt: Some(terminal_receipt),
                            error: Some(easynet_axon::invocation::wire::error_to_wire(
                                terminal_error,
                            )),
                            ..InvokeStreamChunk::default()
                        };
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

    async fn dispatch_remote_selected_route(
        &self,
        request: &InvokeServerStreamRequest,
        selected_route: SelectedInvokeRoute,
        call_mode: CallMode,
    ) -> Result<Response<BoxedDownStream<InvokeStreamChunk>>, Status> {
        let ability = request.function_name.trim().to_string();
        let Some(envelope) = request.envelope.clone() else {
            return Err(Status::invalid_argument(format!(
                "InvokeStream: remote-hosted ability `{ability}` requires the seven-tuple \
                 envelope on the canonical Invocation face",
            )));
        };
        signed_envelope_for_selected_route(
            envelope,
            &selected_route,
            request
                .metadata
                .get(SIGNED_DESCRIPTOR_REF_METADATA_KEY)
                .map(String::as_str),
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

        let mut handle = pending.register_pending_for(&selected_route.execution_host_ura);
        let call_id = handle.call_id();
        let forwarded_request = easynet_axon::pb::axon::v1::InvokeRequest {
            envelope: request.envelope.clone(),
            function_name: request.function_name.clone(),
            arguments: request.arguments.clone(),
            content_type: request.content_type.clone(),
            content_envelope: request.content_envelope.clone(),
            metadata: request.metadata.clone(),
            payload_ref: request.payload_ref.clone(),
            ..easynet_axon::pb::axon::v1::InvokeRequest::default()
        };
        let forwarded_binding = ForwardedInvocationBinding::from_request(&forwarded_request)?;
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

        let (tx, rx) = mpsc::channel::<Result<InvokeStreamChunk, Status>>(16);
        let route_ura = selected_route.route_ura.clone();
        tokio::spawn(async move {
            let mut sequence = 0_u64;
            let mut canonical_invocation_id = None::<String>;
            let mut finalization =
                ForwardedFinalizationVerifier::new(forwarded_binding, receipt_resolver);
            while let Some(event) = handle.recv().await {
                match event {
                    DispatchStreamEvent::Admission(receipt) => {
                        if let Err(status) = finalization.admit(receipt.clone()) {
                            let _ = tx.send(Err(status)).await;
                            break;
                        }
                        canonical_invocation_id = Some(receipt.invocation_id.clone());
                        let chunk = remote_stream_chunk(RemoteStreamChunkParts {
                            invocation_id: receipt.invocation_id.clone(),
                            selected_node_id: route_ura.clone(),
                            state: easynet_axon::invocation::InvocationState::Running,
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
                            selected_node_id: route_ura.clone(),
                            state: easynet_axon::invocation::InvocationState::Running,
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
                            failure: _,
                            request_id: _,
                        } = *result;
                        if let Some(error) = error.filter(|_| terminal_receipt.is_none()) {
                            let _ = tx
                                .send(Err(Status::unavailable(format!(
                                    "remote stream transport failed: {error}"
                                ))))
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
                        let finalized =
                            match finalization.finalize(admission_receipt, terminal_receipt) {
                                Ok(finalized) => finalized,
                                Err(status) => {
                                    let _ = tx.send(Err(status)).await;
                                    break;
                                }
                            };
                        let chunk = remote_stream_chunk(RemoteStreamChunkParts {
                            invocation_id: finalized.terminal_receipt.invocation_id.clone(),
                            selected_node_id: route_ura.clone(),
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

    async fn resolve_stream_route(
        &self,
        request: &InvokeServerStreamRequest,
    ) -> Result<CanonicalRouteSelection, Status> {
        let target_ura = local_stream_target_ura(request)?;
        let ability = request.function_name.trim();
        if ability.is_empty() {
            return Err(Status::invalid_argument(
                "InvokeStream request missing function_name for namespace.resolve",
            ));
        }

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

fn local_stream_target_ura(request: &InvokeServerStreamRequest) -> Result<String, Status> {
    target_ura_from_envelope(request.envelope.as_ref(), "InvokeStream")
}

struct RemoteStreamChunkParts {
    invocation_id: String,
    selected_node_id: String,
    state: easynet_axon::invocation::InvocationState,
    payload: Vec<u8>,
    sequence: u64,
    terminal: bool,
    admission_receipt: Option<easynet_axon::pb::axon::v1::InvocationReceipt>,
    terminal_receipt: Option<easynet_axon::pb::axon::v1::InvocationReceipt>,
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
        selected_node_id: parts.selected_node_id,
        scheduling_reason: "remote-presence-session".to_string(),
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

/// Wire projection of a presence transition for the v1
/// `federation.subscribe_directory` stream.
///
/// Mirrors `daemon::invocation::bidi::state::presence::PresenceEvent` but with
/// `serde::Serialize`-friendly field naming so the JSON encoding
/// is stable for PR-4's schema-compat captures.
#[derive(Debug, serde::Serialize, serde::Deserialize, PartialEq, Eq)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub(crate) enum PresenceEventDelta {
    Online {
        membership_ura: String,
    },
    Offline {
        membership_ura: String,
        reason: &'static str,
    },
}

impl From<crate::daemon::invocation::bidi::state::presence::PresenceEvent> for PresenceEventDelta {
    fn from(event: crate::daemon::invocation::bidi::state::presence::PresenceEvent) -> Self {
        use crate::daemon::invocation::bidi::state::presence::{OfflineReason, PresenceEvent};
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
