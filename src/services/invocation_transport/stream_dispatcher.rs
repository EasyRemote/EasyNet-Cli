// EasyNet Daemon — InvokeStream Dispatcher
// =========================================
//
// File: src/services/invocation_transport/stream_dispatcher.rs
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

use easynet_axon::pb::axon::v1::{InvokeServerStreamRequest, InvokeStreamChunk};

use crate::services::invocation_transport::admission_facade::AdmissionFacade;
use crate::services::invocation_transport::deps::{DirectoryPlane, RuntimePlane};
use crate::services::invocation_transport::descriptor_binding::RuntimeBoundAbility;
use crate::services::invocation_transport::federation_wrappers;
use crate::services::invocation_transport::hosted_agent_delegation::HostedAgentDelegationIssuer;
use crate::services::invocation_transport::invocation_wire::{
    status_from_axon_invoke_error, target_ura_from_envelope, BoxedDownStream,
    FEDERATION_RESULT_CONTENT_TYPE,
};
use crate::services::invocation_transport::route_resolver::SelectedInvokeRoute;
use crate::services::invocation_transport::target_gate::{
    route_negative_status, route_profile_blocked_status, route_selected_remote_host_status,
    TargetGate,
};

/// `InvokeStream` routing surface. Cheap per-call construction: both
/// planes and the gate are `Arc`-shaped.
pub(crate) struct StreamDispatcher {
    admission: AdmissionFacade,
    directory: DirectoryPlane,
    runtime: RuntimePlane,
    gate: TargetGate,
}

