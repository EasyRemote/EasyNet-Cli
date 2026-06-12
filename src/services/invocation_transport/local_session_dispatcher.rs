// EasyNet CLI — `<self>.session` device-side LocalAxonSessionDispatcher
// =================================================================
//
// File: src/services/invocation_transport/local_session_dispatcher.rs
//
// Device-side `<self>.session` dispatcher. It decodes
// `SessionDispatch::Dispatch{call_id, ability, args}`, routes the
// ability through the daemon's boot-threaded Axon `LocalRuntime`, then
// encodes the outcome back as `SessionDispatch::Result`.
//
// Args contract
// -------------
// `SessionDispatch::Dispatch.args` stays wire-opaque at the
// `<self>.invoke_remote` layer, but the in-process local registry is
// JSON-shaped today (`serde_json::Value`). The device-side session
// handler therefore interprets the bytes as JSON exactly at the
// final execution boundary. Malformed JSON is surfaced back to the
// caller as a terminal `SessionDispatch::Result{error: ...}` rather
// than as a transport reset.
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

use crate::services::invocation_transport::invoke_remote_initiator::{
    call_id_hex, SessionContentEnvelope, SessionDispatch,
};
use crate::services::invocation_transport::session_initiator::{
    SessionDispatchError, SessionFrameDispatcher, SessionUpSender, SESSION_STREAM_ID,
};
use crate::services::session_failure::SessionFailure;
use easynet_axon::pb::axon::v1::invoke_bidi_down::Payload as DownPayload;
use easynet_axon::pb::axon::v1::invoke_bidi_up::Payload as UpPayload;
#[cfg(test)]
use easynet_axon::pb::axon::v1::InvokeBidiUp;
use easynet_axon::pb::axon::v1::{BinaryChunk, InvokeBidiDown};

/// Device-side `<self>.session` dispatcher. Executes inbound
/// Dispatch frames against the daemon's shared Axon `LocalRuntime`,
/// returning the result payload or typed failure over the existing
/// `SessionDispatch::Result` wire shape.
#[derive(Clone)]
pub struct LocalAxonSessionDispatcher {
    /// PR-N6 C4 device-side correlation table. Populated in
    /// device-mode boot when the daemon also constructs a
    /// `SessionEscalationHandle`; left `None` in hub or `both`
    /// modes (those daemons never escalate forward_invoke and so
    /// never receive `RequestResult` frames). When set, inbound
    /// `SessionDispatch::RequestResult` frames are routed here
    /// by `call_id`, completing the awaiting dispatcher future.
    escalation_correlation: Option<
        Arc<crate::services::invocation_transport::session_escalation::EscalationCorrelation>,
    >,
    /// Active same-hub remote bidi sessions keyed by dispatcher
    /// call_id. The hub opens the local bidi on the device, then
    /// subsequent `SessionDispatch::BidiInput` frames route through
    /// this table with ability-specific payload mapping.
    remote_bidi_sessions: Arc<Mutex<HashMap<u64, ActiveRemoteBidi>>>,
    /// Active server-stream sessions keyed by dispatcher call_id.
    /// The hub reuses `BidiInput{eof=true}` as the cancel signal
    /// when an InvokeRemoteStream/SSE consumer disconnects.
    remote_stream_sessions: Arc<Mutex<HashMap<u64, CancellationToken>>>,
    /// **Phase 5d — Axon LocalRuntime bridge**. When wired, inbound
    /// `SessionDispatch::Dispatch` frames whose ability is
    /// registered in the runtime are routed through Axon's
    /// `invoke_async`.
    ///
    /// Why this exists: pre-this-PR, a chat call arriving as a
    /// `SessionDispatch::Dispatch` on this host's session bypassed
    /// the runtime entirely. Net effect: the chat succeeded but
    /// the `LedgerSink` installed at boot never observed the
    /// terminal, so the host's `invocations.redb` stayed empty
    /// and the Web UI's history tab showed `0` records even on
    /// successful chats.
    ///
    /// Setting this Arc closes that gap. The runtime owns the
    /// AbilityFn closures, so dispatch
    /// observability (state machine, AbilityChangeEvent, ledger
    /// persistence) is uniform with the unary-invoke path.
    local_runtime: Option<Arc<easynet_axon::invocation::LocalRuntime>>,
    /// Daemon-owned wire profile registry for local bidi abilities. Plugin
    /// declarations are projected into this table at boot so the dispatcher
    /// does not query package state through process-global helpers.
    ability_wire: Arc<crate::runtime::ability_wire::AbilityWireRegistry>,
    /// On-miss device key sync for cross-device origin-caller claims
    /// (see `device_trust_sync`). `None` outside device-mode boot.
    device_trust_sync:
        Option<Arc<crate::services::invocation_transport::device_trust_sync::DeviceTrustSync>>,
}

type LocalBidiWireKind = crate::runtime::ability_wire::AbilityBidiWireKind;

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

impl LocalAxonSessionDispatcher {
    /// Carrier-v1 dispatch (DEC-F004 / step 3): the frame already IS
    /// the canonical invocation — no JSON re-projection, no owner
    /// re-derivation. Replies follow the session's negotiated
    /// contract; a v1 frame on a v0-negotiated session (hub jumped
    /// the gun) still executes and replies JSON, which the hub
    /// dual-reads.
    async fn handle_carrier_v1_dispatch(
        &self,
        call: easynet_axon::pb::axon::v1::DispatchCall,
        outbound: &SessionUpSender,
    ) -> Result<(), SessionDispatchError> {
        use easynet_axon::pb::axon::v1::DispatchResult as PbDispatchResult;

        let call_id = call.call_id;
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
        let wire = crate::runtime::axon_bridge::dispatch_shim::admitted_from_wire_parts(
            envelope,
            function_name,
            request.arguments,
        )
        .map_err(|err| SessionDispatchError::Other(format!("admit carrier-v1 dispatch: {err}")))?;
        let outcome =
            crate::runtime::axon_bridge::dispatch_shim::dispatch_rpc_admitted(&runtime, wire).await;

        if outbound.carrier_v1() {
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
            let reply = PbDispatchResult {
                call_id,
                payload: outcome.payload_bytes,
                terminal: true,
                // The locally-minted execution receipt, projected onto
                // the wire with every audit-bearing field intact — the
                // hub ledgers it (step 2c) and the chain closes at the
                // hub hop (DEC-F004's headline win).
                receipt: outcome
                    .terminal_receipt
                    .as_ref()
                    .map(easynet_axon::invocation::wire::receipt_to_wire),
                failure,
            };
            outbound
                .send_payload(UpPayload::DispatchResult(reply))
                .await
                .map_err(|_| {
                    SessionDispatchError::Other("session up channel closed".to_string())
                })?;
        } else {
            let (payload, error) =
                crate::runtime::axon_bridge::dispatch_shim::outcome_to_invoke_remote_result(
                    outcome,
                );
            let result = SessionDispatch::Result {
                call_id,
                payload,
                terminal: true,
                failure: error.as_ref().map(|reason| {
                    crate::services::session_failure::SessionFailure::from_reason(
                        reason,
                        "INVOCATION_FAILED",
                        false,
                    )
                }),
                error,
                request_id: None,
            };
            let bytes = result.encode_frame().map_err(|err| {
                SessionDispatchError::Other(format!("encode carrier-v0 reply: {err}"))
            })?;
            outbound
                .send_binary_chunk(BinaryChunk {
                    stream_id: SESSION_STREAM_ID,
                    data: bytes,
                    ..BinaryChunk::default()
                })
                .await
                .map_err(|_| {
                    SessionDispatchError::Other("session up channel closed".to_string())
                })?;
        }
        Ok(())
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
        registry: &crate::runtime::ability_wire::AbilityWireRegistry,
        ability: &str,
    ) -> bool {
        matches!(
            registry.bidi_wire_kind_for(ability),
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
            local_runtime: None,
            ability_wire: Arc::new(crate::runtime::ability_wire::AbilityWireRegistry::core()),
            device_trust_sync: None,
        }
    }

    /// Builder seam: attach the daemon-owned wire registry computed from the
    /// same plugin runtime state used for ability registration.
    #[must_use]
    pub fn with_ability_wire_registry(
        mut self,
        registry: Arc<crate::runtime::ability_wire::AbilityWireRegistry>,
    ) -> Self {
        self.ability_wire = registry;
        self
    }

