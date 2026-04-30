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

use std::pin::Pin;
use std::sync::Arc;

use futures::Stream;
use tonic::{Request, Response, Status, Streaming};

use crate::pb::axon::v1::invocation_server::Invocation;
use crate::pb::axon::v1::{
    InvokeBidiDown, InvokeBidiUp, InvokeRequest, InvokeResponse, InvokeServerStreamRequest,
    InvokeStreamChunk,
};
use crate::services::axon_serve::admission_facade::AdmissionFacade;
use crate::services::axon_serve::federation_wrappers::{
    self, ABILITY_FEDERATION_ADVERTISE_AGENT, ABILITY_FEDERATION_FORWARD_INVOKE,
    ABILITY_FEDERATION_HEARTBEAT, ABILITY_FEDERATION_JOIN, ABILITY_FEDERATION_RESOLVE,
    ABILITY_FEDERATION_REVOKE, ABILITY_FEDERATION_SUBSCRIBE_DIRECTORY,
};
use crate::services::presence_registry::PresenceRegistry;

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
#[derive(Debug)]
pub struct DaemonInvocationService {
    presence: Arc<PresenceRegistry>,
    admission: AdmissionFacade,
}

impl DaemonInvocationService {
    /// Construct a service against the supplied presence registry
    /// and admission facade. Production callers wire one registry
    /// per daemon process and share it via `Arc` between the
    /// service, the `<self>.session` accept loop (PR-2), and any
    /// audit-log subscriber. The admission facade is constructed
    /// from `RealmTrustAnchor::load_or_empty(...)` at daemon boot.
    #[must_use]
    pub fn new(presence: Arc<PresenceRegistry>, admission: AdmissionFacade) -> Self {
        Self {
            presence,
            admission,
        }
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

    /// Spec §2.1 reference. PR-1 returns Unimplemented; PR-2
    /// implements `<self>.session` accept (莫浩) and PR-3 implements
    /// `<self>.invoke_remote` (海峰).
    async fn invoke_bidi(
        &self,
        _request: Request<Streaming<InvokeBidiUp>>,
    ) -> Result<Response<Self::InvokeBidiStream>, Status> {
        Err(Status::unimplemented(
            "easynet-daemon: InvokeBidi is the `<self>.session` and `<self>.invoke_remote` \
             entry point; real handlers are RFC-003 PR-2 (`<self>.session`, 莫浩) and PR-3 \
             (`<self>.invoke_remote`, 海峰); see checklists/PR-2-checklist.md and \
             checklists/PR-3-checklist.md",
        ))
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
                                federation_wrappers::build_subscribe_directory_initial(
                                    &presence,
                                );
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
        ..InvokeResponse::default()
    };
    Ok(Response::new(invoke_response))
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
        match svc
            .invoke(invoke_request("custom.ability.x", "{}"))
            .await
        {
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
        let close = tokio::time::timeout(
            std::time::Duration::from_secs(2),
            stream.next(),
        )
        .await
        .expect("pump closes within 2 s after senders drop");
        assert!(close.is_none(), "stream must terminate once all senders drop");
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
        // Build a service whose admission facade has no daemon URI
        // and an empty trust anchor — every external caller is
        // rejected.
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
            Err(err) => assert_eq!(err.code(), tonic::Code::PermissionDenied),
            Ok(_) => panic!("caller outside trust anchor must be rejected"),
        }
    }

    #[tokio::test]
    async fn invoke_stream_rejects_caller_not_in_trust_anchor() {
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
            Err(err) => assert_eq!(err.code(), tonic::Code::PermissionDenied),
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
}