impl StreamDispatcher {
    pub(crate) fn new(
        admission: AdmissionFacade,
        directory: DirectoryPlane,
        runtime: RuntimePlane,
        gate: TargetGate,
    ) -> Self {
        Self {
            admission,
            directory,
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
            (events, presence_weak),
            |(mut events, presence_weak)| async move {
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
                            ..InvokeStreamChunk::default()
                        };
                        Some((Ok(chunk), (events, presence_weak)))
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
                        Some((Ok(chunk), (events, presence_weak)))
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
    ) -> Result<Response<BoxedDownStream<InvokeStreamChunk>>, Status> {
        use crate::services::federation_directory::{
            presence_event_to_directory_event, DirectoryEvent,
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
        let initial_stream = futures::stream::once(async move { Ok(initial_chunk) });
        let event_stream = futures::stream::unfold(
            (events, presence_weak, heartbeat_interval_ms),
            |(mut events, presence_weak, hb_ms)| async move {
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
                                    ..InvokeStreamChunk::default()
                                };
                                Some((Ok(chunk), (events, presence_weak, hb_ms)))
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
                                    ..InvokeStreamChunk::default()
                                };
                                Some((Ok(chunk), (events, presence_weak, hb_ms)))
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
                            unix_ms: crate::services::federation_directory::now_unix_ms(),
                        };
                        let payload = serde_json::to_vec(&hb_evt)
                            .expect("DirectoryEvent::Heartbeat is statically Serialize");
                        let chunk = InvokeStreamChunk {
                            content_type: FEDERATION_RESULT_CONTENT_TYPE.to_string(),
                            payload,
                            ..InvokeStreamChunk::default()
                        };
                        Some((Ok(chunk), (events, presence_weak, hb_ms)))
                    }
                }
            },
        );

        let combined = futures::StreamExt::chain(initial_stream, event_stream);
        Ok(Response::new(
            Box::pin(combined) as BoxedDownStream<InvokeStreamChunk>
        ))
    }

    /// RFC-005 resolve-first local dispatch for every other
    /// server-stream ability: prove the route, require it to execute
    /// on this daemon, then open the stream through Axon
    /// `LocalRuntime` and pump frames out as `InvokeStreamChunk`s.
    pub(crate) async fn dispatch_local_selected_route(
        &self,
        request: &InvokeServerStreamRequest,
    ) -> Result<Response<BoxedDownStream<InvokeStreamChunk>>, Status> {
        let ability = request.function_name.trim();
        let selected_route = self.resolve_local_route(request).await?;
        let Some(runtime) = self.runtime.local_runtime.as_ref() else {
            return Err(Status::failed_precondition(format!(
                "InvokeStream: ability `{ability}` cannot run because Axon LocalRuntime \
                 is not wired at boot"
            )));
        };
        let selected_ability_ura = selected_route.ability_ura.clone();
        let bound_ability =
            RuntimeBoundAbility::from_selected_route("InvokeStream", runtime, &selected_route)
                .await?;
        let selected_descriptor_ref = bound_ability
            .descriptor_ref_for_mode(
                "InvokeStream",
                &selected_route.callee_ura,
                easynet_axon::invocation::CallMode::Stream,
                Some(&selected_route.route_ura),
            )?
            .into_descriptor_ref();
        bound_ability.require_wire_target_matches(
            "InvokeStream",
            &selected_route.callee_ura,
            ability,
            &selected_route.route_ura,
        )?;
        let loopback_admitted = self
            .admission
            .accepts_loopback_envelope(request.envelope.as_ref());
        let wire = match request.envelope.clone() {
            Some(envelope) if loopback_admitted => {
                let metadata = HostedAgentDelegationIssuer::materialize_request_metadata(
                    &request.metadata,
                    &envelope,
                    true,
                    ability,
                )?;
                crate::runtime::axon_bridge::dispatch_shim::local_system_from_wire_parts(
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
                crate::runtime::axon_bridge::dispatch_shim::external_signed_from_wire_parts(
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
            crate::runtime::axon_bridge::dispatch_shim::open_stream_admitted(runtime, wire)
                .await
                .map_err(|err| status_from_axon_invoke_error("InvokeStream", ability, err))?;

        let (tx, rx) = mpsc::channel::<Result<InvokeStreamChunk, Status>>(16);
        let ability_name = selected_ability_ura;
        tokio::spawn(async move {
            while let Some(frame_result) = handle.next_frame().await {
                match frame_result {
                    Ok(frame) => {
                        let terminal = frame.terminal;
                        let content_type = if frame.content_type.is_empty() {
                            FEDERATION_RESULT_CONTENT_TYPE.to_string()
                        } else {
                            frame.content_type
                        };
                        // Payload emptiness is business content; `terminal`
                        // is lifecycle state. Preserve Axon's terminal frame
                        // even for finite streams that complete with
                        // `Ok(Vec::new())`.
                        let chunk = InvokeStreamChunk {
                            content_type,
                            payload: frame.payload,
                            terminal,
                            ..InvokeStreamChunk::default()
                        };
                        if tx.send(Ok(chunk)).await.is_err() || terminal {
                            break;
                        }
                    }
                    Err(err) => {
                        let _ = tx
                            .send(Err(status_from_axon_invoke_error(
                                "InvokeStream",
                                &ability_name,
                                err,
                            )))
                            .await;
                        break;
                    }
                }
            }
        });

        Ok(Response::new(
            Box::pin(ReceiverStream::new(rx)) as BoxedDownStream<InvokeStreamChunk>
        ))
    }

    async fn resolve_local_route(
        &self,
        request: &InvokeServerStreamRequest,
    ) -> Result<SelectedInvokeRoute, Status> {
        let target_ura = local_stream_target_ura(request)?;
        let ability = request.function_name.trim();
        if ability.is_empty() {
            return Err(Status::invalid_argument(
                "InvokeStream request missing function_name for namespace.resolve",
            ));
        }

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
                "InvokeStream",
                &selected_route,
            ));
        }
        Ok(selected_route)
    }
}

fn local_stream_target_ura(request: &InvokeServerStreamRequest) -> Result<String, Status> {
    target_ura_from_envelope(request.envelope.as_ref(), "InvokeStream")
}

/// Wire projection of a presence transition for the v1
/// `federation.subscribe_directory` stream.
///
/// Mirrors `services::presence_registry::PresenceEvent` but with
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

impl From<crate::services::presence_registry::PresenceEvent> for PresenceEventDelta {
    fn from(event: crate::services::presence_registry::PresenceEvent) -> Self {
        use crate::services::presence_registry::{OfflineReason, PresenceEvent};
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