    /// Normalize a local execution outcome onto `call_id` and push it
    /// up the session bidi. Shared by the inline dispatch path and the
    /// spawned claim-dispatch task.
    async fn send_result_frame(
        outbound: &SessionUpSender,
        call_id: u64,
        result: SessionDispatch,
    ) -> Result<(), SessionDispatchError> {
        let result = match result {
            SessionDispatch::Result {
                payload,
                terminal,
                failure,
                error,
                request_id,
                ..
            } => SessionDispatch::Result {
                call_id,
                payload,
                terminal,
                failure,
                error,
                request_id,
            },
            SessionDispatch::Dispatch { .. } | SessionDispatch::BidiOpen { .. } => {
                unreachable!("local execution never returns Dispatch")
            }
            SessionDispatch::BidiInput { .. }
            | SessionDispatch::Request { .. }
            | SessionDispatch::RequestResult { .. } => {
                // PR-N6 wire shape (C2) added these for the
                // device → hub forward_invoke escalation path.
                // LocalAxonSessionDispatcher only handles
                // hub-pushed Dispatch frames + their Result
                // replies; Request/RequestResult never reach
                // this code path by construction.
                unreachable!(
                    "local execution never returns Request / RequestResult \
                     (those flow on the device → hub direction handled by C3/C4)"
                )
            }
        };

        let payload = serde_json::to_vec(&result).map_err(|err| {
            SessionDispatchError::Other(format!("encode SessionDispatch::Result: {err}"))
        })?;

        let payload_len = payload.len();
        crate::op_event!(
            component = local_session_dispatcher,
            kind = sending_result_frame_up_bidi,
            call_id = call_id,
            payload_bytes = payload_len,
        );

        let send_result = outbound
            .send_binary_chunk(BinaryChunk {
                stream_id: SESSION_STREAM_ID,
                data: payload,
                ..BinaryChunk::default()
            })
            .await
            .map_err(|_| SessionDispatchError::Other("outbound channel closed".to_string()));

        if send_result.is_err() {
            crate::op_event!(
                component = local_session_dispatcher,
                kind = result_frame_send_failed,
                call_id = call_id,
                reason = "outbound_channel_closed",
            );
        } else {
            crate::op_event!(
                component = local_session_dispatcher,
                kind = result_frame_sent_up_bidi,
                call_id = call_id,
            );
        }

        send_result
    }

    #[must_use]
    pub fn with_device_trust_sync(
        mut self,
        sync: Arc<crate::services::invocation_transport::device_trust_sync::DeviceTrustSync>,
    ) -> Self {
        self.device_trust_sync = Some(sync);
        self
    }

    /// Builder seam: attach a device-mode escalation correlation
    /// table so inbound `RequestResult` frames complete the
    /// matching pending dispatcher future. Boot calls this in
    /// device-mode only.
    #[must_use]
    pub fn with_escalation_correlation(
        mut self,
        correlation: Arc<
            crate::services::invocation_transport::session_escalation::EscalationCorrelation,
        >,
    ) -> Self {
        self.escalation_correlation = Some(correlation);
        self
    }

    /// **Phase 5d**. Attach the shared Axon `LocalRuntime` so
    /// inbound `SessionDispatch::Dispatch` frames flow through
    /// `invoke_async` (and therefore the wired `LedgerSink`) when
    /// the runtime hosts the ability. Boot wires this from the
    /// same `Arc<LocalRuntime>` the service uses for Phase 4's
    /// `<self>.invoke_remote` arm.
    #[must_use]
    pub fn with_local_runtime(
        mut self,
        runtime: Arc<easynet_axon::invocation::LocalRuntime>,
    ) -> Self {
        self.local_runtime = Some(runtime);
        self
    }

    /// **Phase 5d**. Try to route an inbound `SessionDispatch::Dispatch`
    /// through the wired Axon `LocalRuntime`. Returns:
    ///
    ///   * `Some(SessionDispatch::Result)` — the runtime hosts the
    ///     ability and dispatched it; payload + error fields carry
    ///     the outcome verbatim. LedgerSink wrote a record on the
    ///     terminal event.
    ///   * `None` — runtime not wired OR doesn't host the ability;
    ///     caller returns a typed terminal error.
    ///
    /// Args are passed verbatim as `Vec<u8>` to `invoke_async`; the
    /// `rpc_handler_to_ability_fn` adapter handles JSON decode inside
    /// the runtime handler, so the payload-bytes contract matches the
    /// wire shape.
    async fn try_dispatch_via_axon(
        &self,
        call_id: u64,
        callee_ura: Option<&str>,
        subject_ura: Option<&str>,
        ability: &str,
        args: &[u8],
        metadata: &std::collections::HashMap<String, String>,
        origin_claim: Option<
            &crate::services::invocation_transport::origin_caller::OriginCallerClaim,
        >,
    ) -> Option<SessionDispatch> {
        let runtime = self.local_runtime.as_ref()?;
        if !runtime.has_ability(ability).await {
            return None;
        }
        crate::op_event!(
            component = local_session_dispatcher,
            kind = try_dispatch_via_axon,
            call_id = call_id,
            ability = ability,
        );

        // Inner user-caller pass-through: when the hub/backend attached a
        // browser-signed user claim (typed `origin_caller` field, legacy
        // metadata item as rolling-upgrade fallback), dispatch with the
        // REAL user as caller via `invoke_externally_signed_*`
        // (cryptographic admission) instead of the `_system`
        // trust-domain default. This is what lets fail-closed abilities
        // (remote desktop consent) see the user. Absent → existing
        // path. Malformed → fail closed.
        let origin_caller =
            match crate::services::invocation_transport::origin_caller::OriginCaller::resolve(
                origin_claim,
                metadata,
            ) {
                Ok(oc) => oc,
                Err(err) => {
                    return Some(Self::session_error_result(
                        call_id,
                        format!("<self>.session: invalid origin caller claim: {err}"),
                    ));
                }
            };

        let outcome = if let Some(origin) = origin_caller {
            crate::op_event!(
                component = local_session_dispatcher,
                kind = dispatch_via_axon_user_caller,
                call_id = call_id,
                caller_ura = origin.caller_ura.as_str(),
                ability = ability,
            );
            // Cross-device callers: warm the anchor from the hub on a
            // miss (resolve_key trust sync). Admission below stays
            // local-anchor-authoritative; a failed sync just lets the
            // dispatch fail closed with the precise admission error.
            if let Some(sync) = self.device_trust_sync.as_ref() {
                sync.ensure_caller_key(&origin.caller_ura).await;
            }
            let inner_subject = subject_ura
                .filter(|s| !s.trim().is_empty())
                .or(callee_ura)
                .unwrap_or(ability);
            let inner_callee = callee_ura.unwrap_or(ability);
            // The browser signed the PUBLIC ability name (origin.ability,
            // e.g. `chat`); the hub addressed the owner-scoped dispatch
            // KEY (`ability`, e.g. `demo.chat`) which is what's actually
            // in the local registry. Verify against the signed name, but
            // resolve + launch the handler under the dispatch key — else
            // agent-owned abilities fail `unknown_ability:<public>`.
            let dispatch_key = if ability == origin.public_ability() {
                None
            } else {
                Some(ability.to_string())
            };
            let wire = origin.into_wire_dispatch(inner_callee, inner_subject, args.to_vec());
            crate::runtime::axon_bridge::dispatch_shim::dispatch_rpc_with_dispatch_key(
                runtime,
                wire,
                dispatch_key,
            )
            .await
        } else {
            match (callee_ura, subject_ura) {
                (Some(callee), Some(subject)) if !subject.trim().is_empty() => {
                    crate::runtime::axon_bridge::dispatch_shim::dispatch_rpc_local_with_subject(
                        runtime,
                        callee,
                        subject,
                        ability,
                        args.to_vec(),
                    )
                    .await
                }
                _ => {
                    crate::runtime::axon_bridge::dispatch_shim::dispatch_rpc_local(
                        runtime,
                        ability,
                        args.to_vec(),
                    )
                    .await
                }
            }
        };
        let request_id = outcome.invocation_id.clone();
        let (payload, error) =
            crate::runtime::axon_bridge::dispatch_shim::outcome_to_invoke_remote_result(outcome);
        let failure = error.as_ref().map(|reason| Self::session_failure(reason));
        Some(SessionDispatch::Result {
            call_id,
            payload,
            terminal: true,
            failure,
            error,
            request_id,
        })
    }

    async fn open_stream_via_axon(
        &self,
        callee_ura: Option<&str>,
        subject_ura: Option<&str>,
        ability: &str,
        args: &[u8],
    ) -> Option<Result<easynet_axon::invocation::StreamingInvocationHandle, String>> {
        let runtime = self.local_runtime.as_ref()?;
        let options = runtime.ability_options(ability).await?;
        if !options.modes.stream || options.modes.rpc {
            return None;
        }
        crate::op_event!(
            component = local_session_dispatcher,
            kind = open_stream_via_axon,
            ability = ability,
        );
        let opened = match (callee_ura, subject_ura) {
            (Some(callee), Some(subject)) if !subject.trim().is_empty() => {
                crate::runtime::axon_bridge::dispatch_shim::open_stream_local_with_subject(
                    runtime,
                    callee,
                    subject,
                    ability,
                    args.to_vec(),
                )
                .await
            }
            _ => {
                crate::runtime::axon_bridge::dispatch_shim::open_stream_local(
                    runtime,
                    ability,
                    args.to_vec(),
                )
                .await
            }
        };
        Some(opened.map_err(|err| err.to_string()))
    }

    fn spawn_stream_forwarder(
        call_id: u64,
        mut handle: easynet_axon::invocation::StreamingInvocationHandle,
        outbound: SessionUpSender,
        sessions: Arc<Mutex<HashMap<u64, CancellationToken>>>,
    ) {
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
                let dispatch = match frame_result {
                    Ok(frame) => {
                        if frame.terminal && frame.payload.is_empty() {
                            sent_terminal = true;
                            SessionDispatch::Result {
                                call_id,
                                payload: Vec::new(),
                                terminal: true,
                                failure: None,
                                error: None,
                                request_id: None,
                            }
                        } else {
                            let terminal = frame.terminal;
                            crate::op_event!(
                                component = local_session_dispatcher,
                                kind = forwarding_stream_frame_up_bidi,
                                call_id = call_id,
                                payload_bytes = frame.payload.len(),
                                terminal = terminal,
                            );
                            sent_terminal = sent_terminal || terminal;
                            SessionDispatch::Result {
                                call_id,
                                payload: frame.payload,
                                terminal,
                                failure: None,
                                error: None,
                                request_id: None,
                            }
                        }
                    }
                    Err(err) => {
                        sent_terminal = true;
                        Self::session_error_result(
                            call_id,
                            format!("<self>.session: stream frame failed: {err}"),
                        )
                    }
                };
                let terminal = matches!(dispatch, SessionDispatch::Result { terminal: true, .. });
                if Self::send_dispatch_up(&outbound, &dispatch).await.is_err() || terminal {
                    break;
                }
            }
            if !sent_terminal && !cancelled {
                let dispatch = SessionDispatch::Result {
                    call_id,
                    payload: Vec::new(),
                    terminal: true,
                    failure: None,
                    error: None,
                    request_id: None,
                };
                let _ = Self::send_dispatch_up(&outbound, &dispatch).await;
            }
            // Cancellation must reach the RUNTIME task, not just this
            // forwarder. Dropping the handle alone leaves the ability's
            // emit loop alive holding its stream source — post-cancel
            // the mic.subscribe pipeline kept the cpal capture thread
            // (and the microphone) hot indefinitely, and the
            // context-capture tee never saw its consumer leave
            // (2026-06-10). cancel() is idempotent and a no-op on
            // already-terminal invocations, so the clean-EOS path and
            // the error path are both safe to route through here.
            if let Err(err) = handle.cancel("session stream closed").await {
                let err_msg = err.to_string();
                crate::op_event!(
                    component = local_session_dispatcher,
                    kind = stream_runtime_cancel_failed,
                    call_id = call_id,
                    error = err_msg,
                );
            }
            let mut guard = match sessions.lock() {
                Ok(g) => g,
                Err(poisoned) => poisoned.into_inner(),
            };
            guard.remove(&call_id);
        });
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
                "<self>.session: ability `{ability}` received encrypted args \
                 (encryption={}, key_id={:?}) but no session decryptor is wired",
                content.encryption, content.key_id
            ));
        }
        if !content.content_type.is_empty() && content.content_type != "application/json" {
            return Err(format!(
                "<self>.session: ability `{ability}` received unsupported args content_type {:?}",
                content.content_type
            ));
        }
        if !content.encoding.is_empty() && content.encoding != "identity" {
            return Err(format!(
                "<self>.session: ability `{ability}` received unsupported args encoding {:?}",
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
                    terminal: true,
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
                    terminal: true,
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
                terminal: true,
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
        registry: &crate::runtime::ability_wire::AbilityWireRegistry,
        call_id: u64,
        ability: &str,
        value: &Value,
    ) -> Result<Option<SessionDispatch>, SessionDispatchError> {
        if ability == crate::runtime::agents::pty_attach_ability::ABILITY_PTY_SESSION_ATTACH {
            return Self::map_remote_pty_output(call_id, value);
        }
        if Self::is_json_frame_bidi_with(registry, ability) {
            let frame_type = value.get("type").and_then(Value::as_str);
            let terminal = matches!(frame_type, Some("closed") | Some("error"));
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
                terminal,
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
        if !self.ability_wire.is_bidi_wire_ability(&ability) {
            return Self::send_bidi_result(
                outbound,
                &Self::session_error_result(
                    call_id,
                    format!("remote bidi ability `{ability}` is not wired on <self>.session"),
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
                    "<self>.session: LocalRuntime is not wired for remote bidi",
                ),
                None,
            )
            .await;
        };
        let wire = match crate::runtime::axon_bridge::dispatch_shim::admitted_from_wire_parts(
            envelope,
            ability.clone(),
            request.arguments,
        ) {
            Ok(wire) => wire,
            Err(err) => {
                return Self::send_bidi_result(
                    outbound,
                    &Self::session_error_result(
                        call_id,
                        format!("admit carrier-v1 bidi open: {err}"),
                    ),
                    None,
                )
                .await;
            }
        };
        let handle =
            match crate::runtime::axon_bridge::dispatch_shim::open_bidi_admitted(runtime, wire)
                .await
            {
                Ok(handle) => handle,
                Err(err) => {
                    return Self::send_bidi_result(
                        outbound,
                        &Self::session_error_result(
                            call_id,
                            format!("<self>.session: remote bidi open failed: {err}"),
                        ),
                        None,
                    )
                    .await;
                }
            };
        self.register_remote_bidi(call_id, &ability, handle, outbound);
        Ok(())
    }

    /// Device → hub bidi stream frame, sent per the session's
    /// negotiated contract: a carrier-v1 session gets the proto
    /// `DispatchResult`, a v0 session the retiring JSON `Result`.
    /// `terminal_receipt` is the locally-minted execution receipt for
    /// terminal frames — projected onto the wire so the receipt chain
    /// closes at the hub hop for streaming calls exactly as it does
    /// for the unary arm (DEC-F004's headline win).
    async fn send_bidi_result(
        outbound: &SessionUpSender,
        dispatch: &SessionDispatch,
        terminal_receipt: Option<&easynet_axon::invocation::InvocationReceipt>,
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
                return outbound
                    .send_payload(UpPayload::DispatchResult(PbDispatchResult {
                        call_id: *call_id,
                        payload: payload.clone(),
                        terminal: *terminal,
                        receipt: terminal_receipt
                            .map(easynet_axon::invocation::wire::receipt_to_wire),
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
        if !self.ability_wire.is_bidi_wire_ability(ability) {
            return Self::send_dispatch_up(
                outbound,
                &Self::file_transfer_terminal_error(
                    call_id,
                    format!("remote bidi ability `{ability}` is not wired on <self>.session"),
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
                    "<self>.session: LocalRuntime is not wired for remote bidi",
                ),
            )
            .await;
        };

        let handle = match (callee_ura.as_deref(), subject_ura.as_deref()) {
            (Some(callee), Some(subject)) if !subject.trim().is_empty() => {
                crate::runtime::axon_bridge::dispatch_shim::open_bidi_local_with_subject(
                    runtime, callee, subject, ability, args,
                )
                .await
            }
            _ => runtime.invoke_bidi_async(ability, args, None, None).await,
        };
        let handle = match handle {
            Ok(handle) => handle,
            Err(err) => {
                return Self::send_dispatch_up(
                    outbound,
                    &Self::file_transfer_terminal_error(
                        call_id,
                        format!("<self>.session: remote bidi open failed: {err}"),
                    ),
                )
                .await;
            }
        };
        self.register_remote_bidi(call_id, ability, handle, outbound);
        Ok(())
    }

    /// Bind an opened local bidi handle to `call_id`: register its
    /// input side for `BidiInput` forwarding and pump handler output
    /// back up the session, every frame sent per the negotiated
    /// contract. Shared by the JSON and carrier-v1 open arms.
    fn register_remote_bidi(
        &self,
        call_id: u64,
        ability: &str,
        handle: easynet_axon::invocation::BidiInvocationHandle,
        outbound: &SessionUpSender,
    ) {
        let (handler_in_tx, mut handler_out_rx) = handle.split();

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
            while let Some(frame_result) = handler_out_rx.next_frame().await {
                let frame = match frame_result {
                    Ok(frame) => frame,
                    Err(err) => {
                        let dispatch = LocalAxonSessionDispatcher::file_transfer_terminal_error(
                            call_id,
                            format!("<self>.session: remote file_transfer frame failed: {err}"),
                        );
                        let _ = LocalAxonSessionDispatcher::send_bidi_result(
                            &outbound, &dispatch, None,
                        )
                        .await;
                        break;
                    }
                };
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
                                            "<self>.session: remote bidi output map failed: {err}"
                                        ),
                                    ))
                                }
                            }
                        }
                        Err(err) => Some(LocalAxonSessionDispatcher::file_transfer_terminal_error(
                            call_id,
                            format!("<self>.session: remote bidi output was not JSON: {err}"),
                        )),
                    }
                };
                let Some(mapped) = mapped else {
                    if frame.terminal {
                        break;
                    }
                    continue;
                };
                let terminal = matches!(mapped, SessionDispatch::Result { terminal: true, .. });
                // Terminal frames carry the execution receipt minted by
                // the local runtime (Completed and Failed alike) so the
                // hub can ledger it — same chain closure as the unary
                // arm, fetched from the receiver because the split
                // consumer half retains the receipt surface.
                let terminal_receipt = if terminal {
                    handler_out_rx
                        .receipts()
                        .await
                        .into_iter()
                        .rev()
                        .find(|r| r.state.is_terminal())
                } else {
                    None
                };
                if LocalAxonSessionDispatcher::send_bidi_result(
                    &outbound,
                    &mapped,
                    terminal_receipt.as_ref(),
                )
                .await
                .is_err()
                {
                    break;
                }
                if terminal || frame.terminal {
                    break;
                }
            }
            let mut guard = match sessions.lock() {
                Ok(g) => g,
                Err(poisoned) => poisoned.into_inner(),
            };
            guard.remove(&call_id);
        });
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
            == crate::runtime::agents::pty_attach_ability::ABILITY_PTY_SESSION_ATTACH
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
        // Carrier-v1 dual-read (DEC-F004 / T2.1 step 3): the hub sends
        // DispatchCall for v1-negotiated sessions — the complete
        // canonical InvokeRequest, dispatched without re-projection.
        if let Some(DownPayload::DispatchCall(call)) = frame.payload.as_ref() {
            return self
                .handle_carrier_v1_dispatch(call.clone(), outbound)
                .await;
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

        // Route by variant. PR-N6 C4 added the `RequestResult`
        // direction (hub → device, the reply to a device-side
        // forward_invoke escalation). When the optional
        // `escalation_correlation` is wired (device-mode only),
        // route `RequestResult` to the correlation table to
        // complete the awaiting dispatcher future. `Dispatch`
        // continues to the local-RPC execution path below.
        // `Result` frames flow up from the device, not down,
        // so a down-stream Result is meaningless; ignore
        // (matches prior staging behaviour). `Request` frames
        // are device → hub and never appear here.
        let (
            call_id,
            callee_ura,
            subject_ura,
            ability,
            args,
            args_content_envelope,
            metadata,
            origin_caller,
        ) = match dispatch {
            SessionDispatch::Dispatch {
                call_id,
                callee_ura,
                subject_ura,
                ability,
                args,
                args_content_envelope,
                metadata,
                origin_caller,
            } => {
                let args_bytes = args.len();
                crate::op_event!(
                    component = local_session_dispatcher,
                    kind = received_dispatch_frame,
                    call_id = call_id,
                    ability = ability,
                    args_bytes = args_bytes,
                );
                (
                    call_id,
                    callee_ura,
                    subject_ura,
                    ability,
                    args,
                    args_content_envelope,
                    metadata,
                    origin_caller,
                )
            }
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
                    let fired = correlation.complete(call_id, outcome);
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
            SessionDispatch::Result { .. } | SessionDispatch::Request { .. } => {
                return Ok(());
            }
        };

        let result = if let Err(reason) =
            Self::validate_session_args_content(&ability, &args_content_envelope)
        {
            Ok(Self::session_error_result(call_id, reason))
        } else if let Some(stream_open) = self
            .open_stream_via_axon(
                callee_ura.as_deref(),
                subject_ura.as_deref(),
                &ability,
                &args,
            )
            .await
        {
            match stream_open {
                Ok(handle) => {
                    Self::spawn_stream_forwarder(
                        call_id,
                        handle,
                        outbound.clone(),
                        Arc::clone(&self.remote_stream_sessions),
                    );
                    return Ok(());
                }
                Err(reason) => Ok(Self::session_error_result(call_id, reason)),
            }
        } else {
            // ── Phase 5f: Axon-only session dispatch ───────────────
            //
            // When the shared `LocalRuntime` hosts this ability, route
            // through `invoke_async`. The runtime fires the wired
            // `LedgerSink` on the terminal event. If the runtime does
            // not host the ability, we return a terminal error; there
            // is no legacy RPC fallback from session frames.
            //
            // Every dispatch runs as its own task, replying through
            // the cloned outbound (the spawn_stream_forwarder
            // pattern), for two load-bearing reasons:
            // - Origin-caller dispatches may need the session channel
            //   themselves (resolve_key trust sync escalates up the
            //   SAME bidi this loop consumes) — awaiting them inline
            //   would deadlock the reply until the escalation times
            //   out.
            // - The session frame loop is the device's only inbound
            //   lane. Awaiting ability execution inline serialized
            //   EVERY invocation on this device behind the slowest
            //   in-flight one (a 2s ability turned 1ms echoes into
            //   2s waits, measured 2026-06-12). Concurrent replies
            //   are sequence-safe because SessionUpSender stamps and
            //   enqueues under its single-writer gate.
            let this = self.clone();
            let outbound_task = outbound.clone();
            tokio::spawn(async move {
                let result = match this
                    .try_dispatch_via_axon(
                        call_id,
                        callee_ura.as_deref(),
                        subject_ura.as_deref(),
                        &ability,
                        &args,
                        &metadata,
                        origin_caller.as_ref(),
                    )
                    .await
                {
                    Some(result) => result,
                    None => Self::session_error_result(
                        call_id,
                        format!(
                            "<self>.session: ability `{ability}` is not registered \
                             in Axon LocalRuntime"
                        ),
                    ),
                };
                let _ = Self::send_result_frame(&outbound_task, call_id, result).await;
            });
            return Ok(());
        }?;

        Self::send_result_frame(outbound, call_id, result).await
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    use serde_json::json;
    use std::time::Duration;

    fn dispatch_frame(call_id: u64, ability: &str, args: Vec<u8>) -> InvokeBidiDown {
        let dispatch = SessionDispatch::Dispatch {
            call_id,
            callee_ura: None,
            subject_ura: None,
            ability: ability.to_string(),
            args,
            args_content_envelope: SessionContentEnvelope::plaintext_json(),
            metadata: HashMap::new(),
            origin_caller: None,
        };
        let payload = serde_json::to_vec(&dispatch).expect("encode dispatch");
        InvokeBidiDown {
            sequence: 0,
            payload: Some(DownPayload::BinaryChunk(BinaryChunk {
                data: payload,
                ..BinaryChunk::default()
            })),
            ..InvokeBidiDown::default()
        }
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
        use easynet_axon::pb::axon::v1::{AgentIdentity, DispatchCall, InvokeRequest};
        InvokeBidiDown {
            payload: Some(DownPayload::DispatchCall(DispatchCall {
                call_id,
                request: Some(InvokeRequest {
                    envelope: Some(easynet_axon::pb::axon::v1::Envelope {
                        caller: Some(AgentIdentity {
                            ura: "easynet:///r/t/user/alice".into(),
                            profile: "easynet-strict-v2".into(),
                        }),
                        callee: Some(AgentIdentity {
                            ura: "easynet:///r/t/device/d1".into(),
                            profile: "easynet-strict-v2".into(),
                        }),
                        invocation_nonce: vec![9; 16],
                        ..Default::default()
                    }),
                    function_name: ability.to_string(),
                    arguments: args,
                    ..Default::default()
                }),
                open_bidi: false,
            })),
            ..InvokeBidiDown::default()
        }
    }

    fn carrier_v1_bidi_open(call_id: u64, ability: &str, args: Vec<u8>) -> InvokeBidiDown {
        let mut frame = carrier_v1_call(call_id, ability, args);
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

        let rt = easynet_axon::invocation::LocalRuntime::new();
        let _registry = build_real_daemon_registry_with_runtime(Some(Arc::clone(&rt)));
        let disp = LocalAxonSessionDispatcher::new().with_local_runtime(rt);
        let (tx, mut rx) = mpsc::channel::<InvokeBidiUp>(8);
        let session_tx = SessionUpSender::new(tx);
        session_tx.set_negotiated_contract(1);

        let args = serde_json::to_vec(&json!({
            "mode": "upload",
            "resource_ref": crate::runtime::resources::filesystem::resource_ref_for_local_path(
                &target,
                crate::runtime::resources::filesystem::FilesystemResourceCapability::Write,
            )
            .expect("local fs ResourceRef"),
        }))
        .expect("encode args");
        disp.handle_down(
            carrier_v1_bidi_open(
                77,
                crate::runtime::agents::file_transfer_ability::ABILITY_FILE_TRANSFER,
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
            .receipt
            .expect("terminal bidi frame carries the execution receipt (chain closure)");
        assert_eq!(
            receipt.state,
            easynet_axon::invocation::InvocationState::Completed.to_wire_i32(),
            "receipt must record the terminal state"
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
        let rt = easynet_axon::invocation::LocalRuntime::new();
        let disp = LocalAxonSessionDispatcher::new().with_local_runtime(rt);
        let (tx, mut rx) = mpsc::channel::<InvokeBidiUp>(4);
        let session_tx = SessionUpSender::new(tx);
        session_tx.set_negotiated_contract(1);

        disp.handle_down(carrier_v1_bidi_open(9, "test.echo", b"{}".to_vec()), &session_tx)
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
    async fn carrier_v1_bidi_open_on_v0_session_replies_json() {
        let rt = easynet_axon::invocation::LocalRuntime::new();
        let disp = LocalAxonSessionDispatcher::new().with_local_runtime(rt);
        let (tx, mut rx) = mpsc::channel::<InvokeBidiUp>(4);
        let session_tx = SessionUpSender::new(tx);

        disp.handle_down(carrier_v1_bidi_open(11, "test.echo", b"{}".to_vec()), &session_tx)
            .await
            .expect("open error replies as a frame, not an Err");

        let reply = rx.recv().await.expect("reply produced");
        let chunk = match reply.payload {
            Some(UpPayload::BinaryChunk(c)) => c,
            other => panic!("expected JSON reply on a v0 session, got: {other:?}"),
        };
        match serde_json::from_slice::<SessionDispatch>(&chunk.data).expect("Result decodes") {
            SessionDispatch::Result {
                call_id,
                terminal,
                error,
                ..
            } => {
                assert_eq!(call_id, 11);
                assert!(terminal);
                assert!(error.expect("error populated").contains("not wired"));
            }
            other => panic!("expected SessionDispatch::Result, got: {other:?}"),
        }
    }

    /// Quadrant [new hub, new device]: a v1-negotiated session
    /// receiving DispatchCall executes through the canonical path and
    /// replies with a proto DispatchResult (DEC-F004 / step 3).
    #[tokio::test]
    async fn carrier_v1_dispatch_executes_and_replies_proto_on_v1_session() {
        let rt = easynet_axon::invocation::LocalRuntime::new();
        rt.register_ability(
            "test.echo",
            easynet_axon::invocation::make_ability(|ctx| async move { Ok(ctx.payload.clone()) }),
        )
        .await
        .unwrap();
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
        assert_eq!(result.payload, br#"{"hello":"v1"}"#);
        assert!(result.failure.is_none());
    }

    /// Quadrant [hub jumped the gun, v0 session]: a DispatchCall on a
    /// session still negotiated v0 executes anyway and replies on the
    /// JSON carrier, which the hub dual-reads — no call is lost to
    /// negotiation skew.
    #[tokio::test]
    async fn carrier_v1_dispatch_on_v0_session_replies_json() {
        let rt = easynet_axon::invocation::LocalRuntime::new();
        rt.register_ability(
            "test.echo",
            easynet_axon::invocation::make_ability(|ctx| async move { Ok(ctx.payload.clone()) }),
        )
        .await
        .unwrap();
        let disp = LocalAxonSessionDispatcher::new().with_local_runtime(Arc::clone(&rt));
        let (tx, mut rx) = mpsc::channel::<InvokeBidiUp>(4);
        let session_tx = SessionUpSender::new(tx); // stays v0

        disp.handle_down(
            carrier_v1_call(8, "test.echo", br#"{"hello":"v0"}"#.to_vec()),
            &session_tx,
        )
        .await
        .expect("gun-jumped dispatch still succeeds");

        let reply = tokio::time::timeout(Duration::from_secs(3), rx.recv())
            .await
            .expect("reply within 3s")
            .expect("reply produced");
        let Some(UpPayload::BinaryChunk(chunk)) = reply.payload else {
            panic!(
                "v0 session must reply on the JSON carrier, got {:?}",
                reply.payload
            );
        };
        let parsed = SessionDispatch::decode_frame(&chunk.data).expect("Result decodes");
        let SessionDispatch::Result {
            call_id,
            payload,
            terminal,
            ..
        } = parsed
        else {
            panic!("expected Result variant");
        };
        assert_eq!(call_id, 8);
        assert!(terminal);
        assert_eq!(payload, br#"{"hello":"v0"}"#);
    }

    #[tokio::test]
    async fn dispatch_frame_executes_registered_rpc_and_returns_json_payload() {
        let rt = easynet_axon::invocation::LocalRuntime::new();
        rt.register_ability(
            "test.echo",
            easynet_axon::invocation::make_ability(|ctx| async move { Ok(ctx.payload.clone()) }),
        )
        .await
        .unwrap();
        let disp = LocalAxonSessionDispatcher::new().with_local_runtime(Arc::clone(&rt));
        let (tx, mut rx) = mpsc::channel::<InvokeBidiUp>(4);
        let session_tx = SessionUpSender::new(tx);

        disp.handle_down(
            dispatch_frame(1, "test.echo", br#"{"echo":"args-from-A"}"#.to_vec()),
            &session_tx,
        )
        .await
        .expect("handle_down returns Ok with terminal reply queued");

        let reply = rx.recv().await.expect("reply produced");
        assert_eq!(
            reply.sequence, 1,
            "first post-frame-0 reply must own up-direction sequence 1"
        );
        let chunk = match reply.payload {
            Some(UpPayload::BinaryChunk(c)) => c,
            other => panic!("expected BinaryChunk reply, got: {other:?}"),
        };
        let parsed: SessionDispatch = serde_json::from_slice(&chunk.data).expect("Result decodes");
        match parsed {
            SessionDispatch::Result {
                call_id,
                terminal,
                error,
                failure,
                payload,
                request_id: _,
                ..
            } => {
                assert_eq!(call_id, 1);
                assert!(terminal, "RPC reply is terminal");
                assert_eq!(error, None, "test.echo must succeed");
                assert!(failure.is_none(), "successful RPC must not carry failure");
                let value: serde_json::Value =
                    serde_json::from_slice(&payload).expect("payload decodes as JSON");
                assert_eq!(value, json!({"echo": "args-from-A"}));
            }
            other => panic!("expected SessionDispatch::Result, got: {other:?}"),
        }
    }

    #[tokio::test]
    async fn dispatch_frame_binds_explicit_resource_subject() {
        use easynet_axon::invocation::make_ability;

        let rt = easynet_axon::invocation::LocalRuntime::new();
        rt.register_ability(
            "camera.snapshot",
            make_ability(|ctx| async move {
                let subject = ctx
                    .runtime
                    .axiom_envelope_of(&ctx.invocation_id)
                    .await
                    .map(|signed| signed.envelope.subject.ura)
                    .unwrap_or_default();
                serde_json::to_vec(&json!({ "subject": subject }))
                    .map_err(|err| easynet_axon::invocation::AxonError::internal(err.to_string()))
            }),
        )
        .await
        .unwrap();
        let disp = LocalAxonSessionDispatcher::new().with_local_runtime(Arc::clone(&rt));
        let (tx, mut rx) = mpsc::channel::<InvokeBidiUp>(4);
        let session_tx = SessionUpSender::new(tx);
        let subject = "easynet:///r/acme/resource/camera-1";

        disp.handle_down(
            session_frame(SessionDispatch::Dispatch {
                call_id: 7,
                callee_ura: Some("easynet:///r/acme/device/dev-1".to_string()),
                subject_ura: Some(subject.to_string()),
                ability: "camera.snapshot".to_string(),
                args: b"{}".to_vec(),
                args_content_envelope: SessionContentEnvelope::plaintext_json(),
                metadata: HashMap::new(),
                origin_caller: None,
            }),
            &session_tx,
        )
        .await
        .expect("resource subject dispatch succeeds");

        let reply = rx.recv().await.expect("reply produced");
        let chunk = match reply.payload {
            Some(UpPayload::BinaryChunk(c)) => c,
            other => panic!("expected BinaryChunk reply, got: {other:?}"),
        };
        let parsed: SessionDispatch = serde_json::from_slice(&chunk.data).expect("Result decodes");
        let SessionDispatch::Result { payload, error, .. } = parsed else {
            panic!("expected Result, got {parsed:?}");
        };
        assert_eq!(error, None);
        let value: serde_json::Value = serde_json::from_slice(&payload).unwrap();
        assert_eq!(value, json!({ "subject": subject }));
    }

    #[tokio::test]
    async fn dispatch_frame_stream_only_ability_forwards_non_terminal_frames() {
        use easynet_axon::invocation::{
            make_ability, AbilityCallModes, AbilityOptions, BackpressurePolicy,
        };

        let rt = easynet_axon::invocation::LocalRuntime::new();
        rt.register_ability_with_options(
            "screen.subscribe",
            make_ability(|ctx| async move {
                ctx.emit_progress(
                    serde_json::to_vec(&json!({"seq": 1, "width": 640, "height": 360})).unwrap(),
                    "application/json",
                )
                .await?;
                ctx.emit_progress(
                    serde_json::to_vec(&json!({"seq": 2, "width": 640, "height": 360})).unwrap(),
                    "application/json",
                )
                .await?;
                Ok(Vec::new())
            }),
            AbilityOptions {
                modes: AbilityCallModes::STREAM,
                backpressure: BackpressurePolicy::Unbounded,
            },
        )
        .await
        .unwrap();
        let disp = LocalAxonSessionDispatcher::new().with_local_runtime(Arc::clone(&rt));
        let (tx, mut rx) = mpsc::channel::<InvokeBidiUp>(8);
        let session_tx = SessionUpSender::new(tx);

        disp.handle_down(
            session_frame(SessionDispatch::Dispatch {
                call_id: 8,
                callee_ura: Some("easynet:///r/acme/device/dev-1".to_string()),
                subject_ura: Some("easynet:///r/acme/resource/display-1".to_string()),
                ability: "screen.subscribe".to_string(),
                args: b"{}".to_vec(),
                args_content_envelope: SessionContentEnvelope::plaintext_json(),
                metadata: HashMap::new(),
                origin_caller: None,
            }),
            &session_tx,
        )
        .await
        .expect("stream-only dispatch opens and returns immediately");

        let mut frames = Vec::new();
        for _ in 0..3 {
            let reply = tokio::time::timeout(Duration::from_secs(1), rx.recv())
                .await
                .expect("stream reply should arrive")
                .expect("stream reply produced");
            let chunk = match reply.payload {
                Some(UpPayload::BinaryChunk(c)) => c,
                other => panic!("expected BinaryChunk reply, got: {other:?}"),
            };
            frames.push(serde_json::from_slice::<SessionDispatch>(&chunk.data).unwrap());
        }

        let mut payloads = Vec::new();
        for frame in &frames[..2] {
            match frame {
                SessionDispatch::Result {
                    call_id,
                    terminal,
                    payload,
                    error,
                    ..
                } => {
                    assert_eq!(*call_id, 8);
                    assert!(!terminal, "progress frames are non-terminal");
                    assert!(error.is_none());
                    payloads.push(serde_json::from_slice::<serde_json::Value>(payload).unwrap());
                }
                other => panic!("expected Result, got {other:?}"),
            }
        }
        assert_eq!(
            payloads,
            vec![
                json!({"seq": 1, "width": 640, "height": 360}),
                json!({"seq": 2, "width": 640, "height": 360}),
            ]
        );
        match &frames[2] {
            SessionDispatch::Result {
                call_id,
                terminal,
                payload,
                error,
                ..
            } => {
                assert_eq!(*call_id, 8);
                assert!(*terminal, "stream close emits terminal Result");
                assert!(payload.is_empty());
                assert!(error.is_none());
            }
            other => panic!("expected terminal Result, got {other:?}"),
        }
    }

    #[tokio::test]
    async fn session_dispatch_via_axon_runtime_writes_to_invocation_ledger() {
        // **Phase 5d regression pin.**
        //
        // Before the Axon-only path, the host daemon executed inbound
        // `SessionDispatch::Dispatch` outside Axon's `LocalRuntime`.
        // Net effect: the wired `LedgerSink`
        // never observed terminals from session-dispatched calls,
        // and `<ledger_dir>/invocations.redb` stayed empty even on
        // successful chats. The Web UI's history tab consequently
        // reported `INVOCATION RECORDS: 0` on a host that had just
        // served a successful invoke.
        //
        // This test pins the fix: when a `LocalRuntime` is attached
        // and hosts the target ability, the session-receive path
        // routes through `invoke_async` → LedgerSink → InvocationLedger.
        // One Dispatch frame in → one ledger row out.
        use easynet_axon::invocation::{
            make_ability, AbilityCallModes, AbilityOptions, BackpressurePolicy, InvocationLedger,
            LedgerSink, LocalRuntime,
        };

        let temp = tempfile::tempdir().expect("tempdir");
        let ledger =
            Arc::new(InvocationLedger::open(temp.path().join("inv.redb")).expect("open ledger"));
        let rt = LocalRuntime::new();
        rt.set_ledger_sink(LedgerSink::new(Arc::clone(&ledger)));
        rt.register_ability_with_options(
            "demo.session_echo",
            make_ability(|ctx| async move { Ok(ctx.payload.clone()) }),
            AbilityOptions {
                modes: AbilityCallModes::RPC,
                backpressure: BackpressurePolicy::Unbounded,
            },
        )
        .await
        .unwrap();

        let disp = LocalAxonSessionDispatcher::new().with_local_runtime(Arc::clone(&rt));
        let (tx, mut rx) = mpsc::channel::<InvokeBidiUp>(4);
        let session_tx = SessionUpSender::new(tx);

        disp.handle_down(
            dispatch_frame(
                42,
                "demo.session_echo",
                br#"{"k":"v-from-session"}"#.to_vec(),
            ),
            &session_tx,
        )
        .await
        .expect("session Dispatch routes through Axon and replies with terminal");

        // Wire-shape pin: the terminal frame the legacy path produced
        // and the Axon-routed path produce are byte-identical for
        // the success case (one terminal Result with payload + no
        // error).
        let reply = rx.recv().await.expect("reply produced");
        let chunk = match reply.payload {
            Some(UpPayload::BinaryChunk(c)) => c,
            other => panic!("expected BinaryChunk reply, got: {other:?}"),
        };
        let parsed: SessionDispatch = serde_json::from_slice(&chunk.data).expect("Result decodes");
        match parsed {
            SessionDispatch::Result {
                call_id,
                terminal,
                error,
                payload,
                request_id: _,
                ..
            } => {
                assert_eq!(call_id, 42);
                assert!(terminal, "Axon-routed reply still terminal");
                assert_eq!(error, None);
                let value: serde_json::Value = serde_json::from_slice(&payload).unwrap();
                assert_eq!(value, json!({"k": "v-from-session"}));
            }
            other => panic!("expected Result, got {other:?}"),
        }

        // Ledger persistence pin — the load-bearing claim of Phase 5d.
        // LedgerSink writes on the spawn task; yield + sleep matches
        // Axon's own `ledger_sink_persists_completed_invocation` test
        // pattern.
        tokio::task::yield_now().await;
        tokio::time::sleep(Duration::from_millis(50)).await;
        let records = ledger.list_all().expect("list ledger");
        assert_eq!(
            records.len(),
            1,
            "session-dispatched call must land EXACTLY one row in the ledger"
        );
        assert_eq!(records[0].ability_name, "demo.session_echo");
        assert_eq!(records[0].state, "COMPLETED");
        assert!(records[0].result.is_some());
        assert!(records[0].error.is_none());
        assert!(
            records[0].receipt_chain.verified,
            "audit chain must verify: {}",
            records[0].receipt_chain.verification_detail
        );
    }

    #[tokio::test]
    async fn session_dispatch_rejects_when_runtime_lacks_ability() {
        // Phase 5f invariant: if the runtime is wired but does NOT
        // host the requested ability, the dispatcher returns a typed
        // terminal error. It does not fall back to a legacy dispatcher.
        use easynet_axon::invocation::LocalRuntime;

        let rt = LocalRuntime::new();
        // Intentionally NOT registering `test.echo` in the runtime.
        let disp = LocalAxonSessionDispatcher::new().with_local_runtime(Arc::clone(&rt));
        let (tx, mut rx) = mpsc::channel::<InvokeBidiUp>(4);
        let session_tx = SessionUpSender::new(tx);

        disp.handle_down(
            dispatch_frame(99, "test.echo", br#"{"echo":"fallback"}"#.to_vec()),
            &session_tx,
        )
        .await
        .expect("runtime miss becomes terminal Result error");

        let reply = rx.recv().await.expect("reply produced");
        let chunk = match reply.payload {
            Some(UpPayload::BinaryChunk(c)) => c,
            other => panic!("expected BinaryChunk reply, got: {other:?}"),
        };
        let parsed: SessionDispatch = serde_json::from_slice(&chunk.data).unwrap();
        match parsed {
            SessionDispatch::Result {
                error,
                failure,
                payload,
                ..
            } => {
                assert!(payload.is_empty(), "runtime miss carries no payload");
                let err = error.expect("runtime miss must surface an error");
                assert!(err.contains("test.echo"));
                assert!(err.contains("LocalRuntime"));
                assert_eq!(
                    failure.as_ref().map(|failure| failure.code.as_str()),
                    Some("INVOCATION_FAILED")
                );
            }
            other => panic!("expected Result, got {other:?}"),
        }
    }

    #[tokio::test]
    async fn unregistered_ability_returns_terminal_error() {
        let disp = LocalAxonSessionDispatcher::new();
        let (tx, mut rx) = mpsc::channel::<InvokeBidiUp>(4);
        let session_tx = SessionUpSender::new(tx);

        disp.handle_down(
            dispatch_frame(7, "missing.ability", br#"{}"#.to_vec()),
            &session_tx,
        )
        .await
        .expect("missing ability becomes a terminal wire error, not transport failure");

        let reply = rx.recv().await.expect("reply produced");
        let chunk = match reply.payload {
            Some(UpPayload::BinaryChunk(c)) => c,
            other => panic!("expected BinaryChunk reply, got: {other:?}"),
        };
        let parsed: SessionDispatch = serde_json::from_slice(&chunk.data).expect("Result decodes");
        match parsed {
            SessionDispatch::Result {
                call_id,
                terminal,
                error,
                failure,
                payload,
                request_id: _,
                ..
            } => {
                assert_eq!(call_id, 7);
                assert!(terminal, "error reply must be terminal");
                assert!(payload.is_empty(), "failed dispatch carries no payload");
                let err = error.expect("missing ability must surface error");
                assert!(err.contains("missing.ability"));
                assert_eq!(
                    failure.as_ref().map(|failure| failure.code.as_str()),
                    Some("INVOCATION_FAILED")
                );
            }
            other => panic!("expected SessionDispatch::Result, got: {other:?}"),
        }
    }

    #[tokio::test]
    async fn legacy_only_handler_does_not_execute_without_runtime_registration() {
        // RPC dispatch is Axon-only. A handler that is not registered
        // in the runtime must not execute, which also means a missing
        // handler cannot tear down the session.
        let disp = LocalAxonSessionDispatcher::new();
        let (tx, mut rx) = mpsc::channel::<InvokeBidiUp>(4);
        let session_tx = SessionUpSender::new(tx);

        disp.handle_down(
            dispatch_frame(11, "always.panics", b"{}".to_vec()),
            &session_tx,
        )
        .await
        .expect("legacy-only handler returns terminal error");

        let reply = rx.recv().await.expect("terminal error reply emitted");
        assert_eq!(
            reply.sequence, 1,
            "first post-frame-0 reply must own up-direction sequence 1"
        );
        let chunk = match reply.payload {
            Some(UpPayload::BinaryChunk(c)) => c,
            other => panic!("expected BinaryChunk reply, got: {other:?}"),
        };
        let parsed: SessionDispatch = serde_json::from_slice(&chunk.data).expect("Result decodes");
        match parsed {
            SessionDispatch::Result {
                call_id,
                terminal,
                error,
                failure,
                payload,
                request_id: _,
                ..
            } => {
                assert_eq!(call_id, 11);
                assert!(terminal, "rejection reply must be terminal");
                assert!(payload.is_empty(), "rejected dispatch carries no payload");
                let err = error.expect("legacy-only handler must surface as Result.error");
                assert!(
                    err.contains("always.panics"),
                    "error must name the rejected ability; got: {err}"
                );
                assert!(err.contains("LocalRuntime"));
                assert_eq!(
                    failure.as_ref().map(|failure| failure.code.as_str()),
                    Some("INVOCATION_FAILED")
                );
            }
            other => panic!("expected SessionDispatch::Result, got: {other:?}"),
        }

        // Dispatcher must remain usable for follow-up calls: it
        // still emits a terminal Result frame instead of tearing
        // down the session.
        disp.handle_down(
            dispatch_frame(12, "test.echo", br#"{"after":"panic"}"#.to_vec()),
            &session_tx,
        )
        .await
        .expect("post-panic dispatch must still succeed");
        let follow_up = rx.recv().await.expect("post-panic reply produced");
        assert_eq!(
            follow_up.sequence, 2,
            "follow-up reply on the same session must increment the up sequence"
        );
        let chunk = match follow_up.payload {
            Some(UpPayload::BinaryChunk(c)) => c,
            other => panic!("expected BinaryChunk reply, got: {other:?}"),
        };
        let parsed: SessionDispatch = serde_json::from_slice(&chunk.data).expect("Result decodes");
        match parsed {
            SessionDispatch::Result {
                call_id,
                terminal,
                error,
                ..
            } => {
                assert_eq!(call_id, 12);
                assert!(terminal);
                let err = error.expect("follow-up legacy-only ability must still reject");
                assert!(err.contains("test.echo"));
                assert!(err.contains("LocalRuntime"));
            }
            other => panic!("expected SessionDispatch::Result, got: {other:?}"),
        }
    }

    #[tokio::test]
    async fn malformed_args_bytes_return_terminal_error() {
        let rt = easynet_axon::invocation::LocalRuntime::new();
        rt.register_ability(
            "test.echo",
            crate::runtime::ability_dispatch::rpc_handler_to_ability_fn(Arc::new(Ok)),
        )
        .await
        .unwrap();
        let disp = LocalAxonSessionDispatcher::new().with_local_runtime(Arc::clone(&rt));
        let (tx, mut rx) = mpsc::channel::<InvokeBidiUp>(4);
        let session_tx = SessionUpSender::new(tx);

        disp.handle_down(
            dispatch_frame(9, "test.echo", b"not-json".to_vec()),
            &session_tx,
        )
        .await
        .expect("bad args bytes must be surfaced as a terminal reply");

        let reply = rx.recv().await.expect("reply produced");
        let chunk = match reply.payload {
            Some(UpPayload::BinaryChunk(c)) => c,
            other => panic!("expected BinaryChunk reply, got: {other:?}"),
        };
        let parsed: SessionDispatch = serde_json::from_slice(&chunk.data).expect("Result decodes");
        match parsed {
            SessionDispatch::Result {
                call_id,
                terminal,
                error,
                failure,
                payload,
                request_id: _,
                ..
            } => {
                assert_eq!(call_id, 9);
                assert!(terminal, "malformed args error must be terminal");
                assert!(payload.is_empty());
                let err = error.expect("error message required");
                assert!(err.contains("payload not JSON"));
                assert_eq!(
                    failure.as_ref().map(|failure| failure.code.as_str()),
                    Some("INVOCATION_FAILED")
                );
            }
            other => panic!("expected SessionDispatch::Result, got: {other:?}"),
        }
    }

    #[tokio::test]
    async fn encrypted_dispatch_args_fail_closed_without_local_decryptor() {
        let disp = LocalAxonSessionDispatcher::new();
        let (tx, mut rx) = mpsc::channel::<InvokeBidiUp>(4);
        let session_tx = SessionUpSender::new(tx);

        disp.handle_down(
            session_frame(SessionDispatch::Dispatch {
                call_id: 19,
                callee_ura: None,
                subject_ura: None,
                ability: "test.echo".to_string(),
                args: b"ciphertext".to_vec(),
                args_content_envelope: SessionContentEnvelope {
                    content_type: "application/json".to_string(),
                    encoding: "identity".to_string(),
                    schema_ura: String::new(),
                    encryption: 1,
                    key_id: "session-key-1".to_string(),
                },
                metadata: HashMap::new(),
                origin_caller: None,
            }),
            &session_tx,
        )
        .await
        .expect("unsupported encrypted args become terminal result");

        let reply = rx.recv().await.expect("reply produced");
        let chunk = match reply.payload {
            Some(UpPayload::BinaryChunk(c)) => c,
            other => panic!("expected BinaryChunk reply, got: {other:?}"),
        };
        let parsed: SessionDispatch = serde_json::from_slice(&chunk.data).expect("Result decodes");
        match parsed {
            SessionDispatch::Result {
                call_id,
                terminal,
                error,
                failure,
                payload,
                request_id: _,
                ..
            } => {
                assert_eq!(call_id, 19);
                assert!(terminal);
                assert!(payload.is_empty());
                let err = error.expect("encrypted dispatch must fail closed");
                assert!(err.contains("encrypted args"));
                assert_eq!(
                    failure.as_ref().map(|failure| failure.code.as_str()),
                    Some("INVOCATION_FAILED")
                );
                assert!(
                    !err.contains("non-JSON"),
                    "encrypted bytes must not be parsed as plaintext JSON"
                );
            }
            other => panic!("expected SessionDispatch::Result, got: {other:?}"),
        }
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

    // ── LB-52 Gap 2 — device-mode boot wiring exposes baseline-locomotion ──
    //
    // The hub-pushed Dispatch frame's ability name is resolved
    // against the same Axon `LocalRuntime` the daemon binary wires via
    // `agents::build_registry_for_daemon` → `build_registry_with_services`.
    // That path registers the
    // AXIOM Tier 2.5 Baseline Locomotion Profile (fs.read /
    // fs.write / fs.list / fs.edit / process.exec / shell.run /
    // http.request) unconditionally, BEFORE the mode (hub /
    // device / both) branch in easynet-daemon.rs. So the same
    // ability surface lights up for device-mode daemons as for
    // hub-mode — no separate `register_all_abilities_for_device`
    // path is required.
    //
    // This test pins that invariant by walking the same boot
    // path (real registry construction with empty sub-services)
    // and pushing a real Dispatch frame for `fs.read` against
    // a tempfile through `LocalAxonSessionDispatcher::handle_down`.
    // Asserts the up-channel receives a terminal Result frame
    // whose payload decodes to an `fs.read` response containing
    // the file's bytes.

    fn build_real_daemon_registry_with_runtime(
        local_runtime: Option<Arc<easynet_axon::invocation::LocalRuntime>>,
    ) -> Arc<crate::runtime::ability_dispatch::AxonAbilityCatalog> {
        use crate::runtime::execution::discuss::DiscussService;
        use crate::runtime::execution::loop_instance::LoopService;
        use crate::runtime::execution::permission::PermissionService;
        use crate::runtime::execution::schedule::ScheduleService;
        use crate::runtime::execution::session::SessionService;
        crate::runtime::agents::build_registry_with_services(
            Arc::new(SessionService::new()),
            Arc::new(PermissionService::new()),
            Arc::new(DiscussService::new()),
            Arc::new(ScheduleService::new()),
            Arc::new(LoopService::new()),
            None,
            &Default::default(),
            Arc::new(Vec::new()),
            crate::runtime::agents::PagesIdentity::default(),
            local_runtime,
            Arc::new(
                crate::runtime::agents::agent_lifecycle_ability::SharedHotRegistrarCell::new(),
            ),
        )
    }

    #[tokio::test]
    async fn device_mode_dispatcher_executes_fs_read_through_baseline_locomotion_registry() {
        let tmp = tempfile::tempdir().expect("tempdir");
        let target = tmp.path().join("hello.txt");
        std::fs::write(&target, "device-B-bytes-from-real-fs-read").expect("seed temp file");

        let rt = easynet_axon::invocation::LocalRuntime::new();
        let _registry = build_real_daemon_registry_with_runtime(Some(Arc::clone(&rt)));
        let disp = LocalAxonSessionDispatcher::new().with_local_runtime(Arc::clone(&rt));
        let (tx, mut rx) = mpsc::channel::<InvokeBidiUp>(4);
        let session_tx = SessionUpSender::new(tx);

        let args = serde_json::json!({
            "resource_ref": crate::runtime::resources::filesystem::resource_ref_for_local_path(
                &target,
                crate::runtime::resources::filesystem::FilesystemResourceCapability::Read,
            )
            .expect("local fs ResourceRef"),
            "encoding": "utf8",
        });
        let frame = dispatch_frame(
            42,
            "fs.read",
            serde_json::to_vec(&args).expect("encode args"),
        );

        disp.handle_down(frame, &session_tx)
            .await
            .expect("fs.read dispatches through device-mode registry");

        let reply = rx.recv().await.expect("reply produced");
        let chunk = match reply.payload {
            Some(UpPayload::BinaryChunk(c)) => c,
            other => panic!("expected BinaryChunk reply, got: {other:?}"),
        };
        let parsed: SessionDispatch = serde_json::from_slice(&chunk.data).expect("Result decodes");
        match parsed {
            SessionDispatch::Result {
                call_id,
                terminal,
                error,
                payload,
                request_id: _,
                ..
            } => {
                assert_eq!(call_id, 42);
                assert!(terminal, "fs.read RPC reply is terminal");
                assert_eq!(error, None, "fs.read on a real file must succeed");
                let value: serde_json::Value =
                    serde_json::from_slice(&payload).expect("payload decodes as JSON");
                let bytes = value
                    .get("content")
                    .and_then(|v| v.as_str())
                    .or_else(|| value.get("text").and_then(|v| v.as_str()))
                    .expect("fs.read response carries content/text field");
                assert_eq!(
                    bytes, "device-B-bytes-from-real-fs-read",
                    "payload bytes must come from the device-side filesystem, \
                     not a daemon-internal stub"
                );
            }
            other => panic!("expected SessionDispatch::Result, got: {other:?}"),
        }
    }

    #[tokio::test(flavor = "multi_thread", worker_threads = 2)]
    async fn remote_file_transfer_upload_round_trips_over_session_bidi_frames() {
        let tmp = tempfile::tempdir().expect("tempdir");
        let target = tmp.path().join("upload-from-hub.bin");
        let bytes = b"remote-file-transfer-over-session";

        let rt = easynet_axon::invocation::LocalRuntime::new();
        let _registry = build_real_daemon_registry_with_runtime(Some(Arc::clone(&rt)));
        let disp = LocalAxonSessionDispatcher::new().with_local_runtime(rt);
        let (tx, mut rx) = mpsc::channel::<InvokeBidiUp>(8);
        let session_tx = SessionUpSender::new(tx);

        disp.handle_down(
            session_frame(SessionDispatch::BidiOpen {
                call_id: 77,
                callee_ura: None,
                subject_ura: None,
                ability: crate::runtime::agents::file_transfer_ability::ABILITY_FILE_TRANSFER
                    .to_string(),
                args: serde_json::to_vec(&json!({
                    "mode": "upload",
                    "resource_ref": crate::runtime::resources::filesystem::resource_ref_for_local_path(
                        &target,
                        crate::runtime::resources::filesystem::FilesystemResourceCapability::Write,
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

        let reply = tokio::time::timeout(Duration::from_secs(3), rx.recv())
            .await
            .expect("terminal reply within 3s")
            .expect("reply produced");
        let chunk = match reply.payload {
            Some(UpPayload::BinaryChunk(c)) => c,
            other => panic!("expected BinaryChunk reply, got: {other:?}"),
        };
        let parsed: SessionDispatch = serde_json::from_slice(&chunk.data).expect("Result decodes");
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
                assert!(terminal, "upload completion must be terminal");
                assert!(error.is_none(), "upload must succeed, got {error:?}");
                let value: serde_json::Value =
                    serde_json::from_slice(&payload).expect("payload decodes as JSON");
                assert_eq!(value.get("type").and_then(|v| v.as_str()), Some("complete"));
            }
            other => panic!("expected SessionDispatch::Result, got: {other:?}"),
        }

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
    fn remote_desktop_bidi_closed_frame_is_terminal() {
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
                assert!(terminal);
                assert!(error.is_none());
                assert!(failure.is_none());
            }
            other => panic!("expected SessionDispatch::Result, got: {other:?}"),
        }
    }

    #[test]
    #[cfg(feature = "remote-desktop")]
    fn remote_desktop_bidi_error_frame_is_typed_terminal_failure() {
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
                assert!(terminal);
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
            crate::runtime::ability_wire::AbilityWireRegistry::for_test_plugin_bidi([(
                "remote_desktop.attach".to_string(),
                crate::runtime::ability_wire::AbilityBidiWireKind::JsonFrames,
            )]),
        ))
    }

    #[tokio::test(flavor = "multi_thread", worker_threads = 2)]
    async fn remote_file_transfer_download_round_trips_over_session_bidi_frames() {
        let tmp = tempfile::tempdir().expect("tempdir");
        let target = tmp.path().join("download-to-hub.bin");
        let bytes = b"remote-download-bytes-from-device";
        std::fs::write(&target, bytes).expect("seed file");

        let rt = easynet_axon::invocation::LocalRuntime::new();
        let _registry = build_real_daemon_registry_with_runtime(Some(Arc::clone(&rt)));
        let disp = LocalAxonSessionDispatcher::new().with_local_runtime(rt);
        let (tx, mut rx) = mpsc::channel::<InvokeBidiUp>(16);
        let session_tx = SessionUpSender::new(tx);

        disp.handle_down(
            session_frame(SessionDispatch::BidiOpen {
                call_id: 88,
                callee_ura: None,
                subject_ura: None,
                ability: crate::runtime::agents::file_transfer_ability::ABILITY_FILE_TRANSFER
                    .to_string(),
                args: serde_json::to_vec(&json!({
                    "mode": "download",
                    "resource_ref": crate::runtime::resources::filesystem::resource_ref_for_local_path(
                        &target,
                        crate::runtime::resources::filesystem::FilesystemResourceCapability::Read,
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
        let mut saw_terminal = false;
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
                    assert_eq!(call_id, 88);
                    assert!(error.is_none(), "download must succeed, got {error:?}");
                    if terminal {
                        saw_terminal = true;
                        let value: serde_json::Value =
                            serde_json::from_slice(&payload).expect("payload decodes as JSON");
                        assert_eq!(value.get("type").and_then(|v| v.as_str()), Some("complete"));
                        break;
                    }
                    streamed.extend_from_slice(&payload);
                }
                other => panic!("expected SessionDispatch::Result, got: {other:?}"),
            }
        }

        assert_eq!(streamed, bytes);
        assert!(saw_terminal, "download must emit terminal completion frame");
    }

    #[tokio::test(flavor = "multi_thread", worker_threads = 2)]
    async fn remote_file_transfer_download_missing_file_returns_typed_terminal_failure() {
        let tmp = tempfile::tempdir().expect("tempdir");
        let target = tmp.path().join("missing-download.bin");

        let rt = easynet_axon::invocation::LocalRuntime::new();
        let _registry = build_real_daemon_registry_with_runtime(Some(Arc::clone(&rt)));
        let disp = LocalAxonSessionDispatcher::new().with_local_runtime(rt);
        let (tx, mut rx) = mpsc::channel::<InvokeBidiUp>(8);
        let session_tx = SessionUpSender::new(tx);

        disp.handle_down(
            session_frame(SessionDispatch::BidiOpen {
                call_id: 89,
                callee_ura: None,
                subject_ura: None,
                ability: crate::runtime::agents::file_transfer_ability::ABILITY_FILE_TRANSFER
                    .to_string(),
                args: serde_json::to_vec(&json!({
                    "mode": "download",
                    "resource_ref": crate::runtime::resources::filesystem::resource_ref_for_local_path(
                        &target,
                        crate::runtime::resources::filesystem::FilesystemResourceCapability::Read,
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

        let reply = tokio::time::timeout(Duration::from_secs(3), rx.recv())
            .await
            .expect("terminal failure within 3s")
            .expect("reply produced");
        let chunk = match reply.payload {
            Some(UpPayload::BinaryChunk(c)) => c,
            other => panic!("expected BinaryChunk reply, got: {other:?}"),
        };
        let parsed: SessionDispatch = serde_json::from_slice(&chunk.data).expect("Result decodes");
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
                assert!(terminal, "download failure must be terminal");
                let error = error.expect("terminal error string");
                assert!(
                    error.contains("not_found"),
                    "download failure must preserve handler code, got: {error}"
                );
                let failure = failure.expect("typed terminal failure");
                assert_eq!(failure.code, "NOT_FOUND");
                assert_eq!(failure.message, error);
                let payload: Value = serde_json::from_slice(&payload).expect("json payload");
                assert_eq!(payload["type"], "error");
                assert_eq!(payload["code"], "not_found");
            }
            other => panic!("expected SessionDispatch::Result, got: {other:?}"),
        }
    }
}
